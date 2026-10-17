//! The event journal: every agent event, in order, on disk, until the
//! control plane acknowledges it. Survives host disconnects and supervisor
//! restarts; replayed after every attach.

use anyhow::{Context, Result};
use fctl_proto::uplink::AgentEvent;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Rewrite the journal once this much acknowledged data accumulates.
const COMPACT_BYTES: u64 = 8 * 1024 * 1024;

pub struct Journal {
    path: PathBuf,
    ack_path: PathBuf,
    file: File,
    last_seq: u64,
    acked: u64,
    bytes: u64,
}

impl Journal {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("journal.jsonl");
        let ack_path = dir.join("journal.ack");
        let acked = std::fs::read_to_string(&ack_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let mut last_seq = acked;
        let mut bytes = 0u64;
        if let Ok(f) = File::open(&path) {
            for line in BufReader::new(f).lines() {
                let Ok(line) = line else { break };
                bytes += line.len() as u64 + 1;
                if let Ok(ev) = serde_json::from_str::<AgentEvent>(&line) {
                    last_seq = last_seq.max(ev.agent_seq);
                }
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self {
            path,
            ack_path,
            file,
            last_seq,
            acked,
            bytes,
        })
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn acked(&self) -> u64 {
        self.acked
    }

    /// Appends an event body with the next sequence number.
    pub fn append(&mut self, ts: i64, body: fctl_proto::event::EventBody) -> Result<AgentEvent> {
        let ev = AgentEvent {
            agent_seq: self.last_seq + 1,
            ts,
            body,
        };
        let mut line = serde_json::to_string(&ev)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.bytes += line.len() as u64;
        self.last_seq = ev.agent_seq;
        Ok(ev)
    }

    /// Events with `agent_seq > after`, in order.
    pub fn read_after(&self, after: u64) -> Result<Vec<AgentEvent>> {
        let f = File::open(&self.path)?;
        let mut out = Vec::new();
        for line in BufReader::new(f).lines() {
            let line = line?;
            if let Ok(ev) = serde_json::from_str::<AgentEvent>(&line)
                && ev.agent_seq > after
            {
                out.push(ev);
            }
        }
        Ok(out)
    }

    /// Continues the numbering after `seq`, which the control plane already
    /// has (a fresh sandbox for a session that ran elsewhere). Stored as
    /// acknowledged, so it survives a restart.
    pub fn start_after(&mut self, seq: u64) -> Result<()> {
        if seq <= self.last_seq {
            return Ok(());
        }
        self.last_seq = seq;
        self.acked = seq;
        std::fs::write(&self.ack_path, seq.to_string())?;
        Ok(())
    }

    /// Records the control plane's acknowledgment; compacts when worthwhile.
    pub fn ack(&mut self, seq: u64) -> Result<()> {
        if seq <= self.acked {
            return Ok(());
        }
        self.acked = seq.min(self.last_seq);
        std::fs::write(&self.ack_path, self.acked.to_string())?;
        if self.bytes > COMPACT_BYTES {
            self.compact()?;
        }
        Ok(())
    }

    fn compact(&mut self) -> Result<()> {
        let keep = self.read_after(self.acked)?;
        let tmp = self.path.with_extension("jsonl.tmp");
        let mut f = File::create(&tmp)?;
        let mut bytes = 0u64;
        for ev in &keep {
            let mut line = serde_json::to_string(ev)?;
            line.push('\n');
            bytes += line.len() as u64;
            f.write_all(line.as_bytes())?;
        }
        f.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.bytes = bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::event::{EventBody, LogLevel, LogLine, LogSource};

    fn log(text: &str) -> EventBody {
        EventBody::Log(LogLine {
            source: LogSource::Agent,
            level: LogLevel::Info,
            text: text.into(),
        })
    }

    #[test]
    fn continues_numbering_from_elsewhere() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut j = Journal::open(dir.path())?;
        j.start_after(25)?;
        assert_eq!(j.append(1, log("first here"))?.agent_seq, 26);
        j.start_after(10)?;
        assert_eq!(j.append(2, log("second"))?.agent_seq, 27);
        drop(j);
        let mut j = Journal::open(dir.path())?;
        assert_eq!(j.last_seq(), 27);
        assert_eq!(j.read_after(25)?.len(), 2);
        assert_eq!(j.append(3, log("after a restart"))?.agent_seq, 28);
        Ok(())
    }

    #[test]
    fn appends_replays_and_survives_reopen() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut j = Journal::open(dir.path())?;
        for i in 0..5 {
            j.append(i, log(&format!("line {i}")))?;
        }
        assert_eq!(j.last_seq(), 5);
        assert_eq!(j.read_after(3)?.len(), 2);
        j.ack(3)?;
        drop(j);
        let mut j = Journal::open(dir.path())?;
        assert_eq!((j.last_seq(), j.acked()), (5, 3));
        let ev = j.append(9, log("after reopen"))?;
        assert_eq!(ev.agent_seq, 6);
        j.compact()?;
        let rest = j.read_after(0)?;
        assert_eq!(
            rest.iter().map(|e| e.agent_seq).collect::<Vec<_>>(),
            vec![4, 5, 6]
        );
        Ok(())
    }
}

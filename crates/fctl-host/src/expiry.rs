//! Removing stopped sandboxes after `keep_stopped_hours` (0 keeps them).
//!
//! A stopped session keeps its sandbox so it can resume where it left off.
//! The host notes when each session sandbox stopped (or, for older ones, when
//! it first saw it stopped) in `<data dir>/stopped.json`, and removes it once
//! it has been stopped for longer than the limit. Resuming that session later
//! creates a new sandbox on its branch, and Claude starts a new conversation.

use crate::driver::{SandboxInfo, session_of};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Sandbox name → when it stopped (milliseconds since the epoch).
pub type Stopped = BTreeMap<String, i64>;

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("stopped.json")
}

pub fn load(path: &Path) -> Stopped {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, stopped: &Stopped) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(stopped)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Records that `sandbox` stopped at `now`.
pub fn note_stopped(path: &Path, sandbox: &str, now: i64) -> Result<()> {
    let mut stopped = load(path);
    stopped.insert(sandbox.to_owned(), now);
    save(path, &stopped)
}

/// What to remove now, and what to remember: session sandboxes that are
/// stopped and not in use, stopped since the recorded time (or since now).
/// `keep_ms` of `None` keeps them all.
pub fn plan(
    stopped: &Stopped,
    sandboxes: &[SandboxInfo],
    in_use: &HashSet<String>,
    now: i64,
    keep_ms: Option<i64>,
) -> (Vec<String>, Stopped) {
    let mut remove = Vec::new();
    let mut keep = Stopped::new();
    for sb in sandboxes {
        if session_of(&sb.name).is_none() || sb.running || in_use.contains(&sb.name) {
            continue;
        }
        let since = stopped.get(&sb.name).copied().unwrap_or(now);
        match keep_ms {
            Some(limit) if now - since >= limit => remove.push(sb.name.clone()),
            _ => {
                keep.insert(sb.name.clone(), since);
            }
        }
    }
    (remove, keep)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3_600_000;

    fn sb(name: &str, running: bool) -> SandboxInfo {
        SandboxInfo {
            name: name.into(),
            running,
        }
    }

    #[test]
    fn removes_only_long_stopped_session_sandboxes() {
        let old = "fctl-ses-01aaa";
        let recent = "fctl-ses-01bbb";
        let unseen = "fctl-ses-01ccc";
        let running = "fctl-ses-01ddd";
        let resuming = "fctl-ses-01eee";
        let stopped = Stopped::from([
            (old.into(), 0),
            (recent.into(), 70 * HOUR),
            (running.into(), 0),
            (resuming.into(), 0),
            ("fctl-ses-01gone".into(), 0),
        ]);
        let list = [
            sb(old, false),
            sb(recent, false),
            sb(unseen, false),
            sb(running, true),
            sb(resuming, false),
            sb("fctl-base", false),
            sb("fctl-login-01xyz", false),
        ];
        let in_use = HashSet::from([resuming.to_owned()]);
        let now = 100 * HOUR;
        let (remove, keep) = plan(&stopped, &list, &in_use, now, Some(72 * HOUR));
        assert_eq!(remove, vec![old.to_owned()]);
        // Recorded times carry over; a sandbox seen stopped for the first
        // time starts its clock now; running, in-use and vanished ones go.
        assert_eq!(
            keep,
            Stopped::from([(recent.into(), 70 * HOUR), (unseen.into(), now)])
        );
    }

    #[test]
    fn zero_hours_keeps_everything() {
        let stopped = Stopped::from([("fctl-ses-01aaa".into(), 0)]);
        let (remove, keep) = plan(
            &stopped,
            &[sb("fctl-ses-01aaa", false)],
            &HashSet::new(),
            1_000 * HOUR,
            None,
        );
        assert!(remove.is_empty());
        assert_eq!(keep, stopped);
    }

    #[test]
    fn records_survive_a_restart() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = path(dir.path());
        assert!(load(&p).is_empty());
        note_stopped(&p, "fctl-ses-01aaa", 5)?;
        note_stopped(&p, "fctl-ses-01bbb", 7)?;
        assert_eq!(
            load(&p),
            Stopped::from([("fctl-ses-01aaa".into(), 5), ("fctl-ses-01bbb".into(), 7)])
        );
        Ok(())
    }
}

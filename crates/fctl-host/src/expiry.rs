//! Removing stopped sandboxes after `keep_stopped_hours` (0 keeps them), and
//! sooner when the disk runs low (`min_free_disk_gb`).
//!
//! A stopped session keeps its sandbox so it can resume where it left off.
//! The host notes when each session sandbox stopped (or, for older ones, when
//! it first saw it stopped) in `<data dir>/stopped.json`, and removes it once
//! it has been stopped for longer than the limit. While the disk has less
//! free space than the host keeps, it removes the others too, those stopped
//! longest first, until there is room again. Resuming that session later
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

/// Forgets `sandbox`, once it is removed.
pub fn forget(path: &Path, sandbox: &str) -> Result<()> {
    let mut stopped = load(path);
    if stopped.remove(sandbox).is_some() {
        save(path, &stopped)?;
    }
    Ok(())
}

/// Why the sweep removes a sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// It has been stopped for longer than `keep_stopped_hours`.
    Expired,
    /// The disk has less free space than the host keeps.
    LowDisk,
}

/// What a sweep removes, and what the host remembers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Stopped for longer than the limit: they go now.
    pub expired: Vec<String>,
    /// The others, those stopped longest first: they go while the disk is
    /// low.
    pub oldest_first: Vec<String>,
    /// Every one of them, with when it stopped: they stay on the list
    /// until they are gone, for a host that stops halfway.
    pub stopped: Stopped,
}

impl Plan {
    /// The removals in the order the sweep tries them.
    pub fn removals(self) -> impl Iterator<Item = (String, Why)> {
        let expired = self.expired.into_iter().map(|n| (n, Why::Expired));
        expired.chain(self.oldest_first.into_iter().map(|n| (n, Why::LowDisk)))
    }
}

/// What to remove and what to remember: session sandboxes that are stopped
/// and not in use, stopped since the recorded time (or since now).
/// `keep_ms` of `None` keeps them all, unless the disk runs low.
pub fn plan(
    stopped: &Stopped,
    sandboxes: &[SandboxInfo],
    in_use: &HashSet<String>,
    now: i64,
    keep_ms: Option<i64>,
) -> Plan {
    let mut plan = Plan::default();
    let mut others = Vec::new();
    for sb in sandboxes {
        if session_of(&sb.name).is_none() || sb.running || in_use.contains(&sb.name) {
            continue;
        }
        let since = stopped.get(&sb.name).copied().unwrap_or(now);
        plan.stopped.insert(sb.name.clone(), since);
        match keep_ms {
            Some(limit) if now - since >= limit => plan.expired.push(sb.name.clone()),
            _ => others.push((since, sb.name.clone())),
        }
    }
    others.sort();
    plan.oldest_first = others.into_iter().map(|(_, name)| name).collect();
    plan
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
        let plan = plan(&stopped, &list, &in_use, now, Some(72 * HOUR));
        assert_eq!(plan.expired, vec![old.to_owned()]);
        // Recorded times carry over; a sandbox seen stopped for the first
        // time starts its clock now; running, in-use and vanished ones go.
        assert_eq!(
            plan.stopped,
            Stopped::from([
                (old.into(), 0),
                (recent.into(), 70 * HOUR),
                (unseen.into(), now)
            ])
        );
        // On a low disk, the others go too, those stopped longest first.
        assert_eq!(
            plan.oldest_first,
            vec![recent.to_owned(), unseen.to_owned()]
        );
        assert_eq!(
            plan.removals().collect::<Vec<_>>(),
            vec![
                (old.to_owned(), Why::Expired),
                (recent.to_owned(), Why::LowDisk),
                (unseen.to_owned(), Why::LowDisk)
            ]
        );
    }

    #[test]
    fn zero_hours_keeps_everything_while_there_is_room() {
        let stopped = Stopped::from([("fctl-ses-01aaa".into(), 5), ("fctl-ses-01bbb".into(), 3)]);
        let list = [sb("fctl-ses-01aaa", false), sb("fctl-ses-01bbb", false)];
        let plan = plan(&stopped, &list, &HashSet::new(), 1_000 * HOUR, None);
        assert!(plan.expired.is_empty());
        assert_eq!(plan.stopped, stopped);
        assert_eq!(
            plan.oldest_first,
            vec!["fctl-ses-01bbb".to_owned(), "fctl-ses-01aaa".to_owned()]
        );
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

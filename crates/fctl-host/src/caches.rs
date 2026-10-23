//! Package caches that a project's sessions on this host share
//! (`ProjectSettings::package_caches`): what npm, pnpm, Yarn, Bun, Cargo,
//! uv, pip, Go and Playwright download, so that each session's setup
//! script doesn't fetch it all again.
//!
//! With Docker, a project's caches are a named volume,
//! `fctl-cache-<project>`, on the daemon's disk like its sandboxes. With
//! Apple's `container`, they are a directory in the data directory,
//! `caches/<project>`, which sandboxes mount: a named volume there is an
//! ext4 image, which two running VMs must not share. The process driver's
//! agents use that directory directly. Sandboxes see the caches at
//! [`MOUNT`], and fctl-agent points the package managers there. Lima's VMs
//! get none.
//!
//! `<data dir>/caches.json` records when each project's caches were last
//! used and measured, and which sandboxes were made with them: after a
//! restart, the host still knows whose caches its sandboxes write to. The
//! sweep removes the caches no session has started with for
//! `keep_caches_days`, measures those used since they were last measured
//! and empties the ones past `max_cache_gb`, and while the disk is low
//! empties the others, least recently used first, before it removes
//! stopped sandboxes. It leaves the caches of a project with a session
//! here alone, and a session that starts while its project's caches are
//! measured or emptied waits for that.

use crate::driver::SandboxInfo;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Where sandboxes see their project's package caches.
pub const MOUNT: &str = "/var/cache/fctl";

const VOLUME_PREFIX: &str = "fctl-cache-";

/// Where a project's package caches are on this host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spot {
    /// A Docker volume.
    Volume(String),
    /// A directory on this machine.
    Dir(PathBuf),
}

impl std::fmt::Display for Spot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Volume(name) => write!(f, "the volume {name}"),
            Self::Dir(dir) => write!(f, "{}", dir.display()),
        }
    }
}

/// Whether a project id can name a volume and a directory: the control
/// plane's are `prj_` and letters and digits.
pub fn valid(project: &str) -> bool {
    !project.is_empty()
        && project.len() <= 64
        && project
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The Docker volume of a project's caches: `fctl-cache-prj_01ab…`.
pub fn volume(project: &str) -> String {
    format!("{VOLUME_PREFIX}{project}")
}

/// The project whose caches a Docker volume holds.
pub fn project_of(volume: &str) -> Option<&str> {
    volume.strip_prefix(VOLUME_PREFIX).filter(|p| valid(p))
}

/// The directory of a project's caches, with Apple's `container` and the
/// process driver.
pub fn dir(data_dir: &Path, project: &str) -> PathBuf {
    data_dir.join("caches").join(project)
}

/// The projects whose caches have a directory in `data_dir`.
pub fn dirs(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("caches")) else {
        return Vec::new();
    };
    let mut projects: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|p| valid(p))
        .collect();
    projects.sort();
    projects
}

/// What the host knows of the caches here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Records {
    /// Each project's caches, by project id.
    pub projects: BTreeMap<String, Record>,
    /// Sandbox name → the project whose caches it was made with.
    pub sandboxes: BTreeMap<String, String>,
}

impl Records {
    /// Whether a sandbox on this host was made with `project`'s caches,
    /// and may mount them again.
    pub fn mounted(&self, project: &str) -> bool {
        self.sandboxes.values().any(|p| p == project)
    }

    /// The projects whose caches the sandboxes in `sandboxes` were made
    /// with.
    pub fn projects_of<S: AsRef<str>>(
        &self,
        sandboxes: impl IntoIterator<Item = S>,
    ) -> HashSet<String> {
        sandboxes
            .into_iter()
            .filter_map(|sb| self.sandboxes.get(sb.as_ref()).cloned())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    /// When a session last started with them (milliseconds since the epoch).
    pub used_at: i64,
    /// What they took when last measured, in MiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_mb: Option<u64>,
    /// When they were last measured, or emptied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_at: Option<i64>,
}

impl Record {
    /// Whether no session has started with them since they were found
    /// empty, or emptied: there is nothing to free.
    pub fn empty(&self) -> bool {
        self.size_mb == Some(0) && self.measured_at.is_some_and(|m| m >= self.used_at)
    }

    /// Whether a session started with them since they were last measured.
    pub fn grown(&self) -> bool {
        self.measured_at.is_none_or(|m| m < self.used_at)
    }
}

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("caches.json")
}

pub fn load(path: &Path) -> Records {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, records: &Records) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(records)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Records that a session starts in `sandbox` with `project`'s caches at
/// `now`.
pub fn note_used(path: &Path, sandbox: &str, project: &str, now: i64) -> Result<()> {
    let mut records = load(path);
    records
        .projects
        .entry(project.to_owned())
        .or_default()
        .used_at = now;
    records
        .sandboxes
        .insert(sandbox.to_owned(), project.to_owned());
    save(path, &records)
}

/// Why the sweep measures, empties or removes a project's caches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// No session has started with them for `keep_caches_days`: they go,
    /// or stay emptied while a sandbox here may mount them again.
    Unused,
    /// A session started with them since they were last measured: they
    /// are measured, and emptied past `max_cache_gb`.
    Grown,
    /// The disk has less free space than the host keeps: they are emptied.
    LowDisk,
}

/// What a sweep does with the caches, and what the host remembers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Unused for longer than `keep_caches_days`: they go now.
    pub unused: Vec<String>,
    /// Used since they were last measured: they are measured now.
    pub grown: Vec<String>,
    /// Every other one not in use, least recently used first: they are
    /// emptied while the disk is low.
    pub least_recent_first: Vec<String>,
    /// The records to keep: those of the caches here and of the sessions
    /// here, and the sandboxes that are here.
    pub records: Records,
}

/// What to do with the caches `here` (by project), given the sandboxes on
/// this host and those of the sessions it runs or starts (`sessions`):
/// the caches that one of them uses, a session here or a sandbox that
/// runs, are left alone. A cache with no record starts its clock now.
/// `keep_ms` of `None` keeps unused caches.
pub fn plan(
    records: &Records,
    here: &[String],
    sandboxes: &[SandboxInfo],
    sessions: &HashSet<String>,
    now: i64,
    keep_ms: Option<i64>,
) -> Plan {
    let mut plan = Plan::default();
    let listed = |name: &str| sandboxes.iter().find(|s| s.name == name);
    for (sandbox, project) in &records.sandboxes {
        if sessions.contains(sandbox) || listed(sandbox).is_some() {
            plan.records
                .sandboxes
                .insert(sandbox.clone(), project.clone());
        }
    }
    let in_use = plan.records.projects_of(
        plan.records
            .sandboxes
            .keys()
            .map(String::as_str)
            .filter(|sb| sessions.contains(*sb) || listed(sb).is_some_and(|s| s.running)),
    );
    let mut others = Vec::new();
    for project in here {
        if plan.records.projects.contains_key(project) {
            continue;
        }
        let record = records.projects.get(project).copied().unwrap_or(Record {
            used_at: now,
            ..Record::default()
        });
        plan.records.projects.insert(project.clone(), record);
        if in_use.contains(project) {
            continue;
        }
        if keep_ms.is_some_and(|keep| now - record.used_at >= keep) {
            plan.unused.push(project.clone());
            continue;
        }
        if record.grown() {
            plan.grown.push(project.clone());
        }
        if !record.empty() {
            others.push((record.used_at, project.clone()));
        }
    }
    // Sessions that start with caches that aren't made yet.
    for project in &in_use {
        if let Some(record) = records.projects.get(project) {
            plan.records
                .projects
                .entry(project.clone())
                .or_insert(*record);
        }
    }
    others.sort();
    plan.least_recent_first = others.into_iter().map(|(_, p)| p).collect();
    plan
}

/// The disk the files in `dir` take, in MiB (rounded up), without
/// following symlinks. A missing `dir` takes none.
pub fn dir_size_mb(dir: &Path) -> std::io::Result<u64> {
    let mut bytes: u64 = 0;
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        for entry in entries {
            let entry = entry?;
            let meta = entry.metadata()?;
            if meta.is_dir() {
                todo.push(entry.path());
            }
            bytes = bytes.saturating_add(allocated(&meta));
        }
    }
    Ok(bytes.div_ceil(1024 * 1024))
}

#[cfg(unix)]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated(meta: &std::fs::Metadata) -> u64 {
    meta.len()
}

/// Removes what `dir` holds, and `dir` itself unless `keep`, without
/// following symlinks. Go's module cache makes its directories read-only,
/// so they are made writable first. A missing `dir` is fine.
pub fn clear_dir(dir: &Path, keep: bool) -> std::io::Result<()> {
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    };
    let mut todo = vec![dir.to_owned()];
    while let Some(d) = todo.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&d) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        writable(&d, &meta)?;
        if let Ok(entries) = std::fs::read_dir(&d) {
            // `file_type` doesn't follow symlinks.
            let dirs = entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()));
            todo.extend(dirs.map(|e| e.path()));
        }
    }
    if !keep {
        return gone(std::fs::remove_dir_all(dir));
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            gone(std::fs::remove_dir_all(entry.path()))?;
        } else {
            gone(std::fs::remove_file(entry.path()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn writable(dir: &Path, meta: &std::fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    if mode & 0o700 == 0o700 {
        return Ok(());
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode | 0o700))
}

#[cfg(not(unix))]
fn writable(_dir: &Path, _meta: &std::fs::Metadata) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400_000;

    fn sb(name: &str, running: bool) -> SandboxInfo {
        SandboxInfo {
            name: name.into(),
            running,
        }
    }

    fn record(used_days_ago: i64, measured_days_ago: Option<i64>, size_mb: Option<u64>) -> Record {
        let now = 100 * DAY;
        Record {
            used_at: now - used_days_ago * DAY,
            size_mb,
            measured_at: measured_days_ago.map(|d| now - d * DAY),
        }
    }

    #[test]
    fn names_say_whose_caches_they_are() {
        assert_eq!(volume("prj_01M5ABC"), "fctl-cache-prj_01M5ABC");
        assert_eq!(project_of("fctl-cache-prj_01M5ABC"), Some("prj_01M5ABC"));
        assert_eq!(project_of("fctl-cache-../x"), None);
        assert_eq!(project_of("fctl-sandboxes"), None);
        assert!(!valid("") && !valid("../etc") && !valid("prj-1") && !valid("a/b"));
        assert_eq!(
            dir(Path::new("/data"), "prj_1"),
            PathBuf::from("/data/caches/prj_1")
        );
    }

    #[test]
    fn caches_in_use_are_left_alone_and_the_others_planned() {
        let now = 100 * DAY;
        let records = Records {
            projects: BTreeMap::from([
                // Unused for 20 days.
                ("prj_old".into(), record(20, Some(19), Some(500))),
                // Used since it was last measured.
                ("prj_grown".into(), record(1, Some(2), Some(300))),
                // Measured since its last use.
                ("prj_measured".into(), record(3, Some(2), Some(700))),
                // Emptied since its last use: nothing to free.
                ("prj_empty".into(), record(5, Some(4), Some(0))),
                // A session here uses it, and another made a sandbox for it.
                ("prj_busy".into(), record(30, None, None)),
                ("prj_running".into(), record(30, None, None)),
                // Gone, and nothing uses it.
                ("prj_gone".into(), record(1, None, None)),
                // Not made yet: a session starts with it.
                ("prj_new".into(), record(0, None, None)),
            ]),
            sandboxes: BTreeMap::from([
                ("fctl-ses-01busy".into(), "prj_busy".into()),
                ("fctl-ses-01run".into(), "prj_running".into()),
                ("fctl-ses-01new".into(), "prj_new".into()),
                ("fctl-ses-01stopped".into(), "prj_old".into()),
                ("fctl-ses-01removed".into(), "prj_grown".into()),
            ]),
        };
        let here: Vec<String> = [
            "prj_old",
            "prj_grown",
            "prj_measured",
            "prj_empty",
            "prj_busy",
            "prj_running",
            "prj_unseen",
        ]
        .map(String::from)
        .into();
        let sandboxes = [
            sb("fctl-ses-01busy", false),
            sb("fctl-ses-01run", true),
            sb("fctl-ses-01stopped", false),
        ];
        let sessions = HashSet::from(["fctl-ses-01busy".to_owned(), "fctl-ses-01new".to_owned()]);
        let plan = plan(&records, &here, &sandboxes, &sessions, now, Some(14 * DAY));
        assert_eq!(plan.unused, ["prj_old"]);
        // Never measured: its clock starts now, and it is measured.
        assert_eq!(plan.grown, ["prj_grown", "prj_unseen"]);
        assert_eq!(
            plan.least_recent_first,
            ["prj_measured", "prj_grown", "prj_unseen"]
        );
        assert_eq!(
            plan.records
                .projects
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "prj_busy",
                "prj_empty",
                "prj_grown",
                "prj_measured",
                "prj_new",
                "prj_old",
                "prj_running",
                "prj_unseen"
            ]
        );
        assert_eq!(plan.records.projects["prj_unseen"].used_at, now);
        // A removed sandbox is forgotten; one still here may mount its
        // project's caches again.
        assert!(!plan.records.sandboxes.contains_key("fctl-ses-01removed"));
        assert!(plan.records.mounted("prj_old") && !plan.records.mounted("prj_grown"));
        // 0 days keeps unused caches.
        let kept = super::plan(&records, &here, &sandboxes, &sessions, now, None);
        assert!(kept.unused.is_empty());
        assert_eq!(kept.least_recent_first[0], "prj_old");
    }

    #[test]
    fn records_survive_a_restart() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let p = path(dir.path());
        assert_eq!(load(&p), Records::default());
        note_used(&p, "fctl-ses-01aaa", "prj_1", 5)?;
        note_used(&p, "fctl-ses-01bbb", "prj_1", 7)?;
        let records = load(&p);
        assert_eq!(records.projects["prj_1"].used_at, 7);
        assert_eq!(
            records.projects_of(["fctl-ses-01aaa", "fctl-ses-01ccc"]),
            HashSet::from(["prj_1".to_owned()])
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn clearing_removes_read_only_files_and_leaves_what_symlinks_point_to() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir()?;
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside)?;
        std::fs::write(outside.join("keep"), "x")?;
        let cache = tmp.path().join("caches/prj_1");
        let module = cache.join("go/mod/example.com/m@v1.0.0");
        std::fs::create_dir_all(&module)?;
        std::fs::write(module.join("go.mod"), "module m\n")?;
        std::fs::write(cache.join("npm"), vec![1u8; 2 * 1024 * 1024])?;
        std::os::unix::fs::symlink(&outside, cache.join("link"))?;
        // As Go leaves them.
        std::fs::set_permissions(
            module.join("go.mod"),
            std::fs::Permissions::from_mode(0o444),
        )?;
        std::fs::set_permissions(&module, std::fs::Permissions::from_mode(0o555))?;
        assert!(dir_size_mb(&cache)? >= 2);
        assert_eq!(dirs(tmp.path()), ["prj_1"]);
        clear_dir(&cache, true)?;
        assert!(cache.is_dir() && std::fs::read_dir(&cache)?.next().is_none());
        assert!(outside.join("keep").exists());
        assert_eq!(dir_size_mb(&cache)?, 0);
        clear_dir(&cache, false)?;
        assert!(!cache.exists());
        // Missing already.
        clear_dir(&cache, false)?;
        assert_eq!(dir_size_mb(&cache)?, 0);
        Ok(())
    }
}

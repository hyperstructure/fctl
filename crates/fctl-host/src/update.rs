//! Updating fctl in place: `fctl self update`, and hosts that update
//! themselves (docs/architecture.md#host-updates).
//!
//! A release has an archive for each target on `download_base`,
//! `v<version>/fctl-<target>.tar.gz`, checked against its `.sha256`, with
//! fctl and the Linux agent that sandboxes run. Installing it replaces both
//! next to the running fctl: each is written beside its place first, then
//! renamed over it, so that a running binary keeps its old copy and nothing
//! changes unless every file could be written.
//!
//! A host that updates itself follows the version the control plane says
//! it runs when the host connects ([`follow`]): once that is newer than the
//! host's own, it downloads that release, and tries again while it is
//! missing, since the control plane can run a release before its archives
//! are published. The daemon then installs it and restarts on it once no
//! session is being started or stopped here ([`restart`]); sessions keep
//! running in their sandboxes, as they do over any restart.

use crate::service;
use anyhow::{Context, Result, bail};
use fctl_client::HttpClient;
use fctl_domain::updates::{self, Miss, Version};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::watch;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long one download of a release may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// How long the new fctl may take to say its version.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// Hosts hear of a release together, as they reconnect after the control
/// plane is deployed: each waits up to this long before its first try, so
/// that their downloads spread out.
const SPREAD: Duration = Duration::from_secs(30);

/// Where the running fctl is installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    /// Its path with links followed, as it was when this process started:
    /// once replaced, Linux names the running binary `… (deleted)`.
    pub exe: PathBuf,
}

/// Why fctl can't replace itself where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// Homebrew installed it, and updates it: `brew upgrade fctl`.
    Homebrew,
    /// Its directory can't be written to.
    ReadOnly { dir: PathBuf, error: String },
}

impl std::fmt::Display for Blocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Homebrew => f.write_str(
                "Homebrew installed fctl: `brew upgrade fctl`, then `fctl host restart`, updates it",
            ),
            Self::ReadOnly { dir, error } => write!(
                f,
                "fctl can't write to {} ({error}): `fctl self update`, run with the rights to it, updates it",
                dir.display()
            ),
        }
    }
}

impl Install {
    /// The running fctl.
    pub fn here() -> Result<Self> {
        let exe = std::env::current_exe()
            .and_then(std::fs::canonicalize)
            .context("finding this fctl")?;
        Ok(Self { exe })
    }

    /// The directory fctl and the Linux agent are in.
    pub fn dir(&self) -> &Path {
        self.exe.parent().unwrap_or(Path::new("/"))
    }

    /// Whether Homebrew installed it: it is in Homebrew's Cellar.
    pub fn homebrew(&self) -> bool {
        self.exe.components().any(|c| c.as_os_str() == "Cellar")
    }

    /// Why fctl can't update itself here, if it can't: Homebrew installed
    /// it, or its directory can't be written to (which this tries).
    pub fn blocked(&self) -> Option<Blocked> {
        if self.homebrew() {
            return Some(Blocked::Homebrew);
        }
        let dir = self.dir();
        let probe = dir.join(format!(".fctl-write-check-{}", std::process::id()));
        match std::fs::write(&probe, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
                None
            }
            Err(e) => Some(Blocked::ReadOnly {
                dir: dir.to_owned(),
                error: e.kind().to_string(),
            }),
        }
    }
}

/// This machine's release target, as the installer picks it.
pub fn this_target() -> Result<String> {
    updates::target(
        std::env::consts::OS,
        std::env::consts::ARCH,
        under_rosetta(),
    )
    .map_err(anyhow::Error::msg)
}

fn under_rosetta() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("sysctl")
            .args(["-n", "sysctl.proc_translated"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1")
}

/// `https://fctl.sh/releases`, from a setting that may end in `/`.
pub fn base(download_base: &str) -> &str {
    download_base.trim_end_matches('/')
}

/// The latest release on `base`, as its `latest` says.
pub async fn latest(base: &str) -> Result<Version> {
    let http = HttpClient::new(DOWNLOAD_TIMEOUT);
    let body = http
        .download(&format!("{base}/latest"))
        .await
        .context("looking up the latest release")?;
    let text = String::from_utf8_lossy(&body);
    Version::parse(&text).with_context(|| format!("unexpected version: {}", text.trim()))
}

/// A release of fctl for this machine, downloaded and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// fctl and the Linux agent, by the names they go by.
    pub files: Vec<(String, Vec<u8>)>,
}

/// Downloads `version`'s archive for `target` from `base`, and checks it:
/// its SHA-256, its files, and that the fctl in it runs here and is that
/// version, which it tries with a copy in `dir`, where it goes.
pub async fn download(
    base: &str,
    version: Version,
    target: &str,
    dir: &Path,
) -> Result<Release, Miss> {
    let http = HttpClient::new(DOWNLOAD_TIMEOUT);
    let archive = updates::archive(target);
    let url = format!("{base}/v{version}/{archive}");
    let missing = |e: fctl_client::Error| match e {
        fctl_client::Error::Status { status: 404, .. } => {
            Miss::Missing(format!("{base} has no fctl {version} for {target}"))
        }
        e => Miss::Missing(format!("downloading fctl {version}: {e}")),
    };
    // Its checksum first, which is small: it goes up after the archive.
    let sums = http
        .download(&format!("{url}.sha256"))
        .await
        .map_err(missing)?;
    let bytes = http.download(&url).await.map_err(missing)?;
    let expected = String::from_utf8_lossy(&sums)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if expected.is_empty() || expected != hex::encode(Sha256::digest(&bytes)) {
        return Err(Miss::Unusable(format!(
            "checksum mismatch for {archive} of fctl {version}"
        )));
    }
    let agent = updates::agent(target);
    let files = unpack(&bytes, &["fctl", &agent])
        .map_err(|e| Miss::Unusable(format!("{archive}: {e:#}")))?;
    let Some((_, fctl)) = files.iter().find(|(n, _)| n == "fctl") else {
        return Err(Miss::Unusable(format!("fctl is missing from {archive}")));
    };
    // It must run where it goes, and be that version.
    let probe =
        write_beside(dir, "fctl-probe", fctl).map_err(|e| Miss::Unusable(format!("{e:#}")))?;
    let ran = is_version(&probe, version).await;
    let _ = std::fs::remove_file(&probe);
    ran.map_err(|e| Miss::Unusable(format!("the new fctl {version} did not run here ({e:#})")))?;
    Ok(Release { version, files })
}

/// Whether the fctl at `path` runs, and says it is `version`.
async fn is_version(path: &Path, version: Version) -> Result<()> {
    let out = says_its_version(path).await?;
    let said = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
    }
    if !said.split_whitespace().any(|w| w == version.to_string()) {
        bail!("it says it is {}", said.trim());
    }
    Ok(())
}

/// "Text file busy", on Linux and macOS alike.
const ETXTBSY: i32 = 26;

/// What `path --version` prints. A process that something else here starts
/// holds copies of this process's open files until it runs its program,
/// and running a file while a copy of it is open for writing fails with
/// "Text file busy": it tries again then.
async fn says_its_version(path: &Path) -> Result<std::process::Output> {
    let mut busy = 0;
    loop {
        let run = tokio::process::Command::new(path)
            .arg("--version")
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(PROBE_TIMEOUT, run).await {
            Err(_) => bail!("it didn't answer within {} s", PROBE_TIMEOUT.as_secs()),
            Ok(Err(e)) if e.raw_os_error() == Some(ETXTBSY) && busy < 20 => {
                busy += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(out) => return Ok(out?),
        }
    }
}

/// The named files from a `.tar.gz` release archive.
pub fn unpack(archive: &[u8], names: &[&str]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut out = Vec::new();
    for entry in tar.entries().context("reading the archive")? {
        let mut entry = entry.context("reading the archive")?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let name = path.trim_start_matches("./");
        if names.contains(&name) {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            out.push((name.to_owned(), bytes));
        }
    }
    for n in names {
        if !out.iter().any(|(name, _)| name == n) {
            bail!("{n} is missing from the archive");
        }
    }
    Ok(out)
}

/// Writes `bytes` beside `dir/name`, executable, and returns where.
fn write_beside(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let write = || -> std::io::Result<()> {
        std::fs::write(&tmp, bytes)?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            bail!(
                "cannot write to {}: run the installer with the rights to it instead:\n  curl -fsSL https://fctl.sh/install | FCTL_INSTALL_DIR={} sh",
                dir.display(),
                dir.display()
            );
        }
        return Err(e).with_context(|| format!("writing {}", tmp.display()));
    }
    Ok(tmp)
}

impl Release {
    /// Puts the release in `dir`, in place of the fctl and the Linux agent
    /// there: nothing changes unless every file could be written.
    pub fn install(&self, dir: &Path) -> Result<()> {
        let mut written: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (name, bytes) in &self.files {
            match write_beside(dir, name, bytes) {
                Ok(tmp) => written.push((tmp, dir.join(name))),
                Err(e) => {
                    for (tmp, _) in &written {
                        let _ = std::fs::remove_file(tmp);
                    }
                    return Err(e);
                }
            }
        }
        for (tmp, dest) in &written {
            std::fs::rename(tmp, dest).with_context(|| format!("replacing {}", dest.display()))?;
        }
        Ok(())
    }
}

/// How a host that installed a release starts again on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restart {
    /// It exits, and the host service it is the process of (launchd's or
    /// systemd's, which keep it running) starts the new fctl.
    Service,
    /// It runs the new fctl in its own place: it runs in the foreground,
    /// or under something other than the host service.
    InPlace,
}

impl Restart {
    /// How this process starts again: [`Restart::Service`] when it is the
    /// host service's process.
    pub async fn here() -> Self {
        match tokio::task::spawn_blocking(service::runs_this_process).await {
            Ok(true) => Self::Service,
            _ => Self::InPlace,
        }
    }
}

/// Starts the host again on the fctl at `exe`, which it just installed.
/// Returns only if it can't. With [`Restart::InPlace`], what this process
/// started and leaves behind (the links to its agents) is reaped only when
/// it exits; sessions recover as they do after any restart.
pub fn restart(how: Restart, exe: &Path) -> anyhow::Error {
    match how {
        Restart::Service => std::process::exit(0),
        Restart::InPlace => {
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let e = std::process::Command::new(exe)
                    .args(std::env::args_os().skip(1))
                    .exec();
                anyhow::Error::from(e).context(format!("running {}", exe.display()))
            }
            #[cfg(not(unix))]
            {
                anyhow::anyhow!("running {} in this process's place", exe.display())
            }
        }
    }
}

/// `a minute`, `30 minutes`, `2 hours`.
pub fn in_words(d: Duration) -> String {
    let minutes = d.as_secs().div_ceil(60).max(1);
    match minutes {
        1 => "a minute".into(),
        m if m < 120 => format!("{m} minutes"),
        m if m % 60 == 0 => format!("{} hours", m / 60),
        m => format!("{:.1} hours", m as f64 / 60.0),
    }
}

/// Follows the version the control plane runs, as `target` says it, for a
/// host that updates itself: once that is newer than this host's version,
/// tries to get its release with `fetch` until it has it, and returns it.
/// While the release is missing it tries again every minute, then less
/// often ([`updates::retry_in`]); a newer version, or none, starts over.
/// The first try waits for `hold_off` (after an install that failed), and
/// a few seconds besides ([`SPREAD`]), as other hosts heard of it too.
pub async fn follow<F, Fut>(
    mut target: watch::Receiver<Option<Version>>,
    hold_off: Duration,
    mut fetch: F,
) -> Release
where
    F: FnMut(Version) -> Fut,
    Fut: Future<Output = Result<Release, Miss>>,
{
    let ours = Version::parse(VERSION);
    let mut hold_off = hold_off;
    loop {
        let version = loop {
            let newer = (*target.borrow_and_update()).filter(|v| ours.is_some_and(|o| *v > o));
            if let Some(v) = newer {
                break v;
            }
            if target.changed().await.is_err() {
                return std::future::pending().await;
            }
        };
        let mut wait = hold_off + spread();
        hold_off = Duration::ZERO;
        let mut first: Option<tokio::time::Instant> = None;
        let mut unusable = 0u32;
        let mut said_every_minute = false;
        let mut said_less_often = false;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                changed = target.changed() => {
                    if changed.is_err() {
                        return std::future::pending().await;
                    }
                    // Another version, or none: start over.
                    break;
                }
            }
            let started = *first.get_or_insert_with(tokio::time::Instant::now);
            let miss = match fetch(version).await {
                Ok(release) => return release,
                Err(miss) => miss,
            };
            if matches!(miss, Miss::Unusable(_)) {
                unusable += 1;
            }
            let waited = tokio::time::Instant::now().duration_since(started);
            wait = updates::retry_in(&miss, waited, unusable);
            match &miss {
                Miss::Missing(why) if wait == updates::MISSING_EVERY => {
                    if !said_every_minute {
                        said_every_minute = true;
                        tracing::info!(
                            "can't update to fctl {version} yet ({why}): trying again every minute"
                        );
                    } else {
                        tracing::debug!("fctl {version}: {why}");
                    }
                }
                Miss::Missing(why) => {
                    if !said_less_often {
                        said_less_often = true;
                        tracing::warn!(
                            "still can't update to fctl {version} after an hour ({why}): trying again every {}",
                            in_words(wait)
                        );
                    } else {
                        tracing::debug!("fctl {version}: {why}");
                    }
                }
                Miss::Unusable(why) => tracing::warn!(
                    "can't update to fctl {version}: {why}; trying again in {}",
                    in_words(wait)
                ),
            }
            wait = jitter(wait);
        }
    }
}

/// A random wait of up to [`SPREAD`].
fn spread() -> Duration {
    let r = u16::from_le_bytes(fctl_domain::random_bytes::<2>());
    SPREAD.mul_f64(f64::from(r) / f64::from(u16::MAX))
}

/// `d`, give or take a tenth, so that hosts that try together drift apart.
fn jitter(d: Duration) -> Duration {
    let r = u16::from_le_bytes(fctl_domain::random_bytes::<2>());
    d.mul_f64(0.9 + 0.2 * f64::from(r) / f64::from(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn v(s: &str) -> Result<Version> {
        Version::parse(s).context("a version")
    }

    fn archive(files: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, data) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            h.set_cksum();
            builder.append_data(&mut h, name, *data)?;
        }
        Ok(builder.into_inner()?.finish()?)
    }

    #[test]
    fn unpacks_release_archives() -> Result<()> {
        let archive = archive(&[
            ("fctl", b"new fctl"),
            ("fctl-agent-aarch64-linux", b"agent"),
        ])?;
        let files = unpack(&archive, &["fctl", "fctl-agent-aarch64-linux"])?;
        assert_eq!(files[0], ("fctl".into(), b"new fctl".to_vec()));
        assert!(unpack(&archive, &["fctl", "fctl-agent-x86_64-linux"]).is_err());
        Ok(())
    }

    #[test]
    fn installing_replaces_every_file_or_none() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("fctl"), "old")?;
        std::fs::write(dir.path().join("fctl-agent-aarch64-linux"), "old agent")?;
        let release = Release {
            version: v("0.22.0")?,
            files: vec![
                ("fctl".into(), b"new".to_vec()),
                ("fctl-agent-aarch64-linux".into(), b"new agent".to_vec()),
            ],
        };
        release.install(dir.path())?;
        assert_eq!(std::fs::read_to_string(dir.path().join("fctl"))?, "new");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("fctl-agent-aarch64-linux"))?,
            "new agent"
        );
        // Nothing is left beside them.
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 2);
        // A file that can't be written leaves the others as they were.
        let gone = dir.path().join("gone");
        let broken = Release {
            files: vec![
                ("fctl".into(), b"newer".to_vec()),
                ("../gone/agent".into(), b"agent".to_vec()),
            ],
            ..release
        };
        assert!(broken.install(dir.path()).is_err());
        assert!(!gone.exists());
        assert_eq!(std::fs::read_to_string(dir.path().join("fctl"))?, "new");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 2);
        Ok(())
    }

    #[test]
    fn fctl_from_homebrew_or_a_directory_it_cant_write_doesnt_update_itself() -> Result<()> {
        let homebrew = Install {
            exe: "/opt/homebrew/Cellar/fctl/0.21.0/bin/fctl".into(),
        };
        assert!(homebrew.homebrew());
        assert_eq!(homebrew.blocked(), Some(Blocked::Homebrew));
        let dir = tempfile::tempdir()?;
        let here = Install {
            exe: dir.path().join("fctl"),
        };
        assert_eq!(here.blocked(), None);
        // It leaves nothing behind.
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        let nowhere = Install {
            exe: dir.path().join("missing/fctl"),
        };
        assert!(matches!(nowhere.blocked(), Some(Blocked::ReadOnly { .. })));
        Ok(())
    }

    #[test]
    fn waits_say_how_long_in_words() {
        let minutes = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(in_words(Duration::from_secs(54)), "a minute");
        assert_eq!(in_words(minutes(30)), "30 minutes");
        assert_eq!(in_words(minutes(160)), "2.7 hours");
        assert_eq!(in_words(minutes(720)), "12 hours");
    }

    /// What `follow`'s tries were asked for, and when.
    type Tries = Arc<Mutex<Vec<(Version, tokio::time::Instant)>>>;

    /// A `fetch` that answers `answers` in turn, then that it is missing.
    fn fetcher(
        tries: &Tries,
        answers: Vec<Result<Release, Miss>>,
    ) -> impl FnMut(Version) -> std::future::Ready<Result<Release, Miss>> + use<> {
        let tries = tries.clone();
        let mut answers = answers.into_iter();
        move |version| {
            if let Ok(mut t) = tries.lock() {
                t.push((version, tokio::time::Instant::now()));
            }
            std::future::ready(
                answers
                    .next()
                    .unwrap_or_else(|| Err(Miss::Missing("not there".into()))),
            )
        }
    }

    fn release(version: Version) -> Release {
        Release {
            version,
            files: vec![("fctl".into(), b"new".to_vec())],
        }
    }

    #[tokio::test(start_paused = true)]
    async fn follows_a_newer_version_until_its_release_can_be_downloaded() -> Result<()> {
        let newer = v("99.0.0")?;
        let (tx, rx) = watch::channel(None);
        let tries = Tries::default();
        let missing = || Err(Miss::Missing("not published yet".into()));
        let answers = vec![missing(), missing(), missing(), Ok(release(newer))];
        let started = tokio::time::Instant::now();
        let following = tokio::spawn(follow(rx, Duration::ZERO, fetcher(&tries, answers)));
        // Nothing newer to update to: it waits.
        tokio::time::sleep(Duration::from_secs(3600)).await;
        let ours = v(VERSION)?;
        tx.send_replace(Some(ours));
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert!(tries.lock().is_ok_and(|t| t.is_empty()));
        // The control plane runs a newer one.
        let heard = tokio::time::Instant::now();
        tx.send_replace(Some(newer));
        let got = following.await?;
        assert_eq!(got, release(newer));
        let tries = tries.lock().map(|t| t.clone()).unwrap_or_default();
        assert_eq!(tries.len(), 4);
        assert!(tries.iter().all(|(version, _)| *version == newer));
        // Within half a minute, then about every minute.
        let seconds = Duration::from_secs;
        assert!(tries[0].1.duration_since(heard) <= SPREAD + seconds(1));
        for pair in tries.windows(2) {
            let gap = pair[1].1.duration_since(pair[0].1);
            assert!((seconds(53)..=seconds(67)).contains(&gap), "{gap:?}");
        }
        assert!(started.elapsed() < Duration::from_secs(2 * 3600 + 300));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_version_meanwhile_starts_over_and_unusable_downloads_wait_longer() -> Result<()>
    {
        let (first, second) = (v("99.0.0")?, v("99.0.1")?);
        let (tx, rx) = watch::channel(Some(first));
        let tries = Tries::default();
        let unusable = || Err(Miss::Unusable("checksum mismatch".into()));
        let answers = vec![unusable(), unusable(), Ok(release(second))];
        let following = tokio::spawn(follow(rx, Duration::ZERO, fetcher(&tries, answers)));
        // Two unusable downloads of the first: it waits 10, then 20 minutes.
        tokio::time::sleep(Duration::from_secs(25 * 60)).await;
        assert_eq!(tries.lock().map(|t| t.len()).unwrap_or_default(), 2);
        tx.send_replace(Some(second));
        let got = following.await?;
        assert_eq!(got.version, second);
        let tries = tries.lock().map(|t| t.clone()).unwrap_or_default();
        let minutes = |m: u64| Duration::from_secs(m * 60);
        let gap = tries[1].1.duration_since(tries[0].1);
        assert!(
            (minutes(9)..=minutes(11) + minutes(1) / 60).contains(&gap),
            "{gap:?}"
        );
        // The newer one is tried soon, not after the 18 to 22 minutes.
        assert_eq!(tries[2].0, second);
        assert!(tries[2].1.duration_since(tries[1].1) < minutes(17));
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn an_install_that_failed_holds_off_the_next_try() -> Result<()> {
        let newer = v("99.0.0")?;
        let (_tx, rx) = watch::channel(Some(newer));
        let tries = Tries::default();
        let started = tokio::time::Instant::now();
        let hold_off = Duration::from_secs(10 * 60);
        let got = follow(rx, hold_off, fetcher(&tries, vec![Ok(release(newer))])).await;
        assert_eq!(got.version, newer);
        let at = tries.lock().map(|t| t[0].1).unwrap_or(started);
        assert!(at.duration_since(started) >= hold_off);
        assert!(at.duration_since(started) <= hold_off + SPREAD + Duration::from_secs(1));
        Ok(())
    }

    #[tokio::test]
    async fn downloads_check_the_archive_and_the_new_fctl() -> Result<()> {
        // A release server with one release, whose fctl says its version.
        let dir = tempfile::tempdir()?;
        let script = b"#!/bin/sh\necho fctl 99.0.0\n";
        let target = "x86_64-unknown-linux-musl";
        let good = archive(&[("fctl", script), ("fctl-agent-x86_64-linux", b"agent")])?;
        let sum = hex::encode(Sha256::digest(&good));
        let server = serve(vec![
            (
                "/v99.0.0/fctl-x86_64-unknown-linux-musl.tar.gz".into(),
                good.clone(),
            ),
            (
                "/v99.0.0/fctl-x86_64-unknown-linux-musl.tar.gz.sha256".into(),
                format!("{sum}  fctl-x86_64-unknown-linux-musl.tar.gz\n").into_bytes(),
            ),
            (
                "/v99.0.1/fctl-x86_64-unknown-linux-musl.tar.gz".into(),
                good.clone(),
            ),
            (
                "/v99.0.1/fctl-x86_64-unknown-linux-musl.tar.gz.sha256".into(),
                format!("{}  x\n", "0".repeat(64)).into_bytes(),
            ),
            (
                "/v99.0.2/fctl-x86_64-unknown-linux-musl.tar.gz".into(),
                good.clone(),
            ),
            (
                "/v99.0.2/fctl-x86_64-unknown-linux-musl.tar.gz.sha256".into(),
                format!("{sum}  x\n").into_bytes(),
            ),
        ])
        .await?;
        let got = download(&server, v("99.0.0")?, target, dir.path()).await;
        let Ok(release) = got else {
            anyhow::bail!("{got:?}");
        };
        assert_eq!(release.files.len(), 2);
        // The copy it tried is gone.
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        // Not published yet.
        let got = download(&server, v("99.1.0")?, target, dir.path()).await;
        assert!(
            matches!(&got, Err(Miss::Missing(why)) if why.ends_with("has no fctl 99.1.0 for x86_64-unknown-linux-musl")),
            "{got:?}"
        );
        // A checksum that doesn't match.
        let got = download(&server, v("99.0.1")?, target, dir.path()).await;
        assert!(
            matches!(&got, Err(Miss::Unusable(why)) if why.contains("checksum mismatch")),
            "{got:?}"
        );
        // An fctl that says it is another version.
        let got = download(&server, v("99.0.2")?, target, dir.path()).await;
        assert!(
            matches!(&got, Err(Miss::Unusable(why)) if why.contains("says it is fctl 99.0.0")),
            "{got:?}"
        );
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 0);
        Ok(())
    }

    /// Serves `files` over plain HTTP on this machine, 404 for anything
    /// else, and returns its address.
    async fn serve(files: Vec<(String, Vec<u8>)>) -> Result<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let files = Arc::new(files);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let files = files.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buf = [0u8; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    let text = String::from_utf8_lossy(&request);
                    let path = text.split_whitespace().nth(1).unwrap_or_default();
                    let (status, body) = match files.iter().find(|(p, _)| p == path) {
                        Some((_, body)) => ("200 OK", body.clone()),
                        None => ("404 Not Found", b"Not found\n".to_vec()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        Ok(format!("http://{addr}"))
    }
}

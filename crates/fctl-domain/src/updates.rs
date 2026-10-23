//! Updating fctl: versions, the release a machine downloads, and when a
//! host that updates itself tries again.
//!
//! A host updates itself to the version its control plane runs, once that
//! is newer than its own, unless its `host.toml` turns that off (ADR 0051).
//! On app.fctl.sh that is the latest release, since each release redeploys
//! it. The control plane can run a release some minutes before its archives
//! can be downloaded, though: CI deploys the release commit while the
//! Release workflow still builds them. A host that finds the release
//! missing tries again every minute ([`retry_in`]).

use std::fmt;
use std::time::Duration;

/// A release's version, `0.21.0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    /// `0.21.0`, with or without a leading `v`: three numbers and nothing
    /// else, as releases are numbered.
    pub fn parse(v: &str) -> Option<Self> {
        let mut parts = v.trim().strip_prefix('v').unwrap_or(v.trim()).split('.');
        let mut number = || {
            parts
                .next()
                .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|p| p.parse().ok())
        };
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The version a host of version `ours` updates to when its control plane
/// runs `theirs`: that one when it is newer, never an older one.
pub fn update_to(ours: &str, theirs: &str) -> Option<Version> {
    let theirs = Version::parse(theirs)?;
    (Version::parse(ours)? < theirs).then_some(theirs)
}

/// The release target for an OS and architecture, as the installer names
/// them. An x86_64 build that runs under Rosetta on Apple silicon
/// (`translated`) gets the native one.
pub fn target(os: &str, arch: &str, translated: bool) -> Result<String, String> {
    let os = match os {
        "macos" => "apple-darwin",
        "linux" => "unknown-linux-musl",
        other => {
            return Err(format!(
                "fctl releases are built for macOS and Linux, not {other}"
            ));
        }
    };
    let arch = match arch {
        "x86_64" if translated => "aarch64",
        "aarch64" | "x86_64" => arch,
        other => {
            return Err(format!(
                "fctl releases are built for aarch64 and x86_64, not {other}"
            ));
        }
    };
    Ok(format!("{arch}-{os}"))
}

/// The archive of `target` in a release: fctl, and the Linux agent of its
/// architecture for the sandboxes.
pub fn archive(target: &str) -> String {
    format!("fctl-{target}.tar.gz")
}

/// The Linux agent in `target`'s archive, which fctl looks for next to
/// itself.
pub fn agent(target: &str) -> String {
    let arch = target.split('-').next().unwrap_or("aarch64");
    format!("fctl-agent-{arch}-linux")
}

/// Why a host didn't update to a release this time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss {
    /// The release can't be downloaded: not yet (it isn't published, or
    /// not everywhere yet), or the server or the network failed.
    Missing(String),
    /// What came is no use: a checksum that doesn't match, an archive
    /// without the files it should have, or a new fctl that doesn't run on
    /// this machine.
    Unusable(String),
}

impl fmt::Display for Miss {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(why) | Self::Unusable(why) => f.write_str(why),
        }
    }
}

/// While a release is missing, a host tries again every minute at first.
pub const MISSING_EVERY: Duration = Duration::from_secs(60);
/// For how long it tries every minute: releases come within half an hour
/// of the version the control plane runs.
pub const MISSING_FOR: Duration = Duration::from_secs(60 * 60);
/// How often it tries after that, for a control plane that runs a version
/// its download server doesn't have, such as a build of its own.
pub const MISSING_LATER: Duration = Duration::from_secs(30 * 60);
/// After the first unusable download, it waits this long, twice as long
/// after each further one, up to [`UNUSABLE_AT_MOST`].
pub const UNUSABLE_FIRST: Duration = Duration::from_secs(10 * 60);
pub const UNUSABLE_AT_MOST: Duration = Duration::from_secs(12 * 60 * 60);

/// How long a host waits before it tries a release again after `miss`,
/// having first tried it `waited` ago, when `unusable` downloads of it
/// (this one included) were of no use.
pub fn retry_in(miss: &Miss, waited: Duration, unusable: u32) -> Duration {
    match miss {
        Miss::Missing(_) if waited < MISSING_FOR => MISSING_EVERY,
        Miss::Missing(_) => MISSING_LATER,
        Miss::Unusable(_) => {
            let doublings = unusable.saturating_sub(1).min(16);
            UNUSABLE_FIRST
                .saturating_mul(1 << doublings)
                .min(UNUSABLE_AT_MOST)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u64, minor: u64, patch: u64) -> Version {
        Version {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn versions_are_three_numbers() {
        assert_eq!(Version::parse("0.21.0"), Some(v(0, 21, 0)));
        assert_eq!(Version::parse(" v1.10.2\n"), Some(v(1, 10, 2)));
        assert_eq!(v(1, 10, 2).to_string(), "1.10.2");
        for bad in [
            "",
            "latest",
            "1.2",
            "1.2.3.4",
            "1.2.3-rc.1",
            "1..3",
            "+1.2.3",
            "1.2.-3",
            "v",
            "1.2.x",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(v(1, 10, 0) > v(1, 9, 9));
        assert!(v(0, 22, 0) > v(0, 21, 7));
        assert!(v(1, 0, 0) > v(0, 99, 99));
    }

    #[test]
    fn hosts_update_only_to_a_newer_version() {
        assert_eq!(update_to("0.21.0", "0.22.0"), Some(v(0, 22, 0)));
        assert_eq!(update_to("0.21.0", "0.21.1"), Some(v(0, 21, 1)));
        assert_eq!(update_to("0.21.0", "0.21.0"), None);
        // A control plane that went back to an older version.
        assert_eq!(update_to("0.22.0", "0.21.0"), None);
        // One that says something else, or a host of no release.
        assert_eq!(update_to("0.21.0", "0.22.0/../../evil"), None);
        assert_eq!(update_to("dev", "0.22.0"), None);
    }

    #[test]
    fn targets_match_the_installer() -> Result<(), String> {
        assert_eq!(target("macos", "aarch64", false)?, "aarch64-apple-darwin");
        assert_eq!(target("macos", "x86_64", true)?, "aarch64-apple-darwin");
        assert_eq!(target("macos", "x86_64", false)?, "x86_64-apple-darwin");
        assert_eq!(
            target("linux", "x86_64", false)?,
            "x86_64-unknown-linux-musl"
        );
        assert!(target("windows", "x86_64", false).is_err());
        assert!(target("linux", "riscv64", false).is_err());
        assert_eq!(
            archive("aarch64-apple-darwin"),
            "fctl-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(agent("aarch64-apple-darwin"), "fctl-agent-aarch64-linux");
        assert_eq!(
            agent("x86_64-unknown-linux-musl"),
            "fctl-agent-x86_64-linux"
        );
        Ok(())
    }

    #[test]
    fn a_missing_release_is_tried_every_minute_for_an_hour_then_every_half_hour() {
        let missing = Miss::Missing("not published yet".into());
        let minutes = |m: u64| Duration::from_secs(m * 60);
        assert_eq!(retry_in(&missing, minutes(0), 0), minutes(1));
        assert_eq!(retry_in(&missing, minutes(59), 0), minutes(1));
        assert_eq!(retry_in(&missing, minutes(60), 0), minutes(30));
        assert_eq!(retry_in(&missing, minutes(60 * 24 * 30), 0), minutes(30));
    }

    #[test]
    fn unusable_downloads_are_tried_less_and_less_often() {
        let unusable = Miss::Unusable("checksum mismatch".into());
        let minutes = |m: u64| Duration::from_secs(m * 60);
        let waits: Vec<Duration> = (1..=8)
            .map(|n| retry_in(&unusable, minutes(5), n))
            .collect();
        assert_eq!(
            waits,
            [10, 20, 40, 80, 160, 320, 640, 720].map(minutes).to_vec()
        );
        assert_eq!(retry_in(&unusable, minutes(5), u32::MAX), minutes(720));
    }
}

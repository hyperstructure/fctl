//! What `docker pull` and Apple's `container image pull` print as they
//! run, read for the log: how far along a pull is, whether it reached the
//! image's content, and why it failed.
//!
//! A sandbox image is over a gigabyte. On 2026-11-10 a runner's pulls of
//! one crawled at about 100 KB/s for ten minutes before ghcr.io's HTTP/2
//! stream broke off, while a pull started by hand beside them took a
//! fraction of that: sessions said only "Pulling …" all along, and their
//! failure carried every progress line the CLI had printed.

use std::collections::BTreeSet;
use std::time::Duration;

/// How often a pull says in the log how far along it is.
pub const PROGRESS_EVERY: Duration = Duration::from_secs(30);

/// How many times a pull whose download broke off is tried, and how long
/// it waits before the second try (three times longer before each one
/// after). A new try opens a new connection, and keeps the blobs that
/// came.
pub const PULL_TRIES: u32 = 3;
pub const FIRST_WAIT: Duration = Duration::from_secs(5);

/// What a pull printed so far.
#[derive(Debug, Default)]
pub struct PullProgress {
    /// Apple's latest progress line without its step, `Fetching image 23%
    /// (6 of 7 blobs, 290,1 MB/1,25 GB, 150 KB/s) [3m 12s]`.
    latest: Option<String>,
    /// Docker's layers, and those of them it has.
    layers: BTreeSet<String>,
    done: BTreeSet<String>,
    /// Whether it reached the image's content: then the registry has the
    /// image, and a failure broke off its download.
    pub fetching: bool,
}

impl PullProgress {
    /// Reads a line the pull printed; whether it said how far along the
    /// pull is, rather than something to tell when it fails.
    pub fn read(&mut self, line: &str) -> bool {
        let line = line.trim();
        if let Some(rest) = apple_step(line) {
            // Its count of blobs comes with the image's manifest.
            self.fetching |= rest.contains(" blobs");
            self.latest = Some(rest.to_owned());
            return true;
        }
        if let Some((layer, status)) = docker_layer(line) {
            self.fetching = true;
            self.layers.insert(layer.to_owned());
            if matches!(status, "Pull complete" | "Already exists") {
                self.done.insert(layer.to_owned());
            }
            return true;
        }
        false
    }

    /// How far along the pull is, once it said.
    pub fn summary(&self) -> Option<String> {
        if let Some(latest) = &self.latest {
            return Some(latest.clone());
        }
        (!self.layers.is_empty())
            .then(|| format!("{} of {} layers pulled", self.done.len(), self.layers.len()))
    }
}

/// Apple's progress line without its step: `[1/2] Fetching image 23% (…)`.
fn apple_step(line: &str) -> Option<&str> {
    let (step, rest) = line.strip_prefix('[')?.split_once("] ")?;
    let (n, of) = step.split_once('/')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(n) && digits(of)).then_some(rest)
}

/// A layer's status as Docker prints it without a terminal: `f5d23c7fed46:
/// Pull complete`.
fn docker_layer(line: &str) -> Option<(&str, &str)> {
    let (layer, status) = line.split_once(": ")?;
    let hex = layer.len() == 12 && layer.bytes().all(|b| b.is_ascii_hexdigit());
    hex.then_some((layer, status))
}

/// How long a pull took, as the log says it: `45 s`, `3 min 12 s`.
pub fn took(elapsed: Duration) -> String {
    let s = elapsed.as_secs();
    match (s / 60, s % 60) {
        (0, s) => format!("{s} s"),
        (m, 0) => format!("{m} min"),
        (m, s) => format!("{m} min {s} s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_how_far_apples_pulls_are() {
        let mut p = PullProgress::default();
        assert!(p.read("[1/2] Fetching image [0s]"));
        assert!(!p.fetching, "no manifest yet");
        assert_eq!(p.summary().as_deref(), Some("Fetching image [0s]"));
        let line = "[1/2] Fetching image 3% (5 of 7 blobs, 42,1 MB/1,25 GB, 185 KB/s) [24s]";
        assert!(p.read(line));
        assert!(p.fetching);
        assert_eq!(
            p.summary().as_deref(),
            Some("Fetching image 3% (5 of 7 blobs, 42,1 MB/1,25 GB, 185 KB/s) [24s]")
        );
        let error = "Error: StreamClosed(streamID: HTTP2StreamID(7), errorCode: HTTP2ErrorCode<0x1 ProtocolError>, location: \"NIOHTTP2/HTTP2StreamChannel.swift:892\")";
        assert!(!p.read(error));

        // An image the registry doesn't have never gets to its blobs.
        let mut p = PullProgress::default();
        p.read("[1/2] Fetching image [0s]");
        assert!(!p.read("Error: notFound: \"ghcr.io/acme/agent:1\""));
        assert!(!p.fetching);
    }

    #[test]
    fn reads_how_far_dockers_pulls_are() {
        let mut p = PullProgress::default();
        assert!(!p.read("0.33.2: Pulling from hyperstructure/sandbox"));
        assert_eq!(p.summary(), None);
        for line in [
            "f5d23c7fed46: Already exists",
            "9a3b1c2d4e5f: Pulling fs layer",
            "0123456789ab: Pulling fs layer",
            "9a3b1c2d4e5f: Download complete",
            "9a3b1c2d4e5f: Pull complete",
        ] {
            assert!(p.read(line), "{line}");
        }
        assert!(p.fetching);
        assert_eq!(p.summary().as_deref(), Some("2 of 3 layers pulled"));
        assert!(!p.read("Error response from daemon: manifest unknown"));
        assert!(!p.read("Digest: sha256:0123"));
    }

    #[test]
    fn says_how_long_pulls_took() {
        assert_eq!(took(Duration::from_millis(45_400)), "45 s");
        assert_eq!(took(Duration::from_secs(180)), "3 min");
        assert_eq!(took(Duration::from_secs(192)), "3 min 12 s");
    }
}

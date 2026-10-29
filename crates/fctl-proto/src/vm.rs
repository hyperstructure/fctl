//! Host ⇄ the init of a Firecracker microVM (`fctl-agent` as the VM's PID 1),
//! over the VM's vsock.
//!
//! Firecracker puts a VM's vsock on the host as a unix socket. The host
//! connects to it, asks for [`PORT`] (`CONNECT 1024\n`, which Firecracker
//! answers with `OK <port>\n`) and sends one [`Request`], a line of JSON.
//! The init answers with a [`Reply`] line. After an [`Request::Exec`] it
//! accepted, the connection carries [`frame`]s both ways until the program
//! exits; after an [`Request::Unpack`], the layers' bytes one after the
//! other, then a second [`Reply`].
//!
//! The VM itself reaches the host's [`NET_PORT`], where the host's `passt`
//! takes its network: Ethernet frames, each after its length (4 bytes, big
//! endian), as passt reads them from QEMU.
//!
//! A host may be one version ahead of the init of a VM that was running
//! when it updated, so requests only gain optional fields, and an init
//! answers a request it doesn't know with an error.

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

/// The vsock port the init serves requests on.
pub const PORT: u32 = 1024;

/// The vsock port of the host's network for the VM (`passt`).
pub const NET_PORT: u32 = 1025;

/// The VM's network, as the host's `passt` gives it: the VM's address and
/// prefix, the gateway, and the address that forwards DNS queries to the
/// host's resolver.
pub const ADDRESS: [u8; 4] = [10, 0, 2, 15];
pub const PREFIX: u8 = 24;
pub const GATEWAY: [u8; 4] = [10, 0, 2, 2];
pub const DNS: [u8; 4] = [10, 0, 2, 3];
/// passt's MTU: large frames carry more per trip through the vsock.
pub const MTU: u32 = 65520;

/// The longest request or reply line either side reads.
pub const MAX_LINE: usize = 1024 * 1024;

/// What a VM boots to, from the kernel's command line (`fctl.mode=`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// A session's sandbox: the image's root filesystem (`/dev/vda`, read
    /// only) under the sandbox's own disk (`/dev/vdb`).
    Sandbox,
    /// Unpacks an image's layers onto `/dev/vda`, then powers off.
    Unpack,
}

/// What the init reads from the kernel's command line: `fctl.mode=`,
/// `fctl.name=` (the VM's hostname) and `fctl.time=` (the host's clock at
/// boot, in milliseconds since the Unix epoch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boot {
    pub mode: Mode,
    pub name: String,
    pub time_ms: Option<i64>,
}

impl Boot {
    /// The parameters for the kernel's command line.
    pub fn params(&self) -> String {
        let mode = match self.mode {
            Mode::Sandbox => "sandbox",
            Mode::Unpack => "unpack",
        };
        let mut p = format!("fctl.mode={mode} fctl.name={}", self.name);
        if let Some(t) = self.time_ms {
            p.push_str(&format!(" fctl.time={t}"));
        }
        p
    }

    /// Reads them back from `/proc/cmdline`. `None` without `fctl.mode`.
    pub fn parse(cmdline: &str) -> Option<Self> {
        let mut mode = None;
        let mut name = String::from("fctl");
        let mut time_ms = None;
        for word in cmdline.split_whitespace() {
            match word.split_once('=') {
                Some(("fctl.mode", "sandbox")) => mode = Some(Mode::Sandbox),
                Some(("fctl.mode", "unpack")) => mode = Some(Mode::Unpack),
                Some(("fctl.name", n)) if !n.is_empty() => n.clone_into(&mut name),
                Some(("fctl.time", t)) => time_ms = t.parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            mode: mode?,
            name,
            time_ms,
        })
    }
}

/// What the host asks the init.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Whether the init is up; the reply says its version.
    Hello {
        /// The host's clock, which the VM's follows.
        #[serde(default)]
        now_ms: Option<i64>,
    },
    /// Runs a program.
    Exec(Exec),
    /// Unpacks an image's layers, which follow, onto the VM's disk (only in
    /// a VM booted to [`Mode::Unpack`]).
    Unpack { layers: Vec<Layer> },
    /// Stops every program, flushes the disks and powers the VM off.
    Shutdown,
}

/// A program to run, as `docker exec` runs one: as the image's user, with
/// its environment and in its working directory, unless the request says
/// otherwise.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Exec {
    pub argv: Vec<String>,
    /// `name`, `uid`, `name:group` or `uid:gid`. Unset: root.
    #[serde(default)]
    pub user: Option<String>,
    /// `KEY=value`: the image's environment. The init adds `HOME` (the
    /// user's), `HOSTNAME` and a `PATH` when they are missing.
    #[serde(default)]
    pub env: Vec<String>,
    /// Unset: `/`, as for an image without a working directory.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Whether the host sends the program's input ([`frame::STDIN`]);
    /// without it, the program reads nothing.
    #[serde(default)]
    pub stdin: bool,
    /// Starts it in the background, with no input or output, and answers at
    /// once.
    #[serde(default)]
    pub detach: bool,
    /// The host's clock, which the VM's follows.
    #[serde(default)]
    pub now_ms: Option<i64>,
}

/// A layer of an image, as its manifest describes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    /// `application/vnd.oci.image.layer.v1.tar+gzip`, or Docker's.
    pub media_type: String,
    /// Bytes, as they follow the request.
    pub size: u64,
}

/// The init's answer.
#[skip_serializing_none]
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    /// Why the request failed; absent when it didn't.
    #[serde(default)]
    pub error: Option<String>,
    /// The init's version of fctl (to `hello`).
    #[serde(default)]
    pub version: Option<String>,
    /// What unpacking wrote (its second reply): files and bytes.
    #[serde(default)]
    pub files: Option<u64>,
    #[serde(default)]
    pub bytes: Option<u64>,
}

impl Reply {
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

/// The frames a program's input and output travel in: a kind (1 byte), a
/// length (4 bytes, big endian) and that many bytes.
pub mod frame {
    /// Host → VM: input for the program.
    pub const STDIN: u8 = 0;
    /// Host → VM: the end of its input.
    pub const STDIN_END: u8 = 1;
    /// VM → host: what it writes to its standard output.
    pub const STDOUT: u8 = 2;
    /// VM → host: what it writes to its standard error.
    pub const STDERR: u8 = 3;
    /// VM → host: its exit code (4 bytes, big endian, signed): 128 and the
    /// signal's number for one a signal ended. The last frame.
    pub const EXIT: u8 = 4;

    /// The most bytes one frame carries.
    pub const MAX: usize = 1024 * 1024;

    /// A frame's first five bytes.
    pub fn header(kind: u8, len: usize) -> [u8; 5] {
        let len = u32::try_from(len.min(MAX))
            .unwrap_or(u32::MAX)
            .to_be_bytes();
        [kind, len[0], len[1], len[2], len[3]]
    }

    /// The kind and length in a frame's first five bytes; `None` for a
    /// length over [`MAX`].
    pub fn parse(header: [u8; 5]) -> Option<(u8, usize)> {
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        (len <= MAX).then_some((header[0], len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_parameters_round_trip_through_the_command_line() {
        let boot = Boot {
            mode: Mode::Sandbox,
            name: "fctl-ses-01abc".into(),
            time_ms: Some(1_761_700_000_000),
        };
        let cmdline = format!(
            "console=ttyS0 reboot=k panic=1 {} rdinit=/init",
            boot.params()
        );
        assert_eq!(Boot::parse(&cmdline), Some(boot));
        let unpack = Boot::parse("fctl.mode=unpack quiet");
        assert_eq!(
            unpack.map(|b| (b.mode, b.name)),
            Some((Mode::Unpack, "fctl".into()))
        );
        assert_eq!(Boot::parse("console=ttyS0 root=/dev/vda"), None);
    }

    #[test]
    fn requests_are_tagged_and_take_missing_fields() -> serde_json::Result<()> {
        let exec = Request::Exec(Exec {
            argv: vec!["sh".into(), "-c".into(), "true".into()],
            user: Some("agent".into()),
            ..Exec::default()
        });
        let line = serde_json::to_string(&exec)?;
        assert_eq!(
            line,
            r#"{"type":"exec","argv":["sh","-c","true"],"user":"agent","env":[],"stdin":false,"detach":false}"#
        );
        assert_eq!(serde_json::from_str::<Request>(&line)?, exec);
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"type":"exec","argv":["true"]}"#)?,
            Request::Exec(Exec {
                argv: vec!["true".into()],
                ..Exec::default()
            })
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"type":"hello"}"#)?,
            Request::Hello { now_ms: None }
        );
        // An init that doesn't know a request says so.
        assert!(serde_json::from_str::<Request>(r#"{"type":"snapshot"}"#).is_err());
        assert_eq!(serde_json::to_string(&Reply::default())?, "{}");
        Ok(())
    }

    #[test]
    fn frames_say_their_kind_and_length() {
        let h = frame::header(frame::STDOUT, 300);
        assert_eq!(h, [2, 0, 0, 1, 44]);
        assert_eq!(frame::parse(h), Some((frame::STDOUT, 300)));
        assert_eq!(frame::parse([2, 0xff, 0xff, 0xff, 0xff]), None);
    }
}

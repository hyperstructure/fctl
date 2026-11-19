//! Apple's `container`: sandboxes whose VMs stopped answering, which the
//! host stops so that the runtime runs the other sandboxes' commands again.
//!
//! Apple's `container` runs each sandbox's VM from a helper process of its
//! own (`container-runtime-linux`), which takes one command at a time, and
//! its API server starts every sandbox, and every `exec` in any of them,
//! one at a time. Neither gives up on a VM that doesn't answer, as one
//! whose memory ran out may not: an `exec` in it waits for ever, and with
//! it every `run`, `start` and `exec` on the Mac, while `list`, `inspect`
//! and `system status` answer as ever. Killing the `container` command
//! that waits frees nothing, and `container stop` waits for the helper
//! too ([apple/container#1916]). The helper's end frees them: the API
//! server's call to it fails, and it takes the sandbox for stopped.
//!
//! So when a `container` command that waits on a VM ([`waits_on_vms`])
//! runs out of time, or `container run` takes longer than [`SLOW_START`],
//! the host asks each running sandbox for its numbers (`container stats`),
//! which wait on its helper and its VM, not on the API server, and kills
//! the helper of each of its own sandboxes that doesn't answer within
//! [`STUCK_AFTER`]. It leaves the others, which aren't its own, and says
//! so in its log.
//!
//! [apple/container#1916]: https://github.com/apple/container/issues/1916

use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long a running sandbox's VM may take to answer `container stats`
/// before the host takes it for stuck. A VM answers within a second
/// however busy it is, and one whose memory has run out may never.
pub const STUCK_AFTER: Duration = Duration::from_secs(2 * 60);

/// How long `container run` may take before the host looks for stuck VMs
/// that hold it up, and how often it looks again while it runs. Unpacking
/// an image the first time takes a minute or two.
pub const SLOW_START: Duration = Duration::from_secs(3 * 60);

/// How long what the host found when it last looked stands for commands
/// that ran out of time since: commands that run out together look once.
pub const FRESH_FOR: Duration = Duration::from_secs(30);

/// The program that runs each sandbox's VM in Apple's `container`.
const HELPER: &str = "container-runtime-linux";

/// What the host knows of its sandboxes' stuck VMs.
#[derive(Default)]
pub(crate) struct Stuck {
    /// When the host last finished looking for them, held while it looks:
    /// commands that run out of time meanwhile wait for what it finds.
    pub looked: tokio::sync::Mutex<Option<Instant>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The sandboxes the host stopped for not answering, until they start
    /// again or go.
    stopped: HashSet<String>,
    /// When it last stopped one.
    last: Option<Instant>,
    /// Why Apple's `container` runs no commands, as the host last found:
    /// VMs that don't answer, which it doesn't stop.
    held_up: Option<String>,
}

impl Stuck {
    /// Whether the host stopped the sandbox `name` for not answering, since
    /// it last started.
    pub fn stopped(&self, name: &str) -> bool {
        self.state().stopped.contains(name)
    }

    /// Whether the host stopped a sandbox for not answering since `at`:
    /// the commands that its VM held up may go through now.
    pub fn stopped_since(&self, at: Instant) -> bool {
        self.state().last.is_some_and(|last| last >= at)
    }

    /// The sandbox `name` stopped for not answering.
    pub fn note(&self, name: &str) {
        let mut state = self.state();
        state.stopped.insert(name.to_owned());
        state.last = Some(Instant::now());
    }

    /// The sandbox `name` starts again, or goes.
    pub fn forget(&self, name: &str) {
        self.state().stopped.remove(name);
    }

    /// Why Apple's `container` runs no commands, as the host last found.
    pub fn held_up(&self) -> Option<String> {
        self.state().held_up.clone()
    }

    /// What the host found: why Apple's `container` runs no commands, or
    /// that nothing holds it up.
    pub fn hold_up(&self, why: Option<String>) {
        self.state().held_up = why;
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether `container` with `args` waits on a sandbox's VM, or on the API
/// server that a stuck VM holds up: one that runs out of time may be a
/// stuck VM's doing. `list`, `inspect`, `stats`, images and `system` don't.
pub(crate) fn waits_on_vms(args: &[&str]) -> bool {
    matches!(
        args.first().copied(),
        Some("exec" | "run" | "create" | "start" | "stop" | "kill" | "delete" | "cp" | "clean")
    )
}

/// The running containers in `container list --format json`: fctl's
/// sandboxes and any other.
pub(crate) fn running_in(json: &str) -> Vec<String> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|c| {
            c.pointer("/status/state")
                .or_else(|| c.get("status"))
                .and_then(Value::as_str)
                == Some("running")
        })
        .filter_map(|c| {
            c.get("id")
                .or_else(|| c.pointer("/configuration/id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

/// The processes in `ps -A -ww -o pid= -o command=` that run the VM of the
/// sandbox `name`: `container-runtime-linux start --root … --uuid NAME`.
pub(crate) fn helpers_of(ps: &str, name: &str) -> Vec<i32> {
    ps.lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(char::is_whitespace)?;
            let pid = pid.parse::<i32>().ok().filter(|p| *p > 1)?;
            let words: Vec<&str> = command.split_whitespace().collect();
            let helper = words.iter().any(|w| w.rsplit('/').next() == Some(HELPER));
            let its = words.windows(2).any(|w| w[0] == "--uuid" && w[1] == name);
            (helper && its).then_some(pid)
        })
        .collect()
}

/// What came of the signals the host sent the processes of a VM that
/// doesn't answer: those it stopped, or that had gone already, and why it
/// couldn't stop the others.
pub(crate) fn stopped_of(sent: Vec<(i32, nix::Result<()>)>) -> (Vec<i32>, Vec<String>) {
    let (mut stopped, mut failed) = (Vec::new(), Vec::new());
    for (pid, sent) in sent {
        match sent {
            Ok(()) | Err(nix::errno::Errno::ESRCH) => stopped.push(pid),
            Err(e) => failed.push(format!("pid {pid}: {e}")),
        }
    }
    (stopped, failed)
}

/// What the host's log says of the sandbox `name`, whose VM didn't answer
/// within `after` and which it stopped by killing `pids`.
pub(crate) fn stopped_line(name: &str, after: Duration, pids: &[i32]) -> String {
    let pids = pids
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{name}'s VM did not answer within {}, so the host stopped it (killed its {HELPER}, pid {pids}): one VM that doesn't answer holds up every sandbox of Apple's `container`",
        words(after)
    )
}

/// `2 minutes`, `30 seconds`, `500 ms`.
pub(crate) fn words(d: Duration) -> String {
    match d.as_secs() {
        0 => format!("{} ms", d.as_millis()),
        1 => "a second".into(),
        60 => "a minute".into(),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_that_wait_on_vms_are_known() {
        for args in [
            &["exec", "fctl-ses-1", "true"][..],
            &["run", "-d", "--name", "fctl-ses-1"],
            &["start", "fctl-ses-1"],
            &["stop", "-t", "20", "fctl-ses-1"],
            &["delete", "--force", "fctl-ses-1"],
            &[
                "cp",
                "fctl-agent",
                "fctl-ses-1:/opt/fctl/bin/fctl-agent.new",
            ],
            &["clean", "fctl-ses-1"],
        ] {
            assert!(waits_on_vms(args), "{args:?}");
        }
        for args in [
            &["list", "--all", "--format", "json"][..],
            &["inspect", "fctl-ses-1"],
            &["stats", "--no-stream", "--format", "json", "fctl-ses-1"],
            &["image", "pull", "ghcr.io/hyperstructure/sandbox:0.36.0"],
            &["system", "status"],
            &[],
        ] {
            assert!(!waits_on_vms(args), "{args:?}");
        }
    }

    #[test]
    fn running_containers_are_read_from_the_list() {
        // As Apple's `container` 1.5 lists them, and before (`status` alone).
        let json = r#"[
            {"id": "fctl-ses-1", "configuration": {"id": "fctl-ses-1"}, "status": {"state": "running"}},
            {"configuration": {"id": "fctl-ses-2"}, "status": "running"},
            {"configuration": {"id": "fctl-ses-3"}, "status": "stopped"},
            {"configuration": {"id": "buildkit"}, "status": {"state": "running"}}
        ]"#;
        assert_eq!(running_in(json), ["fctl-ses-1", "fctl-ses-2", "buildkit"]);
        assert!(running_in("[]").is_empty());
        assert!(running_in("Error: XPC connection error").is_empty());
    }

    #[test]
    fn a_sandboxs_vm_is_found_by_its_helper() {
        // `ps` on a Mac whose sandbox stopped answering, with a stand-in
        // for the helper that `sh` runs, as in tests.
        let ps = "    1 /sbin/launchd
  1568 /usr/local/bin/container-apiserver start
 72625 /usr/local/libexec/container/plugins/container-runtime-linux/bin/container-runtime-linux start --root /Users/kalaspuff/Library/Application Support/com.apple.container/containers/fctl-ses-01m7rm7hkxza3q8xpzzbx1prda --uuid fctl-ses-01m7rm7hkxza3q8xpzzbx1prda
 72700 /usr/local/libexec/container/plugins/container-runtime-linux/bin/container-runtime-linux start --root /x/fctl-ses-01m7rm7hkxza3q8xpzzbx1prda2 --uuid fctl-ses-01m7rm7hkxza3q8xpzzbx1prda2
 79316 container run -d --name fctl-ses-01m7s5pg8bs3eahhs1dq4a6sge --label sh.fctl.sandbox=1
 80000 grep --uuid fctl-ses-01m7rm7hkxza3q8xpzzbx1prda
 80001 /bin/sh /tmp/t/container-runtime-linux start --root /tmp/t/r --uuid fctl-ses-2
";
        assert_eq!(
            helpers_of(ps, "fctl-ses-01m7rm7hkxza3q8xpzzbx1prda"),
            [72625]
        );
        assert_eq!(helpers_of(ps, "fctl-ses-2"), [80001]);
        assert!(helpers_of(ps, "fctl-ses-01m7s5pg8bs3eahhs1dq4a6sge").is_empty());
        assert!(helpers_of(ps, "fctl-ses-3").is_empty());
        assert!(helpers_of("", "fctl-ses-2").is_empty());
    }

    #[test]
    fn only_processes_that_went_count_as_stopped() {
        use nix::errno::Errno;
        assert_eq!(
            stopped_of(vec![
                (72625, Ok(())),
                (72626, Err(Errno::ESRCH)),
                (72627, Err(Errno::EPERM))
            ]),
            (
                vec![72625, 72626],
                vec!["pid 72627: EPERM: Operation not permitted".to_owned()]
            )
        );
        assert_eq!(
            stopped_of(vec![(72627, Err(Errno::EPERM))]),
            (
                vec![],
                vec!["pid 72627: EPERM: Operation not permitted".to_owned()]
            )
        );
    }

    #[test]
    fn the_log_says_which_vm_the_host_stopped_and_why() {
        assert_eq!(
            stopped_line("fctl-ses-1", STUCK_AFTER, &[72625]),
            "fctl-ses-1's VM did not answer within 2 minutes, so the host stopped it (killed its container-runtime-linux, pid 72625): one VM that doesn't answer holds up every sandbox of Apple's `container`"
        );
        assert_eq!(words(Duration::from_millis(400)), "400 ms");
        assert_eq!(words(Duration::from_secs(60)), "a minute");
        assert_eq!(words(Duration::from_secs(90)), "90 seconds");
    }

    #[test]
    fn stopped_sandboxes_are_known_until_they_start_again() {
        let stuck = Stuck::default();
        let before = Instant::now();
        assert!(!stuck.stopped("fctl-ses-1"));
        assert!(!stuck.stopped_since(before));
        stuck.note("fctl-ses-1");
        assert!(stuck.stopped("fctl-ses-1"));
        assert!(!stuck.stopped("fctl-ses-2"));
        assert!(stuck.stopped_since(before));
        assert!(!stuck.stopped_since(Instant::now() + Duration::from_secs(1)));
        stuck.forget("fctl-ses-1");
        assert!(!stuck.stopped("fctl-ses-1"));
        assert_eq!(stuck.held_up(), None);
        stuck.hold_up(Some("dev's VM does not answer".into()));
        assert_eq!(stuck.held_up().as_deref(), Some("dev's VM does not answer"));
        stuck.hold_up(None);
        assert_eq!(stuck.held_up(), None);
    }
}

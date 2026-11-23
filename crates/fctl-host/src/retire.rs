//! Machines that retire once idle (`retire_idle_minutes`, `retire_command`
//! in `host.toml`): in a pool that grows and shrinks, a machine whose
//! runner has run nothing for a while takes itself away, so that the
//! autoscaler only ever adds machines and never picks a busy one to remove
//! (docs/architecture.md#machines-that-retire).
//!
//! 1. The daemon notes, every half minute, whether anything runs or is
//!    about to ([`Idle`]): sessions, starts that wait, Claude sign-ins,
//!    chores on sandboxes, an update it is about to install.
//! 2. Idle for long enough, it drains the runner, so that the control plane
//!    sends it nothing more, and looks again: a session the control plane
//!    sent as it drained keeps it, and the runner takes sessions again
//!    unless someone else drained it.
//! 3. Still idle, it runs the command ([`run_command`]). One that fails
//!    leaves the machine a runner, which takes sessions again and tries
//!    once more after another idle while: a command that can't retire it
//!    (the autoscaling group is at its minimum, say) costs nothing.
//! 4. Once the command worked, the runner leaves its workspace (an enrolled
//!    one is removed, as `fctl host uninstall` removes it), the host forgets
//!    its token and stops.

use std::time::Duration;
use tokio::time::Instant;

/// How long the retire command may run.
pub const COMMAND_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Since when nothing has run on this machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct Idle {
    since: Option<Instant>,
}

impl Idle {
    /// Notes at `now` whether anything runs (`busy`), and says whether
    /// nothing has run for `after`.
    pub fn note(&mut self, busy: bool, now: Instant, after: Duration) -> bool {
        if busy {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) >= after
    }

    /// Starts counting again: a retirement that didn't happen waits
    /// another idle while.
    pub fn reset(&mut self) {
        self.since = None;
    }
}

/// What a retirement that stopped short says, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kept {
    /// A session reached the runner as it drained.
    Busy,
    /// The control plane didn't answer the drain.
    Unreachable(String),
    /// The command failed: its exit and what it said.
    Command(String),
}

impl Kept {
    pub fn words(&self) -> String {
        match self {
            Self::Busy => {
                "not retiring: a session came as the runner drained, and it takes sessions again"
                    .into()
            }
            Self::Unreachable(e) => {
                format!("not retiring: draining the runner failed ({e}); trying again later")
            }
            Self::Command(e) => format!(
                "not retiring: retire_command failed ({e}); the runner takes sessions again, and tries again once idle again"
            ),
        }
    }
}

/// Runs `command` with `sh -c`, within `limit`: what it printed when it
/// exits 0, or why not.
pub async fn run_command(command: &str, limit: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.args(["-c", command])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(limit, cmd.output()).await {
        Err(_) => return Err(format!("it ran longer than {} s", limit.as_secs())),
        Ok(Err(e)) => return Err(format!("it didn't start: {e}")),
        Ok(Ok(o)) => o,
    };
    let said = |b: &[u8]| {
        let text = String::from_utf8_lossy(b);
        let text = text.trim();
        // The last of it: what a failing CLI says comes at the end.
        let start = text.char_indices().rev().nth(499).map_or(0, |(i, _)| i);
        text[start..].to_owned()
    };
    if output.status.success() {
        Ok(said(&output.stdout))
    } else {
        let why = match (output.status.code(), said(&output.stderr)) {
            (Some(code), e) if e.is_empty() => format!("exit {code}"),
            (Some(code), e) => format!("exit {code}: {e}"),
            (None, e) => format!("killed by a signal {e}").trim().to_owned(),
        };
        Err(why)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_machine_retires_after_a_whole_idle_while() {
        let after = Duration::from_secs(20 * 60);
        let t0 = Instant::now();
        let mut idle = Idle::default();
        assert!(!idle.note(false, t0, after));
        assert!(!idle.note(false, t0 + Duration::from_secs(19 * 60), after));
        // A session in between starts the while again.
        assert!(!idle.note(true, t0 + Duration::from_secs(19 * 60 + 30), after));
        assert!(!idle.note(false, t0 + Duration::from_secs(20 * 60), after));
        assert!(!idle.note(false, t0 + Duration::from_secs(39 * 60), after));
        assert!(idle.note(false, t0 + Duration::from_secs(40 * 60), after));
        idle.reset();
        assert!(!idle.note(false, t0 + Duration::from_secs(41 * 60), after));
    }

    #[tokio::test]
    async fn the_command_says_why_it_failed() {
        assert_eq!(
            run_command("echo bye", COMMAND_LIMIT).await,
            Ok("bye".to_owned())
        );
        assert_eq!(
            run_command(
                "echo 'ValidationError: at its minimum size' >&2; exit 254",
                COMMAND_LIMIT
            )
            .await,
            Err("exit 254: ValidationError: at its minimum size".to_owned())
        );
        assert_eq!(
            run_command("sleep 5", Duration::from_millis(200)).await,
            Err("it ran longer than 0 s".to_owned())
        );
        assert!(
            Kept::Command("exit 1".into())
                .words()
                .contains("retire_command failed (exit 1)")
        );
    }
}

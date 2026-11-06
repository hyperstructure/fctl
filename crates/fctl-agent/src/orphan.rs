//! The Claude a supervisor left running when it died.
//!
//! Claude's output goes to the supervisor that started it, so a Claude
//! that outlives its supervisor works on unseen: nothing it does reaches
//! the transcript. The supervisor records the Claude it starts, which leads
//! a process group of its own ([`crate::user::command_in_own_group`]), and
//! the one that takes the session over stops that group before it starts
//! Claude again on the same conversation.

use serde::{Deserialize, Serialize};

/// A process, as the state records it: its pid, and when it started, which
/// tells it from a later process with the same pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    /// Clock ticks from the system's boot to its start (`/proc/<pid>/stat`).
    pub started: u64,
}

impl Process {
    /// The process `pid`, while it runs; `None` where processes can't be
    /// told apart (not Linux).
    pub fn of(pid: u32) -> Option<Self> {
        let stat = Stat::read(pid)?;
        (!stat.ended()).then_some(Self {
            pid,
            started: stat.started,
        })
    }

    /// Whether it still runs.
    pub fn running(&self) -> bool {
        Stat::read(self.pid).is_some_and(|s| s.started == self.started && !s.ended())
    }

    /// Stops it with everything in its process group, if it still runs,
    /// and waits up to `within` for it to end. Whether it ran.
    pub async fn stop(&self, within: std::time::Duration) -> bool {
        if !self.running() {
            return false;
        }
        kill_group(self.pid);
        let until = tokio::time::Instant::now() + within;
        while self.running() && tokio::time::Instant::now() < until {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        true
    }
}

/// What `/proc/<pid>/stat` says of a process.
struct Stat {
    state: char,
    started: u64,
}

impl Stat {
    #[cfg(target_os = "linux")]
    fn read(pid: u32) -> Option<Self> {
        parse(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
    }

    #[cfg(not(target_os = "linux"))]
    fn read(_: u32) -> Option<Self> {
        None
    }

    /// A zombie stays until its parent reaps it, which a sandbox's init
    /// (`tail -f /dev/null`) never does.
    fn ended(&self) -> bool {
        matches!(self.state, 'Z' | 'X' | 'x')
    }
}

/// `/proc/<pid>/stat`: the state is the field after the command, which is
/// in parentheses and may hold anything; the start time is the 22nd field.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse(stat: &str) -> Option<Stat> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    // State is the 3rd field, start time the 22nd.
    let started = fields.nth(22 - 4)?.parse().ok()?;
    Some(Stat { state, started })
}

/// Kills the process group `pid` leads, or `pid` alone where it leads none.
#[cfg(target_os = "linux")]
fn kill_group(pid: u32) {
    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::{Pid, getpgid};
    let Ok(raw) = i32::try_from(pid) else { return };
    let pid = Pid::from_raw(raw);
    let leads = getpgid(Some(pid)).is_ok_and(|g| g == pid);
    let _ = if leads {
        killpg(pid, Signal::SIGKILL)
    } else {
        kill(pid, Signal::SIGKILL)
    };
}

#[cfg(not(target_os = "linux"))]
fn kill_group(_: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_state_and_start_of_odd_commands() {
        let stat = "4242 (a (weird) cmd) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 10 18446744073709551615";
        let s = parse(stat);
        assert_eq!(
            s.as_ref().map(|s| (s.state, s.started)),
            Some(('S', 987654))
        );
        assert!(
            parse("4242 (zombie) Z 1 4242 4242 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 55 0 0")
                .is_some_and(|s| s.ended())
        );
        assert!(parse("garbage").is_none());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stops_a_claude_left_running_with_what_it_started() -> anyhow::Result<()> {
        use anyhow::Context;
        use std::os::unix::process::CommandExt;
        // A "Claude" in its own group, with a child of its own there.
        let mut leader = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & echo $! ; wait"])
            .process_group(0)
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let mut out = std::io::BufReader::new(leader.stdout.take().context("stdout")?);
        let mut line = String::new();
        std::io::BufRead::read_line(&mut out, &mut line)?;
        let child: u32 = line.trim().parse()?;
        let claude = Process::of(leader.id()).context("claude")?;
        let sleeper = Process::of(child).context("its child")?;
        assert!(claude.running() && sleeper.running());
        // Reaped here, as a sandbox's init would not: ended counts too.
        let reaper = std::thread::spawn(move || leader.wait());
        assert!(claude.stop(std::time::Duration::from_secs(5)).await);
        assert!(!claude.running());
        let _ = reaper.join();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while sleeper.running() && std::time::Instant::now() < until {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(!sleeper.running(), "the group's other process still runs");
        // One that has gone isn't stopped again, nor a later one with its pid.
        assert!(!claude.stop(std::time::Duration::from_secs(1)).await);
        let later = Process {
            started: claude.started + 1,
            ..claude
        };
        assert!(!later.running());
        Ok(())
    }
}

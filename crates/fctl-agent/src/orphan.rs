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

    /// Stops it with everything in its process group, if any of it still
    /// runs: what Claude started (shells, builds, MCP servers) can outlive
    /// it, as Claude ends once its input closes with its supervisor. Waits
    /// up to `within` for them to end. Whether any of it ran.
    pub async fn stop(&self, within: std::time::Duration) -> bool {
        let leads = self.running();
        if !leads && !self.group_left() {
            return false;
        }
        kill_group(self.pid, leads);
        let until = tokio::time::Instant::now() + within;
        while (self.running() || self.group_left()) && tokio::time::Instant::now() < until {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        true
    }

    /// Whether processes of its group run on without it. A group outlives
    /// its leader with its number, which no new process takes while the
    /// group lasts; a process that has the number now means the group went,
    /// and the number is another's.
    fn group_left(&self) -> bool {
        if Stat::read(self.pid).is_some_and(|s| s.started != self.started) {
            return false;
        }
        in_group(self.pid)
    }
}

/// What `/proc/<pid>/stat` says of a process.
struct Stat {
    state: char,
    group: u32,
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
/// in parentheses and may hold anything, then the parent and the process
/// group; the start time is the 22nd field.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse(stat: &str) -> Option<Stat> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let group = fields.nth(1)?.parse().ok()?;
    // State is the 3rd field, the group the 5th, start time the 22nd.
    let started = fields.nth(22 - 6)?.parse().ok()?;
    Some(Stat {
        state,
        group,
        started,
    })
}

/// Whether a process that hasn't ended is in the process group `group`.
#[cfg(target_os = "linux")]
fn in_group(group: u32) -> bool {
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return false;
    };
    procs
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(Stat::read)
        .any(|s| s.group == group && !s.ended())
}

#[cfg(not(target_os = "linux"))]
fn in_group(_: u32) -> bool {
    false
}

/// Kills the process group `pid` led, or `pid` alone where it runs
/// (`leads`) in another group.
#[cfg(target_os = "linux")]
fn kill_group(pid: u32, leads: bool) {
    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::{Pid, getpgid};
    let Ok(raw) = i32::try_from(pid) else { return };
    let pid = Pid::from_raw(raw);
    let _ = if leads && !getpgid(Some(pid)).is_ok_and(|g| g == pid) {
        kill(pid, Signal::SIGKILL)
    } else {
        killpg(pid, Signal::SIGKILL)
    };
}

#[cfg(not(target_os = "linux"))]
fn kill_group(_: u32, _: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_state_and_start_of_odd_commands() {
        let stat = "4242 (a (weird) cmd) S 1 4242 4242 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 987654 1000 10 18446744073709551615";
        let s = parse(stat);
        assert_eq!(
            s.as_ref().map(|s| (s.state, s.group, s.started)),
            Some(('S', 4242, 987654))
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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stops_what_a_claude_that_ended_left_running() -> anyhow::Result<()> {
        use anyhow::Context;
        use std::io::Write;
        use std::os::unix::process::CommandExt;
        // A "Claude" that starts a child in its group, then ends once its
        // input closes, as Claude does when its supervisor dies.
        let mut leader = std::process::Command::new("sh")
            .args(["-c", "sleep 60 & echo $! ; read x"])
            .process_group(0)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        let mut out = std::io::BufReader::new(leader.stdout.take().context("stdout")?);
        let mut line = String::new();
        std::io::BufRead::read_line(&mut out, &mut line)?;
        let sleeper = Process::of(line.trim().parse()?).context("its child")?;
        let claude = Process::of(leader.id()).context("claude")?;
        leader.stdin.take().context("stdin")?.write_all(b"\n")?;
        leader.wait()?;
        assert!(!claude.running() && sleeper.running());
        assert!(claude.stop(std::time::Duration::from_secs(5)).await);
        assert!(!sleeper.running(), "what Claude started still runs");
        assert!(!claude.stop(std::time::Duration::from_secs(1)).await);
        Ok(())
    }
}

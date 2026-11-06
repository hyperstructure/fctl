//! `fctl-agent serve --user`, as the host starts it in container and VM
//! sandboxes: as root, with Claude and the commands for the session as the
//! user, who can't stop it.
//!
//! It takes root, so it runs where `sudo` asks no password (CI's runners),
//! as a user without `~/.fctl`, where the supervisor puts the session
//! socket and the GitHub tokens: in an fctl sandbox, the sandbox's own
//! fctl-agent serves from there. Elsewhere it says why it didn't run.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const AGENT: &str = env!("CARGO_BIN_EXE_fctl-agent");

/// A Claude Code that says, in `seen` beside it, who it runs as and
/// whether it may stop the supervisor, its parent, then waits.
const CLAUDE: &str = r#"#!/bin/sh
seen="$(dirname "$0")/seen"
{
  echo "args $1"
  echo "ids $(id -u) $(id -g) $(id -G)"
  echo "home $HOME"
  echo "oom $(cat /proc/self/oom_score_adj)"
  echo "group $(ps -o pgid= -p $$ | tr -d ' ') $$"
  if kill -TERM "$PPID" 2>/dev/null; then echo "kill done"; else echo "kill refused"; fi
} >> "$seen"
if [ "$1" = --version ]; then echo "2.1.300 (Claude Code)"; exit 0; fi
exec sleep 60
"#;

fn sudo(args: &[&str]) -> Command {
    let mut cmd = Command::new("sudo");
    cmd.arg("-n").args(args);
    cmd
}

/// Why the test can't run here, if it can't.
fn unable() -> Option<String> {
    let root = sudo(&["true"]).stderr(Stdio::null()).status();
    if !root.is_ok_and(|s| s.success()) {
        return Some("sudo asks for a password".into());
    }
    let home = std::env::var_os("HOME")?;
    let dir = Path::new(&home).join(".fctl");
    dir.exists().then(|| {
        format!(
            "{} exists (an fctl sandbox's own fctl-agent?)",
            dir.display()
        )
    })
}

/// What a running test leaves behind: the user's processes, then root's,
/// and files root made.
struct Cleanup {
    children: Vec<Child>,
    groups: Vec<String>,
    pids: Vec<u32>,
    paths: Vec<PathBuf>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for group in &self.groups {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{group}")])
                .status();
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
        for pid in &self.pids {
            let _ = sudo(&["kill", "-KILL", &pid.to_string()]).status();
        }
        for path in &self.paths {
            let _ = sudo(&["rm", "-rf", &path.display().to_string()]).status();
        }
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(
            Instant::now() < until,
            "{what} didn't happen within a minute"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The ids in a `/proc/<pid>/status` line: real, effective, saved, filesystem.
fn ids(pid: u32, field: &str) -> Vec<u32> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix(field))
        .map(|rest| {
            rest.split_whitespace()
                .filter_map(|n| n.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn claude_runs_as_the_user_and_cannot_stop_fctl_agent() -> anyhow::Result<()> {
    if let Some(why) = unable() {
        eprintln!("skipped: {why}");
        return Ok(());
    }
    let uid = nix::unistd::getuid().as_raw();
    let gid = nix::unistd::getgid().as_raw();
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("root");
    let home = dir.path().join("home");
    std::fs::create_dir(&home)?;
    let claude = dir.path().join("claude");
    std::fs::write(&claude, CLAUDE)?;
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755))?;
    let user_dir = PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".fctl");
    let mut cleanup = Cleanup {
        children: vec![],
        groups: vec![],
        pids: vec![],
        paths: vec![root.clone(), user_dir.clone()],
    };

    let socket = root.join("agent.sock");
    let serve = sudo(&[AGENT, "serve", "--user", &uid.to_string(), "--socket"])
        .arg(&socket)
        .arg("--state")
        .arg(root.join("state"))
        .arg("--log")
        .arg(root.join("agent.log"))
        .stdin(Stdio::null())
        .spawn()?;
    cleanup.children.push(serve);
    let socket_text = socket.display().to_string();
    wait_for("the host's socket", || {
        sudo(&["test", "-S", &socket_text])
            .status()
            .is_ok_and(|s| s.success())
    });

    // fctl-agent itself, which the sudo above started.
    let pgrep = Command::new("pgrep")
        .args(["-f", &format!("^{AGENT} serve --user")])
        .output()?;
    let pid: u32 = String::from_utf8_lossy(&pgrep.stdout)
        .lines()
        .next()
        .and_then(|l| l.trim().parse().ok())
        .ok_or_else(|| anyhow::anyhow!("no fctl-agent serve running"))?;
    cleanup.pids.push(pid);
    // Root is its real and saved user, the user its effective one.
    assert_eq!(ids(pid, "Uid:"), vec![0, uid, 0, uid]);
    assert_eq!(ids(pid, "Gid:")[..2], [gid, gid]);
    // So the user may not stop it, nor reach its files.
    let signal = nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(pid)?),
        nix::sys::signal::Signal::SIGTERM,
    );
    assert_eq!(signal, Err(nix::errno::Errno::EPERM));
    let meta = std::fs::metadata(&root)?;
    assert_eq!((meta.uid(), meta.mode() & 0o777), (0, 0o700));
    assert!(std::fs::read_dir(&root).is_err());

    // The host attaches as root, and starts the session.
    let mut attach = sudo(&[AGENT, "attach", "--socket", &socket_text])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut to_agent = attach
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("stdin"))?;
    let mut from_agent = BufReader::new(
        attach
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("stdout"))?,
    );
    cleanup.children.push(attach);
    let mut hello = String::new();
    from_agent.read_line(&mut hello)?;
    assert!(hello.contains(r#""type":"hello""#), "{hello}");
    let configure = serde_json::json!({
        "type": "configure",
        "config": {
            "spec": spec(),
            "workdir": dir.path().join("workspace"),
            "home": home,
            "claude_bin": claude,
            "claude_args": [],
            "env": {},
            "push_on_commit": false,
        }
    });
    writeln!(to_agent, "{configure}")?;
    to_agent.flush()?;
    // Read what it says, so that it never waits on a full pipe.
    std::thread::spawn(move || for _ in from_agent.lines() {});

    let seen = dir.path().join("seen");
    wait_for("Claude to start", || {
        std::fs::read_to_string(&seen).is_ok_and(|s| s.matches("kill ").count() >= 2)
    });
    let seen = std::fs::read_to_string(&seen)?;
    let groups = String::from_utf8_lossy(&Command::new("id").arg("-G").output()?.stdout)
        .trim()
        .to_owned();
    for (i, run) in seen.split("args ").skip(1).enumerate() {
        let line = |key: &str| {
            run.lines()
                .find_map(|l| l.strip_prefix(key))
                .map(str::to_owned)
                .unwrap_or_default()
        };
        assert_eq!(line("ids "), format!("{uid} {gid} {groups}"), "{seen}");
        assert_eq!(line("oom "), "0");
        assert_eq!(line("kill "), "refused", "{seen}");
        if i > 0 {
            // Claude, in the session's home, leads a process group of its own.
            assert_eq!(line("home "), home.display().to_string());
            let group = line("group ");
            let (pgid, pid) = group.split_once(' ').unwrap_or_default();
            assert_eq!(pgid, pid, "{seen}");
            cleanup.groups.push(pgid.to_owned());
        }
    }
    assert!(
        std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "fctl-agent stopped"
    );
    // What it wrote in the user's home is the user's.
    for written in [
        home.join(".claude.json"),
        dir.path().join("workspace"),
        user_dir.join("state"),
    ] {
        assert_eq!(
            std::fs::metadata(&written)?.uid(),
            uid,
            "{}",
            written.display()
        );
    }
    Ok(())
}

/// A session with no repository, as the host sends it.
fn spec() -> serde_json::Value {
    serde_json::json!({
        "session_id": "ses_test",
        "number": 1,
        "title": "Test",
        "project": {
            "id": "prj_test",
            "slug": "widgets",
            "name": "Widgets",
            "default_branch": "main",
            "settings": {}
        },
        "branch": "fctl/1-test",
        "base_branch": "main",
        "author": { "id": "usr_test", "name": "Ada" },
        "claude_session_id": "00000000-0000-0000-0000-000000000000",
        "resume": false,
        "secrets": {},
        "credentials": {
            "claude": {
                "id": "crd_test",
                "kind": "setup_token",
                "access_token": "test",
                "scopes": []
            }
        },
        "git_author": { "name": "fctl", "email": "fctl@example.com" },
        "co_authors": [],
        "web_url": "https://fctl.test/s/ses_test",
        "idle_timeout_secs": 600
    })
}

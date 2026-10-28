//! Running the host as a service: launchd on macOS, systemd (user) on Linux.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "sh.fctl.host";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// The file launchd writes the host's output to, on macOS.
fn log_path() -> PathBuf {
    home().join("Library/Logs/fctl/host.log")
}

/// How `journalctl` picks out the host's log on Linux, where systemd sends
/// the unit's output to the journal. `--user-unit` rather than `--user -u`:
/// journald keeps a journal for each user only on persistent storage, and
/// otherwise the system journal has the log, for those who may read it.
const USER_UNIT: &str = "--user-unit=fctl-host.service";

/// Where the host's log is, as people find it: the file on macOS, the
/// command that shows it on Linux.
pub fn log_location() -> String {
    if cfg!(target_os = "macos") {
        log_path().display().to_string()
    } else {
        format!("journalctl {USER_UNIT}")
    }
}

/// The last `lines` lines of the host's log, or `None` when there is none
/// that this user can read.
pub fn log_tail(lines: usize) -> Result<Option<Vec<String>>> {
    if !cfg!(target_os = "macos") {
        return journal_tail("journalctl", lines);
    }
    let path = log_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(last(&text, lines))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Why a host has no log, when [`log_tail`] finds none.
pub fn why_no_log() -> String {
    if !status().0 {
        "The host service isn't installed, so there's no log: `fctl host run` prints the host's log as it runs".into()
    } else if cfg!(target_os = "macos") {
        format!("No log at {}", log_path().display())
    } else {
        "The journal has nothing from fctl-host.service that you can read: where journald keeps no persistent journal, `sudo journalctl _SYSTEMD_USER_UNIT=fctl-host.service` shows the host's log".into()
    }
}

fn journal_tail(journalctl: &str, lines: usize) -> Result<Option<Vec<String>>> {
    if lines == 0 {
        return Ok(Some(Vec::new()));
    }
    let n = lines.to_string();
    let args = [
        USER_UNIT,
        "--output=cat",
        "--lines",
        &n,
        "--quiet",
        "--no-pager",
    ];
    match sh(journalctl, &args) {
        Ok(text) => {
            let tail: Vec<String> = text.lines().map(str::to_owned).collect();
            Ok((!tail.is_empty()).then_some(tail))
        }
        // No journal that this user may read: they have none of their own
        // yet, or journald keeps none for users, without persistent storage.
        Err(e) if e.to_string().contains("insufficient permissions") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The last `n` lines of `text`.
fn last(text: &str, n: usize) -> Vec<String> {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(n)..]
        .iter()
        .map(|l| (*l).to_owned())
        .collect()
}

fn plist_path() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LABEL}.plist"))
}

fn unit_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    base.join("systemd/user/fctl-host.service")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launchd job: always running, restarted on exit, agents left alone on restart.
pub fn plist(exe: &Path) -> String {
    let log = log_path().display().to_string();
    let path = std::env::var("PATH").unwrap_or_default();
    let path = format!(
        "/usr/local/bin:/opt/homebrew/bin:{}/.local/bin:{path}:/usr/bin:/bin:/usr/sbin:/sbin",
        home().display()
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>host</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>AbandonProcessGroup</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>{path}</string><key>FCTL_LOG_FORMAT</key><string>plain</string></dict>
</dict>
</plist>
"#,
        exe = xml_escape(&exe.display().to_string()),
        log = xml_escape(&log),
        path = xml_escape(&path),
    )
}

pub fn unit(exe: &Path) -> String {
    format!(
        "[Unit]\nDescription=fctl host: runs Claude agent sessions in sandboxes\nAfter=network-online.target\nWants=network-online.target\n\n\
         [Service]\nExecStart={} host run\nRestart=always\nRestartSec=5\n# Agents run in their own process groups and survive restarts.\nKillMode=process\nEnvironment=FCTL_LOG_FORMAT=plain\n\n\
         [Install]\nWantedBy=default.target\n",
        exe.display()
    )
}

fn sh(program: &str, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("running {program}"))?;
    if !out.status.success() {
        bail!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn uid() -> String {
    sh("id", &["-u"])
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|_| "501".into())
}

/// Installs and starts the service. Returns a short description.
pub fn install(exe: &Path) -> Result<String> {
    if cfg!(target_os = "macos") {
        if let Some(dir) = log_path().parent() {
            std::fs::create_dir_all(dir)?;
        }
        let path = plist_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let domain = format!("gui/{}", uid());
        let _ = sh("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
        std::fs::write(&path, plist(exe))?;
        sh(
            "launchctl",
            &["bootstrap", &domain, &path.display().to_string()],
        )?;
        let _ = sh("launchctl", &["enable", &format!("{domain}/{LABEL}")]);
        Ok(format!("launchd job {LABEL} ({})", path.display()))
    } else if cfg!(target_os = "linux") {
        let path = unit_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, unit(exe))?;
        sh("systemctl", &["--user", "daemon-reload"])?;
        sh(
            "systemctl",
            &["--user", "enable", "--now", "fctl-host.service"],
        )?;
        Ok(format!(
            "systemd user unit fctl-host.service ({}); run `loginctl enable-linger $USER` to keep it running after you log out",
            path.display()
        ))
    } else {
        bail!("installing a service is supported on macOS and Linux; run `fctl host run` yourself")
    }
}

pub fn uninstall() -> Result<()> {
    if cfg!(target_os = "macos") {
        let _ = sh("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
        let _ = std::fs::remove_file(plist_path());
    } else {
        let _ = sh(
            "systemctl",
            &["--user", "disable", "--now", "fctl-host.service"],
        );
        let _ = std::fs::remove_file(unit_path());
        let _ = sh("systemctl", &["--user", "daemon-reload"]);
    }
    Ok(())
}

pub fn restart() -> Result<()> {
    if cfg!(target_os = "macos") {
        sh(
            "launchctl",
            &["kickstart", "-k", &format!("gui/{}/{LABEL}", uid())],
        )?;
    } else {
        sh("systemctl", &["--user", "restart", "fctl-host.service"])?;
    }
    Ok(())
}

/// Whether this process is the one the host service runs, which launchd
/// (`KeepAlive`) or systemd (`Restart=always`) start again when it exits.
pub fn runs_this_process() -> bool {
    let main = if cfg!(target_os = "macos") {
        sh("launchctl", &["print", &format!("gui/{}/{LABEL}", uid())])
            .ok()
            .and_then(|o| launchd_pid(&o))
    } else {
        sh(
            "systemctl",
            &[
                "--user",
                "show",
                "--property=MainPID",
                "--value",
                "fctl-host.service",
            ],
        )
        .ok()
        .and_then(|o| o.trim().parse().ok())
    };
    main == Some(std::process::id())
}

/// The process of a running launchd job, as `launchctl print` says it.
fn launchd_pid(print: &str) -> Option<u32> {
    print
        .lines()
        .find_map(|l| l.trim().strip_prefix("pid = ")?.trim().parse().ok())
}

/// Whether the service is installed, and whether it runs.
pub fn status() -> (bool, bool) {
    if cfg!(target_os = "macos") {
        let installed = plist_path().exists();
        let running = sh("launchctl", &["print", &format!("gui/{}/{LABEL}", uid())])
            .map(|o| o.contains("state = running"))
            .unwrap_or(false);
        (installed, running)
    } else {
        let installed = unit_path().exists();
        let running = sh("systemctl", &["--user", "is-active", "fctl-host.service"])
            .map(|o| o.trim() == "active")
            .unwrap_or(false);
        (installed, running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_process_of_a_launchd_job() {
        let print = "gui/501/sh.fctl.host = {\n\tactive count = 1\n\tpath = /Users/ada/Library/LaunchAgents/sh.fctl.host.plist\n\tstate = running\n\n\tprogram = /Users/ada/.local/bin/fctl\n\targuments = {\n\t\t/Users/ada/.local/bin/fctl\n\t\thost\n\t\trun\n\t}\n\n\tpid = 4242\n\timmediate reason = speculative\n}\n";
        assert_eq!(launchd_pid(print), Some(4242));
        let stopped = "gui/501/sh.fctl.host = {\n\tactive count = 0\n\tstate = not running\n\tlast exit code = 0\n}\n";
        assert_eq!(launchd_pid(stopped), None);
    }

    #[test]
    fn reads_the_host_log_from_the_journal() -> Result<()> {
        use crate::driver::fake::write_script;
        let dir = tempfile::tempdir()?;
        let journalctl = dir.path().join("journalctl");
        let run = |lines| journal_tail(&journalctl.display().to_string(), lines);
        // A stand-in that notes what it was asked, and has two lines.
        write_script(
            &journalctl,
            "#!/bin/sh\necho \"$*\" > \"$0.args\"\nprintf 'starting\\nconnected\\n'\n",
        )?;
        assert_eq!(run(2)?, Some(vec!["starting".into(), "connected".into()]));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("journalctl.args"))?,
            "--user-unit=fctl-host.service --output=cat --lines 2 --quiet --no-pager\n"
        );
        assert_eq!(run(0)?, Some(vec![]));
        // Nothing from the unit is no log, and so is a journal this user may
        // not read.
        write_script(&journalctl, "#!/bin/sh\n")?;
        assert_eq!(run(100)?, None);
        let denied = "No journal files were opened due to insufficient permissions.";
        write_script(
            &journalctl,
            &format!("#!/bin/sh\necho '{denied}' >&2\nexit 1\n"),
        )?;
        assert_eq!(run(100)?, None);
        // Other failures say what journalctl said.
        write_script(
            &journalctl,
            "#!/bin/sh\necho 'Failed to open journal' >&2\nexit 1\n",
        )?;
        let error = run(100).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(error.ends_with(": Failed to open journal"), "{error}");
        Ok(())
    }

    #[test]
    fn takes_the_last_lines() {
        assert_eq!(last("one\ntwo\nthree\n", 2), ["two", "three"]);
        assert_eq!(last("one\n", 100), ["one"]);
        assert!(last("", 100).is_empty());
    }
}

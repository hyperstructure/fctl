//! Running the host as a service: launchd on macOS, systemd (user) on Linux.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "sh.fctl.host";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn log_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        home().join("Library/Logs/fctl/host.log")
    } else {
        crate::config::default_data_dir().join("host.log")
    }
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
    if let Some(dir) = log_path().parent() {
        std::fs::create_dir_all(dir)?;
    }
    if cfg!(target_os = "macos") {
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
}

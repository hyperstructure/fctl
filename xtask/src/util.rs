//! Running commands, finding tools, and making secrets.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use std::cell::Cell;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

thread_local! {
    /// The name `run` puts before each line of a command's output, on a
    /// thread `beside` started.
    static TAG: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Prints a command the way a shell would show it.
fn show(cmd: &Command) {
    let mut line = cmd.get_program().to_string_lossy().into_owned();
    for a in cmd.get_args() {
        let a = a.to_string_lossy();
        if a.contains(' ') {
            line.push_str(&format!(" '{a}'"));
        } else {
            line.push(' ');
            line.push_str(&a);
        }
    }
    eprintln!("{}\x1b[2m$ {line}\x1b[0m", tag());
}

fn tag() -> String {
    TAG.get()
        .map(|name| format!("\x1b[2m{name} │\x1b[0m "))
        .unwrap_or_default()
}

/// Leaves out of a command's environment what `cargo run` tells xtask
/// about its own package (`CARGO_PKG_*`, `CARGO_MANIFEST_*`). Build scripts
/// read some of it (ring's does), and cargo builds a crate again when its
/// build script would see other values: ring and the TLS crates on it would
/// build again whenever cargo ran with and without xtask in turn, and with
/// each new version, which the CI caches' keys leave out.
pub fn without_package_env(cmd: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("CARGO_PKG_") || name.starts_with("CARGO_MANIFEST_") {
            cmd.env_remove(&key);
        }
    }
    cmd
}

/// Runs `f` on a thread of its own, beside others, with each line the
/// commands it runs (`run`) print after `name`; `join` waits for it.
pub fn beside<'scope>(
    s: &'scope std::thread::Scope<'scope, '_>,
    name: &'static str,
    f: impl FnOnce() -> Result<()> + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, Result<()>> {
    s.spawn(move || {
        TAG.set(Some(name));
        f()
    })
}

/// Waits for what `beside` runs.
pub fn join(handle: std::thread::ScopedJoinHandle<'_, Result<()>>) -> Result<()> {
    handle
        .join()
        .unwrap_or_else(|_| Err(anyhow!("a thread of xtask panicked")))
}

/// Runs a command with inherited stdio; fails on a non-zero exit. On a
/// thread `beside` started, its output comes line by line after the
/// thread's name instead.
pub fn run(cmd: &mut Command) -> Result<()> {
    show(cmd);
    without_package_env(cmd);
    let status = match TAG.get() {
        None => cmd.status(),
        Some(_) => relayed(cmd),
    }
    .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", cmd.get_program());
    }
    Ok(())
}

/// Runs a command with its stdout and stderr copied to stderr line by
/// line, after the thread's name. Colors stay on when stderr is a terminal.
fn relayed(cmd: &mut Command) -> std::io::Result<ExitStatus> {
    if std::io::stderr().is_terminal() {
        for (name, value) in [("CARGO_TERM_COLOR", "always"), ("FORCE_COLOR", "1")] {
            if std::env::var_os(name).is_none() {
                cmd.env(name, value);
            }
        }
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let tag = tag();
    let (out, err) = (child.stdout.take(), child.stderr.take());
    std::thread::scope(|s| {
        if let Some(out) = out {
            s.spawn(|| relay(&tag, out));
        }
        if let Some(err) = err {
            s.spawn(|| relay(&tag, err));
        }
    });
    child.wait()
}

fn relay(tag: &str, from: impl Read) {
    for line in BufReader::new(from).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line);
        eprintln!("{tag}{}", line.trim_end_matches('\r'));
    }
}

/// Runs a command and returns its stdout; stderr passes through.
pub fn output(cmd: &mut Command) -> Result<String> {
    show(cmd);
    let out = without_package_env(cmd)
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed ({})", cmd.get_program(), out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Runs a command with `input` on stdin (for secrets: never on the command line).
pub fn run_with_stdin(cmd: &mut Command, input: &str) -> Result<()> {
    show(cmd);
    let mut child = without_package_env(cmd)
        .stdin(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    child
        .stdin
        .take()
        .context("stdin")?
        .write_all(input.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("{:?} failed ({status})", cmd.get_program());
    }
    Ok(())
}

/// How many times `retry_throttled` runs a command that Cloudflare's API
/// keeps throttling.
const THROTTLED_ATTEMPTS: u32 = 4;

/// The longest wait `retry_throttled` takes: Cloudflare blocks a token that
/// went over its limit for five minutes.
const THROTTLED_MAX_WAIT: Duration = Duration::from_secs(5 * 60);

/// Runs a wrangler command that calls Cloudflare's API with `f` (`run`,
/// `output`, …), and runs it again when the API throttled it. Once a token,
/// an account or an address has made too many requests, Cloudflare answers
/// 429 (code 971) with a Retry-After for a while, even to a first request,
/// as app.fctl.sh's deploy found in CI. Wrangler waits that out in only some
/// of `wrangler deploy`'s requests, and gives up at once in its D1, KV and
/// secret commands. A throttled request never ran, and running one of
/// xtask's wrangler commands again ends where one run would have (migrations
/// apply those not applied yet), so the command runs again from the start
/// once the wait the API asked for is over: up to four times, and only for
/// waits of up to five minutes.
pub fn retry_throttled<T>(cmd: &mut Command, f: impl Fn(&mut Command) -> Result<T>) -> Result<T> {
    let mut attempt = 1;
    loop {
        // Where wrangler writes what it did, how it failed included.
        let written = std::env::temp_dir().join(format!(
            "xtask-wrangler-{}-{}.json",
            std::process::id(),
            hex::encode(random_bytes::<8>()?)
        ));
        let result = f(cmd.env("WRANGLER_OUTPUT_FILE_PATH", &written));
        let entries = std::fs::read_to_string(&written).unwrap_or_default();
        let _ = std::fs::remove_file(&written);
        let err = match result {
            Ok(out) => return Ok(out),
            Err(err) => err,
        };
        let Some(wait) = throttled(&entries) else {
            return Err(err);
        };
        if wait > THROTTLED_MAX_WAIT {
            return Err(err.context(format!(
                "Cloudflare's API asked wrangler to wait {} s, longer than xtask waits",
                wait.as_secs()
            )));
        }
        if attempt == THROTTLED_ATTEMPTS {
            return Err(err.context(format!(
                "Cloudflare's API throttled wrangler {attempt} times in a row"
            )));
        }
        attempt += 1;
        // Retry-After is rounded up already; a second more for the clocks.
        let wait = wait + Duration::from_secs(1);
        eprintln!(
            "{}Cloudflare's API throttled wrangler: running it again in {} s (attempt {attempt} of {THROTTLED_ATTEMPTS})",
            tag(),
            wait.as_secs()
        );
        std::thread::sleep(wait);
    }
}

/// How long Cloudflare's API asked a wrangler command to wait before it
/// runs again, when it failed because the API throttled it, from what the
/// command wrote to WRANGLER_OUTPUT_FILE_PATH: its Retry-After, or a minute
/// for a 971 that came without one.
fn throttled(entries: &str) -> Option<Duration> {
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(rename = "type")]
        kind: String,
        code: Option<serde_json::Value>,
        retry_after_ms: Option<f64>,
    }
    let failed = entries
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
        .find(|entry| entry.kind == "command-failed")?;
    let asked = failed
        .retry_after_ms
        .and_then(|ms| Duration::try_from_secs_f64(ms / 1000.0).ok());
    let code = failed.code.and_then(|code| code.as_u64());
    asked.or((code == Some(971)).then_some(Duration::from_secs(60)))
}

/// Whether a program is on PATH.
pub fn has(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// The repository's wrangler (from the root package.json), installing the
/// Node dependencies on first use.
pub fn wrangler(root: &Path) -> Result<Command> {
    let bin = root.join("node_modules/.bin/wrangler");
    if !bin.exists() {
        npm_install(root)?;
    }
    Ok(Command::new(bin))
}

/// Whether `npm ci` installed what the root package.json lists now: a
/// `node_modules` from before the Linear stand-in's workspace lacks its link
/// (and its graphql), which npm makes as it installs the workspaces.
pub fn npm_installed(root: &Path) -> bool {
    root.join("node_modules/@fctl/linear-standin").exists()
}

pub fn npm_install(root: &Path) -> Result<()> {
    if !has("npm") {
        bail!("npm is not installed (Node.js 22 or newer is needed for the web app and wrangler)");
    }
    run(Command::new("npm")
        .args(["ci", "--no-audit", "--no-fund"])
        .current_dir(root))
}

/// `worker-build`, the workers-rs bundler (installed with cargo when missing).
pub fn worker_build() -> Result<Command> {
    if !has("worker-build") {
        eprintln!("worker-build is missing; installing it with cargo");
        run(Command::new("cargo").args([
            "install",
            "worker-build",
            "--version",
            "0.8.7",
            "--locked",
        ]))?;
    }
    Ok(Command::new("worker-build"))
}

/// Builds the web app into web/dist.
pub fn build_web(root: &Path) -> Result<()> {
    if !root.join("node_modules").exists() {
        npm_install(root)?;
    }
    run(Command::new("npm")
        .args(["run", "build", "--workspace", "web"])
        .current_dir(root))
}

/// URL-safe random bytes.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
    Ok(b)
}

/// 32 random bytes, standard base64 (the master key format).
pub fn random_key() -> Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(random_bytes::<32>()?))
}

/// A random URL-safe token.
pub fn random_token() -> Result<String> {
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<24>()?))
}

pub fn repo_root() -> Result<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.parent()
        .map(Path::to_path_buf)
        .context("xtask must live in <repo>/xtask")
}

/// The workspace version (from Cargo.toml).
pub fn version(root: &Path) -> Result<String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml"))?;
    let doc: toml::Table = toml::from_str(&text)?;
    doc.get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .context("workspace.package.version is missing")
}

/// Where cargo puts build output (respects CARGO_TARGET_DIR).
pub fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What wrangler 4.151.1 writes when the API throttles it, as it did
    /// app.fctl.sh's deploy.
    const THROTTLED: &str = r#"{"type":"wrangler-session","version":1,"wrangler_version":"4.151.1","command_line_args":["d1","migrations","apply","DB","--remote","--config","wrangler.deploy.toml"],"log_file_path":"/tmp/wrangler.log","timestamp":"2026-10-23T12:38:07.127Z"}
{"type":"command-failed","version":1,"code":971,"message":"A request to the Cloudflare API (/accounts/0123/d1/database/e84e35ee-6755-4bff-a1ff-ba2539406ba0/query) failed.","retry_after_ms":40000,"timestamp":"2026-10-23T12:38:07.465Z"}
"#;

    #[test]
    fn waits_as_long_as_the_api_asks() {
        assert_eq!(throttled(THROTTLED), Some(Duration::from_secs(40)));
        let no_retry_after = r#"{"type":"command-failed","version":1,"code":971,"message":"…"}"#;
        assert_eq!(throttled(no_retry_after), Some(Duration::from_secs(60)));
    }

    #[test]
    fn other_failures_are_not_throttling() {
        let denied = r#"{"type":"command-failed","version":1,"code":10000,"message":"…"}"#;
        assert_eq!(throttled(denied), None);
        let local = r#"{"type":"command-failed","version":1,"code":"ENOENT","message":"…"}"#;
        assert_eq!(throttled(local), None);
        let session = THROTTLED.lines().next().unwrap();
        assert_eq!(throttled(session), None);
        assert_eq!(throttled(""), None);
        assert_eq!(throttled("not json\n"), None);
    }

    /// Writes `script` to `path`, executable, for a test to run right away.
    /// A process that another test starts holds copies of this process's open
    /// files until it execs, and running a script while such a copy is open for
    /// writing fails with "Text file busy". So `sh` writes the script, and it is
    /// never open here.
    fn write_script(path: &Path, script: &str) -> Result<()> {
        let status = Command::new("sh")
            .args(["-c", r#"printf %s "$2" > "$1" && chmod 755 "$1""#, "sh"])
            .arg(path)
            .arg(script)
            .status()?;
        anyhow::ensure!(status.success(), "writing {}: {status}", path.display());
        Ok(())
    }

    /// A stand-in for wrangler that fails with `entry` (written where
    /// wrangler writes it) on its first `failures` runs, then prints how
    /// many times it ran.
    fn wrangler(name: &str, failures: u32, entry: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("wrangler");
        write_script(
            &bin,
            &format!(
                "#!/bin/sh\necho . >> \"$0.runs\"\nruns=$(wc -l < \"$0.runs\")\n\
                 if [ \"$runs\" -le {failures} ]; then\n  \
                 echo '{entry}' >> \"$WRANGLER_OUTPUT_FILE_PATH\"\n  exit 1\nfi\n\
                 echo $runs\n"
            ),
        )
        .unwrap();
        bin
    }

    fn runs(bin: &Path) -> usize {
        std::fs::read_to_string(bin.with_extension("runs"))
            .unwrap()
            .lines()
            .count()
    }

    const NOW: &str = r#"{"type":"command-failed","version":1,"code":971,"retry_after_ms":0}"#;

    #[test]
    fn runs_a_throttled_command_again() {
        let bin = wrangler("throttled-once", 1, NOW);
        let out = retry_throttled(&mut Command::new(&bin), output).unwrap();
        assert_eq!(out.trim(), "2");
        std::fs::remove_dir_all(bin.parent().unwrap()).unwrap();
    }

    #[test]
    fn gives_up_on_a_command_the_api_keeps_throttling() {
        let bin = wrangler("throttled-always", u32::MAX, NOW);
        let err = retry_throttled(&mut Command::new(&bin), run).unwrap_err();
        assert!(err.to_string().contains("4 times in a row"), "{err:#}");
        assert_eq!(runs(&bin), THROTTLED_ATTEMPTS as usize);
        std::fs::remove_dir_all(bin.parent().unwrap()).unwrap();
    }

    #[test]
    fn runs_other_failures_once() {
        let denied = r#"{"type":"command-failed","version":1,"code":10000}"#;
        let later = r#"{"type":"command-failed","version":1,"code":971,"retry_after_ms":600000}"#;
        for (name, entry, says) in [
            ("denied", denied, "failed (exit status: 1)"),
            ("throttled-for-long", later, "wait 600 s"),
        ] {
            let bin = wrangler(name, 1, entry);
            let err = retry_throttled(&mut Command::new(&bin), run).unwrap_err();
            // The stand-in's own failure, not a failure to start it.
            assert!(err.to_string().contains(says), "{err:#}");
            assert_eq!(runs(&bin), 1);
            std::fs::remove_dir_all(bin.parent().unwrap()).unwrap();
        }
    }
}

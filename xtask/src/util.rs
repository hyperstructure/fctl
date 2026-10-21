//! Running commands, finding tools, and making secrets.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use std::cell::Cell;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

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

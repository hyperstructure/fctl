//! Running commands, finding tools, and making secrets.

use anyhow::{Context, Result, bail};
use base64::Engine;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    eprintln!("\x1b[2m$ {line}\x1b[0m");
}

/// Runs a command with inherited stdio; fails on a non-zero exit.
pub fn run(cmd: &mut Command) -> Result<()> {
    show(cmd);
    let status = cmd
        .status()
        .with_context(|| format!("starting {:?}", cmd.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", cmd.get_program());
    }
    Ok(())
}

/// Runs a command and returns its stdout; stderr passes through.
pub fn output(cmd: &mut Command) -> Result<String> {
    show(cmd);
    let out = cmd
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
    let mut child = cmd
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

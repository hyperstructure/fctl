//! `cargo xtask check`: everything CI checks, in the order that fails fastest.

use crate::util::{self, run};
use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

pub fn check(root: &Path, quick: bool) -> Result<()> {
    run(Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(root))?;
    types_are_current(root)?;
    if !root.join("node_modules").exists() {
        util::npm_install(root)?;
    }
    run(Command::new("npm")
        .args(["run", "check", "--workspace", "web"])
        .current_dir(root))?;
    run(Command::new("cargo")
        .args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(root))?;
    run(Command::new("cargo")
        .args([
            "clippy",
            "-p",
            "fctl-control",
            "--target",
            "wasm32-unknown-unknown",
            "--locked",
            "--",
            "-D",
            "warnings",
        ])
        .current_dir(root))?;
    run(Command::new("cargo")
        .args(["test", "--workspace", "--locked"])
        .current_dir(root))?;
    run(Command::new("npm")
        .args(["test", "--workspace", "web"])
        .current_dir(root))?;
    if !quick {
        util::build_web(root)?;
        if root.join("site/package.json").exists() {
            run(Command::new("npm")
                .args(["run", "build", "--workspace", "site"])
                .current_dir(root))?;
        }
    }
    eprintln!("\nall checks passed");
    Ok(())
}

/// web/src/gen must match what `cargo xtask gen` writes.
fn types_are_current(root: &Path) -> Result<()> {
    let tmp = std::env::temp_dir().join(format!("fctl-gen-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    fctl_proto::export_typescript(&tmp)?;
    let current = root.join("web/src/gen");
    let mut stale = Vec::new();
    for entry in std::fs::read_dir(&tmp)? {
        let entry = entry?;
        let name = entry.file_name();
        let fresh = std::fs::read_to_string(entry.path())?;
        let have = std::fs::read_to_string(current.join(&name)).unwrap_or_default();
        if normalize(&fresh) != normalize(&have) {
            stale.push(name.to_string_lossy().into_owned());
        }
    }
    std::fs::remove_dir_all(&tmp)?;
    if !stale.is_empty() {
        stale.sort();
        bail!(
            "web/src/gen is out of date ({}): run `cargo xtask gen`",
            stale.join(", ")
        );
    }
    Ok(())
}

/// Ignores formatting differences (biome may reflow generated files).
fn normalize(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

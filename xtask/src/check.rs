//! `cargo xtask check`: what CI checks, in the order that fails fastest. CI
//! runs each part in a job of its own, at once, and `cargo xtask dev
//! --smoke` in another.

use crate::util::{self, run};
use anyhow::{Result, bail};
use std::path::Path;
use std::process::Command;

/// A part of `cargo xtask check`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// Formatting, the generated types, and Clippy for the host and the Worker.
    Lint,
    /// `cargo test`.
    Test,
    /// Biome, TypeScript and the tests of the web app, the standalone
    /// server and the Linear stand-in, fctl's Linear queries against
    /// Linear's schema, and the web app's production build.
    Web,
}

impl Part {
    const ALL: [Self; 3] = [Self::Lint, Self::Test, Self::Web];

    fn name(self) -> &'static str {
        match self {
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Web => "web",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        match Self::ALL.into_iter().find(|p| p.name() == name) {
            Some(part) => Ok(part),
            None => bail!("unknown part `{name}`: lint, test or web"),
        }
    }
}

/// The npm workspaces (the root package.json's), whose Biome and tests the
/// web part runs.
const WORKSPACES: [&str; 3] = ["web", "deploy/standalone", "xtask/linear"];

/// Runs `parts`, or all of them when it is empty.
pub fn check(root: &Path, parts: &[Part], quick: bool) -> Result<()> {
    let has = |part| parts.is_empty() || parts.contains(&part);
    if has(Part::Lint) {
        run(Command::new("cargo")
            .args(["fmt", "--all", "--check"])
            .current_dir(root))?;
        types_are_current(root)?;
    }
    if has(Part::Web) {
        if !root.join("node_modules").exists() {
            util::npm_install(root)?;
        }
        for workspace in WORKSPACES {
            run(Command::new("npm")
                .args(["run", "check", "--workspace", workspace])
                .current_dir(root))?;
        }
    }
    if has(Part::Lint) {
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
    }
    if has(Part::Test) {
        run(Command::new("cargo")
            .args(["test", "--workspace", "--locked"])
            .current_dir(root))?;
    }
    if has(Part::Web) {
        for workspace in WORKSPACES {
            run(Command::new("npm")
                .args(["test", "--workspace", workspace])
                .current_dir(root))?;
        }
        crate::linear::check(root)?;
        if !quick {
            util::build_web(root)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_have_names() {
        assert_eq!(Part::parse("lint").unwrap(), Part::Lint);
        assert_eq!(Part::parse("test").unwrap(), Part::Test);
        assert_eq!(Part::parse("web").unwrap(), Part::Web);
        assert!(Part::parse("clippy").is_err());
    }

    /// CI runs each part in a job of its own (with xtask's binary, from
    /// .github/actions/xtask): one it left out would be checked by nothing.
    #[test]
    fn ci_runs_every_part() {
        let workflow = include_str!("../../.github/workflows/ci.yml");
        for part in Part::ALL {
            let step = format!("run: xtask check {}\n", part.name());
            assert!(workflow.contains(&step), "ci.yml has no `{}`", step.trim());
        }
    }
}

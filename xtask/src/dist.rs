//! Release archives: `cargo xtask dist` cross-compiles fctl (and the Linux
//! fctl-agent that goes into sandboxes) with cargo-zigbuild, then packages
//! dist/v<version>/:
//!
//!   fctl-<target>.tar.gz        fctl + fctl-agent-<arch>-linux for its sandboxes
//!   fctl-agent-<arch>-linux     standalone agents (hosts download these on demand)
//!   *.sha256, SHA256SUMS, manifest.json
//!
//! `cargo xtask publish` uploads a dist directory to the releases store
//! (deploy/releases.toml) that https://fctl.sh/releases serves.

use crate::util::{self, run};
use anyhow::{Result, bail};
use flate2::{Compression, write::GzEncoder};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Targets fctl is released for.
pub const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];

/// The oldest macOS the binaries run on.
const MACOS_DEPLOYMENT_TARGET: &str = "12.0";

fn arch(target: &str) -> &str {
    target.split('-').next().unwrap_or(target)
}

fn agent_target(arch: &str) -> String {
    format!("{arch}-unknown-linux-musl")
}

fn agent_asset(arch: &str) -> String {
    format!("fctl-agent-{arch}-linux")
}

fn build(root: &Path, package: &str, target: &str) -> Result<PathBuf> {
    let mut cmd = Command::new("cargo");
    cmd.args([
        "zigbuild",
        "--locked",
        "--profile",
        "dist",
        "-p",
        package,
        "--target",
        target,
    ])
    .current_dir(root);
    if target.ends_with("apple-darwin") {
        cmd.env("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET);
    }
    run(&mut cmd)?;
    let bin = util::target_dir(root)
        .join(target)
        .join("dist")
        .join(package);
    if !bin.is_file() {
        bail!("{} was not built", bin.display());
    }
    Ok(bin)
}

fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// A gzipped tarball with fixed metadata, so identical inputs give identical archives.
fn tarball(out: &Path, files: &[(&str, &Path)]) -> Result<()> {
    let file = std::fs::File::create(out)?;
    let mut tar = tar::Builder::new(GzEncoder::new(file, Compression::best()));
    for (name, path) in files {
        let data = std::fs::read(path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        tar.append_data(&mut header, name, data.as_slice())?;
    }
    tar.into_inner()?.finish()?;
    Ok(())
}

pub fn dist(root: &Path, targets: &[String]) -> Result<PathBuf> {
    if !util::has("cargo-zigbuild") {
        bail!(
            "cargo-zigbuild is missing: `cargo install cargo-zigbuild --locked` (and zig 0.13+ on PATH)"
        );
    }
    let targets: Vec<String> = if targets.is_empty() {
        TARGETS.iter().map(|t| (*t).to_owned()).collect()
    } else {
        targets.to_vec()
    };
    let version = util::version(root)?;
    let out = root.join("dist").join(format!("v{version}"));
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    std::fs::create_dir_all(&out)?;

    // Agents first: every archive carries the one for its architecture.
    let mut arches: Vec<&str> = targets.iter().map(|t| arch(t)).collect();
    arches.sort_unstable();
    arches.dedup();
    for a in &arches {
        let bin = build(root, "fctl-agent", &agent_target(a))?;
        std::fs::copy(&bin, out.join(agent_asset(a)))?;
    }
    for target in &targets {
        let fctl = build(root, "fctl", target)?;
        let agent = out.join(agent_asset(arch(target)));
        tarball(
            &out.join(format!("fctl-{target}.tar.gz")),
            &[("fctl", &fctl), (&agent_asset(arch(target)), &agent)],
        )?;
    }

    // Checksums: one file per asset (what hosts check) and SHA256SUMS (what people check).
    let mut names: Vec<String> = std::fs::read_dir(&out)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    let mut sums = String::new();
    let mut files = Vec::new();
    for n in &names {
        let path = out.join(n);
        let sum = sha256(&path)?;
        std::fs::write(out.join(format!("{n}.sha256")), format!("{sum}  {n}\n"))?;
        sums.push_str(&format!("{sum}  {n}\n"));
        files.push(serde_json::json!({
            "name": n,
            "sha256": sum,
            "size": std::fs::metadata(&path)?.len(),
        }));
    }
    std::fs::write(out.join("SHA256SUMS"), sums)?;
    let manifest = serde_json::json!({ "version": version, "targets": targets, "files": files });
    std::fs::write(
        out.join("manifest.json"),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    eprintln!("\nrelease archives in {}", out.display());
    Ok(out)
}

/// deploy/releases.toml: where releases are published.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Releases {
    kv_namespace_id: String,
    public_url: String,
}

/// Values larger than this do not fit in KV.
const KV_MAX: u64 = 25 * 1024 * 1024;

pub fn publish(root: &Path, version: Option<String>, latest: bool) -> Result<()> {
    let version = match version {
        Some(v) => v.trim_start_matches('v').to_owned(),
        None => util::version(root)?,
    };
    let dir = root.join("dist").join(format!("v{version}"));
    if !dir.join("manifest.json").is_file() {
        bail!(
            "{} has no release: run `cargo xtask dist` first",
            dir.display()
        );
    }
    let config: Releases =
        toml::from_str(&std::fs::read_to_string(root.join("deploy/releases.toml"))?)?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    // The manifest goes last: its presence means the release is complete.
    names.retain(|n| n != "manifest.json");
    names.push("manifest.json".into());
    let put = |key: &str| -> Result<Command> {
        let mut cmd = util::wrangler(root)?;
        cmd.args([
            "kv",
            "key",
            "put",
            "--remote",
            "--namespace-id",
            &config.kv_namespace_id,
            key,
        ])
        .current_dir(root);
        Ok(cmd)
    };
    for n in &names {
        let path = dir.join(n);
        let size = std::fs::metadata(&path)?.len();
        if size > KV_MAX {
            bail!("{n} is {size} bytes, over the 25 MiB a KV value can hold");
        }
        run(put(&format!("v{version}/{n}"))?.arg("--path").arg(&path))?;
    }
    if latest {
        run(put("latest")?.arg(&version))?;
    }
    eprintln!(
        "published v{version}{} to {}/v{version}/",
        if latest { " (latest)" } else { "" },
        config.public_url
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarballs_are_deterministic() {
        let dir = std::env::temp_dir().join(format!("xtask-dist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("bin");
        std::fs::write(&bin, b"#!/bin/sh\necho hi\n").unwrap();
        let (a, b) = (dir.join("a.tar.gz"), dir.join("b.tar.gz"));
        tarball(&a, &[("fctl", &bin)]).unwrap();
        tarball(&b, &[("fctl", &bin)]).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names() {
        assert_eq!(arch("aarch64-apple-darwin"), "aarch64");
        assert_eq!(agent_target("x86_64"), "x86_64-unknown-linux-musl");
        assert_eq!(agent_asset("aarch64"), "fctl-agent-aarch64-linux");
    }
}

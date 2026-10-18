//! Release archives: `cargo xtask dist` cross-compiles fctl (and the Linux
//! fctl-agent that goes into sandboxes) with cargo-zigbuild, then packages
//! dist/v<version>/:
//!
//!   fctl-<target>.tar.gz        fctl + fctl-agent-<arch>-linux for its sandboxes
//!   fctl-agent-<arch>-linux     standalone agents (hosts download these on demand)
//!   *.sha256, SHA256SUMS, manifest.json
//!
//! The two halves also run apart, so that the Release workflow builds the
//! binaries in parallel jobs: `dist build` cross-compiles one binary into
//! dist/bin/v<version>/, and `dist package` makes the archives from them.
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

fn targets_or_all(targets: &[String]) -> Vec<String> {
    if targets.is_empty() {
        TARGETS.iter().map(|t| (*t).to_owned()).collect()
    } else {
        targets.to_vec()
    }
}

fn arches(targets: &[String]) -> Vec<&str> {
    let mut arches: Vec<&str> = targets.iter().map(|t| arch(t)).collect();
    arches.sort_unstable();
    arches.dedup();
    arches
}

/// What the archives for `targets` are made of, as (package, target): the
/// agent for each architecture (every archive carries the one for its own),
/// and fctl for each target.
fn binaries(targets: &[String]) -> Vec<(&'static str, String)> {
    let agents = arches(targets)
        .into_iter()
        .map(|a| ("fctl-agent", agent_target(a)));
    agents
        .chain(targets.iter().map(|t| ("fctl", t.clone())))
        .collect()
}

/// Where `dist build` leaves a binary for `dist package`.
fn built(root: &Path, version: &str, package: &str, target: &str) -> PathBuf {
    root.join("dist")
        .join("bin")
        .join(format!("v{version}"))
        .join(format!("{package}-{target}"))
}

/// rustc looks for a macOS SDK when it links for darwin (SDKROOT, else
/// `xcrun`) and warns when there is none, but zig links against the libSystem
/// stubs it bundles. An empty SDK quiets rustc and leaves the binary as it
/// was; it has the directories cargo-zigbuild adds to zig's search paths
/// when SDKROOT is set, which zig warns about when they are missing.
fn empty_macos_sdk(root: &Path) -> Result<PathBuf> {
    let sdk = std::path::absolute(util::target_dir(root).join("xtask-dist/empty-macos-sdk"))?;
    for dir in ["usr/include", "usr/lib", "System/Library/Frameworks"] {
        std::fs::create_dir_all(sdk.join(dir))?;
    }
    Ok(sdk)
}

fn zigbuild(root: &Path, package: &str, target: &str) -> Result<PathBuf> {
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
        if std::env::var_os("SDKROOT").is_none() && !util::has("xcrun") {
            cmd.env("SDKROOT", empty_macos_sdk(root)?);
        }
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

/// Builds the archives for `targets` (all of them when empty): every binary
/// they need, then the archives.
pub fn dist(root: &Path, targets: &[String]) -> Result<PathBuf> {
    let targets = targets_or_all(targets);
    for (package, target) in binaries(&targets) {
        build(root, package, &target)?;
    }
    package(root, &targets)
}

/// Cross-compiles one release binary (fctl or fctl-agent) into
/// dist/bin/v<version>/.
pub fn build(root: &Path, package: &str, target: &str) -> Result<PathBuf> {
    if !["fctl", "fctl-agent"].contains(&package) {
        bail!("releases have fctl and fctl-agent, not {package}");
    }
    if !util::has("cargo-zigbuild") {
        bail!(
            "cargo-zigbuild is missing: `cargo install cargo-zigbuild --locked` (and zig 0.13+ on PATH)"
        );
    }
    let out = built(root, &util::version(root)?, package, target);
    let bin = zigbuild(root, package, target)?;
    let mut bytes = std::fs::read(&bin)?;
    if target.ends_with("apple-darwin") {
        // zig links with its own deployment target; lower it (see macho.rs).
        let before = crate::macho::set_min_macos(&mut bytes, MACOS_DEPLOYMENT_TARGET)?;
        eprintln!(
            "{target}: minimum macOS {} → {MACOS_DEPLOYMENT_TARGET}",
            crate::macho::decode_version(before)
        );
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, bytes)?;
    std::fs::set_permissions(&out, std::fs::metadata(&bin)?.permissions())?;
    eprintln!("{package} for {target} in {}", out.display());
    Ok(out)
}

/// Makes dist/v<version>/ for `targets` (all of them when empty) from the
/// binaries `dist build` left in dist/bin/v<version>/.
pub fn package(root: &Path, targets: &[String]) -> Result<PathBuf> {
    let targets = targets_or_all(targets);
    let version = util::version(root)?;
    for (package, target) in binaries(&targets) {
        let bin = built(root, &version, package, &target);
        if !bin.is_file() {
            bail!(
                "{} is missing: run `cargo xtask dist build {package} --target {target}`",
                bin.display()
            );
        }
    }
    let out = root.join("dist").join(format!("v{version}"));
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    std::fs::create_dir_all(&out)?;

    for a in arches(&targets) {
        let agent = built(root, &version, "fctl-agent", &agent_target(a));
        std::fs::copy(&agent, out.join(agent_asset(a)))?;
    }
    for target in &targets {
        let fctl = built(root, &version, "fctl", target);
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

    #[test]
    fn an_agent_per_arch_and_fctl_per_target() {
        assert_eq!(
            binaries(&targets_or_all(&[])),
            [
                ("fctl-agent", "aarch64-unknown-linux-musl".to_owned()),
                ("fctl-agent", "x86_64-unknown-linux-musl".to_owned()),
                ("fctl", "aarch64-apple-darwin".to_owned()),
                ("fctl", "x86_64-apple-darwin".to_owned()),
                ("fctl", "aarch64-unknown-linux-musl".to_owned()),
                ("fctl", "x86_64-unknown-linux-musl".to_owned()),
            ]
        );
    }

    /// The Release workflow builds each binary in a job of its own, and
    /// `dist package` fails there, after the tag was pushed, if one is missing.
    #[test]
    fn the_release_workflow_builds_every_binary() {
        let workflow = include_str!("../../.github/workflows/release.yml");
        for (package, target) in binaries(&targets_or_all(&[])) {
            let job = format!("- {{ package: {package}, target: {target} }}");
            assert!(workflow.contains(&job), "release.yml has no `{job}`");
        }
    }

    #[test]
    fn packages_what_dist_build_left() {
        use std::io::Read;
        let root = std::env::temp_dir().join(format!("xtask-package-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace.package]\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        let targets = ["aarch64-apple-darwin".to_owned()];
        assert!(package(&root, &targets).is_err(), "nothing was built yet");
        for (package, target) in binaries(&targets) {
            let bin = built(&root, "1.2.3", package, &target);
            std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
            std::fs::write(&bin, format!("{package} for {target}")).unwrap();
        }

        let out = package(&root, &targets).unwrap();
        assert_eq!(out, root.join("dist/v1.2.3"));
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "SHA256SUMS",
                "fctl-aarch64-apple-darwin.tar.gz",
                "fctl-aarch64-apple-darwin.tar.gz.sha256",
                "fctl-agent-aarch64-linux",
                "fctl-agent-aarch64-linux.sha256",
                "manifest.json",
            ]
        );
        let agent = "fctl-agent for aarch64-unknown-linux-musl";
        assert_eq!(
            std::fs::read_to_string(out.join("fctl-agent-aarch64-linux")).unwrap(),
            agent
        );
        let archive = std::fs::File::open(out.join("fctl-aarch64-apple-darwin.tar.gz")).unwrap();
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        let files: Vec<(String, String)> = archive
            .entries()
            .unwrap()
            .map(|e| {
                let mut e = e.unwrap();
                let mut data = String::new();
                e.read_to_string(&mut data).unwrap();
                (e.path().unwrap().display().to_string(), data)
            })
            .collect();
        assert_eq!(
            files,
            [
                (
                    "fctl".to_owned(),
                    "fctl for aarch64-apple-darwin".to_owned()
                ),
                ("fctl-agent-aarch64-linux".to_owned(), agent.to_owned()),
            ]
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}

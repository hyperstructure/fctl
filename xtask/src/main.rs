//! `cargo xtask <command>`: development automation. Run without arguments for help.

mod check;
mod deploy;
mod dev;
mod dist;
mod util;

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

const HELP: &str = "\
cargo xtask <command>

Develop
  dev [--no-host] [--no-vite]   Run everything locally: control plane, web app (hot reload)
                                and a host whose agents run fake-claude
  check [--quick]               Everything CI checks (fmt, types, lint, tests, builds)
  gen                           Regenerate web/src/gen from crates/fctl-proto

Deploy (deploy/instances/<name>.toml)
  deploy <name> [--create] [--dry-run] [--skip-build]
                                Build and deploy an instance (--create makes its D1 database)
  secrets <name> [--github-key <pem>] [--set <NAME>]... [--new-setup-token]
                                Set the instance's secrets; --set reads values from stdin
  site [--dry-run]              Build and deploy https://fctl.sh (site/)

Release
  dist [--target <triple>]...   Cross-compile release archives into dist/v<version>/
  publish [<version>] [--latest]
                                Upload dist/v<version> to the releases store (fctl.sh/releases)
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = util::repo_root()?;
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => {
            print!("{HELP}");
            return Ok(());
        }
    };
    let mut a = Args::new(rest);
    match cmd {
        "gen" => {
            a.done()?;
            generate_types(&root)
        }
        "check" => {
            let quick = a.flag("--quick");
            a.done()?;
            check::check(&root, quick)
        }
        "dev" => {
            let opts = dev::DevOpts {
                no_host: a.flag("--no-host"),
                no_vite: a.flag("--no-vite"),
            };
            a.done()?;
            dev::dev(&root, opts)
        }
        "deploy" => {
            let opts = deploy::DeployOpts {
                create: a.flag("--create"),
                dry_run: a.flag("--dry-run"),
                skip_build: a.flag("--skip-build"),
            };
            let name = a.positional("instance name")?;
            a.done()?;
            deploy::deploy(&root, &name, opts)
        }
        "secrets" => {
            let opts = deploy::SecretOpts {
                github_key: a.value("--github-key")?.map(PathBuf::from),
                set: a.values("--set")?,
                new_setup_token: a.flag("--new-setup-token"),
            };
            let name = a.positional("instance name")?;
            a.done()?;
            deploy::secrets(&root, &name, opts)
        }
        "site" => {
            let dry_run = a.flag("--dry-run");
            a.done()?;
            deploy_site(&root, dry_run)
        }
        "dist" => {
            let targets = a.values("--target")?;
            a.done()?;
            dist::dist(&root, &targets).map(|_| ())
        }
        "publish" => {
            let latest = a.flag("--latest");
            let version = a.optional();
            a.done()?;
            dist::publish(&root, version, latest)
        }
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        other => {
            print!("{HELP}");
            bail!("unknown command `{other}`")
        }
    }
}

/// A tiny argument parser: flags, `--key value` options and positionals.
struct Args(Vec<String>);

impl Args {
    fn new(rest: &[String]) -> Self {
        Self(rest.to_vec())
    }

    fn flag(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|a| a != name);
        self.0.len() != before
    }

    fn value(&mut self, name: &str) -> Result<Option<String>> {
        Ok(self.values(name)?.pop())
    }

    fn values(&mut self, name: &str) -> Result<Vec<String>> {
        let mut out = Vec::new();
        while let Some(i) = self.0.iter().position(|a| a == name) {
            if i + 1 >= self.0.len() {
                bail!("{name} needs a value");
            }
            out.push(self.0.remove(i + 1));
            self.0.remove(i);
        }
        Ok(out)
    }

    fn positional(&mut self, what: &str) -> Result<String> {
        self.optional().with_context(|| format!("missing {what}"))
    }

    fn optional(&mut self) -> Option<String> {
        let i = self.0.iter().position(|a| !a.starts_with("--"))?;
        Some(self.0.remove(i))
    }

    fn done(&self) -> Result<()> {
        if let Some(extra) = self.0.first() {
            bail!("unexpected argument `{extra}` (cargo xtask help)");
        }
        Ok(())
    }
}

/// Builds and deploys the project site (site/) to fctl.sh.
fn deploy_site(root: &Path, dry_run: bool) -> Result<()> {
    if !root.join("node_modules").exists() {
        util::npm_install(root)?;
    }
    util::run(
        Command::new("npm")
            .args(["run", "build", "--workspace", "site"])
            .current_dir(root),
    )?;
    if dry_run {
        return Ok(());
    }
    util::run(
        util::wrangler(root)?
            .arg("deploy")
            .current_dir(root.join("site")),
    )
}

/// Regenerates `web/src/gen` from the Rust protocol types.
fn generate_types(root: &Path) -> Result<()> {
    let out = root.join("web/src/gen");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    std::fs::create_dir_all(&out)?;
    fctl_proto::export_typescript(&out).context("exporting TypeScript")?;
    // An index that re-exports every type.
    let mut names: Vec<String> = std::fs::read_dir(&out)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".ts") && n != "index.ts")
        .map(|n| n.trim_end_matches(".ts").to_owned())
        .collect();
    names.sort();
    let mut index =
        String::from("// Generated by `cargo xtask gen` from crates/fctl-proto. Do not edit.\n");
    for n in &names {
        index.push_str(&format!("export type {{ {n} }} from \"./{n}\";\n"));
    }
    std::fs::write(out.join("index.ts"), index)?;
    println!("wrote {} types to {}", names.len(), out.display());
    Ok(())
}

//! `cargo xtask <command>`: development automation. Run without arguments for help.

mod check;
mod deploy;
mod dev;
mod dist;
mod macho;
mod release;
mod util;

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const HELP: &str = "\
cargo xtask <command>

Develop
  dev [--no-host] [--no-vite] [--smoke]
                                Run everything locally: control plane, web app (hot reload)
                                and a host whose agents run fake-claude; --smoke runs one
                                session to the end and stops (CI)
  check [--quick]               What CI checks (fmt, types, lint, tests, builds); CI also
                                runs dev --smoke
  gen                           Regenerate web/src/gen from crates/fctl-proto

Deploy (deploy/instances/<name>.toml)
  deploy <name> [--create] [--dry-run] [--skip-build]
                                Build and deploy an instance (--create makes its D1 database)
  secrets <name> [--github-key <pem>] [--set <NAME>]... [--new-setup-token]
                                Set the instance's secrets; --set reads values from stdin
  site                          Where https://fctl.sh lives now (hyperstructure/fctl-site)

Release (see .claude/skills/release/SKILL.md)
  release [--bump auto|patch|minor|major] [--dry-run]
                                Version, CHANGELOG.md, release commit and tag from the
                                Conventional Commits since the last tag (CI pushes them)
  notes [<tag>]                 Release notes for <tag> (default: unreleased changes)
  lint-commit [<subject>] [--range <a..b>]
                                Check Conventional Commit subjects (CI: pull request titles)
  dist [--target <triple>]...   Cross-compile release archives into dist/v<version>/
  dist build <package> --target <triple>
                                Only cross-compile fctl or fctl-agent, into dist/bin/v<version>/
  dist package [--target <triple>]...
                                Only make the archives, from the binaries in dist/bin/v<version>/
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
                smoke: a.flag("--smoke"),
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
            a.done()?;
            println!(
                "https://fctl.sh is built and deployed from hyperstructure/fctl-site.\n\
                 It renders this repository's docs/ at the commit pinned in its fctl-docs.ref,\n\
                 which CI updates when docs/ changes on main."
            );
            Ok(())
        }
        "release" => {
            let bump = match a.value("--bump")? {
                Some(b) => release::Bump::parse(&b)?,
                None => None,
            };
            let dry_run = a.flag("--dry-run");
            a.done()?;
            release::release(&root, release::ReleaseOpts { bump, dry_run }).map(|_| ())
        }
        "notes" => {
            let tag = a.optional();
            a.done()?;
            release::print_notes(&root, tag)
        }
        "lint-commit" => {
            let range = a.value("--range")?;
            let subject = a.optional();
            a.done()?;
            if subject.is_none() && range.is_none() {
                bail!("give a subject or --range <a..b>");
            }
            release::lint_commits(&root, subject, range)
        }
        "dist" => {
            let targets = a.values("--target")?;
            match a.optional().as_deref() {
                None => {
                    a.done()?;
                    dist::dist(&root, &targets).map(|_| ())
                }
                Some("build") => {
                    let package = a.positional("package (fctl or fctl-agent)")?;
                    a.done()?;
                    let [target] = targets.as_slice() else {
                        bail!("dist build takes one --target");
                    };
                    dist::build(&root, &package, target).map(|_| ())
                }
                Some("package") => {
                    a.done()?;
                    dist::package(&root, &targets).map(|_| ())
                }
                Some(other) => bail!("unknown dist command `{other}` (cargo xtask help)"),
            }
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

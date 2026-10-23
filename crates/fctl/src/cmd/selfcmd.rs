//! `fctl self update`: replace this fctl, and the Linux agent next to it,
//! with a release from fctl.sh, the way the installer does: one archive for
//! this platform, checked against its SHA-256 (`fctl_host::update`, which
//! hosts that update themselves use too). Afterwards it restarts the host
//! service (agents keep running) and refreshes installed completions.

use crate::style;
use anyhow::{Context, Result};
use clap::Subcommand;
use fctl_domain::updates::Version;
use fctl_host::{HostConfig, service, update};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Subcommand)]
pub enum SelfCmd {
    /// Update fctl to the latest release; a host service on this machine
    /// restarts on the new version (its agents keep running).
    Update {
        /// Install this version instead of the latest (e.g. 0.1.2).
        #[arg(long, value_name = "VERSION")]
        version: Option<String>,
        /// Only check whether an update is available.
        #[arg(long, conflicts_with = "version")]
        check: bool,
    },
}

fn download_base() -> String {
    let base = std::env::var("FCTL_DOWNLOAD")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://fctl.sh/releases".into());
    update::base(&base).to_owned()
}

pub async fn run(cmd: SelfCmd, json: bool) -> Result<()> {
    let SelfCmd::Update { version, check } = cmd;
    let base = download_base();
    let latest = match version.as_deref() {
        Some(v) => Version::parse(v).with_context(|| format!("unexpected version: {v}"))?,
        None => update::latest(&base).await?,
    };
    let have = Version::parse(VERSION).with_context(|| format!("unexpected version: {VERSION}"))?;
    if check {
        if json {
            return style::print_json(&serde_json::json!({
                "current": VERSION,
                "latest": latest.to_string(),
                "update_available": latest > have,
            }));
        }
        if latest > have {
            println!("fctl {latest} is available (you have {VERSION}). Run `fctl self update`.");
        } else {
            println!("fctl {VERSION} is up to date.");
        }
        return Ok(());
    }
    if version.is_none() && latest <= have {
        println!("fctl {VERSION} is up to date.");
        return Ok(());
    }

    let target = update::this_target()?;
    if !json {
        println!("Downloading fctl {latest} for {target}");
    }
    let install = update::Install::here()?;
    let dir = install.dir();
    // The new fctl has to run here before it replaces this one.
    let release = update::download(&base, latest, &target, dir)
        .await
        .map_err(|miss| anyhow::anyhow!("{miss}; nothing was changed"))?;
    release.install(dir)?;

    // A host service runs the binary at this path: restart it on the new one.
    let (installed, running) = service::status();
    let restarted = installed && running && service::restart().is_ok();
    // Completion scripts list the commands; the new fctl rewrites its own.
    let refreshed = std::process::Command::new(dir.join("fctl"))
        .args(["completions", "--refresh"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
    if json {
        return style::print_json(&serde_json::json!({
            "previous": VERSION,
            "version": latest.to_string(),
            "path": dir.join("fctl"),
            "host_restarted": restarted,
            "completions_refreshed": refreshed,
        }));
    }
    println!(
        "Updated fctl {VERSION} → {} in {}",
        style::bold(&latest.to_string()),
        dir.display()
    );
    if restarted {
        println!("Restarted the host service; its sessions keep running.");
        // The new fctl is at least this one, which hosts update themselves with.
        let updates_itself = latest >= have
            && HostConfig::load().is_ok_and(|cfg| cfg.auto_update)
            && install.blocked().is_none();
        if updates_itself {
            println!(
                "{}",
                style::dim(
                    "The host updates itself from now on, to the version its control plane runs (`fctl host auto-update off` turns that off)."
                )
            );
        }
    } else if installed {
        println!("Restart the host service to use it: fctl host restart");
    }
    if refreshed {
        println!("Refreshed your shell completions.");
    }
    Ok(())
}

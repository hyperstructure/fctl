//! Building the default sandbox image (or the Lima base VM) on this host.

use crate::driver::{Driver, lima::BASE_VM, spawn_error};
use anyhow::{Result, bail};
use std::process::Stdio;

pub const DOCKERFILE: &str = include_str!("../../../images/agent/Dockerfile");
pub const PROVISION: &str = include_str!("../../../images/agent/provision.sh");

/// Builds `tag` with the driver's tooling, streaming output to the terminal.
pub async fn build(driver: &Driver, tag: &str) -> Result<()> {
    if driver.name() != "process" {
        let info = driver.probe().await;
        // Lima only needs limactl here: its base VM is what this builds.
        if !info.available && !(driver.name() == "lima" && info.version.is_some()) {
            bail!(
                "The {} driver is not ready: {}",
                info.name,
                info.detail.unwrap_or_default()
            );
        }
    }
    let dir = std::env::temp_dir().join(format!("fctl-image-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("Dockerfile"), DOCKERFILE)?;
    std::fs::write(dir.join("provision.sh"), PROVISION)?;
    let ctx = dir.display().to_string();
    let result = match driver.name() {
        "apple" => {
            run_inherit(
                "container",
                &[
                    "build",
                    "--tag",
                    tag,
                    "--file",
                    &format!("{ctx}/Dockerfile"),
                    &ctx,
                ],
            )
            .await
        }
        "docker" => run_inherit("docker", &["build", "--tag", tag, &ctx]).await,
        "lima" => build_lima_base(&dir).await,
        _ => {
            println!(
                "The process driver runs agents directly on this machine; there is no image to build."
            );
            println!(
                "Install Claude Code here instead: curl -fsSL https://claude.ai/install.sh | bash"
            );
            Ok(())
        }
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

async fn build_lima_base(dir: &std::path::Path) -> Result<()> {
    run_inherit(
        "limactl",
        &[
            "create",
            "--name",
            BASE_VM,
            "--tty=false",
            "template://debian-12",
        ],
    )
    .await?;
    run_inherit("limactl", &["start", "--tty=false", BASE_VM]).await?;
    let script = dir.join("provision.sh").display().to_string();
    run_inherit(
        "limactl",
        &["copy", &script, &format!("{BASE_VM}:/tmp/provision.sh")],
    )
    .await?;
    // Lima's default user becomes the agent user.
    run_inherit(
        "limactl",
        &[
            "shell",
            BASE_VM,
            "--",
            "sh",
            "-c",
            "sudo AGENT_USER=$(id -un) bash /tmp/provision.sh",
        ],
    )
    .await?;
    run_inherit("limactl", &["stop", BASE_VM]).await?;
    println!("The {BASE_VM} VM is ready; sessions are cloned from it.");
    Ok(())
}

async fn run_inherit(program: &str, args: &[&str]) -> Result<()> {
    println!("$ {program} {}", args.join(" "));
    let status = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .await
        .map_err(|e| spawn_error(program, e))?;
    if !status.success() {
        bail!(
            "{program} {} failed ({status})",
            args.first().copied().unwrap_or_default()
        );
    }
    Ok(())
}

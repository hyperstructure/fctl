//! fctl-agent: the supervisor that runs Claude Code inside a sandbox.
//!
//! - `fctl-agent serve` runs the supervisor (started detached by the host).
//! - `fctl-agent attach` connects stdin/stdout to it (the host's channel).
//! - As PID 1 it is a Firecracker microVM's init, and `vm-serve` and
//!   `vm-run` run what the host asks there (src/vm).
//!
//! See docs/architecture.md and crates/fctl-proto/src/agent.rs.

mod attachments;
mod caches;
mod claude;
mod followed;
mod journal;
mod link;
mod local;
mod prompt;
mod refresh;
mod supervisor;
mod tasks;
#[cfg(target_os = "linux")]
mod vm;
mod workspace;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "fctl-agent",
    version,
    about = "Runs Claude Code for fctl inside a sandbox"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the supervisor.
    Serve {
        /// Unix socket the host attaches to.
        #[arg(
            long,
            env = "FCTL_AGENT_SOCKET",
            default_value = "/tmp/fctl/agent.sock"
        )]
        socket: PathBuf,
        /// Where the journal and state live.
        #[arg(long, env = "FCTL_AGENT_STATE", default_value = "/tmp/fctl/state")]
        state: PathBuf,
        /// Log file (default: stderr).
        #[arg(long)]
        log: Option<PathBuf>,
    },
    /// Bridge stdin/stdout to a running supervisor.
    Attach {
        #[arg(
            long,
            env = "FCTL_AGENT_SOCKET",
            default_value = "/tmp/fctl/agent.sock"
        )]
        socket: PathBuf,
    },
    /// Run what the host asks in a Firecracker microVM, under its init.
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    VmServe,
    /// Run a program as a user, in a session of its own (for vm-serve).
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    VmRun {
        uid: u32,
        gid: u32,
        /// Supplementary groups, separated by commas.
        groups: String,
        cwd: String,
        #[arg(last = true, required = true)]
        argv: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    // As a microVM's PID 1, fctl-agent is its init.
    #[cfg(target_os = "linux")]
    if std::process::id() == 1 {
        vm::init();
    }
    let cli = Cli::parse();
    #[cfg(target_os = "linux")]
    if let Command::VmRun {
        uid,
        gid,
        groups,
        cwd,
        argv,
    } = &cli.command
    {
        return vm::run_as(*uid, *gid, groups, cwd, argv);
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    match cli.command {
        Command::Serve { socket, state, log } => {
            init_logging(log.as_deref())?;
            let r = rt.block_on(supervisor::serve(socket, state));
            if let Err(e) = &r {
                tracing::error!("fctl-agent stopped: {e:#}");
            }
            r
        }
        Command::Attach { socket } => rt.block_on(link::attach(&socket)),
        #[cfg(target_os = "linux")]
        Command::VmServe => {
            init_logging(None)?;
            rt.block_on(vm::serve())
        }
        #[cfg(target_os = "linux")]
        Command::VmRun { .. } => Ok(()),
    }
}

fn init_logging(path: Option<&std::path::Path>) -> anyhow::Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_env("FCTL_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    match path {
        Some(p) => {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)?;
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .init();
        }
        None => tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init(),
    }
    Ok(())
}

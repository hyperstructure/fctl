//! fctl-agent: the supervisor that runs Claude Code inside a sandbox.
//!
//! - `fctl-agent serve` runs the supervisor (started detached by the host).
//! - `fctl-agent attach` connects stdin/stdout to it (the host's channel).
//!
//! See docs/architecture.md and crates/fctl-proto/src/agent.rs.

mod attachments;
mod claude;
mod journal;
mod link;
mod local;
mod prompt;
mod supervisor;
mod tasks;
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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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

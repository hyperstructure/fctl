//! fctl-agent: the supervisor that runs Claude Code inside a sandbox.
//!
//! - `fctl-agent serve` runs the supervisor (started detached by the host),
//!   as root with `--user` in container and VM sandboxes (src/user).
//! - `fctl-agent attach` connects stdin/stdout to it (the host's channel).
//! - As PID 1 it is a Firecracker microVM's init, and `vm-serve` runs what
//!   the host asks there (src/vm).
//! - `fctl-agent run-as` runs a program as another user, for `vm-serve` and
//!   for a supervisor that runs as root.
//! - `fctl-agent share`, on a Firecracker host, serves a project's package
//!   caches to a VM over its vsock (src/share), and `vm-mount` in the VM
//!   mounts them.
//!
//! See docs/architecture.md and crates/fctl-proto/src/agent.rs.

mod attachments;
mod caches;
mod claude;
mod followed;
mod journal;
mod link;
mod local;
mod orphan;
mod prompt;
#[cfg(target_os = "linux")]
mod protected;
mod refresh;
#[cfg(target_os = "linux")]
mod share;
mod supervisor;
mod tasks;
mod user;
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
        /// Run as root, and Claude and the commands for the session as this
        /// user (a name or a uid, as `docker exec -u` takes them), whose
        /// processes can't stop fctl-agent then. Its socket, state and log
        /// go where only root may reach them; the session socket and the
        /// GitHub tokens stay in the user's `~/.fctl`. Linux only.
        #[arg(long)]
        user: Option<String>,
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
    /// Serve a directory to a Firecracker VM over 9P, on the unix socket
    /// where Firecracker connects the VM's vsock port (on the host).
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    Share {
        #[arg(long)]
        dir: PathBuf,
        #[arg(long)]
        socket: PathBuf,
    },
    /// Mount what the host shares on a vsock port at a directory, as a user's
    /// files (in a Firecracker VM, as root).
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    VmMount {
        #[arg(long)]
        port: u32,
        #[arg(long)]
        dir: PathBuf,
        /// Whose files they are in the VM: a name or a uid, as `docker exec
        /// -u` takes them.
        #[arg(long)]
        user: Option<String>,
    },
    /// Run a program as a user, in a session of its own (for vm-serve, and
    /// a supervisor that runs as root).
    #[cfg(target_os = "linux")]
    #[command(hide = true)]
    RunAs {
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
    if let Command::RunAs {
        uid,
        gid,
        groups,
        cwd,
        argv,
    } = &cli.command
    {
        return user::run_as::run_as(*uid, *gid, groups, cwd, argv);
    }
    // Built once a supervisor that runs as root has taken its user's
    // identity, which applies to the threads there are.
    let runtime = || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
    };
    match cli.command {
        Command::Serve {
            socket,
            state,
            log,
            user,
        } => {
            init_logging(log.as_deref())?;
            let r = match user {
                Some(user) => serve_as_root(&user, socket, state, log.as_deref()),
                None => runtime()?.block_on(supervisor::serve(socket, state)),
            };
            if let Err(e) = &r {
                tracing::error!("fctl-agent stopped: {e:#}");
            }
            r
        }
        Command::Attach { socket } => runtime()?.block_on(link::attach(&socket)),
        #[cfg(target_os = "linux")]
        Command::VmServe => {
            // To the VM's console, which the host keeps in a file.
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("FCTL_LOG")
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
                )
                .with_ansi(false)
                .with_writer(std::io::stderr)
                .init();
            runtime()?.block_on(vm::serve())
        }
        #[cfg(target_os = "linux")]
        Command::Share { dir, socket } => {
            init_logging(None)?;
            share::serve(&dir, &socket)
        }
        #[cfg(target_os = "linux")]
        Command::VmMount { port, dir, user } => vm::mount(port, &dir, user.as_deref()),
        #[cfg(target_os = "linux")]
        Command::RunAs { .. } => Ok(()),
    }
}

/// `serve --user`: readies what only root may do, takes the user's identity
/// ([`user::protect`]), then serves.
#[cfg(target_os = "linux")]
fn serve_as_root(
    user: &str,
    socket: PathBuf,
    state: PathBuf,
    log: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    match protected::start(user, &socket, &state, log)? {
        protected::Start::Serving(started) => tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?
            .block_on(supervisor::serve_protected(*started)),
        protected::Start::Taken => Ok(()),
    }
}

#[cfg(not(target_os = "linux"))]
fn serve_as_root(
    _: &str,
    _: PathBuf,
    _: PathBuf,
    _: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    anyhow::bail!("fctl-agent serve --user runs on Linux only")
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

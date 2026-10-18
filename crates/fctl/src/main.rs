//! `fctl`: the command line for fctl, the software factory.
//!
//! People and agents use the same commands; every command that prints data
//! accepts `--json`. `FCTL_SERVER` and `FCTL_TOKEN` configure it without a
//! sign-in (CI, agents). `fctl host …` runs this machine as a runner.

mod cmd;
mod config;
mod render;
mod style;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fctl",
    version,
    about = "A software factory: start Claude agents, follow them, run a host",
    long_about = "fctl starts Claude Code agents on your projects, streams what they do, and lets you message them. \
                  `fctl host` turns this machine into a runner that executes sessions in sandboxes.",
    after_help = "Examples:\n  fctl login https://app.fctl.sh\n  fctl run -p my-app \"Add a dark mode toggle\"\n  fctl sessions --active\n  fctl attach my-app#12\n  fctl host install"
)]
struct Cli {
    /// The fctl instance (default: the one you signed in to).
    #[arg(long, global = true, env = "FCTL_SERVER", hide_env_values = true)]
    server: Option<String>,
    /// Print machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in (opens the browser to approve this device).
    Login {
        /// Instance URL, e.g. https://app.fctl.sh
        url: Option<String>,
        /// Print the link instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Sign out on this machine.
    Logout,
    /// Show who you are signed in as.
    Whoami,
    /// List, create and inspect projects.
    #[command(subcommand)]
    Projects(cmd::projects::ProjectsCmd),
    /// Start a session: an agent working on a task.
    Run(cmd::sessions::RunArgs),
    /// List sessions.
    Sessions(cmd::sessions::ListArgs),
    /// Show a session's details.
    Show {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Rename a session or say what it is about (agents: the session they run in).
    #[command(
        after_help = "Examples:\n  fctl edit my-app#12 --title \"Fix the flaky login test\"\n  \
                            fctl edit my-app#12 --description \"\"       # clear the description\n  \
                            fctl edit --title \"Retry failed uploads\"   # an agent, in its sandbox"
    )]
    Edit(cmd::sessions::EditArgs),
    /// Follow a session live and message the agent (Ctrl-C detaches).
    Attach {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
        /// Follow without sending messages.
        #[arg(long)]
        read_only: bool,
    },
    /// Print a session's transcript.
    Logs {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
        /// Keep following new events.
        #[arg(short, long)]
        follow: bool,
    },
    /// Send a message to a session's agent.
    Send {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
        /// The message (or pipe it in).
        message: Vec<String>,
        /// Send a file with it (repeat for more, up to 10).
        #[arg(short, long = "attach", value_name = "FILE")]
        attach: Vec<std::path::PathBuf>,
    },
    /// Interrupt the agent's current turn.
    Interrupt {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Stop a session (its sandbox is kept so it can resume).
    Stop {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Resume a stopped session, optionally with a new message.
    Resume {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
        /// A message for the agent when it resumes.
        message: Vec<String>,
        /// Send a file with the message (repeat for more, up to 10).
        #[arg(short, long = "attach", value_name = "FILE")]
        attach: Vec<std::path::PathBuf>,
    },
    /// Follow a session: notifications reach you when it waits for your
    /// reply, pauses on a usage limit or fails.
    Follow {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Stop following a session (no more notifications about it).
    Unfollow {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Open a session or project in the browser (or the Claude app with --claude).
    Open {
        /// A session or project (default: the instance).
        target: Option<String>,
        /// Open the session's Remote Control link (Claude apps) instead.
        #[arg(long)]
        claude: bool,
    },
    /// Connect your Claude subscription.
    #[command(subcommand)]
    Claude(cmd::claude::ClaudeCmd),
    /// Choose what notifies you, and see the browsers that show it.
    #[command(subcommand)]
    Notifications(cmd::notifications::NotificationsCmd),
    /// Run this machine as a host for agent sandboxes.
    #[command(subcommand)]
    Host(cmd::host::HostCmd),
    /// Manage access, users and runners (admins).
    #[command(subcommand)]
    Admin(cmd::admin::AdminCmd),
    /// Export your data or delete your account.
    #[command(subcommand)]
    Account(cmd::account::AccountCmd),
    /// Create a personal access token for scripts and CI.
    Token {
        /// What the token is for, e.g. "deploy bot".
        name: String,
        /// Expire after this many days (default: never).
        #[arg(long)]
        days: Option<u32>,
    },
    /// Update fctl itself.
    #[command(name = "self", subcommand)]
    SelfCmd(cmd::selfcmd::SelfCmd),
    /// Shell completions: print them, or set them up with --install.
    Completions {
        /// bash, zsh, fish, elvish or powershell (with --install: yours).
        shell: Option<clap_complete::Shell>,
        /// Set them up for your shell (zsh, bash or fish).
        #[arg(long)]
        install: bool,
        /// Rewrite the scripts installed earlier (fctl self update does this).
        #[arg(long, conflicts_with = "install")]
        refresh: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    let filter = tracing_subscriber::EnvFilter::try_from_env("FCTL_LOG").unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(
            if matches!(cli.command, Command::Host(cmd::host::HostCmd::Run)) {
                "info"
            } else {
                "warn"
            },
        )
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("fctl: {e}");
            std::process::exit(70);
        }
    };
    let code = match rt.block_on(run(cli)) {
        Ok(()) => 0,
        Err(e) => {
            if let Some(api) = e.downcast_ref::<fctl_client::Error>().and_then(|e| e.api()) {
                eprintln!("{} {}", style::red("error:"), api.message);
            } else {
                eprintln!("{} {e:#}", style::red("error:"));
            }
            1
        }
    };
    std::process::exit(code);
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let server = cli.server.as_deref();
    let json = cli.json;
    match cli.command {
        Command::Login { url, no_browser } => {
            cmd::auth::login(url.as_deref().or(server), no_browser, json).await
        }
        Command::Logout => cmd::auth::logout(server).await,
        Command::Whoami => cmd::auth::whoami(server, json).await,
        Command::Projects(c) => cmd::projects::run(server, c, json).await,
        Command::Run(args) => cmd::sessions::start(server, args, json).await,
        Command::Sessions(args) => cmd::sessions::list(server, args, json).await,
        Command::Show { session } => cmd::sessions::show(server, &session, json).await,
        Command::Edit(args) => cmd::sessions::edit(server, args, json).await,
        Command::Attach { session, read_only } => {
            cmd::attach::attach(server, &session, read_only).await
        }
        Command::Logs { session, follow } => {
            cmd::sessions::logs(server, &session, follow, json).await
        }
        Command::Send {
            session,
            message,
            attach,
        } => cmd::sessions::send(server, &session, &message.join(" "), &attach, json).await,
        Command::Interrupt { session } => cmd::sessions::interrupt(server, &session).await,
        Command::Stop { session } => cmd::sessions::stop(server, &session, json).await,
        Command::Resume {
            session,
            message,
            attach,
        } => cmd::sessions::resume(server, &session, &message.join(" "), &attach, json).await,
        Command::Follow { session } => cmd::sessions::follow(server, &session, true, json).await,
        Command::Unfollow { session } => cmd::sessions::follow(server, &session, false, json).await,
        Command::Open { target, claude } => {
            cmd::sessions::open(server, target.as_deref(), claude).await
        }
        Command::Claude(c) => cmd::claude::run(server, c, json).await,
        Command::Notifications(c) => cmd::notifications::run(server, c, json).await,
        Command::Host(c) => cmd::host::run(server, c, json).await,
        Command::Admin(c) => cmd::admin::run(server, c, json).await,
        Command::Account(c) => cmd::account::run(server, c, json).await,
        Command::Token { name, days } => cmd::auth::token(server, &name, days, json).await,
        Command::SelfCmd(c) => cmd::selfcmd::run(c, json).await,
        Command::Completions {
            shell,
            install,
            refresh,
        } => cmd::completions::run(shell, install, refresh),
    }
}

//! `fctl`: the command line for fctl, the software factory.
//!
//! People and agents use the same commands; every command that prints data
//! accepts `--json`. `FCTL_SERVER` and `FCTL_TOKEN` configure it without a
//! sign-in (CI, agents). `fctl host …` runs this machine as a runner.

mod cmd;
mod config;
mod prompt;
mod render;
mod stall;
mod style;
mod verify;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "fctl",
    version,
    about = "A software factory: start Claude agents, follow them, run a host",
    long_about = "fctl starts Claude Code agents on your projects, streams what they do, and lets you message them. \
                  `fctl host` turns this machine into a runner that executes sessions in sandboxes.",
    after_help = "Examples:
  fctl login https://app.fctl.sh
  fctl run -p my-app \"Add a dark mode toggle\"
  fctl sessions --active
  fctl attach my-app#12
  fctl host install

Agents: read https://fctl.sh/llms.txt to learn fctl and this CLI: what fctl is, the words it \
uses, how to drive it from a script or a tool call (access tokens, --json, exit codes, waiting \
for a session), what to avoid, recipes, and links to every doc page as Markdown, all of them in \
one file at https://fctl.sh/llms-full.txt."
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
    /// Your workspaces: their people, runners, GitHub accounts and rules.
    #[command(visible_alias = "workspace")]
    Workspaces {
        #[command(subcommand)]
        cmd: Option<cmd::workspaces::WorkspacesCmd>,
    },
    /// List, create and change projects: their settings, members, secrets
    /// and repositories.
    #[command(subcommand)]
    Projects(cmd::projects::ProjectsCmd),
    /// Start a session: an agent working on a task.
    Run(cmd::sessions::RunArgs),
    /// List sessions.
    Sessions(cmd::sessions::ListArgs),
    /// Search sessions: their titles and descriptions, what people wrote to
    /// their agents and what the agents replied, in every session you see.
    #[command(after_help = "Examples:
  fctl search login redirect           # found as typed, whatever the case and accents
  fctl search \"database migration\" -p my-app
  fctl search --json flaky | jq -r '.sessions[].session.id'")]
    Search(cmd::search::SearchArgs),
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
    /// Publish screenshots for a pull request at links anyone who has one
    /// can open, until they expire (agents, in the sandbox of a session whose
    /// project publishes screenshots). Prints Markdown that shows each.
    #[command(
        after_help = "GitHub takes no uploads from a session's token, so `gh pr edit --attach` fails
                      there. Put the Markdown this prints in the pull request instead.

                      Examples:
  fctl publish before.png after.png
                        fctl publish --json after.png | jq -r '.published[].markdown'"
    )]
    Publish(cmd::publish::PublishArgs),
    /// Resolve review threads of a pull request (agents that review and
    /// comment, in their session's sandbox): GitHub lets only tokens that
    /// can push resolve them, so fctl does it for them.
    #[command(after_help = "Examples:\n  fctl resolve PRRT_kwDOABCD1234\n  \
                            fctl resolve PRRT_kwDOABCD1234 PRRT_kwDOEFGH5678\n  \
                            fctl resolve --unresolve PRRT_kwDOABCD1234")]
    Resolve(cmd::resolve::ResolveArgs),
    /// Read and write the project's tickets (agents, in the sandbox of a
    /// session whose project tracks its work in Linear, or in one
    /// repository's GitHub issues): fctl reaches the tracker for them,
    /// whatever it is.
    #[command(after_help = "Examples:
  fctl ticket                          # the tracker, and the ticket this session works on
  fctl ticket list --search \"login\"
  fctl ticket view ENG-123
  fctl ticket comment ENG-123 \"The fix needs a migration first: should it run in the deploy?\"
  fctl ticket create --title \"Add a test for retries\" --parent ENG-123
  fctl ticket update ENG-123 --state \"In Review\" --add-label Bug")]
    Ticket(cmd::ticket::TicketArgs),
    /// Delete published screenshots before they expire (people who manage
    /// their session).
    Unpublish {
        /// Links to published screenshots.
        #[arg(required = true, value_name = "LINK")]
        links: Vec<String>,
    },
    /// A project's memory: notes agents and people keep for the agents of
    /// later sessions, which each session's agent starts with. In a
    /// session's sandbox, the agent keeps its project's.
    #[command(
        after_help = "Examples:\n  fctl memory                        # the notes, newest first\n  \
                            fctl memory -p my-app --stale      # with the stale ones\n  \
                            fctl memory add \"The e2e tests need Chromium: run npx playwright install chromium first.\"\n  \
                            fctl memory stale mem_01JC3V… --reason \"The script moved to xtask\"\n  \
                            fctl memory duplicate mem_01JC3V… --of mem_01JC2Q…\n  \
                            fctl memory rm mem_01JC3V…         # people who can edit the project"
    )]
    Memory(cmd::memory::MemoryArgs),
    /// Record a retrospective as you finish a piece of work, and update it
    /// as later turns carry on with it (agents, in a session's sandbox), or
    /// read a project's (`fctl retro list`).
    #[command(
        after_help = "Examples:\n  fctl retro --outcome completed --worked-on \"Retried failed uploads (#41).\" \\\n    \
                            --approach \"Used the existing backoff helper.\" --challenges \"The tests need MinIO.\" \\\n    \
                            --insights \"Start MinIO with docker compose up minio first.\"\n  \
                            fctl retro update rtr_01JC3W… --outcome completed   # the parts left out stay\n  \
                            fctl retro list -p my-app --outcome blocked\n  \
                            fctl retro list my-app#12"
    )]
    Retro(cmd::retro::RetroArgs),
    /// Review agents: a project's reviews of its pull requests, asking for
    /// one, and turning them on or off (people); reading fctl's earlier
    /// findings and posting a review (review agents, in their sandbox).
    #[command(after_help = "Examples:
  fctl review                          # the project's reviews, newest first
                              fctl review settings --on --effort high
                              fctl review start acme/web#12
                              fctl review threads                  # a review agent, in its sandbox
                              fctl review submit review.json       # a review agent, in its sandbox")]
    Review(cmd::review::ReviewArgs),
    /// Schedules: tasks fctl starts a session for on its own, once at a set
    /// time or again and again, on the Claude subscription of the person
    /// each runs for. Run one now, or from another tool with an access
    /// token.
    #[command(
        visible_alias = "schedule",
        after_help = "Examples:
  fctl schedules                       # the project's schedules
  fctl schedules add \"Weekly dependencies\" --cron \"0 9 * * 1\" -m \"Update the dependencies…\"
  fctl schedules add --at 2h -m \"Check whether the deploy went out.\"
  fctl schedules run \"Weekly dependencies\"
  fctl schedules pause sch_01JC…
  fctl schedules runs sch_01JC…"
    )]
    Schedules(cmd::schedules::SchedulesArgs),
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
    /// Send a message to a session's agent. A stopped session resumes for
    /// it, on the Claude subscription it ran on.
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
    /// Resume a stopped or failed session on your Claude subscription,
    /// optionally with a new message.
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
    /// Share a session with someone in its workspace, or change what they
    /// may do in it (its owners); without them, list who it is shared
    /// with. Its project's people see it already.
    #[command(
        after_help = "Whoever can message a session's agent can have it hand over what its sandbox holds: the project's secrets, its GitHub tokens and the Claude token it runs on.

Examples:
  fctl share my-app#12                                  # who it is shared with
  fctl share my-app#12 grace@example.com                # they view it and message its agent
  fctl share my-app#12 grace@example.com --role viewer  # they only view it
  fctl unshare my-app#12 grace@example.com"
    )]
    Share(cmd::sessions::ShareArgs),
    /// Stop sharing a session with someone (its owners).
    Unshare {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
        /// Their address in the session's workspace, or their user id.
        person: String,
    },
    /// Archive a session (its owners): lists leave it out, and nothing that
    /// happens to its pull requests reaches its agent. Messages still do.
    Archive {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Bring an archived session back into the lists (its owners).
    Unarchive {
        /// A session: its id (ses_…), its URL, or project#number.
        session: String,
    },
    /// Open a session or project in the browser (or the Claude app with --claude).
    Open {
        /// A session or project (default: the instance).
        target: Option<String>,
        /// Open the session in the Claude app instead, or on claude.ai without one.
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
    /// Manage people, invitations, workspaces, GitHub and runners
    /// (instance admins).
    #[command(subcommand)]
    Admin(cmd::admin::AdminCmd),
    /// Set the address commits credit you with, set up two-factor sign-in,
    /// export your data, or delete your account.
    #[command(subcommand)]
    Account(cmd::account::AccountCmd),
    /// Create a personal access token for scripts, CI and bots, limited to
    /// what it is for.
    Token(cmd::tokens::TokenArgs),
    /// Where you are signed in and your access tokens, and revoke them.
    Tokens {
        #[command(subcommand)]
        cmd: Option<cmd::tokens::TokensCmd>,
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
                if api.code == fctl_proto::ErrorCode::VerificationRequired {
                    eprintln!(
                        "{}",
                        style::dim(
                            "In a terminal, fctl asks for it. Without one, confirm first: `fctl account verify --code <code>`."
                        )
                    );
                }
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
        Command::Workspaces { cmd: c } => {
            let c = c.unwrap_or(cmd::workspaces::WorkspacesCmd::List);
            cmd::workspaces::run(server, c, json).await
        }
        Command::Projects(c) => cmd::projects::run(server, c, json).await,
        Command::Run(args) => cmd::sessions::start(server, args, json).await,
        Command::Sessions(args) => cmd::sessions::list(server, args, json).await,
        Command::Search(args) => cmd::search::run(server, args, json).await,
        Command::Show { session } => cmd::sessions::show(server, &session, json).await,
        Command::Edit(args) => cmd::sessions::edit(server, args, json).await,
        Command::Publish(args) => cmd::publish::publish(args, json).await,
        Command::Unpublish { links } => cmd::publish::unpublish(server, &links, json).await,
        Command::Memory(args) => cmd::memory::run(server, args, json).await,
        Command::Retro(args) => cmd::retro::run(server, args, json).await,
        Command::Review(args) => cmd::review::run(server, args, json).await,
        Command::Schedules(args) => cmd::schedules::run(server, args, json).await,
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
        Command::Share(args) => cmd::sessions::share(server, args, json).await,
        Command::Unshare { session, person } => {
            cmd::sessions::unshare(server, &session, &person, json).await
        }
        Command::Archive { session } => cmd::sessions::archive(server, &session, true, json).await,
        Command::Unarchive { session } => {
            cmd::sessions::archive(server, &session, false, json).await
        }
        Command::Open { target, claude } => {
            cmd::sessions::open(server, target.as_deref(), claude).await
        }
        Command::Claude(c) => cmd::claude::run(server, c, json).await,
        Command::Notifications(c) => cmd::notifications::run(server, c, json).await,
        Command::Host(c) => cmd::host::run(server, c, json).await,
        Command::Admin(c) => cmd::admin::run(server, c, json).await,
        Command::Account(c) => cmd::account::run(server, c, json).await,
        Command::Token(args) => cmd::tokens::create(server, args, json).await,
        Command::Tokens { cmd: c } => {
            let c = c.unwrap_or(cmd::tokens::TokensCmd::List);
            cmd::tokens::run(server, c, json).await
        }
        Command::Resolve(args) => cmd::resolve::resolve(args, json).await,
        Command::Ticket(args) => cmd::ticket::run(args, json).await,
        Command::SelfCmd(c) => cmd::selfcmd::run(c, json).await,
        Command::Completions {
            shell,
            install,
            refresh,
        } => cmd::completions::run(shell, install, refresh),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn every_command_is_well_formed() {
        // Clap's own checks: no flag defined twice in a command, groups of
        // flags that exist, and the like.
        Cli::command().debug_assert();
    }

    #[test]
    fn the_help_points_agents_to_llms_txt() {
        // `fctl` alone prints the short help, as `-h` does; `--help` and
        // `fctl help` the long one. Wrapped to the terminal's width.
        for line in ["fctl", "fctl -h", "fctl --help", "fctl help"] {
            let Err(e) = Cli::try_parse_from(line.split(' ')) else {
                panic!("`{line}` ran a command");
            };
            let help = e.render().to_string();
            let words = help.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                words.contains("Agents: read https://fctl.sh/llms.txt to learn fctl"),
                "`{line}` printed {help}"
            );
        }
    }

    #[test]
    fn sessions_are_shared_and_archived_from_the_top() {
        let parse = |args: &[&str]| {
            Cli::try_parse_from(std::iter::once("fctl").chain(args.iter().copied()))
                .map(|c| c.command)
        };
        assert!(matches!(
            parse(&["unshare", "my-app#12", "grace@example.com"]),
            Ok(Command::Unshare { .. })
        ));
        assert!(matches!(
            parse(&["archive", "my-app#12", "--json"]),
            Ok(Command::Archive { .. })
        ));
        assert!(matches!(
            parse(&["unarchive", "ses_01ABC"]),
            Ok(Command::Unarchive { .. })
        ));
        assert!(matches!(
            parse(&["sessions", "--archived", "-p", "my-app"]),
            Ok(Command::Sessions(cmd::sessions::ListArgs {
                archived: true,
                ..
            }))
        ));
        assert!(matches!(
            parse(&["projects", "secrets", "set", "STRIPE_KEY", "-p", "my-app"]),
            Ok(Command::Projects(cmd::projects::ProjectsCmd::Secrets(_)))
        ));
        assert!(matches!(
            parse(&["projects", "members", "-p", "my-app", "--json"]),
            Ok(Command::Projects(cmd::projects::ProjectsCmd::Members(_)))
        ));
        // A secret's value never comes as an argument.
        assert!(parse(&["projects", "secrets", "set", "STRIPE_KEY", "sk_live_123"]).is_err());
    }
}

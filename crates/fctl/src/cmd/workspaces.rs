//! `fctl workspaces …`: the workspaces you are in, their people, runners,
//! GitHub accounts and settings.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand, ValueEnum};
use fctl_client::Api;
use fctl_proto::api::{
    BaseRole, ConnectGithubRequest, CreateWorkspaceRequest, GithubConnectionStatus,
    InviteToWorkspaceRequest, Runner, UpdateMembershipRequest, UpdateWorkspaceRequest, Workspace,
    WorkspaceInvitation, WorkspaceMember,
};
use fctl_proto::session::{WorkspaceColor, WorkspaceRole};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Clone, Copy, ValueEnum)]
pub enum Color {
    Yellow,
    Pink,
    Blue,
    Ink,
}

impl From<Color> for WorkspaceColor {
    fn from(c: Color) -> Self {
        match c {
            Color::Yellow => Self::Yellow,
            Color::Pink => Self::Pink,
            Color::Blue => Self::Blue,
            Color::Ink => Self::Ink,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Base {
    /// Members see the projects they are members of.
    None,
    /// Members see every project's sessions.
    Viewer,
    /// Members start sessions and message agents in every project.
    Editor,
    /// Members also change every project's settings, secrets and members.
    Maintainer,
}

impl From<Base> for BaseRole {
    fn from(b: Base) -> Self {
        match b {
            Base::None => Self::None,
            Base::Viewer => Self::Viewer,
            Base::Editor => Self::Editor,
            Base::Maintainer => Self::Maintainer,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Role {
    Admin,
    Member,
}

#[derive(Subcommand)]
pub enum WorkspacesCmd {
    /// List your workspaces, and the invitations waiting for you.
    List,
    /// Show a workspace.
    Show {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Create a workspace: you are its primary admin.
    #[command(
        after_help = "Examples:\n  fctl workspaces create \"Side project\" --color pink\n  fctl workspaces create Acme --as ada.lovelace@company.example"
    )]
    Create {
        /// Its name.
        name: String,
        /// Its address in URLs (default: from the name).
        #[arg(long)]
        slug: Option<String>,
        /// Its avatar's color, until you upload an image.
        #[arg(long, value_enum)]
        color: Option<Color>,
        /// The address you are a member as, one of yours (default: your
        /// primary one). Its people see it, and your commits in its
        /// projects credit its commit email.
        #[arg(long = "as", value_name = "EMAIL")]
        email: Option<String>,
    },
    /// Change a workspace's name, address, avatar, rules or setup (admins).
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["name", "slug", "color", "avatar", "base_role", "require_two_factor", "skip_invites"])),
        after_help = "Examples:\n  fctl workspaces set --name hyperstructure --avatar logo.png\n  fctl workspaces set -w side-project --base-role editor\n  fctl workspaces set --require-two-factor on\n  fctl workspaces set -w side-project --skip-invites on"
    )]
    Set {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
        /// Its name.
        #[arg(long)]
        name: Option<String>,
        /// Its address in URLs; links with the old one stop working.
        #[arg(long)]
        slug: Option<String>,
        /// Its avatar's color, when it has no image.
        #[arg(long, value_enum)]
        color: Option<Color>,
        /// An image for its avatar (PNG, JPEG, GIF or WebP, square works
        /// best), or `none` for its initial on its color.
        #[arg(long, value_name = "FILE|none")]
        avatar: Option<String>,
        /// The role every member has in every project, on top of their own.
        #[arg(long, value_enum)]
        base_role: Option<Base>,
        /// Members need two-factor sign-in: it is closed to those without.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        require_two_factor: Option<bool>,
        /// Leave inviting people out of the app's setup checklist, as in a
        /// workspace for one; `off` asks for it again.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        skip_invites: Option<bool>,
    },
    /// Use this workspace by default.
    Use {
        /// The workspace (slug or id).
        workspace: String,
    },
    /// Put your workspaces in the order you want them, in lists and the
    /// app's sidebar: the ones you name first, the others after them as
    /// they were.
    #[command(
        after_help = "Examples:\n  fctl workspaces order side-project acme   # these two first\n  fctl workspaces order acme                # acme first"
    )]
    Order {
        /// Your workspaces (slugs or ids), in order.
        #[arg(required = true, value_name = "WORKSPACE")]
        workspaces: Vec<String>,
    },
    /// The people in a workspace, and the invitations nobody answered yet.
    Members {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Invite someone to a workspace, by an address they sign in with (admins).
    Invite {
        /// An address they sign in with.
        email: String,
        /// Make them an admin.
        #[arg(long)]
        admin: bool,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Take back an invitation (admins).
    Uninvite {
        /// The address the invitation is for.
        email: String,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Make someone an admin or a member (admins).
    Role {
        /// Their address in the workspace, or their user id.
        person: String,
        /// Admins manage the workspace and own all its projects.
        #[arg(value_enum)]
        role: Role,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Remove someone from a workspace (admins). Their sessions there stop.
    Remove {
        /// Their address in the workspace, or their user id.
        person: String,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Hand the primary admin role to another admin (the primary admin).
    Transfer {
        /// Their address in the workspace, or their user id.
        person: String,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Join a workspace you are invited to.
    Join {
        /// The workspace (its slug or name) or the invitation's id.
        workspace: String,
    },
    /// Decline an invitation to a workspace.
    Decline {
        /// The workspace (its slug or name) or the invitation's id.
        workspace: String,
    },
    /// Leave a workspace. Your sessions there stop.
    Leave {
        /// The workspace (slug or id).
        workspace: String,
    },
    /// The runners a workspace's sessions run on.
    Runners {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Let one of your runners run a workspace's sessions (its admins).
    #[command(
        after_help = "Examples:\n  fctl workspaces add-runner -w side-project              # on the runner: this machine\n  fctl workspaces add-runner studio-mac -w side-project\n  fctl workspaces add-runner rnr_01… -w side-project      # an id from `fctl admin runners`"
    )]
    AddRunner {
        /// The runner's name or id (default: this machine's, on a runner).
        runner: Option<String>,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Stop a runner from taking a workspace's sessions. Running ones carry on.
    RemoveRunner {
        /// The runner's name or id (default: this machine's, on a runner).
        runner: Option<String>,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// The GitHub accounts a workspace works in.
    Github {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Let a workspace work in a GitHub account the instance's GitHub App
    /// is installed on (admins). Opens a GitHub sign-in in your browser,
    /// which shows what you may connect: every repository of your own
    /// account, or of an organization whose every repository the app
    /// reaches and you are an admin of; otherwise the repositories you are
    /// an admin of. You choose from it on the workspace's GitHub tab,
    /// unless you name the repositories here.
    #[command(
        after_help = "Examples:\n  fctl workspaces connect\n  fctl workspaces connect hyperstructure\n  fctl workspaces connect acme --repo web --repo design"
    )]
    Connect {
        /// The GitHub account (organization or user). Without it, you
        /// choose from every account GitHub shows you the app on.
        account: Option<String>,
        /// Only this repository of it (repeatable), connected once you sign
        /// in. Without it, you choose the repositories after signing in.
        #[arg(long = "repo", value_name = "NAME", requires = "account")]
        repos: Vec<String>,
        /// Print the link without opening a browser.
        #[arg(long)]
        no_browser: bool,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// Stop a workspace from working in a GitHub account (admins). Its
    /// sessions get no more tokens for it.
    Disconnect {
        /// The GitHub account (organization or user).
        account: String,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// The Claude connection your sessions in a workspace run on.
    Claude {
        /// One of your connections, by its id or name in `fctl claude
        /// status` (an account or a label), or `default` for your default
        /// one. Without it, shows the one in use.
        credential: Option<String>,
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
    },
    /// What happened in a workspace (admins).
    Audit {
        /// The workspace (slug or id); default from `fctl workspaces use`,
        /// else your only one.
        #[arg(short, long)]
        workspace: Option<String>,
        /// Entries before this time (milliseconds), for the page after
        /// one's `next_before`.
        #[arg(long, value_name = "MS")]
        before: Option<i64>,
    },
    /// Delete a workspace with its projects, sessions and transcripts (its
    /// primary admin). This cannot be undone.
    Delete {
        /// The workspace (slug or id).
        workspace: String,
        /// The workspace's name, to confirm without being asked (for scripts).
        #[arg(long, value_name = "NAME")]
        confirm: Option<String>,
    },
}

/// The workspace a command is about: the one named, the default one, or
/// your only one.
pub async fn resolve(api: &Api, named: Option<String>) -> Result<Workspace> {
    if let Some(w) = named.or_else(|| config::CliConfig::load().workspace) {
        return Ok(api.workspace(&w).await?);
    }
    let list = api.workspaces().await?;
    match list.workspaces.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!(
            "You aren't in a workspace yet. Create one with `fctl workspaces create <name>`, or join one you are invited to."
        ),
        more => bail!(
            "Which workspace? Pass --workspace, or set a default with `fctl workspaces use <workspace>`. Yours: {}",
            more.iter()
                .map(|w| w.slug.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Someone in a workspace, by their address there or their user id.
async fn person(api: &Api, workspace: &Workspace, who: &str) -> Result<WorkspaceMember> {
    let members = api.workspace_members(workspace.id.as_str()).await?.members;
    members
        .into_iter()
        .find(|m| m.user.id.as_str() == who || m.email.eq_ignore_ascii_case(who))
        .with_context(|| {
            format!(
                "Nobody in {} is {who}. `fctl workspaces members -w {}` lists them.",
                workspace.name, workspace.slug
            )
        })
}

/// The runner named (its name or id), else this machine's.
async fn runner_or_this(api: &Api, which: Option<&str>) -> Result<Runner> {
    match which {
        Some(which) => super::admin::find_runner(api, which).await,
        None => super::host::this_runner(api).await,
    }
}

/// "primary admin", "admin", "member".
pub fn role(r: WorkspaceRole) -> &'static str {
    match r {
        WorkspaceRole::Owner => "primary admin",
        WorkspaceRole::Admin => "admin",
        WorkspaceRole::Member => "member",
    }
}

fn base(b: BaseRole) -> &'static str {
    match b {
        BaseRole::None => "see the projects they are members of",
        BaseRole::Viewer => "see every project's sessions",
        BaseRole::Editor => "start sessions in every project",
        BaseRole::Maintainer => "start sessions in and manage every project",
    }
}

fn print_workspace(w: &Workspace) {
    let mark = if w.blocked {
        style::amber("closed until you set up two-factor sign-in")
    } else {
        style::dim(role(w.role))
    };
    println!("{:<24} {:<28} {}", style::bold(&w.slug), w.name, mark);
}

fn print_invitation(i: &WorkspaceInvitation) {
    let from = i
        .invited_by
        .as_ref()
        .map(|u| format!(" by {}", u.name))
        .unwrap_or_default();
    let mut line = format!(
        "{:<24} {:<28} {}",
        style::bold(&i.workspace.slug),
        i.workspace.name,
        style::accent(&format!("invited as {}{from}", i.email))
    );
    if i.needs_two_factor {
        line.push_str(&format!(
            "  {}",
            style::amber("set up two-factor sign-in to join")
        ));
    }
    println!("{line}");
}

fn confirm_name(name: &str) -> Result<()> {
    let stdin = std::io::stdin();
    if !stdin.is_terminal() {
        bail!("Pass --confirm \"{name}\" to delete it without being asked");
    }
    print!("Type the workspace's name ({name}) to delete it: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    if line.trim() != name {
        bail!("Not deleted: the name didn't match");
    }
    Ok(())
}

pub async fn run(server: Option<&str>, cmd: WorkspacesCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        WorkspacesCmd::List => {
            let list = api.workspaces().await?;
            if json {
                return style::print_json(&list);
            }
            if list.workspaces.is_empty() && list.invitations.is_empty() {
                println!(
                    "You aren't in a workspace yet. Create one with `fctl workspaces create <name>`."
                );
            }
            for w in &list.workspaces {
                print_workspace(w);
            }
            if !list.invitations.is_empty() {
                println!();
                for i in &list.invitations {
                    print_invitation(i);
                }
                println!(
                    "{}",
                    style::dim("Join one with `fctl workspaces join <workspace>`.")
                );
            }
        }
        WorkspacesCmd::Show { workspace } => {
            let w = resolve(&api, workspace).await?;
            if json {
                return style::print_json(&w);
            }
            println!(
                "{} {}",
                style::bold(&w.name),
                style::dim(&format!("({})", w.slug))
            );
            println!("you:      {}, as {}", role(w.role), w.email);
            if let Some(p) = &w.primary {
                println!("primary:  {}", p.name);
            }
            println!("members:  {} — they {}", w.members, base(w.base_role));
            println!("projects: {}", w.projects);
            println!("runners:  {} ({} online)", w.runners, w.runners_online);
            println!(
                "github:   {}",
                if w.github.is_empty() {
                    "—".to_owned()
                } else {
                    w.github.join(", ")
                }
            );
            println!(
                "2fa:      {}",
                if w.require_two_factor {
                    "required of members"
                } else {
                    "up to each member"
                }
            );
            if w.blocked {
                println!(
                    "{}",
                    style::amber(
                        "It requires two-factor sign-in, which you haven't set up: run `fctl account two-factor enable`."
                    )
                );
            }
            println!("{}", style::dim(&format!("{}/w/{}", api.base(), w.slug)));
        }
        WorkspacesCmd::Create {
            name,
            slug,
            color,
            email,
        } => {
            let w = api
                .create_workspace(&CreateWorkspaceRequest {
                    name,
                    slug,
                    color: color.map(Into::into),
                    email,
                })
                .await?;
            if json {
                return style::print_json(&w);
            }
            println!(
                "{} Created {} {}",
                style::accent("✓"),
                style::bold(&w.slug),
                style::dim(&format!("{}/w/{}", api.base(), w.slug))
            );
            println!(
                "{}",
                style::dim(
                    "Next: invite people (fctl workspaces invite), add a runner (fctl host install --workspace), connect GitHub (fctl workspaces connect)."
                )
            );
        }
        WorkspacesCmd::Set {
            workspace,
            name,
            slug,
            color,
            avatar,
            base_role,
            require_two_factor,
            skip_invites,
        } => {
            let w = resolve(&api, workspace).await?;
            let mut out = w.clone();
            if name.is_some()
                || slug.is_some()
                || color.is_some()
                || base_role.is_some()
                || require_two_factor.is_some()
                || skip_invites.is_some()
            {
                out = api
                    .update_workspace(
                        w.id.as_str(),
                        &UpdateWorkspaceRequest {
                            name,
                            slug,
                            color: color.map(Into::into),
                            base_role: base_role.map(Into::into),
                            require_two_factor,
                            skip_invites,
                        },
                    )
                    .await?;
            }
            if let Some(a) = avatar {
                out = if a.eq_ignore_ascii_case("none") {
                    api.remove_workspace_avatar(w.id.as_str()).await?
                } else {
                    let bytes = std::fs::read(&a).with_context(|| format!("reading {a}"))?;
                    api.set_workspace_avatar(w.id.as_str(), bytes).await?
                };
            }
            if json {
                return style::print_json(&out);
            }
            println!("{} Saved {}", style::accent("✓"), style::bold(&out.slug));
        }
        WorkspacesCmd::Order { workspaces } => {
            let list = api.order_workspaces(&workspaces).await?;
            if json {
                return style::print_json(&list);
            }
            for w in &list.workspaces {
                print_workspace(w);
            }
        }
        WorkspacesCmd::Use { workspace } => {
            let w = api.workspace(&workspace).await?;
            let mut cfg = config::CliConfig::load();
            cfg.workspace = Some(w.slug.clone());
            cfg.save()?;
            if json {
                return style::print_json(&w);
            }
            println!(
                "{} Using {} by default",
                style::accent("✓"),
                style::bold(&w.slug)
            );
        }
        WorkspacesCmd::Members { workspace } => {
            let w = resolve(&api, workspace).await?;
            let list = api.workspace_members(w.id.as_str()).await?;
            if json {
                return style::print_json(&list);
            }
            for m in &list.members {
                println!(
                    "{:<28} {:<36} {:<14} {}",
                    style::bold(&m.user.name),
                    m.email,
                    role(m.role),
                    match m.two_factor {
                        Some(true) => style::accent("2fa"),
                        Some(false) => style::dim("no 2fa"),
                        None => String::new(),
                    }
                );
            }
            for i in &list.invitations {
                println!(
                    "{:<28} {:<36} {:<14} {}",
                    style::dim("invited"),
                    i.email,
                    role(i.role),
                    style::dim(&format!("until {}", style::until(i.expires_at)))
                );
            }
        }
        WorkspacesCmd::Invite {
            email,
            admin,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let inv = api
                .invite_to_workspace(
                    w.id.as_str(),
                    &InviteToWorkspaceRequest {
                        email,
                        role: admin.then_some(WorkspaceRole::Admin),
                    },
                )
                .await?;
            if json {
                return style::print_json(&inv);
            }
            println!(
                "{} Invited {} to {}{}",
                style::accent("✓"),
                inv.email,
                style::bold(&w.name),
                if inv.emailed == Some(true) {
                    " (emailed)"
                } else {
                    ""
                }
            );
            println!(
                "{}",
                style::dim("They join it in the app, or with `fctl workspaces join`.")
            );
        }
        WorkspacesCmd::Uninvite { email, workspace } => {
            let w = resolve(&api, workspace).await?;
            let list = api.workspace_members(w.id.as_str()).await?;
            let inv = list
                .invitations
                .iter()
                .find(|i| i.email.eq_ignore_ascii_case(&email) || i.id.as_str() == email)
                .with_context(|| format!("No invitation to {} is for {email}", w.name))?;
            let out = api
                .revoke_workspace_invitation(w.id.as_str(), inv.id.as_str())
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} Took back the invitation for {}",
                style::accent("✓"),
                inv.email
            );
        }
        WorkspacesCmd::Role {
            person: who,
            role: r,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let m = person(&api, &w, &who).await?;
            let to = match r {
                Role::Admin => WorkspaceRole::Admin,
                Role::Member => WorkspaceRole::Member,
            };
            let out = api
                .set_workspace_role(w.id.as_str(), m.user.id.as_str(), to)
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} {} is {} of {}",
                style::accent("✓"),
                m.user.name,
                if to == WorkspaceRole::Admin {
                    "an admin"
                } else {
                    "a member"
                },
                w.name
            );
        }
        WorkspacesCmd::Remove {
            person: who,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let m = person(&api, &w, &who).await?;
            let out = api
                .remove_workspace_member(w.id.as_str(), m.user.id.as_str())
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} Removed {} from {}",
                style::accent("✓"),
                m.user.name,
                w.name
            );
        }
        WorkspacesCmd::Transfer {
            person: who,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let m = person(&api, &w, &who).await?;
            let out = api
                .transfer_workspace(w.id.as_str(), m.user.id.as_str())
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} {} is the primary admin of {}; you stay an admin",
                style::accent("✓"),
                m.user.name,
                w.name
            );
        }
        WorkspacesCmd::Join { workspace } => {
            let inv = invitation(&api, &workspace).await?;
            let w = api.accept_invitation(inv.id.as_str()).await?;
            if json {
                return style::print_json(&w);
            }
            println!(
                "{} Joined {} as {}",
                style::accent("✓"),
                style::bold(&w.name),
                w.email
            );
        }
        WorkspacesCmd::Decline { workspace } => {
            let inv = invitation(&api, &workspace).await?;
            let out = api.decline_invitation(inv.id.as_str()).await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} Declined the invitation to {}",
                style::accent("✓"),
                inv.workspace.name
            );
        }
        WorkspacesCmd::Leave { workspace } => {
            let w = api.workspace(&workspace).await?;
            let me = api.me().await?;
            let out = api
                .remove_workspace_member(w.id.as_str(), me.user.id.as_str())
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!("{} Left {}", style::accent("✓"), w.name);
        }
        WorkspacesCmd::Runners { workspace } => {
            let w = resolve(&api, workspace).await?;
            let list = api.workspace_runners(w.id.as_str()).await?;
            if json {
                return style::print_json(&list);
            }
            if list.runners.is_empty() {
                println!(
                    "{} has no runners yet. Add one of yours with `fctl workspaces add-runner -w {}` on it, or register a machine with `fctl host install --workspace {}`.",
                    w.name, w.slug, w.slug
                );
            }
            for r in &list.runners {
                let about: Vec<String> = r
                    .created_by
                    .as_ref()
                    .map(|u| format!("{}'s", u.name))
                    .into_iter()
                    .chain([r.id.to_string()])
                    .collect();
                println!(
                    "{:<24} {:<8} {}/{} sessions  {}",
                    style::bold(&r.name),
                    style::runner_status(r),
                    r.active_sessions,
                    r.capacity,
                    style::dim(&about.join(" · "))
                );
            }
        }
        WorkspacesCmd::AddRunner { runner, workspace } => {
            let w = resolve(&api, workspace).await?;
            let r = runner_or_this(&api, runner.as_deref()).await?;
            let list = api
                .add_workspace_runner(w.id.as_str(), r.id.as_str())
                .await?;
            if json {
                return style::print_json(&list);
            }
            println!(
                "{} {} runs {}'s sessions too",
                style::accent("✓"),
                style::bold(&r.name),
                w.name
            );
        }
        WorkspacesCmd::RemoveRunner { runner, workspace } => {
            let w = resolve(&api, workspace).await?;
            let r = runner_or_this(&api, runner.as_deref()).await?;
            let list = api
                .remove_workspace_runner(w.id.as_str(), r.id.as_str())
                .await?;
            if json {
                return style::print_json(&list);
            }
            println!(
                "{} {} takes no more of {}'s sessions",
                style::accent("✓"),
                style::bold(&r.name),
                w.name
            );
        }
        WorkspacesCmd::Github { workspace } => {
            let w = resolve(&api, workspace).await?;
            let g = api.workspace_github(w.id.as_str()).await?;
            if json {
                return style::print_json(&g);
            }
            if !g.configured {
                println!("This instance has no GitHub App.");
                return Ok(());
            }
            if g.connections.is_empty() {
                println!(
                    "{} works in no GitHub account yet. Connect one with `fctl workspaces connect <account>`.",
                    w.name
                );
            }
            for c in &g.connections {
                let repos = c
                    .repos
                    .as_ref()
                    .map(|r| r.join(", "))
                    .unwrap_or_else(|| "every repository".into());
                let state = match (c.status, &c.github_user) {
                    (GithubConnectionStatus::Requested, _) => style::amber("not connected"),
                    (GithubConnectionStatus::Active, Some(login)) => {
                        format!(
                            "{} {}",
                            style::accent("connected"),
                            style::dim(&format!("as @{login}"))
                        )
                    }
                    (GithubConnectionStatus::Active, None) => style::accent("connected"),
                };
                println!(
                    "{:<24} {:<34} {}",
                    style::bold(&c.account),
                    style::dim(&repos),
                    state
                );
            }
            for o in &g.offers {
                let repos = if o.all {
                    "every repository, or some of them".to_owned()
                } else {
                    o.repos.join(", ")
                };
                println!(
                    "{:<24} {:<34} {}",
                    style::bold(&o.account),
                    style::dim(&repos),
                    style::amber("waiting for you to choose, on its GitHub tab")
                );
            }
            if let Some(url) = &g.install_url {
                println!(
                    "{}",
                    style::dim(&format!(
                        "Install the {} app on another account, or give it more repositories: {url}",
                        g.app_slug.as_deref().unwrap_or("GitHub")
                    ))
                );
            } else if g.configured && !g.can_connect {
                println!(
                    "{}",
                    style::dim(
                        "Connecting accounts takes signing in to GitHub, which this instance isn't set up for: its admins add the GitHub App's client secret."
                    )
                );
            }
        }
        WorkspacesCmd::Connect {
            account,
            repos,
            no_browser,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let named = !repos.is_empty();
            let link = api
                .connect_github(
                    w.id.as_str(),
                    &ConnectGithubRequest {
                        account: account.clone(),
                        repos: named.then_some(repos),
                    },
                )
                .await?;
            if json {
                return style::print_json(&link);
            }
            let what = match &account {
                Some(a) => style::bold(a),
                None => "GitHub accounts".to_owned(),
            };
            println!(
                "Connect {what} to {} by signing in to GitHub, in a browser where you're signed in to fctl. The link works for 10 minutes:",
                style::bold(&w.name)
            );
            println!();
            println!("  {}", link.url);
            println!();
            if !no_browser && crate::cmd::auth::open_browser(&link.url) {
                println!("{}", style::dim("Opened it in your browser."));
            }
            println!(
                "{}",
                style::dim(if named {
                    "GitHub shows fctl what you may connect, and the browser comes back to the workspace. `fctl workspaces github` shows what it works in."
                } else {
                    "GitHub shows fctl what you may connect, and the browser comes back to the workspace, where you choose the repositories. `fctl workspaces github` shows what it works in."
                })
            );
        }
        WorkspacesCmd::Disconnect { account, workspace } => {
            let w = resolve(&api, workspace).await?;
            let g = api.disconnect_github(w.id.as_str(), &account).await?;
            if json {
                return style::print_json(&g);
            }
            println!(
                "{} {} no longer works in {account}",
                style::accent("✓"),
                w.name
            );
        }
        WorkspacesCmd::Claude {
            credential,
            workspace,
        } => {
            let w = resolve(&api, workspace).await?;
            let me = api.me().await?;
            let w = match credential {
                Some(c) => {
                    let claude = if c.eq_ignore_ascii_case("default") {
                        String::new()
                    } else {
                        super::claude::pick_connection(&me.claude, &c, style::now_ms())?
                            .id
                            .to_string()
                    };
                    api.update_membership(
                        w.id.as_str(),
                        &UpdateMembershipRequest {
                            claude: Some(claude),
                            ..Default::default()
                        },
                    )
                    .await?
                }
                None => w,
            };
            if json {
                return style::print_json(&w);
            }
            let used = w
                .claude
                .as_ref()
                .and_then(|id| me.claude.iter().find(|c| &c.id == id))
                .or_else(|| me.claude.iter().find(|c| c.is_default));
            match used {
                Some(c) => println!(
                    "Your sessions in {} run on {}{}",
                    style::bold(&w.name),
                    c.label
                        .clone()
                        .or_else(|| c.account_email.clone())
                        .unwrap_or_else(|| c.id.to_string()),
                    if w.claude.is_none() {
                        style::dim(" (your default)")
                    } else {
                        String::new()
                    }
                ),
                None => println!("{}", super::claude::NO_CONNECTION),
            }
        }
        WorkspacesCmd::Audit { workspace, before } => {
            let w = resolve(&api, workspace).await?;
            let list = api.workspace_audit(w.id.as_str(), before).await?;
            if json {
                return style::print_json(&list);
            }
            super::admin::print_audit(&list);
        }
        WorkspacesCmd::Delete { workspace, confirm } => {
            let w = api.workspace(&workspace).await?;
            match confirm {
                Some(c) if c == w.name => {}
                Some(_) => bail!(
                    "Not deleted: --confirm must be the workspace's name, {}",
                    w.name
                ),
                None => confirm_name(&w.name)?,
            }
            let out = api.delete_workspace(w.id.as_str(), &w.name).await?;
            if json {
                return style::print_json(&out);
            }
            println!("{} Deleted {}", style::accent("✓"), w.name);
        }
    }
    Ok(())
}

/// An invitation waiting for you, by its workspace or its id.
async fn invitation(api: &Api, which: &str) -> Result<WorkspaceInvitation> {
    let list = api.workspaces().await?;
    list.invitations
        .into_iter()
        .find(|i| {
            i.id.as_str() == which
                || i.workspace.slug.eq_ignore_ascii_case(which)
                || i.workspace.name.eq_ignore_ascii_case(which)
                || i.workspace.id.as_str() == which
        })
        .with_context(|| {
            format!("No invitation to {which} waits for you. `fctl workspaces` lists them.")
        })
}

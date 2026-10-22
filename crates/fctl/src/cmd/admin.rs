//! `fctl admin …`: people and invitations, workspaces, GitHub, runners and
//! the instance's settings (instance admins).

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand};
use fctl_domain::size;
use fctl_proto::api::{
    AccessRequest, AccessRequestList, CreateInviteRequest, GithubConnectionStatus,
    InstanceSettings, Invite, InviteWorkspace, Runner, RunnerSession, RunnerStatus,
    UpdateInstanceSettingsRequest, UpdateRunnerRequest, UpdateUserRequest, User, UserStatus,
};
use fctl_proto::session::{InstanceRole, WorkspaceRole};
use std::io::{BufRead, IsTerminal, Write};

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Role {
    /// Runs the instance: people, workspaces, GitHub, runners, settings.
    Admin,
    /// Uses the instance.
    Member,
}

#[derive(Subcommand)]
pub enum AdminCmd {
    /// List users.
    Users,
    /// Invite someone to the instance by email, and to workspaces you are
    /// an admin of. They create an account with the link, or connect the
    /// address to the account they have.
    #[command(
        after_help = "Examples:\n  fctl admin invite ada@private.example\n  fctl admin invite grace@example.com -w side-project -w hyperstructure:admin"
    )]
    Invite {
        email: String,
        /// Make them an instance admin.
        #[arg(long)]
        admin: bool,
        /// A workspace to invite them to as well, `slug` or `slug:admin`
        /// (repeatable).
        #[arg(short, long = "workspace", value_name = "WORKSPACE[:admin]")]
        workspaces: Vec<String>,
    },
    /// Make someone an instance admin, or a member again. Only the primary
    /// instance owner takes the role from other admins.
    Role {
        /// Any of their email addresses, or their user id.
        user: String,
        #[arg(value_enum)]
        role: Role,
    },
    /// Hand the role of primary instance owner to an admin (the primary
    /// instance owner). You become an admin.
    Transfer {
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// Disable someone's account: they can't sign in, and their tokens stop
    /// working. Only the primary instance owner disables admins.
    Disable {
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// Let a disabled account sign in again.
    Enable {
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// Delete someone's account, as deleting their own would: the
    /// workspaces only they are in go with it. Only the primary instance
    /// owner deletes admins. This cannot be undone.
    DeleteUser {
        /// Any of their email addresses, or their user id.
        user: String,
        /// Their primary email address, to confirm without being asked.
        #[arg(long, value_name = "EMAIL")]
        confirm: Option<String>,
    },
    /// List every workspace: what it is and who runs it.
    Workspaces,
    /// Give the primary admin role of a workspace whose primary admin's
    /// account is disabled or gone to one of its members. Otherwise its
    /// primary admin hands it over.
    SetPrimary {
        /// The workspace (slug or id).
        workspace: String,
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// Delete a workspace with its projects, sessions and transcripts.
    /// This cannot be undone.
    DeleteWorkspace {
        /// The workspace (slug or id).
        workspace: String,
        /// The workspace's name, to confirm without being asked.
        #[arg(long, value_name = "NAME")]
        confirm: Option<String>,
    },
    /// The accounts the instance's GitHub App is installed on, and the
    /// workspaces that use them. Workspaces' admins connect them.
    #[command(subcommand)]
    Github(GithubCmd),
    /// The people who asked for early access on the sign-in page and have
    /// no account yet, where the instance takes requests (as app.fctl.sh
    /// does): those waiting first, longest waiting first. Invite one with
    /// `fctl admin invite <email>`.
    #[command(
        after_help = "Examples:\n  fctl admin early-access\n  fctl admin invite ada@example.com\n  fctl admin early-access remove spam@example.com"
    )]
    EarlyAccess {
        #[command(subcommand)]
        cmd: Option<EarlyAccessCmd>,
    },
    /// Show who may sign up.
    Allowlist,
    /// Allow an email (invites them) or a domain (`*@example.com`).
    Allow {
        /// An email address, or `*@example.com` for a whole domain.
        pattern: String,
        /// Make them an admin.
        #[arg(long)]
        admin: bool,
    },
    /// Remove an allowlist entry (by its pattern or id); its users lose access.
    Disallow {
        /// The entry's pattern or id.
        pattern: String,
    },
    /// List runners.
    Runners,
    /// Drain a runner: no new session starts on it, and the sessions it has
    /// carry on there.
    Drain {
        /// The runner's name or id.
        runner: String,
    },
    /// Stop draining a runner: it takes new sessions again.
    Undrain {
        /// The runner's name or id.
        runner: String,
    },
    /// Turn off someone's two-factor sign-in, when they lost their phone
    /// and their recovery codes: removes their authenticator app and
    /// recovery codes, and their passkeys where those are the second factor
    /// the instance requires. They can sign in with their password alone
    /// again, or set it up again when the instance requires it.
    ResetTwoFactor {
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// What happened on the instance, newest first: sign-ins, invitations,
    /// people, workspaces, runners and settings (what happened in a
    /// workspace is in `fctl workspaces audit`).
    Audit {
        /// Entries before this time (milliseconds), for the page after
        /// one's `next_before`.
        #[arg(long, value_name = "MS")]
        before: Option<i64>,
    },
    /// Show the instance's settings.
    Settings,
    /// Change the instance's settings.
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["publish_screenshots", "require_two_factor", "two_factor_google", "verify_sensitive_actions"])),
        after_help = "Examples:\n  fctl admin set --publish-screenshots on\n  fctl admin set --require-two-factor on --two-factor-google on\n  fctl admin set --verify-sensitive-actions on"
    )]
    Set {
        /// Let projects publish their agents' screenshots on this instance,
        /// at links anyone who has one can open until they expire. Each
        /// project turns it on too (`fctl projects set --publish-screenshots
        /// on`). Off takes every published screenshot offline.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        publish_screenshots: Option<bool>,
        /// Require two-factor sign-in of everyone who signs in with a
        /// password. People who haven't set it up can do nothing else until
        /// they have, in the browser and with their CLI and access tokens.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        require_two_factor: Option<bool>,
        /// Make Google sign-ins ask for the second factor too, of everyone
        /// who has one. With --require-two-factor, everyone needs one.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        two_factor_google: Option<bool>,
        /// Ask people to confirm it's them, with a passkey or a code from
        /// their authenticator app, before sensitive actions: inviting and
        /// promoting admins, turning two-factor sign-in off, adding a
        /// runner to a workspace, new access tokens and CLI sign-ins,
        /// exporting data, and deleting accounts, projects and workspaces.
        /// Only the primary instance owner changes it; off takes confirming
        /// it's you.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        verify_sensitive_actions: Option<bool>,
    },
}

#[derive(Subcommand)]
pub enum EarlyAccessCmd {
    /// List them (the default).
    List,
    /// Remove someone's request (by their address or the request's id).
    /// Asking again adds it back.
    Remove {
        /// Their email address, or the request's id.
        request: String,
    },
}

#[derive(Subcommand)]
pub enum GithubCmd {
    /// List them, with the workspaces that use each.
    List,
    /// Stop a workspace from working in an account. Only its admins
    /// connect it again, signing in to GitHub.
    Remove { account: String, workspace: String },
}

/// Someone on the instance, by their user id or any of their addresses.
fn find_user<'a>(users: &'a [User], who: &str) -> Result<&'a User> {
    users
        .iter()
        .find(|u| {
            u.id.as_str() == who
                || u.email.eq_ignore_ascii_case(who)
                || u.emails.iter().any(|e| e.eq_ignore_ascii_case(who))
        })
        .with_context(|| format!("No user is {who}. `fctl admin users` lists them."))
}

async fn set_status(
    api: &fctl_client::Api,
    who: &str,
    status: UserStatus,
    json: bool,
) -> Result<()> {
    let users = api.users().await?.users;
    let target = find_user(&users, who)?;
    let req = UpdateUserRequest {
        role: None,
        status: Some(status),
        name: None,
    };
    let updated = api.update_user(target.id.as_str(), &req).await?;
    if json {
        return style::print_json(&updated);
    }
    match status {
        UserStatus::Disabled => println!(
            "{} {} can't sign in any more, and their tokens stopped working",
            style::accent("✓"),
            updated.name
        ),
        _ => println!("{} {} can sign in again", style::accent("✓"), updated.name),
    }
    Ok(())
}

/// An audit log page, one entry a line.
pub(crate) fn print_audit(list: &fctl_proto::api::AuditList) {
    for e in &list.entries {
        println!(
            "{}  {:<22} {:<28} {}",
            style::dim(&format!("{:<9}", style::ago(e.ts))),
            e.actor.as_ref().map(|a| a.name.as_str()).unwrap_or("—"),
            e.action,
            style::dim(e.target.as_deref().unwrap_or_default())
        );
    }
    if let Some(b) = list.next_before {
        println!("{}", style::dim(&format!("Older: --before {b}")));
    }
}

fn print_github(list: &fctl_proto::api::GithubInstallations) {
    if !list.configured {
        println!("This instance has no GitHub App.");
        return;
    }
    if list.installations.is_empty() {
        println!(
            "The {} app isn't installed anywhere yet.",
            list.app_slug.as_deref().unwrap_or("GitHub")
        );
    }
    for i in &list.installations {
        let state = match (i.installation_id, i.repository_selection.as_deref()) {
            (None, _) => style::red("not installed"),
            (_, Some("selected")) => style::dim("selected repositories"),
            _ => style::dim("all repositories"),
        };
        println!("{} {state}", style::bold(&i.account));
        if i.workspaces.is_empty() {
            println!("  {}", style::dim("no workspace uses it"));
        }
        for w in &i.workspaces {
            let repos = w
                .repos
                .as_ref()
                .map(|r| r.join(", "))
                .unwrap_or_else(|| "every repository".into());
            let status = match (w.status, &w.github_user) {
                (GithubConnectionStatus::Requested, _) => style::amber("not connected"),
                (GithubConnectionStatus::Active, Some(login)) => format!(
                    "{} {}",
                    style::accent("connected"),
                    style::dim(&format!(
                        "by {}as @{login}",
                        w.requested_by
                            .as_ref()
                            .map(|u| format!("{} ", u.name))
                            .unwrap_or_default()
                    ))
                ),
                (GithubConnectionStatus::Active, None) => style::accent("connected"),
            };
            println!(
                "  {:<24} {:<32} {status}",
                w.workspace.slug,
                style::dim(&repos)
            );
        }
    }
    if let Some(url) = &list.install_url {
        println!(
            "{}",
            style::dim(&format!("Install it on another account: {url}"))
        );
    }
}

/// Who asked for early access: those waiting, then those invited.
fn print_access_requests(list: &AccessRequestList, now: i64) {
    if list.requests.is_empty() {
        println!("Nobody is waiting for early access.");
        return;
    }
    let (invited, waiting): (Vec<&AccessRequest>, Vec<&AccessRequest>) =
        list.requests.iter().partition(|r| r.invited_at.is_some());
    if !waiting.is_empty() {
        println!("{}", style::bold(&format!("Waiting ({})", list.waiting)));
        for r in &waiting {
            println!("  {:<36} {}", r.email, style::dim(&asked(r, now)));
        }
    }
    if !invited.is_empty() {
        if !waiting.is_empty() {
            println!();
        }
        println!("{}", style::bold(&format!("Invited ({})", list.invited)));
        for r in &invited {
            println!("  {:<36} {}", r.email, style::dim(&invitation(r, now)));
        }
    }
    let shown = list.requests.len() as u64;
    let all = u64::from(list.waiting) + u64::from(list.invited);
    if shown < all {
        println!("{}", style::dim(&format!("…and {} more", all - shown)));
    }
    if !waiting.is_empty() {
        println!(
            "\n{}",
            style::dim("Invite someone with `fctl admin invite <email>`: it prints their link.")
        );
    }
}

/// "asked 2d ago", "asked 5d ago, again 3h ago".
fn asked(r: &AccessRequest, now: i64) -> String {
    let first = format!("asked {}", style::ago_from(r.created_at, now));
    // Asking again within the hour isn't news.
    if r.asked_at - r.created_at > 60 * 60 * 1000 {
        format!("{first}, again {}", style::ago_from(r.asked_at, now))
    } else {
        first
    }
}

/// "invited 1d ago by Maya", with whether the link still works.
fn invitation(r: &AccessRequest, now: i64) -> String {
    let mut s = format!(
        "invited {}",
        style::ago_from(r.invited_at.unwrap_or(now), now)
    );
    if let Some(by) = &r.invited_by {
        s.push_str(&format!(" by {}", by.name));
    }
    s.push_str(if r.invite_expires_at.is_some() {
        ", hasn't joined yet"
    } else {
        ", the link expired"
    });
    s
}

/// An invitation's link, or where it went: an instance that sends email
/// sends it to the address alone.
fn print_invite_link(invite: &Invite) {
    match (&invite.url, invite.emailed) {
        (Some(url), emailed) => println!(
            "Invitation link (valid 14 days{}):\n  {url}",
            if emailed { ", also emailed" } else { "" }
        ),
        (None, true) => println!("Emailed them the invitation link (valid 14 days)."),
        (None, false) => println!(
            "The invitation couldn't be emailed to {}. Invite them again in a while.",
            invite.email
        ),
    }
}

fn print_settings(s: &InstanceSettings) {
    println!(
        "Published screenshots  {}",
        if s.publish_screenshots {
            style::accent("allowed")
        } else {
            style::dim("off")
        }
    );
    println!(
        "  {}",
        style::dim(if s.publish_screenshots {
            "Projects that turn them on let agents publish screenshots for pull requests."
        } else {
            "No project's agents can publish screenshots."
        })
    );
    println!(
        "Two-factor sign-in     {}",
        if s.require_two_factor {
            style::accent("required")
        } else {
            style::dim("optional")
        }
    );
    println!(
        "  {}",
        style::dim(match (s.require_two_factor, s.two_factor_google) {
            (true, true) => "Everyone needs a second factor, and Google sign-ins ask for it too.",
            (true, false) => {
                "Everyone who signs in with a password needs a second factor. Google sign-ins don't ask for it."
            }
            (false, true) => {
                "People choose whether to use it. Google sign-ins ask everyone who has it."
            }
            (false, false) => {
                "People choose whether to use it. Google sign-ins ask only those who chose that."
            }
        })
    );
    println!(
        "Sensitive actions      {}",
        if s.verify_sensitive_actions {
            style::accent("ask to confirm")
        } else {
            style::dim("don't ask")
        }
    );
    println!(
        "  {}",
        style::dim(if s.verify_sensitive_actions {
            "Deleting projects, new access tokens and other sensitive actions take a passkey or a code first, from people who have one."
        } else {
            "Sensitive actions don't ask people to confirm it's them."
        })
    );
}

pub async fn run(server: Option<&str>, cmd: AdminCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        AdminCmd::Invite {
            email,
            admin,
            workspaces,
        } => {
            let workspaces: Vec<InviteWorkspace> = workspaces
                .iter()
                .map(|w| match w.split_once(':') {
                    Some((slug, "admin")) => InviteWorkspace {
                        workspace: slug.to_owned(),
                        role: Some(WorkspaceRole::Admin),
                    },
                    _ => InviteWorkspace {
                        workspace: w.clone(),
                        role: None,
                    },
                })
                .collect();
            let invite = api
                .invite(&CreateInviteRequest {
                    email,
                    role: admin.then_some(InstanceRole::Admin),
                    workspaces: (!workspaces.is_empty()).then_some(workspaces),
                })
                .await?;
            if json {
                return style::print_json(&invite);
            }
            println!(
                "{} Invited {}{}",
                style::accent("✓"),
                invite.email,
                if invite.workspaces.is_empty() {
                    String::new()
                } else {
                    format!(
                        " to {}",
                        invite
                            .workspaces
                            .iter()
                            .map(|w| w.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            );
            print_invite_link(&invite);
        }
        AdminCmd::Transfer { user } => {
            let users = api.users().await?.users;
            let target = find_user(&users, &user)?;
            let updated = api.transfer_instance(target.id.as_str()).await?;
            if json {
                return style::print_json(&updated);
            }
            println!(
                "{} {} is the primary instance owner; you are an admin now",
                style::accent("✓"),
                updated.name
            );
        }
        AdminCmd::Role { user, role } => {
            let users = api.users().await?.users;
            let target = find_user(&users, &user)?;
            let role = match role {
                Role::Admin => InstanceRole::Admin,
                Role::Member => InstanceRole::Member,
            };
            let req = UpdateUserRequest {
                role: Some(role),
                status: None,
                name: None,
            };
            let updated = api.update_user(target.id.as_str(), &req).await?;
            if json {
                return style::print_json(&updated);
            }
            println!(
                "{} {} is {}",
                style::accent("✓"),
                updated.name,
                if role == InstanceRole::Admin {
                    "an instance admin"
                } else {
                    "a member of the instance"
                }
            );
        }
        AdminCmd::Disable { user } => set_status(&api, &user, UserStatus::Disabled, json).await?,
        AdminCmd::Enable { user } => set_status(&api, &user, UserStatus::Active, json).await?,
        AdminCmd::DeleteUser { user, confirm } => {
            let users = api.users().await?.users;
            let target = find_user(&users, &user)?;
            match confirm {
                Some(c) if c.eq_ignore_ascii_case(&target.email) => {}
                Some(_) => bail!(
                    "Not deleted: --confirm must be their primary address, {}",
                    target.email
                ),
                None => {
                    let stdin = std::io::stdin();
                    if !stdin.is_terminal() {
                        bail!(
                            "Pass --confirm {} to delete the account without being asked",
                            target.email
                        );
                    }
                    print!(
                        "Type their address ({}) to delete {}'s account: ",
                        target.email, target.name
                    );
                    std::io::stdout().flush()?;
                    let mut line = String::new();
                    stdin.lock().read_line(&mut line)?;
                    if !line.trim().eq_ignore_ascii_case(&target.email) {
                        bail!("Not deleted: the address didn't match");
                    }
                }
            }
            let ok = api.delete_user(target.id.as_str()).await?;
            if json {
                return style::print_json(&ok);
            }
            println!(
                "{} Deleted {} <{}>",
                style::accent("✓"),
                target.name,
                target.email
            );
        }
        AdminCmd::Workspaces => {
            let list = api.admin_workspaces().await?;
            if json {
                return style::print_json(&list);
            }
            for w in &list.workspaces {
                println!(
                    "{:<24} {:<28} {} {}",
                    style::bold(&w.workspace.slug),
                    w.workspace.name,
                    style::dim(&format!(
                        "{}, {}, {}",
                        style::count(w.members, "member", "members"),
                        style::count(w.projects, "project", "projects"),
                        style::count(w.runners, "runner", "runners")
                    )),
                    style::dim(
                        &w.primary
                            .as_ref()
                            .map(|p| format!("· {}", p.name))
                            .unwrap_or_default()
                    )
                );
                if w.needs_primary {
                    println!(
                        "  {}",
                        style::amber(&format!(
                            "needs a primary admin: `fctl admin set-primary {} <person>`",
                            w.workspace.slug
                        ))
                    );
                }
            }
        }
        AdminCmd::SetPrimary { workspace, user } => {
            let list = api.admin_workspaces().await?;
            let w = list
                .workspaces
                .iter()
                .find(|w| w.workspace.slug == workspace || w.workspace.id.as_str() == workspace)
                .with_context(|| format!("No workspace is {workspace}"))?;
            if !w.needs_primary {
                bail!(
                    "{} has a primary admin, who hands the role over themselves",
                    w.workspace.name
                );
            }
            let users = api.users().await?.users;
            let target = find_user(&users, &user)?;
            if !w.candidates.iter().any(|c| c.id == target.id) {
                bail!(
                    "{} isn't one of {}'s members with an active account: {}",
                    target.name,
                    w.workspace.name,
                    w.candidates
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let ok = api
                .admin_workspace_primary(w.workspace.id.as_str(), target.id.as_str())
                .await?;
            if json {
                return style::print_json(&ok);
            }
            println!(
                "{} {} is {}'s primary admin",
                style::accent("✓"),
                target.name,
                w.workspace.name
            );
        }
        AdminCmd::DeleteWorkspace { workspace, confirm } => {
            let list = api.admin_workspaces().await?;
            let w = list
                .workspaces
                .iter()
                .find(|w| w.workspace.slug == workspace || w.workspace.id.as_str() == workspace)
                .with_context(|| format!("No workspace is {workspace}"))?;
            let name = w.workspace.name.clone();
            let typed = match confirm {
                Some(c) => c,
                None => {
                    use std::io::{BufRead, IsTerminal, Write};
                    if !std::io::stdin().is_terminal() {
                        bail!("Pass --confirm \"{name}\" to delete it without being asked");
                    }
                    println!(
                        "{}",
                        style::red(&format!(
                            "This deletes {name}: its {} projects, their sessions and transcripts, for its {} members.",
                            w.projects, w.members
                        ))
                    );
                    print!("Type its name to confirm: ");
                    std::io::stdout().flush()?;
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line.trim().to_owned()
                }
            };
            let out = api
                .admin_delete_workspace(w.workspace.id.as_str(), &typed)
                .await?;
            if json {
                return style::print_json(&out);
            }
            println!("{} Deleted {name}", style::accent("✓"));
        }
        AdminCmd::Github(c) => {
            let list = match c {
                GithubCmd::List => api.admin_github().await?,
                GithubCmd::Remove { account, workspace } => {
                    api.admin_disconnect_github(&account, &workspace).await?
                }
            };
            if json {
                return style::print_json(&list);
            }
            print_github(&list);
        }
        AdminCmd::Users => {
            let list = api.users().await?;
            if json {
                return style::print_json(&list);
            }
            for u in list.users {
                let role = match u.role {
                    InstanceRole::Owner => "owner",
                    r => r.as_str(),
                };
                println!(
                    "{:<28} {:<32} {:<7} {:<8} {}",
                    style::bold(&u.name),
                    u.email,
                    role,
                    if u.status == fctl_proto::api::UserStatus::Active {
                        style::accent("active")
                    } else {
                        style::red("disabled")
                    },
                    match (u.two_factor, u.passkeys) {
                        (false, 0) => style::dim("no 2fa"),
                        (true, 0) => style::accent("2fa"),
                        (false, n) => style::accent(&passkeys(n)),
                        (true, n) => style::accent(&format!("2fa, {}", passkeys(n))),
                    }
                );
                let mut more = Vec::new();
                if !u.emails.is_empty() {
                    more.push(format!("also {}", u.emails.join(", ")));
                }
                more.push(match u.workspaces {
                    0 => "in no workspace".to_owned(),
                    1 => "in 1 workspace".to_owned(),
                    n => format!("in {n} workspaces"),
                });
                println!("  {}", style::dim(&more.join(" · ")));
            }
        }
        AdminCmd::EarlyAccess { cmd } => match cmd.unwrap_or(EarlyAccessCmd::List) {
            EarlyAccessCmd::List => {
                let list = api.access_requests().await?;
                if json {
                    return style::print_json(&list);
                }
                print_access_requests(&list, style::now_ms());
            }
            EarlyAccessCmd::Remove { request } => {
                let list = api.access_requests().await?;
                let wanted = request.trim().to_lowercase();
                let found = list
                    .requests
                    .into_iter()
                    .find(|r| r.email == wanted || r.id.as_str() == request.trim())
                    .context("Nobody waiting or invited asked with that address or id")?;
                let out = api.remove_access_request(found.id.as_str()).await?;
                if json {
                    return style::print_json(&out);
                }
                println!("Removed {}'s request", found.email);
            }
        },
        AdminCmd::Allowlist => {
            let list = api.allowlist().await?;
            if json {
                return style::print_json(&list);
            }
            for e in list.entries {
                let state = if e.user_id.is_some() {
                    style::accent("joined")
                } else if e.pattern.starts_with("*@") {
                    style::blue("domain")
                } else {
                    style::dim("invited")
                };
                println!("{:<36} {:<7} {}", e.pattern, e.role.as_str(), state);
            }
        }
        AdminCmd::Allow { pattern, admin } => {
            let created = api
                .allow(&pattern, admin.then_some(InstanceRole::Admin))
                .await?;
            if json {
                return style::print_json(&created);
            }
            println!("{} Allowed {}", style::accent("✓"), created.entry.pattern);
            if let Some(inv) = created.invite {
                print_invite_link(&inv);
            }
        }
        AdminCmd::Disallow { pattern } => {
            let list = api.allowlist().await?;
            let entry = list
                .entries
                .into_iter()
                .find(|e| e.pattern == pattern.to_lowercase() || e.id.0 == pattern)
                .context("No such allowlist entry")?;
            let out = api.disallow(&entry.id.0).await?;
            if json {
                return style::print_json(&out);
            }
            println!("Removed {}", entry.pattern);
            for (key, what) in [
                ("disabled", "Disabled"),
                ("disconnected", "Disconnected from their accounts"),
            ] {
                if let Some(d) = out
                    .get(key)
                    .and_then(|v| v.as_array())
                    .filter(|a| !a.is_empty())
                {
                    println!(
                        "{what}: {}",
                        d.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
        }
        AdminCmd::Runners => {
            let list = api.runners().await?;
            if json {
                return style::print_json(&list);
            }
            for r in list.runners {
                let state = style::runner_status(&r);
                let about: Vec<&str> = r
                    .version
                    .as_deref()
                    .into_iter()
                    .chain([r.id.as_str()])
                    .collect();
                println!(
                    "{:<24} {:<8} {:<8} {}/{} sessions  {}",
                    style::bold(&r.name),
                    state,
                    r.driver.as_deref().unwrap_or_default(),
                    r.active_sessions,
                    r.capacity,
                    style::dim(&about.join(" · "))
                );
                if let Some(d) = drain_line(&r) {
                    println!("  {}", style::dim(&d));
                }
                if let Some(d) = low_disk_line(&r) {
                    println!("  {}", style::amber(&d));
                }
                if let Some(l) = &r.sandbox {
                    println!(
                        "  {}",
                        style::dim(&format!("sandboxes: {}", size::limits(l)))
                    );
                }
                if let Some(own) = &r.image {
                    println!(
                        "  {}",
                        style::dim(&format!(
                            "image: {own}; {}; {}",
                            fctl_domain::image::describe_allowed(r.allowed_images.as_deref()),
                            fctl_domain::image::describe_builds(r.allowed_builds.as_deref())
                        ))
                    );
                }
                let free: Vec<String> = [
                    r.memory_available_mb
                        .map(|m| format!("{} of memory", size::memory(m))),
                    r.disk_available_mb
                        .map(|d| format!("{} of disk", size::memory(d))),
                ]
                .into_iter()
                .flatten()
                .collect();
                if !free.is_empty() {
                    println!("  {}", style::dim(&format!("free: {}", free.join(", "))));
                }
            }
        }
        AdminCmd::Audit { before } => {
            let list = api.admin_audit(before).await?;
            if json {
                return style::print_json(&list);
            }
            print_audit(&list);
        }
        AdminCmd::Settings => {
            let settings = api.instance_settings().await?;
            if json {
                return style::print_json(&settings);
            }
            print_settings(&settings);
        }
        AdminCmd::Set {
            publish_screenshots,
            require_two_factor,
            two_factor_google,
            verify_sensitive_actions,
        } => {
            let settings = api
                .update_instance_settings(&UpdateInstanceSettingsRequest {
                    publish_screenshots,
                    require_two_factor,
                    two_factor_google,
                    verify_sensitive_actions,
                })
                .await?;
            if json {
                return style::print_json(&settings);
            }
            print_settings(&settings);
            if require_two_factor == Some(true) || two_factor_google == Some(true) {
                let users = match api.users().await {
                    Ok(list) => list.users,
                    // Requiring it stopped this admin too.
                    Err(e)
                        if e.api().is_some_and(|a| {
                            a.code == fctl_proto::ErrorCode::TwoFactorSetupRequired
                        }) =>
                    {
                        println!(
                            "{}",
                            style::amber(
                                "You haven't set it up yourself yet: run `fctl account two-factor enable`."
                            )
                        );
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                };
                let missing: Vec<&fctl_proto::api::User> = users
                    .iter()
                    .filter(|u| {
                        u.status == fctl_proto::api::UserStatus::Active
                            && !u.two_factor
                            && u.passkeys == 0
                            && fctl_domain::two_factor::required(&settings, u.has_password)
                    })
                    .collect();
                if !missing.is_empty() {
                    println!(
                        "{}",
                        style::amber(&format!(
                            "{} without it yet, who set it up before they can continue: {}",
                            if missing.len() == 1 {
                                "1 person is".to_owned()
                            } else {
                                format!("{} people are", missing.len())
                            },
                            missing
                                .iter()
                                .map(|u| u.email.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    );
                }
            }
        }
        AdminCmd::ResetTwoFactor { user } => {
            let users = api.users().await?.users;
            let target = find_user(&users, &user)?;
            let updated = api.reset_two_factor(target.id.as_str()).await?;
            if json {
                return style::print_json(&updated);
            }
            println!(
                "{} Two-factor sign-in is off for {} <{}>.",
                style::accent("✓"),
                updated.name,
                updated.email
            );
            let required = api
                .instance_settings()
                .await
                .is_ok_and(|s| fctl_domain::two_factor::required(&s, updated.has_password));
            // Passkeys go only where they were the second factor required.
            let gone = if target.passkeys > 0 && updated.passkeys == 0 {
                "Their authenticator app's codes, recovery codes and passkeys no longer work"
            } else {
                "Their authenticator app's codes and recovery codes no longer work"
            };
            println!(
                "{}",
                style::dim(&if required {
                    format!(
                        "{gone}. The instance requires two-factor sign-in, so they set it up again before anything else."
                    )
                } else {
                    format!(
                        "{gone}, and their password alone signs them in until they set it up again."
                    )
                })
            );
        }
        AdminCmd::Drain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = api
                .update_runner(
                    r.id.as_str(),
                    &UpdateRunnerRequest {
                        draining: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Draining {}: no new sessions start on it.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            println!("{}", style::dim(&still_running(r.active_sessions, "there")));
        }
        AdminCmd::Undrain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = api
                .update_runner(
                    r.id.as_str(),
                    &UpdateRunnerRequest {
                        draining: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} {} takes new sessions again.",
                style::accent("✓"),
                style::bold(&r.name)
            );
        }
    }
    Ok(())
}

/// A runner by its id or name.
pub(crate) async fn find_runner(api: &fctl_client::Api, which: &str) -> Result<Runner> {
    let all = api.runners().await?.runners;
    pick_runner(&all, which, style::now_ms()).cloned()
}

/// The runner whose id is `which`, or the only one called that.
fn pick_runner<'a>(all: &'a [Runner], which: &str, now: i64) -> Result<&'a Runner> {
    if let Some(r) = all.iter().find(|r| r.id.as_str() == which) {
        return Ok(r);
    }
    let named: Vec<&Runner> = all.iter().filter(|r| r.name == which).collect();
    match named.as_slice() {
        [r] => Ok(r),
        [] => bail!(
            "No runner {} {which}. `fctl admin runners` lists them, with their ids.",
            if which.starts_with("rnr_") {
                "has the id"
            } else {
                "is called"
            }
        ),
        _ => bail!(
            "{} runners are called {which}; use one of their ids:\n{}",
            named.len(),
            named
                .iter()
                .map(|r| format!("  {}  {}", r.id, tell_apart(r, now)))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// What tells runners with the same name apart: `online, Ada Lovelace's`,
/// `offline, seen 3d ago`.
fn tell_apart(r: &Runner, now: i64) -> String {
    let mut about = vec![style::runner_state(r).to_owned()];
    if r.status != RunnerStatus::Online {
        about.push(match r.last_seen_at {
            Some(t) => format!("seen {}", style::ago_from(t, now)),
            None => "never connected".to_owned(),
        });
    }
    if let Some(u) = &r.created_by {
        about.push(format!("{}'s", u.name));
    }
    about.join(", ")
}

/// "1 passkey", "3 passkeys".
fn passkeys(n: u32) -> String {
    if n == 1 {
        "1 passkey".to_owned()
    } else {
        format!("{n} passkeys")
    }
}

/// Who started draining a runner and when, and what still runs on it:
/// `Ada Lovelace started draining it 3m ago: 2 sessions left`.
pub(crate) fn drain_line(r: &Runner) -> Option<String> {
    let d = r.drain.as_ref()?;
    let who =
        d.by.as_ref()
            .map_or_else(|| "its host".to_owned(), |u| u.name.clone());
    let left = match r.active_sessions {
        0 => "no sessions left".to_owned(),
        1 => "1 session left".to_owned(),
        n => format!("{n} sessions left"),
    };
    Some(format!(
        "{who} started draining it {}: {left}",
        style::ago(d.since)
    ))
}

/// Why no new session starts on a runner whose disk is low:
/// `12.3 GiB of disk free, below the 20 GiB it keeps free: no new sessions
/// start on it`.
pub(crate) fn low_disk_line(r: &Runner) -> Option<String> {
    let (free, min) = (r.disk_available_mb?, r.min_free_disk_mb?);
    (free < min).then(|| {
        format!(
            "{}: no new sessions start on it",
            fctl_domain::disk::below(free, min)
        )
    })
}

/// What a drain leaves running, and what happens to it.
pub(crate) fn still_running(n: u32, place: &str) -> String {
    match n {
        0 => format!("No sessions run {place}."),
        1 => format!(
            "1 session still runs {place} and carries on. Once idle for a minute it stops, and a message resumes it {place}."
        ),
        _ => format!(
            "{n} sessions still run {place} and carry on. Idle ones stop after a minute, and a message resumes them {place}."
        ),
    }
}

/// The sessions a runner runs, one a line.
pub(crate) fn print_sessions(sessions: &[RunnerSession]) {
    for s in sessions {
        let mut status = style::status(s.status);
        if let Some(at) = s.resume_at {
            status = format!(
                "{status} {}",
                style::dim(&format!("(resumes {})", style::until(at)))
            );
        }
        println!(
            "  {} {}#{} {}",
            status,
            s.project_slug,
            s.number,
            style::dim(&s.title)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn runner(id: &str, name: &str, status: &str, last_seen_at: Option<i64>) -> Result<Runner> {
        Ok(serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "status": status,
            "drivers": [],
            "capacity": 4,
            "active_sessions": 0,
            "labels": [],
            "last_seen_at": last_seen_at,
            "created_at": 0,
            "created_by": { "id": "usr_ada", "name": "Ada Lovelace" },
        }))?)
    }

    #[test]
    fn runners_are_found_by_id_or_a_name_only_one_has() -> Result<()> {
        let all = [
            runner("rnr_01STUDIO", "studio-mac", "online", Some(NOW))?,
            runner("rnr_01OLD", "laptop", "offline", None)?,
            runner("rnr_01NEW", "laptop", "online", Some(NOW))?,
        ];
        let id = |which| {
            pick_runner(&all, which, NOW)
                .map(|r| r.id.to_string())
                .map_err(|e| e.to_string())
        };
        assert_eq!(id("rnr_01STUDIO"), Ok("rnr_01STUDIO".into()));
        assert_eq!(id("studio-mac"), Ok("rnr_01STUDIO".into()));
        assert_eq!(id("rnr_01OLD"), Ok("rnr_01OLD".into()));
        assert_eq!(
            id("Studio-Mac"),
            Err(
                "No runner is called Studio-Mac. `fctl admin runners` lists them, with their ids."
                    .into()
            )
        );
        assert_eq!(
            id("rnr_01GONE"),
            Err(
                "No runner has the id rnr_01GONE. `fctl admin runners` lists them, with their ids."
                    .into()
            )
        );
        Ok(())
    }

    #[test]
    fn runners_with_the_same_name_are_told_apart_by_their_ids() -> Result<()> {
        let all = [
            runner("rnr_01OLD", "laptop", "offline", Some(NOW - 3 * 86_400_000))?,
            runner("rnr_01NEW", "laptop", "online", Some(NOW))?,
            runner("rnr_01NEVER", "laptop", "offline", None)?,
        ];
        assert_eq!(
            pick_runner(&all, "laptop", NOW)
                .map_err(|e| e.to_string())
                .err(),
            Some(
                "3 runners are called laptop; use one of their ids:\n  \
                 rnr_01OLD  offline, seen 3d ago, Ada Lovelace's\n  \
                 rnr_01NEW  online, Ada Lovelace's\n  \
                 rnr_01NEVER  offline, never connected, Ada Lovelace's"
                    .into()
            )
        );
        Ok(())
    }

    fn request(fields: serde_json::Value) -> Result<AccessRequest> {
        let mut r = serde_json::json!({
            "id": "acr_01ADA",
            "email": "ada@example.com",
            "created_at": NOW - 2 * 86_400_000,
            "asked_at": NOW - 2 * 86_400_000,
        });
        if let (Some(r), Some(f)) = (r.as_object_mut(), fields.as_object()) {
            r.extend(f.clone());
        }
        Ok(serde_json::from_value(r)?)
    }

    #[test]
    fn requests_say_when_they_asked() -> Result<()> {
        assert_eq!(asked(&request(serde_json::json!({}))?, NOW), "asked 2d ago");
        // Asking again within the hour isn't news; later, it is.
        let soon = request(serde_json::json!({ "asked_at": NOW - 2 * 86_400_000 + 60_000 }))?;
        assert_eq!(asked(&soon, NOW), "asked 2d ago");
        let later = request(serde_json::json!({ "asked_at": NOW - 3 * 3_600_000 }))?;
        assert_eq!(asked(&later, NOW), "asked 2d ago, again 3h ago");
        Ok(())
    }

    #[test]
    fn invited_requests_say_whether_their_link_still_works() -> Result<()> {
        let invited = request(serde_json::json!({
            "invited_at": NOW - 86_400_000,
            "invited_by": { "id": "usr_maya", "name": "Maya" },
            "invite_expires_at": NOW + 13 * 86_400_000,
        }))?;
        assert_eq!(
            invitation(&invited, NOW),
            "invited 1d ago by Maya, hasn't joined yet"
        );
        let expired = request(serde_json::json!({ "invited_at": NOW - 20 * 86_400_000 }))?;
        assert_eq!(
            invitation(&expired, NOW),
            "invited 20d ago, the link expired"
        );
        Ok(())
    }
}

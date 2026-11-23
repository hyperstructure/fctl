//! `fctl admin tokens` and `fctl admin users show|create|set|add|remove`:
//! the instance's admin tokens, and what programs and scripts provision
//! people with them, where the instance's deployment turns them on
//! (docs/architecture.md#admin-tokens).

use crate::style;
use anyhow::{Context, Result, anyhow, bail};
use clap::{ArgGroup, Subcommand};
use fctl_proto::api::{
    AddClaudeTokenRequest, AdminToken, AdminUser, CreateAdminTokenRequest, CreateUserRequest,
    UpdateUserRequest,
};
use fctl_proto::session::{ProjectRole, WorkspaceRole};
use std::io::{BufRead, IsTerminal};

#[derive(Subcommand)]
pub enum AdminTokensCmd {
    /// List the instance's admin tokens (the default), whoever made them.
    List,
    /// Make an admin token, which acts as you on the people endpoints of
    /// the admin API alone: it creates, reads, changes and deletes
    /// accounts, sets their pictures and Claude tokens, and adds them to
    /// any workspace and project. Prints it once.
    #[command(
        after_help = "Examples:\n  fctl admin tokens create \"Okta provisioning\"\n  fctl admin tokens create \"HR sync\" --days 365"
    )]
    Create {
        /// What it is for, as the list shows it.
        name: String,
        /// Expire after this many days (default: never).
        #[arg(long)]
        days: Option<u32>,
    },
    /// Revoke an admin token, anyone's, by its id or name: it stops
    /// working at once.
    Revoke {
        /// Its id (`adt_…`) or name.
        token: String,
    },
}

#[derive(Subcommand)]
pub enum UsersCmd {
    /// List everyone on the instance (the default).
    List,
    /// Someone's account, with the workspaces and projects they are a
    /// member of and the Claude token an admin token set for them. Takes
    /// an admin token (FCTL_TOKEN=fctl_adm_…).
    Show {
        /// Any of their email addresses, or their user id.
        user: String,
    },
    /// Create someone's account, as a member of the instance whose address
    /// the allowlist admits. They sign in with --password-stdin's
    /// password, with the link `fctl admin reset-link` makes, or with
    /// Google where the instance has it and Google hosts their address
    /// (Google Workspace or Gmail). Takes an admin token.
    #[command(
        after_help = "Examples:\n  FCTL_TOKEN=fctl_adm_… fctl admin users create ada@acme.dev --given-name Ada --family-name Lovelace\n  echo \"$PASSWORD\" | fctl admin users create grace@acme.dev --given-name Grace --family-name Hopper --password-stdin"
    )]
    Create {
        email: String,
        /// Their first names, all of them.
        #[arg(long, value_name = "NAMES")]
        given_name: Option<String>,
        /// Their last name.
        #[arg(long, value_name = "NAME", requires = "given_name")]
        family_name: Option<String>,
        /// Their full name, when you don't know its parts.
        #[arg(long, value_name = "NAME", conflicts_with = "given_name")]
        name: Option<String>,
        /// What agents call them (default: their first names).
        #[arg(long, value_name = "NAME")]
        preferred_name: Option<String>,
        /// The address their commits credit them with.
        #[arg(long, value_name = "EMAIL")]
        commit_email: Option<String>,
        /// Read their password from stdin (one line).
        #[arg(long)]
        password_stdin: bool,
    },
    /// Change someone's account: their names, primary address, commit
    /// email, picture or the Claude token their sessions run on. Takes an
    /// admin token. It reads and checks the picture and the token before
    /// it changes anything; each part is then a request of its own, so when
    /// the instance refuses one, those before it stay made.
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["email", "given_name", "family_name", "name", "preferred_name", "commit_email", "avatar", "claude_token"])),
        after_help = "Examples:\n  fctl admin users set ada@acme.dev --email ada.lovelace@acme.dev\n  fctl admin users set ada@acme.dev --family-name Byron --preferred-name Ada\n  fctl admin users set ada@acme.dev --avatar ada.png\n  fctl admin users set ada@acme.dev --commit-email 1234+ada@users.noreply.github.com\n  echo \"$SETUP_TOKEN\" | fctl admin users set ada@acme.dev --claude-token - --claude-label \"Acme seat\"\n  fctl admin users set ada@acme.dev --claude-token none"
    )]
    Set {
        /// Any of their email addresses, or their user id.
        user: String,
        /// Their primary address: it signs in from now on, and the one it
        /// replaces doesn't any more.
        #[arg(long, value_name = "EMAIL")]
        email: Option<String>,
        /// Their first names, all of them.
        #[arg(long, value_name = "NAMES")]
        given_name: Option<String>,
        /// Their last name ("" clears it).
        #[arg(long, value_name = "NAME")]
        family_name: Option<String>,
        /// Their full name, when you don't know its parts.
        #[arg(long, value_name = "NAME", conflicts_with_all = ["given_name", "family_name"])]
        name: Option<String>,
        /// What agents call them ("" for their first names).
        #[arg(long, value_name = "NAME")]
        preferred_name: Option<String>,
        /// The address their commits credit them with ("" removes it).
        #[arg(long, value_name = "EMAIL")]
        commit_email: Option<String>,
        /// Their picture: a PNG, JPEG, GIF or WebP image of at most 256 KB
        /// and 1024 pixels a side (square works best), or `none` for their
        /// initials.
        #[arg(long, value_name = "FILE|none")]
        avatar: Option<String>,
        /// The Claude token (`claude setup-token`'s) their sessions run
        /// on: `-` reads it from stdin, `none` removes the one an admin
        /// token set. It replaces the one an admin token set before.
        #[arg(long, value_name = "-|none")]
        claude_token: Option<String>,
        /// What their settings call the Claude token.
        #[arg(long, value_name = "LABEL", requires = "claude_token")]
        claude_label: Option<String>,
    },
    /// Add someone to workspaces, as a member or an admin, and to
    /// projects, with a role; or change their role there. They join a
    /// project's workspace first. Takes an admin token. Each place is a
    /// request of its own, in order: when the instance refuses one, those
    /// before it stay made.
    #[command(
        group(ArgGroup::new("where").required(true).multiple(true).args(["workspaces", "projects"])),
        after_help = "Examples:\n  fctl admin users add ada@acme.dev -w acme -p web:editor\n  fctl admin users add ada@acme.dev -w platform:admin -p api:maintainer -p docs:viewer"
    )]
    Add {
        /// Any of their email addresses, or their user id.
        user: String,
        /// A workspace, `slug` or `slug:admin` (repeatable).
        #[arg(short, long = "workspace", value_name = "WORKSPACE[:admin]")]
        workspaces: Vec<String>,
        /// A project, `slug:ROLE` with viewer, editor, maintainer or owner
        /// (repeatable; `slug` alone: editor).
        #[arg(short, long = "project", value_name = "PROJECT[:ROLE]")]
        projects: Vec<String>,
    },
    /// Take someone out of projects, and out of workspaces with their
    /// projects there. Takes an admin token.
    #[command(
        group(ArgGroup::new("where").required(true).multiple(true).args(["workspaces", "projects"])),
        after_help = "Examples:\n  fctl admin users remove ada@acme.dev -p web\n  fctl admin users remove ada@acme.dev -w acme"
    )]
    Remove {
        /// Any of their email addresses, or their user id.
        user: String,
        /// A workspace (repeatable).
        #[arg(short, long = "workspace", value_name = "WORKSPACE")]
        workspaces: Vec<String>,
        /// A project (repeatable).
        #[arg(short, long = "project", value_name = "PROJECT")]
        projects: Vec<String>,
    },
}

/// A project role as `fctl admin users add -p slug:ROLE` names it.
fn project_role(spec: &str) -> Result<(String, ProjectRole)> {
    let (slug, role) = match spec.rsplit_once(':') {
        Some((slug, role)) => (
            slug,
            ProjectRole::parse(role).with_context(|| {
                format!("{role} isn't a project role: use viewer, editor, maintainer or owner")
            })?,
        ),
        None => (spec, ProjectRole::Editor),
    };
    Ok((slug.to_owned(), role))
}

/// A workspace and role as `fctl admin users add -w slug[:admin]` names
/// them.
fn workspace_role(spec: &str) -> Result<(String, WorkspaceRole)> {
    Ok(match spec.rsplit_once(':') {
        Some((slug, "admin")) => (slug.to_owned(), WorkspaceRole::Admin),
        Some((slug, "member")) => (slug.to_owned(), WorkspaceRole::Member),
        Some((_, other)) => bail!("{other} isn't a workspace role: use admin or member"),
        None => (spec.to_owned(), WorkspaceRole::Member),
    })
}

/// One line from stdin, or, at a terminal, typed without showing it.
fn secret(prompt: &str) -> Result<String> {
    let value = if std::io::stdin().is_terminal() {
        crate::prompt::read_secret(prompt)?
    } else {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_owned()
    };
    if value.trim().is_empty() {
        bail!("Nothing came on stdin");
    }
    Ok(value)
}

pub async fn tokens(api: &fctl_client::Api, cmd: AdminTokensCmd, json: bool) -> Result<()> {
    match cmd {
        AdminTokensCmd::List => {
            let list = api.admin_tokens().await?;
            if json {
                return style::print_json(&list);
            }
            if !list.enabled {
                println!(
                    "{}",
                    style::amber(
                        "This instance doesn't take admin tokens: its deployment turns them on with FCTL_ADMIN_TOKENS=1."
                    )
                );
            }
            if list.tokens.is_empty() {
                println!(
                    "{}",
                    style::dim("No admin tokens. `fctl admin tokens create NAME` makes one.")
                );
            }
            for t in &list.tokens {
                print_token(t);
            }
            Ok(())
        }
        AdminTokensCmd::Create { name, days } => {
            let created = api
                .create_admin_token(&CreateAdminTokenRequest {
                    name,
                    expires_in_days: days,
                })
                .await?;
            if json {
                return style::print_json(&created);
            }
            println!("{}", created.secret);
            eprintln!(
                "{}",
                style::dim(
                    "Acts as you on the admin API's people endpoints alone. Shown once: use it as FCTL_TOKEN or an `Authorization: Bearer` header."
                )
            );
            Ok(())
        }
        AdminTokensCmd::Revoke { token } => {
            let list = api.admin_tokens().await?;
            let wanted = token.trim();
            let matching: Vec<&AdminToken> = list
                .tokens
                .iter()
                .filter(|t| t.id.as_str() == wanted || t.name == wanted)
                .collect();
            let t = match matching.as_slice() {
                [t] => *t,
                [] => bail!("No admin token is called {wanted}. `fctl admin tokens` lists them."),
                _ => bail!(
                    "{} admin tokens are called {wanted}: revoke one by its id (`fctl admin tokens` lists them)",
                    matching.len()
                ),
            };
            let out = api.revoke_admin_token(t.id.as_str()).await?;
            if json {
                return style::print_json(&out);
            }
            println!(
                "{} Revoked {}",
                style::accent("✓"),
                style::bold(&style::terminal_safe(&t.name))
            );
            Ok(())
        }
    }
}

fn print_token(t: &AdminToken) {
    let mut meta = vec![
        format!("by {}", style::terminal_safe(&t.created_by.name)),
        match t.last_used_at {
            Some(at) => format!("used {}", style::ago(at)),
            None => "not used yet".to_owned(),
        },
    ];
    if let Some(expires) = t.expires_at {
        meta.push(format!("expires {}", style::until(expires)));
    }
    println!(
        "  {}  {}  {}",
        style::bold(&style::terminal_safe(&t.name)),
        style::dim(t.id.as_str()),
        style::dim(&meta.join(" · "))
    );
}

/// Someone's account, as `fctl admin users show` prints it.
pub fn print_user(a: &AdminUser) {
    let u = &a.user;
    println!(
        "{}  {}  {}  {}",
        style::bold(&style::terminal_safe(&u.name)),
        u.email,
        u.role.as_str(),
        match u.status {
            fctl_proto::api::UserStatus::Active => style::accent("active"),
            fctl_proto::api::UserStatus::Disabled => style::red("disabled"),
        }
    );
    let mut lines: Vec<(&str, String)> = vec![("Id", u.id.to_string())];
    if let Some(p) = &u.preferred_name {
        lines.push(("Called", style::terminal_safe(p).into_owned()));
    }
    if !u.emails.is_empty() {
        lines.push(("Also", u.emails.join(", ")));
    }
    lines.push((
        "Commits",
        a.commit_email
            .clone()
            .unwrap_or_else(|| "don't credit them".to_owned()),
    ));
    lines.push((
        "Signs in",
        match (u.has_password, u.two_factor, u.passkeys) {
            (false, _, 0) => "without a password yet".to_owned(),
            (true, false, 0) => "with a password".to_owned(),
            (_, true, _) => "with two-factor sign-in".to_owned(),
            (_, false, _) => "with passkeys".to_owned(),
        },
    ));
    lines.push((
        "Workspaces",
        if a.workspaces.is_empty() {
            "none".to_owned()
        } else {
            a.workspaces
                .iter()
                .map(|w| {
                    format!(
                        "{} ({})",
                        w.workspace.slug,
                        match w.role {
                            WorkspaceRole::Owner => "primary admin",
                            r => r.as_str(),
                        }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        },
    ));
    if !a.projects.is_empty() {
        lines.push((
            "Projects",
            a.projects
                .iter()
                .map(|p| format!("{} ({})", p.slug, p.role.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    lines.push((
        "Claude",
        match &a.claude {
            None => "no token set by an admin token".to_owned(),
            Some(c) => format!(
                "{}, set {}{}{}",
                c.label.as_deref().unwrap_or("a token"),
                style::ago(c.updated_at),
                if c.status == fctl_proto::api::CredentialStatus::Invalid {
                    ", which stopped working"
                } else {
                    ""
                },
                if c.is_default { ", their default" } else { "" }
            ),
        },
    ));
    for (label, value) in lines {
        println!("  {}  {value}", style::dim(&format!("{label:<10}")));
    }
}

pub async fn users(api: &fctl_client::Api, cmd: UsersCmd, json: bool) -> Result<()> {
    match cmd {
        UsersCmd::List => crate::cmd::admin::list_users(api, json).await?,
        UsersCmd::Show { user } => {
            let a = api.admin_user(&user).await?;
            if json {
                return style::print_json(&a);
            }
            print_user(&a);
        }
        UsersCmd::Create {
            email,
            given_name,
            family_name,
            name,
            preferred_name,
            commit_email,
            password_stdin,
        } => {
            if given_name.is_none() && name.is_none() {
                bail!("Say what they are called: --given-name (with --family-name), or --name");
            }
            let password = if password_stdin {
                Some(secret("Their password (input hidden): ")?)
            } else {
                None
            };
            let a = api
                .create_user(&CreateUserRequest {
                    email,
                    given_name,
                    family_name,
                    name,
                    preferred_name,
                    commit_email,
                    password,
                })
                .await?;
            if json {
                return style::print_json(&a);
            }
            println!(
                "{} Created {}'s account ({})",
                style::accent("✓"),
                style::terminal_safe(&a.user.name),
                a.user.id
            );
            if !a.user.has_password {
                println!(
                    "{}",
                    style::dim(&format!(
                        "They have no password: `fctl admin reset-link {}` makes a link that sets one.",
                        a.user.email
                    ))
                );
            }
        }
        UsersCmd::Set {
            user,
            email,
            given_name,
            family_name,
            name,
            preferred_name,
            commit_email,
            avatar,
            claude_token,
            claude_label,
        } => {
            // What it reads and checks comes first: a file it can't read or
            // use, nothing on stdin or something that isn't a Claude token
            // changes nothing.
            let avatar = match avatar.as_deref() {
                None => None,
                Some(a) if a.eq_ignore_ascii_case("none") => Some(None),
                Some(a) => {
                    let bytes = std::fs::read(a).with_context(|| format!("reading {a}"))?;
                    fctl_domain::avatar::check(&bytes)
                        .map_err(|e| anyhow!("{a}: {}", e.message))?;
                    Some(Some(bytes))
                }
            };
            let claude = match claude_token.as_deref() {
                None => None,
                Some("none") => Some(None),
                Some("-") => {
                    let token = secret("Their Claude token (input hidden): ")?
                        .trim()
                        .to_owned();
                    if let Some(problem) = fctl_domain::claude::setup_token_problem(&token) {
                        bail!("{problem}");
                    }
                    Some(Some(token))
                }
                Some(_) => bail!(
                    "--claude-token takes `-`, to read it from stdin, or `none`: a token on the command line would show in your shell's history"
                ),
            };
            let mut said = Vec::new();
            let mut last: Option<AdminUser> = None;
            let mut who = user;
            let update = UpdateUserRequest {
                email: email.clone(),
                given_name,
                family_name,
                name,
                preferred_name,
                commit_email,
                ..Default::default()
            };
            if update != UpdateUserRequest::default() {
                let updated = api.update_user(&who, &update).await?;
                // The rest goes to the account by id: its address may have
                // just changed.
                who = updated.id.to_string();
                if email.is_some() {
                    said.push(format!("Their address is {}", updated.email));
                }
                said.push(format!(
                    "Updated {}'s account",
                    style::terminal_safe(&updated.name)
                ));
            }
            if let Some(picture) = avatar {
                last = Some(match picture {
                    None => {
                        said.push("Their initials show where people see their name".to_owned());
                        api.remove_user_avatar(&who).await?
                    }
                    Some(bytes) => {
                        said.push("Their picture shows where people see their name".to_owned());
                        api.set_user_avatar(&who, bytes).await?
                    }
                });
            }
            if let Some(token) = claude {
                last = Some(match token {
                    None => {
                        said.push(
                            "The Claude token an admin token set for them is gone".to_owned(),
                        );
                        api.remove_user_claude(&who).await?
                    }
                    Some(token) => {
                        said.push("Their sessions run on the Claude token you set".to_owned());
                        api.set_user_claude(
                            &who,
                            &AddClaudeTokenRequest {
                                token,
                                label: claude_label,
                                made_at: None,
                            },
                        )
                        .await?
                    }
                });
            }
            if json {
                let a = match last {
                    Some(a) => a,
                    None => api.admin_user(&who).await?,
                };
                return style::print_json(&a);
            }
            for s in said {
                println!("{} {s}", style::accent("✓"));
            }
        }
        UsersCmd::Add {
            user,
            workspaces,
            projects,
        } => {
            // Every place is read before the first changes.
            let workspaces = workspaces
                .iter()
                .map(|w| workspace_role(w))
                .collect::<Result<Vec<_>>>()?;
            let projects = projects
                .iter()
                .map(|p| project_role(p))
                .collect::<Result<Vec<_>>>()?;
            let mut last = None;
            for (slug, role) in &workspaces {
                let (slug, role) = (slug.as_str(), *role);
                let a = api.set_user_workspace(&user, slug, Some(role)).await?;
                if !json {
                    println!(
                        "{} {} is {} of {slug}",
                        style::accent("✓"),
                        style::terminal_safe(&a.user.name),
                        if role == WorkspaceRole::Admin {
                            "an admin"
                        } else {
                            "a member"
                        }
                    );
                }
                last = Some(a);
            }
            for (slug, role) in &projects {
                let (slug, role) = (slug.as_str(), *role);
                let a = api.set_user_project(&user, slug, role).await?;
                if !json {
                    println!(
                        "{} {} is {} of {slug}",
                        style::accent("✓"),
                        style::terminal_safe(&a.user.name),
                        match role {
                            ProjectRole::Owner => "an owner",
                            ProjectRole::Maintainer => "a maintainer",
                            ProjectRole::Editor => "an editor",
                            ProjectRole::Viewer => "a viewer",
                        }
                    );
                }
                last = Some(a);
            }
            if json && let Some(a) = last {
                return style::print_json(&a);
            }
        }
        UsersCmd::Remove {
            user,
            workspaces,
            projects,
        } => {
            let mut last = None;
            for p in &projects {
                let a = api.remove_user_project(&user, p).await?;
                if !json {
                    println!(
                        "{} {} isn't a member of {p} any more",
                        style::accent("✓"),
                        style::terminal_safe(&a.user.name)
                    );
                }
                last = Some(a);
            }
            for w in &workspaces {
                let a = api.remove_user_workspace(&user, w).await?;
                if !json {
                    println!(
                        "{} {} left {w}, with its projects",
                        style::accent("✓"),
                        style::terminal_safe(&a.user.name)
                    );
                }
                last = Some(a);
            }
            if json && let Some(a) = last {
                return style::print_json(&a);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_roles_after_the_slug() -> Result<()> {
        assert_eq!(
            project_role("web:maintainer")?,
            ("web".to_owned(), ProjectRole::Maintainer)
        );
        assert_eq!(
            project_role("web")?,
            ("web".to_owned(), ProjectRole::Editor)
        );
        assert!(project_role("web:boss").is_err());
        assert_eq!(
            workspace_role("acme:admin")?,
            ("acme".to_owned(), WorkspaceRole::Admin)
        );
        assert_eq!(
            workspace_role("acme")?,
            ("acme".to_owned(), WorkspaceRole::Member)
        );
        assert!(workspace_role("acme:owner").is_err());
        Ok(())
    }
}

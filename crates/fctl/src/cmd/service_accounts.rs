//! `fctl service-accounts`: a workspace's credentials for programs of its
//! own (docs/automation.md#service-accounts), which start sessions for the
//! people who allow them, run its schedules and read its sessions. Its
//! admins make, rename, give new secrets and delete them; every member
//! says whether each may start sessions for them; with a service account's
//! token, `fctl whoami` and `people` say what it is and whom it may act
//! for.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_proto::service_accounts::{
    CreateServiceAccountRequest, ServiceAccount, ServiceAccountAnswer, ServiceAccountPeople,
    UpdateServiceAccountRequest,
};

#[derive(Args)]
pub struct ServiceAccountsArgs {
    /// The workspace (slug or id); default from `fctl workspaces use`, else
    /// your only one.
    #[arg(short, long, global = true)]
    pub workspace: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<ServiceAccountsCmd>,
}

#[derive(Subcommand)]
pub enum ServiceAccountsCmd {
    /// The workspace's service accounts, with your answer to each (the
    /// default); its admins see who keeps each and how its people answered.
    #[command(visible_alias = "ls")]
    List,
    /// Make a service account (the workspace's admins). Prints its secret,
    /// once: the program that holds it starts sessions for the people who
    /// allow it, named with `--on-behalf-of` or the Fctl-On-Behalf-Of
    /// header.
    #[command(after_help = "Examples:
  fctl service-accounts create \"Acme portal\" --description \"Our agent portal at portal.acme.dev\"
  FCTL_TOKEN=fctl_svc_… fctl run -p web --on-behalf-of ada@acme.dev \"Fix the login redirect\"")]
    Create {
        /// What to call it: what uses it.
        name: String,
        /// What it is and where people use it, for the people it asks to
        /// let it start sessions for them.
        #[arg(long)]
        description: Option<String>,
    },
    /// A service account: what it is, who keeps it, and how its people
    /// answered.
    Show {
        /// Its name or id (sva_…).
        service_account: String,
    },
    /// Rename a service account or say what it is (the workspace's admins).
    Set {
        /// Its name or id (sva_…).
        service_account: String,
        #[arg(long)]
        name: Option<String>,
        /// What it is, for the people it asks; "" removes it.
        #[arg(long)]
        description: Option<String>,
    },
    /// Make a service account a new secret, in place of its old one, which
    /// stops working at once (the workspace's admins). You keep it from
    /// then on: one that stopped as the admin who kept it left works again.
    Secret {
        /// Its name or id (sva_…).
        service_account: String,
    },
    /// Delete a service account (the workspace's admins): it works no more,
    /// and the sessions it started stay their people's.
    Rm {
        /// Its name or id (sva_…).
        service_account: String,
    },
    /// Let a service account start sessions for you, on your Claude
    /// subscription, and message their agents for you.
    Allow {
        /// Its name or id (sva_…).
        service_account: String,
    },
    /// Don't let a service account start sessions for you, or no longer.
    Deny {
        /// Its name or id (sva_…).
        service_account: String,
    },
    /// The workspace's people, and whether a service account may act for
    /// each (the workspace's admins; with a service account's token, its
    /// own).
    People {
        /// Its name or id (sva_…); none with a service account's token.
        service_account: Option<String>,
    },
}

/// Whether the client signs in with a service account's token.
fn is_service_account(api: &Api) -> bool {
    api.token()
        .is_some_and(|t| t.starts_with(fctl_domain::tokens::SecretKind::Service.prefix()))
}

pub async fn run(server: Option<&str>, args: ServiceAccountsArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let cmd = args.cmd.unwrap_or(ServiceAccountsCmd::List);
    if is_service_account(&api) {
        return match cmd {
            ServiceAccountsCmd::List => whoami(&api, json).await,
            ServiceAccountsCmd::People { .. } => {
                let me = api.service_account_self().await?;
                let people = api
                    .service_account_people(me.service_account.id.as_str())
                    .await?;
                print_people(&me.service_account, &people, json)
            }
            _ => bail!(
                "A service account's token reads and starts sessions: sign in as one of its workspace's people to change it."
            ),
        };
    }
    let workspace = super::workspaces::resolve(&api, args.workspace).await?;
    let wid = workspace.id.as_str();
    match cmd {
        ServiceAccountsCmd::List => {
            let list = api.service_accounts(wid).await?;
            if json {
                return style::print_json(&list);
            }
            if list.service_accounts.is_empty() {
                println!(
                    "{} has no service accounts.{}",
                    workspace.name,
                    if workspace.role.is_admin() {
                        " Make one for a program of yours: fctl service-accounts create \"Acme portal\""
                    } else {
                        ""
                    }
                );
                return Ok(());
            }
            for (i, s) in list.service_accounts.iter().enumerate() {
                if i > 0 {
                    println!();
                }
                print_account(s);
            }
            if !list.consent {
                println!(
                    "\n{}",
                    style::dim(
                        "This instance doesn't ask people: its service accounts start sessions for anyone in their workspace."
                    )
                );
            }
            Ok(())
        }
        ServiceAccountsCmd::Create { name, description } => {
            let created = api
                .create_service_account(wid, &CreateServiceAccountRequest { name, description })
                .await?;
            if json {
                return style::print_json(&created);
            }
            let s = &created.service_account;
            eprintln!(
                "Made {} ({}), a service account of {}. Its secret, shown once here (a new one replaces it: fctl service-accounts secret {}):",
                style::bold(&style::terminal_safe(&s.name)),
                s.id,
                workspace.name,
                s.id
            );
            println!("{}", created.secret);
            eprintln!("{}", style::dim(&after_made(s, &workspace.slug)));
            Ok(())
        }
        ServiceAccountsCmd::Show { service_account } => {
            let s = find(&api, wid, &service_account).await?;
            let people = if s.can_manage {
                Some(api.service_account_people(s.id.as_str()).await?)
            } else {
                None
            };
            if json {
                return style::print_json(
                    &serde_json::json!({ "service_account": s, "people": people.map(|p| p.people) }),
                );
            }
            print_account(&s);
            if let Some(people) = &people {
                println!();
                print_people_lines(people);
            }
            Ok(())
        }
        ServiceAccountsCmd::Set {
            service_account,
            name,
            description,
        } => {
            if name.is_none() && description.is_none() {
                bail!("Say what to change: --name, --description, or both.");
            }
            let s = find(&api, wid, &service_account).await?;
            let s = api
                .update_service_account(
                    s.id.as_str(),
                    &UpdateServiceAccountRequest { name, description },
                )
                .await?;
            if json {
                return style::print_json(&s);
            }
            print_account(&s);
            Ok(())
        }
        ServiceAccountsCmd::Secret { service_account } => {
            let s = find(&api, wid, &service_account).await?;
            let made = api.new_service_account_secret(s.id.as_str()).await?;
            if json {
                return style::print_json(&made);
            }
            eprintln!(
                "{}'s new secret, shown once here; its old one stopped working, and you keep it from now on:",
                style::bold(&style::terminal_safe(&s.name))
            );
            println!("{}", made.secret);
            Ok(())
        }
        ServiceAccountsCmd::Rm { service_account } => {
            let s = find(&api, wid, &service_account).await?;
            api.delete_service_account(s.id.as_str()).await?;
            if json {
                return style::print_json(&serde_json::json!({ "ok": true }));
            }
            println!(
                "Deleted {}: its secret works no more, and the sessions it started stay their people's.",
                style::terminal_safe(&s.name)
            );
            Ok(())
        }
        ServiceAccountsCmd::Allow { service_account } => {
            answer(&api, wid, &service_account, true, json).await
        }
        ServiceAccountsCmd::Deny { service_account } => {
            answer(&api, wid, &service_account, false, json).await
        }
        ServiceAccountsCmd::People { service_account } => {
            let Some(which) = service_account else {
                bail!(
                    "Which service account? Name it: fctl service-accounts people \"Acme portal\""
                );
            };
            let s = find(&api, wid, &which).await?;
            let people = api.service_account_people(s.id.as_str()).await?;
            print_people(&s, &people, json)
        }
    }
}

async fn answer(api: &Api, workspace: &str, which: &str, allow: bool, json: bool) -> Result<()> {
    let s = find(api, workspace, which).await?;
    let s = api.answer_service_account(s.id.as_str(), allow).await?;
    if json {
        return style::print_json(&s);
    }
    let name = style::terminal_safe(&s.name);
    match (allow, s.consent) {
        (true, _) => println!(
            "{name} may now start sessions for you in {}, on your Claude subscription. Take it back any time: fctl service-accounts deny \"{name}\"",
            s.workspace.name
        ),
        (false, true) => println!("{name} won't start sessions for you."),
        (false, false) => println!(
            "Noted. This instance doesn't ask people, though: {name} starts sessions for anyone in {}.",
            s.workspace.name
        ),
    }
    Ok(())
}

/// A workspace's service account by name or id.
async fn find(api: &Api, workspace: &str, which: &str) -> Result<ServiceAccount> {
    let list = api.service_accounts(workspace).await?;
    let found = list
        .service_accounts
        .iter()
        .find(|s| s.id.as_str() == which)
        .or_else(|| {
            list.service_accounts
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(which.trim()))
        })
        .cloned();
    found.with_context(|| {
        let names: Vec<&str> = list
            .service_accounts
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        if names.is_empty() {
            "The workspace has no service accounts.".to_owned()
        } else {
            format!(
                "No service account is called {which}. Its: {}",
                names.join(", ")
            )
        }
    })
}

/// What to do with a service account just made.
fn after_made(s: &ServiceAccount, workspace: &str) -> String {
    let asks = if s.consent {
        " It starts sessions only for the people who allow it: fctl asks each of them."
    } else {
        ""
    };
    format!(
        "Use it as FCTL_TOKEN, or as the bearer token, naming the person each request acts for:{asks}\n  FCTL_TOKEN=… fctl run -p <project> --on-behalf-of ada@example.com \"…\"\n  curl -H \"Authorization: Bearer …\" -H \"Fctl-On-Behalf-Of: ada@example.com\" …/api/v1/projects/<project>/sessions\nWho has allowed it: fctl service-accounts people \"{}\" -w {workspace}",
        s.name
    )
}

fn answer_words(s: &ServiceAccount) -> String {
    if !s.consent {
        return "starts sessions for anyone here".to_owned();
    }
    match s.answer {
        Some(ServiceAccountAnswer::Allowed) => "may start sessions for you".to_owned(),
        Some(ServiceAccountAnswer::Declined) => "you didn't allow it".to_owned(),
        _ => "asks to start sessions for you: fctl service-accounts allow, or deny".to_owned(),
    }
}

fn print_account(s: &ServiceAccount) {
    let dot = if s.stopped.is_some() {
        style::amber("●")
    } else if s.acts_for_you {
        style::accent("●")
    } else {
        style::dim("○")
    };
    println!(
        "{dot} {}  {}",
        style::bold(&style::terminal_safe(&s.name)),
        style::dim(s.id.as_str())
    );
    if let Some(d) = &s.description {
        println!("  {}", style::terminal_safe(d));
    }
    let mut facts = vec![answer_words(s)];
    if let Some(p) = &s.people {
        facts.push(format!(
            "{} allowed, {} didn't, {} haven't answered",
            p.allowed, p.declined, p.unanswered
        ));
    }
    if let Some(n) = s.sessions {
        facts.push(style::count(n, "session started", "sessions started"));
    }
    if let Some(at) = s.last_used_at {
        facts.push(format!("last used {}", style::ago(at)));
    }
    println!("  {}", style::dim(&facts.join(" · ")));
    if let Some(k) = &s.keeper {
        let hint = s.secret_hint.as_deref().unwrap_or("");
        println!(
            "  {}",
            style::dim(&format!(
                "secret …{hint}, kept by {}{}",
                style::terminal_safe(&k.name),
                s.secret_made_at
                    .map(|at| format!(", made {}", style::ago(at)))
                    .unwrap_or_default()
            ))
        );
    }
    if let Some(why) = &s.stopped {
        println!("  {}", style::amber(&style::terminal_safe(why)));
    }
}

fn print_people(s: &ServiceAccount, people: &ServiceAccountPeople, json: bool) -> Result<()> {
    if json {
        return style::print_json(people);
    }
    println!(
        "{}, in {}:",
        style::bold(&style::terminal_safe(&s.name)),
        s.workspace.name
    );
    print_people_lines(people);
    Ok(())
}

fn print_people_lines(people: &ServiceAccountPeople) {
    let can = people.people.iter().filter(|p| p.acts_for).count();
    for p in &people.people {
        let (dot, words) = if p.acts_for {
            (
                style::accent("●"),
                if people.consent {
                    "allowed it".to_owned()
                } else {
                    "it acts for them".to_owned()
                },
            )
        } else {
            (
                style::dim("○"),
                p.why_not
                    .clone()
                    .unwrap_or_else(|| "it may not act for them".to_owned()),
            )
        };
        println!(
            "{dot} {} <{}>  {}",
            style::terminal_safe(&p.user.name),
            style::terminal_safe(&p.email),
            style::dim(&style::terminal_safe(&words))
        );
    }
    println!(
        "{}",
        style::dim(&format!(
            "It may act for {can} of {}.",
            style::count(people.people.len() as u64, "person", "people")
        ))
    );
}

/// `fctl whoami` with a service account's token.
pub async fn whoami(api: &Api, json: bool) -> Result<()> {
    let me = api.service_account_self().await?;
    if json {
        return style::print_json(&me);
    }
    let s = &me.service_account;
    println!(
        "{}, a service account of {}",
        style::bold(&style::terminal_safe(&s.name)),
        s.workspace.name
    );
    println!("{}", style::dim(&format!("{} · {}", api.base(), s.id)));
    match (&me.acting_for, &me.acting_for_email) {
        (Some(who), email) => println!(
            "Acting for {}{}",
            style::terminal_safe(&who.name),
            email
                .as_deref()
                .map(|e| format!(" <{e}>"))
                .unwrap_or_default()
        ),
        (None, _) => {
            let reach = match &s.people {
                Some(p) if s.consent => format!(
                    "It starts sessions for the people of {} who allowed it: {} of {}.",
                    s.workspace.name,
                    p.allowed,
                    p.allowed + p.declined + p.unanswered
                ),
                _ => format!(
                    "It starts sessions for anyone in {}: this instance doesn't ask people.",
                    s.workspace.name
                ),
            };
            println!("{reach} Name the person with --on-behalf-of.");
        }
    }
    Ok(())
}

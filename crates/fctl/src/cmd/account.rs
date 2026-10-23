//! `fctl account …`: your email addresses and the addresses commits credit
//! you with, two-factor sign-in, passkeys, export your data, delete your
//! account.

use crate::config::{self, Credentials};
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_proto::api::{EmailAddress, UpdateEmailRequest, UpdateMeRequest};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum LinearAccountCmd {
    /// Your Linear accounts (the default).
    #[command(visible_alias = "ls")]
    List,
    /// Link your Linear account: opens Linear's sign-in in your browser,
    /// where you're signed in to fctl. fctl reads who you are there and
    /// forgets the sign-in.
    Connect {
        /// Print the link without opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Forget your Linear account in a Linear workspace: what you delegate
    /// there starts nothing until you link it again.
    Disconnect {
        /// The Linear workspace: its name, its address (linear.app/<key>)
        /// or its id.
        organization: String,
    },
}

#[derive(Subcommand)]
pub enum AccountCmd {
    /// Change your profile: the address commits credit you with.
    #[command(
        after_help = "Examples:\n  fctl account set --commit-email 1234+ada@users.noreply.github.com\n  fctl account set --commit-email \"\"   # commits don't credit you\n  fctl account set --for ada.lovelace@company.example --commit-email ada.lovelace@company.example"
    )]
    Set {
        /// Commits in the sessions you start credit you with this address, in
        /// a Co-authored-by trailer anyone who can read the repository sees.
        /// Use one your GitHub account has (its noreply address works), so
        /// GitHub links the commits to you. "" removes it.
        #[arg(long, value_name = "EMAIL")]
        commit_email: String,
        /// For the workspaces you are a member of as this address of yours
        /// (default: your primary address).
        #[arg(long = "for", value_name = "ADDRESS")]
        address: Option<String>,
    },
    /// Your email addresses: each signs in to your account, has its own
    /// commit email, and is how the workspaces you joined as it know you.
    Emails,
    /// Make one of your addresses the primary one, where fctl writes to you.
    Primary {
        /// One of your addresses.
        address: String,
    },
    /// Disconnect an address from your account. You leave the workspaces
    /// you are a member of as it, and it no longer signs in.
    Disconnect {
        /// One of your addresses, not the primary one.
        address: String,
    },
    /// Your Linear accounts: who fctl knows you as in Linear workspaces, so
    /// that the issues you delegate to fctl there run as you, and your
    /// replies to its agents there reach them.
    #[command(
        after_help = "Examples:\n  fctl account linear\n  fctl account linear connect\n  fctl account linear disconnect acme"
    )]
    Linear {
        #[command(subcommand)]
        cmd: Option<LinearAccountCmd>,
    },
    /// Two-factor sign-in: codes from an authenticator app after your
    /// password, and recovery codes.
    #[command(subcommand, visible_alias = "2fa")]
    TwoFactor(super::two_factor::TwoFactorCmd),
    /// The passkeys you sign in with: list, rename and remove them (your
    /// browser adds them, in Settings).
    #[command(subcommand)]
    Passkeys(super::passkeys::PasskeysCmd),
    /// Download your personal data as JSON: your account, sign-in methods
    /// and two-factor sign-in, devices and tokens, Claude connections
    /// (without secrets), notification settings and browsers, projects, the
    /// sessions you started with their transcripts, the sessions you follow,
    /// and your activity.
    Export {
        /// Where to write it (default: fctl-export-<date>.json here; `-` prints it).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Confirm it's you, where the instance asks for it before sensitive
    /// actions (deleting projects, new access tokens, exporting your
    /// data…): with a code from your authenticator app, or your passkey in
    /// the browser. This CLI then isn't asked again for 15 minutes. In a
    /// terminal, fctl also asks by itself when an action needs it.
    #[command(
        after_help = "Examples:\n  fctl account verify\n  fctl account verify --code 123456 && fctl projects delete old-app --confirm old-app"
    )]
    Verify {
        /// A code from your authenticator app, or a recovery code (asked
        /// for in a terminal when left out).
        #[arg(long)]
        code: Option<String>,
    },
    /// Delete your account and the projects only you are a member of, with
    /// their sessions. This cannot be undone.
    Delete {
        /// Your email address, to confirm without being asked (for scripts).
        #[arg(long, value_name = "EMAIL")]
        confirm: Option<String>,
    },
}

pub async fn run(server: Option<&str>, cmd: AccountCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        AccountCmd::TwoFactor(cmd) => super::two_factor::run(&api, cmd, json).await,
        AccountCmd::Linear { cmd } => {
            linear(&api, cmd.unwrap_or(LinearAccountCmd::List), json).await
        }
        AccountCmd::Passkeys(cmd) => super::passkeys::run(&api, cmd, json).await,
        AccountCmd::Set {
            commit_email,
            address: Some(address),
        } => {
            let me = api
                .update_email(
                    &address,
                    &UpdateEmailRequest {
                        commit_email: Some(commit_email),
                        primary: None,
                    },
                )
                .await?;
            if json {
                return style::print_json(&me);
            }
            print_emails(&me.emails);
            println!(
                "{}",
                style::dim("Sessions take the change when they next start or resume.")
            );
            Ok(())
        }
        AccountCmd::Emails => {
            let me = super::auth::account(&api).await?;
            if json {
                return style::print_json(&me.emails);
            }
            print_emails(&me.emails);
            Ok(())
        }
        AccountCmd::Primary { address } => {
            let me = api
                .update_email(
                    &address,
                    &UpdateEmailRequest {
                        commit_email: None,
                        primary: Some(true),
                    },
                )
                .await?;
            if json {
                return style::print_json(&me);
            }
            println!(
                "{} {} is your primary address",
                style::accent("✓"),
                me.user.email
            );
            Ok(())
        }
        AccountCmd::Disconnect { address } => {
            let me = api.remove_email(&address).await?;
            if json {
                return style::print_json(&me);
            }
            println!("{} Disconnected {address}", style::accent("✓"));
            Ok(())
        }
        AccountCmd::Set {
            commit_email,
            address: None,
        } => {
            let me = api
                .update_me(&UpdateMeRequest {
                    commit_email: Some(commit_email),
                    ..Default::default()
                })
                .await?;
            if json {
                return style::print_json(&me);
            }
            match &me.commit_email {
                Some(email) => println!(
                    "{} Commits in the sessions you start credit you as {} <{email}>",
                    style::accent("✓"),
                    me.user.name
                ),
                None => println!(
                    "{} Commits in the sessions you start don't credit you",
                    style::accent("✓")
                ),
            }
            println!(
                "{}",
                style::dim("Sessions take the change when they next start or resume.")
            );
            Ok(())
        }
        AccountCmd::Verify { code } => {
            let status = match code {
                Some(code) => api.verify(&code).await?,
                None => {
                    let status = api.verification().await?;
                    if !status.required {
                        if json {
                            return style::print_json(&status);
                        }
                        println!(
                            "Sensitive actions don't ask you to confirm it's you here: the instance doesn't ask for it, or you have neither a passkey nor an authenticator app."
                        );
                        return Ok(());
                    }
                    if status.verified {
                        status
                    } else if !std::io::stdin().is_terminal() {
                        bail!(
                            "pass --code with a code from your authenticator app: there is no terminal to ask in"
                        );
                    } else if crate::verify::confirm(&api, None).await? {
                        // It said so already.
                        if !json {
                            return Ok(());
                        }
                        api.verification().await?
                    } else {
                        bail!("Nothing was confirmed.");
                    }
                }
            };
            if json {
                return style::print_json(&status);
            }
            println!(
                "{} {}",
                style::accent("✓"),
                crate::verify::done(&status, style::now_ms())
            );
            Ok(())
        }
        AccountCmd::Export { output } => {
            let data = api.export().await?;
            if output.as_deref().is_some_and(|p| p.as_os_str() == "-") {
                std::io::stdout().write_all(&data)?;
                return Ok(());
            }
            let path = output.unwrap_or_else(|| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                PathBuf::from(fctl_domain::account::export_filename(now))
            });
            std::fs::write(&path, &data)?;
            if json {
                return style::print_json(&serde_json::json!({
                    "path": path,
                    "bytes": data.len(),
                }));
            }
            println!(
                "Wrote your data to {} ({} KB)",
                style::bold(&path.display().to_string()),
                data.len().div_ceil(1024)
            );
            Ok(())
        }
        AccountCmd::Delete { confirm } => {
            let me = super::auth::account(&api).await?;
            let plan = api.account_deletion().await?;
            if let Some(reason) = plan.blocked {
                bail!("{reason}");
            }
            let typed = match confirm {
                Some(email) => email,
                None => {
                    if !std::io::stdin().is_terminal() {
                        bail!(
                            "pass --confirm {} to delete your account without a terminal",
                            me.user.email
                        );
                    }
                    println!("{}", style::red("Deleting your account cannot be undone."));
                    println!(
                        "It deletes your sign-in methods and addresses, devices and tokens, and Claude connections."
                    );
                    for w in &plan.workspaces {
                        println!(
                            "  deletes the workspace {} ({}), which only you are in",
                            style::bold(&w.name),
                            w.slug
                        );
                    }
                    for h in &plan.handovers {
                        println!(
                            "  makes {} the primary admin of {}",
                            style::bold(&h.to.name),
                            h.workspace.name
                        );
                    }
                    for p in &plan.projects {
                        let sessions = match p.sessions {
                            0 => String::new(),
                            1 => ", with its session".into(),
                            n => format!(", with its {n} sessions"),
                        };
                        println!(
                            "  deletes the project {} ({}){sessions}",
                            style::bold(&p.name),
                            p.slug
                        );
                    }
                    if plan.kept_sessions > 0 {
                        println!(
                            "  keeps {} sessions you started in shared workspaces, as a deleted user",
                            plan.kept_sessions
                        );
                    }
                    print!("Type your email address ({}) to confirm: ", me.user.email);
                    std::io::stdout().flush()?;
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line.trim().to_owned()
                }
            };
            if !fctl_domain::account::confirms_deletion(&typed, &me.user.email) {
                bail!("that is not your email address; nothing was deleted");
            }
            let done = api.delete_account(&typed).await?;
            let mut creds = Credentials::load();
            creds.tokens.remove(api.base());
            creds.save()?;
            if json {
                return style::print_json(&done);
            }
            println!("Your account was deleted, and this machine is signed out.");
            Ok(())
        }
    }
}

/// Your addresses, the primary one first, with their commit emails and the
/// workspaces that know you by them.
fn print_emails(emails: &[EmailAddress]) {
    for e in emails {
        let mut line = format!(
            "{:<36} {}",
            style::bold(&e.email),
            if e.primary {
                style::accent("primary")
            } else {
                String::new()
            }
        );
        if !e.workspaces.is_empty() {
            line.push_str(&style::dim(&format!(
                "  {}",
                e.workspaces
                    .iter()
                    .map(|w| w.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        println!("{line}");
        println!(
            "  {}",
            style::dim(&match &e.commit_email {
                Some(c) => format!("commits credit you as {c}"),
                None => "commits don't credit you".into(),
            })
        );
    }
}

async fn linear(api: &fctl_client::Api, cmd: LinearAccountCmd, json: bool) -> Result<()> {
    match cmd {
        LinearAccountCmd::List => {
            let mine = api.my_linear().await?;
            if json {
                return style::print_json(&mine);
            }
            if !mine.configured {
                println!("This instance has no Linear app.");
                return Ok(());
            }
            if mine.accounts.is_empty() {
                println!(
                    "fctl doesn't know your Linear account yet. Link it with `fctl account linear connect`, so that what you delegate to fctl in Linear runs as you."
                );
            }
            for a in &mine.accounts {
                println!(
                    "{:<24} {:<28} {}",
                    style::bold(&style::terminal_safe(&a.organization_name)),
                    style::dim(&style::terminal_safe(
                        &a.url_key
                            .as_deref()
                            .map(|k| format!("linear.app/{k}"))
                            .unwrap_or_default()
                    )),
                    style::terminal_safe(&match &a.email {
                        Some(e) => format!("{} ({e})", a.name),
                        None => a.name.clone(),
                    })
                );
            }
        }
        LinearAccountCmd::Connect { no_browser } => {
            let url = api.link_linear_url();
            if json {
                return style::print_json(&serde_json::json!({ "url": url }));
            }
            println!(
                "Link your Linear account by signing in to Linear, in a browser where you're signed in to fctl:"
            );
            println!();
            println!("  {}", style::terminal_safe(&url));
            println!();
            if !no_browser && crate::cmd::auth::open_browser(&url) {
                println!("{}", style::dim("Opened it in your browser."));
            }
        }
        LinearAccountCmd::Disconnect { organization } => {
            let mine = api.my_linear().await?;
            let n = organization.trim();
            let Some(a) = mine.accounts.iter().find(|a| {
                a.organization == n
                    || a.organization_name.eq_ignore_ascii_case(n)
                    || a.url_key
                        .as_deref()
                        .is_some_and(|k| k.eq_ignore_ascii_case(n))
            }) else {
                anyhow::bail!("You have no Linear account linked in {n}");
            };
            let mine = api.unlink_linear(&a.organization).await?;
            if json {
                return style::print_json(&mine);
            }
            println!(
                "{} fctl forgot your Linear account in {}",
                style::accent("✓"),
                a.organization_name
            );
        }
    }
    Ok(())
}

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
            let me = api.me().await?;
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
            let me = api.me().await?;
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

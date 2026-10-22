//! `fctl claude …`: connect a Claude subscription.

use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_domain::usage::{self as plan, Standing};
use fctl_proto::api::{ClaudeCredentialInfo, ClaudeLoginState, CredentialStatus, PlanLimit};

use std::time::Duration;

#[derive(Subcommand)]
pub enum ClaudeCmd {
    /// Connect your subscription: a long-lived token, or a full login (Remote Control).
    Connect {
        /// A token from `claude setup-token` (otherwise you are asked for one).
        #[arg(long, conflicts_with = "login")]
        token: Option<String>,
        /// Sign in with claude.ai through a host instead (enables Remote Control).
        #[arg(long)]
        login: bool,
    },
    /// Show connected subscriptions and how much of each plan's limits is used.
    Status,
}

/// What to say to someone with no Claude connection.
pub(crate) const NO_CONNECTION: &str =
    "You have no Claude connection yet: connect one with `fctl claude connect`.";

/// How a connection reads in lists: its account, else its label, else its kind.
fn name(c: &ClaudeCredentialInfo) -> String {
    c.account_email
        .clone()
        .or_else(|| c.label.clone())
        .unwrap_or_else(|| c.kind.as_str().into())
}

/// What sort of connection it is: `oauth · Remote Control · max`.
fn about(c: &ClaudeCredentialInfo) -> String {
    let mut about = format!(
        "{} · {}",
        c.kind.as_str(),
        if c.remote_control {
            "Remote Control"
        } else {
            "inference only"
        }
    );
    if let Some(t) = &c.subscription_type {
        about.push_str(&format!(" · {t}"));
    }
    about
}

/// One of your connections that sessions can run on: the one whose id is
/// `which`, else the working one whose account or label it is.
pub(crate) fn pick_connection<'a>(
    mine: &'a [ClaudeCredentialInfo],
    which: &str,
    now: i64,
) -> Result<&'a ClaudeCredentialInfo> {
    let c = match mine.iter().find(|c| c.id.as_str() == which) {
        Some(c) => c,
        None => {
            let named: Vec<&ClaudeCredentialInfo> = mine
                .iter()
                .filter(|c| {
                    [&c.account_email, &c.label]
                        .into_iter()
                        .flatten()
                        .any(|n| n.eq_ignore_ascii_case(which))
                })
                .collect();
            let working: Vec<&ClaudeCredentialInfo> = named
                .iter()
                .copied()
                .filter(|c| c.status == CredentialStatus::Active)
                .collect();
            match (working.as_slice(), named.first().copied()) {
                ([c], _) => *c,
                // It needs reconnecting, which is said below.
                ([], Some(c)) => c,
                ([], None) if mine.is_empty() => bail!(NO_CONNECTION),
                ([], None) => bail!(
                    "None of your Claude connections is {which}. `fctl claude status` lists them, with their ids."
                ),
                (several, _) => bail!(
                    "{} of your Claude connections are {which}; use one of their ids:\n{}",
                    several.len(),
                    several
                        .iter()
                        .map(|c| {
                            let added = style::ago_from(c.created_at, now);
                            format!("  {}  {} · added {added}", c.id, about(c))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }
        }
    };
    if c.status != CredentialStatus::Active {
        bail!(
            "{} ({}) needs reconnecting, so sessions can't run on it: `fctl claude connect` adds a new connection",
            name(c),
            c.id
        );
    }
    Ok(c)
}

/// A credential's plan limits, one line each, as sessions last reported them.
fn print_usage(usage: &[PlanLimit]) {
    let Some(reported) = usage.iter().map(|l| l.observed_at).max() else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let labels: Vec<String> = usage.iter().map(|l| plan::label(&l.window)).collect();
    let width = labels.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    for (l, label) in usage.iter().zip(labels) {
        let resets = l
            .resets_at
            .map(|t| format!("resets {}", style::until(t)))
            .unwrap_or_default();
        let used = l
            .utilization
            .map(|u| format!("{}% used", (u * 100.0).round()));
        let line = match plan::standing(l, now) {
            Standing::Reset => style::dim(&format!(
                "started over {}",
                l.resets_at.map(style::ago).unwrap_or_default()
            )),
            Standing::Reached => style::red(&join(&["limit reached".into(), resets])),
            Standing::Critical => style::red(&join(&[used.unwrap_or_default(), resets])),
            Standing::Close => style::amber(&join(&[used.unwrap_or_default(), resets])),
            Standing::Fine => join(&[used.unwrap_or_default(), resets]),
        };
        println!("    {label:<width$}  {line}");
    }
    println!(
        "    {}",
        style::dim(&format!("reported by a session {}", style::ago(reported)))
    );
}

fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn run(server: Option<&str>, cmd: ClaudeCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        ClaudeCmd::Status => {
            let me = api.me().await?;
            if json {
                return style::print_json(&me.claude);
            }
            if me.claude.is_empty() {
                println!("No Claude subscription connected. Run `fctl claude connect`.");
            }
            for c in &me.claude {
                println!(
                    "{} {} · {}{}{}",
                    if c.is_default {
                        style::accent("●")
                    } else {
                        style::dim("○")
                    },
                    name(c),
                    about(c),
                    if c.status == CredentialStatus::Active {
                        String::new()
                    } else {
                        format!(" · {}", style::red("needs reconnecting"))
                    },
                    style::dim(&format!(" · {}", c.id))
                );
                print_usage(&c.usage);
            }
            Ok(())
        }
        ClaudeCmd::Connect {
            token,
            login: false,
        } => {
            let token = match token {
                Some(t) => t,
                None => {
                    println!(
                        "Sessions run on your own Claude subscription (never on API billing)."
                    );
                    println!("Create a long-lived token on any machine with Claude Code:");
                    println!();
                    println!("    {}", style::accent("claude setup-token"));
                    println!();
                    crate::prompt::read_secret("Paste the token (input hidden): ")?
                }
            };
            let me = api.add_claude_token(&token, None).await?;
            if json {
                return style::print_json(&me);
            }
            println!(
                "{} Connected. New sessions run on this subscription.",
                style::accent("✓")
            );
            println!(
                "{}",
                style::dim(
                    "For Remote Control (follow sessions in the Claude apps), run `fctl claude connect --login`."
                )
            );
            Ok(())
        }
        ClaudeCmd::Connect { login: true, .. } => {
            let mut job = api.start_claude_login().await?;
            println!("Starting the official `claude auth login` in a sandbox on a host…");
            let mut prompted = false;
            for _ in 0..900 {
                match &job.state {
                    ClaudeLoginState::AwaitingCode { url } if !prompted => {
                        prompted = true;
                        // From the host the sign-in runs on, through the
                        // control plane: only Claude's page is opened.
                        if !fctl_domain::remote_control::is_login_url(url) {
                            bail!(
                                "The sign-in link isn't on claude.ai, so it wasn't opened: {}. Start again, or connect a long-lived token with `fctl claude connect`.",
                                style::terminal_safe(url)
                            );
                        }
                        println!();
                        println!("1. Open this link and approve: {}", style::accent(url));
                        let _ = crate::cmd::auth::open_browser(url);
                        println!("2. Copy the code claude.ai shows you.");
                        let code = crate::prompt::read_secret("3. Paste it here (input hidden): ")?;
                        job = api.submit_claude_code(&job.id.0, &code).await?;
                        continue;
                    }
                    ClaudeLoginState::Succeeded { .. } => {
                        if json {
                            return style::print_json(&job);
                        }
                        println!(
                            "{} Connected with a full Claude login: sessions can enable Remote Control.",
                            style::accent("✓")
                        );
                        return Ok(());
                    }
                    ClaudeLoginState::Failed { error } => bail!("The sign-in failed: {error}"),
                    _ => {}
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                job = api.claude_login(&job.id.0).await?;
            }
            bail!("The sign-in timed out")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;
    const DAY: i64 = 86_400_000;

    fn connection(
        id: &str,
        kind: &str,
        account: Option<&str>,
        status: &str,
        created_at: i64,
    ) -> Result<ClaudeCredentialInfo> {
        Ok(serde_json::from_value(serde_json::json!({
            "id": id,
            "kind": kind,
            "account_email": account,
            "remote_control": kind == "oauth",
            "status": status,
            "is_default": false,
            "created_at": created_at,
        }))?)
    }

    fn picked(mine: &[ClaudeCredentialInfo], which: &str) -> Result<String, String> {
        pick_connection(mine, which, NOW)
            .map(|c| c.id.to_string())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn connections_are_picked_by_id_account_or_label() -> Result<()> {
        let mut seat = connection("crd_01SEAT", "setup_token", None, "active", 0)?;
        seat.label = Some("Work seat".into());
        let mine = [
            connection("crd_01ADA", "oauth", Some("ada@example.com"), "active", 0)?,
            connection("crd_01TOKEN", "setup_token", None, "active", 0)?,
            seat,
        ];
        assert_eq!(picked(&mine, "crd_01TOKEN"), Ok("crd_01TOKEN".into()));
        assert_eq!(picked(&mine, "Ada@Example.com"), Ok("crd_01ADA".into()));
        assert_eq!(picked(&mine, "work seat"), Ok("crd_01SEAT".into()));
        assert_eq!(
            picked(&mine, "grace@example.com"),
            Err("None of your Claude connections is grace@example.com. `fctl claude status` lists them, with their ids.".into())
        );
        assert_eq!(picked(&[], "ada@example.com"), Err(NO_CONNECTION.into()));
        Ok(())
    }

    #[test]
    fn an_account_with_several_connections_takes_the_working_one() -> Result<()> {
        let mine = [
            connection("crd_01OLD", "oauth", Some("ada@example.com"), "invalid", 0)?,
            connection("crd_01NEW", "oauth", Some("ada@example.com"), "active", 0)?,
        ];
        assert_eq!(picked(&mine, "ada@example.com"), Ok("crd_01NEW".into()));
        // Named by its id, the broken one says why it can't be used.
        assert_eq!(
            picked(&mine, "crd_01OLD"),
            Err("ada@example.com (crd_01OLD) needs reconnecting, so sessions can't run on it: `fctl claude connect` adds a new connection".into())
        );
        assert_eq!(
            picked(&mine[..1], "ada@example.com"),
            picked(&mine, "crd_01OLD")
        );
        let both = [
            connection(
                "crd_01LOGIN",
                "oauth",
                Some("ada@example.com"),
                "active",
                NOW - 3 * DAY,
            )?,
            connection(
                "crd_01TOKEN",
                "setup_token",
                Some("ada@example.com"),
                "active",
                NOW - 5 * 60_000,
            )?,
        ];
        assert_eq!(
            picked(&both, "ada@example.com"),
            Err(
                "2 of your Claude connections are ada@example.com; use one of their ids:\n  \
                 crd_01LOGIN  oauth · Remote Control · added 3d ago\n  \
                 crd_01TOKEN  setup_token · inference only · added 5m ago"
                    .into()
            )
        );
        Ok(())
    }
}

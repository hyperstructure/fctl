//! `fctl claude …`: connect a Claude subscription.

use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_domain::usage::{self as plan, Standing};
use fctl_proto::api::{ClaudeLoginState, PlanLimit};
use std::io::{BufRead, Write};
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

fn read_secret(prompt: &str) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    if tty {
        let _ = std::process::Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let r = std::io::stdin().lock().read_line(&mut line);
    if tty {
        let _ = std::process::Command::new("stty").arg("echo").status();
        println!();
    }
    r?;
    Ok(line.trim().to_owned())
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
            Standing::Reached => style::amber(&join(&["limit reached".into(), resets])),
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
            for c in me.claude {
                println!(
                    "{} {} · {} · {}{}",
                    if c.is_default {
                        style::accent("●")
                    } else {
                        style::dim("○")
                    },
                    c.account_email
                        .or(c.label)
                        .unwrap_or_else(|| c.kind.as_str().into()),
                    c.kind.as_str(),
                    if c.remote_control {
                        "Remote Control"
                    } else {
                        "inference only"
                    },
                    c.subscription_type
                        .map(|t| format!(" · {t}"))
                        .unwrap_or_default()
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
                    read_secret("Paste the token (input hidden): ")?
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
                        println!();
                        println!("1. Open this link and approve: {}", style::accent(url));
                        let _ = crate::cmd::auth::open_browser(url);
                        println!("2. Copy the code claude.ai shows you.");
                        let code = read_secret("3. Paste it here (input hidden): ")?;
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

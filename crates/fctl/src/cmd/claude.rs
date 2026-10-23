//! `fctl claude …`: connect a Claude subscription.

use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_domain::usage::{self as plan, Standing};
use fctl_proto::api::{
    ClaudeCredentialInfo, ClaudeLoginState, CredentialStatus, PlanLimit, Runner, RunnerStatus,
};

use std::time::Duration;

#[derive(Subcommand)]
pub enum ClaudeCmd {
    /// Connect your subscription: a long-lived token, a full login (Remote
    /// Control), or the token you keep on your runners.
    Connect {
        /// A token from `claude setup-token` (otherwise you are asked for one).
        #[arg(long, conflicts_with_all = ["login", "on_runners"])]
        token: Option<String>,
        /// Sign in with claude.ai through a host instead (enables Remote Control).
        #[arg(long, conflicts_with = "on_runners")]
        login: bool,
        /// Run your sessions on the token you keep on your runners (`fctl
        /// host claude set-token` there), which you never give to fctl. Your
        /// sessions then run only on those runners.
        #[arg(long)]
        on_runners: bool,
    },
    /// Show connected subscriptions and how much of each plan's limits is used.
    Status,
}

/// What names the connection you keep on your runners, wherever a
/// connection is asked for.
pub(crate) const ON_RUNNERS: &str = "on-runners";

/// What to say to someone with no Claude connection.
pub(crate) const NO_CONNECTION: &str =
    "You have no Claude connection yet: connect one with `fctl claude connect`.";

/// How a connection reads in lists: its account, else its label, else its kind.
fn name(c: &ClaudeCredentialInfo) -> String {
    c.account_email
        .clone()
        .or_else(|| c.label.clone())
        .unwrap_or_else(|| {
            if c.on_runners {
                "On your runners".into()
            } else {
                c.kind.as_str().into()
            }
        })
}

/// What sort of connection it is: `oauth · Remote Control · max`, or
/// `kept on your runners · inference only`.
fn about(c: &ClaudeCredentialInfo) -> String {
    let mut about = format!(
        "{} · {}",
        if c.on_runners {
            "kept on your runners"
        } else {
            c.kind.as_str()
        },
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
/// `which`, the one you keep on your runners for `on-runners`, else the
/// working one whose account or label it is.
pub(crate) fn pick_connection<'a>(
    mine: &'a [ClaudeCredentialInfo],
    which: &str,
    now: i64,
) -> Result<&'a ClaudeCredentialInfo> {
    let found = mine.iter().find(|c| c.id.as_str() == which).or_else(|| {
        (which == ON_RUNNERS)
            .then(|| mine.iter().find(|c| c.on_runners))
            .flatten()
    });
    if which == ON_RUNNERS && found.is_none() {
        bail!(
            "You keep no Claude token on your runners yet: run `fctl claude connect --on-runners`, and `fctl host claude set-token` on your runners"
        );
    }
    let c = match found {
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

/// Your runners and whether each keeps your Claude token, for a connection
/// whose token is kept on them: `kept on studio-mac (online); none on
/// build-box`.
fn runners_line(runners: &[Runner]) -> String {
    let mine: Vec<&Runner> = runners.iter().filter(|r| r.yours).collect();
    if mine.is_empty() {
        return style::amber(
            "You have no runners yet, so sessions on it wait: `fctl host install` makes this machine one.",
        );
    }
    let state = |r: &Runner| match r.status {
        RunnerStatus::Online => "online",
        RunnerStatus::Offline => "offline",
        RunnerStatus::Disabled => "disabled",
    };
    let keeping: Vec<String> = mine
        .iter()
        .filter(|r| r.claude_token.is_some_and(|t| t.kept()))
        .map(|r| format!("{} ({})", r.name, state(r)))
        .collect();
    let without: Vec<String> = mine
        .iter()
        .filter(|r| !r.claude_token.is_some_and(|t| t.kept()))
        .map(|r| r.name.clone())
        .collect();
    if keeping.is_empty() {
        return style::amber(
            "None of your runners keeps it yet, so sessions on it wait. On a runner, run `claude setup-token`, then `fctl host claude set-token`: the runner tells fctl within 15 seconds.",
        );
    }
    let mut line = format!("kept on {}", keeping.join(", "));
    if !without.is_empty() {
        line.push_str(&format!("; not on {}", without.join(", ")));
    }
    line
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
            let me = super::auth::account(&api).await?;
            if json {
                return style::print_json(&me.claude);
            }
            if me.claude.is_empty() {
                println!("No Claude subscription connected. Run `fctl claude connect`.");
            }
            // Which runners keep the token of a connection kept on them.
            let runners = if me.claude.iter().any(|c| c.on_runners) {
                api.runners().await.map(|l| l.runners).ok()
            } else {
                None
            };
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
                if c.on_runners
                    && let Some(runners) = &runners
                {
                    println!("    {}", runners_line(runners));
                }
                print_usage(&c.usage);
            }
            Ok(())
        }
        ClaudeCmd::Connect {
            on_runners: true, ..
        } => {
            let me = api.add_claude_on_runners(None).await?;
            if json {
                return style::print_json(&me);
            }
            println!(
                "{} New sessions run on the Claude token you keep on your runners: you never give it to fctl.",
                style::accent("✓")
            );
            match api.runners().await {
                Ok(list) => println!("  {}", runners_line(&list.runners)),
                Err(e) => println!(
                    "  {}",
                    style::dim(&format!("Could not list your runners: {e}"))
                ),
            }
            println!(
                "  {}",
                style::dim(
                    "Your sessions on it run only on your runners that keep it, and wait while none of them is online."
                )
            );
            Ok(())
        }
        ClaudeCmd::Connect {
            token,
            login: false,
            ..
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
    fn the_connection_kept_on_runners_is_named_on_runners() -> Result<()> {
        let mut kept = connection("crd_01KEPT", "setup_token", None, "active", 0)?;
        kept.on_runners = true;
        let token = connection("crd_01TOKEN", "setup_token", None, "active", 0)?;
        assert_eq!(name(&kept), "On your runners");
        assert_eq!(about(&kept), "kept on your runners · inference only");
        let mine = [token.clone(), kept];
        assert_eq!(picked(&mine, "on-runners"), Ok("crd_01KEPT".into()));
        assert!(
            picked(&[token], "on-runners")
                .is_err_and(|e| e.contains("`fctl claude connect --on-runners`"))
        );
        Ok(())
    }

    fn runner(name: &str, yours: bool, status: &str, saved_at: Option<i64>) -> Result<Runner> {
        Ok(serde_json::from_value(serde_json::json!({
            "id": format!("rnr_{name}"), "name": name, "status": status, "drivers": [],
            "capacity": 4, "active_sessions": 0, "labels": [], "created_at": 0,
            "yours": yours,
            "claude_token": saved_at.map(|t| serde_json::json!({ "saved_at": t })),
        }))?)
    }

    #[test]
    fn says_which_of_your_runners_keep_your_token() -> Result<()> {
        let mini = runner("mini", true, "online", Some(1))?;
        let boxed = runner("box", true, "offline", None)?;
        let theirs = runner("theirs", false, "online", None)?;
        assert_eq!(
            runners_line(&[mini.clone(), boxed.clone(), theirs.clone()]),
            "kept on mini (online); not on box"
        );
        assert!(runners_line(&[boxed]).contains("None of your runners keeps it yet"));
        assert!(runners_line(&[theirs]).contains("You have no runners yet"));
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

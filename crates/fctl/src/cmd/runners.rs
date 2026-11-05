//! `fctl runners …`: the runners you see; approving and rejecting the ones
//! that wait in workspaces you are an admin of; managing runners as
//! machines (yours, your workspaces' own as their admin, every one as an
//! instance admin); and workspaces' enrollment tokens, with which machines
//! enroll themselves as their runners.

use crate::cmd::admin::{drain_line, find_runner, low_disk_line, still_running};
use crate::config;
use crate::style;
use anyhow::{Result, bail};
use clap::Subcommand;
use fctl_client::Api;
use fctl_domain::enrollment;
use fctl_proto::api::{CreateEnrollmentTokenRequest, EnrollmentToken, Runner, UpdateRunnerRequest};
use std::io::IsTerminal;

#[derive(Subcommand)]
pub enum RunnersCmd {
    /// The runners you see: yours, your workspaces', and, for instance
    /// admins, every one (the default).
    #[command(visible_alias = "ls")]
    List {
        /// Only this workspace's (its slug or id).
        #[arg(short, long)]
        workspace: Option<String>,
        /// Only the ones that wait for an admin to approve them.
        #[arg(long)]
        pending: bool,
    },
    /// Approve a runner that enrolled in a workspace you are an admin of
    /// and waits for approval: it runs everyone's sessions there from now
    /// on, and its host takes them at once.
    Approve {
        /// The runner's name or id.
        runner: String,
    },
    /// Reject a runner that waits for approval: it is removed, and its
    /// token stops working.
    Reject {
        /// The runner's name or id.
        runner: String,
    },
    /// Rename a runner.
    Rename {
        /// The runner's name or id.
        runner: String,
        /// Its new name.
        name: String,
    },
    /// Take no new sessions; the ones it has carry on, and a message
    /// resumes them there.
    Drain {
        /// The runner's name or id.
        runner: String,
    },
    /// Take new sessions again.
    Undrain {
        /// The runner's name or id.
        runner: String,
    },
    /// Disable a runner: it disconnects, and its token works again only
    /// once it is enabled.
    Disable {
        /// The runner's name or id.
        runner: String,
    },
    /// Enable a disabled runner.
    Enable {
        /// The runner's name or id.
        runner: String,
    },
    /// Remove a runner: its token stops working, and the sessions it runs
    /// stop and resume on another runner. It asks first in a terminal.
    #[command(visible_alias = "rm")]
    Remove {
        /// The runner's name or id.
        runner: String,
        /// Don't ask.
        #[arg(long, short)]
        yes: bool,
    },
    /// Replace a runner's token (yours, or one of a workspace you are an
    /// admin of): the old one stops working at once. Prints the new one,
    /// which `fctl host install --token` takes on the machine.
    Token {
        /// The runner's name or id.
        runner: String,
    },
    /// A workspace's enrollment tokens: machines enroll themselves as its
    /// runners with one (`fctl host install --enroll`), without anyone
    /// signing in on them, as autoscaling creates them (its admins).
    #[command(
        name = "enrollment-tokens",
        visible_alias = "enroll-tokens",
        after_help = "Examples:
  fctl runners enrollment-tokens -w acme
  fctl runners enrollment-tokens create \"Build fleet\" -w acme --expires 90d --max-runners 20 --remove-offline-after 5m
  fctl runners enrollment-tokens create \"Contractors\" -w acme --approval --expires 7d
  fctl runners enrollment-tokens revoke \"Build fleet\" -w acme

On each machine, without signing in:
  FCTL_SERVER=https://app.fctl.sh FCTL_ENROLL_TOKEN=fctl_et_… fctl host install"
    )]
    EnrollmentTokens {
        #[command(subcommand)]
        cmd: Option<EnrollmentTokensCmd>,
        /// The workspace (its slug or id; default from `fctl workspaces
        /// use`, else your only one).
        #[arg(short, long, global = true)]
        workspace: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum EnrollmentTokensCmd {
    /// The workspace's enrollment tokens, newest first (the default).
    #[command(visible_alias = "ls")]
    List,
    /// Make an enrollment token. Prints its secret, once, on stdout.
    Create {
        /// What it is for, which the runners it enrolls show: "Build fleet".
        name: String,
        /// How long it works: 1h, 7d, 90d, 1y, or never.
        #[arg(long, default_value = "90d", value_parser = hours_arg)]
        expires: Hours,
        /// The most runners it enrolls that may be there at once (rejected
        /// and removed ones don't count). Default: no limit.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000))]
        max_runners: Option<u32>,
        /// The runners it enrolls wait for an admin of the workspace to
        /// approve them before they run anything.
        #[arg(long)]
        approval: bool,
        /// Remove the runners it enrolls once they have been offline this
        /// long, as machines autoscaling destroys are: 1m, 5m, 15m, 1h, 1d,
        /// 7d (time the control plane is down doesn't count). Default: they
        /// stay until someone removes them.
        #[arg(long, value_name = "DURATION", value_parser = minutes_arg)]
        remove_offline_after: Option<Minutes>,
    },
    /// Revoke an enrollment token: it enrolls no more runners. The ones it
    /// enrolled stay, and the ones that wait for approval still wait.
    #[command(visible_alias = "rm")]
    Revoke {
        /// Its id (enr_…) or name.
        token: String,
    },
}

/// A duration in hours, or none (`never`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hours(Option<u32>);

/// `90d` and the like; `never`.
fn hours_arg(s: &str) -> std::result::Result<Hours, String> {
    enrollment::parse_hours(s).map(Hours)
}

/// A duration in minutes, or none (`never`, `keep`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Minutes(Option<u32>);

/// `5m`, `1h` and the like; `keep`.
fn minutes_arg(s: &str) -> std::result::Result<Minutes, String> {
    enrollment::parse_minutes(s).map(Minutes)
}

pub async fn run(server: Option<&str>, cmd: RunnersCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        RunnersCmd::List { workspace, pending } => {
            let mut runners = match &workspace {
                Some(w) => {
                    let w = crate::cmd::workspaces::resolve(&api, Some(w.clone())).await?;
                    api.workspace_runners(w.id.as_str()).await?.runners
                }
                None => api.runners().await?.runners,
            };
            if pending {
                runners.retain(|r| r.pending_until.is_some());
            }
            if json {
                return style::print_json(&fctl_proto::api::RunnerList { runners });
            }
            if runners.is_empty() {
                println!(
                    "{}",
                    style::dim(if pending {
                        "No runner waits for approval."
                    } else {
                        "No runners yet. `fctl host install` registers this machine; a workspace's admins make enrollment tokens for many (`fctl runners enrollment-tokens create`)."
                    })
                );
            }
            for r in &runners {
                print_runner(r);
            }
            if runners.iter().any(|r| r.pending_until.is_some()) {
                println!(
                    "{}",
                    style::dim(
                        "`fctl runners approve <name or id>` lets one run its workspace's sessions; `fctl runners reject` removes it."
                    )
                );
            }
            Ok(())
        }
        RunnersCmd::Approve { runner } => {
            let r = find_runner(&api, &runner).await?;
            if r.pending_until.is_none() && r.workspace.is_some() {
                if json {
                    return style::print_json(&r);
                }
                println!(
                    "{} {} was approved already: it runs everyone's sessions in {}.",
                    style::accent("✓"),
                    style::bold(&r.name),
                    workspace_name(&r)
                );
                return Ok(());
            }
            let r = api.approve_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Approved {}: it runs everyone's sessions in {} from now on.",
                style::accent("✓"),
                style::bold(&r.name),
                workspace_name(&r)
            );
            Ok(())
        }
        RunnersCmd::Reject { runner } => {
            let r = find_runner(&api, &runner).await?;
            let ok = api.reject_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&ok);
            }
            println!(
                "{} Rejected {}: it is removed, and its token no longer works.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Rename { runner, name } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.name = Some(name.clone())).await?;
            if json {
                return style::print_json(&r);
            }
            println!("{} Renamed it {}", style::accent("✓"), style::bold(&r.name));
            Ok(())
        }
        RunnersCmd::Drain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.draining = Some(true)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Draining {}: no new sessions start on it.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            println!("{}", style::dim(&still_running(r.active_sessions, "there")));
            Ok(())
        }
        RunnersCmd::Undrain { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.draining = Some(false)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} {} takes new sessions again.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Disable { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.enabled = Some(false)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Disabled {}: it is disconnected, and takes no sessions until `fctl runners enable {}`.",
                style::accent("✓"),
                style::bold(&r.name),
                r.id
            );
            Ok(())
        }
        RunnersCmd::Enable { runner } => {
            let r = find_runner(&api, &runner).await?;
            let r = update(&api, &r, |u| u.enabled = Some(true)).await?;
            if json {
                return style::print_json(&r);
            }
            println!(
                "{} Enabled {}: its host connects again with its token.",
                style::accent("✓"),
                style::bold(&r.name)
            );
            Ok(())
        }
        RunnersCmd::Remove { runner, yes } => {
            let r = find_runner(&api, &runner).await?;
            if !yes && !json && std::io::stdin().is_terminal() {
                let sessions = match r.active_sessions {
                    0 => String::new(),
                    n => format!(
                        " The {} it runs stop{}, and resume on another runner.",
                        style::count(n, "session", "sessions"),
                        if n == 1 { "s" } else { "" }
                    ),
                };
                println!(
                    "Remove {}? Its token stops working.{sessions}",
                    style::bold(&r.name)
                );
                if !confirm("Remove it? [y/N] ")? {
                    println!("Nothing changed.");
                    return Ok(());
                }
            }
            let ok = api.delete_runner(r.id.as_str()).await?;
            if json {
                return style::print_json(&ok);
            }
            println!("{} Removed {}", style::accent("✓"), style::bold(&r.name));
            Ok(())
        }
        RunnersCmd::Token { runner } => {
            let r = find_runner(&api, &runner).await?;
            let reg = api.replace_runner_token(r.id.as_str()).await?;
            if json {
                return style::print_json(&reg);
            }
            println!("{}", reg.token);
            eprintln!(
                "{}",
                style::dim(&format!(
                    "Shown once. {}'s old token no longer works: on the machine, `fctl host install --token <this>` puts the new one in place.",
                    reg.runner.name
                ))
            );
            Ok(())
        }
        RunnersCmd::EnrollmentTokens { cmd, workspace } => {
            let w = crate::cmd::workspaces::resolve(&api, workspace).await?;
            enrollment_tokens(&api, &w, cmd.unwrap_or(EnrollmentTokensCmd::List), json).await
        }
    }
}

async fn enrollment_tokens(
    api: &Api,
    w: &fctl_proto::api::Workspace,
    cmd: EnrollmentTokensCmd,
    json: bool,
) -> Result<()> {
    let id = w.id.as_str();
    match cmd {
        EnrollmentTokensCmd::List => {
            let list = api.enrollment_tokens(id).await?;
            if json {
                return style::print_json(&list);
            }
            if list.tokens.is_empty() {
                println!(
                    "{}",
                    style::dim(&format!(
                        "{} has no enrollment tokens yet. `fctl runners enrollment-tokens create NAME -w {}` makes one.",
                        w.name, w.slug
                    ))
                );
            }
            for t in &list.tokens {
                print_token(t);
            }
            Ok(())
        }
        EnrollmentTokensCmd::Create {
            name,
            expires,
            max_runners,
            approval,
            remove_offline_after,
        } => {
            let created = api
                .create_enrollment_token(
                    id,
                    &CreateEnrollmentTokenRequest {
                        name,
                        expires_in_hours: expires.0,
                        max_runners,
                        approval: Some(approval),
                        remove_offline_after_minutes: remove_offline_after.and_then(|m| m.0),
                    },
                )
                .await?;
            if json {
                return style::print_json(&created);
            }
            println!("{}", created.secret);
            let t = &created.token;
            eprintln!(
                "{}",
                style::dim(&format!(
                    "{} {}",
                    enrollment::describe(t.approval, t.max_runners, t.remove_offline_after_minutes),
                    expiry(t.expires_at)
                ))
            );
            eprintln!(
                "{}",
                style::dim(&format!(
                    "Shown once. On each machine: FCTL_SERVER={} FCTL_ENROLL_TOKEN=<this> fctl host install",
                    api.base()
                ))
            );
            Ok(())
        }
        EnrollmentTokensCmd::Revoke { token } => {
            let list = api.enrollment_tokens(id).await?;
            let wanted = token.trim();
            let matching: Vec<&EnrollmentToken> = list
                .tokens
                .iter()
                .filter(|t| t.id.as_str() == wanted || t.name == wanted)
                .collect();
            let live: Vec<&&EnrollmentToken> =
                matching.iter().filter(|t| t.revoked_at.is_none()).collect();
            let t = match (matching.as_slice(), live.as_slice()) {
                (_, [t]) => **t,
                ([t], []) => *t,
                ([], _) => bail!(
                    "{} has no enrollment token {wanted}. `fctl runners enrollment-tokens -w {}` lists them.",
                    w.name,
                    w.slug
                ),
                _ => bail!(
                    "{} enrollment tokens are called {wanted}: revoke one by its id (`fctl runners enrollment-tokens -w {}` lists them)",
                    live.len(),
                    w.slug
                ),
            };
            let t = api.revoke_enrollment_token(id, t.id.as_str()).await?;
            if json {
                return style::print_json(&t);
            }
            println!(
                "{} Revoked {}: it enrolls no more runners. The {} it enrolled stay.",
                style::accent("✓"),
                style::bold(&t.name),
                style::count(t.runners, "runner", "runners")
            );
            Ok(())
        }
    }
}

/// Changes a runner with one PATCH.
async fn update(
    api: &Api,
    r: &Runner,
    change: impl FnOnce(&mut UpdateRunnerRequest),
) -> Result<Runner> {
    let mut req = UpdateRunnerRequest::default();
    change(&mut req);
    Ok(api.update_runner(r.id.as_str(), &req).await?)
}

fn workspace_name(r: &Runner) -> String {
    r.workspace
        .as_ref()
        .map_or_else(|| "its workspace".to_owned(), |w| w.name.clone())
}

/// `expires in 89d 23h.`, `expired 2d ago.`, `never expires.`
fn expiry(expires_at: Option<i64>) -> String {
    match expires_at {
        None => "It never expires.".to_owned(),
        Some(at) if at <= style::now_ms() => format!("It expired {}.", style::ago(at)),
        Some(at) => format!("It expires {}.", style::until(at)),
    }
}

/// One runner, with whose it is and what it does now.
fn print_runner(r: &Runner) {
    let whose = match (&r.workspace, &r.created_by) {
        (Some(w), _) => format!("{}'s", w.name),
        (None, Some(_)) if r.yours => "yours".to_owned(),
        (None, Some(u)) => format!("{}'s", u.name),
        (None, None) => "nobody's".to_owned(),
    };
    println!(
        "{:<24} {:<8} {:<8} {}/{} sessions  {}",
        style::bold(&r.name),
        style::runner_status(r),
        r.driver.as_deref().unwrap_or_default(),
        r.active_sessions,
        r.capacity,
        style::dim(&format!("{whose} · {}", r.id))
    );
    if let Some(until) = r.pending_until {
        let from = r
            .enrollment
            .as_ref()
            .map(|e| {
                let token = e
                    .token_name
                    .as_deref()
                    .map(|t| format!(" with “{t}”"))
                    .unwrap_or_default();
                let address = match (&e.address, &e.location) {
                    (Some(a), Some(l)) => format!(" from {a} ({l})"),
                    (Some(a), None) => format!(" from {a}"),
                    _ => String::new(),
                };
                format!(" since it enrolled{token}{address}")
            })
            .unwrap_or_default();
        println!(
            "  {}",
            style::amber(&format!(
                "waits for approval{from}; removed {} unless approved",
                style::until(until)
            ))
        );
    } else if let Some(t) = r.enrollment.as_ref().and_then(|e| e.token_name.as_deref()) {
        println!("  {}", style::dim(&format!("enrolled with “{t}”")));
    }
    let facts: Vec<String> = [
        r.hostname.clone(),
        r.os.as_deref()
            .zip(r.arch.as_deref())
            .map(|(os, arch)| format!("{os}/{arch}")),
        r.version.as_ref().map(|v| format!("fctl {v}")),
        Some(match r.last_seen_at {
            Some(t) => format!("seen {}", style::ago(t)),
            None => "never connected".to_owned(),
        }),
    ]
    .into_iter()
    .flatten()
    .collect();
    println!("  {}", style::dim(&facts.join(" · ")));
    if let Some(d) = drain_line(r) {
        println!("  {}", style::dim(&d));
    }
    if let Some(d) = low_disk_line(r) {
        println!("  {}", style::amber(&d));
    }
    if let Some(h) = r.remove_offline_after_minutes {
        println!(
            "  {}",
            style::dim(&format!(
                "removed once offline for {}",
                enrollment::minutes(h)
            ))
        );
    }
}

/// One enrollment token, with what it does and how much it was used.
fn print_token(t: &EnrollmentToken) {
    let now = style::now_ms();
    let state = match (t.revoked_at, t.expires_at) {
        (Some(_), _) => style::red("revoked"),
        (None, Some(at)) if at <= now => style::dim("expired"),
        _ => style::accent("active"),
    };
    let runners = match t.max_runners {
        Some(max) => format!("{} of {max} runners", t.runners),
        None => style::count(t.runners, "runner", "runners"),
    };
    let waiting = match t.pending {
        0 => String::new(),
        n => format!(", {n} waiting for approval"),
    };
    println!(
        "{:<24} {:<8} {runners}{waiting}  {}",
        style::bold(&t.name),
        state,
        style::dim(t.id.as_str())
    );
    let made = match &t.created_by {
        Some(u) => format!("Made by {} {}", u.name, style::ago(t.created_at)),
        None => format!("Made {}", style::ago(t.created_at)),
    };
    let used = t
        .last_used_at
        .map(|at| format!("; last enrolled a runner {}", style::ago(at)))
        .unwrap_or_default();
    println!(
        "  {}",
        style::dim(&format!(
            "{} {} {made}{used}.",
            enrollment::describe(t.approval, t.max_runners, t.remove_offline_after_minutes),
            expiry(t.expires_at)
        ))
    );
}

/// Asks a yes-or-no question on the terminal.
fn confirm(question: &str) -> Result<bool> {
    use std::io::{BufRead, Write};
    print!("{question}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_parse_for_enrollment_tokens() {
        assert_eq!(hours_arg("90d"), Ok(Hours(Some(2160))));
        assert_eq!(hours_arg("never"), Ok(Hours(None)));
        assert!(hours_arg("soon").is_err());
        assert_eq!(minutes_arg("5m"), Ok(Minutes(Some(5))));
        assert_eq!(minutes_arg("1h"), Ok(Minutes(Some(60))));
        assert_eq!(minutes_arg("keep"), Ok(Minutes(None)));
        assert!(minutes_arg("5").is_err());
    }

    #[test]
    fn expiry_says_when() {
        assert_eq!(expiry(None), "It never expires.");
        assert!(expiry(Some(style::now_ms() - 3 * 86_400_000)).starts_with("It expired 3d ago"));
        assert!(
            expiry(Some(style::now_ms() + 3 * 86_400_000 + 60_000)).starts_with("It expires in 3d")
        );
    }
}

//! `fctl admin …`: access, runners and the instance's settings.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand};
use fctl_domain::size;
use fctl_proto::api::{
    InstanceSettings, Runner, RunnerSession, UpdateInstanceSettingsRequest, UpdateRunnerRequest,
};
use fctl_proto::session::InstanceRole;

#[derive(Subcommand)]
pub enum AdminCmd {
    /// List users.
    Users,
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
    /// Show the instance's settings.
    Settings,
    /// Change the instance's settings.
    #[command(
        group(ArgGroup::new("change").required(true).multiple(true).args(["publish_screenshots"])),
        after_help = "Examples:\n  fctl admin set --publish-screenshots on"
    )]
    Set {
        /// Let projects publish their agents' screenshots on this instance,
        /// at links anyone who has one can open until they expire. Each
        /// project turns it on too (`fctl projects set --publish-screenshots
        /// on`). Off takes every published screenshot offline.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        publish_screenshots: Option<bool>,
    },
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
}

pub async fn run(server: Option<&str>, cmd: AdminCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    match cmd {
        AdminCmd::Users => {
            let list = api.users().await?;
            if json {
                return style::print_json(&list);
            }
            for u in list.users {
                println!(
                    "{:<28} {:<32} {:<7} {}",
                    style::bold(&u.name),
                    u.email,
                    u.role.as_str(),
                    if u.status == fctl_proto::api::UserStatus::Active {
                        style::accent("active")
                    } else {
                        style::red("disabled")
                    }
                );
            }
        }
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
            if let Some(inv) = created.invite
                && let Some(url) = inv.url
            {
                println!(
                    "Invitation link (valid 14 days{}):\n  {url}",
                    if inv.emailed { ", also emailed" } else { "" }
                );
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
            if let Some(d) = out
                .get("disabled")
                .and_then(|v| v.as_array())
                .filter(|a| !a.is_empty())
            {
                println!(
                    "Disabled: {}",
                    d.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        AdminCmd::Runners => {
            let list = api.runners().await?;
            if json {
                return style::print_json(&list);
            }
            for r in list.runners {
                let state = match (r.status, &r.drain) {
                    (fctl_proto::api::RunnerStatus::Disabled, _) => style::red("disabled"),
                    (_, Some(_)) if r.active_sessions > 0 => style::amber("draining"),
                    (_, Some(_)) => style::amber("drained"),
                    (fctl_proto::api::RunnerStatus::Online, None) => style::accent("online"),
                    (fctl_proto::api::RunnerStatus::Offline, None) => style::dim("offline"),
                };
                println!(
                    "{:<24} {:<8} {:<8} {}/{} sessions  {}",
                    style::bold(&r.name),
                    state,
                    r.driver.as_deref().unwrap_or_default(),
                    r.active_sessions,
                    r.capacity,
                    style::dim(r.version.as_deref().unwrap_or_default())
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
        AdminCmd::Settings => {
            let settings = api.instance_settings().await?;
            if json {
                return style::print_json(&settings);
            }
            print_settings(&settings);
        }
        AdminCmd::Set {
            publish_screenshots,
        } => {
            let settings = api
                .update_instance_settings(&UpdateInstanceSettingsRequest {
                    publish_screenshots,
                })
                .await?;
            if json {
                return style::print_json(&settings);
            }
            print_settings(&settings);
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
async fn find_runner(api: &fctl_client::Api, which: &str) -> Result<Runner> {
    let all = api.runners().await?.runners;
    if let Some(r) = all.iter().find(|r| r.id.as_str() == which) {
        return Ok(r.clone());
    }
    let named: Vec<&Runner> = all.iter().filter(|r| r.name == which).collect();
    match named.as_slice() {
        [r] => Ok((*r).clone()),
        [] => bail!("No runner is called {which}. `fctl admin runners` lists them."),
        _ => bail!(
            "{} runners are called {which}; use one of their ids: {}",
            named.len(),
            named
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
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

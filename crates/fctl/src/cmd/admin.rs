//! `fctl admin …`: access, runners and the instance's settings.

use crate::config;
use crate::style;
use anyhow::{Context, Result};
use clap::builder::BoolishValueParser;
use clap::{ArgGroup, Subcommand};
use fctl_domain::size;
use fctl_proto::api::{InstanceSettings, UpdateInstanceSettingsRequest};
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
                let state = match r.status {
                    fctl_proto::api::RunnerStatus::Online => style::accent("online"),
                    fctl_proto::api::RunnerStatus::Offline => style::dim("offline"),
                    fctl_proto::api::RunnerStatus::Disabled => style::red("disabled"),
                };
                println!(
                    "{:<24} {:<8} {:<8} {}/{} sessions  {}",
                    style::bold(&r.name),
                    state,
                    r.driver.unwrap_or_default(),
                    r.active_sessions,
                    r.capacity,
                    style::dim(&r.version.unwrap_or_default())
                );
                if let Some(l) = &r.sandbox {
                    println!(
                        "  {}",
                        style::dim(&format!("sandboxes: {}", size::limits(l)))
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
    }
    Ok(())
}

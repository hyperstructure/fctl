//! `fctl admin …`: access and runners.

use crate::config;
use crate::style;
use anyhow::{Context, Result};
use clap::Subcommand;
use fctl_proto::session::InstanceRole;

#[derive(Subcommand)]
pub enum AdminCmd {
    /// List users.
    Users,
    /// Show who may sign up.
    Allowlist,
    /// Allow an email (invites them) or a domain (`*@example.com`).
    Allow {
        pattern: String,
        /// Make them an admin.
        #[arg(long)]
        admin: bool,
    },
    /// Remove an allowlist entry (by its pattern or id); its users lose access.
    Disallow { pattern: String },
    /// List runners.
    Runners,
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
            }
        }
    }
    Ok(())
}

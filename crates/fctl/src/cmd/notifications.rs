//! `fctl notifications …`: what notifies you, and the browsers that show it.
//! Following sessions is `fctl follow` and `fctl unfollow` (sessions.rs).

use crate::config;
use crate::style;
use anyhow::Result;
use clap::Subcommand;
use clap::builder::BoolishValueParser;
use fctl_proto::api::{Notifications, UpdateNotificationsRequest};

#[derive(Subcommand)]
pub enum NotificationsCmd {
    /// Show what notifies you and the browsers your notifications go to.
    Status,
    /// Choose what notifies you (on or off).
    #[command(
        after_help = "Examples:\n  fctl notifications set --waiting off\n  fctl notifications set --failed on --follow-started off"
    )]
    Set {
        /// A session you follow finished its turn and waits for your reply.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        waiting: Option<bool>,
        /// A session you follow paused on a Claude usage limit.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        paused: Option<bool>,
        /// A session you follow failed.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        failed: Option<bool>,
        /// Starting or resuming a session follows it.
        #[arg(long, value_name = "ON|OFF", value_parser = BoolishValueParser::new(), hide_possible_values = true)]
        follow_started: Option<bool>,
    },
    /// Send a test notification to your browsers.
    Test,
    /// Stop sending notifications to a browser (its id from `status`).
    Remove {
        /// The browser's id (psh_…).
        id: String,
    },
}

pub async fn run(server: Option<&str>, cmd: NotificationsCmd, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let shown = match cmd {
        NotificationsCmd::Status => api.notifications().await?,
        NotificationsCmd::Set {
            waiting,
            paused,
            failed,
            follow_started,
        } => {
            api.update_notifications(&UpdateNotificationsRequest {
                waiting,
                paused,
                failed,
                follow_started,
            })
            .await?
        }
        NotificationsCmd::Test => {
            let sent = api.test_notification().await?;
            if json {
                return style::print_json(&sent);
            }
            println!(
                "{} Sent to {} browser{}",
                style::accent("✓"),
                sent.sent,
                if sent.sent == 1 { "" } else { "s" }
            );
            if sent.failed > 0 {
                println!(
                    "{}",
                    style::amber(&format!(
                        "{} could not be reached; expired ones were removed",
                        sent.failed
                    ))
                );
            }
            return Ok(());
        }
        NotificationsCmd::Remove { id } => api.remove_push_device(&id).await?,
    };
    if json {
        return style::print_json(&shown);
    }
    print(&shown, api.base());
    Ok(())
}

fn print(n: &Notifications, base: &str) {
    let mark = |on: bool| {
        if on {
            style::accent("✓")
        } else {
            style::dim("✗")
        }
    };
    let s = &n.settings;
    println!("Notify me when a session I follow");
    println!("  {} waits for my reply", mark(s.waiting));
    println!("  {} pauses on a usage limit", mark(s.paused));
    println!("  {} fails", mark(s.failed));
    println!(
        "{} Starting or resuming a session follows it",
        mark(s.follow_started)
    );
    println!();
    if n.public_key.is_none() {
        println!(
            "{}",
            style::amber("This instance cannot send notifications: it has no FCTL_MASTER_KEY.")
        );
        return;
    }
    if n.devices.is_empty() {
        println!(
            "No browser gets your notifications yet. Turn them on in Settings: {base}/settings"
        );
        return;
    }
    println!("Browsers");
    for d in &n.devices {
        let last = d
            .last_sent_at
            .map(|t| format!(" · last notified {}", style::ago(t)))
            .unwrap_or_default();
        println!(
            "  {} {} {}",
            style::dim(d.id.as_str()),
            style::bold(d.name.as_deref().unwrap_or("Browser")),
            style::dim(&format!(
                "· {} · added {}{last}",
                d.service,
                style::ago(d.created_at)
            ))
        );
    }
}

//! `fctl resolve`: in the sandbox of a session whose agent may comment on
//! GitHub but not push, resolve review threads of its pull requests.
//! GitHub lets only tokens that can push resolve them, so fctl-agent asks
//! the control plane, which does it with a token of its own.

use crate::style;
use anyhow::{Context, Result, bail};
use clap::Args;
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall, ReviewThread};
use std::time::Duration;

#[derive(Args)]
pub struct ResolveArgs {
    /// Review threads, by id (PRRT_…).
    #[arg(required = true, value_name = "THREAD")]
    pub threads: Vec<String>,
    /// Open them again instead.
    #[arg(long)]
    pub unresolve: bool,
}

pub async fn resolve(args: ResolveArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl resolve resolves review threads in the sandbox of an fctl session whose agent reviews and comments. Elsewhere, resolve them on GitHub."
        );
    };
    let mut done: Vec<ReviewThread> = Vec::new();
    for thread in args.threads {
        let request = SessionRequest::Call {
            session_id: Some(SessionId(session.clone())),
            call: AgentCall::ResolveThread {
                thread: thread.clone(),
                unresolve: args.unresolve,
            },
        };
        // fctl-agent waits up to two minutes for the control plane.
        let reply = tokio::time::timeout(
            Duration::from_secs(180),
            super::sessions::ask_agent(&socket, &request),
        )
        .await
        .context("fctl-agent did not answer in time")??;
        let t = match reply {
            SessionReply::Answer {
                answer: AgentAnswer::Thread { thread },
            } => thread,
            SessionReply::Error { error } => bail!("{thread}: {}", error.message),
            _ => bail!("fctl-agent answered another request"),
        };
        if !json {
            println!(
                "{} {} {} on {}",
                style::accent("✓"),
                if t.resolved { "Resolved" } else { "Opened" },
                style::bold(&t.id),
                style::terminal_safe(&t.pull_request_url)
            );
        }
        done.push(t);
    }
    if json {
        return style::print_json(&done);
    }
    Ok(())
}

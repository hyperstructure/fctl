//! `fctl retro`: retrospectives. An agent records one in its session's
//! sandbox as it finishes a piece of work, through fctl-agent; people read
//! a project's or a session's, and delete them, through the API.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use fctl_domain::retrospectives::outcome_label;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall, RetrospectiveList};
use fctl_proto::session::{Retrospective, RetrospectiveOutcome};
use fctl_proto::{RetrospectiveId, SessionId};
use std::time::Duration;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RetroArgs {
    /// How the work ended: completed, partial (part of it is done) or
    /// blocked (it needs a person).
    #[arg(long, value_enum)]
    pub outcome: Option<Outcome>,
    /// What the work was.
    #[arg(long, value_name = "TEXT")]
    pub worked_on: Option<String>,
    /// The approach taken, and why.
    #[arg(long, value_name = "TEXT")]
    pub approach: Option<String>,
    /// What got in the way.
    #[arg(long, value_name = "TEXT")]
    pub challenges: Option<String>,
    /// What a future agent should know: what was learned, or would have
    /// helped to know at the start.
    #[arg(long, value_name = "TEXT")]
    pub insights: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<RetroCmd>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Outcome {
    Completed,
    Partial,
    Blocked,
}

impl From<Outcome> for RetrospectiveOutcome {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Completed => Self::Completed,
            Outcome::Partial => Self::Partial,
            Outcome::Blocked => Self::Blocked,
        }
    }
}

#[derive(Subcommand)]
pub enum RetroCmd {
    /// The retrospectives of a project, or of one session, newest first.
    #[command(visible_alias = "ls")]
    List {
        /// A session: its id (ses_…), its URL, or project#number.
        session: Option<String>,
        /// The project (slug or id); default from `fctl projects use`.
        #[arg(short, long)]
        project: Option<String>,
        /// Only those that ended this way.
        #[arg(long, value_enum)]
        outcome: Option<Outcome>,
        /// How many.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Delete a retrospective (people who manage its session).
    #[command(visible_alias = "rm")]
    Delete {
        /// The retrospective's id (rtr_…).
        id: String,
    },
}

pub async fn run(server: Option<&str>, args: RetroArgs, json: bool) -> Result<()> {
    match args.cmd {
        None => record(args, json).await,
        Some(RetroCmd::List {
            session,
            project,
            outcome,
            limit,
        }) => {
            let api = config::api(server)?;
            let list = match &session {
                Some(reference) => {
                    let s = super::sessions::resolve(&api, reference).await?;
                    let mut list = api.session_retrospectives(s.id.as_str()).await?;
                    if let Some(o) = outcome {
                        let o = RetrospectiveOutcome::from(o);
                        list.retrospectives.retain(|r| r.outcome == o);
                    }
                    list.retrospectives.truncate(limit as usize);
                    list
                }
                None => {
                    let project = super::sessions::project_or_default(project)?;
                    let outcome =
                        outcome.map(|o| fctl_domain::retrospectives::outcome_name(o.into()));
                    api.project_retrospectives(&project, outcome, None, limit)
                        .await?
                }
            };
            if json {
                return style::print_json(&list);
            }
            print_list(&list);
            Ok(())
        }
        Some(RetroCmd::Delete { id }) => {
            let id = RetrospectiveId::parse(id.trim())
                .with_context(|| format!("{id} is not a retrospective's id (rtr_…)"))?;
            config::api(server)?
                .delete_retrospective(id.as_str())
                .await?;
            if json {
                return style::print_json(&fctl_proto::api::Ok { ok: true });
            }
            println!(
                "{} Deleted {}",
                style::accent("✓"),
                style::bold(id.as_str())
            );
            Ok(())
        }
    }
}

/// An agent records a retrospective of its session, through fctl-agent.
async fn record(args: RetroArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl retro records a retrospective in the sandbox of an fctl session, where agents write them. `fctl retro list` reads a project's."
        );
    };
    let (Some(outcome), Some(worked_on)) = (args.outcome, args.worked_on) else {
        bail!(
            "Say how the work ended and what it was: fctl retro --outcome completed --worked-on \"…\" (with --approach, --challenges and --insights)"
        );
    };
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session)),
        call: AgentCall::Retrospective {
            outcome: outcome.into(),
            worked_on,
            approach: args.approach,
            challenges: args.challenges,
            insights: args.insights,
        },
    };
    // fctl-agent waits up to two minutes for the control plane.
    let reply = tokio::time::timeout(
        Duration::from_secs(180),
        super::sessions::ask_agent(&socket, &request),
    )
    .await
    .context("fctl-agent did not answer in time")??;
    let retrospective = match reply {
        SessionReply::Answer {
            answer: AgentAnswer::Retrospective { retrospective },
        } => retrospective,
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!("fctl-agent answered another request"),
    };
    if json {
        return style::print_json(&retrospective);
    }
    println!(
        "{} Recorded a retrospective ({}) of #{}",
        style::accent("✓"),
        outcome_label(retrospective.outcome).to_lowercase(),
        retrospective.session.number
    );
    println!(
        "  {}",
        style::dim("The project's people read it on the session's page.")
    );
    Ok(())
}

fn painted(outcome: RetrospectiveOutcome) -> String {
    let label = outcome_label(outcome);
    match outcome {
        RetrospectiveOutcome::Completed => style::accent(label),
        RetrospectiveOutcome::Partial => style::blue(label),
        RetrospectiveOutcome::Blocked => style::amber(label),
    }
}

fn print_list(list: &RetrospectiveList) {
    if let Some(o) = &list.outcomes {
        let total = o.completed + o.partial + o.blocked;
        if total > 0 {
            let parts: Vec<String> = [
                (o.completed, "completed"),
                (o.partial, "partly done"),
                (o.blocked, "blocked"),
            ]
            .iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, label)| format!("{n} {label}"))
            .collect();
            println!(
                "{}",
                style::dim(&format!(
                    "{}: {}",
                    style::count(total, "retrospective", "retrospectives"),
                    parts.join(", ")
                ))
            );
            println!();
        }
    }
    if list.retrospectives.is_empty() {
        println!("No retrospectives yet. Agents record one as they finish a piece of work.");
        return;
    }
    for (i, r) in list.retrospectives.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_one(r);
    }
}

fn print_one(r: &Retrospective) {
    let who = r
        .started_by
        .as_ref()
        .map(|p| format!(" · {}", p.name))
        .unwrap_or_default();
    println!(
        "{} {} {}",
        painted(r.outcome),
        style::bold(&format!("#{}", r.session.number)),
        style::terminal_safe(&r.session.title)
    );
    println!(
        "  {}",
        style::dim(&style::terminal_safe(&format!(
            "{}{who} · {}",
            r.id,
            style::ago(r.created_at)
        )))
    );
    for (label, text) in [
        ("Worked on", Some(&r.worked_on)),
        ("Approach", r.approach.as_ref()),
        ("Challenges", r.challenges.as_ref()),
        ("Insights", r.insights.as_ref()),
    ] {
        let Some(text) = text else { continue };
        println!("  {}", style::bold(label));
        for line in style::terminal_safe(text).lines() {
            println!("    {line}");
        }
    }
}

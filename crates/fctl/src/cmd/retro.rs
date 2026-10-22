//! `fctl retro`: retrospectives. An agent records one in its session's
//! sandbox as it finishes a piece of work, lists its session's and updates
//! one as later turns carry on with that work, through fctl-agent; people
//! read a project's or a session's, and delete them, through the API.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use fctl_domain::retrospectives::outcome_label;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall, RetrospectiveList};
use fctl_proto::session::{Retrospective, RetrospectiveOutcome};
use fctl_proto::{RetrospectiveId, SessionId};
use std::path::Path;
use std::time::Duration;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct RetroArgs {
    #[command(flatten)]
    pub parts: Parts,
    #[command(subcommand)]
    pub cmd: Option<RetroCmd>,
}

/// How the work ended and what the agent says of it: all of it to record a
/// retrospective, what changed to update one.
#[derive(Args)]
pub struct Parts {
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
    /// The retrospectives of a project, or of one session, newest first. In
    /// a session's sandbox, without a session or a project, the session's
    /// own.
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
    /// Update a retrospective this session recorded, as a later turn
    /// carries on with its piece of work (agents, in a session's sandbox):
    /// the outcome and parts given replace those it had, and the others
    /// stay.
    #[command(
        after_help = "Examples:\n  fctl retro update rtr_01JC3W… --outcome completed\n  fctl retro update rtr_01JC3W… --outcome completed --worked-on \"Retried failed uploads (#41).\" \\\n    --challenges \"The worker's tests need MinIO, which CI lacked until #42.\"\n  fctl retro update rtr_01JC3W… --insights \"\"   # leaves the insights out\n\n`fctl retro list` lists this session's retrospectives, with their ids."
    )]
    Update {
        /// The retrospective's id (rtr_…).
        id: String,
        #[command(flatten)]
        parts: Parts,
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
        None => record(args.parts, json).await,
        Some(RetroCmd::List {
            session,
            project,
            outcome,
            limit,
        }) => {
            let here = if project.is_none() {
                super::sessions::this_sandbox(session.as_deref())
            } else {
                None
            };
            let list = match (here, &session) {
                // The agent's own session, through fctl-agent.
                (Some((socket, id)), _) => {
                    let mut list = RetrospectiveList {
                        retrospectives: listed(&socket, &id).await?,
                        next_before: None,
                        outcomes: None,
                    };
                    only(&mut list, outcome, limit);
                    list
                }
                (None, Some(reference)) => {
                    let api = config::api(server)?;
                    let s = super::sessions::resolve(&api, reference).await?;
                    let mut list = api.session_retrospectives(s.id.as_str()).await?;
                    only(&mut list, outcome, limit);
                    list
                }
                (None, None) => {
                    let api = config::api(server)?;
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
        Some(RetroCmd::Update { id, parts }) => update(&id, parts, json).await,
        Some(RetroCmd::Delete { id }) => {
            let id = retrospective_id(&id)?;
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

fn retrospective_id(id: &str) -> Result<RetrospectiveId> {
    RetrospectiveId::parse(id.trim())
        .with_context(|| format!("{id} is not a retrospective's id (rtr_…)"))
}

/// Keeps those of one outcome, and at most `limit`.
fn only(list: &mut RetrospectiveList, outcome: Option<Outcome>, limit: u32) {
    if let Some(o) = outcome {
        let o = RetrospectiveOutcome::from(o);
        list.retrospectives.retain(|r| r.outcome == o);
    }
    list.retrospectives.truncate(limit as usize);
}

/// Asks the control plane through fctl-agent and the host, for the
/// session of this sandbox.
async fn call(socket: &Path, session: &str, call: AgentCall) -> Result<AgentAnswer> {
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session.to_owned())),
        call,
    };
    // fctl-agent waits up to two minutes for the control plane.
    let reply = tokio::time::timeout(
        Duration::from_secs(180),
        super::sessions::ask_agent(socket, &request),
    )
    .await
    .context("fctl-agent did not answer in time")??;
    match reply {
        SessionReply::Answer { answer } => Ok(answer),
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!("fctl-agent answered another request"),
    }
}

/// The retrospectives the session of this sandbox recorded, newest first.
async fn listed(socket: &Path, session: &str) -> Result<Vec<Retrospective>> {
    match call(socket, session, AgentCall::Retrospectives).await? {
        AgentAnswer::Retrospectives { retrospectives } => Ok(retrospectives),
        _ => bail!("fctl-agent answered another request"),
    }
}

/// The retrospective as the control plane now has it.
fn kept(answer: AgentAnswer) -> Result<Retrospective> {
    match answer {
        AgentAnswer::Retrospective { retrospective } => Ok(retrospective),
        _ => bail!("fctl-agent answered another request"),
    }
}

/// An agent records a retrospective of its session, through fctl-agent.
async fn record(parts: Parts, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl retro records a retrospective in the sandbox of an fctl session, where agents write them. `fctl retro list` reads a project's."
        );
    };
    let (Some(outcome), Some(worked_on)) = (parts.outcome, parts.worked_on) else {
        bail!(
            "Say how the work ended and what it was: fctl retro --outcome completed --worked-on \"…\" (with --approach, --challenges and --insights)"
        );
    };
    let call = AgentCall::Retrospective {
        outcome: outcome.into(),
        worked_on,
        approach: parts.approach,
        challenges: parts.challenges,
        insights: parts.insights,
    };
    let retrospective = kept(self::call(&socket, &session, call).await?)?;
    if json {
        return style::print_json(&retrospective);
    }
    println!(
        "{} Recorded {} ({}) of #{}",
        style::accent("✓"),
        style::bold(retrospective.id.as_str()),
        outcome_label(retrospective.outcome).to_lowercase(),
        retrospective.session.number
    );
    println!(
        "  {}",
        style::dim(&format!(
            "The project's people read it on the session's page. As later turns carry on with this work, update it: fctl retro update {} --outcome …",
            retrospective.id
        ))
    );
    Ok(())
}

/// An agent updates a retrospective of its session, through fctl-agent.
async fn update(id: &str, parts: Parts, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl retro update changes a retrospective in the sandbox of the fctl session that recorded it, where agents write them."
        );
    };
    let id = retrospective_id(id)?;
    if parts.outcome.is_none()
        && parts.worked_on.is_none()
        && parts.approach.is_none()
        && parts.challenges.is_none()
        && parts.insights.is_none()
    {
        bail!(
            "Say what changed: --outcome, --worked-on, --approach, --challenges or --insights (an empty one leaves that part out)"
        );
    }
    let call = AgentCall::UpdateRetrospective {
        id,
        outcome: parts.outcome.map(Into::into),
        worked_on: parts.worked_on,
        approach: parts.approach,
        challenges: parts.challenges,
        insights: parts.insights,
    };
    let retrospective = kept(self::call(&socket, &session, call).await?)?;
    if json {
        return style::print_json(&retrospective);
    }
    println!(
        "{} Updated {} ({}) of #{}",
        style::accent("✓"),
        style::bold(retrospective.id.as_str()),
        outcome_label(retrospective.outcome).to_lowercase(),
        retrospective.session.number
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
    let updated = r
        .updated_at
        .map(|at| format!(" · updated {}", style::ago(at)))
        .unwrap_or_default();
    println!(
        "  {}",
        style::dim(&style::terminal_safe(&format!(
            "{}{who} · {}{updated}",
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

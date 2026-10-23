//! `fctl review`: review agents. People list a project's reviews, ask for
//! one, and turn its review agents on or off and say how they review,
//! through the API; a review agent reads fctl's earlier findings and posts
//! its review in its session's sandbox, through fctl-agent.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_domain::reviews::short;
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{
    AgentAnswer, AgentCall, FindingThread, PostedReview, ResolvedFinding, Review, ReviewAgents,
    ReviewFinding, ReviewState, StartReviewRequest, UpdateReviewAgentsRequest,
};
use fctl_proto::session::SessionStatus;
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Args)]
pub struct ReviewArgs {
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<ReviewCmd>,
}

#[derive(Subcommand)]
pub enum ReviewCmd {
    /// The project's reviews, newest first (the default).
    #[command(visible_alias = "ls")]
    List {
        /// How many.
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Review a pull request's head now, in place of a review of it still
    /// under way (people who start sessions in the project, while its
    /// review agents are on).
    #[command(
        after_help = "Examples:\n  fctl review start 12\n  fctl review start acme/api#7\n  \
                            fctl review start https://github.com/acme/web/pull/12"
    )]
    Start {
        /// The pull request: owner/name#12, its URL, or its number in the
        /// project's repository.
        pull_request: String,
    },
    /// Show the project's review agents, or turn them on or off and say
    /// how they review (the project's owners; the person they run for may
    /// turn them off).
    #[command(
        after_help = "Turning them on runs them on your Claude subscription; another owner takes them over with --mine.\n\nExamples:\n  fctl review settings\n  fctl review settings --on --model opus --effort high\n  fctl review settings --instructions-file review.md\n  fctl review settings --mine\n  fctl review settings --off"
    )]
    Settings {
        /// Turn them on, on your Claude subscription.
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Turn them off. Reviews under way finish.
        #[arg(long)]
        off: bool,
        /// Run them on your Claude subscription from now on.
        #[arg(long)]
        mine: bool,
        /// The model they review with (opus, sonnet, haiku, fable or a
        /// model's ID); `default` for the project's.
        #[arg(long)]
        model: Option<String>,
        /// Their effort level (low, medium, high, xhigh, max); `default` for
        /// the project's.
        #[arg(long)]
        effort: Option<String>,
        /// What they're told besides fctl's own review instructions ("" for
        /// nothing).
        #[arg(long, conflicts_with = "instructions_file")]
        instructions: Option<String>,
        /// Read those instructions from a file (`-`: stdin).
        #[arg(long, value_name = "FILE")]
        instructions_file: Option<PathBuf>,
    },
    /// fctl's earlier findings on the pull request this session reviews,
    /// with their ids, whether they are resolved and what people answered
    /// (review agents, in their session's sandbox).
    Threads,
    /// Post this session's review, once, at the end (review agents, in
    /// their session's sandbox): what it found, and the earlier findings
    /// the new commits fixed.
    #[command(
        after_help = "The file holds JSON:\n\n  {\n    \"summary\": \"Optional: a sentence or two on the pull request as a whole.\",\n    \"findings\": [\n      { \"path\": \"src/upload.rs\", \"line\": 42, \"start_line\": 38, \"severity\": \"high\",\n        \"title\": \"Retries never stop when the server is down\",\n        \"body\": \"`retry_upload` loops until the request succeeds…\",\n        \"suggestion\": \"optional: the lines that replace start_line to line\" }\n    ],\n    \"resolved\": [ { \"thread\": \"PRRT_kwDOABCD1234\", \"note\": \"The loop now gives up after five attempts.\" } ]\n  }\n\nWithout findings it posts nothing new and passes the review's check."
    )]
    Submit {
        /// The review, as JSON (`-` reads it from stdin).
        file: PathBuf,
    },
}

pub async fn run(server: Option<&str>, args: ReviewArgs, json: bool) -> Result<()> {
    match args.cmd.unwrap_or(ReviewCmd::List { limit: 20 }) {
        ReviewCmd::List { limit } => list(server, args.project, limit, json).await,
        ReviewCmd::Start { pull_request } => {
            let api = config::api(server)?;
            let project = super::sessions::project_or_default(args.project)?;
            let review = api
                .start_review(&project, &StartReviewRequest { pull_request })
                .await?;
            if json {
                return style::print_json(&review);
            }
            println!(
                "{} Reviewing {}#{} at {} in session #{}",
                style::accent("✓"),
                review.repo.full_name(),
                review.number,
                short(&review.head_sha),
                review.session_number
            );
            println!(
                "  {}",
                style::dim(&format!(
                    "It runs on {}'s Claude subscription and posts what it finds on GitHub. Follow it: fctl attach {}",
                    review.operator.name, review.session_id
                ))
            );
            Ok(())
        }
        ReviewCmd::Settings {
            on,
            off,
            mine,
            model,
            effort,
            instructions,
            instructions_file,
        } => {
            let api = config::api(server)?;
            let project = super::sessions::project_or_default(args.project)?;
            let instructions = match instructions_file {
                Some(path) => Some(read(&path)?),
                None => instructions,
            };
            let default = |v: Option<String>| {
                v.map(|v| {
                    if v.trim().eq_ignore_ascii_case("default") {
                        String::new()
                    } else {
                        v
                    }
                })
            };
            let req = UpdateReviewAgentsRequest {
                enabled: if on {
                    Some(true)
                } else if off {
                    Some(false)
                } else {
                    None
                },
                run_as_me: mine.then_some(true),
                model: default(model),
                effort: default(effort),
                instructions,
            };
            let changes = req != UpdateReviewAgentsRequest::default();
            let agents = if changes {
                api.update_review_agents(&project, &req).await?
            } else {
                api.review_agents(&project).await?
            };
            if json {
                return style::print_json(&agents);
            }
            print_settings(&project, &agents);
            Ok(())
        }
        ReviewCmd::Threads => threads(json).await,
        ReviewCmd::Submit { file } => submit(&file, json).await,
    }
}

/// A file's text, or stdin's for `-`.
fn read(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

async fn list(server: Option<&str>, project: Option<String>, limit: u32, json: bool) -> Result<()> {
    if project.is_none()
        && let Some((socket, session)) = super::sessions::this_sandbox(None)
        && let Ok(AgentAnswer::ReviewThreads { .. }) =
            call(&socket, &session, AgentCall::ReviewThreads).await
    {
        bail!(
            "This is a review agent's sandbox: `fctl review threads` lists fctl's earlier findings on its pull request, and `fctl review submit` posts its review."
        );
    }
    let api = config::api(server)?;
    let project = super::sessions::project_or_default(project)?;
    let (agents, reviews) = tokio::try_join!(
        api.review_agents(&project),
        api.reviews(&project, None, limit)
    )?;
    if json {
        return style::print_json(&reviews);
    }
    println!(
        "{}",
        style::dim(&format!(
            "Review agents in {project}: {}",
            agents_line(&agents)
        ))
    );
    if let Some(problem) = &agents.problem {
        println!("{}", style::amber(&style::terminal_safe(problem)));
    }
    println!();
    if reviews.reviews.is_empty() {
        println!(
            "No reviews yet. {}",
            if agents.enabled {
                "A pull request's new commit starts one, or `fctl review start <pull request>`."
            } else {
                "The project's owners turn review agents on with `fctl review settings --on`."
            }
        );
        return Ok(());
    }
    for (i, r) in reviews.reviews.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_review(r);
    }
    Ok(())
}

/// `on · Ada's subscription · opus · high`.
fn agents_line(a: &ReviewAgents) -> String {
    if !a.enabled {
        return "off".into();
    }
    let mut parts = vec!["on".to_owned()];
    if let Some(op) = &a.operator {
        parts.push(format!("on {}'s Claude subscription", op.name));
    }
    parts.push(
        a.model
            .clone()
            .unwrap_or_else(|| "the project's model".into()),
    );
    if let Some(e) = &a.effort {
        parts.push(format!("{e} effort"));
    }
    parts.join(" · ")
}

fn print_settings(project: &str, a: &ReviewAgents) {
    println!(
        "Review agents in {project}: {}",
        if a.enabled {
            style::accent("on")
        } else {
            style::dim("off")
        }
    );
    let row = |k: &str, v: String| println!("  {:<14}{}", style::dim(k), style::terminal_safe(&v));
    if let Some(op) = &a.operator {
        row(
            "Runs on",
            format!(
                "{}'s Claude subscription{}",
                op.name,
                if a.enabled { "" } else { " (when on)" }
            ),
        );
    }
    row(
        "Model",
        a.model.clone().unwrap_or_else(|| "the project's".into()),
    );
    row(
        "Effort",
        a.effort.clone().unwrap_or_else(|| "the project's".into()),
    );
    let repos: Vec<String> = a.repos.iter().map(|r| r.full_name()).collect();
    row(
        "Repositories",
        if repos.is_empty() {
            "none: link one the project's agents may write to".into()
        } else {
            repos.join(", ")
        },
    );
    match a.instructions.as_deref() {
        Some(text) => {
            println!("  {}", style::dim("Instructions"));
            for line in style::terminal_safe(text).lines() {
                println!("    {line}");
            }
        }
        None => row("Instructions", "none".into()),
    }
    if let Some(problem) = &a.problem {
        println!();
        println!("{}", style::amber(&style::terminal_safe(problem)));
    }
}

/// What came of a review, in a few words.
pub fn outcome(r: &Review) -> String {
    match r.state {
        ReviewState::Running if r.status == SessionStatus::Queued => "Waiting for a runner".into(),
        ReviewState::Running => "Reviewing".into(),
        ReviewState::Passed => "No issues".into(),
        ReviewState::Commented if r.findings == 0 => match r.still_open {
            1 => "1 earlier finding open".into(),
            n => format!("{n} earlier findings open"),
        },
        ReviewState::Commented => style::count(r.findings, "issue", "issues"),
        ReviewState::Superseded => "Superseded".into(),
        ReviewState::Stopped => "Stopped".into(),
        ReviewState::Failed => "Failed".into(),
    }
}

fn painted(r: &Review) -> String {
    let label = outcome(r);
    match r.state {
        ReviewState::Running => style::blue(&label),
        ReviewState::Passed => style::accent(&label),
        ReviewState::Commented => style::amber(&label),
        ReviewState::Failed => style::red(&label),
        ReviewState::Superseded | ReviewState::Stopped => style::dim(&label),
    }
}

fn print_review(r: &Review) {
    println!(
        "{} {} {}",
        painted(r),
        style::bold(&format!("{}#{}", r.repo.name, r.number)),
        style::terminal_safe(r.title.as_deref().unwrap_or(""))
    );
    let mut facts = vec![
        format!("commit {}", short(&r.head_sha)),
        format!("session #{}", r.session_number),
        style::ago(r.created_at),
    ];
    if r.resolved > 0 {
        facts.insert(0, format!("{} fixed", r.resolved));
    }
    println!("  {}", style::dim(&facts.join(" · ")));
    if let Some(url) = &r.review_url {
        println!("  {}", style::dim(&style::terminal_safe(url)));
    }
    if let Some(reason) = r.reason.as_deref().filter(|_| r.state.is_done()) {
        println!("  {}", style::dim(&style::terminal_safe(reason)));
    }
}

// ───────────────────────────── in a review agent's sandbox ─────────────────────────────

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

fn sandbox(what: &str) -> Result<(PathBuf, String)> {
    super::sessions::this_sandbox(None).with_context(|| {
        format!(
            "fctl review {what} works in the sandbox of a review agent's session. Elsewhere, `fctl review` lists a project's reviews."
        )
    })
}

async fn threads(json: bool) -> Result<()> {
    let (socket, session) = sandbox("threads")?;
    let threads = match call(&socket, &session, AgentCall::ReviewThreads).await? {
        AgentAnswer::ReviewThreads { threads } => threads,
        _ => bail!("fctl-agent answered another request"),
    };
    if json {
        return style::print_json(&threads);
    }
    if threads.is_empty() {
        println!("fctl hasn't posted findings on this pull request before.");
        return Ok(());
    }
    for (i, t) in threads.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_thread(t);
    }
    Ok(())
}

fn print_thread(t: &FindingThread) {
    let state = match (t.resolved, t.outdated) {
        (true, _) => style::dim("resolved"),
        (false, true) => style::amber("open, on lines that changed since"),
        (false, false) => style::amber("open"),
    };
    let place = match (&t.path, t.line) {
        (Some(p), Some(l)) => format!("{p}:{l}"),
        (Some(p), None) => p.clone(),
        _ => String::new(),
    };
    println!(
        "{} {} {}",
        style::bold(&t.id),
        state,
        style::dim(&style::terminal_safe(&place))
    );
    for line in style::terminal_safe(&t.body).lines() {
        println!("  {line}");
    }
    for r in &t.replies {
        let who = r.author.as_deref().unwrap_or("someone");
        println!("  {}", style::blue(&format!("↳ @{who}:")));
        for line in style::terminal_safe(&r.body).lines() {
            println!("    {line}");
        }
    }
}

/// What `fctl review submit` reads.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    findings: Vec<ReviewFinding>,
    #[serde(default)]
    resolved: Vec<ResolvedFinding>,
}

async fn submit(file: &Path, json: bool) -> Result<()> {
    let (socket, session) = sandbox("submit")?;
    let text = read(file)?;
    let sub: Submission = serde_json::from_str(&text).with_context(|| {
        format!(
            "{} isn't a review: it holds JSON with `summary`, `findings` and `resolved`, as `fctl review submit --help` shows",
            file.display()
        )
    })?;
    let answer = call(
        &socket,
        &session,
        AgentCall::SubmitReview {
            summary: sub.summary,
            findings: sub.findings,
            resolved: sub.resolved,
        },
    )
    .await?;
    let posted = match answer {
        AgentAnswer::ReviewPosted { review } => review,
        _ => bail!("fctl-agent answered another request"),
    };
    if json {
        return style::print_json(&posted);
    }
    print_posted(&posted);
    Ok(())
}

fn print_posted(p: &PostedReview) {
    if p.again {
        println!(
            "{} This review was posted already{}: it posts once.",
            style::accent("✓"),
            p.url
                .as_deref()
                .map(|u| format!(" ({u})"))
                .unwrap_or_default()
        );
        return;
    }
    let found = p.comments + p.in_text;
    match &p.url {
        Some(url) => println!(
            "{} Posted {} on GitHub: {url}",
            style::accent("✓"),
            style::count(found, "finding", "findings")
        ),
        None => println!(
            "{} Nothing new to point out: the review's check passed{}",
            style::accent("✓"),
            if p.still_open > 0 {
                " once the earlier findings still open are resolved"
            } else {
                ""
            }
        ),
    }
    if p.in_text > 0 {
        println!(
            "  {}",
            style::dim(&format!(
                "{} on lines the diff doesn't show went in the review's text.",
                style::count(p.in_text, "finding", "findings")
            ))
        );
    }
    if !p.resolved.is_empty() {
        println!(
            "  {}",
            style::dim(&format!(
                "Marked {} fixed and resolved.",
                style::count(
                    p.resolved.len() as u32,
                    "earlier finding",
                    "earlier findings"
                )
            ))
        );
    }
    if p.still_open > 0 {
        println!(
            "  {}",
            style::dim(&format!(
                "{} still open.",
                style::count(p.still_open, "earlier finding is", "earlier findings are")
            ))
        );
    }
    if p.sent_to.is_some() {
        println!(
            "  {}",
            style::dim(
                "fctl sent the findings to the agent of the session that opened the pull request."
            )
        );
    }
}

//! `fctl run`, `sessions`, `show`, `logs`, `send`, `interrupt`, `stop`, `resume`, `open`.

use crate::config;
use crate::render::Renderer;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::Args;
use fctl_client::Api;
use fctl_proto::api::CreateSessionRequest;
use fctl_proto::session::Session;
use std::io::{IsTerminal, Read};

#[derive(Args)]
pub struct RunArgs {
    /// What the agent should do (or pipe it in, or use --file).
    pub prompt: Vec<String>,
    /// The project (slug or id); default from `fctl projects use`.
    #[arg(short, long)]
    pub project: Option<String>,
    /// Read the prompt from a file.
    #[arg(short, long)]
    pub file: Option<std::path::PathBuf>,
    /// Session title (default: from the prompt).
    #[arg(long)]
    pub title: Option<String>,
    /// Branch to work on (default: fctl/<number>-<title>).
    #[arg(long)]
    pub branch: Option<String>,
    /// Branch to start from (default: the project's default branch).
    #[arg(long)]
    pub base: Option<String>,
    /// Claude model (opus, sonnet, fable, or a full model name).
    #[arg(long)]
    pub model: Option<String>,
    /// Effort: low, medium, high, xhigh or max.
    #[arg(long)]
    pub effort: Option<String>,
    /// Start it and return instead of following along.
    #[arg(short, long)]
    pub detach: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// Only this project's sessions (slug or id).
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only active sessions.
    #[arg(short, long)]
    pub active: bool,
    /// How many to show.
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
}

/// Resolves `ses_…`, a URL ending in it, or `project#12`.
pub async fn resolve(api: &Api, reference: &str) -> Result<Session> {
    let r = reference.trim();
    if let Some(id) = r.rsplit('/').next().filter(|s| s.starts_with("ses_")) {
        return Ok(api.session(id).await?);
    }
    if let Some((project, number)) = r.split_once('#') {
        let n: u32 = number
            .parse()
            .context("use project#number, like my-app#12")?;
        let list = api.sessions(Some(project), None, 200).await?;
        return list
            .sessions
            .into_iter()
            .find(|s| s.number == n)
            .with_context(|| {
                format!("{project} has no session #{n} (or it is older than the last 200)")
            });
    }
    bail!("Refer to a session by id (ses_…) or as project#number")
}

pub fn project_or_default(p: Option<String>) -> Result<String> {
    p.or_else(|| config::CliConfig::load().project).context(
        "Which project? Pass --project, or set a default with `fctl projects use <project>`.",
    )
}

pub async fn start(server: Option<&str>, args: RunArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = project_or_default(args.project)?;
    let prompt = if let Some(f) = &args.file {
        std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?
    } else if !args.prompt.is_empty() {
        args.prompt.join(" ")
    } else if !std::io::stdin().is_terminal() {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        s
    } else {
        bail!("What should the agent do? `fctl run -p {project} \"Fix the flaky login test\"`");
    };
    let session = api
        .create_session(
            &project,
            &CreateSessionRequest {
                prompt,
                title: args.title,
                base_branch: args.base,
                branch: args.branch,
                model: args.model,
                effort: args.effort,
            },
        )
        .await?;
    if json {
        return style::print_json(&session);
    }
    println!(
        "{} {} {}",
        style::accent("✓"),
        style::bold(&format!(
            "{}#{} {}",
            session.project_slug, session.number, session.title
        )),
        style::dim(&format!("{}/s/{}", api.base(), session.id))
    );
    if args.detach {
        return Ok(());
    }
    crate::cmd::attach::follow(&api, &session.id.0, false).await
}

pub async fn list(server: Option<&str>, args: ListArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = args.project.or_else(|| config::CliConfig::load().project);
    let status = args.active.then_some("active");
    let list = api.sessions(project.as_deref(), status, args.limit).await?;
    if json {
        return style::print_json(&list);
    }
    if list.sessions.is_empty() {
        println!("No sessions. Start one with `fctl run`.");
    }
    for s in &list.sessions {
        let when = style::ago(s.last_activity_at.unwrap_or(s.created_at));
        let reference = format!("{}#{}", s.project_slug, s.number);
        println!(
            "{:<20} {:<18} {:<44} {}",
            style::bold(&reference),
            style::status(s.status),
            truncate(&s.title, 44),
            style::dim(&when)
        );
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

pub async fn show(server: Option<&str>, reference: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} {}",
        style::bold(&format!("{}#{}", s.project_slug, s.number)),
        s.title
    );
    println!(
        "status:   {}{}",
        style::status(s.status),
        s.status_reason
            .as_deref()
            .map(|r| style::dim(&format!(" — {r}")))
            .unwrap_or_default()
    );
    println!("by:       {}", s.created_by.name);
    if let Some(b) = &s.branch {
        println!(
            "branch:   {b}{}",
            s.base_branch
                .as_deref()
                .map(|x| style::dim(&format!(" (from {x})")))
                .unwrap_or_default()
        );
    }
    if let Some(pr) = &s.pull_request {
        println!("pr:       #{} {}", pr.number, pr.url);
    }
    if let Some(r) = &s.runner {
        println!("runner:   {}", r.name);
    }
    if let Some(m) = &s.model {
        println!("model:    {m}");
    }
    let st = &s.stats;
    println!(
        "work:     {} turns · {} tool calls · {} files (+{} −{})",
        st.turns, st.tool_calls, st.files_changed, st.additions, st.deletions
    );
    if let Some(rc) = s
        .remote_control
        .as_ref()
        .and_then(|r| r.session_url.as_ref())
    {
        println!("claude:   {rc}");
    }
    println!("{}", style::dim(&format!("{}/s/{}", api.base(), s.id)));
    Ok(())
}

pub async fn logs(server: Option<&str>, reference: &str, follow: bool, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let mut r = Renderer::default();
    r.json = json;
    let mut after = 0u64;
    loop {
        let page = api.events(&s.id.0, after, 1000).await?;
        for ev in &page.events {
            after = after.max(ev.seq);
            r.event(ev);
        }
        if !page.has_more {
            break;
        }
    }
    if follow {
        return crate::cmd::attach::follow_after(&api, &s.id.0, true, Some(after), r).await;
    }
    Ok(())
}

pub async fn send(server: Option<&str>, reference: &str, message: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let text = if message.trim().is_empty() && !std::io::stdin().is_terminal() {
        let mut t = String::new();
        std::io::stdin().read_to_string(&mut t)?;
        t
    } else {
        message.to_owned()
    };
    let r = api.send_message(&s.id.0, &text, None).await?;
    if json {
        return style::print_json(&r);
    }
    println!(
        "{} Sent to {}#{}",
        style::accent("✓"),
        s.project_slug,
        s.number
    );
    Ok(())
}

pub async fn interrupt(server: Option<&str>, reference: &str) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    api.interrupt(&s.id.0).await?;
    println!("Interrupted {}#{}", s.project_slug, s.number);
    Ok(())
}

pub async fn stop(server: Option<&str>, reference: &str, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let s = api.stop(&s.id.0).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} {}#{}",
        style::status(s.status),
        s.project_slug,
        s.number
    );
    Ok(())
}

pub async fn resume(
    server: Option<&str>,
    reference: &str,
    message: &str,
    json: bool,
) -> Result<()> {
    let api = config::api(server)?;
    let s = resolve(&api, reference).await?;
    let prompt = (!message.trim().is_empty()).then(|| message.to_owned());
    let s = api.resume(&s.id.0, prompt).await?;
    if json {
        return style::print_json(&s);
    }
    println!(
        "{} Resuming {}#{}",
        style::accent("✓"),
        s.project_slug,
        s.number
    );
    Ok(())
}

pub async fn open(server: Option<&str>, target: Option<&str>, claude: bool) -> Result<()> {
    let api = config::api(server)?;
    let url = match target {
        None => api.base().to_owned(),
        Some(t) if t.starts_with("ses_") || t.contains('#') => {
            let s = resolve(&api, t).await?;
            if claude {
                s.remote_control.and_then(|r| r.session_url).context(
                    "This session has no Remote Control link (it needs a full Claude login)",
                )?
            } else {
                format!("{}/s/{}", api.base(), s.id)
            }
        }
        Some(p) => {
            let p = api.project(p).await?;
            format!("{}/p/{}", api.base(), p.slug)
        }
    };
    if !crate::cmd::auth::open_browser(&url) {
        println!("{url}");
    }
    Ok(())
}

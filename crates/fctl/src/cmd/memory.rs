//! `fctl memory`: a project's memory, the notes agents and people keep for
//! the agents of later sessions. In a session's sandbox the agent reads and
//! keeps its own project's through fctl-agent, without a sign-in; people
//! read and keep any project's they can see through the API.

use crate::config;
use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_client::Api;
use fctl_domain::memory;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AddMemoryRequest, AgentAnswer, AgentCall, UpdateMemoryRequest};
use fctl_proto::session::Memory;
use fctl_proto::{MemoryId, SessionId};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Args)]
pub struct MemoryArgs {
    /// The project (slug or id); default from `fctl projects use`. In a
    /// sandbox, leave it out for the session's own.
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    /// List the stale notes too.
    #[arg(long)]
    pub stale: bool,
    #[command(subcommand)]
    pub cmd: Option<MemoryCmd>,
}

#[derive(Subcommand)]
pub enum MemoryCmd {
    /// List the notes, newest first (the default).
    #[command(visible_alias = "ls")]
    List {
        /// The stale notes too, after the current ones.
        #[arg(long)]
        stale: bool,
    },
    /// Show a note, with where it came from.
    Show {
        /// The note's id (mem_…).
        id: String,
    },
    /// Remember something for the agents of later sessions.
    #[command(
        after_help = "One fact per note, in a sentence or two, that holds beyond this task.\n\nExamples:\n  fctl memory add \"The e2e tests need Chromium: run npx playwright install chromium first.\"\n  fctl memory add \"Run cargo xtask check --quick before pushing.\" --replaces mem_01JC3V…\n  echo \"Deploys happen on Fridays.\" | fctl memory add -p my-app"
    )]
    Add {
        /// What to remember (Markdown; or pipe it in).
        text: Vec<String>,
        /// A note this one takes the place of, which goes stale.
        #[arg(long, value_name = "ID")]
        replaces: Option<String>,
    },
    /// Mark a note stale: it turned out wrong, or out of date. Agents no
    /// longer start with it.
    Stale {
        /// The note's id (mem_…).
        id: String,
        /// Why (agents always say).
        #[arg(long)]
        reason: Option<String>,
    },
    /// Make a stale note current again (people who can edit the project).
    Restore {
        /// The note's id (mem_…).
        id: String,
    },
    /// Change a note's text (people who can edit the project).
    Edit {
        /// The note's id (mem_…).
        id: String,
        /// Its new text (or pipe it in).
        text: Vec<String>,
    },
    /// Delete notes (people who can edit the project).
    #[command(visible_alias = "rm")]
    Delete {
        /// The notes' ids (mem_…).
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

/// Where `fctl memory` reads and keeps notes.
enum Keeper {
    /// The sandbox of a session: its agent, through fctl-agent.
    Agent { socket: PathBuf, session: String },
    /// A person, through the API.
    Person { api: Api, project: Option<String> },
}

impl Keeper {
    fn new(server: Option<&str>, project: Option<String>) -> Result<Self> {
        if project.is_none()
            && let Some((socket, session)) = super::sessions::this_sandbox(None)
        {
            return Ok(Self::Agent { socket, session });
        }
        Ok(Self::Person {
            api: config::api(server)?,
            project,
        })
    }

    async fn list(&self, stale: bool) -> Result<Vec<Memory>> {
        match self {
            Self::Agent { socket, session } => {
                match call(socket, session, AgentCall::Memories { stale }).await? {
                    AgentAnswer::Memories { memories } => Ok(memories),
                    _ => bail!("fctl-agent answered another request"),
                }
            }
            Self::Person { api, project } => {
                let project = super::sessions::project_or_default(project.clone())?;
                let mut notes = api.memories(&project).await?.memories;
                notes.retain(|m| stale || m.stale.is_none());
                Ok(notes)
            }
        }
    }

    fn api(&self, what: &str) -> Result<&Api> {
        match self {
            Self::Person { api, .. } => Ok(api),
            Self::Agent { .. } => bail!(
                "Agents can't {what}: people who can edit the project do that, in its Memory tab or with fctl memory outside the sandbox"
            ),
        }
    }
}

/// Asks the control plane through fctl-agent and the host.
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
        SessionReply::Edited { .. } | SessionReply::Published { .. } => {
            bail!("fctl-agent answered another request")
        }
    }
}

/// The words given, or what is piped in.
fn text_of(words: &[String]) -> Result<String> {
    let text = words.join(" ");
    if text.trim().is_empty() && !std::io::stdin().is_terminal() {
        return Ok(std::io::read_to_string(std::io::stdin())?);
    }
    Ok(text)
}

fn memory_id(id: &str) -> Result<MemoryId> {
    MemoryId::parse(id.trim()).with_context(|| format!("{id} is not a note's id (mem_…)"))
}

pub async fn run(server: Option<&str>, args: MemoryArgs, json: bool) -> Result<()> {
    let cmd = args.cmd.unwrap_or(MemoryCmd::List { stale: args.stale });
    let keeper = Keeper::new(server, args.project)?;
    let now = style::now_ms();
    match cmd {
        MemoryCmd::List { stale } => {
            let notes = keeper.list(stale || args.stale).await?;
            if json {
                return style::print_json(&serde_json::json!({ "memories": notes }));
            }
            print_list(&notes, now);
        }
        MemoryCmd::Show { id } => {
            let id = memory_id(&id)?;
            let note =
                match &keeper {
                    Keeper::Person { api, .. } => api.memory(id.as_str()).await?,
                    Keeper::Agent { .. } => keeper
                        .list(true)
                        .await?
                        .into_iter()
                        .find(|m| m.id == id)
                        .with_context(|| format!("The project's memory has no {id}"))?,
                };
            if json {
                return style::print_json(&note);
            }
            print_note(&note, now);
        }
        MemoryCmd::Add { text, replaces } => {
            let text = text_of(&text)?;
            let replaces = replaces.as_deref().map(memory_id).transpose()?;
            let (note, replaced) = match &keeper {
                Keeper::Agent { socket, session } => {
                    match call(
                        socket,
                        session,
                        AgentCall::Remember {
                            text,
                            replaces: replaces.clone(),
                        },
                    )
                    .await?
                    {
                        AgentAnswer::Memory { memory, replaced } => (memory, replaced),
                        _ => bail!("fctl-agent answered another request"),
                    }
                }
                Keeper::Person { api, project } => {
                    let project = super::sessions::project_or_default(project.clone())?;
                    let note = api.add_memory(&project, &AddMemoryRequest { text }).await?;
                    // A person replaces a note by marking it stale with a pointer.
                    let replaced = match &replaces {
                        Some(old) => Some(
                            api.update_memory(
                                old.as_str(),
                                &UpdateMemoryRequest {
                                    stale: Some(true),
                                    reason: Some(format!("Replaced by {}", note.id)),
                                    ..Default::default()
                                },
                            )
                            .await?,
                        ),
                        None => None,
                    };
                    (note, replaced)
                }
            };
            if json {
                return style::print_json(
                    &serde_json::json!({ "memory": note, "replaced": replaced }),
                );
            }
            println!(
                "{} Remembered {}",
                style::accent("✓"),
                style::bold(note.id.as_str())
            );
            print_text(&note.text);
            if let Some(old) = replaced {
                println!(
                    "{} {} is stale now",
                    style::dim("·"),
                    style::bold(old.id.as_str())
                );
            }
        }
        MemoryCmd::Stale { id, reason } => {
            let id = memory_id(&id)?;
            let note = match &keeper {
                Keeper::Agent { socket, session } => {
                    let Some(reason) = reason.filter(|r| !r.trim().is_empty()) else {
                        bail!("Say why the note is stale: --reason \"…\"");
                    };
                    match call(socket, session, AgentCall::MarkStale { id, reason }).await? {
                        AgentAnswer::Memory { memory, .. } => memory,
                        _ => bail!("fctl-agent answered another request"),
                    }
                }
                Keeper::Person { api, .. } => {
                    api.update_memory(
                        id.as_str(),
                        &UpdateMemoryRequest {
                            stale: Some(true),
                            reason,
                            ..Default::default()
                        },
                    )
                    .await?
                }
            };
            if json {
                return style::print_json(&note);
            }
            println!(
                "{} {} is stale: agents no longer start with it",
                style::accent("✓"),
                style::bold(note.id.as_str())
            );
        }
        MemoryCmd::Restore { id } => {
            let id = memory_id(&id)?;
            let note = keeper
                .api("make a note current again")?
                .update_memory(
                    id.as_str(),
                    &UpdateMemoryRequest {
                        stale: Some(false),
                        ..Default::default()
                    },
                )
                .await?;
            if json {
                return style::print_json(&note);
            }
            println!(
                "{} {} is current again: agents start with it",
                style::accent("✓"),
                style::bold(note.id.as_str())
            );
        }
        MemoryCmd::Edit { id, text } => {
            let id = memory_id(&id)?;
            let text = text_of(&text)?;
            let note = keeper
                .api("change a note")?
                .update_memory(
                    id.as_str(),
                    &UpdateMemoryRequest {
                        text: Some(text),
                        ..Default::default()
                    },
                )
                .await?;
            if json {
                return style::print_json(&note);
            }
            println!(
                "{} Changed {}",
                style::accent("✓"),
                style::bold(note.id.as_str())
            );
            print_text(&note.text);
        }
        MemoryCmd::Delete { ids } => {
            let api = keeper.api("delete notes")?;
            for id in &ids {
                let id = memory_id(id)?;
                api.delete_memory(id.as_str()).await?;
                if !json {
                    println!(
                        "{} Deleted {}",
                        style::accent("✓"),
                        style::bold(id.as_str())
                    );
                }
            }
            if json {
                return style::print_json(&fctl_proto::api::Ok { ok: true });
            }
        }
    }
    Ok(())
}

/// A note's text, indented under its heading.
fn print_text(text: &str) {
    for line in style::terminal_safe(text).lines() {
        println!("  {line}");
    }
}

/// The line that heads a note: its id, where it came from, how old it is.
fn heading(m: &Memory, now: i64) -> String {
    let mut facts = vec![memory::source(m), memory::age(now, m.created_at)];
    if let Some(at) = m.edited_at {
        facts.push(format!("edited {}", memory::age(now, at)));
    }
    let facts = style::terminal_safe(&facts.join(" · ")).into_owned();
    format!("{} {}", style::bold(m.id.as_str()), style::dim(&facts))
}

/// Why and since when a note is stale.
fn staleness(m: &Memory, now: i64) -> Option<String> {
    let s = m.stale.as_ref()?;
    let mut line = format!("stale {}", memory::age(now, s.at));
    match (&s.session, &s.by) {
        (Some(session), _) => line.push_str(&format!(", marked by session #{}", session.number)),
        (None, Some(person)) => line.push_str(&format!(", marked by {}", person.name)),
        _ => {}
    }
    if let Some(reason) = &s.reason {
        line.push_str(&format!(": {reason}"));
    }
    Some(style::terminal_safe(&line).into_owned())
}

fn print_note(m: &Memory, now: i64) {
    println!("{}", heading(m, now));
    print_text(&m.text);
    if let Some(s) = &m.session {
        println!(
            "  {}",
            style::dim(&style::terminal_safe(&format!(
                "from session #{}: {}",
                s.number, s.title
            )))
        );
    }
    if let Some(line) = staleness(m, now) {
        println!("  {}", style::amber(&line));
    }
}

fn print_list(notes: &[Memory], now: i64) {
    if notes.is_empty() {
        println!("No notes yet. Add one: fctl memory add \"…\"");
        return;
    }
    for (i, m) in notes.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("{}", heading(m, now));
        print_text(&m.text);
        if let Some(line) = staleness(m, now) {
            println!("  {}", style::amber(&line));
        }
    }
    let current = notes.iter().filter(|m| m.stale.is_none()).count();
    let stale = notes.len() - current;
    let mut summary = style::count(current as u64, "current note", "current notes");
    if stale > 0 {
        summary.push_str(&format!(", {stale} stale"));
    }
    println!();
    println!("{}", style::dim(&summary));
}

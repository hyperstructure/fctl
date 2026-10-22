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
        after_help = "One fact per note, in a sentence or two, that holds beyond this task.\n\nExamples:\n  fctl memory add \"The e2e tests need Chromium: run npx playwright install chromium first.\"\n  fctl memory add \"Run cargo xtask check --quick before pushing.\" --replaces mem_01JC3V…\n  fctl memory add \"…what both notes say…\" --replaces mem_01JC3V… --replaces mem_01JC2Q…\n  echo \"Deploys happen on Fridays.\" | fctl memory add -p my-app"
    )]
    Add {
        /// What to remember (Markdown; or pipe it in).
        text: Vec<String>,
        /// A note this one takes the place of, which goes stale. Repeat it,
        /// or separate ids with commas, for a note that sums up several.
        #[arg(long, value_name = "ID", value_delimiter = ',')]
        replaces: Vec<String>,
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
    /// Mark a note a duplicate of another that says all it does: it goes
    /// stale, and agents no longer start with it.
    #[command(
        after_help = "When two notes overlap and each says something the other doesn't, add one that says it all instead:\n  fctl memory add \"…\" --replaces mem_01JC3V… --replaces mem_01JC2Q…\n\nExample:\n  fctl memory duplicate mem_01JC3V… --of mem_01JC2Q…"
    )]
    Duplicate {
        /// The note that repeats the other (mem_…).
        id: String,
        /// The note it repeats, a current one, which agents keep starting
        /// with (mem_…).
        #[arg(long, value_name = "ID")]
        of: String,
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
    Agent {
        socket: PathBuf,
        session: String,
        /// The `--server` given, for what only people do.
        server: Option<String>,
    },
    /// A person, through the API.
    Person { api: Api, project: Option<String> },
}

impl Keeper {
    fn new(server: Option<&str>, project: Option<String>) -> Result<Self> {
        if project.is_none()
            && let Some((socket, session)) = super::sessions::this_sandbox(None)
        {
            return Ok(Self::Agent {
                socket,
                session,
                server: server.map(str::to_owned),
            });
        }
        Ok(Self::Person {
            api: config::api(server)?,
            project,
        })
    }

    async fn list(&self, stale: bool) -> Result<Vec<Memory>> {
        match self {
            Self::Agent {
                socket, session, ..
            } => match call(socket, session, AgentCall::Memories { stale }).await? {
                AgentAnswer::Memories { memories } => Ok(memories),
                _ => bail!("fctl-agent answered another request"),
            },
            Self::Person { api, project } => {
                let project = super::sessions::project_or_default(project.clone())?;
                let mut notes = api.memories(&project).await?.memories;
                notes.retain(|m| stale || m.stale.is_none());
                Ok(notes)
            }
        }
    }

    /// The API, for what only people do: in a sandbox too, when a person
    /// signed fctl in there.
    fn api(&self, what: &str) -> Result<Api> {
        match self {
            Self::Person { api, .. } => Ok(api.clone()),
            Self::Agent { server, .. } => config::api(server.as_deref()).map_err(|_| {
                anyhow::anyhow!(
                    "Agents can't {what}: people who can edit the project do that, in its Memory tab or with fctl memory outside the sandbox"
                )
            }),
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
            let ids = replaces
                .iter()
                .map(|id| memory_id(id))
                .collect::<Result<Vec<_>>>()?;
            let replaces = memory::replaced(None, &ids).map_err(|e| anyhow::anyhow!(e.message))?;
            // The first as `replaces`, which fctl-agents and control planes
            // from before `also_replaces` take alone.
            let (first, more) = match replaces.split_first() {
                Some((first, more)) => (Some(first.clone()), more.to_vec()),
                None => (None, Vec::new()),
            };
            let (note, mut replaced) = match &keeper {
                Keeper::Agent {
                    socket, session, ..
                } => {
                    match call(
                        socket,
                        session,
                        AgentCall::Remember {
                            text,
                            replaces: first,
                            also_replaces: more,
                        },
                    )
                    .await?
                    {
                        AgentAnswer::Memory {
                            memory,
                            replaced,
                            also_replaced,
                        } => (memory, replaced.into_iter().chain(also_replaced).collect()),
                        _ => bail!("fctl-agent answered another request"),
                    }
                }
                Keeper::Person { api, project } => {
                    let project = super::sessions::project_or_default(project.clone())?;
                    let note = api
                        .add_memory(
                            &project,
                            &AddMemoryRequest {
                                text,
                                replaces: first,
                                also_replaces: more,
                            },
                        )
                        .await?;
                    // The control plane marked them stale with the new note.
                    let mut replaced = Vec::with_capacity(replaces.len());
                    for old in &replaces {
                        replaced.push(api.memory(old.as_str()).await?);
                    }
                    (note, replaced)
                }
            };
            replaced.retain(|m: &Memory| {
                m.stale.as_ref().and_then(|s| s.replaced_by.as_ref()) == Some(&note.id)
            });
            for id in replaces
                .iter()
                .filter(|id| !replaced.iter().any(|m| &m.id == *id))
            {
                eprintln!(
                    "{}",
                    style::amber(&format!(
                        "{id} is still current: this fctl-agent or control plane replaces one note at a time. Mark it stale: fctl memory stale {id} --reason \"Replaced by {}\"",
                        note.id
                    ))
                );
            }
            if json {
                // `replaced` stays one note, as before notes replaced several.
                let mut replaced = replaced.into_iter();
                return style::print_json(&serde_json::json!({
                    "memory": note,
                    "replaced": replaced.next(),
                    "also_replaced": replaced.collect::<Vec<_>>(),
                }));
            }
            let done = if note.session.is_some() {
                "Remembered"
            } else {
                "Added"
            };
            println!(
                "{} {done} {}",
                style::accent("✓"),
                style::bold(note.id.as_str())
            );
            print_text(&note.text);
            for old in replaced {
                println!(
                    "{} {} is stale now",
                    style::dim("·"),
                    style::bold(old.id.as_str())
                );
            }
        }
        MemoryCmd::Duplicate { id, of } => {
            let id = memory_id(&id)?;
            let of = memory_id(&of)?;
            if id == of {
                bail!("A note can't be a duplicate of itself");
            }
            // The reason too, for fctl-agents and control planes from before
            // duplicates, which mark the note stale with it alone.
            let reason = memory::duplicate_reason(&of);
            let note = match &keeper {
                Keeper::Agent {
                    socket, session, ..
                } => {
                    let mark = AgentCall::MarkStale {
                        id,
                        reason,
                        duplicate_of: Some(of),
                    };
                    match call(socket, session, mark).await? {
                        AgentAnswer::Memory { memory, .. } => memory,
                        _ => bail!("fctl-agent answered another request"),
                    }
                }
                Keeper::Person { api, .. } => {
                    api.update_memory(
                        id.as_str(),
                        &UpdateMemoryRequest {
                            stale: Some(true),
                            reason: Some(reason),
                            duplicate_of: Some(of),
                            ..Default::default()
                        },
                    )
                    .await?
                }
            };
            if json {
                return style::print_json(&note);
            }
            match note.stale.as_ref().and_then(|s| s.duplicate_of.as_ref()) {
                Some(of) => println!(
                    "{} {} is a duplicate of {}: agents no longer start with it",
                    style::accent("✓"),
                    style::bold(note.id.as_str()),
                    style::bold(of.as_str())
                ),
                None => println!(
                    "{} {} is stale: agents no longer start with it",
                    style::accent("✓"),
                    style::bold(note.id.as_str())
                ),
            }
        }
        MemoryCmd::Stale { id, reason } => {
            let id = memory_id(&id)?;
            let note = match &keeper {
                Keeper::Agent {
                    socket, session, ..
                } => {
                    let Some(reason) = reason.filter(|r| !r.trim().is_empty()) else {
                        bail!("Say why the note is stale: --reason \"…\"");
                    };
                    let mark = AgentCall::MarkStale {
                        id,
                        reason,
                        duplicate_of: None,
                    };
                    match call(socket, session, mark).await? {
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

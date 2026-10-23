//! `fctl ticket`: in the sandbox of a session whose project tracks its work
//! in a ticket tracker that agents reach through fctl (Linear, or one
//! repository's GitHub issues), the agent reads and writes the project's
//! tickets. fctl-agent hands each request to the control plane, which holds
//! the tracker's credentials and keeps it to the tickets the project chose.
//! The same commands work whatever the tracker.

use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::tickets::{
    StateFilter, Ticket, TicketAnswer, TicketComment, TicketLink, TicketRequest, TicketState,
};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args)]
pub struct TicketArgs {
    #[command(subcommand)]
    pub cmd: Option<TicketCmd>,
}

#[derive(Subcommand)]
pub enum TicketCmd {
    /// List tickets, most recently changed first (open ones unless asked).
    #[command(visible_alias = "ls")]
    List {
        /// Done and canceled ones instead.
        #[arg(long, conflicts_with = "all")]
        closed: bool,
        /// Open and closed ones.
        #[arg(long)]
        all: bool,
        /// Words their title or description has.
        #[arg(long, value_name = "WORDS")]
        search: Option<String>,
        /// Only those with this label.
        #[arg(long)]
        label: Option<String>,
        /// At most this many (default 20, at most 50).
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Show a ticket, with its description, sub-tickets and comments.
    #[command(visible_alias = "show")]
    View {
        /// Its key: ENG-123, #12, acme/web#12, or its link.
        key: String,
    },
    /// File a new ticket where the project files them.
    #[command(
        after_help = "Examples:\n  fctl ticket create --title \"Retry failed uploads\" --description \"Uploads that time out are lost.\"\n  \
                            fctl ticket create --title \"Add a test for retries\" --parent ENG-123 --label Tests"
    )]
    Create {
        #[arg(long)]
        title: String,
        /// Markdown.
        #[arg(long, conflicts_with = "description_file")]
        description: Option<String>,
        /// The description, from a file (`-` reads it from stdin).
        #[arg(long, value_name = "FILE")]
        description_file: Option<PathBuf>,
        /// Make it a sub-ticket of this one.
        #[arg(long, value_name = "KEY")]
        parent: Option<String>,
        /// Labels, by name (repeat it for more).
        #[arg(long = "label", value_name = "NAME")]
        labels: Vec<String>,
        /// The state it starts in, by name (Linear).
        #[arg(long)]
        state: Option<String>,
        /// Urgent, High, Medium or Low (Linear).
        #[arg(long)]
        priority: Option<String>,
        /// The repository to file it in, for GitHub issues of several
        /// (owner/name).
        #[arg(long = "in", value_name = "OWNER/NAME")]
        place: Option<String>,
    },
    /// Comment on a ticket (Markdown; or pipe it in).
    #[command(
        after_help = "Examples:\n  fctl ticket comment ENG-123 \"The fix is in acme/web#45; it needs a migration first.\"\n  \
                            fctl ticket comment ENG-123 --reply-to 2f6c… \"Yes: both use the old API.\"\n  \
                            fctl ticket comment ENG-123 --file notes.md"
    )]
    Comment {
        key: String,
        /// The comment.
        text: Vec<String>,
        /// The comment, from a file.
        #[arg(long, value_name = "FILE", conflicts_with = "text")]
        file: Option<PathBuf>,
        /// Answer this comment, in trackers that thread them (its id, which
        /// `fctl ticket view` shows).
        #[arg(long, value_name = "ID")]
        reply_to: Option<String>,
    },
    /// Change a ticket: what you name changes, the rest stays.
    #[command(
        after_help = "Examples:\n  fctl ticket update ENG-123 --state \"In Review\"\n  \
                            fctl ticket update ENG-123 --add-label Bug --remove-label Triage\n  \
                            fctl ticket update '#12' --state closed"
    )]
    Update {
        key: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, conflicts_with = "description_file")]
        description: Option<String>,
        /// The description, from a file (`-` reads it from stdin).
        #[arg(long, value_name = "FILE")]
        description_file: Option<PathBuf>,
        /// A state by its name (`fctl ticket states` lists them).
        #[arg(long)]
        state: Option<String>,
        #[arg(long = "add-label", value_name = "NAME")]
        add_labels: Vec<String>,
        #[arg(long = "remove-label", value_name = "NAME")]
        remove_labels: Vec<String>,
        /// Its parent, by key, or `none`.
        #[arg(long, value_name = "KEY")]
        parent: Option<String>,
        /// Urgent, High, Medium, Low or None (Linear).
        #[arg(long)]
        priority: Option<String>,
    },
    /// The states tickets can be in, in order.
    States,
    /// The labels tickets can have.
    Labels,
}

pub async fn run(args: TicketArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl ticket reads and writes a project's tickets in the sandbox of an fctl session, where agents work. Elsewhere, use the tracker itself."
        );
    };
    let request = match args.cmd {
        None => TicketRequest::About,
        Some(TicketCmd::List {
            closed,
            all,
            search,
            label,
            limit,
        }) => TicketRequest::List {
            state: if all {
                StateFilter::All
            } else if closed {
                StateFilter::Closed
            } else {
                StateFilter::Open
            },
            query: search,
            label,
            limit,
        },
        Some(TicketCmd::View { key }) => TicketRequest::View { key },
        Some(TicketCmd::Create {
            title,
            description,
            description_file,
            parent,
            labels,
            state,
            priority,
            place,
        }) => TicketRequest::Create {
            title,
            description: text_or_file(description, description_file)?,
            parent,
            labels,
            state,
            priority,
            place,
        },
        Some(TicketCmd::Comment {
            key,
            text,
            file,
            reply_to,
        }) => {
            let body = match (file, text.is_empty()) {
                (Some(f), _) => read_file(&f)?,
                (None, false) => text.join(" "),
                (None, true) if !std::io::stdin().is_terminal() => read_file(&PathBuf::from("-"))?,
                (None, true) => bail!("Write the comment, or pipe it in"),
            };
            TicketRequest::Comment {
                key,
                body,
                reply_to,
            }
        }
        Some(TicketCmd::Update {
            key,
            title,
            description,
            description_file,
            state,
            add_labels,
            remove_labels,
            parent,
            priority,
        }) => TicketRequest::Update {
            key,
            title,
            description: text_or_file(description, description_file)?,
            state,
            add_labels,
            remove_labels,
            parent,
            priority,
        },
        Some(TicketCmd::States) => TicketRequest::States,
        Some(TicketCmd::Labels) => TicketRequest::Labels,
    };
    let answer = ask(&socket, &session, &request).await?;
    if json {
        return style::print_json(&answer);
    }
    print(&answer);
    Ok(())
}

fn text_or_file(text: Option<String>, file: Option<PathBuf>) -> Result<Option<String>> {
    match (text, file) {
        (Some(t), _) => Ok(Some(t)),
        (None, Some(f)) => Ok(Some(read_file(&f)?)),
        (None, None) => Ok(None),
    }
}

fn read_file(path: &PathBuf) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .context("reading stdin")?;
        return Ok(s);
    }
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Asks the control plane, through fctl-agent and the host.
async fn ask(
    socket: &std::path::Path,
    session: &str,
    request: &TicketRequest,
) -> Result<TicketAnswer> {
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session.to_owned())),
        call: AgentCall::Ticket {
            request: serde_json::to_value(request)?,
        },
    };
    // fctl-agent waits up to two minutes for the control plane.
    let reply = tokio::time::timeout(
        Duration::from_secs(180),
        super::sessions::ask_agent(socket, &request),
    )
    .await
    .context("fctl-agent did not answer in time")??;
    match reply {
        SessionReply::Answer {
            answer: AgentAnswer::Ticket { answer },
        } => serde_json::from_value(answer)
            .context("the control plane answered with something this fctl doesn't know"),
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!("fctl-agent answered another request"),
    }
}

/// Text from the tracker, which anyone who names a label or a state there
/// writes: without the escape sequences it could hold.
fn safe(text: &str) -> std::borrow::Cow<'_, str> {
    style::terminal_safe(text)
}

fn print(answer: &TicketAnswer) {
    match answer {
        TicketAnswer::About { about } => {
            println!("{}", style::bold(&safe(&about.scope)));
            if let Some(f) = &about.files_in {
                println!("{}", style::dim(&format!("New tickets go to {}.", safe(f))));
            }
            for t in &about.tickets {
                println!(
                    "This session works on {} {}",
                    style::bold(&safe(&t.key)),
                    safe(t.title.as_deref().unwrap_or_default())
                );
                println!("  {}", style::dim(&safe(&t.url)));
            }
        }
        TicketAnswer::Tickets { tickets, more } => {
            if tickets.is_empty() {
                println!("{}", style::dim("No tickets match."));
            }
            for t in tickets {
                println!("{}", line(t));
            }
            if *more {
                println!(
                    "{}",
                    style::dim("More match: narrow it down, or ask for more with --limit.")
                );
            }
        }
        TicketAnswer::Ticket { ticket } => print_ticket(ticket),
        TicketAnswer::Comment { ticket, comment } => {
            println!(
                "{} Commented on {}{}",
                style::accent("✓"),
                style::bold(&safe(&ticket.key)),
                comment
                    .url
                    .as_deref()
                    .map(|u| format!(": {}", safe(u)))
                    .unwrap_or_default()
            );
        }
        TicketAnswer::States { states } => {
            for s in states {
                println!("{}  {}", safe(&s.name), style::dim(s.kind.as_str()));
            }
        }
        TicketAnswer::Labels { labels } => {
            for l in labels {
                println!("{}", safe(l));
            }
        }
    }
}

fn state(s: &TicketState) -> String {
    match s.kind {
        k if k.is_open() => safe(&s.name).into_owned(),
        _ => style::dim(&safe(&s.name)),
    }
}

/// A ticket in a list: its key, title, state and labels.
fn line(t: &Ticket) -> String {
    let mut extra = Vec::new();
    if let Some(s) = &t.state {
        extra.push(state(s));
    }
    if let Some(p) = &t.priority {
        extra.push(safe(p).into_owned());
    }
    if !t.labels.is_empty() {
        extra.push(safe(&t.labels.join(", ")).into_owned());
    }
    format!(
        "{}  {}  {}",
        style::bold(&safe(&t.key)),
        safe(&t.title),
        style::dim(&extra.join(" · "))
    )
}

fn link(l: &TicketLink) -> String {
    format!(
        "{} {}{}",
        style::bold(&safe(&l.key)),
        safe(&l.title),
        l.state
            .as_ref()
            .map(|s| format!(" ({})", safe(&s.name)))
            .unwrap_or_default()
    )
}

fn print_ticket(t: &Ticket) {
    println!("{}  {}", style::bold(&safe(&t.key)), safe(&t.title));
    let mut facts = Vec::new();
    if let Some(s) = &t.state {
        facts.push(state(s));
    }
    if let Some(k) = &t.kind {
        facts.push(safe(k).into_owned());
    }
    if let Some(p) = &t.priority {
        facts.push(safe(p).into_owned());
    }
    if !t.labels.is_empty() {
        facts.push(safe(&t.labels.join(", ")).into_owned());
    }
    if let Some(a) = &t.assignee {
        facts.push(format!("assigned to {}", safe(&a.name)));
    }
    if let Some(d) = &t.delegate {
        facts.push(format!("delegated to {}", safe(&d.name)));
    }
    if !facts.is_empty() {
        println!("{}", facts.join(" · "));
    }
    let mut place = Vec::new();
    if let Some(p) = &t.place {
        place.push(safe(p).into_owned());
    }
    place.push(safe(&t.url).into_owned());
    println!("{}", style::dim(&place.join(" · ")));
    for f in &t.fields {
        println!(
            "{} {}",
            style::dim(&format!("{}:", safe(&f.name))),
            safe(&f.value)
        );
    }
    if let Some(p) = &t.parent {
        println!("{} {}", style::dim("Parent:"), link(p));
    }
    if !t.children.is_empty() {
        println!("{}", style::dim("Sub-tickets:"));
        for c in &t.children {
            println!("  {}", link(c));
        }
    }
    println!();
    match t
        .description
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) => println!("{}", safe(d)),
        None => println!("{}", style::dim("No description.")),
    }
    if !t.comments.is_empty() || t.earlier_comments > 0 {
        println!();
        let earlier = if t.earlier_comments > 0 {
            format!(" ({} earlier not shown)", t.earlier_comments)
        } else {
            String::new()
        };
        println!("{}", style::bold(&format!("Comments{earlier}")));
        for c in &t.comments {
            print_comment(c);
        }
    }
}

fn print_comment(c: &TicketComment) {
    let who = c
        .author
        .as_ref()
        .map(|a| {
            if a.app {
                format!("{} (app)", safe(&a.name))
            } else {
                safe(&a.name).into_owned()
            }
        })
        .unwrap_or_else(|| "Someone".into());
    let mut head = vec![who];
    if let Some(at) = c.created_at {
        head.push(style::ago(at));
    }
    head.push(format!("id {}", safe(&c.id)));
    if c.parent.is_some() {
        head.push("a reply".into());
    }
    println!();
    println!("{}", style::dim(&head.join(" · ")));
    for l in c.body.lines() {
        println!("  {}", safe(l));
    }
}

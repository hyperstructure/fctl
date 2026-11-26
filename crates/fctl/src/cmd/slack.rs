//! `fctl slack`: in the sandbox of a session, the agent reads the Slack
//! thread its session started from, and the channels its workspace made
//! readable, and posts in its own thread. fctl-agent hands each request to
//! the control plane, which holds the Slack app's token and keeps each call
//! to what the session may reach. People connect Slack with `fctl
//! workspaces slack` and link their account with `fctl account slack`.

use crate::style;
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::slack::{SlackAnswer, SlackMessageView, SlackPlace, SlackRequest};
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Args)]
pub struct SlackArgs {
    #[command(subcommand)]
    pub cmd: Option<SlackCmd>,
}

#[derive(Subcommand)]
pub enum SlackCmd {
    /// Read the thread this session started from: what's new since you
    /// last read it.
    Thread {
        /// Every message of the thread, not only what's new.
        #[arg(long)]
        all: bool,
    },
    /// Read a message and its thread, by its link (Copy link in Slack), in
    /// a readable channel or this session's own thread.
    Read {
        /// https://….slack.com/archives/C…/p…
        link: String,
    },
    /// Read a readable channel's messages, oldest first.
    History {
        /// The channel: deploys, or its id (a `#` needs quotes in a shell).
        channel: String,
        /// Since how long ago: 30m, 6h, 2d (the latest otherwise).
        #[arg(long, value_name = "DURATION")]
        since: Option<String>,
        /// At most this many (default 50, at most 200).
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Find messages in the readable channels, newest first.
    Search {
        /// The words to look for.
        #[arg(required = true, value_name = "WORDS")]
        query: Vec<String>,
        /// In one channel only.
        #[arg(long)]
        channel: Option<String>,
        /// At most this many (default 20, at most 50).
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Post in this session's thread now, as fctl, for a quick word while
    /// you work. Your turn's last message goes there by itself.
    Reply {
        /// What to post, in Markdown (or pipe it in, or use --file).
        text: Vec<String>,
        /// What to post, from a file (`-` reads stdin).
        #[arg(long, value_name = "FILE", conflicts_with = "text")]
        file: Option<PathBuf>,
    },
}

pub async fn run(args: SlackArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl slack reads and posts in Slack in the sandbox of an fctl session, where agents work. People connect Slack with `fctl workspaces slack` and link their account with `fctl account slack`."
        );
    };
    let request = match args.cmd {
        None => SlackRequest::About,
        Some(SlackCmd::Thread { all }) => SlackRequest::Thread { all },
        Some(SlackCmd::Read { link }) => SlackRequest::Read { link },
        Some(SlackCmd::History {
            channel,
            since,
            limit,
        }) => SlackRequest::History {
            channel,
            since: match since {
                Some(s) => Some(
                    chrono_now_ms()
                        - super::schedules::duration_ms(&s)
                            .filter(|ms| *ms > 0)
                            .with_context(|| {
                                format!(
                                    "{s:?} isn't a duration fctl reads: give one like 30m, 6h or 2d"
                                )
                            })?,
                ),
                None => None,
            },
            limit,
        },
        Some(SlackCmd::Search {
            query,
            channel,
            limit,
        }) => SlackRequest::Search {
            query: query.join(" "),
            channel,
            limit,
        },
        Some(SlackCmd::Reply { text, file }) => {
            let text = match (file, text.is_empty()) {
                (Some(f), _) => read_file(&f)?,
                (None, false) => text.join(" "),
                (None, true) if !std::io::stdin().is_terminal() => read_file(&PathBuf::from("-"))?,
                (None, true) => bail!("Write what to post, or pipe it in"),
            };
            SlackRequest::Reply { text }
        }
    };
    let answer = ask(&socket, &session, &request).await?;
    if json {
        return style::print_json(&answer);
    }
    print(&answer);
    Ok(())
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
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
    request: &SlackRequest,
) -> Result<SlackAnswer> {
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session.to_owned())),
        call: AgentCall::Slack {
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
            answer: AgentAnswer::Slack { answer },
        } => serde_json::from_value(answer)
            .context("the control plane answered with something this fctl doesn't know"),
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!("fctl-agent answered another request"),
    }
}

/// Text from Slack, which anyone there writes: without the escape
/// sequences it could hold.
fn safe(text: &str) -> std::borrow::Cow<'_, str> {
    style::terminal_safe(text)
}

fn print(answer: &SlackAnswer) {
    match answer {
        SlackAnswer::About { about } => {
            match &about.thread {
                Some(t) => {
                    let place = match (t.place, t.channel_name.as_deref()) {
                        (SlackPlace::Dm, _) => "direct messages with fctl".to_owned(),
                        (_, Some(c)) => format!("#{c}"),
                        _ => "a Slack conversation".to_owned(),
                    };
                    println!(
                        "This session started in Slack, in a thread of {}{}",
                        style::bold(&safe(&place)),
                        about
                            .team_name
                            .as_deref()
                            .map(|n| format!(" ({})", safe(n)))
                            .unwrap_or_default()
                    );
                    println!("  {}", style::dim(&safe(&t.url)));
                }
                None => println!("This session didn't start in Slack."),
            }
            if about.readable.is_empty() {
                println!(
                    "{}",
                    style::dim("No channel is readable for this session's agents.")
                );
            } else {
                let names: Vec<String> = about
                    .readable
                    .iter()
                    .map(|c| format!("#{}", c.name))
                    .collect();
                println!("Readable channels: {}", safe(&names.join(", ")));
            }
            println!(
                "{}",
                style::dim(&format!(
                    "Search: {}. Posting in the thread: {}.",
                    if about.search { "yes" } else { "no" },
                    if about.reply { "yes" } else { "no" }
                ))
            );
        }
        SlackAnswer::Messages {
            channel,
            messages,
            more,
            note,
        } => {
            if let Some(c) = channel {
                println!("{}", style::bold(&format!("#{}", safe(&c.name))));
            }
            if messages.is_empty() && note.is_none() {
                println!("{}", style::dim("No messages."));
            }
            for m in messages {
                print_message(m);
            }
            if *more {
                println!(
                    "{}",
                    style::dim(
                        "More came before these, or match: narrow it down, or ask for more with --limit."
                    )
                );
            }
            if let Some(n) = note {
                println!("{}", style::dim(&safe(n)));
            }
        }
        SlackAnswer::Posted { url, .. } => {
            println!(
                "{} Posted in the thread{}",
                style::accent("✓"),
                url.as_deref()
                    .map(|u| format!(": {}", safe(u)))
                    .unwrap_or_default()
            );
        }
    }
}

fn print_message(m: &SlackMessageView) {
    let mut head = format!("{} · {}", safe(&m.by.name), style::ago(m.at));
    if let Some(c) = &m.channel_name {
        head.push_str(&format!(" · #{}", safe(c)));
    }
    if m.delivered {
        head.push_str(" · passed on to you as their message");
    } else if m.new {
        head.push_str(" · new");
    }
    if m.by.bot {
        head.push_str(" · an app");
    }
    println!("{}", style::bold(&head));
    for line in m.text.lines() {
        println!("  {}", safe(line));
    }
    if !m.files.is_empty() {
        println!(
            "  {}",
            style::dim(&format!("Files: {}", safe(&m.files.join(", "))))
        );
    }
    if let Some(n) = m.replies {
        println!("  {}", style::dim(&format!("{n} replies")));
    }
    if let Some(u) = &m.url {
        println!("  {}", style::dim(&safe(u)));
    }
    println!();
}

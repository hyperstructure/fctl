//! `fctl ask`: an agent asks the people who follow its session for their
//! input, in its sandbox, through fctl-agent (`AgentCall::Ask`). People see
//! the question where they follow the session, with answers to click, and
//! hear of it as the turn ends; a person's next message answers it.

use crate::style;
use anyhow::{Context, Result, bail};
use clap::Args;
use fctl_proto::SessionId;
use fctl_proto::agent::{SessionReply, SessionRequest};
use fctl_proto::api::{AgentAnswer, AgentCall};
use fctl_proto::session::SessionQuestion;
use std::time::Duration;

#[derive(Args)]
pub struct AskArgs {
    /// The question, in a sentence or two: what you need to know to go on.
    /// Put the details in your message.
    #[arg(required_unless_present = "withdraw", value_name = "QUESTION")]
    pub question: Vec<String>,
    /// An answer people can give with a click (repeatable, up to 6), sent
    /// as their reply as it is. They can always write their own.
    #[arg(short, long = "option", value_name = "ANSWER")]
    pub options: Vec<String>,
    /// Take back the question you asked: you found the answer yourself.
    #[arg(long, conflicts_with_all = ["question", "options"])]
    pub withdraw: bool,
}

pub async fn run(args: AskArgs, json: bool) -> Result<()> {
    let Some((socket, session)) = super::sessions::this_sandbox(None) else {
        bail!(
            "fctl ask asks the people who follow the session this sandbox runs, for its agent: run it in a session's sandbox. To answer a question, send the session a message (fctl send)."
        );
    };
    let question = if args.withdraw {
        String::new()
    } else {
        args.question.join(" ")
    };
    if !args.withdraw && question.trim().is_empty() {
        bail!("Ask something: fctl ask \"Should prices round to whole kroner?\"");
    }
    let request = SessionRequest::Call {
        session_id: Some(SessionId(session)),
        call: AgentCall::Ask {
            question,
            options: args.options,
        },
    };
    // fctl-agent waits up to two minutes for the control plane.
    let reply = tokio::time::timeout(
        Duration::from_secs(180),
        super::sessions::ask_agent(&socket, &request),
    )
    .await
    .context("fctl-agent did not answer in time")??;
    let asked = match reply {
        SessionReply::Answer {
            answer: AgentAnswer::Asked { question },
        } => question,
        SessionReply::Error { error } => bail!("{}", error.message),
        _ => bail!(
            "This sandbox's fctl-agent can't ask yet: its runner needs a newer fctl (`fctl self update` there). Ask in your message instead, and end your turn."
        ),
    };
    if json {
        return style::print_json(&serde_json::json!({ "question": asked }));
    }
    match asked {
        Some(q) => print_asked(&q),
        None => println!("{} Took the question back", style::accent("✓")),
    }
    Ok(())
}

fn print_asked(q: &SessionQuestion) {
    println!(
        "{} Asked: {}",
        style::accent("✓"),
        style::terminal_safe(&q.text)
    );
    for o in &q.options {
        println!("  · {}", style::terminal_safe(o));
    }
    println!(
        "{}",
        style::dim(
            "People see it where they follow the session, and hear of it as your turn ends: end it now. Their answer comes as a message."
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Fctl {
        #[command(flatten)]
        ask: AskArgs,
    }

    fn parse(args: &[&str]) -> Option<AskArgs> {
        Fctl::try_parse_from(["fctl"].iter().chain(args).copied())
            .ok()
            .map(|f| f.ask)
    }

    #[test]
    fn asks_with_answers_to_click() {
        let Some(a) = parse(&[
            "Round",
            "prices?",
            "--option",
            "Round to whole kroner",
            "-o",
            "Keep øre",
        ]) else {
            panic!("parses");
        };
        assert_eq!(a.question.join(" "), "Round prices?");
        assert_eq!(a.options, ["Round to whole kroner", "Keep øre"]);
        assert!(!a.withdraw);
        // Taking it back takes nothing else, and needs no question.
        assert!(parse(&["--withdraw"]).is_some_and(|a| a.withdraw));
        assert!(parse(&["--withdraw", "Why?"]).is_none());
        assert!(parse(&[]).is_none());
    }
}

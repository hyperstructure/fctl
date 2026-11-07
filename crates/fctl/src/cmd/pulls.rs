//! `fctl pulls`: the pull requests agents opened or worked on in your
//! projects, each once, with the sessions that list it: the web app's pull
//! requests page, in a terminal.

use crate::{config, style};
use anyhow::Result;
use clap::{Args, ValueEnum};
use fctl_domain::pulls::{Listed, SessionRef, listed};

#[derive(Args)]
pub struct PullsArgs {
    /// Only this project's (slug or id).
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only this workspace's (slug or id).
    #[arg(short, long, conflicts_with = "project")]
    pub workspace: Option<String>,
    /// Which ones: open (open, drafts and those in a merge queue), merged,
    /// closed or all.
    #[arg(long, value_enum, default_value = "open")]
    pub state: State,
    /// Only those of sessions you started.
    #[arg(long)]
    pub mine: bool,
    /// Sessions to look through, newest first (at most 200).
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=200))]
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum State {
    Open,
    Merged,
    Closed,
    All,
}

impl State {
    /// What the API's `pulls` takes, and the states `listed` keeps.
    fn query(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Open => ("open", &["open", "draft", "queued"]),
            Self::Merged => ("merged", &["merged"]),
            Self::Closed => ("closed", &["closed"]),
            Self::All => ("any", &[]),
        }
    }
}

pub async fn run(server: Option<&str>, args: PullsArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let project = match &args.workspace {
        Some(_) => None,
        None => args.project,
    };
    // Every session that lists one, whatever its state: each pull request
    // takes the state the session that heard of it last says, which one
    // that heard of it earlier may not.
    let (_, states) = args.state.query();
    let list = api
        .sessions_with_pulls(
            args.workspace.as_deref(),
            project.as_deref(),
            "any",
            args.mine,
            args.limit,
        )
        .await?;
    let found = listed(&list.sessions, states);
    // More sessions than it looked at: older pull requests aren't in it.
    let more = list.next_before.is_some();
    if json {
        return style::print_json(&serde_json::json!({
            "pull_requests": found,
            "next_before": list.next_before,
        }));
    }
    if more {
        eprintln!(
            "{}",
            style::dim(&format!(
                "From the newest {} sessions with pull requests: narrow it with -p or -w, or raise --limit (at most 200), for older ones.",
                args.limit
            ))
        );
    }
    if found.is_empty() {
        println!(
            "{}",
            match args.state {
                State::Open => "No open pull requests: agents open them as they finish their work.",
                State::Merged => "No merged pull requests.",
                State::Closed => "No closed pull requests.",
                State::All => "No pull requests yet: agents open them as they finish their work.",
            }
        );
        return Ok(());
    }
    let now = style::now_ms();
    for l in &found {
        print_line(l, now);
    }
    Ok(())
}

fn print_line(l: &Listed, now: i64) {
    let pr = &l.pull_request;
    let repo = pr
        .repo
        .clone()
        .or_else(|| fctl_domain::pulls::repo_of(&pr.url))
        .map(|r| r.full_name())
        .unwrap_or_default();
    let title = style::plain(pr.title.as_deref().unwrap_or_default());
    let sessions: Vec<String> = l.sessions.iter().map(session_ref).collect();
    println!(
        "{} {}",
        style::bold(&format!("{repo}#{}", pr.number)),
        truncate(&title, 60)
    );
    println!(
        "  {}{}",
        super::sessions::pull_request_state(pr, now).unwrap_or_else(|| style::dim("open")),
        style::dim(&format!(
            " · {} · {}",
            sessions.join(", "),
            style::terminal_safe(&pr.url)
        ))
    );
}

/// A session that lists it, as `fctl pulls` names it: "shop#231", and what
/// tells it apart: its agent waits for an answer, or it was archived.
fn session_ref(s: &SessionRef) -> String {
    let marks: Vec<&str> = [
        s.question.is_some().then_some("asks you"),
        s.archived.then_some("archived"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if marks.is_empty() {
        format!("{}#{}", s.project_slug, s.number)
    } else {
        format!("{}#{} ({})", s.project_slug, s.number, marks.join(", "))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Fctl {
        #[command(flatten)]
        pulls: PullsArgs,
    }

    #[test]
    fn open_ones_by_default_and_any_state_for_all() {
        let open = Fctl::try_parse_from(["fctl"]).map(|f| f.pulls.state.query());
        assert_eq!(open.ok().map(|q| q.0), Some("open"));
        let all = Fctl::try_parse_from(["fctl", "--state", "all"]).map(|f| f.pulls.state.query());
        assert_eq!(all.ok(), Some(("any", &[][..])));
        assert!(Fctl::try_parse_from(["fctl", "-p", "shop", "-w", "romanesco"]).is_err());
        assert!(Fctl::try_parse_from(["fctl", "--limit", "500"]).is_err());
    }

    #[test]
    fn sessions_say_when_their_agent_asks_or_they_were_archived() {
        let session = |archived: bool, question: Option<&str>| SessionRef {
            id: fctl_proto::SessionId("ses_1".into()),
            project_slug: "shop".into(),
            number: 231,
            title: "Add Danish".into(),
            status: fctl_proto::session::SessionStatus::Idle,
            created_by: fctl_proto::session::UserRef {
                id: fctl_proto::UserId("usr_1".into()),
                name: "Noor".into(),
                avatar_url: None,
            },
            archived,
            question: question.map(str::to_owned),
        };
        let refs: Vec<String> = [
            session(false, None),
            session(false, Some("Øre?")),
            session(true, None),
            session(true, Some("Øre?")),
        ]
        .iter()
        .map(session_ref)
        .collect();
        assert_eq!(
            refs,
            [
                "shop#231",
                "shop#231 (asks you)",
                "shop#231 (archived)",
                "shop#231 (asks you, archived)"
            ]
        );
    }
}

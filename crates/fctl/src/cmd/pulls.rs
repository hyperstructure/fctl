//! `fctl pulls`: the pull requests agents opened or worked on in your
//! projects, each once, with the sessions that list it: the web app's pull
//! requests page, in a terminal.

use crate::{config, style};
use anyhow::Result;
use clap::{Args, ValueEnum};
use fctl_domain::pulls::{Listed, listed};

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
    let (pulls, states) = args.state.query();
    let list = api
        .sessions_with_pulls(
            args.workspace.as_deref(),
            project.as_deref(),
            pulls,
            args.mine,
            args.limit,
        )
        .await?;
    let found = listed(&list.sessions, states);
    if json {
        return style::print_json(&serde_json::json!({ "pull_requests": found }));
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
    let sessions: Vec<String> = l
        .sessions
        .iter()
        .map(|s| format!("{}#{}", s.project_slug, s.number))
        .collect();
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
}

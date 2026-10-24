//! `fctl search`: the sessions whose title, description, people's messages
//! or agents' replies say something, and where.

use crate::{config, style};
use anyhow::Result;
use clap::Args;
use fctl_proto::api::{SearchMatch, SearchMatchKind, SearchResults};

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// What to look for: found as typed, whatever the case and accents.
    #[arg(required = true, value_name = "TEXT")]
    pub query: Vec<String>,
    /// Only this project's sessions (slug or id).
    #[arg(short, long)]
    pub project: Option<String>,
    /// Only this workspace's sessions (slug or id).
    #[arg(short, long, conflicts_with = "project")]
    pub workspace: Option<String>,
    /// How many sessions to show, at most 50.
    #[arg(long, default_value_t = 20)]
    pub limit: u32,
}

pub async fn run(server: Option<&str>, args: SearchArgs, json: bool) -> Result<()> {
    let api = config::api(server)?;
    let query = args.query.join(" ");
    let found = api
        .search(
            &query,
            args.workspace.as_deref(),
            args.project.as_deref(),
            args.limit,
        )
        .await?;
    if json {
        return style::print_json(&found);
    }
    print!("{}", render(&found, &query, style::now_ms()));
    Ok(())
}

/// What `fctl search` prints: each session on a line, as `fctl sessions`
/// has it, its title with the matches in it, then where else it matched,
/// one line each, and how many more matches it has.
fn render(found: &SearchResults, query: &str, now: i64) -> String {
    let mut out = String::new();
    if found.sessions.is_empty() {
        out.push_str(&format!(
            "No sessions say “{}”.\n",
            style::terminal_safe(query)
        ));
        return out;
    }
    for (i, hit) in found.sessions.iter().enumerate() {
        let s = &hit.session;
        if i > 0 {
            out.push('\n');
        }
        let reference = format!("{}#{}", s.project_slug, s.number);
        let title = hit
            .matches
            .iter()
            .find(|m| m.kind == SearchMatchKind::Title)
            .map_or_else(
                || style::terminal_safe(&s.title).into_owned(),
                |m| marked(&m.text, &m.ranges),
            );
        let mut facts = vec![style::status(s.status)];
        if s.archived {
            facts.push(style::dim("archived"));
        }
        facts.push(style::dim(&style::ago_from(
            s.last_activity_at.unwrap_or(s.created_at),
            now,
        )));
        out.push_str(&format!(
            "{}  {title}  {}\n",
            style::bold(&reference),
            facts.join("  ")
        ));
        let shown: Vec<&SearchMatch> = hit
            .matches
            .iter()
            .filter(|m| m.kind != SearchMatchKind::Title)
            .collect();
        for m in &shown {
            out.push_str(&format!(
                "  {} {}\n",
                style::dim(&format!("{}:", who(m))),
                marked(&m.text, &m.ranges)
            ));
        }
        let listed = hit.matches.len() as u32;
        if hit.count > listed {
            let more = hit.count - listed;
            out.push_str(&format!(
                "  {}\n",
                style::dim(&format!(
                    "+{}{}",
                    style::count(more, "more match", "more matches"),
                    if hit.count >= 100 { " or so" } else { "" }
                ))
            ));
        }
    }
    if found.more {
        out.push_str(&format!(
            "\n{}\n",
            style::dim("More sessions match: narrow it down, or show more with --limit.")
        ));
    }
    out
}

/// Who said what matched.
fn who(m: &SearchMatch) -> String {
    match m.kind {
        SearchMatchKind::Title => "title".into(),
        SearchMatchKind::Description => "description".into(),
        SearchMatchKind::Reply => "agent".into(),
        SearchMatchKind::Message => m
            .author
            .as_ref()
            .map_or_else(|| "fctl".into(), |a| a.name.clone()),
    }
}

/// `text` with what matched in it brought out (`ranges`, in UTF-16 code
/// units), safe for a terminal.
fn marked(text: &str, ranges: &[[u32; 2]]) -> String {
    let mut out = String::new();
    let mut part = String::new();
    let mut inside = false;
    let mut at = 0u32;
    let flush = |part: &mut String, inside: bool, out: &mut String| {
        if part.is_empty() {
            return;
        }
        let safe = style::terminal_safe(part).into_owned();
        out.push_str(&if inside {
            style::bold(&style::accent(&safe))
        } else {
            safe
        });
        part.clear();
    };
    for c in text.chars() {
        let hit = ranges.iter().any(|&[a, b]| at >= a && at < b);
        if hit != inside {
            flush(&mut part, inside, &mut out);
            inside = hit;
        }
        part.push(c);
        at += c.len_utf16() as u32;
    }
    flush(&mut part, inside, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::api::SessionMatches;
    use fctl_proto::session::{Session, SessionStatus, UserRef};

    fn session(n: u32, title: &str) -> Session {
        serde_json::from_value(serde_json::json!({
            "id": format!("ses_{n}"),
            "project_id": "prj_1",
            "project_slug": "web",
            "number": n,
            "title": title,
            "status": "idle",
            "created_by": {"id": "usr_1", "name": "Ana"},
            "created_at": 0,
            "updated_at": 0,
            "last_activity_at": 3_600_000,
            "stats": {},
            "archived": false,
        }))
        .unwrap_or_else(|e| panic!("{e}"))
    }

    fn hit(
        kind: SearchMatchKind,
        author: Option<&str>,
        text: &str,
        ranges: Vec<[u32; 2]>,
    ) -> SearchMatch {
        SearchMatch {
            kind,
            seq: None,
            ts: None,
            author: author.map(|name| UserRef {
                id: fctl_proto::UserId("usr_1".into()),
                name: name.into(),
                avatar_url: None,
            }),
            text: text.into(),
            ranges,
        }
    }

    #[test]
    fn lists_sessions_with_where_they_matched() {
        let found = SearchResults {
            sessions: vec![SessionMatches {
                session: session(12, "Fix the login redirect"),
                matches: vec![
                    hit(
                        SearchMatchKind::Title,
                        None,
                        "Fix the login redirect",
                        vec![[8, 13]],
                    ),
                    hit(
                        SearchMatchKind::Message,
                        Some("Ana"),
                        "…the login loops…",
                        vec![[5, 10]],
                    ),
                    hit(
                        SearchMatchKind::Reply,
                        None,
                        "Fixed the login.",
                        vec![[10, 15]],
                    ),
                ],
                count: 5,
            }],
            more: true,
        };
        let text = render(&found, "login", 7_200_000);
        assert_eq!(
            text,
            "web#12  Fix the login redirect  ○ idle  1h ago\n  \
             Ana: …the login loops…\n  agent: Fixed the login.\n  +2 more matches\n\n\
             More sessions match: narrow it down, or show more with --limit.\n"
        );
        let mut s = session(3, "Other");
        s.status = SessionStatus::Stopped;
        s.archived = true;
        let found = SearchResults {
            sessions: vec![SessionMatches {
                session: s,
                matches: vec![hit(SearchMatchKind::Message, None, "continue", vec![])],
                count: 1,
            }],
            more: false,
        };
        assert_eq!(
            render(&found, "cont", 7_200_000),
            "web#3  Other  ○ stopped  archived  1h ago\n  fctl: continue\n"
        );
    }

    #[test]
    fn says_when_nothing_matches() {
        let found = SearchResults {
            sessions: vec![],
            more: false,
        };
        assert_eq!(render(&found, "zebra", 0), "No sessions say “zebra”.\n");
    }

    #[test]
    fn marks_matches_by_utf16_ranges() {
        // Without a terminal, matches are plain text.
        assert_eq!(marked("😀 bug\u{7}", &[[3, 6]]), "😀 bug");
    }
}

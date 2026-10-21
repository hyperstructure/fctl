//! A project's memory: notes that agents, and people, keep for the agents
//! of later sessions (`fctl memory`). Each session's agent starts with the
//! project's current notes, adds what a future agent should know, and marks
//! the notes that turned out wrong stale; people read them, change them and
//! delete them. How long notes are, how many a project keeps, which ones an
//! agent starts with, and how lists order them.

use crate::validate;
use fctl_proto::ApiError;
use fctl_proto::runner::MemorySpec;
use fctl_proto::session::Memory;

/// The longest note, in characters: a fact or two, not a document.
pub const MAX_CHARS: usize = 2_000;
/// The longest reason for marking a note stale.
pub const MAX_REASON_CHARS: usize = 500;
/// The most notes a project keeps, stale ones included.
pub const MAX_PER_PROJECT: u32 = 500;
/// How much of the notes' text an agent starts with, in bytes: about as
/// much as Claude Code loads of its own memory as it starts (25 KB).
pub const PROMPT_BYTES: usize = 24_000;
/// The most notes an agent starts with; `fctl memory` lists the others.
pub const PROMPT_NOTES: usize = 200;
/// What a note costs on top of its text where an agent reads it: its id,
/// where it came from and how old it is.
const NOTE_OVERHEAD: usize = 64;

/// A note's text: trimmed, not empty, at most [`MAX_CHARS`] characters,
/// without control characters but for line breaks and tabs.
pub fn text(input: &str) -> Result<String, ApiError> {
    let text = validate::text("text", input, MAX_CHARS)?;
    if text.is_empty() {
        return Err(ApiError::validation("text", "Write what to remember"));
    }
    Ok(text)
}

/// Why a note went stale: trimmed, at most [`MAX_REASON_CHARS`]
/// characters. People may give none; agents always say why.
pub fn reason(input: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(input) = input else {
        return Ok(None);
    };
    let reason = validate::text("reason", input, MAX_REASON_CHARS)?;
    Ok((!reason.is_empty()).then_some(reason))
}

/// What a note is about, on one line: its first line that has text, without
/// Markdown's heading and list marks, at most `max` characters (with an
/// ellipsis when cut).
pub fn headline(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches('#')
                .trim_start_matches(['-', '*', '>'])
                .trim()
        })
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Lists put the current notes first, newest first, then the stale ones,
/// most recently stale first.
pub fn sort(notes: &mut [Memory]) {
    notes.sort_by(|a, b| match (&a.stale, &b.stale) {
        (None, None) => b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(x), Some(y)) => y.at.cmp(&x.at).then(b.id.cmp(&a.id)),
    });
}

/// The notes an agent starts with: of the current ones, newest first, as
/// many as fit in [`PROMPT_BYTES`] and [`PROMPT_NOTES`], and how many more
/// there are. A note that doesn't fit ends the list, so that the newest are
/// all there.
pub fn for_prompt(mut current: Vec<Memory>) -> MemorySpec {
    current.retain(|m| m.stale.is_none());
    sort(&mut current);
    let total = current.len();
    let mut used = 0;
    let mut notes = Vec::new();
    for m in current {
        let cost = m.text.len() + NOTE_OVERHEAD;
        if notes.len() == PROMPT_NOTES || used + cost > PROMPT_BYTES {
            break;
        }
        used += cost;
        notes.push(m);
    }
    let more = (total - notes.len()).try_into().unwrap_or(u32::MAX);
    MemorySpec { notes, more }
}

/// How long ago something happened, for agents and the CLI: `today`,
/// `yesterday`, `3 days ago`, `2 months ago`, `a year ago`.
pub fn age(now: i64, at: i64) -> String {
    const DAY: i64 = 24 * 60 * 60 * 1000;
    let days = (now - at).max(0) / DAY;
    match days {
        0 => "today".to_owned(),
        1 => "yesterday".to_owned(),
        2..=44 => format!("{days} days ago"),
        45..=319 => format!("{} months ago", (days + 15) / 30),
        320..=547 => "a year ago".to_owned(),
        _ => format!("{} years ago", (days + 182) / 365),
    }
}

/// Where a note came from, in a few words: `session #12`, or the person
/// who wrote it.
pub fn source(m: &Memory) -> String {
    match (&m.session, &m.created_by) {
        (Some(s), _) => format!("session #{}", s.number),
        (None, Some(person)) => person.name.clone(),
        (None, None) => "a person".to_owned(),
    }
}

/// What the transcript says when an agent remembers something.
pub fn added_message(m: &Memory) -> String {
    format!("Remembered: {}", headline(&m.text, 140))
}

/// What the transcript says when an agent marks a note stale.
pub fn stale_message(m: &Memory) -> String {
    let what = headline(&m.text, 120);
    match m.stale.as_ref().and_then(|s| s.reason.as_deref()) {
        Some(reason) => format!("Marked a memory stale: {what} ({})", headline(reason, 120)),
        None => format!("Marked a memory stale: {what}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::session::{SessionRef, Staleness, UserRef};
    use fctl_proto::{MemoryId, ProjectId, SessionId, UserId};

    fn note(id: &str, text: &str, created_at: i64) -> Memory {
        Memory {
            id: MemoryId(id.into()),
            project_id: ProjectId("prj_1".into()),
            text: text.into(),
            session: None,
            created_by: None,
            created_at,
            edited_at: None,
            edited_by: None,
            stale: None,
        }
    }

    fn stale(mut m: Memory, at: i64) -> Memory {
        m.stale = Some(Staleness {
            at,
            reason: Some("The script moved to xtask".into()),
            session: None,
            by: None,
            replaced_by: None,
        });
        m
    }

    #[test]
    fn texts_are_trimmed_and_bounded() {
        assert_eq!(
            text("  Run npm ci first \n").ok().as_deref(),
            Some("Run npm ci first")
        );
        assert!(text(" \n ").is_err());
        assert!(text(&"x".repeat(MAX_CHARS)).is_ok());
        assert!(text(&"x".repeat(MAX_CHARS + 1)).is_err());
        assert!(text("a\u{7}b").is_err());
        assert!(text("Line one\n\n- and two").is_ok());
        assert_eq!(reason(None).ok(), Some(None));
        assert_eq!(reason(Some("  ")).ok(), Some(None));
        assert_eq!(reason(Some(" moved ")).ok(), Some(Some("moved".into())));
        assert!(reason(Some(&"x".repeat(MAX_REASON_CHARS + 1))).is_err());
    }

    #[test]
    fn headlines_take_the_first_line_with_text() {
        assert_eq!(
            headline("\n## Tests need npm ci\n\nRun it in web/ first.", 80),
            "Tests need npm ci"
        );
        assert_eq!(
            headline("- `cargo xtask check`   takes 4 minutes", 80),
            "`cargo xtask check` takes 4 minutes"
        );
        assert_eq!(headline("abcdefghij", 5), "abcd…");
        assert_eq!(headline("abcde", 5), "abcde");
        assert_eq!(headline("", 5), "");
    }

    #[test]
    fn lists_put_current_notes_first() {
        let mut notes = vec![
            stale(note("mem_a", "a", 1), 10),
            note("mem_b", "b", 2),
            stale(note("mem_c", "c", 3), 20),
            note("mem_d", "d", 4),
        ];
        sort(&mut notes);
        let ids: Vec<&str> = notes.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["mem_d", "mem_b", "mem_c", "mem_a"]);
    }

    #[test]
    fn agents_start_with_the_newest_notes_that_fit() {
        let notes: Vec<Memory> = (0..10)
            .map(|i| note(&format!("mem_{i}"), &"x".repeat(4_000), i))
            .chain([stale(note("mem_old", "stale", 100), 200)])
            .collect();
        let spec = for_prompt(notes);
        // Five notes of 4 KB fit in 24 KB; the stale one never counts.
        assert_eq!(spec.notes.len(), 5);
        assert_eq!(spec.more, 5);
        assert_eq!(spec.notes[0].id.as_str(), "mem_9");
        let many: Vec<Memory> = (0..300)
            .map(|i| note(&format!("mem_{i}"), "short", i))
            .collect();
        let spec = for_prompt(many);
        assert_eq!(spec.notes.len(), PROMPT_NOTES);
        assert_eq!(spec.more, 100);
        let spec = for_prompt(vec![]);
        assert!(spec.notes.is_empty() && spec.more == 0);
    }

    #[test]
    fn ages_read_as_people_say_them() {
        let day = 24 * 60 * 60 * 1000;
        assert_eq!(age(day / 2, 0), "today");
        assert_eq!(age(day, 0), "yesterday");
        assert_eq!(age(3 * day, 0), "3 days ago");
        assert_eq!(age(60 * day, 0), "2 months ago");
        assert_eq!(age(400 * day, 0), "a year ago");
        assert_eq!(age(800 * day, 0), "2 years ago");
        assert_eq!(age(0, day), "today");
    }

    #[test]
    fn notices_say_what_was_remembered_or_marked_stale() {
        let mut m = note("mem_1", "## Tests need npm ci\nin web/", 1);
        m.session = Some(SessionRef {
            id: SessionId("ses_1".into()),
            number: 12,
            title: "Fix it".into(),
        });
        assert_eq!(added_message(&m), "Remembered: Tests need npm ci");
        assert_eq!(source(&m), "session #12");
        let m = stale(m, 5);
        assert_eq!(
            stale_message(&m),
            "Marked a memory stale: Tests need npm ci (The script moved to xtask)"
        );
        let mut by_person = note("mem_2", "Deploys happen on Fridays", 1);
        by_person.created_by = Some(UserRef {
            id: UserId("usr_1".into()),
            name: "Ada".into(),
            avatar_url: None,
        });
        assert_eq!(source(&by_person), "Ada");
    }
}

//! A project's memory: notes that agents, and people, keep for the agents
//! of later sessions (`fctl memory`). Each session's agent starts with the
//! project's current notes, adds what a future agent should know, and marks
//! the notes that turned out wrong stale; people read them, change them and
//! delete them. How long notes are, how many a project keeps, which ones an
//! agent starts with, how lists order them, and which notes a note may
//! replace or repeat.

use crate::validate;
use fctl_proto::runner::MemorySpec;
use fctl_proto::session::Memory;
use fctl_proto::{ApiError, MemoryId};

/// The longest note, in characters: a fact or two, not a document.
pub const MAX_CHARS: usize = 2_000;
/// The longest reason for marking a note stale.
pub const MAX_REASON_CHARS: usize = 500;
/// The most notes a project keeps, stale ones included.
pub const MAX_PER_PROJECT: u32 = 500;
/// The most notes one note takes the place of: those it sums up.
pub const MAX_REPLACED: usize = 20;
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

/// The notes a new note takes the place of: `replaces`, then
/// `also_replaces`, each once, at most [`MAX_REPLACED`].
pub fn replaced(
    replaces: Option<&MemoryId>,
    also_replaces: &[MemoryId],
) -> Result<Vec<MemoryId>, ApiError> {
    let mut ids: Vec<MemoryId> = Vec::new();
    for id in replaces.into_iter().chain(also_replaces) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    if ids.len() > MAX_REPLACED {
        return Err(ApiError::validation(
            "replaces",
            format!("A note takes the place of at most {MAX_REPLACED} others"),
        ));
    }
    Ok(ids)
}

/// Why a duplicate is stale, unless someone says otherwise.
pub fn duplicate_reason(of: &MemoryId) -> String {
    format!("Duplicate of {of}")
}

/// Whether `note` may be marked a duplicate of `of`, a note of its project:
/// another one, and current, so that agents still start with what it says.
/// A stale `of` points at the note that took its place, or that it repeats.
pub fn check_duplicate(note: &Memory, of: &Memory) -> Result<(), ApiError> {
    if note.id == of.id {
        return Err(ApiError::validation(
            "duplicate_of",
            "A note can't be a duplicate of itself",
        ));
    }
    let Some(stale) = &of.stale else {
        return Ok(());
    };
    let instead = match (&stale.replaced_by, &stale.duplicate_of) {
        (Some(next), _) => format!(
            "{next} took its place, so mark {} a duplicate of that one",
            note.id
        ),
        (None, Some(first)) => format!(
            "it repeats {first}, so mark {} a duplicate of that one",
            note.id
        ),
        (None, None) => format!(
            "mark {} a duplicate of a current note, or stale with a reason",
            note.id
        ),
    };
    Err(ApiError::conflict(format!("{} is stale: {instead}", of.id)))
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

/// What the transcript says when an agent marks a note stale, or a
/// duplicate of another.
pub fn stale_message(m: &Memory) -> String {
    let what = headline(&m.text, 120);
    let Some(stale) = &m.stale else {
        return format!("Marked a memory stale: {what}");
    };
    match (&stale.duplicate_of, stale.reason.as_deref()) {
        (Some(of), _) => format!("Marked a memory a duplicate of {of}: {what}"),
        (None, Some(reason)) => {
            format!("Marked a memory stale: {what} ({})", headline(reason, 120))
        }
        (None, None) => format!("Marked a memory stale: {what}"),
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
            duplicate_of: None,
        });
        m
    }

    fn ids(ids: &[&str]) -> Vec<MemoryId> {
        ids.iter().map(|id| MemoryId((*id).into())).collect()
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
    fn a_note_replaces_each_note_once_and_a_few_at_most() {
        let first = MemoryId("mem_a".into());
        assert_eq!(
            replaced(Some(&first), &ids(&["mem_b", "mem_a", "mem_c", "mem_b"])).ok(),
            Some(ids(&["mem_a", "mem_b", "mem_c"]))
        );
        assert_eq!(replaced(None, &[]).ok(), Some(vec![]));
        let many: Vec<MemoryId> = (0..=MAX_REPLACED)
            .map(|i| MemoryId(format!("mem_{i}")))
            .collect();
        assert!(replaced(None, &many[..MAX_REPLACED]).is_ok());
        let err = replaced(None, &many).err();
        assert_eq!(
            err.as_ref().and_then(|e| e.field.as_deref()),
            Some("replaces")
        );
    }

    #[test]
    fn a_duplicate_repeats_another_current_note() {
        let original = note("mem_a", "Run npm ci first", 1);
        let copy = note("mem_b", "Run npm ci before the tests", 2);
        assert!(check_duplicate(&copy, &original).is_ok());
        let itself = check_duplicate(&copy, &copy).err();
        assert_eq!(
            itself.as_ref().and_then(|e| e.field.as_deref()),
            Some("duplicate_of")
        );
        // A stale note points at what agents start with instead.
        let mut gone = stale(original.clone(), 5);
        assert_eq!(
            check_duplicate(&copy, &gone).err().map(|e| e.message),
            Some(
                "mem_a is stale: mark mem_b a duplicate of a current note, or stale with a reason"
                    .into()
            )
        );
        if let Some(s) = gone.stale.as_mut() {
            s.duplicate_of = Some(MemoryId("mem_0".into()));
        }
        assert_eq!(
            check_duplicate(&copy, &gone).err().map(|e| e.message),
            Some("mem_a is stale: it repeats mem_0, so mark mem_b a duplicate of that one".into())
        );
        if let Some(s) = gone.stale.as_mut() {
            s.replaced_by = Some(MemoryId("mem_c".into()));
        }
        let err = check_duplicate(&copy, &gone).err();
        assert_eq!(
            err.as_ref().map(|e| e.message.as_str()),
            Some("mem_a is stale: mem_c took its place, so mark mem_b a duplicate of that one")
        );
        assert_eq!(err.map(|e| e.code), Some(fctl_proto::ErrorCode::Conflict));
        assert_eq!(duplicate_reason(&original.id), "Duplicate of mem_a");
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
        let mut copy = stale(note("mem_3", "## Tests need npm ci first", 1), 5);
        if let Some(s) = copy.stale.as_mut() {
            s.reason = Some("Duplicate of mem_1".into());
            s.duplicate_of = Some(MemoryId("mem_1".into()));
        }
        assert_eq!(
            stale_message(&copy),
            "Marked a memory a duplicate of mem_1: Tests need npm ci first"
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

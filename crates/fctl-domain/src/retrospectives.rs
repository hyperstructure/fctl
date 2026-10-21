//! Retrospectives: what an agent records as it finishes a piece of its
//! session's work (`fctl retro`), for the project's people. What one holds,
//! how long its parts are, and how many a session records.

use crate::memory::headline;
use crate::validate;
use fctl_proto::ApiError;
use fctl_proto::session::{Retrospective, RetrospectiveOutcome};

/// The longest "worked on", in characters: a few sentences.
pub const MAX_WORKED_ON_CHARS: usize = 2_000;
/// The longest approach, challenges or insights, in characters.
pub const MAX_PART_CHARS: usize = 4_000;
/// The most retrospectives one session records: one for each piece of
/// work, and a session's follow-ups are a handful.
pub const MAX_PER_SESSION: u32 = 50;

/// A retrospective's parts, checked: trimmed, within their lengths, empty
/// optional parts left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parts {
    pub worked_on: String,
    pub approach: Option<String>,
    pub challenges: Option<String>,
    pub insights: Option<String>,
}

/// Checks a retrospective's parts: what it worked on must be said.
pub fn check(
    worked_on: &str,
    approach: Option<&str>,
    challenges: Option<&str>,
    insights: Option<&str>,
) -> Result<Parts, ApiError> {
    let worked_on = validate::text("worked_on", worked_on, MAX_WORKED_ON_CHARS)?;
    if worked_on.is_empty() {
        return Err(ApiError::validation("worked_on", "Say what the work was"));
    }
    let part = |field: &str, s: Option<&str>| -> Result<Option<String>, ApiError> {
        let Some(s) = s else {
            return Ok(None);
        };
        let s = validate::text(field, s, MAX_PART_CHARS)?;
        Ok((!s.is_empty()).then_some(s))
    };
    Ok(Parts {
        worked_on,
        approach: part("approach", approach)?,
        challenges: part("challenges", challenges)?,
        insights: part("insights", insights)?,
    })
}

/// An outcome as people read it.
pub fn outcome_label(outcome: RetrospectiveOutcome) -> &'static str {
    match outcome {
        RetrospectiveOutcome::Completed => "Completed",
        RetrospectiveOutcome::Partial => "Partly done",
        RetrospectiveOutcome::Blocked => "Blocked",
    }
}

/// An outcome as it is stored and sent (`completed`, `partial`,
/// `blocked`).
pub fn outcome_name(outcome: RetrospectiveOutcome) -> &'static str {
    match outcome {
        RetrospectiveOutcome::Completed => "completed",
        RetrospectiveOutcome::Partial => "partial",
        RetrospectiveOutcome::Blocked => "blocked",
    }
}

/// The outcome a name stands for.
pub fn parse_outcome(name: &str) -> Option<RetrospectiveOutcome> {
    match name {
        "completed" => Some(RetrospectiveOutcome::Completed),
        "partial" => Some(RetrospectiveOutcome::Partial),
        "blocked" => Some(RetrospectiveOutcome::Blocked),
        _ => None,
    }
}

/// What the transcript says when an agent records a retrospective.
pub fn message(r: &Retrospective) -> String {
    format!(
        "Recorded a retrospective ({}): {}",
        outcome_label(r.outcome).to_lowercase(),
        headline(&r.worked_on, 140)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use fctl_proto::session::SessionRef;
    use fctl_proto::{ProjectId, RetrospectiveId, SessionId};

    #[test]
    fn parts_are_trimmed_and_bounded() {
        let parts = check(
            " The login redirect. ",
            Some("  "),
            None,
            Some(" Use the helper. "),
        );
        assert_eq!(
            parts.ok(),
            Some(Parts {
                worked_on: "The login redirect.".into(),
                approach: None,
                challenges: None,
                insights: Some("Use the helper.".into()),
            })
        );
        assert!(check("  ", None, None, None).is_err());
        assert!(check(&"x".repeat(MAX_WORKED_ON_CHARS + 1), None, None, None).is_err());
        let long = "x".repeat(MAX_PART_CHARS + 1);
        let err = check("Work", None, Some(&long), None).err();
        assert_eq!(err.and_then(|e| e.field).as_deref(), Some("challenges"));
    }

    #[test]
    fn outcomes_round_trip_through_their_names() {
        for o in [
            RetrospectiveOutcome::Completed,
            RetrospectiveOutcome::Partial,
            RetrospectiveOutcome::Blocked,
        ] {
            assert_eq!(parse_outcome(outcome_name(o)), Some(o));
            assert_eq!(
                serde_json::to_value(o).ok(),
                Some(serde_json::json!(outcome_name(o)))
            );
        }
        assert_eq!(parse_outcome("done"), None);
        assert_eq!(outcome_label(RetrospectiveOutcome::Partial), "Partly done");
    }

    #[test]
    fn the_transcript_names_the_outcome_and_the_work() {
        let r = Retrospective {
            id: RetrospectiveId("rtr_1".into()),
            project_id: ProjectId("prj_1".into()),
            session: SessionRef {
                id: SessionId("ses_1".into()),
                number: 3,
                title: "Fix it".into(),
            },
            started_by: None,
            created_at: 1,
            outcome: RetrospectiveOutcome::Blocked,
            worked_on: "Retry failed uploads in the sync worker.\nMore detail.".into(),
            approach: None,
            challenges: None,
            insights: None,
        };
        assert_eq!(
            message(&r),
            "Recorded a retrospective (blocked): Retry failed uploads in the sync worker."
        );
    }
}

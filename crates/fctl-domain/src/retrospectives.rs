//! Retrospectives: what an agent records as it finishes a piece of its
//! session's work (`fctl retro`), for the project's people, and updates as
//! later turns carry on with that work (`fctl retro update`). What one
//! holds, how long its parts are, how an update changes one, and how many a
//! session records.

use crate::memory::headline;
use crate::validate;
use fctl_proto::ApiError;
use fctl_proto::session::{Retrospective, RetrospectiveOutcome};

/// The longest "worked on", in characters: a few sentences.
pub const MAX_WORKED_ON_CHARS: usize = 2_000;
/// The longest approach, challenges or insights, in characters.
pub const MAX_PART_CHARS: usize = 4_000;
/// The most retrospectives one session records: one for each piece of
/// work, which its agent updates as follow-ups carry on with it.
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

/// What an update says (`fctl retro update`): the outcome and the parts
/// given replace those the retrospective had, and the others stay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change<'a> {
    pub outcome: Option<RetrospectiveOutcome>,
    pub worked_on: Option<&'a str>,
    pub approach: Option<&'a str>,
    pub challenges: Option<&'a str>,
    pub insights: Option<&'a str>,
}

/// A retrospective's outcome and parts once `change` applies to it,
/// checked as when it was recorded: an empty part leaves it out, but what
/// it worked on must still be said.
pub fn update(
    r: &Retrospective,
    change: &Change<'_>,
) -> Result<(RetrospectiveOutcome, Parts), ApiError> {
    if *change == Change::default() {
        return Err(ApiError::bad_request(
            "Say what changed: the outcome, what the work was, the approach, the challenges or the insights",
        ));
    }
    let parts = check(
        change.worked_on.unwrap_or(&r.worked_on),
        change.approach.or(r.approach.as_deref()),
        change.challenges.or(r.challenges.as_deref()),
        change.insights.or(r.insights.as_deref()),
    )?;
    Ok((change.outcome.unwrap_or(r.outcome), parts))
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

/// What the transcript says when an agent records a retrospective, or
/// updates one.
pub fn message(r: &Retrospective) -> String {
    format!(
        "{} a retrospective ({}): {}",
        if r.updated_at.is_some() {
            "Updated"
        } else {
            "Recorded"
        },
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

    fn blocked() -> Retrospective {
        Retrospective {
            id: RetrospectiveId("rtr_1".into()),
            project_id: ProjectId("prj_1".into()),
            session: SessionRef {
                id: SessionId("ses_1".into()),
                number: 3,
                title: "Fix it".into(),
            },
            started_by: None,
            created_at: 1,
            updated_at: None,
            outcome: RetrospectiveOutcome::Blocked,
            worked_on: "Retry failed uploads in the sync worker.\nMore detail.".into(),
            approach: Some("The existing backoff helper.".into()),
            challenges: Some("No credentials for the staging bucket.".into()),
            insights: None,
        }
    }

    #[test]
    fn the_transcript_names_the_outcome_and_the_work() {
        let mut r = blocked();
        assert_eq!(
            message(&r),
            "Recorded a retrospective (blocked): Retry failed uploads in the sync worker."
        );
        r.updated_at = Some(2);
        r.outcome = RetrospectiveOutcome::Completed;
        assert_eq!(
            message(&r),
            "Updated a retrospective (completed): Retry failed uploads in the sync worker."
        );
    }

    #[test]
    fn an_update_changes_what_it_says_and_keeps_the_rest() {
        let r = blocked();
        let change = Change {
            outcome: Some(RetrospectiveOutcome::Completed),
            insights: Some(" Ask for the bucket's credentials first. "),
            ..Change::default()
        };
        assert_eq!(
            update(&r, &change).ok(),
            Some((
                RetrospectiveOutcome::Completed,
                Parts {
                    worked_on: "Retry failed uploads in the sync worker.\nMore detail.".into(),
                    approach: Some("The existing backoff helper.".into()),
                    challenges: Some("No credentials for the staging bucket.".into()),
                    insights: Some("Ask for the bucket's credentials first.".into()),
                }
            ))
        );
        // An empty part leaves it out; the outcome stays when none is given.
        let (outcome, parts) = update(
            &r,
            &Change {
                worked_on: Some("Retried failed uploads (#41)."),
                challenges: Some(" "),
                ..Change::default()
            },
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert_eq!(outcome, RetrospectiveOutcome::Blocked);
        assert_eq!(parts.worked_on, "Retried failed uploads (#41).");
        assert_eq!(parts.challenges, None);
        assert_eq!(
            parts.approach.as_deref(),
            Some("The existing backoff helper.")
        );
    }

    #[test]
    fn an_update_says_something_and_keeps_what_the_work_was() {
        let r = blocked();
        assert!(update(&r, &Change::default()).is_err());
        let emptied = update(
            &r,
            &Change {
                worked_on: Some("  "),
                ..Change::default()
            },
        );
        assert_eq!(
            emptied.err().and_then(|e| e.field).as_deref(),
            Some("worked_on")
        );
        let long = "x".repeat(MAX_PART_CHARS + 1);
        let too_long = update(
            &r,
            &Change {
                approach: Some(&long),
                ..Change::default()
            },
        );
        assert_eq!(
            too_long.err().and_then(|e| e.field).as_deref(),
            Some("approach")
        );
    }
}

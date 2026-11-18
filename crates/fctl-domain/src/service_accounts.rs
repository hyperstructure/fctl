//! Service accounts (docs/architecture.md#service-accounts, ADR 0086): a
//! workspace's credential for a program of its own (a portal where people
//! start agents, a chat bot, a script), which starts sessions for its
//! people, runs its schedules and reads its sessions through the REST API
//! and the CLI.
//!
//! - **Whose.** A workspace's admins make them (`fctl_svc_…`), each with a
//!   name and what it is, which the people it asks read. One works while
//!   the admin who made its secret, its keeper, is an active admin of the
//!   workspace ([`stopped`]): an admin who leaves, or is demoted or
//!   disabled, can't keep acting for people with a secret they kept.
//!   Another admin makes it a new secret, and it works again.
//! - **For whom.** Each request names the person it acts for
//!   (`Fctl-On-Behalf-Of`, by id or address, [`named`]): the session it
//!   starts is theirs, on their Claude subscription, within what they may
//!   do in the project, at most what an editor does. Where the instance
//!   asks people first (`InstanceSettings::service_account_consent`), it
//!   acts only for those who allowed it ([`may_act_for`]).
//! - **What.** Acting for someone, what an access token that runs sessions
//!   does, in its workspace's projects; for nobody, it reads them, and runs
//!   a schedule as the person it runs for ([`allows`]). Never the account,
//!   people, settings, secrets or tokens.

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::api::TokenAccess;
use fctl_proto::service_accounts::ServiceAccountAnswer;

/// The most service accounts a workspace has.
pub const MAX_PER_WORKSPACE: u32 = 10;
/// The longest name.
pub const NAME_MAX: usize = 60;
/// The longest description.
pub const DESCRIPTION_MAX: usize = 280;

/// A service account's name: trimmed, 1 to [`NAME_MAX`] characters.
pub fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::validation(
            "name",
            "Name it for what uses it: \"Acme portal\", say",
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(ApiError::validation("name", "Remove control characters"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(ApiError::validation(
            "name",
            format!("Use at most {NAME_MAX} characters"),
        ));
    }
    Ok(name.to_owned())
}

/// What a service account is, for the people it asks: trimmed, at most
/// [`DESCRIPTION_MAX`] characters; empty is none.
pub fn check_description(description: Option<&str>) -> Result<Option<String>> {
    let Some(d) = description.map(str::trim).filter(|d| !d.is_empty()) else {
        return Ok(None);
    };
    if d.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ApiError::validation(
            "description",
            "Remove control characters",
        ));
    }
    if d.chars().count() > DESCRIPTION_MAX {
        return Err(ApiError::validation(
            "description",
            format!("Use at most {DESCRIPTION_MAX} characters"),
        ));
    }
    Ok(Some(d.to_owned()))
}

/// Whether a workspace with `count` service accounts may have another.
pub fn check_room(count: u32) -> Result<()> {
    if count >= MAX_PER_WORKSPACE {
        return Err(ApiError::new(
            fctl_proto::ErrorCode::Conflict,
            format!(
                "A workspace has at most {MAX_PER_WORKSPACE} service accounts: delete one it no longer uses first"
            ),
        ));
    }
    Ok(())
}

/// The end of a secret, which tells secrets apart without showing them:
/// its last four characters.
pub fn hint(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    chars[chars.len().saturating_sub(4)..].iter().collect()
}

/// The field errors about the person a request names are about, so that
/// a program tells them from other refusals: `on_behalf_of`, for the
/// `Fctl-On-Behalf-Of` header.
pub const FIELD: &str = "on_behalf_of";

/// Whom a request names in `Fctl-On-Behalf-Of`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Named {
    /// By their id, `usr_…`.
    Id(String),
    /// By one of their addresses, lowercased.
    Email(String),
}

/// Reads `Fctl-On-Behalf-Of`: a person's id, or one of their addresses.
pub fn named(value: &str) -> Result<Named> {
    let value = value.trim();
    if fctl_proto::UserId::is_valid(value) {
        return Ok(Named::Id(value.to_owned()));
    }
    if value.contains('@') && !value.chars().any(char::is_whitespace) && value.len() <= 254 {
        return Ok(Named::Email(value.to_lowercase()));
    }
    Err(ApiError::validation(
        FIELD,
        "Name the person a service account acts for by their id (usr_…) or one of their addresses",
    ))
}

/// What a service account may call: `method` on the API path after
/// `/api/v1/` (`segs`), acting for someone (`acting_for`) or for nobody.
/// Acting for someone, what an access token that runs sessions does
/// (`crate::limits::allows`), in its workspace's projects, but signing
/// itself out; for nobody, reading them, its own people and what it is,
/// and running a schedule, which runs for the person it runs for (the
/// control plane names them, [`runs_schedule`]). Roles still apply: this
/// only says what it may try.
pub fn allows(acting_for: bool, method: &str, segs: &[&str]) -> Result<()> {
    // What it is, and whom it may act for.
    if matches!(
        (method, segs),
        (
            "GET",
            ["service-account"] | ["service-accounts", _, "people"]
        )
    ) {
        return Ok(());
    }
    // Live connections check their token again as they stay open, which a
    // service account's isn't: its sessions' events come from
    // `GET /sessions/{id}/events`, and webhooks post what happens.
    let refused = match (method, segs) {
        ("GET", ["sessions", _, "ws"] | ["feed", "ws"]) => true,
        // It signs nobody out, and isn't a person whose account shows.
        ("POST", ["auth", "logout"]) => true,
        ("GET", ["auth", "me"]) => !acting_for,
        _ => false,
    };
    if refused {
        return Err(refusal(acting_for));
    }
    let access = if acting_for {
        TokenAccess::Run
    } else if runs_schedule(method, segs).is_some() {
        // It runs for the person the control plane names.
        return Ok(());
    } else {
        TokenAccess::Read
    };
    crate::limits::allows(access, method, segs).map_err(|_| refusal(acting_for))
}

/// The schedule a request runs, for a service account that names nobody:
/// it runs for the schedule's person, as at its time.
pub fn runs_schedule<'a>(method: &str, segs: &[&'a str]) -> Option<&'a str> {
    match (method, segs) {
        ("POST", ["schedules", id, "runs"]) => Some(id),
        _ => None,
    }
}

fn refusal(acting_for: bool) -> ApiError {
    ApiError::forbidden(if acting_for {
        "A service account starts and steers sessions for the person it acts for, and reads its workspace's projects, sessions and schedules: nothing else."
    } else {
        "A service account that acts for nobody reads its workspace's projects, sessions and schedules, and runs schedules: to start or steer a session, name the person it acts for in the Fctl-On-Behalf-Of header (fctl --on-behalf-of)."
    })
}

/// Why a service account works for nobody now, when it doesn't: the admin
/// who made its secret (`keeper`, by name, absent once their account is
/// gone) is no longer an active admin of `workspace`.
pub fn stopped(keeper_admin: bool, keeper: Option<&str>, workspace: &str) -> Option<String> {
    if keeper_admin {
        return None;
    }
    Some(match keeper {
        Some(name) => format!(
            "{name}, who made its secret, is no longer an admin of {workspace}. An admin makes it a new secret to start it again."
        ),
        None => format!(
            "The admin who made its secret is no longer on the instance. An admin of {workspace} makes it a new secret to start it again."
        ),
    })
}

/// Why a service account works for nobody now whose keeper (`keeper`, by
/// name) has to set up two-factor sign-in, which the instance or its
/// workspace requires, before they do anything else: it acts with their
/// authority, as their own tokens do, which wait too.
pub fn keeper_needs_two_factor(keeper: &str) -> String {
    format!(
        "{keeper}, who made its secret, has to set up two-factor sign-in first, which this instance or its workspace requires. It starts again once they have."
    )
}

/// What decides whether a service account may act for someone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Person {
    /// They are a member of its workspace.
    pub member: bool,
    /// Their account is active.
    pub active: bool,
    /// Two-factor sign-in, which the instance or the workspace requires,
    /// waits for them to set it up: until they have, they do nothing else.
    pub needs_two_factor: bool,
    /// Their answer to it.
    pub answer: Option<ServiceAccountAnswer>,
}

/// Why a service account may not act for someone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhyNot {
    NotMember,
    Disabled,
    TwoFactor,
    Unanswered,
    Declined,
}

impl WhyNot {
    /// Why, for its admins and the program, about `who` (their name) in
    /// `workspace`, of service account `name`.
    pub fn reason(self, who: &str, name: &str, workspace: &str) -> String {
        match self {
            Self::NotMember => format!("{who} isn't a member of {workspace}"),
            Self::Disabled => format!("{who}'s account is disabled"),
            Self::TwoFactor => {
                format!("{who} has to set up two-factor sign-in first, which {workspace} requires")
            }
            Self::Unanswered => format!(
                "{who} hasn't allowed {name} to start sessions for them yet: fctl asks them when they next open it"
            ),
            Self::Declined => format!("{who} chose not to let {name} start sessions for them"),
        }
    }

    /// What a request that names them answers, about [`FIELD`].
    pub fn error(self, who: &str, name: &str, workspace: &str) -> ApiError {
        let mut e = match self {
            Self::NotMember => ApiError::new(
                fctl_proto::ErrorCode::NotFound,
                format!("Nobody by that id or address is a member of {workspace}."),
            ),
            _ => ApiError::new(
                fctl_proto::ErrorCode::AccessDenied,
                format!("{}.", self.reason(who, name, workspace)),
            ),
        };
        e.field = Some(FIELD.to_owned());
        e
    }
}

/// Whether a service account may act for `person` now: a member of its
/// workspace, whose account is active and open to it, who allowed it,
/// unless the instance doesn't ask people (`consent` false).
pub fn may_act_for(consent: bool, person: Person) -> std::result::Result<(), WhyNot> {
    if !person.member {
        return Err(WhyNot::NotMember);
    }
    if !person.active {
        return Err(WhyNot::Disabled);
    }
    if person.needs_two_factor {
        return Err(WhyNot::TwoFactor);
    }
    if !consent {
        return Ok(());
    }
    match person.answer {
        Some(ServiceAccountAnswer::Allowed) => Ok(()),
        Some(ServiceAccountAnswer::Declined) => Err(WhyNot::Declined),
        _ => Err(WhyNot::Unanswered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(path: &str) -> Vec<&str> {
        path.split('/').collect()
    }

    #[test]
    fn names_and_descriptions_are_checked() {
        assert_eq!(check_name("  Acme portal "), Ok("Acme portal".to_owned()));
        assert!(check_name("   ").is_err());
        assert!(check_name("a\u{7}b").is_err());
        assert!(check_name(&"x".repeat(NAME_MAX + 1)).is_err());
        assert!(check_name(&"é".repeat(NAME_MAX)).is_ok());
        assert_eq!(check_description(None), Ok(None));
        assert_eq!(check_description(Some("  ")), Ok(None));
        assert_eq!(
            check_description(Some(" Our portal.\nAsk #agents. ")),
            Ok(Some("Our portal.\nAsk #agents.".to_owned()))
        );
        assert!(check_description(Some("a\u{0}b")).is_err());
        assert!(check_description(Some(&"x".repeat(DESCRIPTION_MAX + 1))).is_err());
    }

    #[test]
    fn a_workspace_has_ten_at_most() {
        assert!(check_room(0).is_ok());
        assert!(check_room(MAX_PER_WORKSPACE - 1).is_ok());
        assert!(check_room(MAX_PER_WORKSPACE).is_err());
    }

    #[test]
    fn hints_show_the_end_alone() {
        assert_eq!(hint("fctl_svc_abcdefx8Kq"), "x8Kq");
        assert_eq!(hint("ab"), "ab");
        assert_eq!(hint(""), "");
    }

    #[test]
    fn people_are_named_by_id_or_address() {
        assert_eq!(
            named(" usr_01JABCDEF "),
            Ok(Named::Id("usr_01JABCDEF".to_owned()))
        );
        assert_eq!(
            named("Ada@Example.com"),
            Ok(Named::Email("ada@example.com".to_owned()))
        );
        assert!(named("Ada Lovelace").is_err());
        assert!(named("").is_err());
        assert!(named("ada @example.com").is_err());
    }

    #[test]
    fn acting_for_someone_it_runs_sessions_and_nothing_more() {
        for (method, path) in [
            ("POST", "projects/web/sessions"),
            ("POST", "sessions/ses_1/messages"),
            ("POST", "sessions/ses_1/interrupt"),
            ("POST", "sessions/ses_1/stop"),
            ("POST", "sessions/ses_1/resume"),
            ("POST", "projects/web/attachments"),
            ("POST", "schedules/sch_1/runs"),
            ("GET", "sessions"),
            ("GET", "sessions/ses_1/events"),
            ("GET", "projects"),
            ("GET", "auth/me"),
            ("GET", "service-account"),
        ] {
            assert!(allows(true, method, &segs(path)).is_ok(), "{method} {path}");
        }
        for (method, path) in [
            // What only the person does, or someone who manages things.
            ("PATCH", "projects/web"),
            ("POST", "projects/web/schedules"),
            ("PATCH", "schedules/sch_1"),
            ("GET", "projects/web/secrets"),
            ("POST", "auth/tokens"),
            ("POST", "auth/logout"),
            ("PATCH", "auth/me"),
            ("GET", "me/claude"),
            ("POST", "workspaces/acme/invitations"),
            ("POST", "workspaces/acme/service-accounts"),
            ("PUT", "service-accounts/sva_1/answer"),
            ("GET", "admin/users"),
            // Live connections check a token of the person's again.
            ("GET", "sessions/ses_1/ws"),
            ("GET", "feed/ws"),
        ] {
            assert!(
                allows(true, method, &segs(path)).is_err(),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn acting_for_nobody_it_reads_and_runs_schedules() {
        for (method, path) in [
            ("GET", "projects"),
            ("GET", "projects/web"),
            ("GET", "projects/web/schedules"),
            ("GET", "schedules/sch_1/runs"),
            ("GET", "sessions"),
            ("GET", "sessions/ses_1"),
            ("GET", "sessions/ses_1/events"),
            ("GET", "workspaces"),
            ("GET", "service-account"),
            ("GET", "service-accounts/sva_1/people"),
            ("POST", "schedules/sch_1/runs"),
        ] {
            assert!(
                allows(false, method, &segs(path)).is_ok(),
                "{method} {path}"
            );
        }
        for (method, path) in [
            ("POST", "projects/web/sessions"),
            ("POST", "sessions/ses_1/messages"),
            ("POST", "sessions/ses_1/stop"),
            ("GET", "auth/me"),
            ("POST", "auth/logout"),
            ("GET", "sessions/ses_1/ws"),
            ("GET", "service-accounts/sva_1"),
            ("GET", "workspaces/acme/members"),
        ] {
            assert!(
                allows(false, method, &segs(path)).is_err(),
                "{method} {path}"
            );
        }
        assert_eq!(
            runs_schedule("POST", &segs("schedules/sch_1/runs")),
            Some("sch_1")
        );
        assert_eq!(runs_schedule("GET", &segs("schedules/sch_1/runs")), None);
    }

    #[test]
    fn it_stops_when_its_keeper_goes() {
        assert_eq!(stopped(true, Some("Ada"), "Acme"), None);
        let why = stopped(false, Some("Ada"), "Acme").unwrap_or_default();
        assert!(why.starts_with("Ada, who made its secret"), "{why}");
        assert!(stopped(false, None, "Acme").is_some());
        assert_eq!(
            keeper_needs_two_factor("Ada"),
            "Ada, who made its secret, has to set up two-factor sign-in first, which this instance or its workspace requires. It starts again once they have."
        );
    }

    #[test]
    fn it_acts_only_for_people_who_allowed_it_where_the_instance_asks() {
        let allowed = Person {
            member: true,
            active: true,
            needs_two_factor: false,
            answer: Some(ServiceAccountAnswer::Allowed),
        };
        assert_eq!(may_act_for(true, allowed), Ok(()));
        let unanswered = Person {
            answer: None,
            ..allowed
        };
        assert_eq!(may_act_for(true, unanswered), Err(WhyNot::Unanswered));
        assert_eq!(may_act_for(false, unanswered), Ok(()));
        let declined = Person {
            answer: Some(ServiceAccountAnswer::Declined),
            ..allowed
        };
        assert_eq!(may_act_for(true, declined), Err(WhyNot::Declined));
        // Where the instance doesn't ask, nobody chooses.
        assert_eq!(may_act_for(false, declined), Ok(()));
        // Whatever they answered: only for active members it may act for.
        assert_eq!(
            may_act_for(
                false,
                Person {
                    member: false,
                    ..allowed
                }
            ),
            Err(WhyNot::NotMember)
        );
        assert_eq!(
            may_act_for(
                true,
                Person {
                    active: false,
                    ..allowed
                }
            ),
            Err(WhyNot::Disabled)
        );
        assert_eq!(
            may_act_for(
                false,
                Person {
                    needs_two_factor: true,
                    ..allowed
                }
            ),
            Err(WhyNot::TwoFactor)
        );
    }

    #[test]
    fn reasons_name_the_person_and_the_account() {
        let r = WhyNot::Unanswered.reason("Ada Lovelace", "Acme portal", "Acme");
        assert!(
            r.starts_with("Ada Lovelace hasn't allowed Acme portal"),
            "{r}"
        );
        let gone = WhyNot::NotMember.error("Ada", "Acme portal", "Acme");
        assert_eq!(gone.code, fctl_proto::ErrorCode::NotFound);
        // Nothing about whether the address has an account elsewhere.
        assert!(!gone.message.contains("Ada"), "{}", gone.message);
        let declined = WhyNot::Declined.error("Ada", "Acme portal", "Acme");
        assert_eq!(declined.code, fctl_proto::ErrorCode::AccessDenied);
        assert_eq!(declined.field.as_deref(), Some(FIELD));
    }
}

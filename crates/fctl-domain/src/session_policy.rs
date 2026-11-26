//! How long sign-ins last, and who makes access tokens: the instance's
//! session policies (#618; `InstanceSettings::browser_max_hours` and the
//! rest, [`SessionPolicy`]).
//!
//! - A browser's sign-in ends a set number of hours after it was made,
//!   however much it's used, and after a set time unused (30 days unless
//!   the instance says less).
//! - A CLI's ends a set number of days after it was made, and after 180
//!   days unused as before.
//! - An access token can't be made to last longer than the instance lets
//!   one, and one made before that ends when it gets that old.
//! - Members may be kept from making access tokens: theirs stop working.
//!
//! Every request checks its token against them ([`ended`]), so that a
//! policy holds the sign-ins made before it too; saving one also ends
//! those it doesn't let last, and sets the others' end to the policy's, so
//! that what people see of them says so. Where a domain requires its
//! provider, signing in again goes through it, which checks that its
//! person may still sign in: how short a policy makes sign-ins is how soon
//! someone the provider closed out is out of fctl too.

use crate::enrollment::minutes;
use fctl_proto::ApiError;
use fctl_proto::api::{
    DEFAULT_BROWSER_IDLE_MINUTES, MAX_ACCESS_TOKEN_DAYS, MAX_BROWSER_MAX_HOURS, MAX_CLI_MAX_DAYS,
    MIN_BROWSER_IDLE_MINUTES, SessionPolicy, TokenKind,
};

pub const MINUTE_MS: i64 = 60 * 1000;
pub const HOUR_MS: i64 = 60 * MINUTE_MS;
pub const DAY_MS: i64 = 24 * HOUR_MS;

/// A CLI's sign-in ends this long after it was last used.
pub const CLI_IDLE_MS: i64 = 180 * DAY_MS;

/// How often, at most, a token's use is recorded (`tokens.last_used_at`),
/// where the instance lets browsers sit unused for long.
pub const TOUCH_MS: i64 = 10 * MINUTE_MS;

/// The longest a token of `kind` lasts from its sign-in, where the policy
/// limits it.
pub fn max_age_ms(p: &SessionPolicy, kind: TokenKind) -> Option<i64> {
    let (n, unit) = match kind {
        TokenKind::Web => (p.browser_max_hours, HOUR_MS),
        TokenKind::Cli => (p.cli_max_days, DAY_MS),
        TokenKind::Pat => (p.access_token_max_days, DAY_MS),
    };
    (n > 0).then(|| i64::from(n) * unit)
}

/// How long a token of `kind` lasts unused. An access token lasts until
/// the end its maker chose, used or not.
pub fn idle_ms(p: &SessionPolicy, kind: TokenKind) -> Option<i64> {
    match kind {
        TokenKind::Web => Some(browser_idle_ms(p)),
        TokenKind::Cli => Some(CLI_IDLE_MS),
        TokenKind::Pat => None,
    }
}

/// How long a browser lasts unused, within the bounds a policy may set,
/// whatever was saved.
fn browser_idle_ms(p: &SessionPolicy) -> i64 {
    let m = p
        .browser_idle_minutes
        .clamp(MIN_BROWSER_IDLE_MINUTES, DEFAULT_BROWSER_IDLE_MINUTES);
    i64::from(m) * MINUTE_MS
}

/// When a token of `kind` whose sign-in was at `created_at` ends, as it's
/// used at `now`: unused long enough, at its policy's longest, or at
/// `chosen`, the end its maker chose, whichever comes first. `None`: never.
pub fn expires_at(
    p: &SessionPolicy,
    kind: TokenKind,
    created_at: i64,
    now: i64,
    chosen: Option<i64>,
) -> Option<i64> {
    let idle = idle_ms(p, kind).map(|i| now.saturating_add(i));
    let max = max_age_ms(p, kind).map(|m| created_at.saturating_add(m));
    [chosen, idle, max].into_iter().flatten().min()
}

/// How often a browser's use is recorded: often enough, where browsers
/// sign out after a short while unused, that one in use isn't taken for
/// unused between two records.
pub fn touch_after_ms(p: &SessionPolicy) -> i64 {
    TOUCH_MS.min(browser_idle_ms(p) / 4)
}

/// A browser's or CLI's sign-in, or an access token, as [`ended`] reads
/// it.
#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: TokenKind,
    /// When its sign-in was, or the access token was made.
    pub created_at: i64,
    /// When its use was last recorded.
    pub last_used_at: Option<i64>,
    /// When it ends, as kept.
    pub expires_at: Option<i64>,
    /// Its person is a member of the instance, not an admin.
    pub member: bool,
}

impl Token {
    /// It as a socket opened with it reads it while it stays open: used at
    /// `now`, as an open socket is in use though nothing records it. A
    /// browser's or a CLI's stored end is its last recorded use plus the
    /// time it may go unused, capped by the policy's longest, so it goes
    /// too: [`ended`] applies that longest from its sign-in. An access
    /// token keeps the end its maker chose.
    pub fn in_use(self, now: i64) -> Self {
        Self {
            last_used_at: Some(now),
            expires_at: match self.kind {
                TokenKind::Pat => self.expires_at,
                TokenKind::Web | TokenKind::Cli => None,
            },
            ..self
        }
    }
}

/// Why a token no longer signs its person in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// It ended as kept: unused too long, or when its maker chose.
    Expired,
    /// Its sign-in is older than the policy lets one of its kind be.
    TooOld,
    /// A browser unused longer than the policy lets one be.
    Unused,
    /// A member's access token, where members make none.
    MembersMakeNone,
}

/// Whether, and why, `t` no longer signs its person in at `now` under
/// `p`.
pub fn ended(p: &SessionPolicy, t: &Token, now: i64) -> Option<Ended> {
    if t.expires_at.is_some_and(|e| e <= now) {
        return Some(Ended::Expired);
    }
    if max_age_ms(p, t.kind).is_some_and(|m| t.created_at.saturating_add(m) <= now) {
        return Some(Ended::TooOld);
    }
    if t.kind == TokenKind::Web
        && t.last_used_at
            .unwrap_or(t.created_at)
            .saturating_add(browser_idle_ms(p))
            <= now
    {
        return Some(Ended::Unused);
    }
    if t.kind == TokenKind::Pat && t.member && !p.member_access_tokens {
        return Some(Ended::MembersMakeNone);
    }
    None
}

/// When `t` ends under `p`, unless it's used again first: as kept, or
/// sooner where the policy says less. What people see of it says this.
pub fn ends_at(p: &SessionPolicy, t: &Token) -> Option<i64> {
    let max = max_age_ms(p, t.kind).map(|m| t.created_at.saturating_add(m));
    let unused = (t.kind == TokenKind::Web).then(|| {
        t.last_used_at
            .unwrap_or(t.created_at)
            .saturating_add(browser_idle_ms(p))
    });
    [t.expires_at, max, unused].into_iter().flatten().min()
}

/// What a CLI's or an access token's request that `why` refuses answers.
/// A browser's is signed out, and signs in again.
pub fn refusal(p: &SessionPolicy, kind: TokenKind, why: Ended) -> String {
    match (kind, why) {
        (TokenKind::Pat, Ended::MembersMakeNone) => {
            "Only this instance's admins make access tokens now, so yours stopped working: use the CLI signed in as you (fctl login), or ask an admin.".to_owned()
        }
        (TokenKind::Pat, Ended::TooOld) => format!(
            "This access token is older than this instance lets one last ({}): make another in Settings → Access tokens.",
            days(p.access_token_max_days)
        ),
        (TokenKind::Cli, Ended::TooOld) => format!(
            "This instance signs CLIs out {} after they sign in. Sign in again: fctl login",
            days(p.cli_max_days)
        ),
        (TokenKind::Pat, _) => "This access token has expired: make another in Settings → Access tokens.".to_owned(),
        _ => "Your sign-in has expired. Sign in again: fctl login".to_owned(),
    }
}

/// `a day`, `30 days`, `a year`.
pub fn days(n: u32) -> String {
    match n {
        365 => "a year".to_owned(),
        n => minutes(n.saturating_mul(1440)),
    }
}

/// `an hour`, `12 hours`, `a day`, `a week`.
pub fn hours(n: u32) -> String {
    match n {
        8760 => "a year".to_owned(),
        n => minutes(n.saturating_mul(60)),
    }
}

/// Checks `p` as an admin saves it: each limit within its bounds.
pub fn check(p: &SessionPolicy) -> Result<(), ApiError> {
    if p.browser_max_hours > MAX_BROWSER_MAX_HOURS {
        return Err(ApiError::validation(
            "browser_max_hours",
            format!(
                "A browser can be kept signed in {MAX_BROWSER_MAX_HOURS} hours (a year) at most: 0 lets it stay as long as it's used"
            ),
        ));
    }
    if !(MIN_BROWSER_IDLE_MINUTES..=DEFAULT_BROWSER_IDLE_MINUTES).contains(&p.browser_idle_minutes)
    {
        return Err(ApiError::validation(
            "browser_idle_minutes",
            format!(
                "A browser unused for {MIN_BROWSER_IDLE_MINUTES} minutes to 30 days ({DEFAULT_BROWSER_IDLE_MINUTES} minutes) can be signed out"
            ),
        ));
    }
    if p.cli_max_days > MAX_CLI_MAX_DAYS {
        return Err(ApiError::validation(
            "cli_max_days",
            format!(
                "A CLI can be kept signed in {MAX_CLI_MAX_DAYS} days at most: 0 lets it stay as long as it's used"
            ),
        ));
    }
    if p.access_token_max_days > MAX_ACCESS_TOKEN_DAYS {
        return Err(ApiError::validation(
            "access_token_max_days",
            format!(
                "Access tokens can be made to last {MAX_ACCESS_TOKEN_DAYS} days at most: 0 lets their makers choose"
            ),
        ));
    }
    Ok(())
}

/// Checks the days a new access token is to last (`None`: for ever)
/// against the longest `p` lets one.
pub fn check_token_days(p: &SessionPolicy, days_asked: Option<u32>) -> Result<(), ApiError> {
    let max = p.access_token_max_days;
    if max > 0 && days_asked.is_none_or(|d| d > max) {
        // In days, as people choose a token's: "1 day", "90 days".
        let longest = if max == 1 {
            "1 day".to_owned()
        } else {
            format!("{max} days")
        };
        return Err(ApiError::validation(
            "expires_in_days",
            format!("This instance's access tokens last {longest} at most: choose that or less"),
        ));
    }
    Ok(())
}

/// Whether `next` lets sign-ins last longer than `current`, or lets
/// members make access tokens again: such a change takes confirming it's
/// you.
pub fn relaxes(current: &SessionPolicy, next: &SessionPolicy) -> bool {
    // 0 is no limit: longer than any.
    let longer = |was: u32, now: u32| was > 0 && (now == 0 || now > was);
    longer(current.browser_max_hours, next.browser_max_hours)
        || next.browser_idle_minutes > current.browser_idle_minutes
        || longer(current.cli_max_days, next.cli_max_days)
        || longer(current.access_token_max_days, next.access_token_max_days)
        || (!current.member_access_tokens && next.member_access_tokens)
}

/// What `p` says, a sentence for browsers, CLIs and access tokens each.
pub fn describe(p: &SessionPolicy) -> Vec<String> {
    let idle = minutes(p.browser_idle_minutes);
    let browsers = if p.browser_max_hours > 0 {
        format!(
            "Browsers sign in again after {}, and after {idle} unused.",
            hours(p.browser_max_hours)
        )
    } else {
        format!("Browsers stay signed in while they're used, until {idle} unused.")
    };
    let clis = if p.cli_max_days > 0 {
        format!(
            "CLIs sign in again after {}, and after 180 days unused.",
            days(p.cli_max_days)
        )
    } else {
        "CLIs stay signed in while they're used, until 180 days unused.".to_owned()
    };
    let who = if p.member_access_tokens {
        "Everyone makes access tokens"
    } else {
        "Only admins make access tokens"
    };
    let tokens = if p.access_token_max_days > 0 {
        format!(
            "{who}, each lasting {} at most.",
            days(p.access_token_max_days)
        )
    } else {
        format!("{who}, each lasting as long as its maker chooses.")
    };
    vec![browsers, clis, tokens]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> SessionPolicy {
        SessionPolicy::default()
    }

    fn token(kind: TokenKind, created_at: i64, last_used_at: Option<i64>) -> Token {
        Token {
            kind,
            created_at,
            last_used_at,
            expires_at: None,
            member: true,
        }
    }

    #[test]
    fn by_default_sign_ins_last_as_they_did() {
        let p = policy();
        // A browser: 30 days after its last use; a CLI: 180.
        assert_eq!(
            expires_at(&p, TokenKind::Web, 0, 5 * DAY_MS, None),
            Some(35 * DAY_MS)
        );
        assert_eq!(
            expires_at(&p, TokenKind::Cli, 0, DAY_MS, None),
            Some(181 * DAY_MS)
        );
        // An access token: when its maker chose, or never.
        assert_eq!(expires_at(&p, TokenKind::Pat, 0, DAY_MS, None), None);
        assert_eq!(
            expires_at(&p, TokenKind::Pat, 0, DAY_MS, Some(90 * DAY_MS)),
            Some(90 * DAY_MS)
        );
        // Used often, nothing ends.
        let used = token(TokenKind::Web, 0, Some(400 * DAY_MS));
        assert_eq!(ended(&p, &used, 401 * DAY_MS), None);
        assert_eq!(touch_after_ms(&p), TOUCH_MS);
    }

    #[test]
    fn a_policy_caps_each_kind_from_its_sign_in() {
        let p = SessionPolicy {
            browser_max_hours: 12,
            cli_max_days: 30,
            access_token_max_days: 90,
            ..policy()
        };
        // However recently it's used, a browser ends 12 hours in.
        assert_eq!(
            expires_at(&p, TokenKind::Web, 0, 11 * HOUR_MS, None),
            Some(12 * HOUR_MS)
        );
        assert_eq!(
            ended(
                &p,
                &token(TokenKind::Web, 0, Some(11 * HOUR_MS)),
                12 * HOUR_MS
            ),
            Some(Ended::TooOld)
        );
        assert_eq!(
            expires_at(&p, TokenKind::Cli, 0, DAY_MS, None),
            Some(30 * DAY_MS)
        );
        // An access token made to last for ever lasts 90 days; one that
        // ends sooner, then.
        assert_eq!(
            expires_at(&p, TokenKind::Pat, 0, 0, None),
            Some(90 * DAY_MS)
        );
        assert_eq!(
            expires_at(&p, TokenKind::Pat, 0, 0, Some(7 * DAY_MS)),
            Some(7 * DAY_MS)
        );
        // Made before the policy, it ends once it's that old, and says so.
        assert_eq!(
            ended(&p, &token(TokenKind::Pat, 0, None), 90 * DAY_MS),
            Some(Ended::TooOld)
        );
        assert_eq!(
            ends_at(&p, &token(TokenKind::Pat, 0, None)),
            Some(90 * DAY_MS)
        );
        let soon = Token {
            expires_at: Some(DAY_MS),
            ..token(TokenKind::Pat, 0, None)
        };
        assert_eq!(ends_at(&p, &soon), Some(DAY_MS));
        assert_eq!(ends_at(&policy(), &token(TokenKind::Pat, 0, None)), None);
        // A browser: at the first of its policy's longest and its time
        // unused.
        assert_eq!(
            ends_at(&p, &token(TokenKind::Web, 0, Some(HOUR_MS))),
            Some(12 * HOUR_MS)
        );
        assert_eq!(
            ended(&p, &token(TokenKind::Pat, 0, None), 89 * DAY_MS),
            None
        );
    }

    #[test]
    fn browsers_unused_too_long_sign_out() {
        let p = SessionPolicy {
            browser_idle_minutes: 15,
            ..policy()
        };
        let t = token(TokenKind::Web, 0, Some(HOUR_MS));
        assert_eq!(ended(&p, &t, HOUR_MS + 14 * MINUTE_MS), None);
        assert_eq!(ended(&p, &t, HOUR_MS + 15 * MINUTE_MS), Some(Ended::Unused));
        // Never used since its sign-in: from then.
        let fresh = token(TokenKind::Web, 0, None);
        assert_eq!(ended(&p, &fresh, 15 * MINUTE_MS), Some(Ended::Unused));
        // CLIs and access tokens aren't browsers.
        assert_eq!(
            ended(&p, &token(TokenKind::Cli, 0, None), 15 * MINUTE_MS),
            None
        );
        // Its use is recorded often enough not to be taken for unused.
        assert!(touch_after_ms(&p) * 4 <= 15 * MINUTE_MS);
        // Out of bounds as saved, it reads within them.
        let short = SessionPolicy {
            browser_idle_minutes: 0,
            ..policy()
        };
        assert_eq!(
            expires_at(&short, TokenKind::Web, 0, 0, None),
            Some(5 * MINUTE_MS)
        );
    }

    #[test]
    fn an_open_socket_is_in_use() {
        let p = SessionPolicy {
            browser_max_hours: 12,
            browser_idle_minutes: 15,
            ..policy()
        };
        // A browser last recorded in use an hour ago, its stored end the
        // 15 minutes after: a request ends it, its open socket doesn't.
        let watching = Token {
            expires_at: Some(15 * MINUTE_MS),
            ..token(TokenKind::Web, 0, Some(0))
        };
        assert_eq!(ended(&p, &watching, HOUR_MS), Some(Ended::Expired));
        assert_eq!(ended(&p, &watching.in_use(HOUR_MS), HOUR_MS), None);
        // Past the policy's longest from its sign-in, it ends either way.
        assert_eq!(
            ended(&p, &watching.in_use(12 * HOUR_MS), 12 * HOUR_MS),
            Some(Ended::TooOld)
        );
        // An access token keeps its maker's end.
        let pat = Token {
            expires_at: Some(HOUR_MS),
            ..token(TokenKind::Pat, 0, None)
        };
        assert_eq!(
            ended(&p, &pat.in_use(HOUR_MS), HOUR_MS),
            Some(Ended::Expired)
        );
    }

    #[test]
    fn members_tokens_stop_where_members_make_none() {
        let p = SessionPolicy {
            member_access_tokens: false,
            ..policy()
        };
        let member = token(TokenKind::Pat, 0, None);
        assert_eq!(ended(&p, &member, 1), Some(Ended::MembersMakeNone));
        let admin = Token {
            member: false,
            ..member
        };
        assert_eq!(ended(&p, &admin, 1), None);
        // Their CLIs stay signed in.
        assert_eq!(ended(&p, &token(TokenKind::Cli, 0, None), 1), None);
        assert!(refusal(&p, TokenKind::Pat, Ended::MembersMakeNone).contains("fctl login"));
    }

    #[test]
    fn ended_as_kept_comes_first() {
        let t = Token {
            expires_at: Some(10),
            ..token(TokenKind::Pat, 0, None)
        };
        assert_eq!(ended(&policy(), &t, 10), Some(Ended::Expired));
        assert_eq!(ended(&policy(), &t, 9), None);
    }

    #[test]
    fn policies_are_checked_as_they_are_saved() {
        assert!(check(&policy()).is_ok());
        for bad in [
            SessionPolicy {
                browser_max_hours: MAX_BROWSER_MAX_HOURS + 1,
                ..policy()
            },
            SessionPolicy {
                browser_idle_minutes: MIN_BROWSER_IDLE_MINUTES - 1,
                ..policy()
            },
            SessionPolicy {
                browser_idle_minutes: DEFAULT_BROWSER_IDLE_MINUTES + 1,
                ..policy()
            },
            SessionPolicy {
                cli_max_days: MAX_CLI_MAX_DAYS + 1,
                ..policy()
            },
            SessionPolicy {
                access_token_max_days: MAX_ACCESS_TOKEN_DAYS + 1,
                ..policy()
            },
        ] {
            assert!(check(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn new_tokens_last_no_longer_than_the_policy_lets_them() {
        let p = SessionPolicy {
            access_token_max_days: 90,
            ..policy()
        };
        assert!(check_token_days(&p, Some(90)).is_ok());
        assert!(check_token_days(&p, Some(7)).is_ok());
        let refused = check_token_days(&p, Some(365)).err();
        assert_eq!(
            refused.map(|e| e.message),
            Some(
                "This instance's access tokens last 90 days at most: choose that or less"
                    .to_owned()
            )
        );
        let year = SessionPolicy {
            access_token_max_days: 365,
            ..policy()
        };
        assert!(
            check_token_days(&year, None)
                .err()
                .is_some_and(|e| e.message.contains("last 365 days at most"))
        );
        let day = SessionPolicy {
            access_token_max_days: 1,
            ..policy()
        };
        assert!(
            check_token_days(&day, Some(7))
                .err()
                .is_some_and(|e| e.message.contains("last 1 day at most"))
        );
        // For ever is longer than any.
        assert!(check_token_days(&p, None).is_err());
        assert!(check_token_days(&policy(), None).is_ok());
    }

    #[test]
    fn longer_sign_ins_and_members_tokens_relax_a_policy() {
        let strict = SessionPolicy {
            browser_max_hours: 12,
            browser_idle_minutes: 60,
            cli_max_days: 30,
            access_token_max_days: 90,
            member_access_tokens: false,
        };
        assert!(!relaxes(&strict, &strict));
        assert!(!relaxes(&policy(), &strict));
        for looser in [
            SessionPolicy {
                browser_max_hours: 0,
                ..strict.clone()
            },
            SessionPolicy {
                browser_max_hours: 24,
                ..strict.clone()
            },
            SessionPolicy {
                browser_idle_minutes: 120,
                ..strict.clone()
            },
            SessionPolicy {
                cli_max_days: 0,
                ..strict.clone()
            },
            SessionPolicy {
                access_token_max_days: 365,
                ..strict.clone()
            },
            SessionPolicy {
                member_access_tokens: true,
                ..strict.clone()
            },
        ] {
            assert!(relaxes(&strict, &looser), "{looser:?}");
        }
    }

    #[test]
    fn a_policy_reads_as_sentences() {
        assert_eq!(
            describe(&policy()),
            [
                "Browsers stay signed in while they're used, until 30 days unused.",
                "CLIs stay signed in while they're used, until 180 days unused.",
                "Everyone makes access tokens, each lasting as long as its maker chooses.",
            ]
        );
        let strict = SessionPolicy {
            browser_max_hours: 12,
            browser_idle_minutes: 60,
            cli_max_days: 30,
            access_token_max_days: 365,
            member_access_tokens: false,
        };
        assert_eq!(
            describe(&strict),
            [
                "Browsers sign in again after 12 hours, and after an hour unused.",
                "CLIs sign in again after 30 days, and after 180 days unused.",
                "Only admins make access tokens, each lasting a year at most.",
            ]
        );
    }
}

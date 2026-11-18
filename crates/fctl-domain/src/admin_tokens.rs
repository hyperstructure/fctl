//! Instance admin tokens (docs/architecture.md#admin-tokens, ADR 0085):
//! what a program provisions people with, on a self-hosted instance whose
//! deployment turns them on (`FCTL_ADMIN_TOKENS`), which app.fctl.sh's
//! never does.
//!
//! An instance admin makes one (`fctl_adm_…`), and it acts as them while
//! they are an active admin. It reaches the admin API's people endpoints
//! alone ([`allows`]): it creates accounts and reads them, changes their
//! names, addresses and commit emails, sets their pictures and Claude
//! tokens, adds them to any workspace and project and takes them out
//! again, disables, enables and deletes them, and makes their password
//! reset links. What reaches past what instance admins do in the browser
//! (creating accounts, changing what is someone's own, and placing them in
//! workspaces and projects) takes an admin token ([`only_tokens`]), and
//! never touches the primary instance owner's account, nor an admin's but
//! with one of the primary owner's tokens ([`check_target`]).

use crate::validate::Result;
use fctl_proto::ApiError;
use fctl_proto::session::InstanceRole;

/// The longest an admin token may last, in days, when it expires at all.
pub const MAX_DAYS: u32 = 3650;

/// Whether an admin token may call `method` on the API path after
/// `/api/v1/` (`segs`): the people endpoints of the admin API, the list of
/// workspaces to place people in, and what anyone may. Nothing else: not
/// the instance's settings, its allowlist, invitations, roles or other
/// tokens, and nothing inside workspaces and projects but who is in them.
pub fn allows(method: &str, segs: &[&str]) -> Result<()> {
    let reaches = matches!(
        (method, segs),
        ("GET", ["instance" | "health"])
            | ("GET", ["admin", "workspaces"])
            | ("GET" | "POST", ["admin", "users"])
            | ("GET" | "PATCH" | "DELETE", ["admin", "users", _])
            | ("POST", ["admin", "users", _, "reset-link"])
            | ("PUT" | "DELETE", ["admin", "users", _, "avatar" | "claude"])
            | (
                "PUT" | "DELETE",
                ["admin", "users", _, "workspaces" | "projects", _]
            )
    );
    if reaches {
        Ok(())
    } else {
        Err(ApiError::forbidden(
            "An admin token only provisions people: it creates, reads, changes and deletes accounts and places them in workspaces and projects (`/api/v1/admin/users`). Do the rest signed in as an admin.",
        ))
    }
}

/// Whether `method` on `segs` takes an admin token: what reaches past what
/// instance admins do signed in, who otherwise neither create accounts,
/// change what is someone's own nor look inside workspaces they aren't in.
pub fn only_tokens(method: &str, segs: &[&str]) -> bool {
    matches!(
        (method, segs),
        ("POST", ["admin", "users"])
            | ("GET", ["admin", "users", _])
            | ("PUT" | "DELETE", ["admin", "users", _, "avatar" | "claude"])
            | (
                "PUT" | "DELETE",
                ["admin", "users", _, "workspaces" | "projects", _]
            )
    )
}

/// What a request that takes an admin token answers otherwise: `enabled`,
/// the instance takes them.
pub fn needs_token(enabled: bool) -> ApiError {
    ApiError::forbidden(if enabled {
        "This takes an admin token: make one under Admin → Settings → Admin tokens, or with `fctl admin tokens create`, and send it as the bearer token."
    } else {
        "This takes an admin token, which this instance doesn't take: its deployment turns them on with FCTL_ADMIN_TOKENS=1."
    })
}

/// Whether an admin token whose maker's instance role is `maker` may change
/// the account of someone whose role is `target`, or where they are a
/// member: a member's, and an admin's with the primary instance owner's
/// token alone, as only they rename, disable and delete admins. No token
/// changes the primary instance owner's: whoever held one could otherwise
/// point their address elsewhere and take the instance over.
pub fn check_target(maker: InstanceRole, target: InstanceRole) -> Result<()> {
    match target {
        InstanceRole::Owner => Err(ApiError::forbidden(
            "Admin tokens don't change the primary instance owner's account: they change it themselves, in Settings.",
        )),
        InstanceRole::Admin if maker != InstanceRole::Owner => Err(ApiError::forbidden(
            "Only the primary instance owner's admin tokens change admins' accounts",
        )),
        _ => Ok(()),
    }
}

/// When a token made at `now` to last `days` (`None` or 0: for good)
/// expires.
pub fn expires_at(days: Option<u32>, now: i64) -> Result<Option<i64>> {
    match days {
        None | Some(0) => Ok(None),
        Some(d) if d <= MAX_DAYS => Ok(Some(now + i64::from(d) * 24 * 60 * 60 * 1000)),
        Some(_) => Err(ApiError::validation(
            "expires_in_days",
            format!("Use at most {MAX_DAYS} days"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(path: &str) -> Vec<&str> {
        path.split('/').collect()
    }

    #[test]
    fn reaches_the_people_endpoints_alone() {
        for (method, path) in [
            ("GET", "admin/users"),
            ("POST", "admin/users"),
            ("GET", "admin/users/usr_1"),
            ("PATCH", "admin/users/ada@example.com"),
            ("DELETE", "admin/users/usr_1"),
            ("POST", "admin/users/usr_1/reset-link"),
            ("PUT", "admin/users/usr_1/avatar"),
            ("DELETE", "admin/users/usr_1/claude"),
            ("PUT", "admin/users/usr_1/workspaces/acme"),
            ("DELETE", "admin/users/usr_1/projects/web"),
            ("GET", "admin/workspaces"),
            ("GET", "instance"),
        ] {
            assert!(allows(method, &segs(path)).is_ok(), "{method} {path}");
        }
        for (method, path) in [
            // Making more of itself, or tokens of anyone's.
            ("POST", "admin/tokens"),
            ("POST", "auth/tokens"),
            // Running the instance: who runs it, who gets in, its settings.
            ("POST", "admin/transfer"),
            ("POST", "admin/allowlist"),
            ("POST", "admin/invites"),
            ("PATCH", "admin/settings"),
            ("GET", "admin/audit"),
            ("DELETE", "admin/users/usr_1/two-factor"),
            ("POST", "admin/workspaces/wsp_1/delete"),
            // What's inside workspaces and projects, and its maker's own.
            ("GET", "projects"),
            ("GET", "sessions"),
            ("POST", "projects/web/sessions"),
            ("GET", "workspaces"),
            ("GET", "auth/me"),
            ("PATCH", "auth/me"),
            ("GET", "feed/ws"),
        ] {
            assert!(allows(method, &segs(path)).is_err(), "{method} {path}");
        }
    }

    #[test]
    fn what_reaches_past_admins_takes_a_token() {
        assert!(only_tokens("POST", &segs("admin/users")));
        assert!(only_tokens("GET", &segs("admin/users/usr_1")));
        assert!(only_tokens("PUT", &segs("admin/users/usr_1/claude")));
        assert!(only_tokens(
            "PUT",
            &segs("admin/users/usr_1/workspaces/acme")
        ));
        // What admins do signed in, too.
        assert!(!only_tokens("GET", &segs("admin/users")));
        assert!(!only_tokens("PATCH", &segs("admin/users/usr_1")));
        assert!(!only_tokens("DELETE", &segs("admin/users/usr_1")));
        assert!(!only_tokens("POST", &segs("admin/users/usr_1/reset-link")));
        // Every route only tokens take, tokens reach.
        for (method, path) in [
            ("POST", "admin/users"),
            ("GET", "admin/users/usr_1"),
            ("PUT", "admin/users/usr_1/avatar"),
            ("DELETE", "admin/users/usr_1/projects/web"),
        ] {
            assert!(allows(method, &segs(path)).is_ok(), "{method} {path}");
        }
    }

    #[test]
    fn nobodys_token_changes_the_primary_owner_and_admins_take_the_owners() {
        use InstanceRole::*;
        assert!(check_target(Owner, Member).is_ok());
        assert!(check_target(Admin, Member).is_ok());
        assert!(check_target(Owner, Admin).is_ok());
        assert!(check_target(Admin, Admin).is_err());
        assert!(check_target(Owner, Owner).is_err());
        assert!(check_target(Admin, Owner).is_err());
    }

    #[test]
    fn expires_when_asked_within_ten_years() {
        let day = 24 * 60 * 60 * 1000;
        assert_eq!(expires_at(None, 5), Ok(None));
        assert_eq!(expires_at(Some(0), 5), Ok(None));
        assert_eq!(expires_at(Some(90), 5), Ok(Some(5 + 90 * day)));
        assert!(expires_at(Some(MAX_DAYS + 1), 5).is_err());
    }
}

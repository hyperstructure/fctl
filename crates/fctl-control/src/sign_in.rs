//! The instance's sign-in providers and the rules that require one of the
//! people at some domains (`fctl_domain::sign_in`), as the control plane
//! applies them: what the sign-in page shows, how refusals read, and the
//! accounts a change to the rules concerns.

use crate::config::Config;
use crate::db::q;
use crate::error::AppResult;
use crate::model::UserRow;
use fctl_domain::sign_in::{GOOGLE, Person, Required, domains_words};
use fctl_proto::api::{
    InstanceSettings, SignInInfo, SignInProvider, SignInProviderStatus, SignInRequirement,
    TokenKind,
};
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use worker::D1Database;

/// The providers this control plane knows, by id, as their buttons name
/// them, and where their sign-in starts.
const PROVIDERS: [(&str, &str, &str); 1] = [(GOOGLE, "Google", "/api/v1/auth/google/start")];

/// A provider's name, as buttons and messages say it: `Google`.
pub fn provider_name(id: &str) -> &str {
    PROVIDERS
        .iter()
        .find(|(p, _, _)| *p == id)
        .map_or(id, |(_, name, _)| name)
}

/// Whether the instance's deployment set `id` up.
pub fn configured(cfg: &Config, id: &str) -> bool {
    match id {
        GOOGLE => cfg.google.is_some(),
        _ => false,
    }
}

/// The ids of the providers set up on this instance.
pub fn providers(cfg: &Config) -> Vec<&'static str> {
    PROVIDERS
        .iter()
        .map(|(id, _, _)| *id)
        .filter(|id| configured(cfg, id))
        .collect()
}

/// Every provider this control plane knows, as instance admins see them,
/// with `people`, the active accounts one of its accounts signs in to.
pub fn statuses(cfg: &Config, people: &[Person]) -> Vec<SignInProviderStatus> {
    PROVIDERS
        .iter()
        .map(|(id, name, _)| SignInProviderStatus {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            configured: configured(cfg, id),
            setup: match *id {
                GOOGLE => "GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET".to_owned(),
                _ => String::new(),
            },
            redirect_url: match *id {
                GOOGLE => crate::api::google::redirect_url(cfg),
                _ => String::new(),
            },
            people: people
                .iter()
                .filter(|p| p.identities.iter().any(|(provider, _)| provider == id))
                .count() as u32,
        })
        .collect()
}

/// What the sign-in page needs ([`fctl_proto::api::InstanceInfo::sign_in`]).
pub fn info(cfg: &Config, settings: &InstanceSettings) -> SignInInfo {
    SignInInfo {
        providers: PROVIDERS
            .iter()
            .filter(|(id, _, _)| configured(cfg, id))
            .map(|(id, name, start)| SignInProvider {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                start: (*start).to_owned(),
            })
            .collect(),
        rules: settings.sign_in.clone(),
        self_sign_up: settings.self_sign_up,
    }
}

/// What a rule requires, as refusals begin: "acme.com signs in with
/// Google".
pub fn says(required: &Required) -> String {
    format!(
        "{} {} in with {}",
        domains_words(&required.domains),
        if required.domains.len() == 1 {
            "signs"
        } else {
            "sign"
        },
        provider_name(&required.provider)
    )
}

/// The address to sign in as: "your acme.com address", "your acme.com or
/// acme.dev address".
fn as_address(required: &Required) -> String {
    let domains = match required.domains.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    };
    format!("your {domains} address")
}

/// The refusal of a sign-in with a password, a reset link's included, by
/// an account a rule covers.
pub fn password_refused(required: &Required) -> ApiError {
    ApiError::new(
        ErrorCode::AccessDenied,
        format!(
            "{}: continue with {} as {}. Passwords don't sign in to accounts there.",
            says(required),
            provider_name(&required.provider),
            as_address(required)
        ),
    )
}

/// The refusal of a sign-in with a passkey alone by an account whose rule
/// doesn't allow that.
pub fn passkey_refused(required: &Required) -> ApiError {
    ApiError::new(
        ErrorCode::AccessDenied,
        format!(
            "{}: continue with {} as {}. Your passkey still confirms it's you once you have.",
            says(required),
            provider_name(&required.provider),
            as_address(required)
        ),
    )
}

/// The refusal of a CLI's or an access token's request, whose sign-in
/// doesn't meet the rule that covers its person.
pub fn token_refused(required: &Required, kind: TokenKind) -> ApiError {
    let provider = provider_name(&required.provider);
    let next = match kind {
        TokenKind::Pat => format!(
            "This access token was made with a sign-in that wasn't: sign in with {provider} as {}, then make a new one.",
            as_address(required)
        ),
        TokenKind::Cli | TokenKind::Web => format!(
            "Sign in again with fctl login, then continue with {provider} as {}.",
            as_address(required)
        ),
    };
    ApiError::new(
        ErrorCode::Unauthorized,
        format!("{} now. {next}", says(required)),
    )
}

/// [`fctl_proto::api::Me::sign_in`] for `user`.
pub fn requirement(user: &UserRow, settings: &InstanceSettings) -> Option<SignInRequirement> {
    let addresses = user.addresses();
    let addresses: Vec<&str> = addresses.iter().map(String::as_str).collect();
    let rules = fctl_domain::sign_in::covering(&settings.sign_in, &addresses);
    let rule = rules.first()?;
    let required = fctl_domain::sign_in::required(&settings.sign_in, &addresses, false)?;
    Some(SignInRequirement {
        provider: rule.provider.clone(),
        provider_name: provider_name(&rule.provider).to_owned(),
        domains: required.domains,
        passkeys: rule.passkeys,
        exempt: user.is_owner(),
    })
}

#[derive(Deserialize)]
struct PersonRow {
    id: String,
    name: String,
    email: String,
    avatar_url: Option<String>,
    role: String,
    #[serde(deserialize_with = "crate::db::int_bool", default)]
    password: bool,
    #[serde(deserialize_with = "crate::db::int", default)]
    passkeys: i64,
    /// JSON array of addresses.
    #[serde(default)]
    emails: Option<String>,
    /// JSON array of `{provider, email}`.
    #[serde(default)]
    identities: Option<String>,
}

#[derive(Deserialize)]
struct IdentityJson {
    provider: Option<String>,
    email: Option<String>,
}

/// The active accounts, as the rules concern them: their addresses, ways
/// in and linked providers. Deleted accounts, which keep a row of their
/// own, aren't active.
pub async fn people(db: &D1Database) -> AppResult<Vec<Person>> {
    let rows: Vec<PersonRow> = q(
        db,
        "SELECT u.id, u.name, u.email, u.avatar_url, u.role, u.password_hash IS NOT NULL AS password, \
         (SELECT COUNT(*) FROM passkeys p WHERE p.user_id = u.id) AS passkeys, \
         (SELECT json_group_array(e.email) FROM user_emails e WHERE e.user_id = u.id) AS emails, \
         (SELECT json_group_array(json_object('provider', i.provider, 'email', i.email)) \
            FROM identities i WHERE i.user_id = u.id) AS identities \
         FROM users u WHERE u.status = 'active' ORDER BY u.name",
        &[],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let mut addresses: Vec<String> = r
                .emails
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<Option<String>>>(j).ok())
                .map(|v| v.into_iter().flatten().collect())
                .unwrap_or_default();
            if !addresses.contains(&r.email) {
                addresses.insert(0, r.email.clone());
            }
            let identities = r
                .identities
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<IdentityJson>>(j).ok())
                .map(|v| {
                    v.into_iter()
                        .filter_map(|i| Some((i.provider?, i.email)))
                        .collect()
                })
                .unwrap_or_default();
            Person {
                owner: r.role == "owner",
                password: r.password,
                passkeys: r.passkeys > 0,
                id: r.id,
                name: r.name,
                email: r.email,
                avatar_url: r.avatar_url,
                addresses,
                identities,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn required(domains: &[&str]) -> Required {
        Required {
            provider: GOOGLE.to_owned(),
            domains: domains.iter().map(|d| (*d).to_owned()).collect(),
            passkeys: false,
        }
    }

    #[test]
    fn refusals_name_the_provider_and_the_address_to_sign_in_as() {
        assert_eq!(
            says(&required(&["acme.com"])),
            "acme.com signs in with Google"
        );
        assert_eq!(
            says(&required(&["acme.com", "acme.dev"])),
            "acme.com and acme.dev sign in with Google"
        );
        assert_eq!(
            password_refused(&required(&["acme.com"])).message,
            "acme.com signs in with Google: continue with Google as your acme.com address. Passwords don't sign in to accounts there."
        );
        assert!(
            token_refused(&required(&["acme.com", "acme.dev"]), TokenKind::Pat)
                .message
                .contains("as your acme.com or acme.dev address, then make a new one")
        );
        assert!(
            token_refused(&required(&["acme.com"]), TokenKind::Cli)
                .message
                .contains("fctl login")
        );
        assert_eq!(provider_name("okta"), "okta");
    }
}

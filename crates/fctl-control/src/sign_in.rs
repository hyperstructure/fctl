//! The instance's sign-in providers and the rules that require one of the
//! people at some domains (`fctl_domain::sign_in`), as the control plane
//! applies them: what the sign-in page shows, how refusals read, and the
//! accounts a change to the rules concerns.

use crate::config::Config;
use crate::db::{b, n, q, s};
use crate::error::AppResult;
use crate::model::UserRow;
use fctl_domain::sign_in::{GOOGLE, Person, Required, SignedIn, domains_words};
use fctl_proto::api::{
    InstanceSettings, SessionPolicy, SignInInfo, SignInProvider, SignInProviderStatus,
    SignInRequirement, SignInRule, TokenKind,
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

/// The refusal of a sign-in made `how` that the rule covering its person
/// doesn't take: a password's, a passkey's or another provider account's.
pub fn refused(required: &Required, how: &SignedIn) -> ApiError {
    match &how.method {
        Some(fctl_domain::sign_in::Method::Passkey) => passkey_refused(required),
        Some(fctl_domain::sign_in::Method::Provider(_)) => ApiError::new(
            ErrorCode::AccessDenied,
            format!(
                "{}: continue with {} as {}. The account you signed in with isn't one.",
                says(required),
                provider_name(&required.provider),
                as_address(required)
            ),
        ),
        Some(fctl_domain::sign_in::Method::Password) | None => password_refused(required),
    }
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

/// The refusal of an admin token its admin made with a sign-in their
/// domain's rule doesn't meet.
pub fn admin_token_refused(required: &Required) -> ApiError {
    let provider = provider_name(&required.provider);
    ApiError::new(
        ErrorCode::Unauthorized,
        format!(
            "{} now, and this admin token was made with a sign-in that wasn't: its admin makes a new one after signing in with {provider} as {}.",
            says(required),
            as_address(required)
        ),
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
    /// JSON array of `{provider, domain}`.
    #[serde(default)]
    identities: Option<String>,
    /// JSON array of `{kind, method, domain}`: CLI sign-ins, access tokens
    /// and admin tokens.
    #[serde(default)]
    tokens: Option<String>,
}

#[derive(Deserialize)]
struct IdentityJson {
    provider: Option<String>,
    domain: Option<String>,
}

#[derive(Deserialize)]
struct TokenJson {
    kind: String,
    method: Option<String>,
    domain: Option<String>,
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
         (SELECT json_group_array(json_object('provider', i.provider, 'domain', i.vouched_domain)) \
            FROM identities i WHERE i.user_id = u.id) AS identities, \
         (SELECT json_group_array(json_object('kind', t.kind, 'method', t.auth_method, 'domain', t.auth_domain)) FROM \
            (SELECT kind, auth_method, auth_domain FROM tokens WHERE user_id = u.id AND kind IN ('cli', 'pat') \
             UNION ALL SELECT 'admin', auth_method, auth_domain FROM admin_tokens WHERE user_id = u.id) t) AS tokens \
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
                        .filter_map(|i| Some((i.provider?, i.domain)))
                        .collect()
                })
                .unwrap_or_default();
            let tokens = r
                .tokens
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<TokenJson>>(j).ok())
                .map(|v| {
                    v.into_iter()
                        .map(|t| {
                            let how = SignedIn::read(t.method.as_deref(), t.domain.as_deref());
                            (t.kind, how)
                        })
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
                tokens,
            }
        })
        .collect())
}

/// Unlinks `user_id`'s accounts of `provider`, and signs them out
/// everywhere, as a password reset does: the sessions those accounts made
/// may be someone else's, where the address went to someone new. How many
/// accounts it unlinked; none, and it changes nothing.
pub async fn unlink(db: &D1Database, user_id: &str, provider: &str) -> AppResult<i64> {
    let linked: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM identities WHERE user_id = ? AND provider = ?",
        &[s(user_id), s(provider)],
    )?
    .first()
    .await?;
    let n = linked.map_or(0, |c| c.n);
    if n == 0 {
        return Ok(0);
    }
    let mut statements = vec![q(
        db,
        "DELETE FROM identities WHERE user_id = ? AND provider = ?",
        &[s(user_id), s(provider)],
    )?];
    statements.extend(crate::auth::sign_out_everywhere(db, user_id, None)?);
    crate::db::batch(db, statements).await?;
    Ok(n)
}

/// What deletes, once `rules` are saved, the sign-ins of the accounts they
/// cover that they don't meet: browser sessions, CLI sign-ins, access
/// tokens and admin tokens made another way, those from before fctl kept
/// how included. They no longer let anyone in, and none comes back when a
/// rule goes. The primary instance owner keeps theirs. One statement for
/// each table, whatever the number of people.
pub fn revoke_unmet(db: &D1Database, rules: &[SignInRule]) -> AppResult<Vec<crate::db::Query>> {
    let rules = serde_json::to_string(rules)?;
    // The domains of the addresses of the account `{who}` names.
    let domains = |who: &str| {
        format!(
            "SELECT lower(substr(e.email, instr(e.email, '@') + 1)) FROM user_emails e WHERE e.user_id = {who} \
             UNION SELECT lower(substr(u.email, instr(u.email, '@') + 1)) FROM users u WHERE u.id = {who}"
        )
    };
    let mut statements = Vec::new();
    for table in ["tokens", "admin_tokens"] {
        statements.push(q(
            db,
            &format!(
                "DELETE FROM {table} WHERE user_id IN (SELECT o.id FROM users o WHERE o.role != 'owner') \
                 AND EXISTS (SELECT 1 FROM json_each(?1) r, json_each(r.value, '$.domains') d \
                   WHERE d.value IN ({mine})) \
                 AND NOT EXISTS (SELECT 1 FROM json_each(?1) r \
                   WHERE EXISTS (SELECT 1 FROM json_each(r.value, '$.domains') d WHERE d.value IN ({mine})) \
                   AND ((COALESCE({table}.auth_method, '') = json_extract(r.value, '$.provider') \
                         AND COALESCE({table}.auth_domain, '') IN (SELECT value FROM json_each(r.value, '$.domains'))) \
                     OR (json_extract(r.value, '$.passkeys') AND COALESCE({table}.auth_method, '') = 'passkey')))",
                mine = domains(&format!("{table}.user_id")),
            ),
            &[s(&rules)],
        )?);
    }
    Ok(statements)
}

/// The live browser and CLI sign-ins and access tokens that `policy`
/// ends at `now`, as a condition on `tokens` and its parameters
/// (`fctl_domain::session_policy::ended`, which every request checks).
fn ended_by(policy: &SessionPolicy, now: i64) -> (&'static str, Vec<wasm_bindgen::JsValue>) {
    use fctl_domain::session_policy::{idle_ms, max_age_ms};
    let max = |kind| n(max_age_ms(policy, kind).unwrap_or(0));
    (
        "(expires_at IS NULL OR expires_at > ?1) AND (            (kind = 'web' AND ((?2 > 0 AND created_at + ?2 <= ?1) OR COALESCE(last_used_at, created_at) + ?3 <= ?1))            OR (kind = 'cli' AND ?4 > 0 AND created_at + ?4 <= ?1)            OR (kind = 'pat' AND ((?5 > 0 AND created_at + ?5 <= ?1)              OR (?6 AND user_id IN (SELECT id FROM users WHERE role = 'member')))))",
        vec![
            n(now),
            max(TokenKind::Web),
            n(idle_ms(policy, TokenKind::Web).unwrap_or(i64::MAX / 2)),
            max(TokenKind::Cli),
            max(TokenKind::Pat),
            b(!policy.member_access_tokens),
        ],
    )
}

/// What saving `policy` ends at once: browser and CLI sign-ins older than
/// it lets them be, browsers unused longer, and access tokens older than
/// it lets them be, or members' where members make none. Counted by kind
/// and by whether a member of the instance holds it.
pub async fn ended_by_policy(
    db: &D1Database,
    policy: &SessionPolicy,
    now: i64,
) -> AppResult<Vec<(TokenKind, bool, u32)>> {
    #[derive(Deserialize)]
    struct Row {
        kind: String,
        #[serde(deserialize_with = "crate::db::int")]
        member: i64,
        #[serde(deserialize_with = "crate::db::int")]
        n: i64,
    }
    let (ended, params) = ended_by(policy, now);
    let rows: Vec<Row> = q(
        db,
        &format!(
            "SELECT kind, (user_id IN (SELECT id FROM users WHERE role = 'member')) AS member, COUNT(*) AS n              FROM tokens WHERE {ended} GROUP BY kind, member"
        ),
        &params,
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| Some((TokenKind::parse(&r.kind)?, r.member != 0, r.n as u32)))
        .collect())
}

/// What deletes, once `policy` is saved, the sign-ins and access tokens it
/// ends ([`ended_by_policy`]): they no longer let anyone in, and none
/// comes back when the policy is relaxed.
pub fn revoke_ended(
    db: &D1Database,
    policy: &SessionPolicy,
    now: i64,
) -> AppResult<crate::db::Query> {
    let (ended, params) = ended_by(policy, now);
    q(db, &format!("DELETE FROM tokens WHERE {ended}"), &params)
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

#[cfg(all(test, target_arch = "wasm32"))]
mod d1_tests {
    use super::*;
    use crate::db::batch;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn left(db: &D1Database, table: &str) -> AppResult<Vec<String>> {
        #[derive(Deserialize)]
        struct Id {
            id: String,
        }
        let rows: Vec<Id> = q(db, &format!("SELECT id FROM {table} ORDER BY id"), &[])?
            .all()
            .await?;
        Ok(rows.into_iter().map(|r| r.id).collect())
    }

    #[wasm_bindgen_test]
    async fn unlinking_signs_its_person_out_everywhere() -> AppResult<()> {
        let db = testing::database();
        for sql in [
            "INSERT INTO users (id, email, name, role, status, created_at, updated_at) VALUES \
             ('usr_bob', 'bob@acme.dev', 'Bob', 'admin', 'active', 0, 0), \
             ('usr_cy', 'cy@acme.dev', 'Cy', 'member', 'active', 0, 0)",
            "INSERT INTO identities (id, user_id, provider, subject, email, created_at, vouched_domain) VALUES \
             ('idn_bob_old', 'usr_bob', 'google', 'g-bob-old', 'bob@acme.dev', 0, 'acme.dev'), \
             ('idn_bob_gmail', 'usr_bob', 'google', 'g-bob-gmail', 'bob@gmail.com', 0, 'gmail.com'), \
             ('idn_cy', 'usr_cy', 'google', 'g-cy', 'cy@acme.dev', 0, 'acme.dev')",
            "INSERT INTO tokens (id, hash, kind, user_id, created_at, auth_method, auth_domain) VALUES \
             ('tok_bob_web', 'h1', 'web', 'usr_bob', 0, 'google', 'acme.dev'), \
             ('tok_bob_pat', 'h2', 'pat', 'usr_bob', 0, 'google', 'acme.dev'), \
             ('tok_cy', 'h3', 'web', 'usr_cy', 0, 'google', 'acme.dev')",
            "INSERT INTO admin_tokens (id, hash, name, user_id, created_at, auth_method, auth_domain) VALUES \
             ('adt_bob', 'h4', 'Sync', 'usr_bob', 0, 'google', 'acme.dev')",
            "INSERT INTO device_codes (device_hash, user_code, client_name, status, user_id, created_at, expires_at) VALUES \
             ('dev_bob', 'BCDF-GHJK', 'laptop', 'approved', 'usr_bob', 0, 9000000000000)",
        ] {
            q(&db, sql, &[])?.run().await?;
        }
        assert_eq!(unlink(&db, "usr_bob", GOOGLE).await?, 2);
        assert_eq!(left(&db, "identities").await?, ["idn_cy"]);
        // His sessions, access and admin tokens go, and the CLI sign-in he
        // approved signs no CLI in; Cy's stay.
        assert_eq!(left(&db, "tokens").await?, ["tok_cy"]);
        assert!(left(&db, "admin_tokens").await?.is_empty());
        #[derive(Deserialize)]
        struct Status {
            status: String,
        }
        let code: Option<Status> = q(
            &db,
            "SELECT status FROM device_codes WHERE device_hash = 'dev_bob'",
            &[],
        )?
        .first()
        .await?;
        assert_eq!(code.map(|c| c.status).as_deref(), Some("denied"));
        // Nothing left to unlink: nothing changes.
        assert_eq!(unlink(&db, "usr_bob", GOOGLE).await?, 0);
        assert_eq!(left(&db, "tokens").await?, ["tok_cy"]);
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn saving_rules_deletes_the_sign_ins_they_dont_meet() -> AppResult<()> {
        let db = testing::database();
        for sql in [
            "INSERT INTO users (id, email, name, role, status, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@acme.dev', 'Ada', 'owner', 'active', 0, 0), \
             ('usr_bob', 'bob@acme.dev', 'Bob', 'admin', 'active', 0, 0), \
             ('usr_cy', 'cy@gmail.com', 'Cy', 'member', 'active', 0, 0), \
             ('usr_eve', 'eve@contractor.io', 'Eve', 'member', 'active', 0, 0)",
            // Cy's work address is connected to his account.
            "INSERT INTO user_emails (email, user_id, created_at) VALUES ('ada@acme.dev', 'usr_ada', 0), \
             ('bob@acme.dev', 'usr_bob', 0), ('cy@gmail.com', 'usr_cy', 0), ('cy@ACME.dev', 'usr_cy', 0), \
             ('eve@contractor.io', 'usr_eve', 0)",
            "INSERT INTO tokens (id, hash, kind, user_id, created_at, auth_method, auth_domain) VALUES \
             ('tok_ada', 'h1', 'web', 'usr_ada', 0, 'password', NULL), \
             ('tok_bob_old', 'h2', 'pat', 'usr_bob', 0, NULL, NULL), \
             ('tok_bob_pw', 'h3', 'cli', 'usr_bob', 0, 'password', NULL), \
             ('tok_bob_gmail', 'h4', 'web', 'usr_bob', 0, 'google', 'gmail.com'), \
             ('tok_bob_google', 'h5', 'web', 'usr_bob', 0, 'google', 'acme.dev'), \
             ('tok_bob_passkey', 'h6', 'web', 'usr_bob', 0, 'passkey', NULL), \
             ('tok_cy_pw', 'h7', 'web', 'usr_cy', 0, 'password', NULL), \
             ('tok_eve_pw', 'h8', 'pat', 'usr_eve', 0, 'password', NULL)",
            "INSERT INTO admin_tokens (id, hash, name, user_id, created_at, auth_method, auth_domain) VALUES \
             ('adt_ada', 'h9', 'Sync', 'usr_ada', 0, NULL, NULL), \
             ('adt_bob_old', 'h10', 'Okta', 'usr_bob', 0, NULL, NULL), \
             ('adt_bob_google', 'h11', 'HR', 'usr_bob', 0, 'google', 'acme.dev')",
        ] {
            q(&db, sql, &[])?.run().await?;
        }
        let rule = |passkeys: bool| SignInRule {
            domains: vec!["acme.dev".to_owned()],
            provider: GOOGLE.to_owned(),
            passkeys,
        };
        // Passkeys let in: Bob's passkey sign-in stays.
        batch(&db, revoke_unmet(&db, &[rule(true)])?).await?;
        assert_eq!(
            left(&db, "tokens").await?,
            ["tok_ada", "tok_bob_google", "tok_bob_passkey", "tok_eve_pw"]
        );
        // Then not: it goes too. The owner's and the contractor's stay.
        batch(&db, revoke_unmet(&db, &[rule(false)])?).await?;
        assert_eq!(
            left(&db, "tokens").await?,
            ["tok_ada", "tok_bob_google", "tok_eve_pw"]
        );
        assert_eq!(
            left(&db, "admin_tokens").await?,
            ["adt_ada", "adt_bob_google"]
        );
        // No rule: nothing more goes.
        batch(&db, revoke_unmet(&db, &[])?).await?;
        assert_eq!(left(&db, "tokens").await?.len(), 3);
        Ok(())
    }

    /// What saving a session policy ends is what every request refuses
    /// under it (`fctl_domain::session_policy::ended`).
    #[wasm_bindgen_test]
    async fn saving_a_session_policy_ends_what_it_doesnt_let_last() -> AppResult<()> {
        use fctl_domain::session_policy::{self as sp, DAY_MS, HOUR_MS};
        let db = testing::database();
        let now = 100 * DAY_MS;
        q(
            &db,
            "INSERT INTO users (id, email, name, role, status, created_at, updated_at) VALUES              ('usr_ada', 'ada@acme.dev', 'Ada', 'owner', 'active', 0, 0),              ('usr_bob', 'bob@acme.dev', 'Bob', 'member', 'active', 0, 0)",
            &[],
        )?
        .run()
        .await?;
        // id, kind, user, created, last used, expires.
        let tokens: [(&str, TokenKind, &str, i64, Option<i64>, Option<i64>); 9] = [
            // A browser signed in 13 hours ago, used a minute ago.
            (
                "tok_web_old",
                TokenKind::Web,
                "usr_ada",
                now - 13 * HOUR_MS,
                Some(now - 60_000),
                Some(now + DAY_MS),
            ),
            // One signed in an hour ago, unused for 50 minutes.
            (
                "tok_web_idle",
                TokenKind::Web,
                "usr_bob",
                now - HOUR_MS,
                Some(now - 50 * 60_000),
                Some(now + DAY_MS),
            ),
            (
                "tok_web_new",
                TokenKind::Web,
                "usr_bob",
                now - HOUR_MS,
                Some(now - 60_000),
                Some(now + DAY_MS),
            ),
            (
                "tok_cli_old",
                TokenKind::Cli,
                "usr_ada",
                now - 40 * DAY_MS,
                Some(now),
                Some(now + DAY_MS),
            ),
            (
                "tok_cli_new",
                TokenKind::Cli,
                "usr_bob",
                now - 20 * DAY_MS,
                Some(now),
                Some(now + DAY_MS),
            ),
            // Access tokens: an admin's for ever, 100 days old; a member's.
            ("tok_pat_ada", TokenKind::Pat, "usr_ada", 0, None, None),
            (
                "tok_pat_ada_new",
                TokenKind::Pat,
                "usr_ada",
                now - DAY_MS,
                None,
                None,
            ),
            (
                "tok_pat_bob",
                TokenKind::Pat,
                "usr_bob",
                now - DAY_MS,
                None,
                None,
            ),
            // Expired already: no change counts it.
            (
                "tok_gone",
                TokenKind::Web,
                "usr_bob",
                0,
                Some(0),
                Some(now - 1),
            ),
        ];
        for (id, kind, user, created, used, expires) in tokens {
            q(
                &db,
                "INSERT INTO tokens (id, hash, kind, user_id, created_at, last_used_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                &[s(id), s(id), s(kind.as_str()), s(user), n(created), crate::db::on(used), crate::db::on(expires)],
            )?
            .run()
            .await?;
        }
        let policy = SessionPolicy {
            browser_max_hours: 12,
            browser_idle_minutes: 30,
            cli_max_days: 30,
            access_token_max_days: 90,
            member_access_tokens: false,
        };
        // As each request would read them.
        let refused: Vec<&str> = tokens
            .iter()
            .filter(|(_, kind, user, created, used, expires)| {
                expires.is_none_or(|e| e > now)
                    && sp::ended(
                        &policy,
                        &sp::Token {
                            kind: *kind,
                            created_at: *created,
                            last_used_at: *used,
                            expires_at: *expires,
                            member: *user == "usr_bob",
                        },
                        now,
                    )
                    .is_some()
            })
            .map(|t| t.0)
            .collect();
        assert_eq!(
            refused,
            [
                "tok_web_old",
                "tok_web_idle",
                "tok_cli_old",
                "tok_pat_ada",
                "tok_pat_bob"
            ]
        );
        let mut counted = ended_by_policy(&db, &policy, now).await?;
        counted.sort_by_key(|(kind, member, _)| (kind.as_str(), *member));
        assert_eq!(
            counted,
            [
                (TokenKind::Cli, false, 1),
                (TokenKind::Pat, false, 1),
                (TokenKind::Pat, true, 1),
                (TokenKind::Web, false, 1),
                (TokenKind::Web, true, 1),
            ]
        );
        revoke_ended(&db, &policy, now)?.run().await?;
        assert_eq!(
            left(&db, "tokens").await?,
            ["tok_cli_new", "tok_gone", "tok_pat_ada_new", "tok_web_new"]
        );
        // The defaults end nothing more.
        assert!(
            ended_by_policy(&db, &SessionPolicy::default(), now)
                .await?
                .is_empty()
        );
        Ok(())
    }
}

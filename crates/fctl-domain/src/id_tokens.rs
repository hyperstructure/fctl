//! Identity tokens (docs/identity-tokens.md, ADR 0090): what the OpenID
//! Connect ID token an agent mints says (`fctl id-token`), how it is put
//! together, which of the instance's keys sign it and which its issuers
//! publish. The control plane signs (`crate::id_tokens` there, with
//! WebCrypto); this is the rest.
//!
//! Each workspace is an issuer of its own, `{FCTL_PUBLIC_URL}/oidc/{wsp_…}`,
//! so that a cloud set up to trust one workspace's agents trusts no other
//! workspace's on the same instance, whatever its conditions forget. The
//! instance's keys sign for all of them: a token's `iss` is signed too, so
//! none moves from one workspace's issuer to another's.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fctl_proto::hooks::WebhookOrigin;
use fctl_proto::identity::SubjectPart;
use fctl_proto::session::GithubAccess;
use fctl_proto::{ApiError, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// How long a token lasts: long enough to trade for a cloud's credentials,
/// short enough that one left in a log or a transcript is soon worth
/// nothing.
pub const LIFETIME_SECS: i64 = 10 * 60;

/// A token counts from this long before it was signed (`nbf`), for
/// verifiers whose clocks run a little behind.
pub const SKEW_SECS: i64 = 5;

/// The signature algorithm: RS256, which every cloud and JWT library takes.
pub const ALGORITHM: &str = "RS256";

/// The size of the instance's RSA keys.
pub const MODULUS_BITS: u32 = 2048;

/// The longest audience and nonce an agent may ask for.
pub const MAX_AUDIENCE_LEN: usize = 512;
pub const MAX_NONCE_LEN: usize = 512;

/// The claim AWS reads session tags from (`AssumeRoleWithWebIdentity`).
pub const AWS_TAGS_CLAIM: &str = "https://aws.amazon.com/tags";

/// Where the instance's issuers live: `/oidc/{wsp_…}`.
pub const ISSUER_PATH: &str = "/oidc/";
/// An issuer's discovery document and keys, after its address.
pub const DISCOVERY_PATH: &str = "/.well-known/openid-configuration";
pub const JWKS_PATH: &str = "/.well-known/jwks.json";

const DAY_MS: i64 = 24 * 3600 * 1000;
/// A key signs for about this long before another takes over.
pub const ROTATE_AFTER_MS: i64 = 30 * DAY_MS;
/// A new key is published this long before it signs, so that verifiers
/// that keep the keys for a while have it before they meet it.
pub const PUBLISH_AHEAD_MS: i64 = 2 * DAY_MS;
/// A key stays published this long after it stopped signing: far past the
/// last token it signed.
pub const RETIRED_KEPT_MS: i64 = 2 * DAY_MS;

/// The issuer of workspace `workspace_id`'s tokens on the instance at
/// `public_url`.
pub fn issuer(public_url: &str, workspace_id: &str) -> String {
    format!(
        "{}{ISSUER_PATH}{workspace_id}",
        public_url.trim_end_matches('/')
    )
}

pub fn discovery_url(issuer: &str) -> String {
    format!("{issuer}{DISCOVERY_PATH}")
}

pub fn jwks_url(issuer: &str) -> String {
    format!("{issuer}{JWKS_PATH}")
}

/// The issuer without its scheme: what AWS names an identity provider by,
/// and its condition keys after (`app.fctl.sh/oidc/wsp_…:sub`).
pub fn issuer_host_path(issuer: &str) -> &str {
    issuer
        .strip_prefix("https://")
        .or_else(|| issuer.strip_prefix("http://"))
        .unwrap_or(issuer)
}

/// What an issuer serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Document {
    Discovery,
    Keys,
}

/// The workspace and document a path under [`ISSUER_PATH`] asks for:
/// `/oidc/wsp_…/.well-known/openid-configuration` or `…/jwks.json`.
pub fn document(path: &str) -> Option<(&str, Document)> {
    let rest = path.strip_prefix(ISSUER_PATH)?;
    let (workspace, doc) = rest.split_once('/')?;
    let doc = match format!("/{doc}").as_str() {
        DISCOVERY_PATH => Document::Discovery,
        JWKS_PATH => Document::Keys,
        _ => return None,
    };
    WorkspaceId::is_valid(workspace).then_some((workspace, doc))
}

/// What started a session, as its tokens say (`started_by`), in the words
/// and order webhooks say it (`crate::hooks::origin`): an autopilot, a
/// schedule, a ticket delegated to fctl, a service account, or else its
/// person, with their own hands or one of their access tokens.
pub fn started_by(
    autopilot: bool,
    schedule: bool,
    ticket: bool,
    service_account: bool,
) -> WebhookOrigin {
    if autopilot {
        WebhookOrigin::Autopilot
    } else if schedule {
        WebhookOrigin::Schedule
    } else if ticket {
        WebhookOrigin::Ticket
    } else if service_account {
        WebhookOrigin::ServiceAccount
    } else {
        WebhookOrigin::Person
    }
}

/// What the control plane knows of a session as it mints a token for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub workspace_id: String,
    pub workspace_slug: String,
    pub project_id: String,
    pub project_slug: String,
    pub session_id: String,
    pub session_number: u32,
    pub session_url: String,
    /// The person it runs for: who started it, or for whom a schedule,
    /// an autopilot, a delegated ticket or a service account started it.
    pub user_id: String,
    /// The address they are a member of the workspace as.
    pub user_email: Option<String>,
    pub runner_id: String,
    pub runner_name: Option<String>,
    /// A runner its owner added for their own sessions, which runs no one
    /// else's in the workspace.
    pub runner_personal: bool,
    /// The project's primary repository, `owner/name`.
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub github_access: GithubAccess,
    pub started_by: WebhookOrigin,
    pub schedule_id: Option<String>,
    /// The ticket it started from: `ENG-123`, `acme/web#12`.
    pub ticket: Option<String>,
    /// The autopilot it belongs to (its project), and how: `pilot` for its
    /// own session, `crew` for one it started.
    pub autopilot_id: Option<String>,
    pub autopilot_role: Option<String>,
    pub service_account_id: Option<String>,
}

/// What an agent asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ask<'a> {
    pub audience: &'a str,
    pub nonce: Option<&'a str>,
    /// The parts of its subject; none for the project and the person.
    pub subject: &'a [SubjectPart],
    pub aws_session_tags: bool,
}

/// Printable ASCII without spaces, as audiences and nonces are.
fn printable(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_graphic())
}

/// Checks what an agent asks for: an audience and a nonce of printable
/// ASCII without spaces, not too long.
pub fn check(ask: &Ask) -> Result<(), ApiError> {
    let audience = ask.audience;
    if audience.is_empty() {
        return Err(ApiError::validation(
            "audience",
            "Say whom the token is for: its audience, such as sts.amazonaws.com or your service's address.",
        ));
    }
    if audience.len() > MAX_AUDIENCE_LEN || !printable(audience) {
        return Err(ApiError::validation(
            "audience",
            format!(
                "An audience is printable ASCII without spaces, up to {MAX_AUDIENCE_LEN} characters."
            ),
        ));
    }
    if let Some(nonce) = ask.nonce
        && (nonce.is_empty() || nonce.len() > MAX_NONCE_LEN || !printable(nonce))
    {
        return Err(ApiError::validation(
            "nonce",
            format!("A nonce is printable ASCII without spaces, up to {MAX_NONCE_LEN} characters."),
        ));
    }
    Ok(())
}

/// The parts of a subject as a token has them: those asked for, once each,
/// in [`SubjectPart::ALL`]'s order; the project and the person when none
/// are.
pub fn subject_parts(asked: &[SubjectPart]) -> Vec<SubjectPart> {
    if asked.is_empty() {
        return vec![SubjectPart::Project, SubjectPart::User];
    }
    SubjectPart::ALL
        .into_iter()
        .filter(|p| asked.contains(p))
        .collect()
}

/// A token's subject: `project:prj_…:user:usr_…`, or the parts asked for.
/// No value has a `:`, so a pattern can't match one part as another.
pub fn subject(facts: &Facts, asked: &[SubjectPart]) -> String {
    subject_parts(asked)
        .into_iter()
        .map(|part| {
            let value = match part {
                SubjectPart::Workspace => facts.workspace_id.as_str(),
                SubjectPart::Project => facts.project_id.as_str(),
                SubjectPart::User => facts.user_id.as_str(),
                SubjectPart::Session => facts.session_id.as_str(),
                SubjectPart::Runner => facts.runner_id.as_str(),
                SubjectPart::GithubAccess => facts.github_access.as_str(),
            };
            format!("{}:{value}", part.as_str())
        })
        .collect::<Vec<_>>()
        .join(":")
}

/// The subject of a project's agents' tokens, with `*` for the person: the
/// pattern a cloud matches to trust them all.
pub fn project_subject(project_id: &str) -> String {
    format!("project:{project_id}:user:*")
}

/// AWS session tags, in the form `AssumeRoleWithWebIdentity` reads them:
/// each a list of one value, all of them transitive, so that a role the
/// session assumes from there keeps them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsTags {
    pub principal_tags: std::collections::BTreeMap<String, Vec<String>>,
    pub transitive_tag_keys: Vec<String>,
}

/// The session tags of an AWS token: `fctl:workspace`, `fctl:project`,
/// `fctl:user`, `fctl:session`, `fctl:runner`, `fctl:github_access` and
/// `fctl:started_by`. Ids and words only, which AWS takes as tag values
/// and which say nothing about anyone that the logs shouldn't keep.
pub fn aws_tags(facts: &Facts) -> AwsTags {
    let tags = [
        ("fctl:workspace", facts.workspace_id.as_str()),
        ("fctl:project", facts.project_id.as_str()),
        ("fctl:user", facts.user_id.as_str()),
        ("fctl:session", facts.session_id.as_str()),
        ("fctl:runner", facts.runner_id.as_str()),
        ("fctl:github_access", facts.github_access.as_str()),
        ("fctl:started_by", facts.started_by.as_str()),
    ];
    AwsTags {
        principal_tags: tags
            .iter()
            .map(|(k, v)| ((*k).to_owned(), vec![(*v).to_owned()]))
            .collect(),
        transitive_tag_keys: tags.iter().map(|(k, _)| (*k).to_owned()).collect(),
    }
}

/// What a token says (docs/identity-tokens.md#claims).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    pub workspace_id: String,
    pub workspace_slug: String,
    pub project_id: String,
    pub project_slug: String,
    pub session_id: String,
    pub session_number: u32,
    pub session_url: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
    pub runner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_name: Option<String>,
    pub runner_personal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    pub github_access: GithubAccess,
    pub started_by: WebhookOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autopilot_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account_id: Option<String>,
    #[serde(
        rename = "https://aws.amazon.com/tags",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub aws_tags: Option<AwsTags>,
}

/// Every claim a token may have, as the discovery document lists them.
pub const CLAIMS: &[&str] = &[
    "iss",
    "sub",
    "aud",
    "iat",
    "nbf",
    "exp",
    "jti",
    "nonce",
    "workspace_id",
    "workspace_slug",
    "project_id",
    "project_slug",
    "session_id",
    "session_number",
    "session_url",
    "user_id",
    "user_email",
    "runner_id",
    "runner_name",
    "runner_personal",
    "repository",
    "branch",
    "base_branch",
    "github_access",
    "started_by",
    "schedule_id",
    "ticket",
    "autopilot_id",
    "autopilot_role",
    "service_account_id",
    AWS_TAGS_CLAIM,
];

/// The claims of a token for `facts`, as `ask` asks, signed at `now_ms`
/// by `issuer`, with the unique id `jti`.
pub fn claims(facts: &Facts, issuer: &str, ask: &Ask, jti: &str, now_ms: i64) -> Claims {
    let iat = now_ms.div_euclid(1000);
    let f = facts.clone();
    Claims {
        iss: issuer.to_owned(),
        sub: subject(facts, ask.subject),
        aud: ask.audience.to_owned(),
        iat,
        nbf: iat - SKEW_SECS,
        exp: iat + LIFETIME_SECS,
        jti: jti.to_owned(),
        nonce: ask.nonce.map(str::to_owned),
        aws_tags: ask.aws_session_tags.then(|| aws_tags(facts)),
        workspace_id: f.workspace_id,
        workspace_slug: f.workspace_slug,
        project_id: f.project_id,
        project_slug: f.project_slug,
        session_id: f.session_id,
        session_number: f.session_number,
        session_url: f.session_url,
        user_id: f.user_id,
        user_email: f.user_email,
        runner_id: f.runner_id,
        runner_name: f.runner_name,
        runner_personal: f.runner_personal,
        repository: f.repository,
        branch: f.branch,
        base_branch: f.base_branch,
        github_access: f.github_access,
        started_by: f.started_by,
        schedule_id: f.schedule_id,
        ticket: f.ticket,
        autopilot_id: f.autopilot_id,
        autopilot_role: f.autopilot_role,
        service_account_id: f.service_account_id,
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// What the key `kid` signs: `base64url(header).base64url(claims)`.
pub fn signing_input(kid: &str, claims: &Claims) -> String {
    let header = json!({ "alg": ALGORITHM, "typ": "JWT", "kid": kid });
    let payload = serde_json::to_vec(claims).unwrap_or_default();
    format!("{}.{}", b64(header.to_string().as_bytes()), b64(&payload))
}

/// The token: what was signed, then the signature.
pub fn token(signing_input: &str, signature: &[u8]) -> String {
    format!("{signing_input}.{}", b64(signature))
}

/// A token's claims, read without checking its signature: for showing
/// what one says, never for trusting it.
pub fn read_claims(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let (_, payload, _, None) = (parts.next()?, parts.next()?, parts.next()?, parts.next()) else {
        return None;
    };
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A key's id: its RFC 7638 thumbprint, the base64url SHA-256 of its
/// public members in order.
pub fn thumbprint(n: &str, e: &str) -> String {
    let canonical = format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#);
    b64(&Sha256::digest(canonical.as_bytes()))
}

/// A public key as an issuer's key set lists it.
pub fn public_jwk(kid: &str, n: &str, e: &str) -> Value {
    json!({ "kty": "RSA", "use": "sig", "alg": ALGORITHM, "kid": kid, "n": n, "e": e })
}

/// An issuer's key set.
pub fn jwks(keys: Vec<Value>) -> Value {
    json!({ "keys": keys })
}

/// An issuer's discovery document (OpenID Connect Discovery 1.0): no
/// authorization or token endpoint, since its tokens come from sandboxes
/// alone, as GitHub Actions' and Cursor's do.
pub fn discovery(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "jwks_uri": jwks_url(issuer),
        "response_types_supported": ["id_token"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": [ALGORITHM],
        "scopes_supported": ["openid"],
        "claims_supported": CLAIMS,
        "x_fctl_subject_parts_supported": SubjectPart::ALL.map(SubjectPart::as_str),
    })
}

/// One of the instance's keys, as [`plan`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyAge {
    pub kid: String,
    pub created_at: i64,
}

/// What to do with the instance's keys at a moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyPlan {
    /// The key that signs now.
    pub signer: Option<String>,
    /// The keys the issuers list: those to come, the signer and those that
    /// stopped signing lately.
    pub published: Vec<String>,
    /// Those no longer listed, to delete.
    pub expired: Vec<String>,
    /// The newest key is old enough for another to follow it, or there is
    /// none: make one.
    pub make_new: bool,
}

/// Which key signs at `now`, which the issuers publish and whether to make
/// one: a new key is published [`PUBLISH_AHEAD_MS`] before it signs,
/// takes over [`ROTATE_AFTER_MS`] after the last one was made, and stays
/// published [`RETIRED_KEPT_MS`] after it stops signing. The first key
/// signs at once: nobody can have kept a list without it.
pub fn plan(keys: &[KeyAge], now: i64) -> KeyPlan {
    let mut keys: Vec<&KeyAge> = keys.iter().collect();
    // Newest first, and in a stable order for keys made in one millisecond.
    keys.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.kid.cmp(&b.kid))
    });
    let make_new = keys
        .first()
        .is_none_or(|k| now - k.created_at >= ROTATE_AFTER_MS);
    let signer = keys
        .iter()
        .position(|k| now - k.created_at >= PUBLISH_AHEAD_MS)
        .or_else(|| keys.len().checked_sub(1));
    let Some(signer) = signer else {
        return KeyPlan {
            make_new,
            ..KeyPlan::default()
        };
    };
    let mut published = Vec::new();
    let mut expired = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        // An older key stopped signing when the next newer one started.
        let newer = i.checked_sub(1).and_then(|j| keys.get(j));
        let shown = i <= signer
            || newer.is_some_and(|n| now < n.created_at + PUBLISH_AHEAD_MS + RETIRED_KEPT_MS);
        if shown {
            published.push(k.kid.clone());
        } else {
            expired.push(k.kid.clone());
        }
    }
    KeyPlan {
        signer: keys.get(signer).map(|k| k.kid.clone()),
        published,
        expired,
        make_new,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            workspace_id: "wsp_acme".into(),
            workspace_slug: "acme".into(),
            project_id: "prj_web".into(),
            project_slug: "web".into(),
            session_id: "ses_42".into(),
            session_number: 42,
            session_url: "https://app.fctl.sh/s/ses_42".into(),
            user_id: "usr_ada".into(),
            user_email: Some("ada@acme.dev".into()),
            runner_id: "rnr_box".into(),
            runner_name: Some("build-box".into()),
            runner_personal: false,
            repository: Some("acme/web".into()),
            branch: Some("fctl/42-fix-the-cart".into()),
            base_branch: Some("main".into()),
            github_access: GithubAccess::Push,
            started_by: WebhookOrigin::Person,
            schedule_id: None,
            ticket: None,
            autopilot_id: None,
            autopilot_role: None,
            service_account_id: None,
        }
    }

    fn ask(audience: &str) -> Ask<'_> {
        Ask {
            audience,
            nonce: None,
            subject: &[],
            aws_session_tags: false,
        }
    }

    #[test]
    fn each_workspace_is_an_issuer_of_its_own() {
        let iss = issuer("https://app.fctl.sh/", "wsp_01ABC");
        assert_eq!(iss, "https://app.fctl.sh/oidc/wsp_01ABC");
        assert_eq!(
            discovery_url(&iss),
            "https://app.fctl.sh/oidc/wsp_01ABC/.well-known/openid-configuration"
        );
        assert_eq!(
            jwks_url(&iss),
            "https://app.fctl.sh/oidc/wsp_01ABC/.well-known/jwks.json"
        );
        assert_eq!(issuer_host_path(&iss), "app.fctl.sh/oidc/wsp_01ABC");
        assert_eq!(
            issuer_host_path("http://localhost:8787/oidc/wsp_1"),
            "localhost:8787/oidc/wsp_1"
        );
    }

    #[test]
    fn issuers_serve_their_discovery_document_and_keys_only() {
        assert_eq!(
            document("/oidc/wsp_01ABC/.well-known/openid-configuration"),
            Some(("wsp_01ABC", Document::Discovery))
        );
        assert_eq!(
            document("/oidc/wsp_01ABC/.well-known/jwks.json"),
            Some(("wsp_01ABC", Document::Keys))
        );
        for path in [
            "/oidc/wsp_01ABC",
            "/oidc/wsp_01ABC/",
            "/oidc/wsp_01ABC/.well-known/other",
            "/oidc/prj_01ABC/.well-known/jwks.json",
            "/oidc/wsp_01ABC/x/.well-known/jwks.json",
            "/oidc/../.well-known/jwks.json",
            "/oidc//.well-known/jwks.json",
            "/api/v1/oidc/wsp_01ABC/.well-known/jwks.json",
        ] {
            assert_eq!(document(path), None, "{path}");
        }
    }

    #[test]
    fn a_token_says_who_it_is_for_and_for_how_long() {
        let c = claims(
            &facts(),
            "https://app.fctl.sh/oidc/wsp_acme",
            &ask("sts.amazonaws.com"),
            "idt_1",
            1_700_000_000_999,
        );
        assert_eq!(c.iss, "https://app.fctl.sh/oidc/wsp_acme");
        assert_eq!(c.sub, "project:prj_web:user:usr_ada");
        assert_eq!(c.aud, "sts.amazonaws.com");
        assert_eq!(c.iat, 1_700_000_000);
        assert_eq!(c.nbf, 1_700_000_000 - SKEW_SECS);
        assert_eq!(c.exp, 1_700_000_000 + LIFETIME_SECS);
        let v = serde_json::to_value(&c).unwrap_or_default();
        assert_eq!(v["github_access"], "push");
        assert_eq!(v["started_by"], "person");
        assert_eq!(v["session_number"], 42);
        assert_eq!(v["runner_personal"], false);
        // What it doesn't know or wasn't asked for, it leaves out.
        for absent in ["nonce", "schedule_id", "ticket", AWS_TAGS_CLAIM] {
            assert!(v.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn the_discovery_document_lists_every_claim_a_token_has() {
        let mut f = facts();
        f.schedule_id = Some("sch_1".into());
        f.ticket = Some("ENG-1".into());
        f.autopilot_id = Some("prj_web".into());
        f.autopilot_role = Some("crew".into());
        f.service_account_id = Some("sva_1".into());
        let full = Ask {
            audience: "a",
            nonce: Some("n"),
            subject: &[],
            aws_session_tags: true,
        };
        let v = serde_json::to_value(claims(&f, "i", &full, "j", 0)).unwrap_or_default();
        let Some(object) = v.as_object() else {
            panic!("claims are an object");
        };
        let listed: Vec<&str> = CLAIMS.to_vec();
        let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        let doc = discovery("https://app.fctl.sh/oidc/wsp_acme");
        assert_eq!(doc["claims_supported"], json!(listed));
        assert_eq!(doc["issuer"], "https://app.fctl.sh/oidc/wsp_acme");
        assert_eq!(
            doc["jwks_uri"],
            "https://app.fctl.sh/oidc/wsp_acme/.well-known/jwks.json"
        );
        assert_eq!(
            doc["id_token_signing_alg_values_supported"],
            json!(["RS256"])
        );
    }

    #[test]
    fn agents_choose_the_parts_of_the_subject_in_a_fixed_order() {
        let f = facts();
        assert_eq!(subject(&f, &[]), "project:prj_web:user:usr_ada");
        assert_eq!(subject(&f, &[SubjectPart::Project]), "project:prj_web");
        assert_eq!(
            subject(
                &f,
                &[
                    SubjectPart::GithubAccess,
                    SubjectPart::Workspace,
                    SubjectPart::Project,
                    SubjectPart::GithubAccess
                ]
            ),
            "workspace:wsp_acme:project:prj_web:github_access:push"
        );
        assert_eq!(
            subject(&f, &[SubjectPart::Runner, SubjectPart::Session]),
            "session:ses_42:runner:rnr_box"
        );
        assert_eq!(project_subject("prj_web"), "project:prj_web:user:*");
    }

    #[test]
    fn audiences_and_nonces_are_printable_and_short() {
        assert!(check(&ask("sts.amazonaws.com")).is_ok());
        assert!(check(&ask("api://AzureADTokenExchange")).is_ok());
        assert!(check(&ask("")).is_err());
        assert!(check(&ask("two words")).is_err());
        assert!(check(&ask("tab\there")).is_err());
        assert!(check(&ask("ünïcode")).is_err());
        assert!(check(&ask(&"a".repeat(MAX_AUDIENCE_LEN))).is_ok());
        assert!(check(&ask(&"a".repeat(MAX_AUDIENCE_LEN + 1))).is_err());
        let with = |nonce| Ask {
            nonce: Some(nonce),
            ..ask("x")
        };
        assert!(check(&with("n-0S6_WzA2Mj")).is_ok());
        assert!(check(&with("")).is_err());
        assert!(check(&with("new\nline")).is_err());
        assert!(check(&with(&"n".repeat(MAX_NONCE_LEN + 1))).is_err());
    }

    #[test]
    fn aws_session_tags_come_only_when_asked_and_are_all_transitive() {
        let mut f = facts();
        f.started_by = WebhookOrigin::Schedule;
        let tagged = Ask {
            aws_session_tags: true,
            ..ask("sts.amazonaws.com")
        };
        let v = serde_json::to_value(claims(&f, "i", &tagged, "j", 0)).unwrap_or_default();
        let tags = &v[AWS_TAGS_CLAIM];
        assert_eq!(tags["principal_tags"]["fctl:project"], json!(["prj_web"]));
        assert_eq!(tags["principal_tags"]["fctl:user"], json!(["usr_ada"]));
        assert_eq!(
            tags["principal_tags"]["fctl:started_by"],
            json!(["schedule"])
        );
        assert_eq!(
            tags["principal_tags"]["fctl:github_access"],
            json!(["push"])
        );
        let keys = tags["transitive_tag_keys"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(keys.len(), 7);
        for key in &keys {
            let key = key.as_str().unwrap_or_default();
            assert!(tags["principal_tags"].get(key).is_some(), "{key}");
            // What AWS takes in a tag's key: letters, digits and _.:/=+-@.
            assert!(
                key.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_.:/=+-@".contains(c))
            );
        }
    }

    #[test]
    fn started_by_says_what_webhooks_say() {
        assert_eq!(
            started_by(false, false, false, false),
            WebhookOrigin::Person
        );
        assert_eq!(
            started_by(false, false, false, true),
            WebhookOrigin::ServiceAccount
        );
        assert_eq!(started_by(false, false, true, true), WebhookOrigin::Ticket);
        assert_eq!(started_by(false, true, true, true), WebhookOrigin::Schedule);
        assert_eq!(started_by(true, true, true, true), WebhookOrigin::Autopilot);
    }

    #[test]
    fn tokens_are_put_together_as_jws_compact() {
        let c = claims(&facts(), "i", &ask("a"), "j", 0);
        let input = signing_input("kid-1", &c);
        let t = token(&input, &[1, 2, 3]);
        assert_eq!(t.matches('.').count(), 2);
        assert!(t.ends_with(".AQID"));
        let header = input.split('.').next().unwrap_or_default();
        let header: Value = URL_SAFE_NO_PAD
            .decode(header)
            .ok()
            .and_then(|h| serde_json::from_slice(&h).ok())
            .unwrap_or_default();
        assert_eq!(
            header,
            json!({ "alg": "RS256", "typ": "JWT", "kid": "kid-1" })
        );
        let read = read_claims(&t).unwrap_or_default();
        assert_eq!(read["sub"], "project:prj_web:user:usr_ada");
        assert_eq!(read_claims("not.a"), None);
        assert_eq!(read_claims("a.b.c.d"), None);
    }

    #[test]
    fn a_keys_id_is_its_rfc_7638_thumbprint() {
        // RFC 7638, section 3.1.
        let n = "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw";
        assert_eq!(
            thumbprint(n, "AQAB"),
            "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs"
        );
        let jwk = public_jwk("kid", "n", "AQAB");
        assert_eq!(jwk["use"], "sig");
        assert_eq!(jwk["alg"], "RS256");
        assert_eq!(jwks(vec![jwk.clone()]), json!({ "keys": [jwk] }));
    }

    fn key(kid: &str, days_ago: i64, now: i64) -> KeyAge {
        KeyAge {
            kid: kid.into(),
            created_at: now - days_ago * DAY_MS,
        }
    }

    #[test]
    fn the_first_key_signs_at_once() {
        let now = 1_000 * DAY_MS;
        assert_eq!(
            plan(&[], now),
            KeyPlan {
                make_new: true,
                ..KeyPlan::default()
            }
        );
        let p = plan(&[key("a", 0, now)], now);
        assert_eq!(p.signer.as_deref(), Some("a"));
        assert_eq!(p.published, ["a"]);
        assert!(!p.make_new);
        assert!(p.expired.is_empty());
    }

    #[test]
    fn a_new_key_is_published_before_it_signs_and_the_old_one_after() {
        let now = 1_000 * DAY_MS;
        // Thirty days on, a key is due to make way.
        assert!(plan(&[key("a", 30, now)], now).make_new);
        assert!(!plan(&[key("a", 29, now)], now).make_new);
        // Its successor is listed at once, but signs two days later.
        let young = plan(&[key("a", 31, now), key("b", 1, now)], now);
        assert_eq!(young.signer.as_deref(), Some("a"));
        assert_eq!(young.published, ["b", "a"]);
        assert!(!young.make_new);
        let taken_over = plan(&[key("a", 33, now), key("b", 3, now)], now);
        assert_eq!(taken_over.signer.as_deref(), Some("b"));
        // The old one stays two days after it stopped signing, then goes.
        assert_eq!(taken_over.published, ["b", "a"]);
        let gone = plan(&[key("a", 35, now), key("b", 5, now)], now);
        assert_eq!(gone.signer.as_deref(), Some("b"));
        assert_eq!(gone.published, ["b"]);
        assert_eq!(gone.expired, ["a"]);
    }

    #[test]
    fn a_quiet_instance_makes_its_next_key_when_it_signs_again() {
        let now = 1_000 * DAY_MS;
        // Nothing signed for months: the old key goes on signing until the
        // new one has been published long enough.
        let p = plan(&[key("old", 90, now), key("new", 0, now)], now);
        assert_eq!(p.signer.as_deref(), Some("old"));
        assert_eq!(p.published, ["new", "old"]);
        assert!(!p.make_new);
    }
}

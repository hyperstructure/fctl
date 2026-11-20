//! Identity tokens (docs/identity-tokens.md, ADR 0090): the instance's
//! signing keys, the tokens agents mint with them (`AgentCall::IdToken`,
//! which their host sends with its runner token) and what each workspace's
//! issuer serves to whoever checks them: its discovery document and keys,
//! at `/oidc/{wsp_…}/.well-known/…`, which anyone may read.
//!
//! `fctl_domain::id_tokens` says what a token holds and which key signs
//! when; this signs, with WebCrypto, and keeps the keys in D1
//! (`id_token_keys`), their private halves sealed with the master key.

use crate::api::calls::RunnerSession;
use crate::config::Config;
use crate::crypto::{self, Keys};
use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, forbidden, internal};
use fctl_domain::id_tokens::{self as domain, Ask, Document, Facts, KeyAge};
use fctl_proto::session::SessionStatus;
use fctl_proto::{ApiError, ErrorCode, Millis};
use js_sys::{Array, Object, Reflect, Uint8Array};
use serde::Deserialize;
use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use worker::{D1Database, Env, Method, Request, Response};

/// How many tokens a session mints in a minute, at most: an agent mints
/// one now and then, and fctl-agent one for each of its clouds every few
/// minutes.
const LIMIT_MESSAGE: &str = "This session minted many identity tokens this minute: wait a minute, and keep each until it nearly expires (10 minutes).";

thread_local! {
    /// The key that signed last, imported once for as long as the isolate
    /// lives, by its id.
    static SIGNER: RefCell<Option<(String, web_sys::CryptoKey)>> = const { RefCell::new(None) };
}

#[derive(Deserialize)]
struct KeyRow {
    kid: String,
    public_jwk: String,
    private_enc: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
}

/// Associated data of a sealed private key.
fn aad(kid: &str) -> String {
    format!("id-token-key:{kid}")
}

fn set(obj: &Object, key: &str, value: &JsValue) -> AppResult<()> {
    Reflect::set(obj, &JsValue::from_str(key), value)?;
    Ok(())
}

/// RS256 to WebCrypto, with a new key's size and exponent.
fn rs256(generate: bool) -> AppResult<Object> {
    let algo = Object::new();
    set(&algo, "name", &"RSASSA-PKCS1-v1_5".into())?;
    set(&algo, "hash", &"SHA-256".into())?;
    if generate {
        set(&algo, "modulusLength", &JsValue::from(domain::MODULUS_BITS))?;
        set(
            &algo,
            "publicExponent",
            &Uint8Array::from(&[1u8, 0, 1][..]).into(),
        )?;
    }
    Ok(algo)
}

/// A JavaScript value as JSON.
fn to_json(v: &JsValue) -> AppResult<serde_json::Value> {
    let text: String = js_sys::JSON::stringify(v)?.into();
    Ok(serde_json::from_str(&text)?)
}

/// A new key, made and stored unless another was made since the newest
/// one we saw ([`domain::plan`] asked for it): two Workers that find it
/// due at once store one.
async fn make_key(db: &D1Database, keys: &Keys, now: i64) -> AppResult<()> {
    let subtle = crypto::subtle()?;
    let usages = Array::of2(&"sign".into(), &"verify".into());
    let promise = subtle.generate_key_with_object(&rs256(true)?, true, &usages)?;
    // A CryptoKeyPair is a dictionary, not a class: nothing to check it against.
    let pair: web_sys::CryptoKeyPair = JsFuture::from(promise).await?.unchecked_into();
    let jwk = JsFuture::from(subtle.export_key("jwk", &pair.get_private_key())?).await?;
    let private = to_json(&jwk)?;
    let member = |name: &str| {
        private
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| internal(format!("the new signing key has no {name}")))
    };
    let (modulus, exponent) = (member("n")?, member("e")?);
    let kid = domain::thumbprint(&modulus, &exponent);
    let public = domain::public_jwk(&kid, &modulus, &exponent);
    let sealed = keys.seal_json(&aad(&kid), &private)?;
    q(
        db,
        "INSERT INTO id_token_keys (kid, public_jwk, private_enc, created_at) \
         SELECT ?1, ?2, ?3, ?4 WHERE NOT EXISTS (SELECT 1 FROM id_token_keys WHERE created_at > ?5)",
        &[
            s(&kid),
            s(&public.to_string()),
            s(&sealed),
            n(now),
            n(now - domain::ROTATE_AFTER_MS),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// The keys as they should be at `now`: a new one made when one is due
/// (with the master key; without it, none), those no longer published
/// gone. Returns the published keys, newest first, and the signer.
async fn current(
    db: &D1Database,
    keys: Option<&Keys>,
    now: i64,
) -> AppResult<(Vec<KeyRow>, Option<KeyRow>)> {
    let load = || async {
        q(
            db,
            "SELECT kid, public_jwk, private_enc, created_at FROM id_token_keys ORDER BY created_at DESC, kid",
            &[],
        )?
        .all::<KeyRow>()
        .await
    };
    let ages = |rows: &[KeyRow]| -> Vec<KeyAge> {
        rows.iter()
            .map(|r| KeyAge {
                kid: r.kid.clone(),
                created_at: r.created_at,
            })
            .collect()
    };
    let mut rows = load().await?;
    let mut plan = domain::plan(&ages(&rows), now);
    if plan.make_new
        && let Some(keys) = keys
    {
        make_key(db, keys, now).await?;
        rows = load().await?;
        plan = domain::plan(&ages(&rows), now);
    }
    for kid in &plan.expired {
        q(db, "DELETE FROM id_token_keys WHERE kid = ?", &[s(kid)])?
            .run()
            .await?;
    }
    let mut published = Vec::new();
    let mut signer = None;
    for row in rows {
        if plan.signer.as_deref() == Some(row.kid.as_str()) {
            signer = Some(KeyRow {
                kid: row.kid.clone(),
                public_jwk: row.public_jwk.clone(),
                private_enc: row.private_enc.clone(),
                created_at: row.created_at,
            });
        }
        if plan.published.contains(&row.kid) {
            published.push(row);
        }
    }
    Ok((published, signer))
}

/// The private key `row` holds, imported for signing (once per isolate).
async fn signing_key(row: &KeyRow, keys: &Keys) -> AppResult<web_sys::CryptoKey> {
    let cached = SIGNER.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(kid, _)| *kid == row.kid)
            .map(|(_, key)| key.clone())
    });
    if let Some(key) = cached {
        return Ok(key);
    }
    let private: serde_json::Value = keys.open_json(&aad(&row.kid), &row.private_enc)?;
    let jwk: Object = js_sys::JSON::parse(&private.to_string())?.unchecked_into();
    let usages = Array::of1(&"sign".into());
    let promise =
        crypto::subtle()?.import_key_with_object("jwk", &jwk, &rs256(false)?, false, &usages)?;
    let key: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
    SIGNER.with(|c| *c.borrow_mut() = Some((row.kid.clone(), key.clone())));
    Ok(key)
}

/// `input` signed with RS256 by the key `row` holds.
async fn sign(row: &KeyRow, keys: &Keys, input: &[u8]) -> AppResult<Vec<u8>> {
    let key = signing_key(row, keys).await?;
    let promise = crypto::subtle()?.sign_with_object_and_u8_array(&rs256(false)?, &key, input)?;
    Ok(Uint8Array::new(&JsFuture::from(promise).await?).to_vec())
}

/// What the control plane knows of a session for its tokens, besides
/// [`RunnerSession`].
#[derive(Deserialize)]
struct FactsRow {
    #[serde(deserialize_with = "crate::db::int")]
    number: i64,
    status: String,
    created_by: String,
    branch: Option<String>,
    base_branch: Option<String>,
    schedule_id: Option<String>,
    ticket: Option<String>,
    autopilot_id: Option<String>,
    autopilot_role: Option<String>,
    service_account_id: Option<String>,
    runner_id: Option<String>,
    project_slug: String,
    default_branch: String,
    workspace_slug: Option<String>,
    user_email: Option<String>,
    runner_name: Option<String>,
    #[serde(default, deserialize_with = "crate::db::int_bool")]
    runner_personal: bool,
}

async fn facts(db: &D1Database, public_url: &str, session: &RunnerSession) -> AppResult<Facts> {
    let row: Option<FactsRow> = q(
        db,
        "SELECT s.number, s.status, s.created_by, s.branch, s.base_branch, s.schedule_id, s.ticket, \
         s.autopilot_id, s.autopilot_role, s.service_account_id, s.runner_id, \
         p.slug AS project_slug, p.default_branch, w.slug AS workspace_slug, wm.email AS user_email, \
         r.name AS runner_name, rw.personal AS runner_personal \
         FROM sessions s JOIN projects p ON p.id = s.project_id \
         LEFT JOIN workspaces w ON w.id = p.workspace_id \
         LEFT JOIN workspace_members wm ON wm.workspace_id = p.workspace_id AND wm.user_id = s.created_by \
         LEFT JOIN runners r ON r.id = s.runner_id \
         LEFT JOIN runner_workspaces rw ON rw.runner_id = s.runner_id AND rw.workspace_id = p.workspace_id \
         WHERE s.id = ?",
        &[s(&session.id)],
    )?
    .first()
    .await?;
    let row = row.ok_or_else(|| crate::error::not_found("Session"))?;
    // A token vouches for a session whose sandbox runs: a runner whose
    // session stopped there mints nothing more for it.
    if !SessionStatus::parse(&row.status).is_some_and(SessionStatus::is_active) {
        return Err(forbidden(
            "This session isn't running: identity tokens are minted only while its sandbox runs.",
        ));
    }
    let (Some(runner_id), Some(workspace_slug)) = (row.runner_id, row.workspace_slug) else {
        return Err(internal("the session has no runner or workspace"));
    };
    let ticket = row
        .ticket
        .as_deref()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .and_then(|t| t.get("key").and_then(|k| k.as_str()).map(str::to_owned));
    Ok(Facts {
        workspace_id: session.workspace_id.clone(),
        workspace_slug,
        project_id: session.project_id.clone(),
        project_slug: row.project_slug,
        session_id: session.id.clone(),
        session_number: u32::try_from(row.number).unwrap_or(0),
        session_url: format!("{}/s/{}", public_url.trim_end_matches('/'), session.id),
        user_id: row.created_by,
        user_email: row.user_email,
        runner_id,
        runner_name: row.runner_name,
        runner_personal: row.runner_personal,
        repository: session.repo.as_ref().map(|r| r.full_name()),
        // As its spec says them (`spec::build`), for sessions from before
        // their rows had them.
        branch: Some(row.branch.unwrap_or_else(|| format!("fctl/{}", row.number))),
        base_branch: Some(row.base_branch.unwrap_or(row.default_branch)),
        github_access: session.github_access,
        started_by: domain::started_by(
            row.autopilot_id.is_some(),
            row.schedule_id.is_some(),
            ticket.is_some(),
            row.service_account_id.is_some(),
        ),
        schedule_id: row.schedule_id,
        ticket,
        autopilot_id: row.autopilot_id,
        autopilot_role: row.autopilot_role,
        service_account_id: row.service_account_id,
    })
}

/// Mints an identity token for `session`'s agent, as `ask` asks: only
/// where its project lets its agents, never for a review agent, whose pull
/// request's text could steer what runs in its sandbox, and only while the
/// session runs. Returns the token and when it expires.
pub async fn mint(
    ctx: &Ctx,
    session: &RunnerSession,
    ask: &Ask<'_>,
) -> AppResult<(String, Millis)> {
    if session.review {
        return Err(forbidden(
            "Review agents mint no identity tokens: what they review could steer what runs in their sandbox.",
        ));
    }
    if !session.settings.id_tokens {
        return Err(forbidden(
            "This project's agents mint no identity tokens. Its maintainers can turn them on in the project's settings, under Identity OIDC.",
        ));
    }
    domain::check(ask).map_err(AppError)?;
    ctx.limit(&format!("id-token:{}", session.id), LIMIT_MESSAGE)
        .await?;
    let keys = crypto::keys(&ctx.cfg)?;
    issue(&ctx.db, &ctx.cfg.public_url, &keys, session, ask, ctx.now).await
}

/// Signs a token for `session` as `ask` asks, at `now`, once the
/// session's facts say it runs.
async fn issue(
    db: &D1Database,
    public_url: &str,
    keys: &Keys,
    session: &RunnerSession,
    ask: &Ask<'_>,
    now: i64,
) -> AppResult<(String, Millis)> {
    let facts = facts(db, public_url, session).await?;
    let (_, signer) = current(db, Some(keys), now).await?;
    let signer = signer.ok_or_else(|| internal("no key signs identity tokens"))?;
    let issuer = domain::issuer(public_url, &facts.workspace_id);
    let jti = fctl_domain::ids::new_id("idt", now);
    let claims = domain::claims(&facts, &issuer, ask, &jti, now);
    let input = domain::signing_input(&signer.kid, &claims);
    let signature = sign(&signer, keys, input.as_bytes()).await?;
    Ok((domain::token(&input, &signature), claims.exp * 1000))
}

/// Whether a workspace is there for its issuer to serve.
async fn workspace_exists(db: &D1Database, id: &str) -> AppResult<bool> {
    let row: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM workspaces WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    Ok(row.is_some_and(|r| r.n > 0))
}

/// What an issuer answers.
enum Answer {
    Json(serde_json::Value),
    /// Its discovery document's address, for whoever opened the issuer's
    /// own, as the app shows it.
    SeeDiscovery(String),
}

/// An issuer's document, which anyone may read, and any page's script
/// fetch: `GET /oidc/{wsp_…}/.well-known/openid-configuration` or
/// `…/jwks.json`; its own address sends people to the first. Unknown
/// workspaces have none.
pub async fn serve(env: &Env, req: &Request, path: &str) -> worker::Result<Response> {
    let Some((workspace_id, document)) = domain::document(path) else {
        return Response::error("Not found", 404);
    };
    if !matches!(req.method(), Method::Get | Method::Head) {
        return Response::error("Method not allowed", 405);
    }
    let cfg = Config::from_env(env);
    let answer = async {
        let db = env.d1("DB")?;
        if !workspace_exists(&db, workspace_id).await? {
            return Ok(None);
        }
        let issuer = domain::issuer(&cfg.public_url, workspace_id);
        AppResult::Ok(Some(match document {
            Document::Issuer => Answer::SeeDiscovery(domain::discovery_url(&issuer)),
            Document::Discovery => Answer::Json(domain::discovery(&issuer)),
            Document::Keys => {
                // The first key is made as an issuer is first asked for its
                // keys, when the instance has a master key to seal it with.
                let keys = crypto::keys(&cfg).ok();
                let (published, _) = current(&db, keys.as_ref(), crate::ctx::now_ms()).await?;
                Answer::Json(domain::jwks(
                    published
                        .iter()
                        .filter_map(|k| serde_json::from_str(&k.public_jwk).ok())
                        .collect(),
                ))
            }
        }))
    };
    let body = match answer.await {
        Ok(Some(Answer::Json(body))) => body,
        Ok(Some(Answer::SeeDiscovery(url))) => {
            return Response::redirect_with_status(url.parse()?, 302);
        }
        Ok(None) => return Response::error("Not found", 404),
        Err(e) => {
            worker::console_error!("identity token issuer: {}", e.0.message);
            return Ok(crate::http::error_response(&ApiError::new(
                ErrorCode::Unavailable,
                "The issuer can't answer right now; try again in a minute.",
            )));
        }
    };
    let mut response = Response::ok(body.to_string())?;
    let headers = response.headers_mut();
    headers.set("Content-Type", "application/json")?;
    headers.set("Cache-Control", "public, max-age=3600")?;
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    Ok(response)
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    const DAY: i64 = 24 * 3600 * 1000;

    fn keys() -> Keys {
        Keys::derive(&[7u8; 32])
    }

    /// A string at `pointer` in `v`, or "" without one.
    fn at<'a>(v: &'a serde_json::Value, pointer: &str) -> &'a str {
        v.pointer(pointer)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
    }

    /// The instance makes its first key when first asked, signs with it,
    /// and verifies what it signed with the key its issuers publish.
    #[wasm_bindgen_test]
    async fn the_published_key_verifies_what_the_signer_signs() -> AppResult<()> {
        let db = testing::database();
        let now = 1_800_000_000_000;
        let (published, signer) = current(&db, Some(&keys()), now).await?;
        let signer = signer.ok_or_else(|| internal("no signer"))?;
        assert_eq!(published.len(), 1);
        let first = published
            .first()
            .ok_or_else(|| internal("nothing published"))?;
        assert_eq!(first.kid, signer.kid);
        let public: serde_json::Value = serde_json::from_str(&first.public_jwk)?;
        assert_eq!(at(&public, "/kid"), signer.kid.as_str());
        assert_eq!(at(&public, "/alg"), "RS256");
        assert!(
            public.get("d").is_none(),
            "the private exponent stays private"
        );
        let input = b"header.payload";
        let signature = sign(&signer, &keys(), input).await?;
        assert_eq!(signature.len(), 256);
        // Verify with the public key alone, as a cloud does.
        let jwk: Object = js_sys::JSON::parse(&public.to_string())?.unchecked_into();
        let usages = Array::of1(&"verify".into());
        let promise = crypto::subtle()?.import_key_with_object(
            "jwk",
            &jwk,
            &rs256(false)?,
            false,
            &usages,
        )?;
        let key: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
        let verify = |data: &[u8]| {
            crypto::subtle().and_then(|subtle| {
                Ok(subtle.verify_with_object_and_u8_array_and_u8_array(
                    &rs256(false)?,
                    &key,
                    &signature,
                    data,
                )?)
            })
        };
        assert_eq!(JsFuture::from(verify(input)?).await?.as_bool(), Some(true));
        assert_eq!(
            JsFuture::from(verify(b"other")?).await?.as_bool(),
            Some(false)
        );
        // Without the master key, no key is made, and none is published.
        let bare = testing::database();
        let (none, signer) = current(&bare, None, now).await?;
        assert!(none.is_empty() && signer.is_none());
        Ok(())
    }

    /// Acme's web project, whose session 7 runs for Ada on her own
    /// runner, started by a schedule from a ticket; session 8 stopped.
    async fn acme(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@home.example', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO workspace_members (workspace_id, user_id, email, role, created_at) VALUES ('wsp_acme', 'usr_ada', 'ada@acme.dev', 'owner', 0)",
            "INSERT INTO projects (id, workspace_id, slug, name, repo_owner, repo_name, created_by, created_at, updated_at) \
             VALUES ('prj_web', 'wsp_acme', 'web', 'Web', 'acme', 'web', 'usr_ada', 0, 0)",
            "INSERT INTO runners (id, name, token_hash, created_by, created_at) VALUES ('rnr_box', 'build-box', 'h', 'usr_ada', 0)",
            "INSERT INTO runner_workspaces (runner_id, workspace_id, personal, created_at) VALUES ('rnr_box', 'wsp_acme', 1, 0)",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, runner_id, branch, base_branch, \
             claude_session_id, created_at, updated_at, schedule_id, ticket) VALUES \
             ('ses_7', 'prj_web', 7, 'Fix the cart', 'x', 'running', 'usr_ada', 'rnr_box', 'fctl/7-fix-the-cart', 'main', 'c7', 0, 0, \
              'sch_nightly', '{\"tracker\":\"linear\",\"id\":\"i1\",\"key\":\"ENG-1\",\"url\":\"https://linear.app/acme/issue/ENG-1\"}'), \
             ('ses_8', 'prj_web', 8, 'Old', 'x', 'stopped', 'usr_ada', 'rnr_box', 'fctl/8-old', 'main', 'c8', 0, 0, NULL, NULL)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    fn session(id: &str) -> RunnerSession {
        RunnerSession {
            id: id.to_owned(),
            project_id: "prj_web".into(),
            created_by: "usr_ada".into(),
            settings: fctl_proto::session::ProjectSettings::default(),
            workspace_id: "wsp_acme".into(),
            repo: Some(fctl_proto::session::RepoRef {
                owner: "acme".into(),
                name: "web".into(),
            }),
            github_access: fctl_proto::session::GithubAccess::Comment,
            review: false,
        }
    }

    /// A token says what D1 knows of its session, and the issuer's
    /// published key verifies it.
    #[wasm_bindgen_test]
    async fn a_token_says_what_d1_knows_of_its_session() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let now = 1_800_000_000_000;
        let ask = Ask {
            audience: "https://api.acme.dev",
            nonce: Some("n-1"),
            subject: &[],
            aws_session_tags: true,
        };
        let (token, expires_at) = issue(
            &db,
            "https://fctl.acme.dev",
            &keys(),
            &session("ses_7"),
            &ask,
            now,
        )
        .await?;
        assert_eq!(expires_at, now + domain::LIFETIME_SECS * 1000);
        let claims = domain::read_claims(&token).ok_or_else(|| internal("unreadable"))?;
        let want = serde_json::json!({
            "iss": "https://fctl.acme.dev/oidc/wsp_acme",
            "sub": "project:prj_web:user:usr_ada",
            "aud": "https://api.acme.dev",
            "nonce": "n-1",
            "workspace_id": "wsp_acme",
            "workspace_slug": "acme",
            "project_id": "prj_web",
            "project_slug": "web",
            "session_id": "ses_7",
            "session_number": 7,
            "session_url": "https://fctl.acme.dev/s/ses_7",
            "user_id": "usr_ada",
            // The address she is a member of the workspace as.
            "user_email": "ada@acme.dev",
            "runner_id": "rnr_box",
            "runner_name": "build-box",
            "runner_personal": true,
            "repository": "acme/web",
            "branch": "fctl/7-fix-the-cart",
            "base_branch": "main",
            "github_access": "comment",
            "started_by": "schedule",
            "schedule_id": "sch_nightly",
            "ticket": "ENG-1",
        });
        for (k, v) in want.as_object().into_iter().flatten() {
            assert_eq!(claims.get(k), Some(v), "{k}");
        }
        assert_eq!(
            claims
                .get(domain::AWS_TAGS_CLAIM)
                .and_then(|t| t.pointer("/principal_tags/fctl:runner")),
            Some(&serde_json::json!(["rnr_box"]))
        );
        // Signed by the key the issuer publishes.
        let (published, _) = current(&db, None, now).await?;
        let first = published
            .first()
            .ok_or_else(|| internal("nothing published"))?;
        let public: serde_json::Value = serde_json::from_str(&first.public_jwk)?;
        let mut parts = token.rsplitn(2, '.');
        let (signature, input) = (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        );
        let header: serde_json::Value =
            crypto::b64url_decode(input.split('.').next().unwrap_or_default())
                .and_then(|h| serde_json::from_slice(&h).ok())
                .unwrap_or_default();
        assert_eq!(header.get("kid"), public.get("kid"));
        let signature = crypto::b64url_decode(signature).unwrap_or_default();
        let jwk: Object = js_sys::JSON::parse(&public.to_string())?.unchecked_into();
        let usages = Array::of1(&"verify".into());
        let promise = crypto::subtle()?.import_key_with_object(
            "jwk",
            &jwk,
            &rs256(false)?,
            false,
            &usages,
        )?;
        let key: web_sys::CryptoKey = JsFuture::from(promise).await?.dyn_into()?;
        let verified = crypto::subtle()?.verify_with_object_and_u8_array_and_u8_array(
            &rs256(false)?,
            &key,
            &signature,
            input.as_bytes(),
        )?;
        assert_eq!(JsFuture::from(verified).await?.as_bool(), Some(true));
        // A stopped session's runner gets nothing more for it.
        let stopped = issue(
            &db,
            "https://fctl.acme.dev",
            &keys(),
            &session("ses_8"),
            &ask,
            now,
        )
        .await;
        assert_eq!(
            stopped.err().map(|e| e.0.code),
            Some(fctl_proto::ErrorCode::Forbidden)
        );
        Ok(())
    }

    /// Keys follow each other as `fctl_domain::id_tokens::plan` says, and
    /// two Workers that find one due at once make one.
    #[wasm_bindgen_test]
    async fn keys_take_turns_and_the_old_one_goes() -> AppResult<()> {
        let db = testing::database();
        let start = 1_800_000_000_000;
        let (_, first) = current(&db, Some(&keys()), start).await?;
        let first = first.ok_or_else(|| internal("no first key"))?.kid;
        // A month on, the next is made at once, and published, but the
        // first still signs; both Workers asking make one.
        let month = start + 31 * DAY;
        let k = keys();
        let (a, b) = futures::join!(current(&db, Some(&k), month), current(&db, Some(&k), month));
        let (published, signer) = a?;
        b?;
        assert_eq!(published.len(), 2);
        assert_eq!(signer.map(|k| k.kid).as_deref(), Some(first.as_str()));
        let next = published
            .first()
            .map(|k| k.kid.clone())
            .ok_or_else(|| internal("nothing published"))?;
        assert_ne!(next, first);
        // Two days later the new one signs, and the old one is still there.
        let (published, signer) = current(&db, Some(&keys()), month + 2 * DAY).await?;
        assert_eq!(signer.map(|k| k.kid).as_deref(), Some(next.as_str()));
        assert_eq!(published.len(), 2);
        // Two more days and it is gone, from D1 too.
        let (published, _) = current(&db, Some(&keys()), month + 4 * DAY).await?;
        assert_eq!(
            published.iter().map(|k| k.kid.as_str()).collect::<Vec<_>>(),
            [next.as_str()]
        );
        let left: Vec<KeyRow> = q(&db, "SELECT * FROM id_token_keys", &[])?.all().await?;
        assert_eq!(left.len(), 1);
        Ok(())
    }
}

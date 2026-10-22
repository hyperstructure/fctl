//! Passkeys (docs/architecture.md#passkeys): WebAuthn credentials people
//! sign in with, by fingerprint, face or screen lock.
//!
//! Every ceremony signs a challenge that works once, for ten minutes. The
//! browser's answer names it in `clientDataJSON`, so nothing else has to
//! travel with it. Making a passkey, and one as a second factor, store
//! theirs (`passkey_challenges`, by SHA-256); signing in signs its with the
//! master key instead, so that the sign-in page asks for one on every load
//! for free, and records it only once it's used.
//! `fctl_domain::webauthn` checks the answers; WebCrypto the signatures.

use crate::auth;
use crate::crypto;
use crate::ctx::Ctx;
use crate::db::{Count, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, internal, not_found, validation};
use crate::http;
use crate::model::UserRow;
use fctl_domain::two_factor as policy;
use fctl_domain::webauthn::{self, ALGORITHMS, Expect, b64url, from_b64url};
use fctl_domain::{cbor, random_bytes};
use fctl_proto::api::*;
use fctl_proto::{ApiError, ErrorCode, PasskeyId};
use serde::Deserialize;
use worker::{Request, Response};

/// How long a ceremony's challenge works.
const CHALLENGE_TTL_MS: i64 = 10 * 60 * 1000;
/// How long the browser waits for the person (WebAuthn's `timeout`).
const TIMEOUT_MS: u32 = 5 * 60 * 1000;
/// Passkeys one account can have.
const MAX_PASSKEYS: i64 = 20;
/// Longest passkey name.
const MAX_NAME: usize = 60;

// ───────────────────────────── the site ─────────────────────────────

/// The relying party's id: the app's host name.
fn rp_id(ctx: &Ctx) -> AppResult<String> {
    url::Url::parse(&ctx.cfg.public_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .ok_or_else(|| internal("FCTL_PUBLIC_URL has no host name"))
}

/// Whether a ceremony happened on the app: its own origin, or in
/// development any port of localhost (Vite serves the app on its own).
fn own_origin(ctx: &Ctx, origin: &str) -> bool {
    origin == ctx.cfg.origin()
        || (ctx.cfg.dev
            && (origin == "http://localhost" || origin.starts_with("http://localhost:")))
}

/// The user handle passkeys carry: the account's id, never its email.
fn user_handle(user_id: &str) -> String {
    b64url(user_id.as_bytes())
}

fn refused(e: webauthn::Error) -> AppError {
    use webauthn::Error::*;
    let message = match e {
        NotVerified => {
            "Your device didn't confirm it's you. Use a passkey with a fingerprint, face or screen lock."
        }
        UnsupportedKey => "fctl can't use this kind of passkey. Try another passkey provider.",
        WrongOrigin | WrongRp => "This passkey belongs to another site.",
        _ => "The passkey's answer didn't check out. Try again.",
    };
    validation("passkey", message)
}

// ───────────────────────────── the list ─────────────────────────────

#[derive(Deserialize)]
struct PasskeyRow {
    id: String,
    user_id: String,
    credential_id: String,
    public_key: String,
    #[serde(deserialize_with = "crate::db::int")]
    sign_count: i64,
    transports: Option<String>,
    aaguid: Option<String>,
    #[serde(deserialize_with = "crate::db::int_bool", default)]
    backed_up: bool,
    name: String,
    #[serde(deserialize_with = "crate::db::int")]
    created_at: i64,
    #[serde(deserialize_with = "crate::db::opt_int", default)]
    last_used_at: Option<i64>,
}

impl PasskeyRow {
    fn to_passkey(&self) -> Passkey {
        Passkey {
            id: PasskeyId(self.id.clone()),
            name: self.name.clone(),
            provider: self
                .aaguid
                .as_deref()
                .and_then(webauthn::provider)
                .map(str::to_owned),
            synced: self.backed_up,
            credential_id: self.credential_id.clone(),
            created_at: self.created_at,
            last_used_at: self.last_used_at,
        }
    }

    fn descriptor(&self) -> PasskeyDescriptor {
        PasskeyDescriptor {
            kind: "public-key".into(),
            id: self.credential_id.clone(),
            transports: self
                .transports
                .as_deref()
                .and_then(|t| serde_json::from_str(t).ok()),
        }
    }
}

async fn rows(ctx: &Ctx, user_id: &str) -> AppResult<Vec<PasskeyRow>> {
    q(
        &ctx.db,
        "SELECT * FROM passkeys WHERE user_id = ? ORDER BY created_at",
        &[s(user_id)],
    )?
    .all()
    .await
}

/// `Me.passkeys`.
pub async fn list(ctx: &Ctx, user_id: &str) -> AppResult<Vec<Passkey>> {
    Ok(rows(ctx, user_id)
        .await?
        .iter()
        .map(PasskeyRow::to_passkey)
        .collect())
}

async fn me(ctx: &Ctx, user_id: &str) -> AppResult<Response> {
    let user = auth::user_by_id(ctx, user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    http::ok(&auth::me(ctx, &user).await?)
}

// ───────────────────────────── challenges ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Register,
    SignIn,
    SecondFactor,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::SignIn => "sign_in",
            Self::SecondFactor => "second_factor",
        }
    }
}

/// A new challenge for a ceremony, base64url.
async fn challenge(
    ctx: &Ctx,
    purpose: Purpose,
    user_id: Option<&str>,
    sign_in_hash: Option<&str>,
) -> AppResult<String> {
    let bytes: [u8; 32] = random_bytes();
    q(
        &ctx.db,
        "INSERT INTO passkey_challenges (hash, purpose, user_id, sign_in_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)",
        &[
            s(&hex::encode(crypto::sha256(&bytes))),
            s(purpose.as_str()),
            os(user_id),
            os(sign_in_hash),
            n(ctx.now),
            n(ctx.now + CHALLENGE_TTL_MS),
        ],
    )?
    .run()
    .await?;
    Ok(b64url(&bytes))
}

#[derive(Deserialize)]
struct ChallengeRow {
    purpose: String,
    user_id: Option<String>,
    sign_in_hash: Option<String>,
}

/// A sign-in challenge: random bits and when they run out, signed, so that
/// nothing is stored until it's used.
#[derive(serde::Serialize, Deserialize)]
struct SignedChallenge {
    exp: i64,
    nonce: String,
}

fn sign_in_challenge(ctx: &Ctx) -> AppResult<String> {
    let signed = ctx.keys()?.sign(&serde_json::to_vec(&SignedChallenge {
        exp: ctx.now + CHALLENGE_TTL_MS,
        nonce: b64url(&random_bytes::<16>()),
    })?);
    Ok(b64url(signed.as_bytes()))
}

/// Uses up a signed sign-in challenge: it must be ours and current, and it
/// is recorded until it expires, so that it works once.
async fn spend(ctx: &Ctx, challenge: &[u8]) -> AppResult<ChallengeRow> {
    let signed = std::str::from_utf8(challenge)
        .ok()
        .and_then(|text| ctx.keys().ok()?.verify(text))
        .and_then(|payload| serde_json::from_slice::<SignedChallenge>(&payload).ok())
        .filter(|st| st.exp > ctx.now)
        .ok_or_else(stale)?;
    let recorded = q(
        &ctx.db,
        "INSERT INTO passkey_challenges (hash, purpose, created_at, expires_at) VALUES (?, 'sign_in', ?, ?) \
         ON CONFLICT (hash) DO NOTHING",
        &[
            s(&hex::encode(crypto::sha256(challenge))),
            n(ctx.now),
            n(signed.exp),
        ],
    )?
    .run()
    .await?;
    if recorded == 0 {
        return Err(stale());
    }
    Ok(ChallengeRow {
        purpose: Purpose::SignIn.as_str().to_owned(),
        user_id: None,
        sign_in_hash: None,
    })
}

fn stale() -> AppError {
    validation(
        "passkey",
        "That took too long, or the passkey answered twice. Try again.",
    )
}

/// Uses up the challenge the browser signed: each works once, for what it
/// was made for. Only a challenge for `purpose` goes: one spent signing in
/// stays recorded (`spend`), or an answer naming it here would let that
/// sign-in be replayed.
async fn claim(ctx: &Ctx, challenge: &[u8], purpose: Purpose) -> AppResult<ChallengeRow> {
    let row: Option<ChallengeRow> = q(
        &ctx.db,
        "DELETE FROM passkey_challenges WHERE hash = ? AND purpose = ? AND expires_at > ? \
         RETURNING purpose, user_id, sign_in_hash",
        &[
            s(&hex::encode(crypto::sha256(challenge))),
            s(purpose.as_str()),
            n(ctx.now),
        ],
    )?
    .first()
    .await?;
    row.filter(|r| r.purpose == purpose.as_str())
        .ok_or_else(stale)
}

// ───────────────────────────── making one ─────────────────────────────

/// Making a passkey takes proof it's the person: a sign-in in this browser
/// within the last 15 minutes, or else the password, or else, without one,
/// a code from the authenticator app. A passkey signs in without either,
/// so a stolen session must not be able to add one.
async fn prove(ctx: &Ctx, user: &UserRow, body: &PasskeyOptionsRequest) -> AppResult<()> {
    if crate::api::two_factor::fresh_sign_in(ctx) {
        return Ok(());
    }
    if let Some(phc) = user.password_hash.as_deref() {
        let given = body.password.as_deref().unwrap_or_default();
        if given.is_empty() {
            return Err(validation(
                "password",
                "Enter your password to add a passkey",
            ));
        }
        if !auth::verify_password(given, phc) {
            return Err(validation("password", "That isn't your password"));
        }
        return Ok(());
    }
    if user.has_second_factor() {
        let code = body
            .code
            .as_deref()
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| {
                validation(
                    "code",
                    "Enter a code from your authenticator app to add a passkey",
                )
            })?;
        crate::api::two_factor::confirm_code(ctx, user, code).await?;
        return Ok(());
    }
    Err(validation(
        "sign_in",
        "Sign out and in again, then add the passkey: it takes a sign-in from the last 15 minutes.",
    ))
}

/// `POST /api/v1/me/passkeys/options`: what the browser needs to make a
/// passkey.
pub async fn creation_options(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: PasskeyOptionsRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("passkey-new:{}", user.id)).await?;
    prove(ctx, &user, &body).await?;
    let existing = rows(ctx, &user.id).await?;
    if existing.len() as i64 >= MAX_PASSKEYS {
        return Err(conflict(
            "You have as many passkeys as an account can have. Remove one first.",
        ));
    }
    let challenge = challenge(ctx, Purpose::Register, Some(&user.id), None).await?;
    http::ok(&PasskeyCreationOptions {
        rp: PasskeyRp {
            id: rp_id(ctx)?,
            name: crate::instance::name(&ctx.db, &ctx.cfg).await?,
        },
        user: PasskeyUser {
            id: user_handle(&user.id),
            name: user.email.clone(),
            display_name: user.name.clone(),
        },
        challenge,
        pub_key_cred_params: ALGORITHMS
            .iter()
            .map(|alg| PasskeyAlgorithm {
                kind: "public-key".into(),
                alg: *alg,
            })
            .collect(),
        timeout: TIMEOUT_MS,
        exclude_credentials: existing.iter().map(PasskeyRow::descriptor).collect(),
        authenticator_selection: PasskeyAuthenticatorSelection {
            resident_key: "required".into(),
            require_resident_key: true,
            user_verification: "required".into(),
        },
        attestation: "none".into(),
    })
}

/// A passkey's name: what its person typed, its provider's, or the device's
/// the browser suggests.
fn clean_name(name: &str) -> Option<String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    (!name.is_empty()).then(|| name.chars().take(MAX_NAME).collect())
}

/// `POST /api/v1/me/passkeys`: keeps the passkey the browser just made.
pub async fn add(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: AddPasskeyRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    ctx.rate_limit(&format!("passkey-new:{}", user.id)).await?;
    let credential = &body.credential;
    let client_data_json = from_b64url(&credential.response.client_data_json)
        .ok_or_else(|| refused(webauthn::Error::Malformed))?;
    let client = webauthn::client_data(&client_data_json).map_err(refused)?;
    let row = claim(ctx, &client.challenge, Purpose::Register).await?;
    if row.user_id.as_deref() != Some(user.id.as_str()) {
        return Err(stale());
    }
    let attestation = from_b64url(&credential.response.attestation_object)
        .ok_or_else(|| refused(webauthn::Error::Malformed))?;
    let rp = rp_id(ctx)?;
    let origin = |o: &str| own_origin(ctx, o);
    let made = webauthn::check_registration(
        &client,
        &attestation,
        &Expect {
            rp_id: &rp,
            origin: &origin,
            user_verification: true,
        },
    )
    .map_err(refused)?;
    if from_b64url(&credential.raw_id).as_deref() != Some(made.credential_id.as_slice()) {
        return Err(refused(webauthn::Error::Malformed));
    }
    let credential_id = b64url(&made.credential_id);
    let aaguid = webauthn::uuid(&made.aaguid);
    let name = webauthn::provider(&aaguid)
        .map(str::to_owned)
        .or_else(|| body.name.as_deref().and_then(clean_name))
        .unwrap_or_else(|| "Passkey".to_owned());
    let transports = credential
        .response
        .transports
        .as_ref()
        .map(|t| {
            let known: Vec<&String> = t
                .iter()
                .filter(|t| {
                    matches!(
                        t.as_str(),
                        "ble" | "hybrid" | "internal" | "nfc" | "smart-card" | "usb"
                    )
                })
                .collect();
            serde_json::to_string(&known)
        })
        .transpose()?;
    let id = fctl_domain::ids::passkey_id(ctx.now);
    // In one statement: at most MAX_PASSKEYS, and each passkey once.
    let inserted = q(
        &ctx.db,
        "INSERT INTO passkeys (id, user_id, credential_id, public_key, algorithm, sign_count, transports, aaguid, backed_up, name, created_at) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11 \
         WHERE (SELECT COUNT(*) FROM passkeys WHERE user_id = ?2) < ?12 \
         ON CONFLICT (credential_id) DO NOTHING",
        &[
            s(id.as_str()),
            s(&user.id),
            s(&credential_id),
            s(&b64url(&made.public_key_cose)),
            n(made.public_key.algorithm()),
            n(i64::from(made.sign_count)),
            os(transports.as_deref()),
            s(&aaguid),
            crate::db::b(made.backed_up),
            s(&name),
            n(ctx.now),
            n(MAX_PASSKEYS),
        ],
    )?
    .run()
    .await?;
    if inserted == 0 {
        return Err(conflict(
            "This passkey is added already, or you have as many as an account can have.",
        ));
    }
    ctx.audit(
        "passkey.added",
        Some(&user.id),
        Some(serde_json::json!({ "name": name, "synced": made.backed_up })),
    );
    // A passkey is a second factor: workspaces that require one open now.
    crate::workspaces::changed(ctx, vec![user.id.clone()]);
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    crate::email::notice(
        ctx,
        &user,
        "A passkey was added to your account",
        format!(
            "A passkey, \"{name}\", now signs in to your {instance} account ({}) without a password or code.\n\nIf you didn't add it, remove it in Settings now and change your password: {}",
            user.email,
            ctx.cfg.url("/settings")
        ),
    );
    me(ctx, &user.id).await
}

/// One of the caller's passkeys, by id.
async fn own(ctx: &Ctx, user_id: &str, id: &str) -> AppResult<PasskeyRow> {
    let row: Option<PasskeyRow> = q(
        &ctx.db,
        "SELECT * FROM passkeys WHERE id = ? AND user_id = ?",
        &[s(id), s(user_id)],
    )?
    .first()
    .await?;
    row.ok_or_else(|| not_found("Passkey"))
}

/// `PATCH /api/v1/me/passkeys/{id}`: renames a passkey.
pub async fn rename(ctx: &mut Ctx, req: &mut Request, id: &str) -> AppResult<Response> {
    let body: UpdatePasskeyRequest = http::body(req).await?;
    let user = ctx.user()?.clone();
    let passkey = own(ctx, &user.id, id).await?;
    let name = clean_name(&body.name).ok_or_else(|| validation("name", "Give it a name"))?;
    q(
        &ctx.db,
        "UPDATE passkeys SET name = ? WHERE id = ?",
        &[s(&name), s(&passkey.id)],
    )?
    .run()
    .await?;
    me(ctx, &user.id).await
}

/// `DELETE /api/v1/me/passkeys/{id}`: removes a passkey, unless it is the
/// last way in, or the second factor the instance requires.
pub async fn remove(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let passkey = own(ctx, &user.id, id).await?;
    let others = user.passkeys - 1;
    let identities: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM identities WHERE user_id = ?",
        &[s(&user.id)],
    )?
    .first()
    .await?;
    if others == 0 && !user.has_password() && identities.is_none_or(|c| c.n == 0) {
        return Err(conflict(
            "This passkey is your only way to sign in. Set a password or link Google first.",
        ));
    }
    let instance = crate::instance::settings(&ctx.db).await?;
    if others == 0
        && !policy::can_remove_last_passkey(&instance, user.has_password(), user.factors())
    {
        return Err(conflict(
            "This instance requires two-factor sign-in, and this passkey is yours. Set up an authenticator app first, or add another passkey.",
        ));
    }
    if others == 0
        && !policy::can_remove_last_passkey(
            &user.two_factor_settings(&instance),
            user.has_password(),
            user.factors(),
        )
    {
        return Err(conflict(
            "A workspace you are in requires two-factor sign-in, and this passkey is yours. Set up an authenticator app first, add another passkey, or leave the workspace.",
        ));
    }
    // The checks again, in the statement, for removals that race.
    let removed = q(
        &ctx.db,
        "DELETE FROM passkeys WHERE id = ?1 AND user_id = ?2 \
         AND (EXISTS (SELECT 1 FROM passkeys o WHERE o.user_id = ?2 AND o.id != ?1) \
              OR EXISTS (SELECT 1 FROM users u WHERE u.id = ?2 AND u.password_hash IS NOT NULL) \
              OR EXISTS (SELECT 1 FROM identities i WHERE i.user_id = ?2)) \
         AND (?3 = 0 OR EXISTS (SELECT 1 FROM passkeys o WHERE o.user_id = ?2 AND o.id != ?1) \
              OR EXISTS (SELECT 1 FROM authenticator_apps a WHERE a.user_id = ?2))",
        &[
            s(&passkey.id),
            s(&user.id),
            crate::db::b(policy::required(
                &user.two_factor_settings(&instance),
                user.has_password(),
            )),
        ],
    )?
    .run()
    .await?;
    if removed == 0 {
        return Err(conflict(
            "Your sign-in methods changed meanwhile. Reload the page and try again.",
        ));
    }
    ctx.audit(
        "passkey.removed",
        Some(&user.id),
        Some(serde_json::json!({ "name": passkey.name })),
    );
    crate::workspaces::changed(ctx, vec![user.id.clone()]);
    me(ctx, &user.id).await
}

// ───────────────────────────── signing in ─────────────────────────────

fn request_options(
    challenge: String,
    rp_id: String,
    allow: Vec<PasskeyDescriptor>,
    user_verification: &str,
) -> PasskeyRequestOptions {
    PasskeyRequestOptions {
        challenge,
        rp_id,
        timeout: TIMEOUT_MS,
        allow_credentials: allow,
        user_verification: user_verification.into(),
    }
}

/// `POST /api/v1/auth/passkey/options`: what the browser needs to sign in
/// with a passkey; the sign-in page asks on load, so that the browser can
/// offer passkeys in the email field.
pub async fn sign_in_options(ctx: &mut Ctx) -> AppResult<Response> {
    http::ok(&request_options(
        sign_in_challenge(ctx)?,
        rp_id(ctx)?,
        Vec::new(),
        "required",
    ))
}

/// What a checked passkey answer was for.
struct Signed {
    challenge: ChallengeRow,
    passkey: PasskeyRow,
}

/// Checks a passkey's answer to one of our challenges: the challenge, the
/// passkey, the device's checks and the signature. Moves the counter on.
async fn check(
    ctx: &Ctx,
    answer: &PasskeyAssertion,
    purpose: Purpose,
    user_verification: bool,
) -> AppResult<Signed> {
    let malformed = || refused(webauthn::Error::Malformed);
    let client_data_json = from_b64url(&answer.response.client_data_json).ok_or_else(malformed)?;
    let client = webauthn::client_data(&client_data_json).map_err(refused)?;
    let challenge = match purpose {
        Purpose::SignIn => spend(ctx, &client.challenge).await?,
        _ => claim(ctx, &client.challenge, purpose).await?,
    };
    let credential_id = b64url(&from_b64url(&answer.raw_id).ok_or_else(malformed)?);
    let passkey: Option<PasskeyRow> = q(
        &ctx.db,
        "SELECT * FROM passkeys WHERE credential_id = ?",
        &[s(&credential_id)],
    )?
    .first()
    .await?;
    let Some(passkey) = passkey else {
        // The browser can tell its provider to forget it (signalUnknownCredential).
        return Err(AppError(ApiError::new(
            ErrorCode::NotFound,
            "This passkey isn't on this account anymore. Sign in another way, and remove it from your passkey provider.",
        )));
    };
    if let Some(handle) = answer.response.user_handle.as_deref()
        && handle != user_handle(&passkey.user_id)
    {
        return Err(malformed());
    }
    let authenticator_data =
        from_b64url(&answer.response.authenticator_data).ok_or_else(malformed)?;
    let rp = rp_id(ctx)?;
    let origin = |o: &str| own_origin(ctx, o);
    let asserted = webauthn::check_assertion(
        &client,
        &client_data_json,
        &authenticator_data,
        &Expect {
            rp_id: &rp,
            origin: &origin,
            user_verification,
        },
    )
    .map_err(refused)?;
    let cose = from_b64url(&passkey.public_key).ok_or_else(|| internal("stored passkey key"))?;
    let key = cbor::decode_all(&cose)
        .ok()
        .and_then(|k| webauthn::public_key(&k).ok())
        .ok_or_else(|| internal("stored passkey key"))?;
    let signature = from_b64url(&answer.response.signature).ok_or_else(malformed)?;
    if !crypto::verify_passkey(&key, &signature, &asserted.signed).await? {
        return Err(refused(webauthn::Error::Malformed));
    }
    let stored = u32::try_from(passkey.sign_count.max(0)).unwrap_or(u32::MAX);
    if !webauthn::counter_ok(stored, asserted.sign_count) {
        ctx.audit(
            "passkey.counter_went_back",
            Some(&passkey.user_id),
            Some(serde_json::json!({ "name": passkey.name, "stored": stored, "received": asserted.sign_count })),
        );
        return Err(validation(
            "passkey",
            "This passkey's counter went back, which a copied passkey's would. Sign in another way, and remove the passkey in Settings.",
        ));
    }
    q(
        &ctx.db,
        "UPDATE passkeys SET sign_count = ?, backed_up = ?, last_used_at = ? WHERE id = ?",
        &[
            n(i64::from(asserted.sign_count)),
            crate::db::b(asserted.backed_up),
            n(ctx.now),
            s(&passkey.id),
        ],
    )?
    .run()
    .await?;
    Ok(Signed { challenge, passkey })
}

/// `POST /api/v1/auth/passkey`: signs in with a passkey. The device checked
/// who it is, so no second factor follows.
pub async fn sign_in(ctx: &mut Ctx, req: &mut Request) -> AppResult<Response> {
    let body: PasskeySignInRequest = http::body(req).await?;
    ctx.rate_limit(&format!("passkey:{}", ctx.ip_key())).await?;
    let signed = check(ctx, &body.credential, Purpose::SignIn, true).await?;
    let user = auth::user_by_id(ctx, &signed.passkey.user_id)
        .await?
        .ok_or_else(|| not_found("User"))?;
    if !user.is_active() {
        return Err(AppError(ApiError::new(
            ErrorCode::AccessDenied,
            "Your account is disabled. Ask an admin for access.",
        )));
    }
    auth::start_web_session(ctx, &user).await?;
    ctx.audit(
        "user.login",
        Some(&user.id),
        Some(serde_json::json!({ "via": "passkey", "passkey": signed.passkey.name })),
    );
    http::ok(&crate::api::auth::signed_in(auth::me(ctx, &user).await?))
}

/// `POST /api/v1/auth/two-factor/passkey`: options to finish the waiting
/// sign-in with one of its person's passkeys.
pub async fn second_factor_options(ctx: &mut Ctx, req: &Request) -> AppResult<Response> {
    http::check_same_origin(req, ctx.cfg.origin(), ctx.cfg.dev)?;
    let (hash, user_id) = crate::api::two_factor::waiting_sign_in(ctx, req).await?;
    ctx.rate_limit(&format!("2fa-user:{user_id}")).await?;
    let allow: Vec<PasskeyDescriptor> = rows(ctx, &user_id)
        .await?
        .iter()
        .map(PasskeyRow::descriptor)
        .collect();
    if allow.is_empty() {
        return Err(conflict(
            "This account has no passkeys. Enter a code instead.",
        ));
    }
    let challenge = challenge(ctx, Purpose::SecondFactor, Some(&user_id), Some(&hash)).await?;
    http::ok(&request_options(challenge, rp_id(ctx)?, allow, "preferred"))
}

/// Checks a passkey that finishes the sign-in `sign_in_hash` of `user`.
pub async fn second_factor(
    ctx: &Ctx,
    user: &UserRow,
    sign_in_hash: &str,
    answer: &PasskeyAssertion,
) -> AppResult<String> {
    let signed = check(ctx, answer, Purpose::SecondFactor, false).await?;
    if signed.challenge.sign_in_hash.as_deref() != Some(sign_in_hash)
        || signed.challenge.user_id.as_deref() != Some(user.id.as_str())
        || signed.passkey.user_id != user.id
    {
        return Err(stale());
    }
    Ok(signed.passkey.name)
}

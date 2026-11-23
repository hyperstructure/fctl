//! Signing in with a provider, whichever it is (Google today), and what
//! instance admins see and change of how people sign in.
//!
//! A provider's own module checks what the provider sent (`api::google`)
//! and hands it over as [`Provided`]. [`finish`] then links the provider's
//! account to the person's, signs them in or makes their account, as the
//! instance's sign-in rules say (`fctl_domain::sign_in`):
//!
//! - An account a rule covers signs in only with the rule's provider, as an
//!   address at one of its domains that the provider vouches for.
//! - A provider's first sign-in links to the account of the address it
//!   vouches for when a rule makes it the way in for that address's domain,
//!   or when the account has no way in at all (an admin token made it).
//!   Otherwise people link it from Settings, as linking by address would
//!   let one account take over another.
//! - An account a rule covers loses its password at its first sign-in that
//!   meets the rule, and with it the sign-ins made another way: the
//!   password signs nobody in any more. The primary instance owner keeps
//!   theirs, as the way in while the provider is down.

use crate::auth;
use crate::ctx::Ctx;
use crate::db::{Count, batch, n, os, q, s};
use crate::error::{AppError, AppResult, conflict, not_found};
use crate::http;
use crate::model::{UserRow, user_columns};
use fctl_domain::allowlist::AllowPattern;
use fctl_domain::sign_in::{self as rules, SignedIn};
use fctl_proto::api::*;
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use worker::{D1Database, Request, Response};

/// What a provider's sign-in said of the person, once its module checked
/// it came from the provider.
#[derive(Clone, Debug)]
pub struct Provided {
    /// The provider's id (`google`).
    pub provider: &'static str,
    /// Its account's id, which never changes (Google's `sub`).
    pub subject: String,
    /// The address its account shows, lowercase.
    pub email: Option<String>,
    /// Whether the provider says it verified `email`.
    pub verified: bool,
    /// `email`, where the provider vouches that the person holds it now
    /// (Google: where it hosts it, `fctl_domain::google::vouches_for`).
    pub vouched: Option<String>,
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub picture: Option<String>,
}

impl Provided {
    /// How this sign-in was made, as the session it starts keeps it.
    fn how(&self) -> SignedIn {
        SignedIn::provider(self.provider, self.vouched.as_deref())
    }

    /// The provider's error `code`, which the pages that started it
    /// explain (`google_link_required`).
    fn code(&self, code: &str) -> String {
        format!("{}_{code}", self.provider)
    }
}

/// What the sign-in was for.
pub enum Intent {
    /// Signing in, then going to `return_to`.
    Login { return_to: String },
    /// Linking the provider's account to `user_id`'s, from their Settings.
    Link { user_id: String, return_to: String },
}

/// `path` when it is a path of the app, else the home page.
pub fn safe_return(path: Option<String>) -> String {
    path.filter(|p| fctl_domain::validate::is_app_path(p))
        .unwrap_or_else(|| "/".to_owned())
}

/// Redirects back into the app with an error the page explains, and the
/// query's `extra` (`&domains=acme.com`).
pub fn fail(page: &str, code: &str, extra: &str) -> AppResult<Response> {
    let sep = if page.contains('?') { '&' } else { '?' };
    http::redirect(&format!("{page}{sep}error={code}{extra}"))
}

/// The refusal of a sign-in with a provider's account that doesn't meet
/// the rule covering the account: the page names the domains to sign in
/// as.
fn wrong_account(p: &Provided, required: &rules::Required) -> AppResult<Response> {
    let domains = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("domains", &required.domains.join(","))
        .append_pair("provider", &required.provider)
        .finish();
    fail("/login", &p.code("wrong_account"), &format!("&{domains}"))
}

/// Finishes a provider's sign-in, which said `p`.
pub async fn finish(ctx: &mut Ctx, p: Provided, intent: Intent) -> AppResult<Response> {
    let linked: Option<UserRow> = q(
        &ctx.db,
        &format!(
            "SELECT {} FROM identities i JOIN users u ON u.id = i.user_id WHERE i.provider = ? AND i.subject = ?",
            user_columns("u")
        ),
        &[s(p.provider), s(&p.subject)],
    )?
    .first()
    .await?;
    match intent {
        Intent::Link { user_id, return_to } => link(ctx, &p, linked, &user_id, &return_to).await,
        Intent::Login { return_to } => login(ctx, &p, linked, &return_to).await,
    }
}

/// Links the provider's account to `user_id`'s, from their Settings.
async fn link(
    ctx: &mut Ctx,
    p: &Provided,
    linked: Option<UserRow>,
    user_id: &str,
    return_to: &str,
) -> AppResult<Response> {
    let Some(user) = auth::user_by_id(ctx, user_id)
        .await?
        .filter(UserRow::is_active)
    else {
        return fail("/settings", &p.code("state"), "");
    };
    let done = format!(
        "{}?linked={}",
        safe_return(Some(return_to.to_owned())),
        p.provider
    );
    if let Some(other) = &linked {
        if other.id != user_id {
            return fail("/settings", &p.code("linked_elsewhere"), "");
        }
        refresh(&ctx.db, p).await?;
        return http::redirect(&done);
    }
    identity_insert(&ctx.db, user_id, p, ctx.now)?.run().await?;
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    let name = crate::sign_in::provider_name(p.provider);
    crate::email::notice(
        ctx,
        &user,
        &format!("A {name} account was linked to your account"),
        format!(
            "The {name} account {} now signs in to your {instance} account ({}).\n\nIf you didn't link it, unlink it in Settings now and change your password: {}",
            p.email.as_deref().unwrap_or("without an email address"),
            user.email,
            ctx.cfg.url("/settings?tab=security")
        ),
    );
    if user.avatar_url.is_none() {
        adopt_picture(ctx, user_id, p).await;
    }
    ctx.audit(
        "identity.linked",
        Some(user_id),
        Some(serde_json::json!({ "provider": p.provider })),
    );
    http::redirect(&done)
}

/// Signs in with the provider's account: the account it is linked to, the
/// account of the address it vouches for, or a new one.
async fn login(
    ctx: &mut Ctx,
    p: &Provided,
    linked: Option<UserRow>,
    return_to: &str,
) -> AppResult<Response> {
    let settings = crate::instance::settings(&ctx.db).await?;
    let how = p.how();
    if let Some(user) = linked {
        if !user.is_active() {
            return fail("/login", "disabled", "");
        }
        // Their own Gmail, say, where their organization's domain signs in
        // with its Google Workspace.
        if let Err(required) = user.admits(&settings, &how) {
            return wrong_account(p, &required);
        }
        refresh(&ctx.db, p).await?;
        return sign_in(ctx, user, return_to, how, &settings).await;
    }
    if p.email.is_none() || !p.verified {
        return fail("/login", &p.code("unverified"), "");
    }
    // A provider vouches for an address only where it is authoritative for
    // it (Google: Gmail's, or a Google Workspace account's): any other an
    // account shows was verified once, when the account was made, and may
    // be someone else's now (a former employee's, a recycled one). Signing
    // up with one takes the invitation sent to it, which only its mailbox
    // gets.
    let Some(email) = p.vouched.clone() else {
        return fail("/login", &p.code("not_authoritative"), "");
    };
    if let Some(user) = auth::user_by_email(ctx, &email).await? {
        return first_sign_in(ctx, user, p, &email, return_to, &settings).await;
    }
    // A new account, where the address's domain doesn't require another
    // way in.
    if let Err(required) = rules::admits(&settings.sign_in, &[&email], false, &how) {
        return wrong_account(p, &required);
    }
    let Some((pattern, role)) = crate::api::auth::allow_match(ctx, &email).await? else {
        return fail("/login", "not_allowed", "");
    };
    // Accounts only from the directory: a domain's entry admits the
    // accounts at it and makes none; an invitation's or an admin's entry
    // for the address itself does.
    if !settings.self_sign_up && !AllowPattern::parse(&pattern).is_ok_and(|a| a.is_exact()) {
        return fail("/login", "sign_up_closed", "");
    }
    sign_up(ctx, p, &email, role, return_to).await
}

/// Makes an account for `email`, which the provider's account of `p`
/// signs in to, and signs them in.
async fn sign_up(
    ctx: &mut Ctx,
    p: &Provided,
    email: &str,
    role: fctl_proto::session::InstanceRole,
    return_to: &str,
) -> AppResult<Response> {
    let id = fctl_domain::ids::user_id(ctx.now);
    let role = if role == fctl_proto::session::InstanceRole::Admin {
        "admin"
    } else {
        "member"
    };
    // The provider's first and last name when it has them; else its full
    // name or the email's local part.
    let fallback = p
        .name
        .clone()
        .unwrap_or_else(|| email.split('@').next().unwrap_or("New user").to_owned());
    let person =
        fctl_domain::validate::person_name(p.given_name.as_deref(), p.family_name.as_deref(), None)
            .or_else(|_| fctl_domain::validate::person_name(None, None, Some(&fallback)))
            .or_else(|_| fctl_domain::validate::person_name(None, None, Some("New user")))?;
    // At once, and failing whole: an address another account took since
    // it was looked up (connecting it, say) makes no second account for it.
    batch(
        &ctx.db,
        vec![
            q(
                &ctx.db,
                "INSERT INTO users (id, email, name, given_name, family_name, role, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?)",
                &[
                    s(id.as_str()),
                    s(email),
                    s(&person.full),
                    os(person.given.as_deref()),
                    os(person.family.as_deref()),
                    s(role),
                    n(ctx.now),
                    n(ctx.now),
                ],
            )?,
            q(
                &ctx.db,
                "INSERT INTO user_emails (email, user_id, created_at) VALUES (?, ?, ?)",
                &[s(email), s(id.as_str()), n(ctx.now)],
            )?,
            identity_insert(&ctx.db, id.as_str(), p, ctx.now)?,
        ],
    )
    .await?;
    let user = auth::user_by_id(ctx, id.as_str())
        .await?
        .ok_or_else(|| not_found("User"))?;
    auth::start_web_session(ctx, &user, false, p.how()).await?;
    // Their Gravatar first, which only people who chose a picture have;
    // else the provider's, which is a letter for those who didn't.
    let user = crate::api::auth::new_gravatar(ctx, user).await?;
    if user.avatar_url.is_none() {
        adopt_picture(ctx, &user.id, p).await;
    }
    ctx.audit(
        "user.signup",
        Some(&user.id),
        Some(serde_json::json!({ "via": p.provider })),
    );
    crate::api::workspaces::joined_after_sign_up(ctx, &user, email).await?;
    http::redirect(return_to)
}

/// Signs in `user`, whom the provider's account signs in to and whose
/// sign-in rules it meets, and goes on to `return_to`: by way of the page
/// that asks for a code, when their two-factor sign-in asks for one on the
/// provider's sign-ins. Where a rule covers them, their password goes
/// first.
async fn sign_in(
    ctx: &mut Ctx,
    user: UserRow,
    return_to: &str,
    how: SignedIn,
    settings: &InstanceSettings,
) -> AppResult<Response> {
    if !user.is_active() {
        return fail("/login", "disabled", "");
    }
    let user = match user.sign_in_required(settings) {
        Some(_) => move_off_password(ctx, user, &how, settings).await?,
        None => user,
    };
    let two_factor = user.two_factor_settings(settings);
    if fctl_domain::two_factor::asks(
        &two_factor,
        fctl_domain::two_factor::Via::Google,
        user.has_password(),
        user.factors(),
        user.two_factor_google,
    ) {
        // The page that asks for the code finishes it, then goes on to
        // `return_to`.
        crate::api::two_factor::challenge(
            ctx,
            &user,
            ChallengeReason::Google,
            crate::api::two_factor::Pending {
                return_to: Some(return_to.to_owned()),
                auth_domain: how.domain.clone(),
                ..Default::default()
            },
        )
        .await?;
        return http::redirect("/two-factor");
    }
    let via = how.method_str().map(str::to_owned);
    auth::start_web_session(ctx, &user, false, how).await?;
    ctx.audit(
        "user.login",
        Some(&user.id),
        Some(serde_json::json!({ "via": via })),
    );
    http::redirect(return_to)
}

/// Takes the password of `user`, whom a rule covers, at a sign-in `how`
/// that meets it: it signs them in no more. Their reset links and the
/// sign-ins made another way go with it, so that none comes back if the
/// rule does. Returns them as they are now.
async fn move_off_password(
    ctx: &Ctx,
    user: UserRow,
    how: &SignedIn,
    settings: &InstanceSettings,
) -> AppResult<UserRow> {
    let addresses = user.addresses();
    let addresses: Vec<&str> = addresses.iter().map(String::as_str).collect();
    let covering = rules::covering(&settings.sign_in, &addresses);
    // The sign-ins that meet one of the rules, as `method domain`.
    let meeting: Vec<String> = covering
        .iter()
        .flat_map(|r| r.domains.iter().map(|d| format!("{} {d}", r.provider)))
        .collect();
    let passkeys = covering.iter().any(|r| r.passkeys);
    let mut statements = vec![q(
        &ctx.db,
        "DELETE FROM tokens WHERE user_id = ?1 \
         AND NOT (COALESCE(auth_method, '') || ' ' || COALESCE(auth_domain, '') IN (SELECT value FROM json_each(?2)) \
           OR (?3 AND COALESCE(auth_method, '') = 'passkey'))",
        &[
            s(&user.id),
            s(&serde_json::to_string(&meeting)?),
            crate::db::b(passkeys),
        ],
    )?];
    if user.has_password() {
        statements.extend([
            q(
                &ctx.db,
                "UPDATE users SET password_hash = NULL, updated_at = ? WHERE id = ? AND role != 'owner'",
                &[n(ctx.now), s(&user.id)],
            )?,
            q(
                &ctx.db,
                "UPDATE password_resets SET used_at = ? WHERE user_id = ? AND used_at IS NULL",
                &[n(ctx.now), s(&user.id)],
            )?,
            q(
                &ctx.db,
                "DELETE FROM sign_in_challenges WHERE user_id = ? AND reason IN ('password', 'password_reset')",
                &[s(&user.id)],
            )?,
        ]);
        ctx.audit(
            "user.password_removed",
            Some(&user.id),
            Some(serde_json::json!({ "via": "sign_in_rule", "provider": how.method_str() })),
        );
    }
    batch(&ctx.db, statements).await?;
    Ok(auth::user_by_id(ctx, &user.id).await?.unwrap_or(user))
}

/// A provider's sign-in with `email`, the address of `user`, whom none of
/// its accounts signs in to yet; the provider vouches that the person
/// holds the address ([`first`]).
async fn first_sign_in(
    ctx: &mut Ctx,
    user: UserRow,
    p: &Provided,
    email: &str,
    return_to: &str,
    settings: &InstanceSettings,
) -> AppResult<Response> {
    let how = p.how();
    if user.is_active()
        && let Err(required) = user.admits(settings, &how)
    {
        return wrong_account(p, &required);
    }
    // A rule makes the provider the way in for the address's domain.
    let by_rule = settings.sign_in.iter().any(|r| rules::meets(r, &how));
    match first(&ctx.db, &user, p, email, ctx.now, by_rule).await? {
        First::Linked => {}
        First::Signs => return sign_in(ctx, user, return_to, how, settings).await,
        First::Refused(code) => return fail("/login", &code, ""),
    }
    let instance = crate::instance::name(&ctx.db, &ctx.cfg).await?;
    let name = crate::sign_in::provider_name(p.provider);
    crate::email::notice(
        ctx,
        &user,
        &format!("A {name} account was linked to your account"),
        if by_rule && !user.is_owner() {
            format!(
                "You signed in to your {instance} account ({}) with the {name} account {email}: it signs you in from now on, as {instance}'s admins require for your address.{}\n\nIf it wasn't you, tell an admin of {instance} now.",
                user.email,
                if user.has_password() {
                    " Your password no longer signs you in, so it was removed."
                } else {
                    ""
                }
            )
        } else {
            format!(
                "You signed in to your {instance} account ({}) for the first time, with the {name} account {email}: it signs you in from now on.\n\nIf it wasn't you, tell an admin of {instance} now. To sign in another way too, add a password or a passkey in Settings: {}",
                user.email,
                ctx.cfg.url("/settings?tab=security")
            )
        },
    );
    if user.avatar_url.is_none() {
        adopt_picture(ctx, &user.id, p).await;
    }
    let response = sign_in(ctx, user.clone(), return_to, how, settings).await;
    // After the sign-in, so that the log names them as who linked it, and
    // whether or not the sign-in went through. Linked for want of any way
    // in, it starts their session at once: two-factor sign-in asks for a
    // code only of accounts with an authenticator app or a passkey, which
    // nobody set up on an account nobody could sign in to.
    ctx.audit(
        "identity.linked",
        Some(&user.id),
        Some(serde_json::json!({
            "provider": p.provider,
            "via": if by_rule { "sign_in_rule" } else { "first_sign_in" },
        })),
    );
    response
}

/// What a provider's sign-in with the address of `user`, whom none of its
/// accounts signs in to yet, does with the account.
#[derive(Debug, PartialEq)]
enum First {
    /// It linked the provider's account to it.
    Linked,
    /// This account of the provider signs in to it already: another
    /// request's first sign-in with it linked it a moment ago.
    Signs,
    /// It refuses, with the error the sign-in page explains.
    Refused(String),
}

/// Linking a provider's account by address would let it take over an
/// account someone signs in to another way: they sign in that way, then
/// link it from Settings. Two kinds of account have nobody to take it from:
/// one with no way to sign in at all (no password, passkey or linked
/// account), as an admin token makes them for an address
/// (`api::provision`), and nobody has signed in to; and one whose domain a
/// rule makes this provider the way in for (`by_rule`), where the
/// provider is authoritative for the address and its account the way in
/// already. Their first sign-in with it links it.
async fn first(
    db: &D1Database,
    user: &UserRow,
    p: &Provided,
    email: &str,
    now: i64,
    by_rule: bool,
) -> AppResult<First> {
    if !user.is_active() {
        return Ok(First::Refused("disabled".to_owned()));
    }
    if link_first(db, &user.id, p, email, now, by_rule).await? {
        return Ok(First::Linked);
    }
    #[derive(Deserialize)]
    struct Owner {
        user_id: String,
    }
    let owner: Option<Owner> = q(
        db,
        "SELECT user_id FROM identities WHERE provider = ? AND subject = ?",
        &[s(p.provider), s(&p.subject)],
    )?
    .first()
    .await?;
    match owner {
        Some(o) if o.user_id == user.id => Ok(First::Signs),
        Some(_) => Ok(First::Refused(p.code("linked_elsewhere"))),
        None => Ok(First::Refused(link_required(user, p))),
    }
}

/// Links the provider's account of `p` to the account `user_id` at its
/// first sign-in, as one statement that checks again that the account is
/// active and has `email`, the address the provider vouched for, and,
/// unless a rule makes the provider the way in for it (`by_rule`), that it
/// has no way in: a password, a passkey or a linked account given it since
/// it was read, or an address an admin token changed meanwhile, leaves it
/// as it is. Whether it linked it.
async fn link_first(
    db: &D1Database,
    user_id: &str,
    p: &Provided,
    email: &str,
    now: i64,
    by_rule: bool,
) -> AppResult<bool> {
    let linked = q(
        db,
        "INSERT OR IGNORE INTO identities (id, user_id, provider, subject, email, name, avatar_url, created_at) \
         SELECT ?1, ?2, ?8, ?3, ?4, ?5, ?6, ?7 \
         WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = ?2 AND u.status = 'active' \
           AND (?9 OR u.password_hash IS NULL) \
           AND (u.email = ?4 OR EXISTS (SELECT 1 FROM user_emails e WHERE e.user_id = ?2 AND e.email = ?4))) \
         AND (?9 OR NOT EXISTS (SELECT 1 FROM passkeys p WHERE p.user_id = ?2)) \
         AND (?9 OR NOT EXISTS (SELECT 1 FROM identities i WHERE i.user_id = ?2))",
        &[
            s(fctl_domain::ids::identity_id(now).as_str()),
            s(user_id),
            s(&p.subject),
            s(email),
            os(p.name.as_deref()),
            os(p.picture.as_deref()),
            n(now),
            s(p.provider),
            crate::db::b(by_rule),
        ],
    )?
    .run()
    .await?;
    Ok(linked == 1)
}

/// Why a provider's sign-in didn't link to `user`, who signs in another
/// way, as the sign-in page tells them: sign in that way, then link it
/// from Settings.
fn link_required(user: &UserRow, p: &Provided) -> String {
    p.code(if user.has_password() {
        "link_required"
    } else if user.passkeys > 0 {
        "link_passkey"
    } else {
        // Another account of the provider.
        "link_other"
    })
}

/// Keeps what the provider's account shows now: its address, name and
/// picture, as Settings shows it.
async fn refresh(db: &D1Database, p: &Provided) -> AppResult<()> {
    q(
        db,
        "UPDATE identities SET email = COALESCE(?, email), name = COALESCE(?, name), avatar_url = COALESCE(?, avatar_url) \
         WHERE provider = ? AND subject = ?",
        &[
            os(p.email.as_deref()),
            os(p.name.as_deref()),
            os(p.picture.as_deref()),
            s(p.provider),
            s(&p.subject),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// The provider's picture as the avatar of someone who has none: a copy
/// (`api::avatars::adopt_google`). Without it, they keep their initials
/// until they upload one.
async fn adopt_picture(ctx: &Ctx, user_id: &str, p: &Provided) {
    if p.provider == rules::GOOGLE
        && let Some(picture) = p.picture.as_deref()
        && let Err(e) = crate::api::avatars::adopt_google(ctx, user_id, picture).await
    {
        worker::console_log!("google picture not copied: {e}");
    }
}

/// The statement that links the provider's account of `p` to `user_id`.
fn identity_insert(
    db: &D1Database,
    user_id: &str,
    p: &Provided,
    now: i64,
) -> AppResult<crate::db::Query> {
    q(
        db,
        "INSERT INTO identities (id, user_id, provider, subject, email, name, avatar_url, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        &[
            s(fctl_domain::ids::identity_id(now).as_str()),
            s(user_id),
            s(p.provider),
            s(&p.subject),
            os(p.email.as_deref()),
            os(p.name.as_deref()),
            os(p.picture.as_deref()),
            n(now),
        ],
    )
}

#[derive(Deserialize)]
struct IdentityBrief {
    id: String,
    provider: String,
    email: Option<String>,
}

/// `DELETE /api/v1/auth/identities/{id}`: unlinks one of the caller's
/// providers' accounts, unless it is their last way in, or the one their
/// domain's rule takes.
pub async fn unlink(ctx: &mut Ctx, id: &str) -> AppResult<Response> {
    let user = ctx.user()?.clone();
    let mine: Vec<IdentityBrief> = q(
        &ctx.db,
        "SELECT id, provider, email FROM identities WHERE user_id = ?",
        &[s(&user.id)],
    )?
    .all()
    .await?;
    let Some(target) = mine.iter().find(|i| i.id == id) else {
        return Err(not_found("Linked account"));
    };
    let settings = ctx.settings();
    if let Some(required) = user.sign_in_required(&settings) {
        let addresses = user.addresses();
        let addresses: Vec<&str> = addresses.iter().map(String::as_str).collect();
        let covering = rules::covering(&settings.sign_in, &addresses);
        let meets = |i: &IdentityBrief| {
            let how = SignedIn::provider(&i.provider, i.email.as_deref());
            covering.iter().any(|r| rules::meets(r, &how))
        };
        if meets(target) && !mine.iter().any(|i| i.id != id && meets(i)) {
            return Err(conflict(&format!(
                "{}, as this account: it stays linked while it is your way in.",
                crate::sign_in::says(&required)
            )));
        }
    }
    let others: Option<Count> = q(
        &ctx.db,
        "SELECT COUNT(*) AS n FROM identities WHERE user_id = ? AND id != ?",
        &[s(&user.id), s(id)],
    )?
    .first()
    .await?;
    if user.password_hash.is_none() && user.passkeys == 0 && others.map_or(0, |c| c.n) == 0 {
        return Err(AppError(ApiError::new(
            ErrorCode::Conflict,
            "Set a password or add a passkey first, or you would have no way to sign in",
        )));
    }
    let changed = q(
        &ctx.db,
        "DELETE FROM identities WHERE id = ? AND user_id = ?",
        &[s(id), s(&user.id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Err(not_found("Linked account"));
    }
    ctx.audit(
        "identity.unlinked",
        Some(&user.id),
        Some(serde_json::json!({ "provider": target.provider })),
    );
    http::ok(&auth::me(ctx, &user).await?)
}

// ───────────────────────────── instance admins ─────────────────────────────

/// `GET /api/v1/admin/sign-in`: how people sign in to the instance.
pub async fn overview(ctx: &Ctx) -> AppResult<Response> {
    ctx.admin()?;
    let settings = crate::instance::settings(&ctx.db).await?;
    let people = crate::sign_in::people(&ctx.db).await?;
    let how = signed_in_with(ctx);
    let count = |f: fn(&rules::Person) -> bool| people.iter().filter(|p| f(p)).count() as u32;
    http::ok(&SignInOverview {
        providers: crate::sign_in::statuses(&ctx.cfg, &people),
        coverage: settings
            .sign_in
            .iter()
            .map(|r| rules::coverage(r, &people))
            .collect(),
        rules: settings.sign_in,
        self_sign_up: settings.self_sign_up,
        active: people.len() as u32,
        passwords: count(|p| p.password),
        passkeys: count(|p| p.passkeys),
        no_way_in: count(rules::Person::no_way_in),
        owner: people.iter().find(|p| p.owner).map(rules::Person::to_api),
        method: how.method_str().map(str::to_owned),
        method_domain: how.domain,
        email: ctx.cfg.email.is_some(),
    })
}

/// How the caller's sign-in was made.
fn signed_in_with(ctx: &Ctx) -> SignedIn {
    ctx.auth
        .as_ref()
        .map(|a| a.signed_in_with.clone())
        .unwrap_or_default()
}

/// `POST /api/v1/admin/sign-in/preview`: what saving the request's rules
/// would do, before anything changes.
pub async fn preview(ctx: &Ctx, req: &mut Request) -> AppResult<Response> {
    let actor = ctx.admin()?.clone();
    let body: SignInPreviewRequest = http::body(req).await?;
    let after =
        rules::check_rules(&body.rules, &crate::sign_in::providers(&ctx.cfg)).map_err(AppError)?;
    let before = crate::instance::settings(&ctx.db).await?.sign_in;
    let people = crate::sign_in::people(&ctx.db).await?;
    let mut preview = rules::preview(&before, &after, &people, &actor.id, &signed_in_with(ctx));
    preview.email = ctx.cfg.email.is_some();
    http::ok(&preview)
}

/// A change to the rules that an admin saves, once it is checked.
pub struct Change {
    before: Vec<SignInRule>,
    people: Vec<rules::Person>,
}

/// Checks that saving `after` in place of `before` signs out no admin who
/// might not sign in again: the one who saves it, unless the sign-in they
/// save it with meets it, or they are the primary instance owner.
pub async fn check(ctx: &Ctx, before: &[SignInRule], after: &[SignInRule]) -> AppResult<Change> {
    let actor = ctx.user()?.clone();
    let people = crate::sign_in::people(&ctx.db).await?;
    let preview = rules::preview(before, after, &people, &actor.id, &signed_in_with(ctx));
    if preview.you == SignInYou::SignsOut {
        let addresses = actor.addresses();
        let addresses: Vec<&str> = addresses.iter().map(String::as_str).collect();
        let message = match rules::required(after, &addresses, false) {
            Some(required) => {
                let provider = crate::sign_in::provider_name(&required.provider);
                format!(
                    "This would sign you out: {}, and you signed in another way. Sign in with {provider} as your {} address first, which shows that it works, then save it.",
                    crate::sign_in::says(&required),
                    rules::domains_words(&required.domains).replace(" and ", " or ")
                )
            }
            None => {
                "This would sign you out. Sign in with the provider first, then save it.".to_owned()
            }
        };
        return Err(conflict(&message));
    }
    Ok(Change {
        before: before.to_vec(),
        people,
    })
}

/// Tells the people whose password, or passkeys alone, `change` stopped
/// signing them in, by email where the instance sends it, and logs the
/// change: in batches, after the response.
pub fn tell(ctx: &Ctx, change: Change, after: &[SignInRule]) {
    let affected = rules::affected(&change.before, after, &change.people);
    ctx.audit(
        "sign_in.rules_changed",
        None,
        Some(serde_json::json!({
            "rules": after,
            "passwords": affected.iter().filter(|(_, l)| l.password).count(),
            "passkeys": affected.iter().filter(|(_, l)| l.passkeys).count(),
        })),
    );
    let Some(mail) = ctx.cfg.email.clone() else {
        return;
    };
    let url = ctx.cfg.url("/login");
    let instance = ctx.cfg.instance_name.clone();
    let messages: Vec<(String, String, String)> = affected
        .iter()
        .filter_map(|(person, lost)| {
            let addresses: Vec<&str> = person.addresses.iter().map(String::as_str).collect();
            let required = rules::required(after, &addresses, false)?;
            let provider = crate::sign_in::provider_name(&required.provider);
            let mut lines = vec![format!(
                "The admins of {instance} now require signing in with {provider} at {}: from now on, you sign in to your account ({}) with {provider}, as your {} address. On the sign-in page, choose Continue with {provider}: {url}",
                rules::domains_words(&required.domains),
                person.email,
                rules::domains_words(&required.domains).replace(" and ", " or "),
            )];
            if lost.password {
                lines.push(
                    "Your password no longer signs you in, and goes at your first sign-in with it."
                        .to_owned(),
                );
            }
            if lost.passkeys {
                lines.push(format!(
                    "Your passkeys no longer sign you in alone. They still confirm it's you after {provider}'s sign-in."
                ));
            }
            Some((
                person.email.clone(),
                format!("You now sign in with {provider}"),
                lines.join("\n\n"),
            ))
        })
        .collect();
    if messages.is_empty() {
        return;
    }
    ctx.defer(async move { crate::email::send_each(&mail, &messages).await });
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_800_000_000_000;

    /// Bob, whom an admin token made as `api::provision::create` does: an
    /// active member, with an allowlist entry of his own and no way to sign
    /// in. Ada signs in with a password, Cy with a passkey and Dee with
    /// another Google account; Eve's account, made as Bob's was, is
    /// disabled.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, password_hash, role, status, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@acme.dev', 'Ada', 'hash', 'admin', 'active', 0, 0), \
             ('usr_bob', 'bob@acme.dev', 'Bob', NULL, 'member', 'active', 0, 0), \
             ('usr_cy', 'cy@acme.dev', 'Cy', NULL, 'member', 'active', 0, 0), \
             ('usr_dee', 'dee@acme.dev', 'Dee', NULL, 'member', 'active', 0, 0), \
             ('usr_eve', 'eve@acme.dev', 'Eve', NULL, 'member', 'disabled', 0, 0)",
            "INSERT INTO user_emails (email, user_id, created_at) VALUES ('ada@acme.dev', 'usr_ada', 0), \
             ('bob@acme.dev', 'usr_bob', 0), ('cy@acme.dev', 'usr_cy', 0), ('dee@acme.dev', 'usr_dee', 0), \
             ('eve@acme.dev', 'usr_eve', 0)",
            "INSERT INTO allowlist (id, pattern, role, note, created_by, created_at) VALUES \
             ('alw_bob', 'bob@acme.dev', 'member', 'Admin token', 'usr_ada', 0), \
             ('alw_eve', 'eve@acme.dev', 'member', 'Admin token', 'usr_ada', 0)",
            "INSERT INTO passkeys (id, user_id, credential_id, public_key, algorithm, name, created_at) VALUES \
             ('psk_cy', 'usr_cy', 'cred-cy', 'key', -7, 'Laptop', 0)",
            "INSERT INTO identities (id, user_id, provider, subject, email, created_at) VALUES \
             ('idn_dee', 'usr_dee', 'google', 'g-dee-personal', 'dee@gmail.com', 0)",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    /// What Google said of the account with `sub` and `email`, a Google
    /// Workspace account's.
    fn google(sub: &str, email: &str) -> Provided {
        Provided {
            provider: rules::GOOGLE,
            subject: sub.into(),
            email: Some(email.into()),
            verified: true,
            vouched: Some(email.into()),
            name: Some("Someone".into()),
            given_name: None,
            family_name: None,
            picture: None,
        }
    }

    async fn user(db: &D1Database, id: &str) -> AppResult<UserRow> {
        q(
            db,
            &format!("SELECT {} FROM users u WHERE u.id = ?", user_columns("u")),
            &[s(id)],
        )?
        .first()
        .await?
        .ok_or_else(|| not_found("User"))
    }

    /// The Google accounts that sign in to `user_id`.
    async fn googles(db: &D1Database, user_id: &str) -> AppResult<Vec<String>> {
        #[derive(Deserialize)]
        struct Subject {
            subject: String,
        }
        let rows: Vec<Subject> = q(
            db,
            "SELECT subject FROM identities WHERE user_id = ? AND provider = 'google' ORDER BY subject",
            &[s(user_id)],
        )?
        .all()
        .await?;
        Ok(rows.into_iter().map(|r| r.subject).collect())
    }

    fn refused(code: &str) -> First {
        First::Refused(code.to_owned())
    }

    #[wasm_bindgen_test]
    async fn a_provisioned_account_takes_its_first_google_sign_in() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let bob = user(&db, "usr_bob").await?;
        let g = google("g-bob", "bob@acme.dev");
        assert_eq!(
            first(&db, &bob, &g, "bob@acme.dev", NOW, false).await?,
            First::Linked
        );
        assert_eq!(googles(&db, "usr_bob").await?, vec!["g-bob".to_owned()]);
        // Then it is his way in: the same Google account signs in, and
        // another with his address links to it only from his Settings.
        assert_eq!(
            first(&db, &bob, &g, "bob@acme.dev", NOW + 1, false).await?,
            First::Signs
        );
        let other = google("g-bob-2", "bob@acme.dev");
        assert_eq!(
            first(
                &db,
                &user(&db, "usr_bob").await?,
                &other,
                "bob@acme.dev",
                NOW + 2,
                false
            )
            .await?,
            refused("google_link_other")
        );
        assert_eq!(googles(&db, "usr_bob").await?, vec!["g-bob".to_owned()]);
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn an_account_someone_signs_in_to_keeps_to_its_way_in() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        for (id, email, code) in [
            ("usr_ada", "ada@acme.dev", "google_link_required"),
            ("usr_cy", "cy@acme.dev", "google_link_passkey"),
            ("usr_dee", "dee@acme.dev", "google_link_other"),
            // A disabled account takes none, though nobody signs in to it.
            ("usr_eve", "eve@acme.dev", "disabled"),
        ] {
            let g = google(&format!("g-{id}"), email);
            let found = first(&db, &user(&db, id).await?, &g, email, NOW, false).await?;
            assert_eq!(found, refused(code), "{id}");
        }
        assert!(googles(&db, "usr_ada").await?.is_empty());
        assert!(googles(&db, "usr_cy").await?.is_empty());
        assert_eq!(
            googles(&db, "usr_dee").await?,
            vec!["g-dee-personal".to_owned()]
        );
        assert!(googles(&db, "usr_eve").await?.is_empty());
        // Nor does one that had no way in a moment ago, and has one now.
        let bob = user(&db, "usr_bob").await?;
        q(
            &db,
            "UPDATE users SET password_hash = 'hash' WHERE id = 'usr_bob'",
            &[],
        )?
        .run()
        .await?;
        let g = google("g-bob", "bob@acme.dev");
        assert!(matches!(
            first(&db, &bob, &g, "bob@acme.dev", NOW, false).await?,
            First::Refused(_)
        ));
        assert!(googles(&db, "usr_bob").await?.is_empty());
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn where_a_rule_makes_google_the_way_in_it_links_whatever_the_account_has()
    -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Ada's password, Cy's passkey and Dee's personal Google account
        // don't stop it.
        for (id, email) in [
            ("usr_ada", "ada@acme.dev"),
            ("usr_cy", "cy@acme.dev"),
            ("usr_dee", "dee@acme.dev"),
        ] {
            let g = google(&format!("g-{id}"), email);
            let found = first(&db, &user(&db, id).await?, &g, email, NOW, true).await?;
            assert_eq!(found, First::Linked, "{id}");
        }
        assert_eq!(
            googles(&db, "usr_dee").await?,
            vec!["g-dee-personal".to_owned(), "g-usr_dee".to_owned()]
        );
        // A disabled account still takes none; one that is another's stays
        // theirs.
        let eve = user(&db, "usr_eve").await?;
        let g = google("g-eve", "eve@acme.dev");
        assert_eq!(
            first(&db, &eve, &g, "eve@acme.dev", NOW, true).await?,
            refused("disabled")
        );
        let bob = user(&db, "usr_bob").await?;
        let ada_google = google("g-usr_ada", "bob@acme.dev");
        assert_eq!(
            first(&db, &bob, &ada_google, "bob@acme.dev", NOW, true).await?,
            refused("google_linked_elsewhere")
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn the_link_holds_to_the_address_google_vouched_for() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // An admin token gives Bob a new address after the sign-in found
        // his account by the old one: the old one's Google account gets
        // nothing, by a rule or not.
        let bob = user(&db, "usr_bob").await?;
        for sql in [
            "UPDATE users SET email = 'bob@new.acme.dev' WHERE id = 'usr_bob'",
            "UPDATE user_emails SET email = 'bob@new.acme.dev' WHERE email = 'bob@acme.dev'",
        ] {
            q(&db, sql, &[])?.run().await?;
        }
        let old = google("g-bob-old", "bob@acme.dev");
        for by_rule in [false, true] {
            assert!(matches!(
                first(&db, &bob, &old, "bob@acme.dev", NOW, by_rule).await?,
                First::Refused(_)
            ));
        }
        assert!(googles(&db, "usr_bob").await?.is_empty());
        // Any address of his signs in, the ones connected to his account too.
        q(
            &db,
            "INSERT INTO user_emails (email, user_id, created_at) VALUES ('bob@acme.io', 'usr_bob', 0)",
            &[],
        )?
        .run()
        .await?;
        let connected = google("g-bob-io", "bob@acme.io");
        assert_eq!(
            first(
                &db,
                &user(&db, "usr_bob").await?,
                &connected,
                "bob@acme.io",
                NOW,
                false
            )
            .await?,
            First::Linked
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn two_first_sign_ins_at_once_link_one_google_account() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let bob = user(&db, "usr_bob").await?;
        // Two Google accounts with his address: one links, the other is
        // turned away.
        let (one, two) = (
            google("g-bob", "bob@acme.dev"),
            google("g-bob-2", "bob@acme.dev"),
        );
        let (a, b) = futures::join!(
            first(&db, &bob, &one, "bob@acme.dev", NOW, false),
            first(&db, &bob, &two, "bob@acme.dev", NOW, false),
        );
        let mut both = [a?, b?];
        both.sort_by_key(|f| format!("{f:?}"));
        assert_eq!(both, [First::Linked, refused("google_link_other")]);
        let linked: Option<Count> = q(
            &db,
            "SELECT COUNT(*) AS n FROM identities WHERE user_id = 'usr_bob'",
            &[],
        )?
        .first()
        .await?;
        assert_eq!(linked.map(|c| c.n), Some(1));
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn the_same_google_account_twice_at_once_signs_in_both_times() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let bob = user(&db, "usr_bob").await?;
        let g = google("g-bob", "bob@acme.dev");
        let (a, b) = futures::join!(
            first(&db, &bob, &g, "bob@acme.dev", NOW, false),
            first(&db, &bob, &g, "bob@acme.dev", NOW, false),
        );
        let mut both = [a?, b?];
        both.sort_by_key(|f| format!("{f:?}"));
        assert_eq!(both, [First::Linked, First::Signs]);
        assert_eq!(googles(&db, "usr_bob").await?, vec!["g-bob".to_owned()]);
        Ok(())
    }
}

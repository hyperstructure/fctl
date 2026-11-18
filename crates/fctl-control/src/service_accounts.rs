//! Service accounts in D1 (docs/architecture.md#service-accounts, ADR
//! 0086): finding one by its secret, whom a request acts for
//! (`Fctl-On-Behalf-Of`), and how each looks to its people and admins.
//! The rules are `fctl_domain::service_accounts`'.

use crate::ctx::{Ctx, ServiceCaller};
use crate::db::{int, int_bool, n, opt_int, q, s};
use crate::error::{AppError, AppResult, not_found};
use crate::model::{UserRow, user_columns, workspace_ref};
use fctl_domain::limits::{Limits, Places};
use fctl_domain::service_accounts::{self as policy, Named, Person, WhyNot};
use fctl_proto::api::{InstanceSettings, TokenAccess};
use fctl_proto::service_accounts::{
    ON_BEHALF_OF, ServiceAccount, ServiceAccountAnswer, ServiceAccountPerson, ServiceAccountRef,
    ServiceAccountTally,
};
use fctl_proto::session::{UserRef, WorkspaceRole};
use fctl_proto::{ApiError, ServiceAccountId, UserId};
use serde::Deserialize;
use worker::{D1Database, Request};

/// SELECT for service accounts with their workspace, keeper, maker, how
/// their people answered, and the caller's own answer. Bind: `?1` = the
/// caller's user id (`''` for none).
pub const SELECT: &str = "SELECT sa.id, sa.workspace_id, sa.name, sa.description, sa.hint, \
    sa.keeper_id, ku.name AS keeper_name, ku.avatar_url AS keeper_avatar, sa.secret_made_at, \
    sa.created_by, cu.name AS creator_name, cu.avatar_url AS creator_avatar, \
    sa.created_at, sa.last_used_at, \
    EXISTS (SELECT 1 FROM workspace_members km WHERE km.workspace_id = sa.workspace_id \
      AND km.user_id = sa.keeper_id AND km.role IN ('owner', 'admin') AND ku.status = 'active') AS keeper_admin, \
    ku.password_hash IS NOT NULL AS keeper_password, \
    EXISTS (SELECT 1 FROM authenticator_apps kaa WHERE kaa.user_id = sa.keeper_id) AS keeper_authenticator, \
    (SELECT COUNT(*) FROM passkeys kpk WHERE kpk.user_id = sa.keeper_id) AS keeper_passkeys, \
    w.slug AS workspace_slug, w.name AS workspace_name, w.color AS workspace_color, \
    w.avatar_updated_at AS workspace_avatar_at, w.require_two_factor AS workspace_two_factor, \
    (SELECT COUNT(*) FROM workspace_members x WHERE x.workspace_id = sa.workspace_id) AS members, \
    (SELECT COUNT(*) FROM service_account_answers a JOIN workspace_members am \
      ON am.workspace_id = sa.workspace_id AND am.user_id = a.user_id \
      WHERE a.service_account_id = sa.id AND a.allowed = 1) AS allowed, \
    (SELECT COUNT(*) FROM service_account_answers a JOIN workspace_members am \
      ON am.workspace_id = sa.workspace_id AND am.user_id = a.user_id \
      WHERE a.service_account_id = sa.id AND a.allowed = 0) AS declined, \
    (SELECT COUNT(*) FROM sessions ss WHERE ss.service_account_id = sa.id) AS sessions, \
    me.allowed AS my_answer, me.answered_at AS my_answered_at \
    FROM service_accounts sa JOIN workspaces w ON w.id = sa.workspace_id \
    LEFT JOIN users ku ON ku.id = sa.keeper_id \
    LEFT JOIN users cu ON cu.id = sa.created_by \
    LEFT JOIN service_account_answers me ON me.service_account_id = sa.id AND me.user_id = ?1";

#[derive(Clone, Debug, Deserialize)]
pub struct Row {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub description: Option<String>,
    pub hint: String,
    pub keeper_id: Option<String>,
    pub keeper_name: Option<String>,
    pub keeper_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub secret_made_at: i64,
    pub created_by: Option<String>,
    pub creator_name: Option<String>,
    pub creator_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub last_used_at: Option<i64>,
    /// Its keeper is an active admin of its workspace, without which it
    /// works for nobody.
    #[serde(deserialize_with = "int_bool", default)]
    pub keeper_admin: bool,
    /// Its keeper's sign-in methods, which say whether they have to set up
    /// two-factor sign-in first ([`Row::keeper_waits`]).
    #[serde(deserialize_with = "int_bool", default)]
    pub keeper_password: bool,
    #[serde(deserialize_with = "int_bool", default)]
    pub keeper_authenticator: bool,
    #[serde(deserialize_with = "int", default)]
    pub keeper_passkeys: i64,
    pub workspace_slug: String,
    pub workspace_name: String,
    pub workspace_color: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub workspace_avatar_at: Option<i64>,
    /// Its workspace requires two-factor sign-in.
    #[serde(deserialize_with = "int_bool", default)]
    pub workspace_two_factor: bool,
    #[serde(deserialize_with = "int", default)]
    pub members: i64,
    #[serde(deserialize_with = "int", default)]
    pub allowed: i64,
    #[serde(deserialize_with = "int", default)]
    pub declined: i64,
    #[serde(deserialize_with = "int", default)]
    pub sessions: i64,
    #[serde(deserialize_with = "opt_int", default)]
    pub my_answer: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    pub my_answered_at: Option<i64>,
}

fn user_ref(
    id: &Option<String>,
    name: &Option<String>,
    avatar: &Option<String>,
) -> Option<UserRef> {
    match (id, name) {
        (Some(id), Some(name)) => Some(UserRef {
            id: UserId(id.clone()),
            name: name.clone(),
            avatar_url: avatar.clone(),
        }),
        _ => None,
    }
}

fn count(n: i64) -> u32 {
    u32::try_from(n.max(0)).unwrap_or(u32::MAX)
}

/// An answer as D1 keeps it (`allowed`).
pub fn answer(allowed: Option<i64>) -> Option<ServiceAccountAnswer> {
    allowed.map(|a| {
        if a == 1 {
            ServiceAccountAnswer::Allowed
        } else {
            ServiceAccountAnswer::Declined
        }
    })
}

impl Row {
    /// Why it works for nobody now, when it doesn't: its keeper is no
    /// longer an active admin of its workspace, or, under the instance's
    /// `settings`, has to set up two-factor sign-in first.
    pub fn stopped(&self, settings: &InstanceSettings) -> Option<String> {
        policy::stopped(
            self.keeper_admin,
            self.keeper_name.as_deref(),
            &self.workspace_name,
        )
        .or_else(|| {
            self.keeper_waits(settings).then(|| {
                policy::keeper_needs_two_factor(self.keeper_name.as_deref().unwrap_or("Its keeper"))
            })
        })
    }

    /// Whether its keeper has to set up two-factor sign-in, which the
    /// instance or its workspace requires, before they do anything else:
    /// it acts with their authority, and waits as their own tokens do.
    pub fn keeper_waits(&self, settings: &InstanceSettings) -> bool {
        let factors = fctl_domain::two_factor::Factors {
            authenticator: self.keeper_authenticator,
            passkeys: self.keeper_passkeys > 0,
        };
        fctl_domain::two_factor::setup_required(settings, self.keeper_password, factors)
            || (self.workspace_two_factor
                && fctl_domain::two_factor::workspace_blocked(
                    settings,
                    self.keeper_password,
                    factors,
                ))
    }

    pub fn workspace(&self) -> fctl_proto::session::WorkspaceRef {
        workspace_ref(
            &self.workspace_id,
            &self.workspace_slug,
            &self.workspace_name,
            self.workspace_color.as_deref(),
            self.workspace_avatar_at,
        )
    }

    /// As the caller sees it, under the instance's `settings`: every member
    /// its name, what it is and their own answer; its admins (`manages`)
    /// the rest.
    pub fn to_api(&self, settings: &InstanceSettings, manages: bool) -> ServiceAccount {
        let consent = consent(settings);
        let answer = answer(self.my_answer);
        let tally = ServiceAccountTally {
            allowed: count(self.allowed),
            declined: count(self.declined),
            unanswered: count(self.members - self.allowed - self.declined),
        };
        ServiceAccount {
            id: ServiceAccountId(self.id.clone()),
            workspace: self.workspace(),
            name: self.name.clone(),
            description: self.description.clone(),
            created_by: user_ref(&self.created_by, &self.creator_name, &self.creator_avatar),
            created_at: self.created_at,
            answer,
            answered_at: self.my_answered_at.filter(|_| answer.is_some()),
            acts_for_you: !consent || answer == Some(ServiceAccountAnswer::Allowed),
            consent,
            keeper: manages
                .then(|| user_ref(&self.keeper_id, &self.keeper_name, &self.keeper_avatar))
                .flatten(),
            stopped: manages.then(|| self.stopped(settings)).flatten(),
            secret_hint: manages.then(|| self.hint.clone()),
            secret_made_at: manages.then_some(self.secret_made_at),
            last_used_at: self.last_used_at.filter(|_| manages),
            people: manages.then_some(tally),
            sessions: manages.then(|| count(self.sessions)),
            can_manage: manages,
        }
    }
}

/// A service account by id, as `caller` (`''`: nobody) sees it.
pub async fn row(db: &D1Database, caller: &str, id: &str) -> AppResult<Option<Row>> {
    q(
        db,
        &format!("{SELECT} WHERE sa.id = ?2"),
        &[s(caller), s(id)],
    )?
    .first()
    .await
}

/// A workspace's service accounts, oldest first, as `caller` sees them.
pub async fn of_workspace(
    db: &D1Database,
    caller: &str,
    workspace_id: &str,
) -> AppResult<Vec<Row>> {
    q(
        db,
        &format!("{SELECT} WHERE sa.workspace_id = ?2 ORDER BY sa.created_at, sa.id"),
        &[s(caller), s(workspace_id)],
    )?
    .all()
    .await
}

/// How many service accounts a workspace has.
pub async fn count_of(db: &D1Database, workspace_id: &str) -> AppResult<u32> {
    let c: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM service_accounts WHERE workspace_id = ?",
        &[s(workspace_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| count(c.n)))
}

/// Whether the instance asks people before service accounts act for them.
pub fn consent(settings: &InstanceSettings) -> bool {
    settings.service_account_consent
}

// ───────────────────────────── authenticating ─────────────────────────────

/// Resolves a service account's secret (`fctl_svc_…`): while its keeper is
/// an active admin of its workspace, it reads its workspace as them, held
/// to reading there, until [`gate`] names the person a request acts for.
pub async fn authenticate(ctx: &mut Ctx, secret: &str) -> AppResult<()> {
    let found = q(
        &ctx.db,
        &format!("{SELECT} WHERE sa.hash = ?2"),
        &[s(""), s(&fctl_domain::tokens::hash_secret(secret))],
    )?
    .first::<Row>()
    .await?;
    let Some(found) = found else {
        ctx.rate_limit(&format!("svc:{}", ctx.ip_key())).await?;
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::Unauthorized,
            "Unknown service account: its workspace's admins may have deleted it, or made it a new secret. Ask one of them for the one it has now.",
        )));
    };
    let settings = crate::instance::settings(&ctx.db).await?;
    // Its keeper left or lost the role, or, as it acts with their
    // authority, has to set up two-factor sign-in first, as their own
    // tokens wait too.
    if let Some(why) = found.stopped(&settings) {
        return Err(AppError(ApiError::new(
            fctl_proto::ErrorCode::AccessDenied,
            format!("This service account works for nobody now: {why}"),
        )));
    }
    let keeper = match found.keeper_id.as_deref() {
        Some(id) => crate::auth::user_by_id(ctx, id).await?,
        None => None,
    };
    let Some(keeper) = keeper else {
        return Err(AppError(ApiError::forbidden(
            "This service account works for nobody now: an admin of its workspace makes it a new secret to start it again.",
        )));
    };
    if found
        .last_used_at
        .is_none_or(|t| ctx.now - t > 10 * 60 * 1000)
    {
        let db = crate::db::handle(&ctx.db);
        let (id, now) = (found.id.clone(), ctx.now);
        ctx.defer(async move {
            q(
                &db,
                "UPDATE service_accounts SET last_used_at = ? WHERE id = ?",
                &[n(now), s(&id)],
            )?
            .run()
            .await?;
            Ok(())
        });
    }
    ctx.auth = Some(crate::ctx::Authed {
        user: keeper,
        token_id: found.id.clone(),
        // Not one of `TokenKind`'s, which clients read: `service_account`
        // says what it is; to the API it's a tool's token.
        kind: fctl_proto::api::TokenKind::Pat,
        via_cookie: false,
        signed_in_at: found.secret_made_at,
        verified_at: None,
        two_factor_setup_required: false,
        settings,
        limits: Some(held(&found.workspace_id, TokenAccess::Read)),
        autopilot: None,
        admin_token: false,
        service_account: Some(ServiceCaller {
            id: found.id.clone(),
            name: found.name.clone(),
            workspace_id: found.workspace_id.clone(),
            workspace_name: found.workspace_name.clone(),
            acting_for: None,
        }),
    });
    Ok(())
}

/// What a service account does in its workspace: `access` there, nowhere
/// else.
fn held(workspace_id: &str, access: TokenAccess) -> Limits {
    Limits {
        access,
        places: Some(Places {
            workspaces: vec![workspace_id.to_owned()],
            projects: Vec::new(),
        }),
        github_access: None,
        most: None,
    }
}

/// Whom a service account's request acts for, before it goes on: the
/// person `Fctl-On-Behalf-Of` names, or, running a schedule without one,
/// the person the schedule runs for. It then acts as them, held to what an
/// access token that runs sessions does in its workspace, while they may
/// be acted for (`fctl_domain::service_accounts::may_act_for`). Without
/// anyone, it reads its workspace (`allows`).
pub async fn gate(ctx: &mut Ctx, req: &Request, method: &str, segs: &[&str]) -> AppResult<()> {
    let Some(caller) = ctx.auth.as_ref().and_then(|a| a.service_account.clone()) else {
        return Ok(());
    };
    let header = crate::http::header(req, ON_BEHALF_OF);
    let named = match header.as_deref().map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => Some(policy::named(h).map_err(AppError)?),
        None => match policy::runs_schedule(method, segs) {
            Some(schedule) => Some(Named::Id(
                schedule_person(&ctx.db, &caller, &crate::http::decode(schedule)).await?,
            )),
            None => None,
        },
    };
    policy::allows(named.is_some(), method, segs).map_err(AppError)?;
    let Some(named) = named else {
        return Ok(());
    };
    let person = act_for(ctx, &caller, &named).await?;
    if let Some(a) = ctx.auth.as_mut() {
        a.user = person;
        a.limits = Some(held(&caller.workspace_id, TokenAccess::Run));
        if let Some(sa) = a.service_account.as_mut() {
            sa.acting_for = Some(a.user.id.clone());
        }
    }
    Ok(())
}

/// The person schedule `id` of the service account's workspace runs for.
async fn schedule_person(db: &D1Database, caller: &ServiceCaller, id: &str) -> AppResult<String> {
    #[derive(Deserialize)]
    struct Found {
        owner_id: Option<String>,
    }
    let found: Option<Found> = q(
        db,
        "SELECT sc.owner_id FROM schedules sc JOIN projects p ON p.id = sc.project_id \
         WHERE sc.id = ? AND p.workspace_id = ?",
        &[s(id), s(&caller.workspace_id)],
    )?
    .first()
    .await?;
    let found = found.ok_or_else(|| not_found("Schedule"))?;
    found.owner_id.ok_or_else(|| {
        AppError(ApiError::validation(
            policy::FIELD,
            "This schedule runs for nobody now: name the person to run it for in the Fctl-On-Behalf-Of header (fctl --on-behalf-of)",
        ))
    })
}

/// The person `named` is, while the service account may act for them, or
/// why it may not.
async fn act_for(ctx: &Ctx, caller: &ServiceCaller, named: &Named) -> AppResult<UserRow> {
    let person = match named {
        Named::Id(id) => crate::auth::user_by_id(ctx, id).await?,
        Named::Email(email) => crate::auth::user_by_email(ctx, email).await?,
    };
    let workspace = &caller.workspace_name;
    let refuse = |why: WhyNot, who: &str| AppError(why.error(who, &caller.name, workspace));
    let Some(person) = person else {
        return Err(refuse(WhyNot::NotMember, ""));
    };
    #[derive(Deserialize)]
    struct Found {
        #[serde(deserialize_with = "int_bool")]
        member: bool,
        #[serde(deserialize_with = "int_bool", default)]
        require_two_factor: bool,
        #[serde(deserialize_with = "opt_int", default)]
        allowed: Option<i64>,
    }
    let found: Option<Found> = q(
        &ctx.db,
        "SELECT EXISTS (SELECT 1 FROM workspace_members m WHERE m.workspace_id = w.id AND m.user_id = ?2) AS member, \
         w.require_two_factor, \
         (SELECT a.allowed FROM service_account_answers a WHERE a.service_account_id = ?3 AND a.user_id = ?2) AS allowed \
         FROM workspaces w WHERE w.id = ?1",
        &[s(&caller.workspace_id), s(&person.id), s(&caller.id)],
    )?
    .first()
    .await?;
    let found = found.ok_or_else(|| not_found("Workspace"))?;
    let settings = ctx.settings();
    let needs_two_factor =
        fctl_domain::two_factor::setup_required(&settings, person.has_password(), person.factors())
            || person.blocked_by(&settings, found.require_two_factor);
    let state = Person {
        member: found.member,
        active: person.is_active(),
        needs_two_factor,
        answer: answer(found.allowed),
    };
    policy::may_act_for(consent(&settings), state).map_err(|why| refuse(why, &person.name))?;
    Ok(person)
}

// ───────────────────────────── people ─────────────────────────────

/// A member of a service account's workspace, with their answer to it.
#[derive(Deserialize)]
struct PersonRow {
    #[serde(flatten)]
    user: UserRow,
    member_email: String,
    member_role: String,
    #[serde(deserialize_with = "opt_int", default)]
    allowed: Option<i64>,
    #[serde(deserialize_with = "opt_int", default)]
    answered_at: Option<i64>,
}

/// The people of service account `sa`'s workspace (`require_two_factor`:
/// whether it requires two-factor sign-in), by name, with their answers
/// and whether it may act for them now.
pub async fn people(
    ctx: &Ctx,
    sa: &Row,
    require_two_factor: bool,
) -> AppResult<Vec<ServiceAccountPerson>> {
    let rows: Vec<PersonRow> = q(
        &ctx.db,
        &format!(
            "SELECT {}, m.email AS member_email, m.role AS member_role, a.allowed, a.answered_at \
             FROM workspace_members m JOIN users u ON u.id = m.user_id \
             LEFT JOIN service_account_answers a ON a.service_account_id = ?2 AND a.user_id = m.user_id \
             WHERE m.workspace_id = ?1 ORDER BY u.name COLLATE NOCASE, u.id",
            user_columns("u")
        ),
        &[s(&sa.workspace_id), s(&sa.id)],
    )?
    .all()
    .await?;
    let settings = ctx.settings();
    let consent = consent(&settings);
    Ok(rows
        .into_iter()
        .map(|r| {
            let answer = answer(r.allowed);
            let needs_two_factor = fctl_domain::two_factor::setup_required(
                &settings,
                r.user.has_password(),
                r.user.factors(),
            ) || r.user.blocked_by(&settings, require_two_factor);
            let why = policy::may_act_for(
                consent,
                Person {
                    member: true,
                    active: r.user.is_active(),
                    needs_two_factor,
                    answer,
                },
            )
            .err();
            ServiceAccountPerson {
                user: r.user.to_ref(),
                email: r.member_email,
                role: WorkspaceRole::parse(&r.member_role).unwrap_or(WorkspaceRole::Member),
                answer,
                answered_at: r.answered_at.filter(|_| answer.is_some()),
                acts_for: why.is_none(),
                blocked: matches!(why, Some(WhyNot::Disabled | WhyNot::TwoFactor)),
                why_not: why.map(|w| w.reason(&r.user.name, &sa.name, &sa.workspace_name)),
            }
        })
        .collect())
}

/// Forgets the answers of `user_id` to the service accounts of
/// `workspace_id`, as they leave it: joining again asks again.
pub fn forget_answers(
    db: &D1Database,
    workspace_id: &str,
    user_id: &str,
) -> AppResult<crate::db::Query> {
    q(
        db,
        "DELETE FROM service_account_answers WHERE user_id = ?2 AND service_account_id IN \
         (SELECT id FROM service_accounts WHERE workspace_id = ?1)",
        &[s(workspace_id), s(user_id)],
    )
}

// ───────────────────────────── workspaces' lists ─────────────────────────────

/// A workspace's service accounts as its member sees them, from the JSON
/// [`crate::workspaces::WORKSPACE_SELECT`] reads (`service_accounts`).
#[derive(Deserialize)]
struct Brief {
    id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    creator_name: Option<String>,
    #[serde(default)]
    creator_avatar: Option<String>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    allowed: Option<i64>,
    #[serde(default)]
    answered_at: Option<i64>,
}

/// The JSON of a workspace's service accounts with `?1`'s answers, for
/// SELECTs of workspaces (`w`): oldest first.
#[macro_export]
macro_rules! service_account_briefs {
    () => {
        "(SELECT json_group_array(json(j)) FROM (SELECT json_object('id', sa.id, 'name', sa.name, \
         'description', sa.description, 'created_by', sa.created_by, 'creator_name', bu.name, \
         'creator_avatar', bu.avatar_url, 'created_at', sa.created_at, 'allowed', ba.allowed, \
         'answered_at', ba.answered_at) AS j \
         FROM service_accounts sa LEFT JOIN users bu ON bu.id = sa.created_by \
         LEFT JOIN service_account_answers ba ON ba.service_account_id = sa.id AND ba.user_id = ?1 \
         WHERE sa.workspace_id = w.id ORDER BY sa.created_at, sa.id))"
    };
}

/// A workspace's service accounts as a member (`admin`: one of its admins)
/// sees them in its lists, from [`service_account_briefs`].
pub fn briefs(
    json: Option<&str>,
    workspace: &fctl_proto::session::WorkspaceRef,
    consent: bool,
    admin: bool,
) -> Vec<ServiceAccount> {
    let list: Vec<Brief> = json
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    list.into_iter()
        .map(|b| {
            let answer = answer(b.allowed);
            ServiceAccount {
                id: ServiceAccountId(b.id),
                workspace: workspace.clone(),
                name: b.name,
                description: b.description,
                created_by: user_ref(&b.created_by, &b.creator_name, &b.creator_avatar),
                created_at: b.created_at,
                answer,
                answered_at: b.answered_at.filter(|_| answer.is_some()),
                acts_for_you: !consent || answer == Some(ServiceAccountAnswer::Allowed),
                consent,
                keeper: None,
                stopped: None,
                secret_hint: None,
                secret_made_at: None,
                last_used_at: None,
                people: None,
                sessions: None,
                can_manage: admin,
            }
        })
        .collect()
}

/// The JSON of a workspace's service accounts in brief, for SELECTs of
/// invitations (`i`).
#[macro_export]
macro_rules! service_account_refs {
    () => {
        "(SELECT json_group_array(json(j)) FROM (SELECT json_object('id', sa.id, \
         'name', sa.name, 'description', sa.description) AS j FROM service_accounts sa \
         WHERE sa.workspace_id = i.workspace_id ORDER BY sa.created_at, sa.id))"
    };
}

/// The service accounts of an invitation's workspace, from
/// [`service_account_refs`].
pub fn refs(json: Option<&str>) -> Vec<ServiceAccountRef> {
    json.and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn run(db: &D1Database, sql: &str) -> AppResult<()> {
        q(db, sql, &[])?.run().await?;
        Ok(())
    }

    /// Acme, whose admin Ada made the portal's secret; Bob allowed it,
    /// Cy declined, Dee hasn't answered, and Eve left after allowing it.
    async fn acme(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@acme.dev', 'Ada', 'admin', 0, 0), ('usr_bob', 'bob@acme.dev', 'Bob', 'member', 0, 0), \
             ('usr_cy', 'cy@acme.dev', 'Cy', 'member', 0, 0), ('usr_dee', 'dee@acme.dev', 'Dee', 'member', 0, 0), \
             ('usr_eve', 'eve@acme.dev', 'Eve', 'member', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES ('wsp_acme', 'acme', 'Acme', 0, 0)",
            "INSERT INTO workspace_members (workspace_id, user_id, email, role, created_at) VALUES \
             ('wsp_acme', 'usr_ada', 'ada@acme.dev', 'owner', 0), ('wsp_acme', 'usr_bob', 'bob@acme.dev', 'member', 0), \
             ('wsp_acme', 'usr_cy', 'cy@acme.dev', 'member', 0), ('wsp_acme', 'usr_dee', 'dee@acme.dev', 'member', 0)",
            "INSERT INTO service_accounts (id, workspace_id, name, description, hash, hint, keeper_id, secret_made_at, \
             created_by, created_at, updated_at) VALUES ('sva_portal', 'wsp_acme', 'Acme portal', 'Our portal', 'h1', 'x8Kq', \
             'usr_ada', 5, 'usr_ada', 5, 5)",
            "INSERT INTO service_account_answers (service_account_id, user_id, allowed, answered_at) VALUES \
             ('sva_portal', 'usr_bob', 1, 7), ('sva_portal', 'usr_cy', 0, 8), ('sva_portal', 'usr_eve', 1, 9)",
        ] {
            run(db, sql).await?;
        }
        Ok(())
    }

    /// Who answered what counts its members alone, and an admin sees it.
    #[wasm_bindgen_test]
    async fn its_admins_see_how_its_people_answered() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        let found = row(&db, "usr_bob", "sva_portal")
            .await?
            .ok_or_else(|| crate::error::internal("no service account"))?;
        assert!(found.keeper_admin);
        let settings = InstanceSettings::default();
        assert_eq!(found.stopped(&settings), None);
        let admin = found.to_api(&settings, true);
        assert_eq!(
            admin.people,
            Some(ServiceAccountTally {
                allowed: 1,
                declined: 1,
                unanswered: 2,
            })
        );
        assert_eq!(admin.secret_hint.as_deref(), Some("x8Kq"));
        // Bob's own view: his answer, nothing of the others'.
        let bob = found.to_api(&settings, false);
        assert_eq!(bob.answer, Some(ServiceAccountAnswer::Allowed));
        assert!(bob.acts_for_you);
        assert_eq!(bob.people, None);
        assert_eq!(bob.secret_hint, None);
        assert_eq!(bob.keeper, None);
        Ok(())
    }

    /// Demoted, its keeper leaves it working for nobody; leaving forgets
    /// a member's answers.
    #[wasm_bindgen_test]
    async fn it_stops_with_its_keeper_and_leavers_are_asked_again() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        run(
            &db,
            "UPDATE workspace_members SET role = 'member' WHERE user_id = 'usr_ada'",
        )
        .await?;
        let found = row(&db, "", "sva_portal")
            .await?
            .ok_or_else(|| crate::error::internal("no service account"))?;
        assert!(!found.keeper_admin);
        assert!(
            found
                .stopped(&InstanceSettings::default())
                .is_some_and(|w| w.starts_with("Ada, who made its secret"))
        );
        forget_answers(&db, "wsp_acme", "usr_bob")?.run().await?;
        let found = row(&db, "usr_bob", "sva_portal")
            .await?
            .ok_or_else(|| crate::error::internal("no service account"))?;
        assert_eq!(found.my_answer, None);
        Ok(())
    }

    /// It waits while its keeper has to set up two-factor sign-in, which
    /// the instance or its workspace requires, and its admins see why.
    #[wasm_bindgen_test]
    async fn it_waits_while_its_keeper_has_to_set_up_two_factor() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        run(
            &db,
            "UPDATE users SET password_hash = 'phc' WHERE id = 'usr_ada'",
        )
        .await?;
        let open = InstanceSettings::default();
        let required = InstanceSettings {
            require_two_factor: true,
            ..InstanceSettings::default()
        };
        let waits = policy::keeper_needs_two_factor("Ada");
        let sa = portal(&db).await?;
        // Nothing asks it of Ada yet.
        assert_eq!(sa.stopped(&open), None);
        // The instance requires it: Ada, with a password and nothing else,
        // sets it up first, and the portal waits, as her tokens do.
        assert_eq!(sa.stopped(&required).as_deref(), Some(waits.as_str()));
        assert_eq!(
            sa.to_api(&required, true).stopped.as_deref(),
            Some(waits.as_str())
        );
        assert_eq!(sa.to_api(&required, false).stopped, None);
        // So does the workspace's own requirement.
        run(
            &db,
            "UPDATE workspaces SET require_two_factor = 1 WHERE id = 'wsp_acme'",
        )
        .await?;
        let sa = portal(&db).await?;
        assert_eq!(sa.stopped(&open).as_deref(), Some(waits.as_str()));
        // With an authenticator app, it works again.
        run(
            &db,
            "INSERT INTO authenticator_apps (user_id, secret_enc, created_at) VALUES ('usr_ada', 'sealed', 1)",
        )
        .await?;
        let sa = portal(&db).await?;
        assert_eq!(sa.stopped(&required), None);
        assert_eq!(sa.stopped(&open), None);
        Ok(())
    }

    /// Acme's portal, as nobody in particular reads it.
    async fn portal(db: &D1Database) -> AppResult<Row> {
        row(db, "", "sva_portal")
            .await?
            .ok_or_else(|| crate::error::internal("no service account"))
    }

    /// Members' lists carry each service account with their own answer.
    #[wasm_bindgen_test]
    async fn members_lists_carry_their_own_answers() -> AppResult<()> {
        let db = testing::database();
        acme(&db).await?;
        #[derive(Deserialize)]
        struct J {
            service_accounts: Option<String>,
        }
        let found: Option<J> = q(
            &db,
            concat!(
                "SELECT ",
                crate::service_account_briefs!(),
                " AS service_accounts FROM workspaces w WHERE w.id = 'wsp_acme'"
            ),
            &[s("usr_cy")],
        )?
        .first()
        .await?;
        let workspace = workspace_ref("wsp_acme", "acme", "Acme", None, None);
        let list = briefs(
            found.and_then(|f| f.service_accounts).as_deref(),
            &workspace,
            true,
            false,
        );
        assert_eq!(list.len(), 1);
        let first = list
            .first()
            .ok_or_else(|| crate::error::internal("no service account"))?;
        assert_eq!(first.name, "Acme portal");
        assert_eq!(first.answer, Some(ServiceAccountAnswer::Declined));
        assert!(!first.acts_for_you);
        // Where the instance doesn't ask, it acts for everyone.
        let unasked = briefs(
            Some(r#"[{"id":"sva_portal","name":"Acme portal","created_at":5}]"#),
            &workspace,
            false,
            false,
        );
        assert!(unasked.first().is_some_and(|s| s.acts_for_you));
        Ok(())
    }
}

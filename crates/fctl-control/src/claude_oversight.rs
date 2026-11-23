//! Claude oversight (docs/architecture.md#claude-oversight, #627): what
//! sessions say of the Claude connection they run on (an answer, a refusal,
//! a usage limit), marking a connection Claude refuses ended, what a
//! workspace's admins and an instance's see of people's connections, and
//! telling people and their admins before a token runs out and as one
//! stops working. The pure parts are `fctl_domain::claude_oversight`'s.

use crate::config::Config;
use crate::ctx::Ctx;
use crate::db::{Query, batch, n, q, s};
use crate::error::AppResult;
use crate::model::{CredentialRow, USAGE_JSON};
use crate::workspaces::WorkspaceRow;
use fctl_domain::claude_oversight::{self as domain, Connection, Pause, Told};
use fctl_domain::cron::DAY_MS;
use fctl_domain::events::{BillingGuard, billing_guard};
use fctl_proto::api::{
    ClaudeAdminEmail, ClaudeOversight, ClaudeOverview, ClaudeState, InstanceSettings, MemberClaude,
};
use fctl_proto::session::UserRef;
use fctl_proto::{Millis, UserId};
use serde::Deserialize;
use std::collections::HashMap;
use worker::{D1Database, Env};

/// How often a session records that Claude answered: once a run starts,
/// then at most this often.
pub const ANSWERED_EVERY_MS: i64 = 10 * 60_000;

/// What a workspace's admins are told when they ask for more than the
/// instance lets them see.
pub fn beyond_most(most: ClaudeOversight) -> String {
    let what = match most {
        ClaudeOversight::Off => "nothing of members' Claude connections",
        ClaudeOversight::Readiness => {
            "only whether members' sessions can run, and when their tokens run out"
        }
        ClaudeOversight::Details => "everything fctl shows",
    };
    format!(
        "This instance lets workspace admins see {what}: its admins decide that in Admin → Settings"
    )
}

/// What a session's Claude messages said of the connection it runs on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    /// Claude answered: it takes the connection.
    pub answered: bool,
    /// Claude refused the connection.
    pub refused: bool,
    /// Usage limits it paused on: the limit and when it resets.
    pub pauses: Vec<(Option<String>, Option<Millis>)>,
}

impl Seen {
    /// Notes what one of the session's Claude messages says.
    pub fn note(&mut self, v: &serde_json::Value) {
        if domain::answered(v) {
            self.answered = true;
        }
        if domain::refused(v) {
            self.refused = true;
        }
        if let Some(BillingGuard::LimitReached { resets_at, limit }) = billing_guard(v) {
            self.pauses.push((limit, resets_at));
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.answered && !self.refused && self.pauses.is_empty()
    }
}

/// Keeps what session `session_id`'s Claude said at `now` of the connection
/// it runs on: when Claude last answered it, each usage limit it paused on,
/// and each refusal, and marks it ended once Claude refused it in as many
/// sessions as the instance says since it last answered it.
pub async fn record(env: &Env, session_id: &str, seen: Seen, now: Millis) -> AppResult<()> {
    if seen.is_empty() {
        return Ok(());
    }
    let db = env.d1("DB")?;
    batch(&db, statements(&db, session_id, &seen, now)?).await?;
    if seen.refused && !seen.answered {
        let settings = crate::instance::settings(&db).await?;
        if let Some(id) = refused_enough(&db, session_id, settings.claude_refusals).await? {
            end(&Config::from_env(env), &db, &id, now).await?;
        }
    }
    Ok(())
}

/// The statements that keep `seen` for session `session_id`.
pub fn statements(
    db: &D1Database,
    session_id: &str,
    seen: &Seen,
    now: Millis,
) -> AppResult<Vec<Query>> {
    let mut out = Vec::new();
    if seen.answered {
        out.push(q(
            db,
            "UPDATE claude_credentials SET answered_at = ?1 \
             WHERE id = (SELECT credential_id FROM sessions WHERE id = ?2) AND COALESCE(answered_at, 0) < ?1",
            &[n(now), s(session_id)],
        )?);
    }
    for (limit, resets_at) in &seen.pauses {
        out.push(q(
            db,
            "INSERT OR IGNORE INTO claude_limit_pauses (credential_id, session_id, window, resets_at, at) \
             SELECT credential_id, id, ?, ?, ? FROM sessions WHERE id = ? AND credential_id IS NOT NULL \
             AND EXISTS (SELECT 1 FROM claude_credentials c WHERE c.id = sessions.credential_id)",
            &[
                s(limit.as_deref().unwrap_or_default()),
                n(resets_at.unwrap_or(0)),
                n(now),
                s(session_id),
            ],
        )?);
    }
    if seen.refused && !seen.answered {
        out.push(q(
            db,
            "INSERT INTO claude_refusals (credential_id, session_id, at) \
             SELECT credential_id, id, ? FROM sessions WHERE id = ? AND credential_id IS NOT NULL \
             AND EXISTS (SELECT 1 FROM claude_credentials c WHERE c.id = sessions.credential_id) \
             ON CONFLICT (credential_id, session_id) DO UPDATE SET at = excluded.at",
            &[n(now), s(session_id)],
        )?);
    }
    Ok(out)
}

/// The id of the working connection session `session_id` runs on, when
/// Claude refused it in at least `needed` sessions since it last answered
/// it.
pub async fn refused_enough(
    db: &D1Database,
    session_id: &str,
    needed: u32,
) -> AppResult<Option<String>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        #[serde(deserialize_with = "crate::db::int")]
        refusals: i64,
    }
    let row: Option<Row> = q(
        db,
        "SELECT c.id, (SELECT COUNT(*) FROM claude_refusals r WHERE r.credential_id = c.id \
           AND r.at > COALESCE(c.answered_at, 0)) AS refusals \
         FROM sessions JOIN claude_credentials c ON c.id = sessions.credential_id \
         WHERE sessions.id = ? AND c.status = 'active'",
        &[s(session_id)],
    )?
    .first()
    .await?;
    Ok(row
        .filter(|r| r.refusals >= i64::from(needed.max(1)))
        .map(|r| r.id))
}

/// Marks connection `id` ended at `now`, as Claude stopped taking it, and
/// tells its person, and the admins of the workspaces whose sessions of
/// theirs it ran, who hear of each. Returns whether it worked until now.
pub async fn end(cfg: &Config, db: &D1Database, id: &str, now: Millis) -> AppResult<bool> {
    let changed = q(
        db,
        "UPDATE claude_credentials SET status = 'invalid', ended_at = ?, updated_at = ? \
         WHERE id = ? AND status = 'active'",
        &[n(now), n(now), s(id)],
    )?
    .run()
    .await?;
    if changed == 0 {
        return Ok(false);
    }
    #[derive(Deserialize)]
    struct Owner {
        user_id: String,
    }
    let owner: Option<Owner> = q(
        db,
        "SELECT user_id FROM claude_credentials WHERE id = ?",
        &[s(id)],
    )?
    .first()
    .await?;
    if let Some(o) = &owner {
        let _ = q(
            db,
            "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) \
             VALUES (?, ?, NULL, 'claude.ended', ?, ?, NULL, NULL)",
            &[
                s(&fctl_domain::ids::new_id("aud", now)),
                n(now),
                s(&o.user_id),
                s(&serde_json::json!({ "credential": id }).to_string()),
            ],
        )?
        .run()
        .await;
    }
    if let Err(e) = tell_ended(cfg, db, id, now).await {
        worker::console_error!("telling of a Claude connection that ended: {}", e.0.message);
    }
    Ok(true)
}

/// Someone a workspace's view lists, and what their sessions there run on.
#[derive(Clone, Debug, Deserialize)]
struct PersonRow {
    user_id: String,
    name: String,
    #[serde(default)]
    avatar_url: Option<String>,
    email: String,
    /// The connection they chose for the workspace.
    #[serde(default)]
    chosen: Option<String>,
    #[serde(default)]
    default_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct PauseRow {
    credential_id: String,
    window: String,
    #[serde(deserialize_with = "crate::db::int")]
    resets_at: i64,
    #[serde(deserialize_with = "crate::db::int")]
    at: i64,
}

/// People and their connections, with their limit pauses: of workspace
/// `workspace_id`'s members, or, without one, of every active account.
struct World {
    people: Vec<PersonRow>,
    conns: HashMap<String, Vec<CredentialRow>>,
    pauses: HashMap<String, Vec<Pause>>,
}

impl World {
    async fn load(db: &D1Database, workspace_id: Option<&str>, now: Millis) -> AppResult<Self> {
        let people: Vec<PersonRow> = match workspace_id {
            Some(w) => q(
                db,
                "SELECT m.user_id, u.name, u.avatar_url, m.email, m.credential_id AS chosen, \
                 u.default_credential_id AS default_id \
                 FROM workspace_members m JOIN users u ON u.id = m.user_id WHERE m.workspace_id = ? \
                 ORDER BY CASE m.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, u.name",
                &[s(w)],
            )?,
            None => q(
                db,
                "SELECT u.id AS user_id, u.name, u.avatar_url, u.email, NULL AS chosen, \
                 u.default_credential_id AS default_id FROM users u WHERE u.status = 'active' \
                 ORDER BY u.name",
                &[],
            )?,
        }
        .all()
        .await?;
        let whose = match workspace_id {
            Some(_) => {
                "c.user_id IN (SELECT user_id FROM workspace_members WHERE workspace_id = ?1)"
            }
            None => "c.user_id IN (SELECT id FROM users WHERE status = 'active')",
        };
        let bind: Vec<_> = workspace_id.map(s).into_iter().collect();
        let rows: Vec<CredentialRow> = q(
            db,
            &format!(
                "SELECT c.*, {USAGE_JSON} AS usage FROM claude_credentials c WHERE {whose} \
                 ORDER BY c.created_at DESC"
            ),
            &bind,
        )?
        .all()
        .await?;
        let mut conns: HashMap<String, Vec<CredentialRow>> = HashMap::new();
        for r in rows {
            conns.entry(r.user_id.clone()).or_default().push(r);
        }
        // Only the pauses of sessions in the workspace: what its admins see
        // is what happened there.
        let pause_rows: Vec<PauseRow> = match workspace_id {
            Some(w) => q(
                db,
                "SELECT p.credential_id, p.window, p.resets_at, p.at FROM claude_limit_pauses p \
                 JOIN sessions x ON x.id = p.session_id JOIN projects pr ON pr.id = x.project_id \
                 WHERE pr.workspace_id = ? AND p.at > ?",
                &[s(w), n(now - domain::KEPT_MS)],
            )?,
            None => q(
                db,
                "SELECT credential_id, window, resets_at, at FROM claude_limit_pauses WHERE at > ?",
                &[n(now - domain::KEPT_MS)],
            )?,
        }
        .all()
        .await?;
        let mut pauses: HashMap<String, Vec<Pause>> = HashMap::new();
        for p in pause_rows {
            pauses.entry(p.credential_id).or_default().push(Pause {
                at: p.at,
                limit: (!p.window.is_empty()).then_some(p.window),
                resets_at: (p.resets_at > 0).then_some(p.resets_at),
            });
        }
        Ok(Self {
            people,
            conns,
            pauses,
        })
    }

    /// Each person as admins see them, with `oversight` and `limit_pauses`.
    fn members(
        &self,
        oversight: ClaudeOversight,
        limit_pauses: bool,
        now: Millis,
    ) -> Vec<(MemberClaude, Option<CredentialRow>, &PersonRow)> {
        self.people
            .iter()
            .map(|p| {
                let rows = self.conns.get(&p.user_id).cloned().unwrap_or_default();
                let conns: Vec<Connection> =
                    rows.iter().map(CredentialRow::to_connection).collect();
                let picked = domain::in_use(&conns, p.chosen.as_deref(), p.default_id.as_deref())
                    .map(|(c, _)| c)
                    .or_else(|| domain::ended(&conns, p.chosen.as_deref(), p.default_id.as_deref()))
                    .map(|c| c.id.clone());
                let pauses = picked
                    .as_ref()
                    .and_then(|id| self.pauses.get(id))
                    .cloned()
                    .unwrap_or_default();
                let m = domain::member(
                    UserRef {
                        id: UserId(p.user_id.clone()),
                        name: p.name.clone(),
                        avatar_url: p.avatar_url.clone(),
                    },
                    p.email.clone(),
                    &conns,
                    p.chosen.as_deref(),
                    p.default_id.as_deref(),
                    &pauses,
                    oversight,
                    limit_pauses,
                    now,
                );
                let row = picked.and_then(|id| rows.into_iter().find(|r| r.id == id));
                (m, row, p)
            })
            .collect()
    }
}

/// `GET /api/v1/workspaces/{id}/claude`: its members, as much as its admins
/// see.
pub async fn workspace(
    ctx: &Ctx,
    w: &WorkspaceRow,
    settings: &InstanceSettings,
) -> AppResult<ClaudeOverview> {
    let o = w.claude_oversight(settings);
    let members = if o.oversight == ClaudeOversight::Off {
        Vec::new()
    } else {
        World::load(&ctx.db, Some(&w.id), ctx.now)
            .await?
            .members(o.oversight, o.limit_pauses, ctx.now)
            .into_iter()
            .map(|(m, _, _)| m)
            .collect()
    };
    Ok(ClaudeOverview {
        oversight: o.oversight,
        limit_pauses: o.limit_pauses && o.oversight == ClaudeOversight::Details,
        admin_tokens: ctx.cfg.admin_tokens,
        members,
    })
}

/// `GET /api/v1/admin/claude`: every active account, each with their
/// default connection (else their newest that works), as workspace admins
/// see their members' with everything shown.
pub async fn instance(ctx: &Ctx) -> AppResult<ClaudeOverview> {
    let members = World::load(&ctx.db, None, ctx.now)
        .await?
        .members(ClaudeOversight::Details, true, ctx.now)
        .into_iter()
        .map(|(m, _, _)| m)
        .collect();
    Ok(ClaudeOverview {
        oversight: ClaudeOversight::Details,
        limit_pauses: true,
        admin_tokens: ctx.cfg.admin_tokens,
        members,
    })
}

// ───────────────────────────── telling people ─────────────────────────────

/// A workspace whose admins may hear of a member's connection.
#[derive(Clone, Debug, Deserialize)]
struct WorkspaceBrief {
    id: String,
    slug: String,
    name: String,
    #[serde(default)]
    claude_oversight: Option<String>,
    #[serde(default)]
    claude_admin_email: Option<String>,
}

impl WorkspaceBrief {
    fn oversight(&self, settings: &InstanceSettings) -> ClaudeOversight {
        domain::effective(
            settings.claude_oversight,
            self.claude_oversight
                .as_deref()
                .and_then(ClaudeOversight::parse),
        )
    }

    fn admin_email(&self) -> ClaudeAdminEmail {
        self.claude_admin_email
            .as_deref()
            .and_then(ClaudeAdminEmail::parse)
            .unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct AdminAddress {
    email: String,
}

/// The addresses of workspace `workspace_id`'s admins, as they are members
/// of it, but `but`'s.
async fn admin_addresses(db: &D1Database, workspace_id: &str, but: &str) -> AppResult<Vec<String>> {
    let rows: Vec<AdminAddress> = q(
        db,
        "SELECT m.email FROM workspace_members m JOIN users u ON u.id = m.user_id \
         WHERE m.workspace_id = ? AND m.role IN ('owner', 'admin') AND m.user_id != ? AND u.status = 'active'",
        &[s(workspace_id), s(but)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|r| r.email).collect())
}

/// The workspaces of `user_id`'s whose admins hear of each of their
/// members' connections and whose sessions of theirs connection `id` runs
/// (or, ended, would run), with the address they are a member there as.
async fn told_each(
    db: &D1Database,
    user_id: &str,
    id: &str,
    settings: &InstanceSettings,
) -> AppResult<Vec<(WorkspaceBrief, String)>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        slug: String,
        name: String,
        #[serde(default)]
        claude_oversight: Option<String>,
        #[serde(default)]
        claude_admin_email: Option<String>,
        email: String,
        #[serde(default)]
        chosen: Option<String>,
        #[serde(default)]
        default_id: Option<String>,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT w.id, w.slug, w.name, w.claude_oversight, w.claude_admin_email, m.email, \
         m.credential_id AS chosen, u.default_credential_id AS default_id \
         FROM workspace_members m JOIN workspaces w ON w.id = m.workspace_id JOIN users u ON u.id = m.user_id \
         WHERE m.user_id = ? AND w.claude_admin_email = 'each'",
        &[s(user_id)],
    )?
    .all()
    .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let conns: Vec<CredentialRow> = q(
        db,
        &format!(
            "SELECT c.*, {USAGE_JSON} AS usage FROM claude_credentials c WHERE c.user_id = ? \
             ORDER BY c.created_at DESC"
        ),
        &[s(user_id)],
    )?
    .all()
    .await?;
    let conns: Vec<Connection> = conns.iter().map(CredentialRow::to_connection).collect();
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let picked = domain::in_use(&conns, r.chosen.as_deref(), r.default_id.as_deref())
                .map(|(c, _)| c)
                .or_else(|| domain::ended(&conns, r.chosen.as_deref(), r.default_id.as_deref()))?;
            let brief = WorkspaceBrief {
                id: r.id,
                slug: r.slug,
                name: r.name,
                claude_oversight: r.claude_oversight,
                claude_admin_email: r.claude_admin_email,
            };
            (picked.id == id && brief.oversight(settings) != ClaudeOversight::Off)
                .then_some((brief, r.email))
        })
        .collect())
}

#[derive(Deserialize)]
struct Holder {
    #[serde(flatten)]
    row: CredentialRow,
    user_email: String,
    user_name: String,
}

const HOLDER_SELECT: &str = "SELECT c.*, u.email AS user_email, u.name AS user_name \
    FROM claude_credentials c JOIN users u ON u.id = c.user_id";

/// Emails the person whose connection `id` ended, and the admins who hear
/// of each.
async fn tell_ended(cfg: &Config, db: &D1Database, id: &str, now: Millis) -> AppResult<()> {
    let Some(mail) = cfg.email.clone() else {
        return Ok(());
    };
    let h: Option<Holder> = q(
        db,
        &format!("{HOLDER_SELECT} WHERE c.id = ? AND u.status = 'active'"),
        &[s(id)],
    )?
    .first()
    .await?;
    let Some(h) = h else { return Ok(()) };
    let instance = crate::instance::name(db, cfg)
        .await
        .unwrap_or_else(|_| cfg.instance_name.clone());
    let (subject, text) = domain::ended_notice(
        &instance,
        h.row.label.as_deref(),
        h.row.kind() == fctl_proto::runner::ClaudeCredentialKind::Oauth,
        h.row.provisioned,
        &cfg.url("/settings?tab=claude"),
    );
    if let Err(e) = crate::email::send(&mail, &h.user_email, &subject, &text).await {
        worker::console_error!("emailing of an ended Claude connection: {}", e.0.message);
    }
    let settings = crate::instance::settings(db).await?;
    let told = Told {
        name: h.user_name.clone(),
        email: String::new(),
        runs_out: None,
        ended_at: Some(now),
        provisioned: h.row.provisioned,
    };
    tell_admins(db, cfg, &mail, &h.row.user_id, id, told, &settings, now).await
}

/// Emails the admins who hear of each of `user_id`'s workspaces where
/// connection `id` runs their sessions, of `told`.
#[allow(clippy::too_many_arguments)]
async fn tell_admins(
    db: &D1Database,
    cfg: &Config,
    mail: &crate::config::EmailConfig,
    user_id: &str,
    id: &str,
    told: Told,
    settings: &InstanceSettings,
    now: Millis,
) -> AppResult<()> {
    for (w, email) in told_each(db, user_id, id, settings).await? {
        let t = Told {
            email,
            ..told.clone()
        };
        let (subject, text) = domain::admin_notice(
            &w.name,
            &t,
            cfg.admin_tokens,
            &cfg.url(&format!("/w/{}?tab=people", w.slug)),
            &cfg.url(&format!("/w/{}?tab=settings", w.slug)),
            now,
        );
        for to in admin_addresses(db, &w.id, user_id).await? {
            if let Err(e) = crate::email::send(mail, &to, &subject, &text).await {
                worker::console_error!("emailing a workspace's admins: {}", e.0.message);
            }
        }
    }
    Ok(())
}

/// How many warnings, then digests, one hourly run sends at most.
const WARNINGS_PER_RUN: i64 = 50;
const DIGESTS_PER_RUN: i64 = 20;

/// The hourly job's part: warns people whose long-lived token runs out
/// within 30 days, then within 7 (`domain::warning_due`), and the admins
/// who hear of each; then sends the weekly emails of workspaces whose
/// admins hear of them once a week. Returns how many people it warned and
/// how many workspaces it summed up. Only where the instance sends email.
pub async fn warn(env: &Env, db: &D1Database, now: Millis) -> AppResult<(usize, usize)> {
    let cfg = Config::from_env(env);
    let Some(mail) = cfg.email.clone() else {
        return Ok((0, 0));
    };
    let settings = crate::instance::settings(db).await?;
    let instance = crate::instance::name(db, &cfg)
        .await
        .unwrap_or_else(|_| cfg.instance_name.clone());
    let mut warned = 0;
    for h in due_warnings(db, now).await? {
        let Some((at, estimated)) = h.row.runs_out() else {
            continue;
        };
        // Once: a run alongside this one may have warned them a moment ago.
        let claimed = q(
            db,
            "UPDATE claude_credentials SET warned_at = ? WHERE id = ? AND COALESCE(warned_at, 0) = ?",
            &[n(now), s(&h.row.id), n(h.row.warned_at.unwrap_or(0))],
        )?
        .run()
        .await?;
        if claimed == 0 {
            continue;
        }
        warned += 1;
        let (subject, text) = domain::expiry_notice(
            &instance,
            h.row.label.as_deref(),
            at,
            estimated,
            h.row.provisioned,
            &cfg.url("/settings?tab=claude"),
            now,
        );
        if let Err(e) = crate::email::send(&mail, &h.user_email, &subject, &text).await {
            worker::console_error!("warning of a Claude token that runs out: {}", e.0.message);
        }
        let told = Told {
            name: h.user_name.clone(),
            email: String::new(),
            runs_out: Some((at, estimated)),
            ended_at: None,
            provisioned: h.row.provisioned,
        };
        if let Err(e) = tell_admins(
            db,
            &cfg,
            &mail,
            &h.row.user_id,
            &h.row.id,
            told,
            &settings,
            now,
        )
        .await
        {
            worker::console_error!("warning admins of a Claude token: {}", e.0.message);
        }
    }
    let digests = digests(db, &cfg, &mail, &settings, now).await?;
    Ok((warned, digests))
}

/// The long-lived tokens whose person a warning is due to at `now`
/// (`domain::warning_due`), the nearest first.
async fn due_warnings(db: &D1Database, now: Millis) -> AppResult<Vec<Holder>> {
    let due: Vec<Holder> = q(
        db,
        &format!(
            "{HOLDER_SELECT} WHERE c.status = 'active' AND c.kind = 'setup_token' AND c.on_runners = 0 \
             AND c.expires_at > ?1 AND c.expires_at <= ?1 + ?2 AND u.status = 'active' \
             AND (c.warned_at IS NULL OR c.warned_at < c.expires_at - ?2 \
               OR (c.expires_at <= ?1 + ?3 AND c.warned_at < c.expires_at - ?3)) \
             ORDER BY c.expires_at LIMIT ?4"
        ),
        &[
            n(now),
            n(domain::SOON_DAYS * DAY_MS),
            n(domain::LAST_DAYS * DAY_MS),
            n(WARNINGS_PER_RUN),
        ],
    )?
    .all()
    .await?;
    Ok(due
        .into_iter()
        .filter(|h| {
            h.row
                .runs_out()
                .is_some_and(|(at, _)| domain::warning_due(at, h.row.warned_at, now).is_some())
        })
        .collect())
}

/// Sends the weekly email of each workspace whose admins hear of their
/// members' connections once a week, and whose last one went a week ago or
/// more, when it has anything to say.
async fn digests(
    db: &D1Database,
    cfg: &Config,
    mail: &crate::config::EmailConfig,
    settings: &InstanceSettings,
    now: Millis,
) -> AppResult<usize> {
    #[derive(Deserialize)]
    struct Due {
        #[serde(flatten)]
        w: WorkspaceBrief,
        #[serde(deserialize_with = "crate::db::opt_int", default)]
        claude_digest_at: Option<i64>,
    }
    let due: Vec<Due> = q(
        db,
        "SELECT id, slug, name, claude_oversight, claude_admin_email, claude_digest_at FROM workspaces \
         WHERE claude_admin_email = 'weekly' AND (claude_digest_at IS NULL OR claude_digest_at <= ?) \
         ORDER BY COALESCE(claude_digest_at, 0) LIMIT ?",
        &[n(now - 7 * DAY_MS), n(DIGESTS_PER_RUN)],
    )?
    .all()
    .await?;
    let mut sent = 0;
    for d in due {
        // Once a week: a run alongside this one may have taken it.
        let claimed = q(
            db,
            "UPDATE workspaces SET claude_digest_at = ? WHERE id = ? AND COALESCE(claude_digest_at, 0) = ?",
            &[n(now), s(&d.w.id), n(d.claude_digest_at.unwrap_or(0))],
        )?
        .run()
        .await?;
        if claimed == 0 {
            continue;
        }
        let oversight = d.w.oversight(settings);
        if oversight == ClaudeOversight::Off || d.w.admin_email() != ClaudeAdminEmail::Weekly {
            continue;
        }
        let world = World::load(db, Some(&d.w.id), now).await?;
        let (mut ended, mut soon) = (Vec::new(), Vec::new());
        for (m, row, _) in world.members(oversight, false, now) {
            let told = Told {
                name: m.user.name.clone(),
                email: m.email.clone(),
                runs_out: None,
                ended_at: m.ended_at,
                provisioned: row.as_ref().is_some_and(|r| r.provisioned),
            };
            match (m.state, m.runs_out_at) {
                (ClaudeState::Ended, _) => ended.push(told),
                (_, Some(at)) if domain::soon(at, now) && at > now => soon.push(Told {
                    runs_out: Some((at, m.runs_out_estimated)),
                    ..told
                }),
                _ => {}
            }
        }
        let Some((subject, text)) = domain::digest(
            &d.w.name,
            &ended,
            &soon,
            cfg.admin_tokens,
            &cfg.url(&format!("/w/{}?tab=people", d.w.slug)),
            &cfg.url(&format!("/w/{}?tab=settings", d.w.slug)),
            now,
        ) else {
            continue;
        };
        for to in admin_addresses(db, &d.w.id, "").await? {
            if let Err(e) = crate::email::send(mail, &to, &subject, &text).await {
                worker::console_error!(
                    "emailing a workspace's weekly Claude summary: {}",
                    e.0.message
                );
            }
        }
        sent += 1;
    }
    Ok(sent)
}

/// The rows of what fctl keeps 30 days that are older: limit pauses and
/// refusals.
pub fn expire(db: &D1Database, now: Millis) -> AppResult<Vec<Query>> {
    Ok(vec![
        q(
            db,
            "DELETE FROM claude_limit_pauses WHERE at < ?",
            &[n(now - domain::KEPT_MS)],
        )?,
        q(
            db,
            "DELETE FROM claude_refusals WHERE at < ?",
            &[n(now - domain::KEPT_MS)],
        )?,
    ])
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::db::Count;
    use crate::testing;
    use fctl_proto::api::ClaudeConnectionKind;
    use wasm_bindgen_test::wasm_bindgen_test;

    const NOW: i64 = 1_800_000_000_000;

    /// Acme, with Ada its admin, Grace on a long-lived token an admin token
    /// set, Linus on a token Claude refused, and Ken with nothing; each with
    /// a session in its project, and Grace one in her own workspace too.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, role, created_at, updated_at) VALUES \
             ('usr_ada', 'ada@acme.dev', 'Ada', 'admin', 0, 0), ('usr_grace', 'grace@acme.dev', 'Grace', 'member', 0, 0), \
             ('usr_linus', 'linus@acme.dev', 'Linus', 'member', 0, 0), ('usr_ken', 'ken@acme.dev', 'Ken', 'member', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_at, updated_at) VALUES \
             ('wsp_acme', 'acme', 'Acme', 0, 0), ('wsp_home', 'home', 'Home', 0, 0)",
            "INSERT INTO workspace_members (workspace_id, user_id, email, role, created_at) VALUES \
             ('wsp_acme', 'usr_ada', 'ada@acme.dev', 'owner', 0), ('wsp_acme', 'usr_grace', 'grace@acme.dev', 'member', 0), \
             ('wsp_acme', 'usr_linus', 'linus@acme.dev', 'member', 0), ('wsp_acme', 'usr_ken', 'ken@acme.dev', 'member', 0), \
             ('wsp_home', 'usr_grace', 'grace@acme.dev', 'owner', 0)",
            "INSERT INTO claude_credentials (id, user_id, kind, secret_enc, status, expires_at, created_at, updated_at, provisioned, answered_at) VALUES \
             ('crd_grace', 'usr_grace', 'setup_token', 'x', 'active', 1800864000000, 1768536000000, 0, 1, 1799999000000)",
            "INSERT INTO claude_credentials (id, user_id, kind, secret_enc, status, expires_at, created_at, updated_at, ended_at) VALUES \
             ('crd_linus', 'usr_linus', 'setup_token', 'x', 'invalid', 1810000000000, 1778000000000, 0, 1799900000000)",
            "INSERT INTO claude_credentials (id, user_id, kind, secret_enc, status, subscription_type, created_at, updated_at, last_used_at) VALUES \
             ('crd_ada', 'usr_ada', 'oauth', 'x', 'active', 'max', 0, 0, 1799999999000)",
            "UPDATE users SET default_credential_id = 'crd_grace' WHERE id = 'usr_grace'",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) VALUES \
             ('prj_web', 'web', 'Web', 'usr_ada', 0, 0, 'wsp_acme'), ('prj_mine', 'mine', 'Mine', 'usr_grace', 0, 0, 'wsp_home')",
            "INSERT INTO sessions (id, project_id, number, title, prompt, status, created_by, claude_session_id, created_at, updated_at, credential_id) VALUES \
             ('ses_1', 'prj_web', 1, 'One', 'Go', 'running', 'usr_grace', 'c1', 0, 0, 'crd_grace'), \
             ('ses_2', 'prj_web', 2, 'Two', 'Go', 'running', 'usr_grace', 'c2', 0, 0, 'crd_grace'), \
             ('ses_3', 'prj_mine', 1, 'Three', 'Go', 'running', 'usr_grace', 'c3', 0, 0, 'crd_grace')",
        ] {
            q(db, sql, &[])?.run().await?;
        }
        Ok(())
    }

    async fn count(db: &D1Database, sql: &str) -> AppResult<i64> {
        let c: Option<Count> = q(db, sql, &[])?.first().await?;
        Ok(c.map_or(0, |c| c.n))
    }

    fn seen(answered: bool, refused: bool, pauses: &[(&str, i64)]) -> Seen {
        Seen {
            answered,
            refused,
            pauses: pauses
                .iter()
                .map(|(l, r)| (Some((*l).to_owned()), Some(*r)))
                .collect(),
        }
    }

    #[wasm_bindgen_test]
    async fn ends_a_token_once_enough_sessions_saw_claude_refuse_it() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        batch(&db, statements(&db, "ses_1", &seen(false, true, &[]), NOW)?).await?;
        // Again in the same session: still one.
        batch(
            &db,
            statements(&db, "ses_1", &seen(false, true, &[]), NOW + 1)?,
        )
        .await?;
        assert_eq!(refused_enough(&db, "ses_1", 2).await?, None);
        assert_eq!(
            refused_enough(&db, "ses_1", 1).await?,
            Some("crd_grace".into())
        );
        batch(
            &db,
            statements(&db, "ses_2", &seen(false, true, &[]), NOW + 2)?,
        )
        .await?;
        assert_eq!(
            refused_enough(&db, "ses_2", 2).await?,
            Some("crd_grace".into())
        );
        // Claude answered since: the refusals before don't count.
        batch(
            &db,
            statements(&db, "ses_3", &seen(true, false, &[]), NOW + 3)?,
        )
        .await?;
        assert_eq!(refused_enough(&db, "ses_2", 1).await?, None);
        assert_eq!(
            count(
                &db,
                "SELECT answered_at AS n FROM claude_credentials WHERE id = 'crd_grace'"
            )
            .await?,
            NOW + 3
        );
        // An older answer doesn't move it back.
        batch(&db, statements(&db, "ses_1", &seen(true, false, &[]), NOW)?).await?;
        assert_eq!(
            count(
                &db,
                "SELECT answered_at AS n FROM claude_credentials WHERE id = 'crd_grace'"
            )
            .await?,
            NOW + 3
        );
        // They go a month on.
        for st in expire(&db, NOW + domain::KEPT_MS + 10)? {
            st.run().await?;
        }
        assert_eq!(
            count(&db, "SELECT COUNT(*) AS n FROM claude_refusals").await?,
            0
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn counts_a_pause_once_however_often_its_report_comes() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let paused = seen(false, false, &[("seven_day", NOW + 86_400_000)]);
        batch(&db, statements(&db, "ses_1", &paused, NOW)?).await?;
        batch(&db, statements(&db, "ses_1", &paused, NOW + 5)?).await?;
        batch(&db, statements(&db, "ses_2", &paused, NOW + 6)?).await?;
        assert_eq!(
            count(
                &db,
                "SELECT COUNT(*) AS n FROM claude_limit_pauses WHERE credential_id = 'crd_grace'"
            )
            .await?,
            2
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn shows_admins_each_member_as_their_sessions_there_run() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Paused twice in Acme, once at home: Acme's admins see Acme's.
        for (session, at) in [
            ("ses_1", NOW - 1000),
            ("ses_2", NOW - 500),
            ("ses_3", NOW - 100),
        ] {
            batch(
                &db,
                statements(
                    &db,
                    session,
                    &seen(false, false, &[("five_hour", NOW + 3_600_000)]),
                    at,
                )?,
            )
            .await?;
        }
        let world = World::load(&db, Some("wsp_acme"), NOW).await?;
        let members: Vec<MemberClaude> = world
            .members(ClaudeOversight::Details, true, NOW)
            .into_iter()
            .map(|(m, _, _)| m)
            .collect();
        let names: Vec<&str> = members.iter().map(|m| m.user.name.as_str()).collect();
        assert_eq!(names, ["Ada", "Grace", "Ken", "Linus"]);
        let by = |name: &str| members.iter().find(|m| m.user.name == name).cloned();
        let ada = by("Ada").unwrap_or_else(|| unreachable!());
        assert_eq!(ada.state, ClaudeState::Works);
        let conn = ada.connection.unwrap_or_else(|| unreachable!());
        assert_eq!(conn.kind, ClaudeConnectionKind::Login);
        assert_eq!(conn.plan.as_deref(), Some("max"));
        assert_eq!(ada.runs_out_at, None);
        let grace = by("Grace").unwrap_or_else(|| unreachable!());
        assert_eq!(grace.state, ClaudeState::Works);
        assert_eq!(grace.runs_out_at, Some(1_800_864_000_000));
        assert!(grace.runs_out_estimated);
        assert!(
            grace
                .connection
                .as_ref()
                .is_some_and(|c| c.set_by_admin_token)
        );
        assert_eq!(grace.paused_until, Some(NOW + 3_600_000));
        assert_eq!(
            grace.pauses.as_ref().map(|p| (p.week, p.month)),
            Some((2, 2))
        );
        let ken = by("Ken").unwrap_or_else(|| unreachable!());
        assert_eq!(ken.state, ClaudeState::None);
        assert_eq!(ken.connection, None);
        let linus = by("Linus").unwrap_or_else(|| unreachable!());
        assert_eq!(linus.state, ClaudeState::Ended);
        assert_eq!(linus.ended_at, Some(1_799_900_000_000));
        // Every active account, for instance admins: Grace's pauses at home too.
        let all = World::load(&db, None, NOW).await?;
        let grace = all
            .members(ClaudeOversight::Details, true, NOW)
            .into_iter()
            .find(|(m, _, _)| m.user.name == "Grace")
            .map(|(m, _, _)| m);
        assert_eq!(grace.and_then(|g| g.pauses).map(|p| p.week), Some(3));
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn warns_once_a_month_before_and_once_a_week_before() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        // Grace's runs out on 1800864000000: ten days after NOW.
        let due = due_warnings(&db, NOW).await?;
        assert_eq!(
            due.iter().map(|h| h.row.id.as_str()).collect::<Vec<_>>(),
            ["crd_grace"]
        );
        q(
            &db,
            "UPDATE claude_credentials SET warned_at = ? WHERE id = 'crd_grace'",
            &[n(NOW)],
        )?
        .run()
        .await?;
        assert!(due_warnings(&db, NOW + 86_400_000).await?.is_empty());
        // Six days before: the week's warning.
        assert_eq!(due_warnings(&db, NOW + 4 * 86_400_000).await?.len(), 1);
        // Linus's ended: nothing to warn of.
        q(
            &db,
            "UPDATE claude_credentials SET expires_at = ? WHERE id = 'crd_linus'",
            &[n(NOW + 86_400_000)],
        )?
        .run()
        .await?;
        assert_eq!(due_warnings(&db, NOW + 4 * 86_400_000).await?.len(), 1);
        Ok(())
    }
}

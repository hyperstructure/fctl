//! Webhooks (docs/architecture.md#webhooks, ADR 0083): the `webhooks` rows
//! in D1, their secrets, and handing what happens in a workspace to its
//! HooksObject when one of its webhooks wants it. The HooksObject
//! (`do_hooks`) posts them; `api::hooks` is how people manage them.
//!
//! Sources call [`emit`] after their answer (`wait_until`): it reads which
//! webhooks the workspace has from D1 at most every [`CACHED_MS`] per
//! isolate, and asks the HooksObject nothing when none of them wants the
//! event, so that a workspace without webhooks costs a cached lookup.

use crate::crypto::Keys;
use crate::db::{Count, int, int_bool, opt_int, q, s};
use crate::error::AppResult;
use crate::internal::{self, HookEmit, HooksChanged};
use fctl_domain::hooks::{Event, Hook, wants};
use fctl_proto::hooks::{HookPlace, Webhook, WebhookHealth, WebhookOrigin};
use fctl_proto::session::UserRef;
use fctl_proto::{ProjectId, UserId, WebhookId, WorkspaceId};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use worker::{D1Database, Env};

/// SELECT for [`HookRow`]; `WHERE` and `ORDER BY` follow.
pub const SELECT: &str = "SELECT h.id, h.workspace_id, h.project_id, p.slug AS project_slug, \
    p.name AS project_name, h.name, h.url, h.events, h.projects, h.started_by, h.secret, \
    h.previous_secret, h.previous_until, h.enabled, h.disabled_reason, h.disabled_at, h.created_by, \
    u.name AS creator_name, u.avatar_url AS creator_avatar, h.created_at, h.updated_at \
    FROM webhooks h LEFT JOIN projects p ON p.id = h.project_id LEFT JOIN users u ON u.id = h.created_by";

/// A webhook's row, with its project's and its maker's names.
#[derive(Clone, Debug, Deserialize)]
pub struct HookRow {
    pub id: String,
    pub workspace_id: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_slug: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    pub name: Option<String>,
    pub url: String,
    pub events: String,
    pub projects: String,
    pub started_by: String,
    pub secret: String,
    pub previous_secret: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub previous_until: Option<i64>,
    #[serde(deserialize_with = "int_bool")]
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    #[serde(deserialize_with = "opt_int", default)]
    pub disabled_at: Option<i64>,
    pub created_by: Option<String>,
    #[serde(default)]
    pub creator_name: Option<String>,
    #[serde(default)]
    pub creator_avatar: Option<String>,
    #[serde(deserialize_with = "int")]
    pub created_at: i64,
    #[serde(deserialize_with = "int")]
    pub updated_at: i64,
}

fn list<T: serde::de::DeserializeOwned>(json: &str) -> Vec<T> {
    serde_json::from_str(json).unwrap_or_default()
}

impl HookRow {
    pub fn events(&self) -> Vec<String> {
        list(&self.events)
    }

    pub fn projects(&self) -> Vec<String> {
        list(&self.projects)
    }

    pub fn started_by(&self) -> Vec<WebhookOrigin> {
        list(&self.started_by)
    }

    /// What it wants, for [`wants`].
    pub fn hook(&self) -> Hook {
        Hook {
            project_id: self.project_id.clone(),
            events: self.events(),
            projects: self.projects(),
            started_by: self.started_by(),
            enabled: self.enabled,
        }
    }

    /// The API's webhook: `health` from its HooksObject, and whether the
    /// caller manages it.
    pub fn to_webhook(&self, health: Option<WebhookHealth>, can_manage: bool) -> Webhook {
        Webhook {
            id: WebhookId(self.id.clone()),
            workspace_id: WorkspaceId(self.workspace_id.clone()),
            project: self.project_id.as_ref().map(|id| HookPlace {
                id: id.clone(),
                slug: self.project_slug.clone().unwrap_or_default(),
                name: self.project_name.clone().unwrap_or_default(),
            }),
            name: self.name.clone(),
            url: self.url.clone(),
            events: self.events(),
            projects: self.projects().into_iter().map(ProjectId).collect(),
            started_by: self.started_by(),
            enabled: self.enabled,
            disabled_reason: self.disabled_reason.clone().filter(|_| !self.enabled),
            disabled_at: self.disabled_at.filter(|_| !self.enabled),
            previous_secret_until: self
                .previous_until
                .filter(|_| self.previous_secret.is_some()),
            created_by: self.created_by.as_ref().map(|id| UserRef {
                id: UserId(id.clone()),
                name: self.creator_name.clone().unwrap_or_default(),
                avatar_url: self.creator_avatar.clone(),
            }),
            created_at: self.created_at,
            updated_at: self.updated_at,
            health,
            can_manage,
        }
    }

    /// Its secret, opened.
    pub fn secret(&self, keys: &Keys) -> AppResult<String> {
        open_secret(keys, &self.id, &self.secret)
    }

    /// The keys that sign its deliveries at `now`: its secret's, then its
    /// predecessor's while that still signs too.
    pub fn signing_keys(&self, keys: &Keys, now: i64) -> AppResult<Vec<Vec<u8>>> {
        let mut out = Vec::new();
        if let Some(k) = fctl_domain::hooks::key(&self.secret(keys)?) {
            out.push(k);
        }
        if let (Some(previous), Some(until)) = (&self.previous_secret, self.previous_until)
            && until > now
            && let Some(k) = fctl_domain::hooks::key(&open_secret(keys, &self.id, previous)?)
        {
            out.push(k);
        }
        Ok(out)
    }
}

/// A webhook's secret, sealed for its row.
pub fn seal_secret(keys: &Keys, id: &str, secret: &str) -> AppResult<String> {
    keys.seal(&format!("webhook:{id}"), secret.as_bytes())
}

pub fn open_secret(keys: &Keys, id: &str, sealed: &str) -> AppResult<String> {
    let bytes = keys.open(&format!("webhook:{id}"), sealed)?;
    String::from_utf8(bytes).map_err(crate::error::internal)
}

/// A workspace's webhooks, its projects' too, oldest first.
pub async fn of_workspace(db: &D1Database, workspace_id: &str) -> AppResult<Vec<HookRow>> {
    q(
        db,
        &format!("{SELECT} WHERE h.workspace_id = ? ORDER BY h.created_at, h.id"),
        &[s(workspace_id)],
    )?
    .all()
    .await
}

pub async fn row(db: &D1Database, id: &str) -> AppResult<Option<HookRow>> {
    q(db, &format!("{SELECT} WHERE h.id = ?"), &[s(id)])?
        .first()
        .await
}

/// How many webhooks a workspace has, its projects' included.
pub async fn count(db: &D1Database, workspace_id: &str) -> AppResult<u32> {
    let c: Option<Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM webhooks WHERE workspace_id = ?",
        &[s(workspace_id)],
    )?
    .first()
    .await?;
    Ok(c.map_or(0, |c| u32::try_from(c.n.max(0)).unwrap_or(u32::MAX)))
}

// ───────────────────────────── handing events over ─────────────────────────────

/// How long an isolate goes by the webhooks it read for a workspace before
/// it reads them again: a change takes that long, at most, to reach the
/// events of sessions in other isolates.
pub const CACHED_MS: i64 = 30_000;

/// A workspace's webhooks, as far as choosing what they get goes, with
/// when they were read.
type Read = (i64, Rc<Vec<Hook>>);

thread_local! {
    static WANTED: RefCell<HashMap<String, Read>> = RefCell::new(HashMap::new());
    /// Each project's workspace, which never changes.
    static WORKSPACE_OF: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Forgets what this isolate read of a workspace's webhooks: they changed.
pub fn forget_cached(workspace_id: &str) {
    WANTED.with(|w| w.borrow_mut().remove(workspace_id));
}

async fn wanted(db: &D1Database, workspace_id: &str, now: i64) -> AppResult<Rc<Vec<Hook>>> {
    let cached = WANTED.with(|w| {
        w.borrow()
            .get(workspace_id)
            .filter(|(at, _)| now - at < CACHED_MS)
            .map(|(_, hooks)| hooks.clone())
    });
    if let Some(hooks) = cached {
        return Ok(hooks);
    }
    #[derive(Deserialize)]
    struct Row {
        project_id: Option<String>,
        events: String,
        projects: String,
        started_by: String,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT project_id, events, projects, started_by FROM webhooks WHERE workspace_id = ? AND enabled = 1",
        &[s(workspace_id)],
    )?
    .all()
    .await?;
    let hooks = Rc::new(
        rows.into_iter()
            .map(|r| Hook {
                project_id: r.project_id,
                events: list(&r.events),
                projects: list(&r.projects),
                started_by: list(&r.started_by),
                enabled: true,
            })
            .collect::<Vec<_>>(),
    );
    WANTED.with(|w| {
        let mut w = w.borrow_mut();
        // An isolate that hears of many workspaces keeps the latest few.
        if w.len() > 500 {
            w.clear();
        }
        w.insert(workspace_id.to_owned(), (now, hooks.clone()));
    });
    Ok(hooks)
}

/// The workspace of a project.
pub async fn workspace_of(db: &D1Database, project_id: &str) -> AppResult<Option<String>> {
    if let Some(w) = WORKSPACE_OF.with(|m| m.borrow().get(project_id).cloned()) {
        return Ok(Some(w));
    }
    #[derive(Deserialize)]
    struct Row {
        workspace_id: Option<String>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT workspace_id FROM projects WHERE id = ?",
        &[s(project_id)],
    )?
    .first()
    .await?;
    let found = row.and_then(|r| r.workspace_id);
    if let Some(w) = &found {
        WORKSPACE_OF.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 5000 {
                m.clear();
            }
            m.insert(project_id.to_owned(), w.clone());
        });
    }
    Ok(found)
}

/// Hands `event` to the HooksObject of its workspace (`workspace_id`, else
/// its project's), when one of the workspace's webhooks wants it.
pub async fn emit(env: &Env, workspace_id: Option<&str>, event: Event) -> AppResult<()> {
    let db = env.d1("DB")?;
    let workspace = match (workspace_id, event.project_id.as_deref()) {
        (Some(w), _) => w.to_owned(),
        (None, Some(p)) => match workspace_of(&db, p).await? {
            Some(w) => w,
            None => return Ok(()),
        },
        (None, None) => return Ok(()),
    };
    let hooks = wanted(&db, &workspace, crate::ctx::now_ms()).await?;
    if !hooks
        .iter()
        .any(|h| wants(h, event.kind, event.project_id.as_deref(), event.origin))
    {
        return Ok(());
    }
    let stub = internal::hooks_stub(env, &workspace)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/emit",
        &HookEmit {
            workspace_id: workspace.clone(),
            event_id: fctl_domain::ids::new_id("evt", event.at),
            kind: event.kind.as_str().to_owned(),
            project_id: event.project_id,
            session_id: event.session_id.map(|s| s.0),
            origin: event.origin,
            gist: event.gist,
            data: event.data,
            at: event.at,
        },
    )
    .await?;
    Ok(())
}

/// Tells a workspace's HooksObject that its webhooks changed (`forget`:
/// webhook `webhook_id` went, with its deliveries), and this isolate's
/// cache too.
pub async fn changed(env: &Env, workspace_id: &str, webhook_id: Option<&str>, forget: bool) {
    forget_cached(workspace_id);
    let told: AppResult<serde_json::Value> = async {
        let stub = internal::hooks_stub(env, workspace_id)?;
        internal::post(
            &stub,
            if forget { "/forget" } else { "/changed" },
            &HooksChanged {
                workspace_id: workspace_id.to_owned(),
                webhook_id: webhook_id.map(str::to_owned),
                project_id: None,
                fresh: None,
            },
        )
        .await
    }
    .await;
    if let Err(e) = told {
        worker::console_error!(
            "telling {workspace_id}'s webhooks they changed: {}",
            e.0.message
        );
    }
}

/// Forgets a deleted project in its workspace's delivery logs: what its
/// events said, and its own webhooks' deliveries (the rows cascaded).
pub async fn forget_project(env: &Env, workspace_id: &str, project_id: &str) -> AppResult<()> {
    forget_cached(workspace_id);
    let stub = internal::hooks_stub(env, workspace_id)?;
    let _: serde_json::Value = internal::post(
        &stub,
        "/forget-project",
        &HooksChanged {
            workspace_id: workspace_id.to_owned(),
            webhook_id: None,
            project_id: Some(project_id.to_owned()),
            fresh: None,
        },
    )
    .await?;
    Ok(())
}

/// Forgets everything a deleted workspace's webhooks sent.
pub async fn purge(env: &Env, workspace_id: &str) -> AppResult<()> {
    forget_cached(workspace_id);
    let stub = internal::hooks_stub(env, workspace_id)?;
    let _: serde_json::Value = internal::post(&stub, "/purge", &serde_json::json!({})).await?;
    Ok(())
}

#[cfg(all(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::testing;
    use fctl_domain::hooks::Kind;
    use wasm_bindgen_test::wasm_bindgen_test;

    async fn run_sql(db: &D1Database, sql: &str) -> AppResult<()> {
        q(db, sql, &[])?.run().await?;
        Ok(())
    }

    /// Acme, with Web and Api, a webhook of its own for Web's and Api's
    /// questions, one of Web's own, and one that is off.
    async fn world(db: &D1Database) -> AppResult<()> {
        for sql in [
            "INSERT INTO users (id, email, name, created_at, updated_at) VALUES ('usr_ada', 'ada@example.com', 'Ada', 0, 0)",
            "INSERT INTO workspaces (id, slug, name, created_by, created_at, updated_at) \
             VALUES ('wsp_1', 'acme', 'Acme', 'usr_ada', 0, 0)",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) \
             VALUES ('prj_web', 'web', 'Web', 'usr_ada', 0, 0, 'wsp_1')",
            "INSERT INTO projects (id, slug, name, created_by, created_at, updated_at, workspace_id) \
             VALUES ('prj_api', 'api', 'Api', 'usr_ada', 0, 0, 'wsp_1')",
            "INSERT INTO webhooks (id, workspace_id, project_id, name, url, events, projects, started_by, secret, \
             created_by, created_at, updated_at) VALUES ('whk_slack', 'wsp_1', NULL, 'Slack', \
             'https://hooks.slack.com/x', '[\"session.question\"]', '[\"prj_web\",\"prj_api\"]', '[\"person\"]', \
             'sealed', 'usr_ada', 1, 1)",
            "INSERT INTO webhooks (id, workspace_id, project_id, url, secret, created_by, created_at, updated_at) \
             VALUES ('whk_web', 'wsp_1', 'prj_web', 'https://example.com/web', 'sealed', 'usr_ada', 2, 2)",
            "INSERT INTO webhooks (id, workspace_id, url, secret, enabled, disabled_reason, disabled_at, created_at, updated_at) \
             VALUES ('whk_off', 'wsp_1', 'https://example.com/off', 'sealed', 0, 'Every delivery failed.', 9, 3, 3)",
        ] {
            run_sql(db, sql).await?;
        }
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn a_workspace_s_webhooks_read_as_the_api_shows_them() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        let rows = of_workspace(&db, "wsp_1").await?;
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["whk_slack", "whk_web", "whk_off"]);
        assert_eq!(count(&db, "wsp_1").await?, 3);
        let slack = rows[0].to_webhook(None, true);
        assert_eq!(slack.name.as_deref(), Some("Slack"));
        assert_eq!(slack.events, ["session.question"]);
        assert_eq!(slack.projects.len(), 2);
        assert_eq!(slack.started_by, [WebhookOrigin::Person]);
        assert_eq!(slack.created_by.map(|u| u.name), Some("Ada".to_owned()));
        assert!(slack.project.is_none());
        let web = rows[1].to_webhook(None, false);
        assert_eq!(web.project.map(|p| p.slug), Some("web".to_owned()));
        assert!(web.events.is_empty() && !web.can_manage);
        let off = rows[2].to_webhook(None, true);
        assert!(!off.enabled);
        assert_eq!(
            off.disabled_reason.as_deref(),
            Some("Every delivery failed.")
        );
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn events_go_by_the_webhooks_an_isolate_read_lately() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        forget_cached("wsp_1");
        let hooks = wanted(&db, "wsp_1", 1_000).await?;
        // The one that is off isn't read.
        assert_eq!(hooks.len(), 2);
        assert!(hooks.iter().any(|h| wants(
            h,
            Kind::SessionQuestion,
            Some("prj_api"),
            Some(WebhookOrigin::Person)
        )));
        assert!(!hooks.iter().any(|h| wants(
            h,
            Kind::SessionQuestion,
            Some("prj_api"),
            Some(WebhookOrigin::Schedule)
        )));
        run_sql(&db, "UPDATE webhooks SET enabled = 1 WHERE id = 'whk_off'").await?;
        // Within the half minute, the isolate goes by what it read.
        assert_eq!(wanted(&db, "wsp_1", 1_000 + CACHED_MS - 1).await?.len(), 2);
        // After it, or once told of a change here, it reads them again.
        assert_eq!(wanted(&db, "wsp_1", 1_000 + CACHED_MS).await?.len(), 3);
        run_sql(&db, "DELETE FROM webhooks WHERE id = 'whk_web'").await?;
        forget_cached("wsp_1");
        assert_eq!(wanted(&db, "wsp_1", 1_000 + CACHED_MS + 1).await?.len(), 2);
        Ok(())
    }

    #[wasm_bindgen_test]
    async fn a_project_s_webhooks_go_with_it_and_its_workspace_is_found() -> AppResult<()> {
        let db = testing::database();
        world(&db).await?;
        assert_eq!(
            workspace_of(&db, "prj_api").await?.as_deref(),
            Some("wsp_1")
        );
        assert_eq!(workspace_of(&db, "prj_gone").await?, None);
        run_sql(&db, "DELETE FROM projects WHERE id = 'prj_web'").await?;
        assert!(row(&db, "whk_web").await?.is_none());
        assert!(row(&db, "whk_slack").await?.is_some());
        run_sql(&db, "DELETE FROM workspaces WHERE id = 'wsp_1'").await?;
        assert_eq!(count(&db, "wsp_1").await?, 0);
        Ok(())
    }
}

/// Tells a workspace's HooksObject that `webhook_id` was turned on again:
/// it fails afresh from now, rather than from before it was off, so that
/// the next failure doesn't turn it off again at once.
pub async fn turned_on(env: &Env, workspace_id: &str, webhook_id: &str) {
    forget_cached(workspace_id);
    let told: AppResult<serde_json::Value> = async {
        let stub = internal::hooks_stub(env, workspace_id)?;
        internal::post(
            &stub,
            "/changed",
            &HooksChanged {
                workspace_id: workspace_id.to_owned(),
                webhook_id: None,
                project_id: None,
                fresh: Some(webhook_id.to_owned()),
            },
        )
        .await
    }
    .await;
    if let Err(e) = told {
        worker::console_error!(
            "telling {workspace_id}'s webhooks {webhook_id} is on: {}",
            e.0.message
        );
    }
}

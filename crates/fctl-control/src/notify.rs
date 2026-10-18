//! Notifications (docs/architecture.md#notifications): who follows which
//! session, what notifies them, and handing a session's changes to the
//! NotifyObject of each person who should hear about them.

use crate::db::{n, q, s};
use crate::error::AppResult;
use crate::internal::{self, Notify};
use fctl_domain::notify::{self, Event, Follower};
use fctl_proto::api::NotificationSettings;
use fctl_proto::session::{InstanceRole, ProjectRole, SessionRole};
use serde::Deserialize;
use worker::{D1Database, Env};

#[derive(Deserialize)]
struct SettingsRow {
    settings: String,
}

/// A person's notification settings: the defaults until they change them.
pub async fn settings(db: &D1Database, user_id: &str) -> AppResult<NotificationSettings> {
    let row: Option<SettingsRow> = q(
        db,
        "SELECT settings FROM notification_settings WHERE user_id = ?",
        &[s(user_id)],
    )?
    .first()
    .await?;
    Ok(row
        .and_then(|r| serde_json::from_str(&r.settings).ok())
        .unwrap_or_default())
}

pub async fn save_settings(
    db: &D1Database,
    user_id: &str,
    settings: &NotificationSettings,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "INSERT INTO notification_settings (user_id, settings, updated_at) VALUES (?, ?, ?) \
         ON CONFLICT (user_id) DO UPDATE SET settings = excluded.settings, updated_at = excluded.updated_at",
        &[s(user_id), s(&serde_json::to_string(settings)?), n(now)],
    )?
    .run()
    .await?;
    Ok(())
}

pub async fn follow(db: &D1Database, session_id: &str, user_id: &str, now: i64) -> AppResult<()> {
    q(
        db,
        "INSERT OR IGNORE INTO session_followers (session_id, user_id, created_at) VALUES (?, ?, ?)",
        &[s(session_id), s(user_id), n(now)],
    )?
    .run()
    .await?;
    Ok(())
}

pub async fn unfollow(db: &D1Database, session_id: &str, user_id: &str) -> AppResult<()> {
    q(
        db,
        "DELETE FROM session_followers WHERE session_id = ? AND user_id = ?",
        &[s(session_id), s(user_id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// Follows a session someone started or resumed, unless they turned that off.
pub async fn follow_own(
    db: &D1Database,
    session_id: &str,
    user_id: &str,
    now: i64,
) -> AppResult<()> {
    if settings(db, user_id).await?.follow_started {
        follow(db, session_id, user_id, now).await?;
    }
    Ok(())
}

/// A follower of a session with a browser to notify, and their access to it.
#[derive(Deserialize)]
struct FollowerRow {
    user_id: String,
    role: String,
    member_role: Option<String>,
    share_role: Option<String>,
    created_by: String,
    settings: Option<String>,
}

const FOLLOWERS: &str = "SELECT f.user_id AS user_id, u.role AS role, m.role AS member_role, \
    sh.role AS share_role, s.created_by AS created_by, ns.settings AS settings \
    FROM session_followers f \
    JOIN sessions s ON s.id = f.session_id \
    JOIN users u ON u.id = f.user_id \
    LEFT JOIN project_members m ON m.project_id = s.project_id AND m.user_id = f.user_id \
    LEFT JOIN session_shares sh ON sh.session_id = f.session_id AND sh.user_id = f.user_id \
    LEFT JOIN notification_settings ns ON ns.user_id = f.user_id \
    WHERE f.session_id = ? AND u.status = 'active' \
    AND EXISTS (SELECT 1 FROM push_subscriptions p WHERE p.user_id = f.user_id)";

impl FollowerRow {
    fn follower(self) -> Follower {
        Follower {
            instance_role: InstanceRole::parse(&self.role).unwrap_or(InstanceRole::Member),
            project_role: self.member_role.as_deref().and_then(ProjectRole::parse),
            share_role: self.share_role.as_deref().and_then(SessionRole::parse),
            is_creator: self.created_by == self.user_id,
            settings: self
                .settings
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok())
                .unwrap_or_default(),
            user_id: self.user_id,
        }
    }
}

/// Tells the NotifyObject of every follower who should hear about `event`:
/// those who can still see the session, want this kind of news and have a
/// browser for it, except the people in `except`.
pub async fn session_changed(env: &Env, event: Event, except: Vec<String>) -> AppResult<()> {
    let db = env.d1("DB")?;
    let rows: Vec<FollowerRow> = q(&db, FOLLOWERS, &[s(event.session_id.as_str())])?
        .all()
        .await?;
    let followers: Vec<Follower> = rows.into_iter().map(FollowerRow::follower).collect();
    for user_id in notify::recipients(&followers, event.kind, &except) {
        let posted: AppResult<serde_json::Value> = async {
            let stub = internal::notify_stub(env, user_id)?;
            internal::post(
                &stub,
                "/event",
                &Notify {
                    user_id: user_id.to_owned(),
                    event: event.clone(),
                },
            )
            .await
        }
        .await;
        if let Err(e) = posted {
            worker::console_error!("notifying {user_id}: {}", e.0.message);
        }
    }
    Ok(())
}

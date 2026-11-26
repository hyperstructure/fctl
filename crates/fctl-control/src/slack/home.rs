//! Slack as a place to hear from fctl (docs/architecture.md#slack): fctl's
//! Home tab and `/fctl status` (what needs you, what runs, your recent pull
//! requests), notifications as direct messages from the app for people who
//! chose "Tell me in Slack", and projects' chosen events in a channel.

use super::Slack;
use crate::config::Config;
use crate::db::{n, q, s};
use crate::error::AppResult;
use fctl_domain::hooks::Kind as HookKind;
use fctl_domain::notify::{Event as NotifyEvent, Kind};
use fctl_domain::slack::blocks::{self, Home, HomeSession};
use fctl_domain::slack::text;
use fctl_proto::session::{PullRequestRef, SessionQuestion, SessionStatus};
use fctl_proto::slack::SlackProject;
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{D1Database, Env};

#[derive(Deserialize)]
struct Mine {
    id: String,
    number: i64,
    title: String,
    status: String,
    slug: String,
    question: Option<String>,
    pull_requests: Option<String>,
}

/// Someone's sessions as Slack lists them: what needs them, what runs, and
/// their newest pull requests.
async fn lists(
    cfg: &Config,
    db: &D1Database,
    user: &str,
    now: i64,
) -> AppResult<(
    Vec<HomeSession>,
    Vec<HomeSession>,
    Vec<(String, String, String)>,
)> {
    let rows: Vec<Mine> = q(
        db,
        "SELECT s.id, s.number, s.title, s.status, p.slug, s.question, s.pull_requests FROM sessions s \
         JOIN projects p ON p.id = s.project_id WHERE s.created_by = ? AND s.archived = 0 AND s.updated_at > ? \
         AND NOT EXISTS (SELECT 1 FROM reviews rv WHERE rv.session_id = s.id) ORDER BY s.updated_at DESC LIMIT 40",
        &[s(user), n(now - 14 * 24 * 60 * 60 * 1000)],
    )?
    .all()
    .await?;
    let mut needs = Vec::new();
    let mut running = Vec::new();
    let mut pulls = Vec::new();
    for r in &rows {
        let status = SessionStatus::parse(&r.status);
        let question = r
            .question
            .as_deref()
            .and_then(|j| serde_json::from_str::<SessionQuestion>(j).ok());
        let item = |words: String| HomeSession {
            reference: format!("{}#{}", r.slug, r.number),
            title: r.title.clone(),
            url: cfg.url(&format!("/s/{}", r.id)),
            status: words,
        };
        let prs: Vec<PullRequestRef> = r
            .pull_requests
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default();
        let all_done = !prs.is_empty()
            && prs
                .iter()
                .all(|p| matches!(p.state.as_deref(), Some("merged" | "closed")));
        match status {
            Some(SessionStatus::Idle) if question.is_some() => needs.push(item(format!(
                "Asks: {}",
                question
                    .map(|q| text::clip(&q.text, 120))
                    .unwrap_or_default()
            ))),
            Some(SessionStatus::Idle) if !all_done => {
                needs.push(item("Waiting for a reply".into()))
            }
            Some(SessionStatus::Paused) => needs.push(item("Paused on a usage limit".into())),
            Some(SessionStatus::Failed) => needs.push(item("Failed".into())),
            Some(
                SessionStatus::Queued
                | SessionStatus::Provisioning
                | SessionStatus::Preparing
                | SessionStatus::Running,
            ) => running.push(item(match status {
                Some(SessionStatus::Running) => "Working".into(),
                _ => "Starting".into(),
            })),
            _ => {}
        }
        for p in prs.iter().rev() {
            if pulls.len() >= 8 {
                break;
            }
            let label = fctl_domain::repos::parse_remote(&p.url)
                .map(|repo| format!("{}#{}", repo.full_name(), p.number))
                .unwrap_or_else(|| format!("#{}", p.number));
            if !pulls.iter().any(|(l, _, _)| *l == label) {
                pulls.push((label, p.title.clone().unwrap_or_default(), p.url.clone()));
            }
        }
    }
    Ok((needs, running, pulls))
}

/// Shows someone fctl's Home tab, as they open it.
pub async fn publish(
    _env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    slack_user: &str,
    now: i64,
) -> AppResult<()> {
    let connected = !super::connected(db, &slack.team).await?.is_empty();
    let home = match super::who(db, slack, slack_user, now).await? {
        Some(user) if connected => {
            let (needs_you, running, pulls) = lists(cfg, db, &user.id, now).await?;
            Home {
                name: Some(user.addressed()),
                link: None,
                needs_you,
                running,
                pulls,
                app_url: cfg.url("/"),
                can_start: true,
            }
        }
        _ if connected => Home {
            link: Some(super::inbound::link_url(cfg, &slack.team, "")),
            app_url: cfg.url("/"),
            ..Home::default()
        },
        _ => Home {
            app_url: cfg.url("/"),
            ..Home::default()
        },
    };
    slack.publish_home(slack_user, &blocks::home(&home)).await?;
    Ok(())
}

/// `/fctl status`: what needs them and what runs, only for them.
pub async fn status(
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    slack_user: &str,
    now: i64,
) -> AppResult<Value> {
    let Some(user) = super::who(db, slack, slack_user, now).await? else {
        let link = super::inbound::link_url(cfg, &slack.team, "");
        return Ok(json!({
            "response_type": "ephemeral", "text": "Link your Slack account to fctl to see your sessions.",
            "blocks": blocks::link_account(&link, "/fctl status lists your sessions"),
        }));
    };
    let (needs, running, _) = lists(cfg, db, &user.id, now).await?;
    let line = |h: &HomeSession| {
        format!(
            "• *{}* {}: {}",
            text::link(&h.url, &h.reference),
            text::escape(&text::clip(&h.title, 100)),
            text::escape(&h.status)
        )
    };
    let mut parts = Vec::new();
    parts.push(if needs.is_empty() {
        "*Needs you*\nNothing waits for you.".to_owned()
    } else {
        format!(
            "*Needs you*\n{}",
            needs
                .iter()
                .take(10)
                .map(line)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    parts.push(if running.is_empty() {
        "*Running*\nNo session of yours runs now.".to_owned()
    } else {
        format!(
            "*Running*\n{}",
            running
                .iter()
                .take(10)
                .map(line)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    Ok(json!({
        "response_type": "ephemeral",
        "text": format!("{} need you, {} running", needs.len(), running.len()),
        "blocks": [
            { "type": "section", "text": { "type": "mrkdwn", "verbatim": true, "text": parts.join("\n\n") } },
            { "type": "context", "elements": [{ "type": "mrkdwn", "text": format!("<{}|Open fctl>", cfg.url("/")) }] },
        ],
    }))
}

// ───────────────────────────── notifications ─────────────────────────────

/// Where a person's notifications about a session go in Slack: their
/// account in a Slack workspace the session's workspace works in, where
/// its admins let notifications go there.
#[derive(Deserialize)]
struct Target {
    team_id: String,
    slack_user_id: String,
}

async fn target(db: &D1Database, user: &str, session_id: &str) -> AppResult<Option<Target>> {
    #[derive(Deserialize)]
    struct Row {
        team_id: String,
        slack_user_id: String,
        settings: String,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT a.team_id, a.slack_user_id, ws.settings FROM slack_accounts a \
         JOIN workspace_slack ws ON ws.team_id = a.team_id AND ws.revoked_at IS NULL \
         JOIN projects p ON p.workspace_id = ws.workspace_id JOIN sessions s ON s.project_id = p.id \
         WHERE a.user_id = ? AND s.id = ? ORDER BY a.linked_at",
        &[s(user), s(session_id)],
    )?
    .all()
    .await?;
    Ok(rows
        .into_iter()
        .find(|r| {
            serde_json::from_str::<fctl_proto::slack::SlackSettings>(&r.settings)
                .unwrap_or_default()
                .notifications
        })
        .map(|r| Target {
            team_id: r.team_id,
            slack_user_id: r.slack_user_id,
        }))
}

/// Sends `user` their notifications in Slack, as NotifyDO gathered them:
/// one with the agent's question and its answers, or what happened, for a
/// single session; a digest for several. Each goes to the Slack workspace
/// of its session's workspace. Whether any went.
pub async fn notify(env: &Env, user: &str, events: &[NotifyEvent]) -> AppResult<bool> {
    let cfg = Config::from_env(env);
    if cfg.slack.is_none() || events.is_empty() {
        return Ok(false);
    }
    let db = env.d1("DB")?;
    // By Slack workspace: each gets its own message.
    let mut by_team: Vec<(Target, Vec<&NotifyEvent>)> = Vec::new();
    for e in events {
        let Some(t) = target(&db, user, e.session_id.as_str()).await? else {
            continue;
        };
        match by_team.iter_mut().find(|(x, _)| x.team_id == t.team_id) {
            Some((_, list)) => list.push(e),
            None => by_team.push((t, vec![e])),
        }
    }
    let keys = crate::crypto::keys(&cfg)?;
    let mut sent = false;
    for (t, list) in by_team {
        let Ok(slack) = super::client(&db, &cfg, &keys, &t.team_id).await else {
            continue;
        };
        let dm = match slack.open_dm(&t.slack_user_id).await {
            Ok(d) => d,
            Err(e) => {
                worker::console_error!("opening someone's messages with fctl: {e:?}");
                continue;
            }
        };
        let posted = match list.as_slice() {
            [one] => notify_one(&cfg, &db, &slack, &dm, &t.slack_user_id, one).await,
            many => {
                let lines: Vec<String> = many
                    .iter()
                    .take(10)
                    .map(|e| {
                        format!(
                            "• *{}* {}: {}",
                            text::link(
                                &cfg.url(&format!("/s/{}", e.session_id.as_str())),
                                &e.reference
                            ),
                            text::escape(&text::clip(&e.title, 100)),
                            words(e.kind)
                        )
                    })
                    .collect();
                let headline = format!(
                    ":bell: *{} sessions need you*\n{}",
                    many.len(),
                    lines.join("\n")
                );
                slack
                    .post(
                        &dm,
                        None,
                        &format!("{} sessions need you", many.len()),
                        Some(&json!([{ "type": "section", "text": { "type": "mrkdwn", "verbatim": true, "text": headline } }])),
                    )
                    .await
                    .map(|_| ())
            }
        };
        match posted {
            Ok(()) => sent = true,
            Err(e) => worker::console_error!("notifying in Slack: {e:?}"),
        }
    }
    Ok(sent)
}

fn words(kind: Kind) -> &'static str {
    match kind {
        Kind::Waiting => "waits for your reply",
        Kind::Asking | Kind::Autopilot => "asks you something",
        Kind::Paused => "paused on a usage limit",
        Kind::Failed => "failed",
        Kind::AutopilotPaused | Kind::AutopilotFailing => "needs a look",
    }
}

async fn notify_one(
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    dm: &str,
    slack_user: &str,
    e: &NotifyEvent,
) -> Result<(), super::Failure> {
    let url = e
        .url
        .clone()
        .unwrap_or_else(|| cfg.url(&format!("/s/{}", e.session_id.as_str())));
    let link = text::link(&url, &e.reference);
    let title = text::escape(&text::clip(&e.title, 120));
    let (emoji, verb) = match e.kind {
        Kind::Waiting => (":speech_balloon:", "waits for your reply"),
        Kind::Asking | Kind::Autopilot => (":raising_hand:", "asks you"),
        Kind::Paused => (":double_vertical_bar:", "paused on a Claude usage limit"),
        Kind::Failed => (":x:", "failed"),
        Kind::AutopilotPaused | Kind::AutopilotFailing => (":warning:", "needs a look"),
    };
    let headline = format!("{emoji} *{link}* {verb}\n{title}");
    // A question with its answers, which a click sends as the clicker.
    let question = if matches!(e.kind, Kind::Asking) {
        crate::spec::session_row(db, e.session_id.as_str())
            .await
            .ok()
            .flatten()
            .and_then(|r| r.question)
            .and_then(|j| serde_json::from_str::<SessionQuestion>(&j).ok())
    } else {
        None
    };
    let block_id = question
        .as_ref()
        .map(|q| super::outbound::question_block(e.session_id.as_str(), q.asked_at));
    let b = blocks::notification(
        &headline,
        e.detail.as_deref(),
        &url,
        question
            .as_ref()
            .zip(block_id.as_deref())
            .map(|(q, id)| (q.text.as_str(), q.options.as_slice(), id)),
    );
    let fallback = format!("{} {verb}: {}", e.reference, text::clip(&e.title, 120));
    let ts = slack.post(dm, None, &fallback, Some(&b)).await?;
    // A reply in its thread reaches the session.
    if let Err(err) = super::outbound::keep_notified(
        db,
        &slack.team,
        dm,
        &ts,
        e.session_id.as_str(),
        slack_user,
        crate::ctx::now_ms(),
    )
    .await
    {
        worker::console_error!("keeping a notification's thread: {}", err.0.message);
    }
    Ok(())
}

// ───────────────────────────── project events ─────────────────────────────

/// Posts a project's event to the channel it chose (`SlackProject::events`)
/// through fctl's app, where its workspace lets projects do that.
pub async fn project_event(
    env: &Env,
    db: &D1Database,
    e: &fctl_domain::hooks::Event,
) -> AppResult<()> {
    let Some(project_id) = e.project_id.as_deref() else {
        return Ok(());
    };
    #[derive(Deserialize)]
    struct Row {
        settings: String,
        workspace_id: Option<String>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT settings, workspace_id FROM projects WHERE id = ?",
        &[s(project_id)],
    )?
    .first()
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let slack_settings: SlackProject = serde_json::from_str::<Value>(&row.settings)
        .ok()
        .and_then(|v| v.get("slack").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let Some(events) = slack_settings.events else {
        return Ok(());
    };
    let kinds: Vec<String> = if events.kinds.is_empty() {
        fctl_proto::slack::SlackEvents::DEFAULT
            .iter()
            .map(|k| (*k).to_owned())
            .collect()
    } else {
        events.kinds.clone()
    };
    if !kinds.iter().any(|k| k == e.kind.as_str()) {
        return Ok(());
    }
    let workspace = row.workspace_id.unwrap_or_default();
    let allowed = super::connected(db, &events.team)
        .await?
        .into_iter()
        .any(|c| c.workspace_id == workspace && c.settings().project_events);
    if !allowed {
        return Ok(());
    }
    let (slack, _, cfg) = super::client_for(env, &events.team).await?;
    let url = e
        .data
        .pointer("/session/url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            e.session_id
                .as_ref()
                .map(|s| cfg.url(&format!("/s/{}", s.as_str())))
        })
        .unwrap_or_else(|| cfg.url("/"));
    let emoji = match e.kind {
        HookKind::SessionReadyForReview => ":eyes:",
        HookKind::SessionFailed => ":x:",
        HookKind::SessionQuestion => ":raising_hand:",
        HookKind::SessionFinished => ":speech_balloon:",
        HookKind::SessionPaused => ":double_vertical_bar:",
        HookKind::SessionStarted => ":arrow_forward:",
        HookKind::PullRequestOpened => ":arrow_heading_up:",
        HookKind::PullRequestMerged => ":large_purple_circle:",
        _ => ":bell:",
    };
    let gist = text::escape(&text::clip(&e.gist, 300));
    let detail = e
        .data
        .pointer("/session/title")
        .and_then(Value::as_str)
        .filter(|t| !e.gist.contains(*t));
    slack
        .post(
            &events.channel,
            None,
            &e.gist,
            Some(&blocks::event(emoji, &gist, detail, &url)),
        )
        .await?;
    Ok(())
}

//! What sessions that started in Slack post back there
//! (docs/architecture.md#slack): a card in the thread at once, kept
//! current in place as the session moves (how it stands, the agent's plan,
//! its pull requests with their checks); the last message of each turn as
//! a reply; the agent's question with a button per answer; a pause or a
//! failure in words; and, in someone's direct messages, the agent's words
//! as they come (a stream). Only replies, questions, pauses and failures
//! post new messages, so that the thread stays readable.

use super::Slack;
use super::inbound::{ThreadRow, thread_of};
use crate::config::Config;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult};
use fctl_domain::slack::blocks::{self, Card, Pull, Status, Task};
use fctl_domain::slack::text;
use fctl_proto::event::{Notice, NoticeLevel};
use fctl_proto::session::{Session, SessionQuestion, SessionStatus};
use fctl_proto::slack::{SlackNotice, SlackPlace};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use worker::{D1Database, Env};

/// A session that answers in a Slack thread: the thread, and fctl's bot
/// there.
struct Answering {
    thread: ThreadRow,
    slack: Slack,
    db: D1Database,
    cfg: Config,
}

async fn answering(env: &Env, session_id: &str) -> AppResult<Option<Answering>> {
    let db = env.d1("DB")?;
    let Some(thread) = thread_of(&db, session_id).await? else {
        return Ok(None);
    };
    let (slack, db, cfg) = super::client_for(env, &thread.team_id).await?;
    Ok(Some(Answering {
        thread,
        slack,
        db,
        cfg,
    }))
}

/// How a session stands, as its card says it, from its summary and what
/// SessionDO keeps of the agent's step and plan.
pub fn card_of(s: &Session, url: &str, step: Option<String>, plan: Vec<Task>) -> Card {
    let pulls: Vec<Pull> = s
        .pull_requests
        .iter()
        .map(|p| Pull {
            label: fctl_domain::repos::parse_remote(&p.url)
                .map(|r| format!("{}#{}", r.full_name(), p.number))
                .unwrap_or_else(|| format!("#{}", p.number)),
            url: p.url.clone(),
            title: p.title.clone(),
            state: p.state.clone(),
            checks: p.checks.as_ref().map(|c| (c.passed, c.failed, c.pending)),
        })
        .collect();
    let merged = !s.pull_requests.is_empty()
        && s.pull_requests
            .iter()
            .all(|p| matches!(p.state.as_deref(), Some("merged" | "closed")))
        && s.pull_requests
            .iter()
            .any(|p| p.state.as_deref() == Some("merged"));
    let status = match s.status {
        SessionStatus::Queued => Status::Queued,
        SessionStatus::Provisioning | SessionStatus::Preparing => Status::Starting,
        SessionStatus::Running => Status::Working { step },
        SessionStatus::Idle if s.question.is_some() => Status::Asking,
        SessionStatus::Idle | SessionStatus::Stopping | SessionStatus::Stopped if merged => {
            Status::Done
        }
        SessionStatus::Idle => Status::Waiting,
        SessionStatus::Paused => Status::Paused {
            until: s.resume_at,
            until_utc: s.resume_at.map(fctl_domain::slack::utc),
        },
        SessionStatus::Stopping | SessionStatus::Stopped => Status::Stopped,
        SessionStatus::Failed => Status::Failed {
            reason: s.status_reason.clone(),
        },
    };
    Card {
        title: s.title.clone(),
        url: url.to_owned(),
        reference: format!("{}#{}", s.project_slug, s.number),
        project: s
            .workspace
            .as_ref()
            .map(|w| w.name.clone())
            .unwrap_or_else(|| s.project_slug.clone()),
        by: s.created_by.name.clone(),
        status,
        plan,
        pulls,
    }
}

/// Posts or changes the card: Slack's `plan` block where it takes one, a
/// list where it refuses it.
async fn show_card(
    slack: &Slack,
    channel: &str,
    thread_ts: &str,
    card_ts: Option<&str>,
    card: &Card,
) -> Result<Option<String>, super::Failure> {
    let fallback = blocks::card_text(card);
    for plan_block in [true, false] {
        let b = blocks::card(card, plan_block);
        let r = match card_ts {
            Some(ts) => slack.update(channel, ts, &fallback, &b).await.map(|_| None),
            None => slack
                .post(channel, Some(thread_ts), &fallback, Some(&b))
                .await
                .map(Some),
        };
        match r {
            Err(super::Failure::Slack(code))
                if plan_block && !card.plan.is_empty() && code.starts_with("invalid_blocks") =>
            {
                continue;
            }
            other => return other,
        }
    }
    Ok(None)
}

/// A session started from a thread: Slack's agent session working, where
/// Slack shows one, and its card there at once, which SessionDO posts, as
/// it does every change of it after (`card`).
pub async fn started(
    env: &Env,
    slack: &Slack,
    db: &D1Database,
    thread_id: &str,
    session_id: &str,
    initiator: &str,
) {
    let Ok(Some(t)) = super::inbound::thread(db, thread_id).await else {
        return;
    };
    let title = crate::spec::session_row(db, session_id)
        .await
        .ok()
        .flatten()
        .map(|r| r.title)
        .unwrap_or_default();
    slack
        .set_status(
            &t.channel_id,
            &t.thread_ts,
            "processing",
            Some(&title),
            Some(initiator),
        )
        .await;
    let poked = async {
        let stub = crate::internal::session_stub(env, session_id)?;
        let _: Value = crate::internal::post(&stub, "/slack-card", &serde_json::json!({})).await?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(e) = poked {
        worker::console_error!("the first card of {session_id}: {}", e.0.message);
    }
}

/// Claims the posting of a thread's first card, so that two changes that
/// race post one: `false` when another is posting it (for less than half a
/// minute).
async fn claim_card(db: &D1Database, id: &str, now: i64) -> AppResult<bool> {
    let claimed = q(
        db,
        "UPDATE slack_threads SET card_ts = ?1 WHERE id = ?2 AND (card_ts IS NULL OR \
         (card_ts LIKE 'posting:%' AND CAST(substr(card_ts, 9) AS INTEGER) < ?3))",
        &[s(&format!("posting:{now}")), s(id), n(now - 30_000)],
    )?
    .run()
    .await?;
    Ok(claimed > 0)
}

/// The least time between two changes of a card that only the agent's
/// step changes: Slack takes about 50 changes a minute in a Slack
/// workspace, which every session there shares.
pub const STEP_EVERY_MS: i64 = 15_000;

/// Whether a card worth showing differs from what was shown, and soon
/// enough after it, for what only the agent's step changes.
pub fn due(shown: Option<&Card>, at: i64, card: &Card, now: i64) -> bool {
    match shown {
        None => true,
        Some(old) if old == card => false,
        Some(old) => {
            let steps = |st: &Status| match st {
                Status::Working { .. } => Status::Working { step: None },
                other => other.clone(),
            };
            let mut a = old.clone();
            let mut b = card.clone();
            a.status = steps(&a.status);
            b.status = steps(&b.status);
            a != b || now - at >= STEP_EVERY_MS
        }
    }
}

/// Changes the session's card to `card`, and Slack's agent session with
/// how it stands, from SessionDO as the session moves.
pub async fn card(env: &Env, session_id: &str, card: &Card, status_changed: bool) -> AppResult<()> {
    let Some(a) = answering(env, session_id).await? else {
        return Ok(());
    };
    let t = &a.thread;
    let now = crate::ctx::now_ms();
    let card_ts = t.card_ts.as_deref().filter(|c| !c.starts_with("posting:"));
    let first = card_ts.is_none();
    if first && !claim_card(&a.db, &t.id, now).await? {
        return Ok(());
    }
    let posted = show_card(&a.slack, &t.channel_id, &t.thread_ts, card_ts, card).await;
    let keep = |ts: Option<String>| {
        let db = &a.db;
        let id = t.id.clone();
        async move {
            q(
                db,
                "UPDATE slack_threads SET card_ts = ? WHERE id = ?",
                &[crate::db::os(ts.as_deref()), s(&id)],
            )?
            .run()
            .await
        }
    };
    match posted {
        Ok(Some(ts)) => {
            keep(Some(ts)).await?;
        }
        Ok(None) => {}
        // A card someone deleted: a new one, below.
        Err(super::Failure::Slack(c)) if c == "message_not_found" || c == "cant_update_message" => {
            let again = show_card(&a.slack, &t.channel_id, &t.thread_ts, None, card).await;
            keep(again.ok().flatten()).await?;
        }
        Err(e) => {
            if first {
                keep(None).await?;
            }
            return Err(e.into());
        }
    }
    if status_changed {
        let status = match &card.status {
            Status::Queued | Status::Starting | Status::Working { .. } => "processing",
            Status::Asking => "suspended",
            Status::Stopped | Status::Done => "closed",
            _ => "active",
        };
        a.slack
            .set_status(&t.channel_id, &t.thread_ts, status, None, None)
            .await;
    }
    Ok(())
}

/// A turn ended: the agent's last message goes to the thread as a reply
/// (unless it streamed there already), then its question with a button
/// per answer, where it asks one.
pub async fn turn_ended(
    env: &Env,
    session_id: &str,
    text: &str,
    question: Option<&SessionQuestion>,
) -> AppResult<()> {
    let Some(a) = answering(env, session_id).await? else {
        return Ok(());
    };
    let t = &a.thread;
    let url = a.cfg.url(&format!("/s/{session_id}"));
    let streamed = finish_stream(&a, text).await;
    if !streamed && !text.trim().is_empty() {
        let (b, fallback) = blocks::reply(text, &url);
        let posted = match a
            .slack
            .post(&t.channel_id, Some(&t.thread_ts), &fallback, Some(&b))
            .await
        {
            Err(super::Failure::Slack(c)) if c.starts_with("invalid_blocks") => {
                a.slack
                    .post(
                        &t.channel_id,
                        Some(&t.thread_ts),
                        &fallback,
                        Some(&blocks::reply_mrkdwn(text, &url)),
                    )
                    .await
            }
            other => other,
        };
        match posted {
            Ok(ts) => {
                let link = a.slack.link(&t.channel_id, &ts, Some(&t.thread_ts)).await;
                notice(
                    env,
                    session_id,
                    "slack_replied",
                    format!("Replied in {}", where_of(t, &a)),
                    SlackNotice {
                        place: t.place(),
                        channel_name: channel_name(&a.db, session_id).await,
                        body: Some(text::cut(text, text::MAX_REPLY, "…").0),
                        url: Some(link),
                        messages: None,
                    },
                )
                .await;
            }
            Err(e) => {
                worker::console_error!("replying in Slack for {session_id}: {e:?}");
            }
        }
    }
    if let Some(q) = question {
        ask(&a, session_id, q).await?;
    }
    Ok(())
}

/// The agent asked something after its turn ended: the question goes to
/// the thread at once.
pub async fn asked(env: &Env, session_id: &str, question: &SessionQuestion) -> AppResult<()> {
    let Some(a) = answering(env, session_id).await? else {
        return Ok(());
    };
    ask(&a, session_id, question).await
}

async fn ask(a: &Answering, session_id: &str, question: &SessionQuestion) -> AppResult<()> {
    let t = &a.thread;
    let block_id = question_block(session_id, question.asked_at);
    let (b, fallback) = blocks::question(&question.text, &question.options, &block_id);
    let ts = a
        .slack
        .post(&t.channel_id, Some(&t.thread_ts), &fallback, Some(&b))
        .await?;
    q(
        &a.db,
        "UPDATE slack_threads SET question_ts = ?, updated_at = ? WHERE id = ?",
        &[s(&ts), n(crate::ctx::now_ms()), s(&t.id)],
    )?
    .run()
    .await?;
    a.slack
        .set_status(&t.channel_id, &t.thread_ts, "suspended", None, None)
        .await;
    Ok(())
}

/// What names a question in the buttons that answer it: a click on an
/// older one answers nothing.
pub fn question_block(session_id: &str, asked_at: i64) -> String {
    format!("fctl_q:{session_id}:{asked_at}")
}

/// A session that answers in a Slack thread failed, or paused on a usage
/// limit: the thread says so, and when a paused one carries on, in each
/// reader's own time zone.
pub async fn halted(
    env: &Env,
    session_id: &str,
    status: SessionStatus,
    reason: Option<&str>,
    until: Option<i64>,
) -> AppResult<()> {
    let Some(a) = answering(env, session_id).await? else {
        return Ok(());
    };
    let t = &a.thread;
    let (emoji, words) = match status {
        SessionStatus::Failed => (
            ":x:",
            format!(
                "The fctl session failed{}. People who manage it can resume it in fctl.",
                reason
                    .map(|r| format!(": {}", text::escape(&text::clip(r, 400))))
                    .unwrap_or_default()
            ),
        ),
        SessionStatus::Paused => (
            ":double_vertical_bar:",
            match until {
                Some(at) => format!(
                    "The session pauses until {}: its Claude subscription reached a usage limit. It carries on by itself.",
                    blocks::local_time(at, &fctl_domain::slack::utc(at))
                ),
                None => "The session pauses: its Claude subscription reached a usage limit. It carries on by itself.".to_owned(),
            },
        ),
        _ => return Ok(()),
    };
    let fallback = words.replace("<!date^", "");
    a.slack
        .post(
            &t.channel_id,
            Some(&t.thread_ts),
            &fallback,
            Some(&blocks::note(emoji, &words)),
        )
        .await?;
    a.slack
        .set_status(
            &t.channel_id,
            &t.thread_ts,
            if status == SessionStatus::Failed {
                "closed"
            } else {
                "active"
            },
            None,
            None,
        )
        .await;
    Ok(())
}

// ───────────────────────────── streams ─────────────────────────────

/// A stream of the agent's words into a thread of someone's direct
/// messages with the app (`chat.startStream`): its message, when it
/// started, and how much it holds. Slack ends one after ten minutes or
/// 12,000 characters, after which the turn's last message is posted as a
/// reply instead.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Stream {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
    #[serde(default)]
    started_at: i64,
    #[serde(default)]
    chars: usize,
    /// The words it streamed this turn, so that the turn's end adds only
    /// what's new.
    #[serde(default)]
    text: String,
    /// Ended by Slack or by its limits: the rest goes as a reply.
    #[serde(default)]
    over: bool,
    /// The event's token for Slack's Real-time Search, kept with the
    /// thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    action_token: Option<String>,
}

const STREAM_MS: i64 = 9 * 60 * 1000;
const STREAM_CHARS: usize = 11_000;

async fn save_stream(db: &D1Database, id: &str, st: &Stream) -> AppResult<()> {
    q(
        db,
        "UPDATE slack_threads SET stream = ? WHERE id = ?",
        &[s(&serde_json::to_string(st)?), s(id)],
    )?
    .run()
    .await?;
    Ok(())
}

fn stream_of(t: &ThreadRow) -> Stream {
    t.stream
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default()
}

/// The agent wrote something in its turn: in someone's direct messages,
/// it streams there as it comes.
pub async fn stream(env: &Env, session_id: &str, words: &str) -> AppResult<()> {
    let Some(a) = answering(env, session_id).await? else {
        return Ok(());
    };
    let t = &a.thread;
    if t.place() != SlackPlace::Dm || words.trim().is_empty() {
        return Ok(());
    }
    let mut st = stream_of(t);
    if st.over {
        return Ok(());
    }
    let now = crate::ctx::now_ms();
    let chunk = format!(
        "{}{}",
        if st.text.is_empty() { "" } else { "\n\n" },
        text::defuse(words.trim())
    );
    if st.chars + chunk.chars().count() > STREAM_CHARS
        || st.ts.is_some() && now - st.started_at > STREAM_MS
    {
        if let Some(ts) = st.ts.as_deref() {
            let _ = a
                .slack
                .stop_stream(&t.channel_id, ts, None, "processing")
                .await;
        }
        st.over = true;
        return save_stream(&a.db, &t.id, &st).await;
    }
    let r = match st.ts.as_deref() {
        None => a
            .slack
            .start_stream(&t.channel_id, &t.thread_ts, &chunk)
            .await
            .map(|ts| {
                st.ts = Some(ts);
                st.started_at = now;
            }),
        Some(ts) => a.slack.append_stream(&t.channel_id, ts, &chunk).await,
    };
    match r {
        Ok(()) => {
            st.chars += chunk.chars().count();
            st.text.push_str(&chunk);
        }
        // Stopped by its person, ended by Slack, or not taken here: the
        // turn's end posts its reply.
        Err(e) => {
            worker::console_log!("streaming into Slack for {session_id}: {e:?}");
            st.over = true;
        }
    }
    save_stream(&a.db, &t.id, &st).await
}

/// The turn ended: its stream ends with what's left of the last message.
/// Whether the stream said it all, so that no reply follows.
async fn finish_stream(a: &Answering, last: &str) -> bool {
    let t = &a.thread;
    let st = stream_of(t);
    let Some(ts) = st.ts.as_deref() else {
        return false;
    };
    let said = !st.over && st.text.contains(text::defuse(last.trim()).as_str());
    let _ = a.slack.stop_stream(&t.channel_id, ts, None, "active").await;
    let next = Stream {
        action_token: st.action_token.clone(),
        ..Stream::default()
    };
    if let Err(e) = save_stream(&a.db, &t.id, &next).await {
        worker::console_error!("ending a stream: {}", e.0.message);
    }
    said
}

/// The action token the event that started a thread carried, for Slack's
/// Real-time Search.
pub(crate) fn action_token(t: &ThreadRow) -> Option<String> {
    stream_of(t).action_token
}

// ───────────────────────────── the transcript ─────────────────────────────

/// Where a thread is, in words: `#web-team`, or "your messages with fctl".
fn where_of(t: &ThreadRow, a: &Answering) -> String {
    match t.place() {
        SlackPlace::Dm => "their messages with fctl in Slack".to_owned(),
        _ => format!("the thread in {}", a.slack.team_name),
    }
}

async fn channel_name(db: &D1Database, session_id: &str) -> Option<String> {
    crate::spec::session_row(db, session_id)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.slack)
        .and_then(|j| serde_json::from_str::<fctl_proto::slack::SessionSlack>(&j).ok())
        .and_then(|s| s.channel_name)
}

/// Adds a notice to a session's transcript, saying what fctl or the agent
/// did in Slack for it.
pub async fn notice(env: &Env, session_id: &str, code: &str, message: String, slack: SlackNotice) {
    let mut n = Notice::new(NoticeLevel::Info, code, message);
    n.slack = Some(slack);
    let sent = async {
        let stub = crate::internal::session_stub(env, session_id)?;
        let _: Value = crate::internal::post(&stub, "/notice", &n).await?;
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(e) = sent {
        worker::console_error!("a Slack notice for {session_id}: {}", e.0.message);
    }
}

/// Keeps a notification's thread in someone's direct messages with the
/// app, so that a reply there reaches the session it is about.
pub async fn keep_notified(
    db: &D1Database,
    team: &str,
    channel: &str,
    ts: &str,
    session_id: &str,
    user: &str,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "INSERT INTO slack_threads (id, team_id, channel_id, thread_ts, place, session_id, slack_user_id, state, updates, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'dm', ?, ?, 'started', 0, ?, ?) ON CONFLICT (id) DO NOTHING",
        &[
            s(&fctl_domain::slack::thread_id(team, channel, ts)),
            s(team),
            s(channel),
            s(ts),
            s(session_id),
            os(Some(user)),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

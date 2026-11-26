//! What Slack sends fctl's app (docs/architecture.md#slack), over HTTP
//! (`api::slack`) or Socket Mode (`slack::socket`), and the sessions it
//! starts and steers.
//!
//! - A mention of the app in a channel, a message in someone's direct
//!   messages with it, `/fctl` and its shortcut ask for a session, as the
//!   person who wrote them, once fctl knows who they are there
//!   (`slack_accounts`): in the channel's project where its admins mapped
//!   one, else the only project they can start sessions in, else the one
//!   they choose in a message only they see.
//! - Each thread reaches one session (`slack_threads`): a reply there is
//!   its author's message to the agent when they may message the session,
//!   and stays information for the agent otherwise. A click on one of the
//!   agent's answers answers as the clicker.
//! - Someone fctl doesn't know yet is asked to link their Slack account,
//!   only they see it, and their request waits an hour.

use super::{Slack, outbound};
use crate::config::Config;
use crate::crypto;
use crate::db::{n, os, q, s};
use crate::error::{AppError, AppResult};
use crate::internal::{Deliver, Delivered};
use crate::model::{PROJECT_SELECT, ProjectRow, UserRow};
use fctl_domain::slack::blocks::{self, Choice, act, callback};
use fctl_domain::slack::task::{self, Earlier, How};
use fctl_domain::slack::{
    self as sl, Action, Event, EventCallback, Interaction, MessageEvent, text,
};
use fctl_proto::error::ErrorCode;
use fctl_proto::event::{EventBody, MessageSource, UserMessage};
use fctl_proto::session::UserRef;
use fctl_proto::slack::{
    SessionSlack, SlackEvent, SlackMessage, SlackPerson, SlackPlace, SlackSettings,
};
use fctl_proto::uplink::{Delivery, DeliveryKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use worker::{D1Database, Env};

// ───────────────────────────── threads ─────────────────────────────

/// A thread that reaches a session, or asked for one, as D1 keeps it.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ThreadRow {
    pub id: String,
    pub team_id: String,
    pub channel_id: String,
    pub thread_ts: String,
    pub place: String,
    pub session_id: Option<String>,
    pub slack_user_id: Option<String>,
    pub state: String,
    pub waiting_for: Option<String>,
    pub payload: Option<String>,
    pub card_ts: Option<String>,
    pub read_ts: Option<String>,
    pub delivered: Option<String>,
    pub stream: Option<String>,
}

impl ThreadRow {
    pub fn place(&self) -> SlackPlace {
        SlackPlace::parse(&self.place).unwrap_or(SlackPlace::Other)
    }

    /// The messages fctl passed on to the agent as people's, by `ts`.
    pub fn delivered(&self) -> Vec<String> {
        self.delivered
            .as_deref()
            .and_then(|d| serde_json::from_str(d).ok())
            .unwrap_or_default()
    }
}

pub(crate) const THREAD_SELECT: &str = "SELECT id, team_id, channel_id, thread_ts, place, session_id, slack_user_id, state, \
    waiting_for, payload, card_ts, question_ts, read_ts, delivered, updates, stream FROM slack_threads";

pub(crate) async fn thread(db: &D1Database, id: &str) -> AppResult<Option<ThreadRow>> {
    q(db, &format!("{THREAD_SELECT} WHERE id = ?"), &[s(id)])?
        .first()
        .await
}

/// The thread a session answers in: the newest that reached it and takes
/// its updates, while its project's workspace still works in that Slack
/// workspace.
pub(crate) async fn thread_of(db: &D1Database, session_id: &str) -> AppResult<Option<ThreadRow>> {
    q(
        db,
        &format!(
            "{THREAD_SELECT} t WHERE t.session_id = ?1 AND t.state = 'started' AND t.updates = 1 AND EXISTS (SELECT 1 FROM sessions s \
             JOIN projects p ON p.id = s.project_id JOIN workspace_slack ws ON ws.workspace_id = p.workspace_id \
             WHERE s.id = ?1 AND ws.team_id = t.team_id AND ws.revoked_at IS NULL) ORDER BY t.updated_at DESC LIMIT 1"
        ),
        &[s(session_id)],
    )?
    .first()
    .await
}

async fn set_state(
    db: &D1Database,
    id: &str,
    state: &str,
    session_id: Option<&str>,
    now: i64,
) -> AppResult<()> {
    q(
        db,
        "UPDATE slack_threads SET state = ?, session_id = COALESCE(?, session_id), waiting_for = NULL, payload = NULL, \
         expires_at = NULL, updated_at = ? WHERE id = ?",
        &[s(state), os(session_id), n(now), s(id)],
    )?
    .run()
    .await?;
    Ok(())
}

/// Keeps a request for a thread until its person may act, for an hour.
async fn wait(db: &D1Database, id: &str, waiting_for: &str, ask: &Ask, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE slack_threads SET state = 'waiting', waiting_for = ?, payload = ?, expires_at = ?, updated_at = ? WHERE id = ?",
        &[
            s(waiting_for),
            s(&serde_json::to_string(ask)?),
            n(now + sl::WAIT_MS),
            n(now),
            s(id),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Notes that fctl passed message `ts` on to the agent, so that `fctl
/// slack thread` tells its author's words from others': the newest 200.
pub(crate) async fn delivered(db: &D1Database, t: &ThreadRow, ts: &str, now: i64) -> AppResult<()> {
    let mut all = t.delivered();
    if !all.iter().any(|d| d == ts) {
        all.push(ts.to_owned());
    }
    let skip = all.len().saturating_sub(200);
    let kept: Vec<&String> = all.iter().skip(skip).collect();
    q(
        db,
        "UPDATE slack_threads SET delivered = ?, updated_at = ? WHERE id = ?",
        &[s(&serde_json::to_string(&kept)?), n(now), s(&t.id)],
    )?
    .run()
    .await?;
    Ok(())
}

// ───────────────────────────── a request ─────────────────────────────

/// Someone asking for a session: what fctl keeps of it while it waits for
/// them (to link their account, or choose a project), as little as it
/// needs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Ask {
    pub team: String,
    pub channel: String,
    #[serde(default)]
    pub channel_name: Option<String>,
    pub place: SlackPlace,
    /// The thread it goes on in, and the message that asked.
    pub thread_ts: String,
    pub message_ts: String,
    pub user: String,
    /// What they asked, as Slack wrote it.
    pub text: String,
    pub how: AskHow,
    /// The message sent with the shortcut: its conversation, its `ts`, its
    /// thread's, its author and words.
    #[serde(default)]
    pub quoted: Option<Quoted>,
    /// The event's action token, for Slack's Real-time Search.
    #[serde(default)]
    pub action_token: Option<String>,
    /// fctl posts the session's card and replies in the thread; otherwise
    /// in the person's direct messages with the app.
    #[serde(default = "yes")]
    pub updates: bool,
    /// The project they chose in a form.
    #[serde(default)]
    pub project: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AskHow {
    Mentioned,
    Direct,
    Shortcut,
    Command,
}

impl AskHow {
    fn how(self) -> How {
        match self {
            Self::Mentioned => How::Mentioned,
            Self::Direct => How::Direct,
            Self::Shortcut => How::Shortcut,
            Self::Command => How::Command,
        }
    }

    fn event(self) -> SlackEvent {
        match self {
            Self::Mentioned => SlackEvent::Mentioned,
            Self::Direct => SlackEvent::Direct,
            Self::Shortcut => SlackEvent::Shortcut,
            Self::Command => SlackEvent::Command,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Quoted {
    pub channel: String,
    pub ts: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    pub text: String,
}

impl Ask {
    fn thread_id(&self) -> String {
        sl::thread_id(&self.team, &self.channel, &self.thread_ts)
    }
}

// ───────────────────────────── events ─────────────────────────────

/// An event from the Events API or Socket Mode.
pub async fn event(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    e: &EventCallback,
    raw: &Value,
    now: i64,
) -> AppResult<()> {
    if e.enterprise_install {
        worker::console_log!(
            "Slack event from an org-wide install of {}: ignored",
            e.team_id
        );
        return Ok(());
    }
    match &e.event {
        Event::AppUninstalled => {
            worker::console_log!("fctl's Slack app was uninstalled from {}", e.team_id);
            return super::mark_revoked(db, &e.team_id, now).await;
        }
        Event::TokensRevoked { bots } => {
            if let Some(row) = super::installation(db, &e.team_id).await?
                && bots.contains(&row.bot_user_id)
            {
                worker::console_log!("the bot token of {} was revoked", e.team_id);
                return super::mark_revoked(db, &e.team_id, now).await;
            }
            return Ok(());
        }
        Event::Other => return Ok(()),
        _ => {}
    }
    let keys = crypto::keys(cfg)?;
    let slack = match super::client(db, cfg, &keys, &e.team_id).await {
        Ok(s) => s,
        // Not installed (any more): nothing to do.
        Err(_) => return Ok(()),
    };
    let token = raw
        .pointer("/event/action_token")
        .and_then(Value::as_str)
        .map(str::to_owned);
    match &e.event {
        Event::AppMention(m) => {
            if e.shared {
                refuse_shared(&slack, m).await;
                return Ok(());
            }
            mention(env, cfg, db, &slack, m, token, now).await
        }
        Event::Message(m) => {
            if !m.by_person() || e.shared {
                return Ok(());
            }
            message(env, cfg, db, &slack, m, token, now).await
        }
        Event::AppHomeOpened { user, tab } if tab == "home" => {
            super::home::publish(env, cfg, db, &slack, user, now).await
        }
        Event::AgentSessionStopped {
            user,
            channel,
            thread_ts,
        } => stopped(env, db, &slack, channel, thread_ts, user.as_deref(), now).await,
        _ => Ok(()),
    }
}

async fn refuse_shared(slack: &Slack, m: &MessageEvent) {
    if let Some(user) = &m.user {
        let _ = slack
            .ephemeral(
                &m.channel,
                user,
                Some(m.thread()),
                "fctl doesn't work in channels shared with other organizations yet.",
                &blocks::only_you(
                    ":no_entry_sign:",
                    "fctl doesn't work in channels shared with other organizations (Slack Connect) yet: ask it in one of your own channels, or in your messages with it.",
                ),
            )
            .await;
    }
}

/// Someone mentioned fctl's app in a channel: a new request, or a message
/// to the session the thread reaches.
async fn mention(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    m: &MessageEvent,
    action_token: Option<String>,
    now: i64,
) -> AppResult<()> {
    let Some(user) = m.user.clone() else {
        return Ok(());
    };
    let id = sl::thread_id(&slack.team, &m.channel, m.thread());
    if let Some(t) = thread(db, &id).await? {
        return in_thread(env, cfg, db, slack, &t, m, now).await;
    }
    let ask = Ask {
        team: slack.team.clone(),
        channel: m.channel.clone(),
        channel_name: None,
        place: SlackPlace::Channel,
        thread_ts: m.thread().to_owned(),
        message_ts: m.ts.clone(),
        user,
        text: m.text.clone(),
        how: AskHow::Mentioned,
        quoted: None,
        action_token,
        updates: true,
        project: None,
    };
    begin(env, cfg, db, slack, ask, now).await
}

/// A message in a conversation fctl's app is in: someone's direct messages
/// with it (a new request, or a reply), or a reply in a thread that reaches
/// a session. Mentions come as their own event.
async fn message(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    m: &MessageEvent,
    action_token: Option<String>,
    now: i64,
) -> AppResult<()> {
    let Some(user) = m.user.clone() else {
        return Ok(());
    };
    if !m.in_dm() && (m.mentions(&slack.bot_user) || !m.is_reply()) {
        return Ok(());
    }
    let id = sl::thread_id(&slack.team, &m.channel, m.thread());
    if let Some(t) = thread(db, &id).await? {
        return in_thread(env, cfg, db, slack, &t, m, now).await;
    }
    if !m.in_dm() {
        // A reply in a thread fctl has nothing to do with.
        return Ok(());
    }
    let ask = Ask {
        team: slack.team.clone(),
        channel: m.channel.clone(),
        channel_name: None,
        place: SlackPlace::Dm,
        thread_ts: m.thread().to_owned(),
        message_ts: m.ts.clone(),
        user,
        text: m.text.clone(),
        how: AskHow::Direct,
        quoted: None,
        action_token,
        updates: true,
        project: None,
    };
    begin(env, cfg, db, slack, ask, now).await
}

/// A message in a thread fctl knows: to its session once it started, or
/// moving on what waits there, from whoever asked.
async fn in_thread(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    t: &ThreadRow,
    m: &MessageEvent,
    now: i64,
) -> AppResult<()> {
    let Some(author) = m.user.as_deref() else {
        return Ok(());
    };
    match (t.state.as_str(), t.session_id.as_deref()) {
        ("started", Some(session_id)) => reply(env, cfg, db, slack, t, session_id, m, now).await,
        // Before its session starts, only whoever asked moves it on: anyone
        // else's message would start it, or choose its project, on their
        // subscription. Theirs takes it up again, as things may have
        // changed (they linked their account meanwhile, say).
        _ if t.slack_user_id.as_deref() == Some(author) => {
            let kept = t
                .payload
                .as_deref()
                .and_then(|p| serde_json::from_str::<Ask>(p).ok());
            let ask = match kept {
                Some(mut k) => {
                    if !text::without_mention(&m.text, &slack.bot_user).is_empty() {
                        k.text = format!("{}\n\n{}", k.text, m.text);
                    }
                    k
                }
                None => Ask {
                    team: slack.team.clone(),
                    channel: m.channel.clone(),
                    channel_name: None,
                    place: if m.in_dm() {
                        SlackPlace::Dm
                    } else {
                        SlackPlace::Channel
                    },
                    thread_ts: t.thread_ts.clone(),
                    message_ts: m.ts.clone(),
                    user: author.to_owned(),
                    text: m.text.clone(),
                    how: if m.in_dm() {
                        AskHow::Direct
                    } else {
                        AskHow::Mentioned
                    },
                    quoted: None,
                    action_token: None,
                    updates: true,
                    project: None,
                },
            };
            take(env, cfg, db, slack, &ask, None, now).await
        }
        _ if m.mentions(&slack.bot_user) => {
            let _ = slack
                .ephemeral(
                    &m.channel,
                    author,
                    Some(m.thread()),
                    "Only the person who asked can start this one.",
                    &blocks::only_you(
                        ":hand:",
                        "Only the person who asked in this thread can start its session. To have fctl work on something as you, mention it in a new thread.",
                    ),
                )
                .await;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// A new request: its thread, kept as new, and seen at once (a reaction),
/// then taken up.
async fn begin(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    ask: Ask,
    now: i64,
) -> AppResult<()> {
    let id = ask.thread_id();
    let inserted = q(
        db,
        "INSERT INTO slack_threads (id, team_id, channel_id, thread_ts, place, slack_user_id, state, updates, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 'new', ?, ?, ?) ON CONFLICT (id) DO NOTHING",
        &[
            s(&id),
            s(&ask.team),
            s(&ask.channel),
            s(&ask.thread_ts),
            s(ask.place.as_str()),
            s(&ask.user),
            n(i64::from(ask.updates)),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    if inserted == 0 {
        // Slack sent it again.
        return Ok(());
    }
    take(env, cfg, db, slack, &ask, None, now).await
}

// ───────────────────────────── taking a request up ─────────────────────────────

/// Says something about a request where only its person sees it: in a
/// channel, a message only they see; in their direct messages, a message
/// in the thread, which only they see anyway.
async fn tell(slack: &Slack, ask: &Ask, words: &str, blocks: &Value) {
    let posted = if ask.place == SlackPlace::Dm {
        slack
            .post(&ask.channel, Some(&ask.thread_ts), words, Some(blocks))
            .await
            .map(|_| ())
    } else {
        slack
            .ephemeral(&ask.channel, &ask.user, Some(&ask.thread_ts), words, blocks)
            .await
    };
    if let Err(e) = posted {
        worker::console_error!("telling {} in Slack: {e:?}", ask.user);
    }
}

/// Takes a request up, or says why it can't: refused, the thread keeps it
/// for an hour, so that a reply there tries again.
async fn take(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    ask: &Ask,
    chosen: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let Err(err) = take_up(env, cfg, db, slack, ask, chosen, now).await else {
        return Ok(());
    };
    let id = ask.thread_id();
    if thread(db, &id)
        .await?
        .is_some_and(|t| t.state == "started" || t.state == "waiting")
    {
        return Err(err);
    }
    // fctl's own errors say nothing people can act on.
    let why = (err.0.code != ErrorCode::Internal).then_some(err.0.message.as_str());
    wait(db, &id, "retry", ask, now).await?;
    let words = task::couldnt_start(why);
    tell(
        slack,
        ask,
        &words,
        &blocks::only_you(":warning:", &text::escape(&words)),
    )
    .await;
    Err(err)
}

/// A project someone may start a request's session in.
struct Candidate {
    row: ProjectRow,
    /// The caller administers its workspace: they may remember it for the
    /// channel.
    admin: bool,
}

/// A candidate, as tests see it.
#[cfg(all(test, target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub(crate) struct CandidateView {
    pub id: String,
    pub admin: bool,
}

#[cfg(all(test, target_arch = "wasm32"))]
pub(crate) async fn candidate_views(
    db: &D1Database,
    slack: &Slack,
    user: &UserRow,
    ask: &Ask,
) -> AppResult<Vec<CandidateView>> {
    Ok(candidates(db, slack, user, ask)
        .await?
        .into_iter()
        .map(|c| CandidateView {
            id: c.row.id,
            admin: c.admin,
        })
        .collect())
}

/// Takes a request to a session: as its person, in the project the
/// channel maps to, the only one they may use, or the one they chose.
async fn take_up(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    ask: &Ask,
    chosen: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let id = ask.thread_id();
    let Some(user) = super::who(db, slack, &ask.user, now).await? else {
        wait(db, &id, "link", ask, now).await?;
        let link = link_url(cfg, &slack.team, &id);
        tell(
            slack,
            ask,
            "Link your Slack account to fctl, and fctl starts on this as you.",
            &blocks::link_account(&link, "fctl starts on this at once"),
        )
        .await;
        return Ok(());
    };
    let refused = id.clone();
    let refuse = |why: String| async move {
        set_state(db, &refused, "refused", None, now).await?;
        tell(
            slack,
            ask,
            &why,
            &blocks::only_you(":no_entry_sign:", &text::escape(&why)),
        )
        .await;
        Ok::<(), AppError>(())
    };
    // Slack keeps its guests from agent apps; so does fctl.
    if let Ok(who) = slack.user(&ask.user).await
        && (who.guest || who.bot)
    {
        return refuse(
            "Slack's guests can't use fctl's agents here: ask a member of this Slack workspace."
                .into(),
        )
        .await;
    }
    // The channel's name and kind, for the session and what fctl says.
    let mut ask = ask.clone();
    if ask.place != SlackPlace::Dm {
        match slack.conversation(&ask.channel).await {
            Ok(c) if c.shared => {
                return refuse("fctl doesn't work in channels shared with other organizations (Slack Connect) yet.".into()).await;
            }
            Ok(c) => {
                ask.place = if c.place == SlackPlace::Dm {
                    SlackPlace::Dm
                } else if c.place == SlackPlace::Channel {
                    SlackPlace::Channel
                } else {
                    SlackPlace::Private
                };
                ask.channel_name = c.name.or(ask.channel_name.take());
            }
            Err(e) => worker::console_error!("Slack's conversations.info: {e:?}"),
        }
    }
    let candidates = candidates(db, slack, &user, &ask).await?;
    let usable: Vec<&Candidate> = candidates.iter().collect();
    if usable.is_empty() {
        return refuse(nothing_to_start(
            &user,
            &ask,
            &mapped_elsewhere(db, slack, &user, &ask).await?,
        ))
        .await;
    }
    let wanted = chosen.or(ask.project.as_deref());
    let pick = match wanted {
        Some(p) => usable
            .iter()
            .copied()
            .find(|c| c.row.id == p || c.row.slug.eq_ignore_ascii_case(p)),
        None => None,
    };
    let candidate = match (pick, usable.as_slice()) {
        (Some(c), _) => c,
        (None, [only]) => *only,
        (None, _) => {
            // Their own choice for their direct messages, kept.
            let kept = if ask.place == SlackPlace::Dm {
                dm_project(db, &user.id, &slack.team).await?
            } else {
                None
            };
            match kept.and_then(|k| usable.iter().copied().find(|c| c.row.id == k)) {
                Some(c) => c,
                None => {
                    wait(db, &id, "project", &ask, now).await?;
                    let several_workspaces = usable
                        .iter()
                        .map(|c| c.row.workspace_id().to_owned())
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                        > 1;
                    let choices: Vec<Choice> = usable
                        .iter()
                        .map(|c| Choice {
                            id: c.row.id.clone(),
                            name: c.row.name.clone(),
                            workspace: several_workspaces.then(|| {
                                c.row.workspace.workspace_name.clone().unwrap_or_default()
                            }),
                        })
                        .collect();
                    let remember = (ask.place != SlackPlace::Dm && usable.iter().any(|c| c.admin))
                        .then(|| ask.channel_name.clone())
                        .flatten();
                    tell(
                        slack,
                        &ask,
                        "Which project should work on this?",
                        &blocks::choose_project(&choices, remember.as_deref(), &id),
                    )
                    .await;
                    return Ok(());
                }
            }
        }
    };
    let project = &candidate.row;
    let Some(credential) =
        crate::api::sessions::credential_for(db, &user, project.workspace_id()).await?
    else {
        return refuse(format!(
            "{}, connect your Claude subscription in fctl first (Settings → Claude): sessions run on it. Then ask again.",
            user.addressed()
        ))
        .await;
    };
    let github_access =
        fctl_domain::limits::session_github(None, project.settings().github_access, None)
            .map_err(AppError)?;
    // What they asked, as text, and what the thread said before.
    let names = names(slack, &ask).await;
    let name_of = |id: &str| names.get(id).cloned();
    let request = text::plain(&text::without_mention(&ask.text, &slack.bot_user), &name_of);
    let (earlier, more, read) = context(slack, &ask, &names).await;
    let quoted = ask.quoted.as_ref().map(|q| Earlier {
        by: q
            .user
            .as_deref()
            .and_then(|u| names.get(u).cloned())
            .unwrap_or_else(|| "someone".into()),
        at: utc(sl::ts_millis(&q.ts).unwrap_or(now)),
        text: text::plain(&q.text, &name_of),
    });
    let url = slack
        .link(&ask.channel, &ask.message_ts, Some(&ask.thread_ts))
        .await;
    let thread_url = slack.link(&ask.channel, &ask.thread_ts, None).await;
    let mut prompt = task::task(&task::Asked {
        by: &user.name,
        how: ask.how.how(),
        channel: ask
            .channel_name
            .as_deref()
            .filter(|_| ask.place != SlackPlace::Dm),
        url: &url,
        request: &request,
        quoted: quoted.as_ref(),
        earlier: &earlier,
        more,
    });
    if !read {
        prompt.push_str("\n\n[fctl couldn't read the thread's earlier messages just now: `fctl slack thread --all` reads them.]");
    }
    let first_line = if request.trim().is_empty() {
        quoted
            .as_ref()
            .map(|q| q.text.clone())
            .unwrap_or_else(|| "A request from Slack".into())
    } else {
        request.clone()
    };
    let title =
        fctl_domain::validate::session_title(&fctl_domain::naming::title_from_prompt(&first_line))
            .unwrap_or_else(|_| "A request from Slack".into());
    let session_slack = SessionSlack {
        team: slack.team.clone(),
        team_name: Some(slack.team_name.clone()).filter(|n| !n.is_empty()),
        channel: ask.channel.clone(),
        channel_name: ask
            .channel_name
            .clone()
            .filter(|_| ask.place != SlackPlace::Dm),
        place: ask.place,
        thread_ts: ask.thread_ts.clone(),
        url: thread_url,
    };
    let by = person_of(slack, &ask.user, &user).await;
    let quoted_by = ask
        .quoted
        .as_ref()
        .and(quoted.as_ref())
        .map(|q| SlackPerson {
            name: q.by.clone(),
            user: ask.quoted.as_ref().and_then(|q| q.user.clone()),
            handle: None,
            avatar_url: None,
            bot: false,
        });
    let (session_id, number) = crate::starts::start(
        env,
        db,
        crate::starts::NewSession {
            project,
            title,
            branch: crate::starts::Branch::Titled,
            base_branch: project.default_branch.clone(),
            model: None,
            effort: None,
            user: &user,
            credential: &credential,
            github_access,
            prompt,
            author: Some(user.to_ref()),
            source: MessageSource::Api,
            ticket_message: None,
            schedule_id: None,
            ticket: None,
            slack: Some(session_slack.clone()),
            slack_message: Some(SlackMessage {
                event: ask.how.event(),
                by: Some(by),
                place: ask.place,
                channel_name: session_slack.channel_name.clone(),
                team_name: session_slack.team_name.clone(),
                body: Some(request.clone()).filter(|r| !r.trim().is_empty()),
                url: Some(url.clone()),
                context: (!earlier.is_empty()).then_some(earlier.len() as u32),
                quoted: quoted_by,
                dropped: false,
            }),
            autopilot: None,
            follow: true,
            service_account: None,
            now,
        },
    )
    .await?;
    set_state(db, &id, "started", Some(session_id.as_str()), now).await?;
    // The message that asked counts as passed on.
    if let Some(t) = thread(db, &id).await? {
        delivered(db, &t, &ask.message_ts, now).await?;
    }
    if let Some(token) = ask.action_token.as_deref() {
        let _ = q(
            db,
            "UPDATE slack_threads SET stream = json_set(COALESCE(stream, '{}'), '$.action_token', ?) WHERE id = ?",
            &[s(token), s(&id)],
        )?
        .run()
        .await;
    }
    if let Err(e) = crate::auth::mark_active(db, &user.id, now).await {
        worker::console_error!("noting when a person was last active: {}", e.0.message);
    }
    audit(
        db,
        project.workspace_id(),
        &user.id,
        session_id.as_str(),
        json!({
            "project": project.slug,
            "number": number,
            "github_access": github_access,
            "slack": { "team": slack.team, "channel": ask.channel, "thread": ask.thread_ts },
        }),
        now,
    )
    .await;
    slack.react(&ask.channel, &ask.message_ts, "eyes").await;
    outbound::started(env, slack, db, &id, session_id.as_str(), &ask.user).await;
    Ok(())
}

/// Why nothing can start for them, in words.
fn nothing_to_start(user: &UserRow, ask: &Ask, mapped: &[String]) -> String {
    match mapped {
        [] => format!(
            "{}, there's no fctl project here you can start sessions in: a workspace connected to this Slack workspace makes you an editor of one, and lets {} start sessions.",
            user.addressed(),
            if ask.place == SlackPlace::Dm {
                "direct messages"
            } else {
                "mentions in channels"
            }
        ),
        names => format!(
            "{}, mentions here start sessions in {}, which you can't start sessions in: its owners make you an editor of it.",
            user.addressed(),
            names.join(" or ")
        ),
    }
}

/// The projects a request may start a session in: of fctl workspaces
/// connected to the Slack workspace that let such requests start
/// sessions, not archived, that the person may start sessions in; only the
/// channel's own, where its admins mapped one.
async fn candidates(
    db: &D1Database,
    slack: &Slack,
    user: &UserRow,
    ask: &Ask,
) -> AppResult<Vec<Candidate>> {
    let settings_of: BTreeMap<String, SlackSettings> = super::connected(db, &slack.team)
        .await?
        .into_iter()
        .map(|c| {
            let st = c.settings();
            (c.workspace_id, st)
        })
        .collect();
    let allows = |w: &str| {
        settings_of.get(w).is_some_and(|st| {
            if ask.place == SlackPlace::Dm {
                st.direct_messages
            } else {
                st.channels
            }
        })
    };
    let workspaces: Vec<&String> = settings_of.keys().filter(|w| allows(w)).collect();
    if workspaces.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<ProjectRow> = q(
        db,
        &format!(
            "{PROJECT_SELECT} WHERE p.archived_at IS NULL AND p.workspace_id IN (SELECT value FROM json_each(?2)) ORDER BY p.name"
        ),
        &[s(&user.id), s(&serde_json::to_string(&workspaces)?)],
    )?
    .all()
    .await?;
    let instance = crate::instance::settings(db).await?;
    let mapped: Vec<String> = if ask.place == SlackPlace::Dm {
        Vec::new()
    } else {
        mapped_projects(db, &slack.team, &ask.channel).await?
    };
    let mut out = Vec::new();
    for row in rows {
        if !mapped.is_empty() && !mapped.contains(&row.id) {
            continue;
        }
        let role = crate::access::project_role(user, &instance, None, &row);
        if !fctl_domain::perm::can_start_sessions(role) {
            continue;
        }
        let admin = row
            .workspace
            .workspace_role
            .as_deref()
            .is_some_and(|r| r == "owner" || r == "admin");
        out.push(Candidate { row, admin });
    }
    Ok(out)
}

/// The projects channel `channel` maps to, in workspaces still connected.
async fn mapped_projects(db: &D1Database, team: &str, channel: &str) -> AppResult<Vec<String>> {
    #[derive(Deserialize)]
    struct Row {
        project_id: String,
    }
    let rows: Vec<Row> = q(
        db,
        "SELECT c.project_id FROM slack_channels c JOIN workspace_slack ws ON ws.workspace_id = c.workspace_id AND ws.team_id = c.team_id \
         WHERE c.team_id = ? AND c.channel_id = ? AND c.project_id IS NOT NULL AND ws.revoked_at IS NULL",
        &[s(team), s(channel)],
    )?
    .all()
    .await?;
    Ok(rows.into_iter().map(|r| r.project_id).collect())
}

/// The names of the projects the channel maps to that they can see but
/// not start sessions in.
async fn mapped_elsewhere(
    db: &D1Database,
    slack: &Slack,
    user: &UserRow,
    ask: &Ask,
) -> AppResult<Vec<String>> {
    if ask.place == SlackPlace::Dm {
        return Ok(Vec::new());
    }
    let mapped = mapped_projects(db, &slack.team, &ask.channel).await?;
    if mapped.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<ProjectRow> = q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id IN (SELECT value FROM json_each(?2))"),
        &[s(&user.id), s(&serde_json::to_string(&mapped)?)],
    )?
    .all()
    .await?;
    let instance = crate::instance::settings(db).await?;
    Ok(rows
        .into_iter()
        .filter(|r| crate::access::project_role(user, &instance, None, r).is_some())
        .map(|r| r.name)
        .collect())
}

/// The project someone's direct messages start sessions in, where they
/// chose to keep one.
async fn dm_project(db: &D1Database, user: &str, team: &str) -> AppResult<Option<String>> {
    #[derive(Deserialize)]
    struct Row {
        dm_project_id: Option<String>,
    }
    let row: Option<Row> = q(
        db,
        "SELECT dm_project_id FROM slack_accounts WHERE user_id = ? AND team_id = ?",
        &[s(user), s(team)],
    )?
    .first()
    .await?;
    Ok(row.and_then(|r| r.dm_project_id))
}

/// Who asked, as Slack names them, for the transcript.
async fn person_of(slack: &Slack, slack_user: &str, user: &UserRow) -> SlackPerson {
    match slack.user(slack_user).await {
        Ok(u) => SlackPerson {
            name: u.name,
            user: Some(slack_user.to_owned()),
            handle: u.handle,
            avatar_url: u.avatar_url,
            bot: false,
        },
        Err(_) => SlackPerson {
            name: user.name.clone(),
            user: Some(slack_user.to_owned()),
            handle: None,
            avatar_url: user.avatar_url.clone(),
            bot: false,
        },
    }
}

/// The names of the people a request and its thread mention or are by, by
/// their Slack id: at most a dozen asked of Slack.
async fn names(slack: &Slack, ask: &Ask) -> BTreeMap<String, String> {
    let mut ids = text::mentioned(&ask.text);
    if let Some(q) = &ask.quoted {
        ids.extend(q.user.clone());
        ids.extend(text::mentioned(&q.text));
    }
    ids.push(ask.user.clone());
    let mut out = BTreeMap::new();
    for id in ids.into_iter().filter(|i| *i != slack.bot_user).take(12) {
        if out.contains_key(&id) {
            continue;
        }
        if let Ok(u) = slack.user(&id).await {
            out.insert(id, u.name);
        }
    }
    out.insert(slack.bot_user.clone(), "fctl".into());
    out
}

/// A time in UTC, as tasks show it.
fn utc(ms: i64) -> String {
    sl::utc(ms)
}

/// The thread's earlier messages, before the one that asked: what Slack
/// gives fctl to read (`conversations.replies`), with whether more came
/// before, and whether fctl could read them at all.
async fn context(
    slack: &Slack,
    ask: &Ask,
    names: &BTreeMap<String, String>,
) -> (Vec<Earlier>, bool, bool) {
    // The thread the request (or the message it sent) is in.
    let (channel, thread_ts, before) = match &ask.quoted {
        // A message in a thread, or one that has replies (Slack names its
        // own thread then); a message alone has nothing more to read.
        Some(q) => match q.thread_ts.as_deref() {
            Some(t) => (q.channel.as_str(), t, None),
            None => return (Vec::new(), false, true),
        },
        None if ask.thread_ts != ask.message_ts => (
            ask.channel.as_str(),
            ask.thread_ts.as_str(),
            Some(ask.message_ts.as_str()),
        ),
        None => return (Vec::new(), false, true),
    };
    let found = slack.replies(channel, thread_ts, None, 50).await;
    let (messages, more) = match found {
        Ok(m) => m,
        // A shortcut's message in a conversation fctl isn't in: nothing to
        // read, which is no failure.
        Err(super::Failure::Slack(c)) if c == "not_in_channel" || c == "channel_not_found" => {
            return (Vec::new(), false, true);
        }
        Err(e) => {
            worker::console_error!("reading a thread for its context: {e:?}");
            return (Vec::new(), false, false);
        }
    };
    let quoted_ts = ask.quoted.as_ref().map(|q| q.ts.as_str());
    let name = |m: &Value| -> String {
        let user = m.get("user").and_then(Value::as_str).unwrap_or_default();
        names
            .get(user)
            .cloned()
            .or_else(|| {
                m.pointer("/user_profile/real_name")
                    .or_else(|| m.pointer("/bot_profile/name"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| {
                if user.is_empty() {
                    "an app".into()
                } else {
                    format!("@{user}")
                }
            })
    };
    let name_of = |id: &str| names.get(id).cloned();
    let earlier: Vec<Earlier> = messages
        .iter()
        .filter(|m| {
            let ts = m.get("ts").and_then(Value::as_str).unwrap_or_default();
            Some(ts) != quoted_ts && before.is_none_or(|b| ts < b)
        })
        .filter(|m| {
            m.get("subtype")
                .and_then(Value::as_str)
                .is_none_or(|s| s == "bot_message" || s == "thread_broadcast" || s == "file_share")
        })
        .map(|m| Earlier {
            by: name(m),
            at: utc(m
                .get("ts")
                .and_then(Value::as_str)
                .and_then(sl::ts_millis)
                .unwrap_or(0)),
            text: text::plain(
                m.get("text").and_then(Value::as_str).unwrap_or_default(),
                &name_of,
            ),
        })
        .filter(|e| !e.text.trim().is_empty())
        .collect();
    (earlier, more, true)
}

// ───────────────────────────── replies ─────────────────────────────

/// A message in a thread that reaches a session: the agent's next message,
/// as its author's, once fctl knows them and they may message the session;
/// otherwise it stays in the thread as information.
#[allow(clippy::too_many_arguments)]
async fn reply(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    t: &ThreadRow,
    session_id: &str,
    m: &MessageEvent,
    now: i64,
) -> AppResult<()> {
    let Some(author) = m.user.as_deref() else {
        return Ok(());
    };
    if !connected(db, session_id, &slack.team).await? {
        return Ok(());
    }
    let user = super::who(db, slack, author, now).await?;
    let Some(user) = user else {
        // Someone fctl doesn't know: their words stay information, unless
        // they mentioned fctl, which asks it something.
        if m.mentions(&slack.bot_user) {
            keep_pending(db, t, author, m, now).await?;
            let link = link_url(cfg, &slack.team, &t.id);
            let _ = slack
                .ephemeral(
                    &m.channel,
                    author,
                    Some(&t.thread_ts),
                    "Link your Slack account to fctl, and your message reaches its agent.",
                    &blocks::link_account(&link, "fctl passes your message on to the agent"),
                )
                .await;
        }
        return Ok(());
    };
    if !may_message(db, &user, session_id).await? {
        if m.mentions(&slack.bot_user) || t.slack_user_id.as_deref() == Some(author) {
            let _ = slack
                .ephemeral(
                    &m.channel,
                    author,
                    Some(&t.thread_ts),
                    "You can't message this session.",
                    &blocks::only_you(
                        ":lock:",
                        &format!(
                            "{}, you can't message this session in fctl, so your message stays here for its agent to read rather than reach it as yours.",
                            text::escape(&user.addressed())
                        ),
                    ),
                )
                .await;
        }
        return Ok(());
    }
    let body = text::plain(
        &text::without_mention(&m.text, &slack.bot_user),
        &|id: &str| (id == slack.bot_user).then(|| "fctl".to_owned()),
    );
    if body.trim().is_empty() && m.files.is_empty() {
        return Ok(());
    }
    let row = crate::spec::session_row(db, session_id).await?;
    if row.as_ref().is_some_and(|r| {
        let status = r.status();
        !status.accepts_messages() && !status.resumes_for_messages()
    }) {
        let _ = slack
            .post(
                &m.channel,
                Some(&t.thread_ts),
                "The fctl session failed, so your reply didn't reach its agent.",
                Some(&blocks::note(
                    ":warning:",
                    &format!(
                        "The fctl session failed, so your reply didn't reach its agent: resume it in fctl to carry on, at {}",
                        cfg.url(&format!("/s/{session_id}"))
                    ),
                )),
            )
            .await;
        return Ok(());
    }
    let mut words = body.clone();
    if !m.files.is_empty() {
        words.push_str(&format!(
            "\n\n[They attached {} in Slack, which fctl didn't pass on: {}]",
            if m.files.len() == 1 {
                "a file"
            } else {
                "files"
            },
            m.file_names().join(", ")
        ));
    }
    let message = SlackMessage {
        event: SlackEvent::Replied,
        by: Some(person_of(slack, author, &user).await),
        place: t.place(),
        channel_name: None,
        team_name: Some(slack.team_name.clone()).filter(|n| !n.is_empty()),
        body: Some(body.clone()),
        url: Some(slack.link(&m.channel, &m.ts, Some(&t.thread_ts)).await),
        context: None,
        quoted: None,
        dropped: false,
    };
    match deliver(
        env,
        session_id,
        task::replied(&user.name, &words),
        &user,
        message,
        Some(format!("slack:{}:{}", m.channel, m.ts)),
    )
    .await
    {
        Ok(()) => {
            delivered(db, t, &m.ts, now).await?;
            slack.react(&m.channel, &m.ts, "eyes").await;
        }
        Err(e) => {
            let _ = slack
                .ephemeral(
                    &m.channel,
                    author,
                    Some(&t.thread_ts),
                    "fctl couldn't pass your reply on.",
                    &blocks::only_you(
                        ":warning:",
                        &format!(
                            "fctl couldn't pass your reply on: {}",
                            text::escape(&e.0.message)
                        ),
                    ),
                )
                .await;
        }
    }
    Ok(())
}

/// Keeps a message to a started session from someone fctl doesn't know
/// yet, for when they link their account, for an hour.
async fn keep_pending(
    db: &D1Database,
    t: &ThreadRow,
    author: &str,
    m: &MessageEvent,
    now: i64,
) -> AppResult<()> {
    let mut kept: serde_json::Map<String, Value> = t
        .payload
        .as_deref()
        .filter(|_| t.waiting_for.as_deref() == Some("reply"))
        .and_then(|p| serde_json::from_str::<Value>(p).ok())
        .and_then(|v| v.get("replies").and_then(Value::as_object).cloned())
        .unwrap_or_default();
    kept.insert(
        author.to_owned(),
        json!({ "text": m.text, "ts": m.ts, "channel": m.channel }),
    );
    q(
        db,
        "UPDATE slack_threads SET waiting_for = 'reply', payload = ?, expires_at = ?, updated_at = ? WHERE id = ?",
        &[
            s(&json!({ "replies": kept }).to_string()),
            n(now + sl::WAIT_MS),
            n(now),
            s(&t.id),
        ],
    )?
    .run()
    .await?;
    Ok(())
}

/// Whether session `session_id`'s project is in an fctl workspace that
/// works in Slack workspace `team`: one that disconnected it hears nothing
/// more from there.
async fn connected(db: &D1Database, session_id: &str, team: &str) -> AppResult<bool> {
    let c: Option<crate::db::Count> = q(
        db,
        "SELECT COUNT(*) AS n FROM sessions s JOIN projects p ON p.id = s.project_id \
         JOIN workspace_slack ws ON ws.workspace_id = p.workspace_id WHERE s.id = ? AND ws.team_id = ? AND ws.revoked_at IS NULL",
        &[s(session_id), s(team)],
    )?
    .first()
    .await?;
    Ok(c.is_some_and(|c| c.n > 0))
}

/// Someone pressed stop on the agent session in Slack: the agent's turn
/// stops, as their stop in fctl would.
async fn stopped(
    env: &Env,
    db: &D1Database,
    slack: &Slack,
    channel: &str,
    thread_ts: &str,
    user: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let id = sl::thread_id(&slack.team, channel, thread_ts);
    let Some(t) = thread(db, &id).await? else {
        return Ok(());
    };
    let Some(session_id) = t.session_id.as_deref().filter(|_| t.state == "started") else {
        return Ok(());
    };
    let by = match user {
        Some(u) => super::who(db, slack, u, now).await?,
        None => None,
    };
    if let Some(b) = &by
        && !may_message(db, b, session_id).await?
    {
        return Ok(());
    }
    interrupt(env, session_id, by.map(|b| b.to_ref())).await?;
    slack
        .set_status(channel, thread_ts, "active", None, None)
        .await;
    Ok(())
}

// ───────────────────────────── interactions ─────────────────────────────

/// What fctl answers an interaction with at once, within Slack's three
/// seconds; the rest happens after ([`interaction`]).
pub fn quick(i: &Interaction) -> Option<Value> {
    match i {
        Interaction::ViewSubmission {
            callback_id,
            values,
            ..
        } if callback_id == callback::START || callback_id == callback::SEND => {
            let task = values
                .iter()
                .find(|(b, _)| b == "task")
                .map(|(_, v)| v.trim())
                .unwrap_or_default();
            let project = values.iter().any(|(b, v)| b == "project" && !v.is_empty());
            if !project {
                return Some(
                    json!({ "response_action": "errors", "errors": { "project": "Choose a project" } }),
                );
            }
            if callback_id == callback::START && task.is_empty() {
                return Some(
                    json!({ "response_action": "errors", "errors": { "task": "Say what the agent should do" } }),
                );
            }
            Some(json!({ "response_action": "clear" }))
        }
        _ => None,
    }
}

/// What someone did with fctl's buttons, menus, forms and shortcuts.
pub async fn interaction(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    i: &Interaction,
    now: i64,
) -> AppResult<()> {
    let team = match i {
        Interaction::BlockActions { actor, .. }
        | Interaction::ViewSubmission { actor, .. }
        | Interaction::MessageAction { actor, .. }
        | Interaction::Shortcut { actor, .. } => actor.team.clone(),
        Interaction::Other => return Ok(()),
    };
    let keys = crypto::keys(cfg)?;
    let Ok(slack) = super::client(db, cfg, &keys, &team).await else {
        return Ok(());
    };
    match i {
        Interaction::BlockActions {
            actor,
            channel,
            message_ts,
            thread_ts,
            actions,
            response_url,
            trigger_id,
            checked,
            ..
        } => {
            for a in actions {
                let done = action(
                    env,
                    cfg,
                    db,
                    &slack,
                    &actor.user,
                    channel.as_deref(),
                    message_ts.as_deref(),
                    thread_ts.as_deref(),
                    a,
                    response_url.as_deref(),
                    trigger_id.as_deref(),
                    checked,
                    now,
                )
                .await;
                if let Err(e) = done {
                    worker::console_error!("a Slack action ({}): {}", a.action_id, e.0.message);
                }
            }
            Ok(())
        }
        Interaction::MessageAction {
            actor,
            callback_id,
            channel,
            channel_name,
            message,
            response_url,
            trigger_id,
        } if callback_id == callback::SEND_SHORTCUT => {
            send_form(
                cfg,
                db,
                &slack,
                &actor.user,
                channel,
                channel_name.as_deref(),
                message,
                response_url.as_deref(),
                trigger_id,
                now,
            )
            .await
        }
        Interaction::Shortcut {
            actor,
            callback_id,
            trigger_id,
        } if callback_id == callback::START_SHORTCUT => {
            start_form(cfg, db, &slack, &actor.user, trigger_id, "", None, now).await
        }
        Interaction::ViewSubmission {
            actor,
            callback_id,
            private_metadata,
            values,
            checked,
        } => {
            submitted(
                env,
                cfg,
                db,
                &slack,
                &actor.user,
                callback_id,
                private_metadata,
                values,
                checked,
                now,
            )
            .await
        }
        _ => Ok(()),
    }
}

/// A button or a menu.
#[allow(clippy::too_many_arguments)]
async fn action(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    user: &str,
    channel: Option<&str>,
    message_ts: Option<&str>,
    thread_ts: Option<&str>,
    a: &Action,
    response_url: Option<&str>,
    trigger_id: Option<&str>,
    checked: &[(String, Vec<String>)],
    now: i64,
) -> AppResult<()> {
    let id = a.action_id.as_str();
    if let Some(n) = id.strip_prefix(act::ANSWER) {
        let Some(block) = a.block_id.as_deref() else {
            return Ok(());
        };
        return answer(
            env,
            cfg,
            db,
            slack,
            user,
            block,
            n.parse().unwrap_or(usize::MAX),
            channel,
            message_ts,
            thread_ts,
            response_url,
            now,
        )
        .await;
    }
    if id.starts_with(act::PROJECT) {
        let Some((thread_id, project)) = a.value.as_deref().and_then(|v| v.rsplit_once('|')) else {
            return Ok(());
        };
        let remember = checked
            .iter()
            .any(|(b, v)| b == act::REMEMBER && v.iter().any(|x| x == "remember"));
        return chose(
            env,
            cfg,
            db,
            slack,
            user,
            thread_id,
            project,
            remember,
            response_url,
            now,
        )
        .await;
    }
    if id == act::CANCEL {
        if let Some(thread_id) = a.value.as_deref()
            && let Some(t) = thread(db, thread_id).await?
            && t.slack_user_id.as_deref() == Some(user)
            && t.state == "waiting"
        {
            set_state(db, thread_id, "refused", None, now).await?;
        }
        if let Some(r) = response_url {
            super::respond(r, json!({ "delete_original": true })).await;
        }
        return Ok(());
    }
    if id == act::START
        && let Some(trigger) = trigger_id
    {
        return start_form(cfg, db, slack, user, trigger, "", None, now).await;
    }
    // Links (Open in fctl, Link your account): Slack opens them itself.
    Ok(())
}

/// Someone clicked an answer to the agent's question: their message, as
/// they may send one.
#[allow(clippy::too_many_arguments)]
async fn answer(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    clicker: &str,
    block_id: &str,
    index: usize,
    channel: Option<&str>,
    message_ts: Option<&str>,
    thread_ts: Option<&str>,
    response_url: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let only_you = |words: String| async move {
        if let Some(r) = response_url {
            super::respond(
                r,
                json!({
                    "response_type": "ephemeral", "replace_original": false,
                    "text": words, "blocks": blocks::only_you(":lock:", &text::escape(&words)),
                }),
            )
            .await;
        }
    };
    // `fctl_q:<session>:<asked_at>`.
    let mut parts = block_id.splitn(3, ':');
    let (Some("fctl_q"), Some(session_id), Some(asked_at)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Ok(());
    };
    let Some(row) = crate::spec::session_row(db, session_id).await? else {
        return Ok(());
    };
    let question = row
        .question
        .as_deref()
        .and_then(|j| serde_json::from_str::<fctl_proto::session::SessionQuestion>(j).ok());
    let Some(question) = question.filter(|q| q.asked_at.to_string() == asked_at) else {
        only_you(
            "That question was answered already, or the agent asked something else since.".into(),
        )
        .await;
        return Ok(());
    };
    let Some(option) = question.options.get(index).cloned() else {
        return Ok(());
    };
    let Some(user) = super::who(db, slack, clicker, now).await? else {
        let link = link_url(cfg, &slack.team, "");
        if let Some(r) = response_url {
            super::respond(
                r,
                json!({
                    "response_type": "ephemeral", "replace_original": false,
                    "text": "Link your Slack account to fctl to answer.",
                    "blocks": blocks::link_account(&link, "you answer with a click"),
                }),
            )
            .await;
        }
        return Ok(());
    };
    if !may_message(db, &user, session_id).await? {
        only_you(format!(
            "{}, only people who may message this session answer it: you can't, in fctl.",
            user.addressed()
        ))
        .await;
        return Ok(());
    }
    let message = SlackMessage {
        event: SlackEvent::Answered,
        by: Some(person_of(slack, clicker, &user).await),
        place: if channel.is_some_and(|c| c.starts_with('D')) {
            SlackPlace::Dm
        } else {
            SlackPlace::Channel
        },
        channel_name: None,
        team_name: Some(slack.team_name.clone()).filter(|n| !n.is_empty()),
        body: Some(option.clone()),
        url: None,
        context: None,
        quoted: None,
        dropped: false,
    };
    deliver(
        env,
        session_id,
        option.clone(),
        &user,
        message,
        Some(format!("slack:answer:{session_id}:{asked_at}:{clicker}")),
    )
    .await?;
    if let (Some(c), Some(ts)) = (channel, message_ts) {
        let _ = slack
            .update(
                c,
                ts,
                &format!("{} answered: {option}", user.name),
                &blocks::answered(&question.text, &user.name, &option),
            )
            .await;
        if let Some(thread_ts) = thread_ts
            && let Some(t) = thread(db, &sl::thread_id(&slack.team, c, thread_ts)).await?
        {
            let _ = q(
                db,
                "UPDATE slack_threads SET question_ts = NULL, updated_at = ? WHERE id = ? AND question_ts = ?",
                &[n(now), s(&t.id), s(ts)],
            )?
            .run()
            .await;
        }
    }
    Ok(())
}

/// Someone chose the project for their request.
#[allow(clippy::too_many_arguments)]
async fn chose(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    user: &str,
    thread_id: &str,
    project: &str,
    remember: bool,
    response_url: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let Some(t) = thread(db, thread_id).await? else {
        return Ok(());
    };
    if t.slack_user_id.as_deref() != Some(user) || t.state != "waiting" {
        return Ok(());
    }
    let Some(ask) = t
        .payload
        .as_deref()
        .and_then(|p| serde_json::from_str::<Ask>(p).ok())
    else {
        return Ok(());
    };
    if let Some(r) = response_url {
        super::respond(r, json!({ "delete_original": true })).await;
    }
    if remember
        && ask.place != SlackPlace::Dm
        && let Some(person) = super::who(db, slack, user, now).await?
        && let Err(e) = remember_channel(db, &person, slack, &ask, project, now).await
    {
        worker::console_error!("remembering a channel's project: {}", e.0.message);
    }
    take(env, cfg, db, slack, &ask, Some(project), now).await
}

/// An admin's choice of project for a channel, kept as its mapping.
async fn remember_channel(
    db: &D1Database,
    person: &UserRow,
    slack: &Slack,
    ask: &Ask,
    project: &str,
    now: i64,
) -> AppResult<()> {
    let row: Option<ProjectRow> = q(
        db,
        &format!("{PROJECT_SELECT} WHERE p.id = ?2"),
        &[s(&person.id), s(project)],
    )?
    .first()
    .await?;
    let Some(row) = row.filter(|r| {
        r.workspace
            .workspace_role
            .as_deref()
            .is_some_and(|w| w == "owner" || w == "admin")
    }) else {
        return Ok(());
    };
    q(
        db,
        "INSERT INTO slack_channels (workspace_id, team_id, channel_id, name, private, project_id, readable, added_by, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 0, ?, ?, ?) ON CONFLICT (workspace_id, team_id, channel_id) DO UPDATE SET \
         project_id = excluded.project_id, name = excluded.name, updated_at = excluded.updated_at",
        &[
            s(row.workspace_id()),
            s(&slack.team),
            s(&ask.channel),
            s(ask.channel_name.as_deref().unwrap_or(&ask.channel)),
            n(i64::from(ask.place == SlackPlace::Private)),
            s(&row.id),
            s(&person.id),
            n(now),
            n(now),
        ],
    )?
    .run()
    .await?;
    audit(
        db,
        row.workspace_id(),
        &person.id,
        &ask.channel,
        json!({ "action": "slack.channel_mapped", "project": row.slug, "channel": ask.channel_name }),
        now,
    )
    .await;
    Ok(())
}

/// The projects someone may start sessions in from Slack, for a form.
async fn choices(
    db: &D1Database,
    slack: &Slack,
    user: &UserRow,
    place: SlackPlace,
    channel: &str,
) -> AppResult<Vec<Choice>> {
    let ask = Ask {
        team: slack.team.clone(),
        channel: channel.to_owned(),
        channel_name: None,
        place,
        thread_ts: String::new(),
        message_ts: String::new(),
        user: String::new(),
        text: String::new(),
        how: AskHow::Command,
        quoted: None,
        action_token: None,
        updates: true,
        project: None,
    };
    let candidates = candidates(db, slack, user, &ask).await?;
    let several = candidates
        .iter()
        .map(|c| c.row.workspace_id().to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 1;
    Ok(candidates
        .into_iter()
        .map(|c| Choice {
            workspace: several.then(|| c.row.workspace.workspace_name.clone().unwrap_or_default()),
            id: c.row.id,
            name: c.row.name,
        })
        .collect())
}

/// Opens the form that starts a session, for the Home tab, `/fctl` and the
/// global shortcut.
#[allow(clippy::too_many_arguments)]
async fn start_form(
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    user: &str,
    trigger_id: &str,
    task: &str,
    channel: Option<&str>,
    now: i64,
) -> AppResult<()> {
    let Some(person) = super::who(db, slack, user, now).await? else {
        let link = link_url(cfg, &slack.team, "");
        let dm = slack.open_dm(user).await?;
        let _ = slack
            .post(
                &dm,
                None,
                "Link your Slack account to fctl to start sessions.",
                Some(&blocks::link_account(
                    &link,
                    "you start sessions from Slack",
                )),
            )
            .await;
        return Ok(());
    };
    let choices = choices(db, slack, &person, SlackPlace::Dm, channel.unwrap_or("")).await?;
    if choices.is_empty() {
        let dm = slack.open_dm(user).await?;
        let words = "There's no fctl project here you can start sessions in: a workspace connected to this Slack workspace makes you an editor of one.";
        let _ = slack
            .post(
                &dm,
                None,
                words,
                Some(&blocks::only_you(":no_entry_sign:", words)),
            )
            .await;
        return Ok(());
    }
    let kept = dm_project(db, &person.id, &slack.team).await?;
    let metadata = json!({ "channel": channel }).to_string();
    slack
        .open_view(
            trigger_id,
            &blocks::start_form(&choices, kept.as_deref(), task, &metadata),
        )
        .await?;
    Ok(())
}

/// Opens Send to fctl…'s form on a message.
#[allow(clippy::too_many_arguments)]
async fn send_form(
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    user: &str,
    channel: &str,
    channel_name: Option<&str>,
    message: &MessageEvent,
    response_url: Option<&str>,
    trigger_id: &str,
    now: i64,
) -> AppResult<()> {
    let Some(person) = super::who(db, slack, user, now).await? else {
        let link = link_url(cfg, &slack.team, "");
        if let Some(r) = response_url {
            super::respond(
                r,
                json!({ "response_type": "ephemeral", "text": "Link your Slack account to fctl to send it messages.",
                    "blocks": blocks::link_account(&link, "you send messages to fctl") }),
            )
            .await;
        }
        return Ok(());
    };
    let place = if channel.starts_with('D') {
        SlackPlace::Dm
    } else {
        SlackPlace::Channel
    };
    let choices = choices(db, slack, &person, place, channel).await?;
    if choices.is_empty() {
        if let Some(r) = response_url {
            let words = "There's no fctl project here you can start sessions in: a workspace connected to this Slack workspace makes you an editor of one.";
            super::respond(
                r,
                json!({ "response_type": "ephemeral", "text": words, "blocks": blocks::only_you(":no_entry_sign:", words) }),
            )
            .await;
        }
        return Ok(());
    }
    // fctl posts in the message's thread only where its app is.
    let member = match slack.conversation(channel).await {
        Ok(c) => c.member && !c.shared && c.place != SlackPlace::Dm,
        Err(_) => false,
    };
    let mapped = mapped_projects(db, &slack.team, channel).await?;
    let chosen = mapped
        .iter()
        .find(|m| choices.iter().any(|c| c.id == **m))
        .cloned();
    let by = match message.user.as_deref() {
        Some(u) => slack
            .user(u)
            .await
            .map(|u| u.name)
            .unwrap_or_else(|_| "someone".into()),
        None => "an app".into(),
    };
    let names: BTreeMap<String, String> = BTreeMap::new();
    let quoted_text = text::plain(&message.text, &|id: &str| names.get(id).cloned());
    // What the message said, as Slack gave it, so that starting the session
    // needn't read it again (unlisted apps read threads once a minute).
    let metadata = json!({
        "c": channel, "n": channel_name, "ts": message.ts, "tt": message.thread_ts,
        "u": message.user, "m": member, "t": text::clip(&message.text, 1800),
    })
    .to_string();
    slack
        .open_view(
            trigger_id,
            &blocks::send_form(
                &choices,
                chosen.as_deref(),
                &by,
                &quoted_text,
                member,
                &metadata,
            ),
        )
        .await?;
    Ok(())
}

/// A form someone sent: a session to start.
#[allow(clippy::too_many_arguments)]
async fn submitted(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    slack: &Slack,
    user: &str,
    callback_id: &str,
    metadata: &str,
    values: &[(String, String)],
    checked: &[(String, Vec<String>)],
    now: i64,
) -> AppResult<()> {
    let value = |b: &str| {
        values
            .iter()
            .find(|(k, _)| k == b)
            .map(|(_, v)| v.trim().to_owned())
    };
    let project = value("project").unwrap_or_default();
    let task_text = value("task").unwrap_or_default();
    let meta: Value = serde_json::from_str(metadata).unwrap_or(Value::Null);
    let m = |k: &str| meta.get(k).and_then(Value::as_str).map(str::to_owned);
    let mut ask = match callback_id {
        c if c == callback::SEND => {
            let Some(channel) = m("c") else {
                return Ok(());
            };
            let updates = checked
                .iter()
                .any(|(b, v)| b == "updates" && v.iter().any(|x| x == "updates"))
                && meta.get("m").and_then(Value::as_bool) == Some(true);
            let ts = m("ts").unwrap_or_default();
            Ask {
                team: slack.team.clone(),
                channel: channel.clone(),
                channel_name: m("n"),
                place: SlackPlace::Channel,
                thread_ts: m("tt").unwrap_or_else(|| ts.clone()),
                message_ts: ts.clone(),
                user: user.to_owned(),
                text: task_text,
                how: AskHow::Shortcut,
                quoted: Some(Quoted {
                    channel,
                    ts: ts.clone(),
                    thread_ts: m("tt"),
                    user: m("u"),
                    text: m("t").unwrap_or_default(),
                }),
                action_token: None,
                updates,
                project: Some(project),
            }
        }
        c if c == callback::START => Ask {
            team: slack.team.clone(),
            // `/fctl` in a channel, where its thread goes when fctl's app
            // is there; the Home tab and the shortcut: none.
            channel: m("channel")
                .filter(|c| !c.starts_with('D'))
                .unwrap_or_default(),
            channel_name: None,
            place: SlackPlace::Dm,
            thread_ts: String::new(),
            message_ts: String::new(),
            user: user.to_owned(),
            text: task_text,
            how: AskHow::Command,
            quoted: None,
            action_token: None,
            updates: true,
            project: Some(project),
        },
        _ => return Ok(()),
    };
    // The message sent, as Slack has it now: fctl reads it where its app
    // can, and keeps only what the session needs.
    if let Some(qd) = ask.quoted.as_mut()
        && (qd.text.is_empty() || qd.text.ends_with('…'))
        && let Ok((msgs, _)) = slack
            .replies(
                &qd.channel,
                qd.thread_ts.as_deref().unwrap_or(&qd.ts),
                None,
                50,
            )
            .await
        && let Some(found) = msgs
            .iter()
            .find(|x| x.get("ts").and_then(Value::as_str) == Some(qd.ts.as_str()))
    {
        qd.text = found
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
    }
    // Where its card goes: the message's thread; for `/fctl`, a new thread
    // in its channel, where fctl's app is; else a new thread in their
    // direct messages with the app.
    if !ask.updates || ask.how == AskHow::Command {
        let words = if ask.text.trim().is_empty() {
            "Starting a session…".to_owned()
        } else {
            format!("Starting a session: {}", text::clip(&ask.text, 200))
        };
        let in_channel = if ask.how == AskHow::Command && !ask.channel.is_empty() {
            let who = super::who(db, slack, user, now).await?;
            let said = format!(
                "*{}* asked fctl: {}",
                text::escape(&who.map_or_else(|| "Someone".to_owned(), |u| u.name)),
                text::escape(&text::clip(&ask.text, 1500))
            );
            slack
                .post(
                    &ask.channel,
                    None,
                    &words,
                    Some(&blocks::note(":speech_balloon:", &said)),
                )
                .await
                .ok()
        } else {
            None
        };
        match in_channel {
            Some(ts) => {
                ask.place = SlackPlace::Channel;
                ask.thread_ts = ts.clone();
                ask.message_ts = ts;
            }
            None => {
                let dm = slack.open_dm(user).await?;
                let ts = slack
                    .post(
                        &dm,
                        None,
                        &words,
                        Some(&blocks::note(
                            ":hourglass_flowing_sand:",
                            &text::escape(&words),
                        )),
                    )
                    .await?;
                ask.channel = dm;
                ask.place = SlackPlace::Dm;
                ask.thread_ts = ts.clone();
                ask.message_ts = ts;
            }
        }
        ask.updates = true;
    }
    begin(env, cfg, db, slack, ask, now).await
}

// ───────────────────────────── /fctl ─────────────────────────────

/// What fctl answers `/fctl` with at once, within Slack's three seconds:
/// help at once; otherwise nothing, as what follows comes by itself.
pub fn quick_command(c: &sl::Command) -> Value {
    match sl::asked(&c.text) {
        sl::Asked::Help => json!({
            "response_type": "ephemeral",
            "text": "/fctl <what to do> starts a session as you; /fctl status lists yours.",
            "blocks": [
                { "type": "section", "text": { "type": "mrkdwn", "verbatim": true, "text":
                    "*/fctl <what to do>* starts an agent on it, as you, here: its card in this channel says how it goes, and replies in its thread steer it.\n*/fctl status* lists your sessions, and what waits for you.\nYou can also mention fctl in a thread, write to it in its messages, or use *Send to fctl…* on any message." } }
            ]
        }),
        _ => json!({}),
    }
}

/// `/fctl`, after fctl answered at once.
pub async fn command(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    c: &sl::Command,
    now: i64,
) -> AppResult<()> {
    if c.enterprise_install {
        return Ok(());
    }
    let keys = crypto::keys(cfg)?;
    let Ok(slack) = super::client(db, cfg, &keys, &c.actor.team).await else {
        return Ok(());
    };
    match sl::asked(&c.text) {
        sl::Asked::Help => Ok(()),
        sl::Asked::Status => {
            let message = super::home::status(cfg, db, &slack, &c.actor.user, now).await?;
            if let Some(r) = &c.response_url {
                super::respond(r, message).await;
            }
            Ok(())
        }
        sl::Asked::Start(task_text) => {
            let Some(person) = super::who(db, &slack, &c.actor.user, now).await? else {
                let link = link_url(cfg, &slack.team, "");
                if let Some(r) = &c.response_url {
                    super::respond(
                        r,
                        json!({ "response_type": "ephemeral", "text": "Link your Slack account to fctl to start sessions.",
                            "blocks": blocks::link_account(&link, "you start sessions with /fctl") }),
                    )
                    .await;
                }
                return Ok(());
            };
            let dm = c.channel.starts_with('D');
            let place = if dm {
                SlackPlace::Dm
            } else {
                SlackPlace::Channel
            };
            let choices = choices(db, &slack, &person, place, &c.channel).await?;
            let mapped = mapped_projects(db, &slack.team, &c.channel).await?;
            let project = match choices.as_slice() {
                [only] => Some(only.id.clone()),
                _ => mapped
                    .iter()
                    .find(|m| choices.iter().any(|ch| ch.id == **m))
                    .cloned(),
            };
            let Some(project) = project else {
                // Several to choose from: the form, with the task in it.
                if let Some(trigger) = &c.trigger_id {
                    return start_form(
                        cfg,
                        db,
                        &slack,
                        &c.actor.user,
                        trigger,
                        &task_text,
                        Some(&c.channel),
                        now,
                    )
                    .await;
                }
                return Ok(());
            };
            // Its card starts a thread in the channel, where fctl's app is;
            // else in their direct messages with it.
            let posted = slack
                .post(
                    &c.channel,
                    None,
                    &format!(
                        "{} asked fctl: {}",
                        person.name,
                        text::clip(&task_text, 200)
                    ),
                    Some(&blocks::note(
                        ":speech_balloon:",
                        &format!(
                            "*{}* asked fctl: {}",
                            text::escape(&person.name),
                            text::escape(&text::clip(&task_text, 1500))
                        ),
                    )),
                )
                .await;
            let (channel, ts, place) = match posted {
                Ok(ts) => (c.channel.clone(), ts, place),
                Err(_) => {
                    let d = slack.open_dm(&c.actor.user).await?;
                    let ts = slack
                        .post(
                            &d,
                            None,
                            &format!("You asked fctl: {}", text::clip(&task_text, 200)),
                            Some(&blocks::note(
                                ":speech_balloon:",
                                &format!(
                                    "You asked fctl: {}",
                                    text::escape(&text::clip(&task_text, 1500))
                                ),
                            )),
                        )
                        .await?;
                    (d, ts, SlackPlace::Dm)
                }
            };
            let ask = Ask {
                team: slack.team.clone(),
                channel,
                channel_name: c.channel_name.clone(),
                place,
                thread_ts: ts.clone(),
                message_ts: ts,
                user: c.actor.user.clone(),
                text: task_text,
                how: AskHow::Command,
                quoted: None,
                action_token: None,
                updates: true,
                project: Some(project),
            };
            begin(env, cfg, db, &slack, ask, now).await
        }
    }
}

// ───────────────────────────── linking ─────────────────────────────

/// The link that links someone's Slack account, from a thread that waits
/// for it.
pub(crate) fn link_url(cfg: &Config, team: &str, thread: &str) -> String {
    let mut u = url::form_urlencoded::Serializer::new(String::new());
    u.append_pair("team", team);
    if !thread.is_empty() {
        u.append_pair("thread", thread);
    }
    cfg.url(&format!("/api/v1/slack/link?{}", u.finish()))
}

/// Someone linked their Slack account in `team`: what waited for them goes
/// on. Returns the sessions it started or reached.
pub async fn linked(
    env: &Env,
    cfg: &Config,
    db: &D1Database,
    team: &str,
    slack_user: &str,
    now: i64,
) -> AppResult<Vec<String>> {
    let waiting: Vec<ThreadRow> = q(
        db,
        &format!(
            "{THREAD_SELECT} WHERE team_id = ? AND expires_at > ? AND waiting_for IS NOT NULL ORDER BY created_at"
        ),
        &[s(team), n(now)],
    )?
    .all()
    .await?;
    if waiting.is_empty() {
        return Ok(Vec::new());
    }
    let keys = crypto::keys(cfg)?;
    let slack = super::client(db, cfg, &keys, team).await?;
    let mut reached = Vec::new();
    for t in waiting {
        let result = match t.waiting_for.as_deref() {
            Some("link" | "retry") if t.slack_user_id.as_deref() == Some(slack_user) => match t
                .payload
                .as_deref()
                .and_then(|p| serde_json::from_str::<Ask>(p).ok())
            {
                Some(ask) => take(env, cfg, db, &slack, &ask, None, now).await,
                None => Ok(()),
            },
            Some("reply") => {
                let kept = t
                    .payload
                    .as_deref()
                    .and_then(|p| serde_json::from_str::<Value>(p).ok())
                    .and_then(|v| v.pointer(&format!("/replies/{slack_user}")).cloned());
                match (kept, t.session_id.as_deref()) {
                    (Some(k), Some(session_id)) => {
                        let m = MessageEvent {
                            user: Some(slack_user.to_owned()),
                            text: k
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            ts: k
                                .get("ts")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            thread_ts: Some(t.thread_ts.clone()),
                            channel: t.channel_id.clone(),
                            ..MessageEvent::default()
                        };
                        reply(env, cfg, db, &slack, &t, session_id, &m, now).await
                    }
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        };
        if let Err(err) = result {
            worker::console_error!("taking up {} after a link: {}", t.id, err.0.message);
        }
        if let Some(after) = thread(db, &t.id).await?
            && after.state == "started"
            && let Some(sid) = after.session_id
            && after.slack_user_id.as_deref() == Some(slack_user)
        {
            reached.push(sid);
        }
    }
    Ok(reached)
}

// ───────────────────────────── to sessions ─────────────────────────────

/// Whether `user` may message session `session_id`.
pub(crate) async fn may_message(
    db: &D1Database,
    user: &UserRow,
    session_id: &str,
) -> AppResult<bool> {
    let settings = crate::instance::settings(db).await?;
    let role = crate::access::session_role_now(db, user, &settings, None, session_id).await?;
    Ok(role.is_some_and(|r| r.can_message()))
}

/// Hands a message from Slack to a session's agent, resuming the session
/// if it stopped, as a person's message does.
pub(crate) async fn deliver(
    env: &Env,
    session_id: &str,
    text: String,
    user: &UserRow,
    slack: SlackMessage,
    client_key: Option<String>,
) -> AppResult<()> {
    let id = fctl_domain::ids::delivery_id(crate::ctx::now_ms());
    let author = user.to_ref();
    let stub = crate::internal::session_stub(env, session_id)?;
    let _: Delivered = crate::internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Message {
                    text: text.clone(),
                    author: Some(author.clone()),
                    author_called: user.called(),
                    attachments: Vec::new(),
                    autopilot: None,
                },
            },
            client_key,
            by_autopilot: false,
            event: EventBody::UserMessage(UserMessage {
                id,
                author: Some(author),
                text,
                source: MessageSource::Api,
                attachments: Vec::new(),
                follow_up: None,
                review: None,
                ticket: None,
                slack: Some(slack),
                autopilot: None,
                wake: None,
            }),
        },
    )
    .await?;
    Ok(())
}

/// Interrupts a session's turn, as someone's stop in Slack.
async fn interrupt(env: &Env, session_id: &str, by: Option<UserRef>) -> AppResult<()> {
    let id = fctl_domain::ids::delivery_id(crate::ctx::now_ms());
    let stub = crate::internal::session_stub(env, session_id)?;
    let _: Delivered = crate::internal::post(
        &stub,
        "/deliver",
        &Deliver {
            delivery: Delivery {
                id: id.clone(),
                kind: DeliveryKind::Interrupt { by: by.clone() },
            },
            client_key: None,
            by_autopilot: false,
            event: EventBody::Interrupt(fctl_proto::event::Interrupt { id, by }),
        },
    )
    .await?;
    Ok(())
}

pub(crate) async fn audit(
    db: &D1Database,
    workspace_id: &str,
    actor: &str,
    target: &str,
    data: Value,
    now: i64,
) {
    let action = data
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("session.created")
        .to_owned();
    let written = async {
        q(
            db,
            "INSERT INTO audit_log (id, ts, actor_id, action, target, data, ip, workspace_id) VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            &[
                s(&fctl_domain::ids::new_id("aud", now)),
                n(now),
                s(actor),
                s(&action),
                s(target),
                s(&data.to_string()),
                os(Some(workspace_id).filter(|w| !w.is_empty())),
            ],
        )?
        .run()
        .await
    }
    .await;
    if let Err(e) = written {
        worker::console_error!("audit for Slack: {}", e.0.message);
    }
}

/// Forgets what waited longer than an hour, and threads that never
/// started after a month (the hourly job).
pub async fn expire(db: &D1Database, now: i64) -> AppResult<()> {
    q(
        db,
        "UPDATE slack_threads SET state = CASE WHEN state IN ('waiting', 'new') THEN 'expired' ELSE state END, \
         waiting_for = NULL, payload = NULL, expires_at = NULL, updated_at = ?1 WHERE expires_at IS NOT NULL AND expires_at <= ?1",
        &[n(now)],
    )?
    .run()
    .await?;
    q(
        db,
        "DELETE FROM slack_threads WHERE state IN ('expired', 'refused', 'new') AND updated_at < ?",
        &[n(now - 30 * 24 * 60 * 60 * 1000)],
    )?
    .run()
    .await?;
    q(
        db,
        "DELETE FROM slack_calls WHERE at < ?",
        &[n(now - 60 * 60 * 1000)],
    )?
    .run()
    .await?;
    Ok(())
}

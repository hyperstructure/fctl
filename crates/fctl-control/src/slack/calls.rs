//! What agents read and post in Slack with `fctl slack` in their sandbox
//! (`AgentCall::Slack`, docs/architecture.md#slack). The agent never holds
//! a Slack token: each call comes here, which checks the conversation with
//! Slack (`conversations.info`) against the session before it reads
//! anything ([`fctl_domain::slack::access`]):
//!
//! - its own thread, in any conversation it started in;
//! - the public channels its workspace's admins made readable, where its
//!   project reads them;
//! - and posts only in its own thread, where its workspace lets agents.
//!
//! Each session reads, searches and posts so many times an hour
//! (`slack_calls`).

use super::Slack;
use super::inbound::{ThreadRow, thread_of};
use crate::api::calls::RunnerSession;
use crate::ctx::Ctx;
use crate::db::{n, q, s};
use crate::error::{AppError, AppResult, forbidden};
use fctl_domain::slack::access::{self, CallKind, Reach, Reading};
use fctl_domain::slack::{self as sl, blocks, text};
use fctl_proto::slack::{
    SessionSlack, SlackAbout, SlackAnswer, SlackChannelRef, SlackMessageView, SlackNotice,
    SlackPerson, SlackRequest, SlackSettings,
};
use fctl_proto::{ApiError, ErrorCode};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use worker::D1Database;

/// What a session reaches in Slack.
pub struct Reached {
    /// Its thread, where it started in Slack and its workspace still works
    /// there.
    pub thread: Option<(SessionSlack, ThreadRow)>,
    /// The Slack workspace it reads, and the channels it may read there.
    pub team: Option<String>,
    pub readable: Vec<SlackChannelRef>,
    pub settings: SlackSettings,
}

/// What session `session_id` of workspace `workspace` reaches in Slack,
/// as its project's `read_channels` says.
pub async fn reached(
    db: &D1Database,
    session_id: &str,
    workspace: &str,
    read_channels: bool,
    session_slack: Option<SessionSlack>,
) -> AppResult<Reached> {
    #[derive(Deserialize)]
    struct Conn {
        team_id: String,
        settings: String,
    }
    let conns: Vec<Conn> = q(
        db,
        "SELECT ws.team_id, ws.settings FROM workspace_slack ws JOIN slack_installations i ON i.team_id = ws.team_id \
         WHERE ws.workspace_id = ? AND ws.revoked_at IS NULL AND i.status = 'active' ORDER BY ws.created_at",
        &[s(workspace)],
    )?
    .all()
    .await?;
    let thread = match session_slack {
        Some(slack) if conns.iter().any(|c| c.team_id == slack.team) => {
            thread_of(db, session_id).await?.map(|t| (slack, t))
        }
        _ => None,
    };
    // The thread's Slack workspace, else the workspace's first.
    let conn = match &thread {
        Some((slack, _)) => conns.iter().find(|c| c.team_id == slack.team),
        None => conns.first(),
    };
    let Some(conn) = conn else {
        return Ok(Reached {
            thread: None,
            team: None,
            readable: Vec::new(),
            settings: SlackSettings::default(),
        });
    };
    let settings: SlackSettings = serde_json::from_str(&conn.settings).unwrap_or_default();
    #[derive(Deserialize)]
    struct Ch {
        channel_id: String,
        name: String,
    }
    let readable: Vec<SlackChannelRef> = if read_channels {
        let rows: Vec<Ch> = q(
            db,
            "SELECT channel_id, name FROM slack_channels WHERE workspace_id = ? AND team_id = ? AND readable = 1 AND private = 0 ORDER BY name",
            &[s(workspace), s(&conn.team_id)],
        )?
        .all()
        .await?;
        rows.into_iter()
            .map(|r| SlackChannelRef {
                id: r.channel_id,
                name: r.name,
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Reached {
        thread,
        team: Some(conn.team_id.clone()),
        readable,
        settings,
    })
}

/// Takes the session's place among its calls of `kind` in the last hour,
/// in one statement: `false` when it made its most.
pub(crate) async fn take_place(
    db: &D1Database,
    session_id: &str,
    kind: CallKind,
    now: i64,
) -> AppResult<bool> {
    let added = q(
        db,
        "INSERT INTO slack_calls (session_id, kind, at) SELECT ?1, ?2, ?3 WHERE \
         (SELECT COUNT(*) FROM slack_calls WHERE session_id = ?1 AND kind = ?2 AND at > ?3 - 3600000) < ?4",
        &[
            s(session_id),
            s(kind.as_str()),
            n(now),
            n(i64::from(access::limit_of(kind))),
        ],
    )?
    .run()
    .await?;
    Ok(added > 0)
}

async fn limit(db: &D1Database, session_id: &str, kind: CallKind, now: i64) -> AppResult<()> {
    if take_place(db, session_id, kind, now).await? {
        Ok(())
    } else {
        Err(AppError(ApiError::new(
            ErrorCode::RateLimited,
            access::limited(kind),
        )))
    }
}

/// `AgentCall::Slack`: the answer, as JSON.
pub async fn call(ctx: &Ctx, session: &RunnerSession, request: Value) -> AppResult<Value> {
    let request: SlackRequest = serde_json::from_value(request).map_err(|_| {
        AppError(ApiError::bad_request(
            "This control plane doesn't know that `fctl slack` command: its sandbox's fctl is newer.",
        ))
    })?;
    if session.review {
        return Err(forbidden("Review agents reach nothing in Slack."));
    }
    let row = crate::spec::session_row(&ctx.db, &session.id)
        .await?
        .ok_or_else(|| crate::error::not_found("Session"))?;
    let session_slack = row
        .slack
        .as_deref()
        .and_then(|j| serde_json::from_str::<SessionSlack>(j).ok());
    let r = reached(
        &ctx.db,
        &session.id,
        &session.workspace_id,
        session.settings.slack.read_channels,
        session_slack,
    )
    .await?;
    let Some(team) = r.team.clone() else {
        return Err(forbidden(
            "This session's workspace works in no Slack workspace: its admins connect one under Integrations.",
        ));
    };
    if r.thread.is_none() && r.readable.is_empty() {
        return Err(forbidden(
            "This session reaches nothing in Slack: it didn't start there, and its workspace makes no channel readable for its agents.",
        ));
    }
    let keys = ctx.keys()?;
    let slack = super::client(&ctx.db, &ctx.cfg, &keys, &team).await?;
    let answer = match request {
        SlackRequest::About => SlackAnswer::About {
            about: SlackAbout {
                team_name: Some(slack.team_name.clone()),
                thread: r.thread.as_ref().map(|(s, _)| s.clone()),
                readable: r.readable.clone(),
                search: r.settings.search && !r.readable.is_empty(),
                reply: r.settings.agent_replies && r.thread.is_some(),
            },
        },
        SlackRequest::Thread { all } => {
            let Some((own, t)) = r.thread.as_ref() else {
                return Err(forbidden(
                    "This session didn't start in Slack, so it has no thread there: `fctl slack history` and `read` read the readable channels.",
                ));
            };
            limit(&ctx.db, &session.id, CallKind::Read, ctx.now).await?;
            read_thread(ctx, &slack, own, t, all).await?
        }
        SlackRequest::Read { link } => {
            let Some(l) = sl::link(&link) else {
                return Err(AppError(ApiError::bad_request(
                    "That isn't a link to a Slack message: copy one with Copy link, https://….slack.com/archives/C…/p…",
                )));
            };
            if l.domain.is_some() && slack.domain.is_some() && l.domain != slack.domain {
                return Err(forbidden(
                    "That message is in another Slack workspace than this session's.",
                ));
            }
            limit(&ctx.db, &session.id, CallKind::Read, ctx.now).await?;
            let c = slack.conversation(&l.channel).await?;
            let readable: Vec<String> = r.readable.iter().map(|c| c.id.clone()).collect();
            let reach = Reach {
                thread: r
                    .thread
                    .as_ref()
                    .map(|(own, _)| (own.channel.as_str(), own.thread_ts.as_str())),
                readable: &readable,
            };
            access::may_read(&reach, &c, Reading::Thread(l.thread())).map_err(AppError)?;
            let (messages, more) = slack.thread(&l.channel, l.thread(), None, 100, 3).await?;
            let names = names(&slack, &messages).await;
            let views = views(
                &slack,
                &l.channel,
                c.name.as_deref(),
                &messages,
                &names,
                None,
                &[],
            );
            SlackAnswer::Messages {
                channel: c.name.clone().map(|name| SlackChannelRef {
                    id: l.channel.clone(),
                    name,
                }),
                messages: views,
                more,
                note: None,
            }
        }
        SlackRequest::History {
            channel,
            since,
            limit: max,
        } => {
            let channels: Vec<(String, String)> = r
                .readable
                .iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect();
            let Some(id) = access::find_channel(&channel, &channels).map(str::to_owned) else {
                return Err(forbidden(&format!(
                    "{} isn't readable for this session's agents: {}",
                    channel.trim(),
                    readable_names(&r.readable)
                )));
            };
            limit(&ctx.db, &session.id, CallKind::Read, ctx.now).await?;
            let c = slack.conversation(&id).await?;
            let readable: Vec<String> = r.readable.iter().map(|c| c.id.clone()).collect();
            let reach = Reach {
                thread: None,
                readable: &readable,
            };
            access::may_read(&reach, &c, Reading::Channel).map_err(AppError)?;
            let oldest = since.map(sl::millis_ts);
            let max = max.unwrap_or(50).clamp(1, 200);
            let (mut messages, more) = slack.history(&id, oldest.as_deref(), max).await?;
            messages.reverse();
            let names = names(&slack, &messages).await;
            SlackAnswer::Messages {
                channel: Some(SlackChannelRef {
                    id: id.clone(),
                    name: c.name.clone().unwrap_or_default(),
                }),
                messages: views(&slack, &id, c.name.as_deref(), &messages, &names, None, &[]),
                more,
                note: None,
            }
        }
        SlackRequest::Search {
            query,
            channel,
            limit: max,
        } => {
            if !r.settings.search {
                return Err(forbidden(
                    "This workspace doesn't let agents search Slack: its admins choose that under Integrations → Slack.",
                ));
            }
            if r.readable.is_empty() {
                return Err(forbidden(
                    "No channel is readable for this session's agents, so there's nothing to search.",
                ));
            }
            let query = query.trim();
            if query.is_empty() || query.chars().count() > 200 {
                return Err(AppError(ApiError::bad_request(
                    "Say what to look for, in at most 200 characters.",
                )));
            }
            limit(&ctx.db, &session.id, CallKind::Search, ctx.now).await?;
            let within: Vec<SlackChannelRef> = match channel.as_deref() {
                Some(c) => {
                    let channels: Vec<(String, String)> = r
                        .readable
                        .iter()
                        .map(|c| (c.id.clone(), c.name.clone()))
                        .collect();
                    match access::find_channel(c, &channels) {
                        Some(id) => r.readable.iter().filter(|x| x.id == id).cloned().collect(),
                        None => {
                            return Err(forbidden(&format!(
                                "{} isn't readable for this session's agents: {}",
                                c.trim(),
                                readable_names(&r.readable)
                            )));
                        }
                    }
                }
                None => r.readable.clone(),
            };
            let token = r
                .thread
                .as_ref()
                .and_then(|(_, t)| super::outbound::action_token(t));
            search(
                &slack,
                query,
                &within,
                &r.readable,
                token.as_deref(),
                max.unwrap_or(20).clamp(1, 50),
                ctx.now,
            )
            .await?
        }
        SlackRequest::Reply { text: words } => {
            let Some((own, _)) = r.thread.as_ref() else {
                return Err(forbidden(
                    "This session didn't start in Slack, so it has no thread to post in: fctl posts in no other conversation.",
                ));
            };
            if !r.settings.agent_replies {
                return Err(forbidden(
                    "This workspace doesn't let agents post in Slack themselves: fctl posts your turn's last message in the thread.",
                ));
            }
            let words = words.trim();
            if words.is_empty() {
                return Err(AppError(ApiError::bad_request("Say something to post.")));
            }
            if words.chars().count() > 3_000 {
                return Err(AppError(ApiError::bad_request(
                    "Post at most 3,000 characters: the rest belongs in your turn's last message, which fctl posts with a link to the session.",
                )));
            }
            limit(&ctx.db, &session.id, CallKind::Post, ctx.now).await?;
            let url = ctx.cfg.url(&format!("/s/{}", session.id));
            let (b, fallback) = blocks::reply(words, &url);
            let ts = match slack
                .post(&own.channel, Some(&own.thread_ts), &fallback, Some(&b))
                .await
            {
                Err(super::Failure::Slack(c)) if c.starts_with("invalid_blocks") => {
                    slack
                        .post(
                            &own.channel,
                            Some(&own.thread_ts),
                            &fallback,
                            Some(&blocks::reply_mrkdwn(words, &url)),
                        )
                        .await?
                }
                other => other?,
            };
            let link = slack.link(&own.channel, &ts, Some(&own.thread_ts)).await;
            super::outbound::notice(
                &ctx.env,
                &session.id,
                "slack_posted",
                "Posted in Slack".to_owned(),
                SlackNotice {
                    place: own.place,
                    channel_name: own.channel_name.clone(),
                    body: Some(words.to_owned()),
                    url: Some(link.clone()),
                    messages: None,
                },
            )
            .await;
            SlackAnswer::Posted {
                ts,
                url: Some(link),
            }
        }
    };
    Ok(serde_json::to_value(answer)?)
}

fn readable_names(readable: &[SlackChannelRef]) -> String {
    if readable.is_empty() {
        return "no channel is.".into();
    }
    let names: Vec<String> = readable.iter().map(|c| format!("#{}", c.name)).collect();
    format!("it may read {}.", names.join(", "))
}

/// The session's thread: what's new since its agent last read it, or all
/// of it. Notes what it read up to.
async fn read_thread(
    ctx: &Ctx,
    slack: &Slack,
    own: &SessionSlack,
    t: &ThreadRow,
    all: bool,
) -> AppResult<SlackAnswer> {
    let c = slack.conversation(&own.channel).await?;
    let reach = Reach {
        thread: Some((own.channel.as_str(), own.thread_ts.as_str())),
        readable: &[],
    };
    access::may_read(&reach, &c, Reading::Thread(&own.thread_ts)).map_err(AppError)?;
    let since = if all { None } else { t.read_ts.clone() };
    // What's new since it last read, first; all of it, page after page.
    let found = slack
        .thread(&own.channel, &own.thread_ts, since.as_deref(), 200, 5)
        .await;
    let (messages, more) = match found {
        Ok(m) => m,
        Err(super::Failure::Limited(after)) => {
            return Ok(SlackAnswer::Messages {
                channel: None,
                messages: Vec::new(),
                more: false,
                note: Some(format!(
                    "Slack limits how often fctl reads threads here: try again in {after} seconds."
                )),
            });
        }
        Err(e) => return Err(e.into()),
    };
    // fctl's own messages (the card, its replies) say nothing new.
    let theirs: Vec<Value> = messages
        .into_iter()
        .filter(|m| {
            m.get("user").and_then(Value::as_str) != Some(slack.bot_user.as_str())
                && m.get("subtype")
                    .and_then(Value::as_str)
                    .is_none_or(|s| s != "bot_message" || m.get("bot_id").is_some())
        })
        .collect();
    let newest = theirs
        .iter()
        .filter_map(|m| m.get("ts").and_then(Value::as_str))
        .max()
        .map(str::to_owned);
    let names = names(slack, &theirs).await;
    let delivered = t.delivered();
    let mut views = views(
        slack,
        &own.channel,
        own.channel_name.as_deref(),
        &theirs,
        &names,
        since.as_deref(),
        &delivered,
    );
    if !all {
        views.retain(|v| v.new);
    }
    if let Some(newest) = newest
        && t.read_ts.as_deref().is_none_or(|r| newest.as_str() > r)
    {
        q(
            &ctx.db,
            "UPDATE slack_threads SET read_ts = ? WHERE id = ?",
            &[s(&newest), s(&t.id)],
        )?
        .run()
        .await?;
    }
    let note = (!all && views.is_empty()).then(|| {
        "Nothing new in the thread since you last read it: `fctl slack thread --all` shows all of it."
            .to_owned()
    });
    Ok(SlackAnswer::Messages {
        channel: own.channel_name.clone().map(|name| SlackChannelRef {
            id: own.channel.clone(),
            name,
        }),
        messages: views,
        more,
        note,
    })
}

/// The names of the people who wrote `messages`, or whom they mention, by
/// their Slack id: at most 15 asked of Slack.
async fn names(slack: &Slack, messages: &[Value]) -> BTreeMap<String, SlackPerson> {
    let mut ids: Vec<String> = Vec::new();
    for m in messages {
        if let Some(u) = m.get("user").and_then(Value::as_str)
            && !ids.iter().any(|i| i == u)
        {
            ids.push(u.to_owned());
        }
        for u in text::mentioned(m.get("text").and_then(Value::as_str).unwrap_or_default()) {
            if !ids.contains(&u) {
                ids.push(u);
            }
        }
    }
    let mut out = BTreeMap::new();
    out.insert(
        slack.bot_user.clone(),
        SlackPerson {
            name: "fctl".into(),
            user: Some(slack.bot_user.clone()),
            handle: None,
            avatar_url: None,
            bot: true,
        },
    );
    for id in ids.into_iter().filter(|i| *i != slack.bot_user).take(15) {
        if let Ok(u) = slack.user(&id).await {
            out.insert(
                id.clone(),
                SlackPerson {
                    name: u.name,
                    user: Some(id),
                    handle: u.handle,
                    avatar_url: None,
                    bot: u.bot,
                },
            );
        }
    }
    out
}

/// Slack's messages as `fctl slack` shows them, oldest first: new since
/// `since`, passed on to the agent where `delivered` has them.
fn views(
    slack: &Slack,
    channel: &str,
    channel_name: Option<&str>,
    messages: &[Value],
    names: &BTreeMap<String, SlackPerson>,
    since: Option<&str>,
    delivered: &[String],
) -> Vec<SlackMessageView> {
    let name_of = |id: &str| names.get(id).map(|p| p.name.clone());
    messages
        .iter()
        .filter(|m| {
            m.get("subtype").and_then(Value::as_str).is_none_or(|s| {
                matches!(
                    s,
                    "bot_message" | "thread_broadcast" | "file_share" | "me_message"
                )
            })
        })
        .filter_map(|m| {
            let ts = m.get("ts").and_then(Value::as_str)?.to_owned();
            let user = m.get("user").and_then(Value::as_str);
            let by = match user.and_then(|u| names.get(u)) {
                Some(p) => p.clone(),
                None => SlackPerson {
                    name: m
                        .pointer("/bot_profile/name")
                        .or_else(|| m.pointer("/user_profile/real_name"))
                        .or_else(|| m.get("username"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| user.map(|u| format!("@{u}")))
                        .unwrap_or_else(|| "an app".into()),
                    user: user.map(str::to_owned),
                    handle: None,
                    avatar_url: None,
                    bot: m.get("bot_id").is_some(),
                },
            };
            let thread_ts = m
                .get("thread_ts")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some(SlackMessageView {
                at: sl::ts_millis(&ts).unwrap_or_default(),
                text: text::plain(
                    m.get("text").and_then(Value::as_str).unwrap_or_default(),
                    &name_of,
                ),
                channel_name: channel_name.map(str::to_owned),
                url: Some(match slack.domain.as_deref() {
                    Some(d) => sl::permalink(Some(d), channel, &ts, thread_ts.as_deref()),
                    None => sl::permalink(None, channel, &ts, thread_ts.as_deref()),
                }),
                replies: m
                    .get("reply_count")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .filter(|n| *n > 0),
                new: since.is_none_or(|s| ts.as_str() > s),
                delivered: delivered.contains(&ts),
                files: m
                    .get("files")
                    .and_then(Value::as_array)
                    .map(|f| {
                        f.iter()
                            .filter_map(|f| {
                                f.get("name")
                                    .or_else(|| f.get("title"))
                                    .and_then(Value::as_str)
                            })
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default(),
                thread_ts,
                ts,
                by,
            })
        })
        .collect()
}

/// Messages of the readable channels `within` that say `query`, newest
/// first: through Slack's Real-time Search, where the thread's event gave
/// fctl a token for it, keeping only what it found in those channels; else
/// in their last week, read here.
/// How many readable channels a search reads itself at most, where
/// Slack's Real-time Search doesn't answer for them.
const SEARCHED_CHANNELS: usize = 10;

/// The readable channels' checks, as Slack has each now, as history checks
/// it: one made private, shared with another organization or archived since
/// it was made readable isn't searched. Each is asked once.
struct Checks<'a> {
    slack: &'a Slack,
    readable: Vec<String>,
    seen: std::collections::HashMap<String, Check>,
}

#[derive(Clone)]
enum Check {
    /// Agents may read it: its name now.
    Open(String),
    /// They may not, any more.
    Closed,
    /// Slack didn't say just now.
    Unknown,
}

impl<'a> Checks<'a> {
    fn new(slack: &'a Slack, readable: &[SlackChannelRef]) -> Self {
        Self {
            slack,
            readable: readable.iter().map(|c| c.id.clone()).collect(),
            seen: std::collections::HashMap::new(),
        }
    }

    async fn check(&mut self, id: &str) -> Check {
        if let Some(c) = self.seen.get(id) {
            return c.clone();
        }
        let reach = Reach {
            thread: None,
            readable: &self.readable,
        };
        let c = match self.slack.conversation(id).await {
            Ok(conv) => match access::may_read(&reach, &conv, Reading::Channel) {
                Ok(()) => Check::Open(conv.name.unwrap_or_default()),
                Err(_) => Check::Closed,
            },
            Err(super::Failure::Slack(code))
                if code == "channel_not_found" || code == "not_in_channel" =>
            {
                Check::Closed
            }
            Err(_) => Check::Unknown,
        };
        self.seen.insert(id.to_owned(), c.clone());
        c
    }
}

/// Searches `within`, readable channels of `readable`: with Slack's
/// Real-time Search where the session has an action token, keeping what it
/// finds in channels agents may read now; else, or where it finds nothing
/// there, by reading the channels' last week itself, ten at most.
async fn search(
    slack: &Slack,
    query: &str,
    within: &[SlackChannelRef],
    readable: &[SlackChannelRef],
    token: Option<&str>,
    max: u32,
    now: i64,
) -> AppResult<SlackAnswer> {
    let mut checks = Checks::new(slack, readable);
    let mut unanswered = false;
    // Slack's Real-time Search ranks every public channel's messages: those
    // outside the channels searched drop, and so do those of channels agents
    // may no longer read.
    if let Some(token) = token
        && let Ok(found) = slack.search(query, token, max).await
    {
        let mut out: Vec<SlackMessageView> = Vec::new();
        for m in found
            .pointer("/results/messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let channel = m
                .get("channel_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(c) = within.iter().find(|c| c.id == channel) else {
                continue;
            };
            let name = match checks.check(channel).await {
                Check::Open(name) if !name.is_empty() => name,
                Check::Open(_) => c.name.clone(),
                Check::Closed => continue,
                Check::Unknown => {
                    unanswered = true;
                    continue;
                }
            };
            let ts = m
                .get("message_ts")
                .and_then(Value::as_str)
                .unwrap_or_default();
            out.push(SlackMessageView {
                ts: ts.to_owned(),
                at: sl::ts_millis(ts).unwrap_or_default(),
                by: SlackPerson {
                    name: m
                        .get("author_name")
                        .and_then(Value::as_str)
                        .unwrap_or("someone")
                        .to_owned(),
                    user: m
                        .get("author_user_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    handle: None,
                    avatar_url: None,
                    bot: m
                        .get("is_author_bot")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                },
                text: text::plain(
                    m.get("content").and_then(Value::as_str).unwrap_or_default(),
                    &|_| None,
                ),
                channel_name: Some(name),
                url: m
                    .get("permalink")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                thread_ts: m
                    .get("thread_ts")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                replies: None,
                new: false,
                delivered: false,
                files: Vec::new(),
            });
        }
        if !out.is_empty() {
            return Ok(SlackAnswer::Messages {
                channel: None,
                messages: out,
                more: false,
                note: unanswered.then(|| UNANSWERED.to_owned()),
            });
        }
    }
    // Read here: each channel's last week, the words all there.
    let words: Vec<String> = query
        .split_whitespace()
        .map(|w| w.trim_matches('"').to_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    let oldest = sl::millis_ts(now - 7 * 24 * 60 * 60 * 1000);
    let mut out: Vec<SlackMessageView> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let (mut read, mut closed) = (0, 0);
    for c in within.iter().take(SEARCHED_CHANNELS) {
        let name = match checks.check(&c.id).await {
            Check::Open(name) => name,
            Check::Closed => {
                closed += 1;
                continue;
            }
            Check::Unknown => {
                unanswered = true;
                continue;
            }
        };
        let (messages, _) = match slack.history(&c.id, Some(&oldest), 200).await {
            Ok(m) => m,
            Err(super::Failure::Limited(after)) => {
                notes.push(format!(
                    "Slack limits how often fctl reads channels here: some weren't searched. Try again in {after} seconds."
                ));
                continue;
            }
            Err(super::Failure::Down(_)) => {
                unanswered = true;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        read += 1;
        let hits: Vec<Value> = messages
            .into_iter()
            .filter(|m| {
                let t = m
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                words.iter().all(|w| t.contains(w.as_str()))
            })
            .collect();
        let names = names(slack, &hits).await;
        let shown = if name.is_empty() { &c.name } else { &name };
        out.extend(views(slack, &c.id, Some(shown), &hits, &names, None, &[]));
    }
    if read == 0 && notes.is_empty() {
        // Nothing could be read: say why, apart from what Slack didn't answer.
        return Err(if unanswered {
            AppError(ApiError::new(
                ErrorCode::Unavailable,
                "Slack didn't answer fctl just now, so nothing was searched: try again in a minute.",
            ))
        } else if closed > 0 {
            forbidden(
                "None of the channels this session's agents may read can be read now: they were made private, shared with another organization or archived since.",
            )
        } else {
            forbidden(
                "No channel is readable for this session's agents, so there's nothing to search.",
            )
        });
    }
    if unanswered {
        notes.push(UNANSWERED.to_owned());
    }
    if within.len() > SEARCHED_CHANNELS {
        notes.push(format!(
            "fctl read the first {SEARCHED_CHANNELS} of {} readable channels: name one with --channel to search another.",
            within.len()
        ));
    }
    out.sort_by_key(|a| std::cmp::Reverse(a.at));
    let more = out.len() > max as usize;
    out.truncate(max as usize);
    notes.push(
        "fctl searched the readable channels' last week, for messages with all of those words."
            .into(),
    );
    Ok(SlackAnswer::Messages {
        channel: None,
        messages: out,
        more,
        note: Some(notes.join(" ")),
    })
}

/// What a search says where Slack didn't say whether some channels are
/// still readable.
const UNANSWERED: &str = "Slack didn't answer for some channels just now, so they weren't searched: try again in a minute.";

/// What a session's agent reaches in Slack and is told of it
/// (`SessionSpec::slack`): `None` where it reaches nothing there.
pub async fn spec(
    db: &D1Database,
    session_id: &str,
    workspace: &str,
    project: &fctl_proto::session::ProjectSettings,
    session_slack: Option<SessionSlack>,
) -> AppResult<Option<fctl_proto::slack::SlackSpec>> {
    let r = reached(
        db,
        session_id,
        workspace,
        project.slack.read_channels,
        session_slack,
    )
    .await?;
    if r.team.is_none() || (r.thread.is_none() && r.readable.is_empty()) {
        return Ok(None);
    }
    let thread = r.thread.as_ref().map(|(s, _)| s.clone());
    let search = r.settings.search && !r.readable.is_empty();
    let reply = r.settings.agent_replies && thread.is_some();
    let instructions = fctl_domain::slack::task::instructions(&fctl_domain::slack::task::Reach {
        thread: thread.as_ref(),
        readable: &r.readable,
        search,
        reply,
    });
    Ok(Some(fctl_proto::slack::SlackSpec {
        without_cli: thread.as_ref().map(fctl_domain::slack::task::without_cli),
        thread,
        readable: r.readable,
        search,
        reply,
        instructions,
    }))
}

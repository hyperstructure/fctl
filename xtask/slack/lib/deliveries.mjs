// What Slack sends fctl's app, shaped as Slack sends it (docs.slack.dev/apis/
// events-api, /interactivity, /interactivity/implementing-slash-commands)
// and signed as Slack signs it (`v0=` and the HMAC-SHA256 of
// `v0:<timestamp>:<body>` with the app's signing secret): events in an
// `event_callback` envelope, as JSON; interactions as a form whose
// `payload` is JSON; `/fctl` as a form of its fields.

import { createHmac, randomBytes } from "node:crypto";
import { viewJson } from "./api.mjs";
import * as W from "./world.mjs";

/** The verification token Slack still puts in what it sends, deprecated: apps ignore it. */
export const VERIFICATION_TOKEN = "fctlDevVerificationTok3n";

/** The events fctl's app subscribes to (fctl_domain::slack::manifest::BOT_EVENTS). */
export const BOT_EVENTS = [
  "agent_session_stopped",
  "app_home_opened",
  "app_mention",
  "app_uninstalled",
  "message.channels",
  "message.groups",
  "message.im",
  "tokens_revoked",
];

/** `X-Slack-Signature` for `body` at `timestamp` (seconds): `v0=` and the hex HMAC-SHA256 of `v0:<timestamp>:<body>`. */
export function sign(secret, timestamp, body) {
  return `v0=${createHmac("sha256", secret).update(`v0:${timestamp}:`).update(body).digest("hex")}`;
}

const digits = (n) => [...randomBytes(n)].map((b) => b % 10).join("");

/** A new trigger id, of Slack's shape. */
export const triggerId = () => `${digits(10)}.${digits(12)}.${randomBytes(16).toString("hex")}`;

/** A `ts` for now, as events' `event_ts` and `action_ts` are. */
export function eventTs(now = Date.now()) {
  return `${Math.floor(now / 1000)}.${String((now % 1000) * 1000 + Math.floor(Math.random() * 1000)).padStart(6, "0")}`;
}

// ───────────────────────────── events ─────────────────────────────

/** The `event_callback` envelope of an event: one installation that sees it, fctl's bot. */
export function envelope(w, event, { shared = false } = {}) {
  const context = { et: event.type, tid: w.team.id, aid: W.APP_ID, cid: event.channel ?? null };
  return {
    token: VERIFICATION_TOKEN,
    team_id: w.team.id,
    context_team_id: w.team.id,
    context_enterprise_id: null,
    api_app_id: W.APP_ID,
    event,
    type: "event_callback",
    event_id: W.newId(w, "Ev", 11),
    event_time: Math.floor(Date.now() / 1000),
    authorizations: [
      {
        enterprise_id: null,
        team_id: w.team.id,
        user_id: W.BOT_USER,
        is_bot: true,
        is_enterprise_install: false,
      },
    ],
    is_ext_shared_channel: shared,
    event_context: `4-${Buffer.from(JSON.stringify(context)).toString("base64url")}`,
  };
}

/** What Slack adds of a person to their messages in a channel shared with another organization. */
function sharedFields(w, m) {
  const u = W.user(w, m.user);
  if (!u) return {};
  return {
    user_team: w.team.id,
    source_team: w.team.id,
    user_profile: {
      avatar_hash: u.id.toLowerCase().slice(1, 13),
      image_72: W.avatar(u.id, 72),
      first_name: u.first_name,
      real_name: u.real_name,
      display_name: u.is_bot ? "" : u.name,
      team: w.team.id,
      name: u.name,
      is_restricted: u.is_restricted,
      is_ultra_restricted: u.is_ultra_restricted,
    },
  };
}

/** A `message` event (`message.channels`, `.groups`, `.im`) of message `m`. */
export function messageEvent(w, m, { actionToken } = {}) {
  const c = W.conversation(w, m.channel);
  const { reactions, ...json } = W.messageJson(w, m);
  return {
    ...json,
    ...(c.is_ext_shared && !m.bot_id ? sharedFields(w, m) : {}),
    channel: m.channel,
    event_ts: m.ts,
    channel_type: W.channelType(c),
    ...(actionToken ? { action_token: actionToken } : {}),
  };
}

/** An `app_mention` event of message `m`: no `channel_type`, as Slack sends it. */
export function mentionEvent(w, m, { actionToken } = {}) {
  const c = W.conversation(w, m.channel);
  const reply = m.thread_ts && m.thread_ts !== m.ts ? W.message(w, m.channel, m.thread_ts) : null;
  return {
    ...(m.client_msg_id ? { client_msg_id: m.client_msg_id } : {}),
    type: "app_mention",
    text: m.text,
    user: m.user,
    ts: m.ts,
    team: w.team.id,
    ...(m.blocks ? { blocks: m.blocks } : {}),
    ...(m.files ? { files: m.files } : {}),
    ...(c.is_ext_shared ? sharedFields(w, m) : {}),
    channel: m.channel,
    event_ts: m.ts,
    ...(reply ? { thread_ts: m.thread_ts, parent_user_id: reply.user } : {}),
    ...(actionToken ? { action_token: actionToken } : {}),
  };
}

/** A `message_changed` event: the message as it is now, and as it was. */
export function changedEvent(w, m, before) {
  const c = W.conversation(w, m.channel);
  const ts = eventTs();
  const { reactions, ...after } = W.messageJson(w, m);
  return {
    type: "message",
    subtype: "message_changed",
    message: after,
    previous_message: before,
    channel: m.channel,
    hidden: true,
    ts,
    event_ts: ts,
    channel_type: W.channelType(c),
  };
}

/** A `message_deleted` event. */
export function deletedEvent(w, m, before) {
  const c = W.conversation(w, m.channel);
  const ts = eventTs();
  return {
    type: "message",
    subtype: "message_deleted",
    hidden: true,
    deleted_ts: m.ts,
    previous_message: before,
    channel: m.channel,
    ts,
    event_ts: ts,
    channel_type: W.channelType(c),
  };
}

/** `app_home_opened`: someone opened the app's Home tab, with its view once one was published. */
export function homeOpened(w, userId, view) {
  return {
    type: "app_home_opened",
    user: userId,
    channel: W.dmOf(w, userId)?.id,
    tab: "home",
    event_ts: eventTs(),
    ...(view ? { view: viewJson(w, view) } : {}),
  };
}

/** `agent_session_stopped`: someone pressed stop on a thread's agent session. */
export function sessionStopped(channel, threadTs, userId, streaming) {
  return {
    type: "agent_session_stopped",
    channel,
    thread_ts: threadTs,
    user: userId,
    streaming_message_ts: streaming,
    event_ts: eventTs(),
  };
}

export const appUninstalled = () => ({ type: "app_uninstalled" });

/** `tokens_revoked`: whose tokens went, by their user ids. */
export const tokensRevoked = (gone) => ({
  type: "tokens_revoked",
  tokens: { oauth: gone.oauth, bot: gone.bot },
});

/** Slack's check of the events address as it is set: the answer must carry the challenge. */
export const urlVerification = (challenge) => ({
  token: VERIFICATION_TOKEN,
  challenge,
  type: "url_verification",
});

// ───────────────────────────── interactions ─────────────────────────────

const userRef = (w, u) => ({ id: u.id, username: u.name, name: u.name, team_id: w.team.id });
const teamRef = (w) => ({ id: w.team.id, domain: w.team.domain });
const channelRef = (c) => ({ id: c.id, name: c.is_im ? "directmessage" : c.name });

const base = (w, type, user, more) => ({
  type,
  user: userRef(w, user),
  api_app_id: W.APP_ID,
  token: VERIFICATION_TOKEN,
  ...more,
  team: teamRef(w),
  enterprise: null,
  is_enterprise_install: false,
});

/**
 * `block_actions`: someone used a button or a menu, in a message (`message`,
 * `ephemeral` when only they see it, which Slack sends without the message)
 * or in a view (`view`: the Home tab, a modal). `state` is what the
 * message's or view's inputs hold.
 */
export function blockActions(
  w,
  { user, actions, triggerId: trigger, responseUrl, state, message, ephemeral, view },
) {
  if (view) {
    return base(w, "block_actions", user, {
      container: { type: "view", view_id: view.id },
      trigger_id: trigger,
      view: { ...viewJson(w, view), state: { values: state } },
      actions,
    });
  }
  const c = W.conversation(w, message.channel);
  const { reactions, ...json } = ephemeral ? {} : W.messageJson(w, message);
  return base(w, "block_actions", user, {
    container: {
      type: "message",
      message_ts: message.ts,
      channel_id: c.id,
      is_ephemeral: Boolean(ephemeral),
    },
    trigger_id: trigger,
    channel: channelRef(c),
    ...(ephemeral ? {} : { message: json }),
    state: { values: state },
    response_url: responseUrl,
    actions,
  });
}

/** An action as `block_actions` gives it: a button's value, or what a menu has chosen. */
export function actionOf(element, blockId, chosen) {
  const at = { action_id: element.action_id, block_id: blockId, type: element.type, action_ts: eventTs() };
  switch (element.type) {
    case "button":
      return {
        ...at,
        text: element.text,
        ...(element.value !== undefined ? { value: element.value } : {}),
        ...(element.style ? { style: element.style } : {}),
      };
    case "static_select":
      return {
        ...at,
        selected_option: chosen ?? null,
        ...(element.placeholder ? { placeholder: element.placeholder } : {}),
      };
    case "checkboxes":
    case "multi_static_select":
      return { ...at, selected_options: chosen ?? [] };
    case "radio_buttons":
      return { ...at, selected_option: chosen ?? null };
    default:
      return at;
  }
}

/** `view_submission`: someone sent a modal, with what its inputs hold. */
export function viewSubmission(w, { user, view, values, triggerId: trigger }) {
  return base(w, "view_submission", user, {
    trigger_id: trigger,
    view: { ...viewJson(w, view), state: { values } },
    response_urls: [],
  });
}

/** `message_action`: a message shortcut (Send to fctl…) on message `message`. */
export function messageAction(w, { user, message, callbackId, triggerId: trigger, responseUrl }) {
  const c = W.conversation(w, message.channel);
  const { reactions, ...json } = W.messageJson(w, message);
  return {
    type: "message_action",
    token: VERIFICATION_TOKEN,
    action_ts: eventTs(),
    team: teamRef(w),
    user: userRef(w, user),
    channel: channelRef(c),
    is_enterprise_install: false,
    enterprise: null,
    callback_id: callbackId,
    trigger_id: trigger,
    response_url: responseUrl,
    message_ts: message.ts,
    message: json,
  };
}

/** `shortcut`: a global shortcut (Start an fctl session): no channel, no response_url. */
export function shortcut(w, { user, callbackId, triggerId: trigger }) {
  return {
    type: "shortcut",
    token: VERIFICATION_TOKEN,
    action_ts: eventTs(),
    team: teamRef(w),
    user: { id: user.id, username: user.name, team_id: w.team.id },
    is_enterprise_install: false,
    enterprise: null,
    callback_id: callbackId,
    trigger_id: trigger,
  };
}

/** `/fctl`, as the form Slack posts to the command's address. */
export function command(w, { user, channel, text, responseUrl, triggerId: trigger }) {
  const name = channel.is_im ? "directmessage" : channel.is_private ? "privategroup" : channel.name;
  return new URLSearchParams({
    token: VERIFICATION_TOKEN,
    team_id: w.team.id,
    team_domain: w.team.domain,
    channel_id: channel.id,
    channel_name: name,
    user_id: user.id,
    user_name: user.name,
    command: "/fctl",
    text,
    api_app_id: W.APP_ID,
    is_enterprise_install: "false",
    response_url: responseUrl,
    trigger_id: trigger,
  });
}

// ───────────────────────────── sending ─────────────────────────────

/**
 * Sends `body` to `url` as Slack does, signed with the app's `secret`:
 * fctl's answer (`status` 0 when it didn't answer within `timeoutMs`, with
 * `reason`, as Slack's retries name it), how long it took, and whether it
 * asked Slack not to try again (`X-Slack-No-Retry`).
 */
export async function deliver(url, secret, body, contentType, { timeoutMs = 3000, retry = null } = {}) {
  const timestamp = String(Math.floor(Date.now() / 1000));
  const headers = {
    "Content-Type": contentType,
    "User-Agent": "Slackbot 1.0 (+https://api.slack.com/robots)",
    "X-Slack-Request-Timestamp": timestamp,
    "X-Slack-Signature": sign(secret, timestamp, body),
  };
  if (retry) {
    headers["X-Slack-Retry-Num"] = String(retry.num);
    headers["X-Slack-Retry-Reason"] = retry.reason;
  }
  const started = Date.now();
  try {
    const res = await fetch(url, { method: "POST", headers, body, signal: AbortSignal.timeout(timeoutMs) });
    const text = await res.text();
    return {
      status: res.status,
      body: text,
      ms: Date.now() - started,
      contentType: res.headers.get("content-type") ?? "",
      noRetry: res.headers.get("x-slack-no-retry") === "1",
    };
  } catch (e) {
    const timedOut = e?.name === "TimeoutError" || e?.name === "AbortError";
    return {
      status: 0,
      body: timedOut ? `no answer within ${timeoutMs} ms` : String(e?.cause?.code ?? e?.message ?? e),
      ms: Date.now() - started,
      reason: timedOut ? "http_timeout" : "connection_failed",
      noRetry: false,
    };
  }
}

/** Why Slack tries an event again, in its words (`X-Slack-Retry-Reason`). */
export const retryReason = (answer) =>
  answer.reason ?? (answer.status >= 300 ? "http_error" : "unknown_error");

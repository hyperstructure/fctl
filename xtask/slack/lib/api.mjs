// Slack's Web API, as the stand-in answers it (docs.slack.dev/reference/
// methods): the methods fctl calls, with Slack's arguments, the scopes each
// needs, who may see and post where (fctl's bot sees the conversations it is
// in, and public channels' names), and Slack's errors, which come as HTTP 200
// `{ ok: false, error }`. Each method takes the request's context (`ctx`:
// the world, the token's record, what the call sets off) and its arguments,
// and returns Slack's answer or throws a `SlackError`.

import { randomBytes } from "node:crypto";
import * as B from "./blocks.mjs";
import * as W from "./world.mjs";

/** An error Slack answers with: its code, and what it adds (`needed`, `response_metadata`…). */
export class SlackError extends Error {
  constructor(error, extra = {}) {
    super(error);
    this.error = error;
    this.extra = extra;
  }
}

const refuse = (error, extra) => {
  throw new SlackError(error, extra);
};

/** `invalid_arguments`, with Slack's words for what is wrong. */
const invalid = (...messages) =>
  refuse("invalid_arguments", { response_metadata: { messages: messages.map((m) => `[ERROR] ${m}`) } });

/**
 * Errors that say fctl sent Slack something it shouldn't, rather than
 * that Acme is as it is (a channel fctl isn't in, a message someone
 * deleted): the stand-in counts them as problems.
 */
export const BAD_REQUESTS = new Set([
  "bad_client_secret",
  "bad_redirect_uri",
  "cant_update_message",
  "exchanged_trigger_id",
  "expired_trigger_id",
  "invalid_action_token",
  "invalid_arguments",
  "invalid_auth",
  "invalid_blocks",
  "invalid_blocks_format",
  "invalid_client_id",
  "invalid_code",
  "invalid_cursor",
  "invalid_grant_type",
  "invalid_json",
  "invalid_limit",
  "invalid_name",
  "invalid_status",
  "invalid_thread_ts",
  "invalid_trigger_id",
  "json_not_object",
  "markdown_text_conflict",
  "message_not_owned_by_app",
  "missing_scope",
  "msg_too_long",
  "no_text",
  "not_allowed_token_type",
  "not_authed",
  "standin_unsupported",
  "streaming_mode_mismatch",
  "thread_ts_required",
  "unknown_method",
]);

/** Methods that take a JSON body (docs.slack.dev: "application/json"); the rest ignore one, as Slack does. */
export const JSON_METHODS = new Set([
  "agents.sessions.setStatus",
  "assistant.search.context",
  "auth.test",
  "chat.appendStream",
  "chat.postEphemeral",
  "chat.postMessage",
  "chat.startStream",
  "chat.stopStream",
  "chat.update",
  "conversations.join",
  "conversations.open",
  "reactions.add",
  "views.open",
  "views.publish",
]);

/** How long a trigger opens a view: Slack's three seconds, once. */
export const TRIGGER_TTL_MS = 3000;

/** How long a stream lasts from `chat.startStream`, and how much markdown it holds in all. */
export const STREAM_MS = 10 * 60 * 1000;
export const STREAM_CHARS = 12_000;

// ───────────────────────────── arguments ─────────────────────────────

const str = (v) => {
  if (v === undefined || v === null) return undefined;
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return undefined;
};
const bool = (v) => v === true || v === 1 || v === "true" || v === "1";

/** A whole number argument, `fallback` without one, `NaN` for what isn't one. */
function int(v, fallback) {
  if (v === undefined || v === null || v === "") return fallback;
  const n = Number(v);
  return Number.isInteger(n) ? n : Number.NaN;
}

/** An argument that is JSON in a form and itself in a JSON body (`blocks`, `view`). */
function parsed(v) {
  if (typeof v !== "string") return v;
  try {
    return JSON.parse(v);
  } catch {
    return Symbol.for("unparsed");
  }
}

/** A `ts` as a number of microseconds, to compare. */
function micros(ts) {
  const [s, us = ""] = String(ts).split(".");
  if (!/^\d+$/.test(s) || !/^\d*$/.test(us)) return null;
  return BigInt(s) * 1_000_000n + BigInt(us.padEnd(6, "0").slice(0, 6) || "0");
}

const encodeCursor = (offset) => Buffer.from(`next:${offset}`).toString("base64");
function decodeCursor(cursor) {
  if (!cursor) return 0;
  const m = /^next:(\d+)$/.exec(Buffer.from(String(cursor), "base64").toString());
  if (!m) refuse("invalid_cursor");
  return Number(m[1]);
}

// ───────────────────────────── who sees what ─────────────────────────────

function need(ctx, scope) {
  if (!ctx.record.scopes.includes(scope)) {
    refuse("missing_scope", { needed: scope, provided: ctx.record.scopes.join(",") });
  }
}

/** The scope a conversation's kind needs to look it up (`read`) or read it (`history`). */
function scopeFor(c, what) {
  const kind = c.is_im ? "im" : c.is_mpim ? "mpim" : c.is_private ? "groups" : "channels";
  return `${kind}:${what}`;
}

/** A conversation the token may know of: every public one, and the private ones and DMs it is in. */
function find(ctx, id) {
  const c = W.conversation(ctx.w, str(id) ?? "");
  if (!c) refuse("channel_not_found");
  if ((c.is_im || c.is_private) && !W.isMember(c, ctx.record.user)) refuse("channel_not_found");
  return c;
}

/** The token's user is in the conversation, as reading and posting there need. */
function inside(ctx, c) {
  if (!W.isMember(c, ctx.record.user))
    refuse(c.is_private || c.is_im ? "channel_not_found" : "not_in_channel");
}

/** A conversation as `conversations.info` and `.list` give it. */
export function conversationJson(w, c, viewer, { members = false } = {}) {
  const updated = c.created * 1000;
  if (c.is_im) {
    return {
      id: c.id,
      created: c.created,
      is_archived: false,
      is_im: true,
      is_org_shared: false,
      context_team_id: w.team.id,
      updated,
      user: c.user,
      is_user_deleted: false,
      priority: 0,
      is_open: true,
    };
  }
  const topic = (value) => ({ value, creator: value ? c.creator : "", last_set: value ? c.created : 0 });
  return {
    id: c.id,
    name: c.name,
    is_channel: c.is_channel,
    is_group: c.is_group,
    is_im: false,
    is_mpim: false,
    is_private: c.is_private,
    created: c.created,
    is_archived: c.is_archived,
    is_general: false,
    unlinked: 0,
    name_normalized: c.name,
    is_shared: c.is_ext_shared,
    is_org_shared: false,
    is_pending_ext_shared: false,
    pending_shared: [],
    context_team_id: w.team.id,
    updated,
    parent_conversation: null,
    creator: c.creator,
    is_ext_shared: c.is_ext_shared,
    shared_team_ids: c.is_ext_shared ? [w.team.id, "T0GLOBEX"] : [w.team.id],
    ...(c.is_ext_shared ? { connected_team_ids: ["T0GLOBEX"], conversation_host_id: w.team.id } : {}),
    pending_connected_team_ids: [],
    is_member: W.isMember(c, viewer),
    topic: topic(c.topic),
    purpose: topic(c.purpose),
    properties: {},
    previous_names: [],
    ...(members ? { num_members: c.members.length } : {}),
  };
}

/** A time zone's name and offset now, as `users.info` gives them. */
function zone(tz, now = new Date()) {
  const part = (style) =>
    new Intl.DateTimeFormat("en-US", { timeZone: tz, timeZoneName: style })
      .formatToParts(now)
      .find((p) => p.type === "timeZoneName")?.value ?? "";
  const m = /GMT([+-])(\d+)(?::(\d+))?/.exec(part("shortOffset"));
  const offset = m ? (m[1] === "-" ? -1 : 1) * (Number(m[2]) * 3600 + Number(m[3] ?? 0) * 60) : 0;
  return { tz_label: part("long"), tz_offset: offset };
}

/** Someone as `users.info` gives them: their address only with `users:read.email`. */
export function userJson(w, u, { email = false, locale = false } = {}) {
  const images = {};
  for (const size of [24, 32, 48, 72, 192, 512, 1024]) images[`image_${size}`] = W.avatar(u.id, size);
  const profile = {
    title: "",
    phone: "",
    skype: "",
    real_name: u.real_name,
    real_name_normalized: u.real_name,
    display_name: u.is_bot ? "" : u.name,
    display_name_normalized: u.is_bot ? "" : u.name,
    fields: null,
    status_text: "",
    status_emoji: "",
    status_emoji_display_info: [],
    status_expiration: 0,
    avatar_hash: u.id.toLowerCase().slice(1, 13),
    ...(email && u.email ? { email: u.email } : {}),
    first_name: u.first_name,
    last_name: u.last_name,
    ...images,
    image_original: W.avatar(u.id, 1024),
    is_custom_image: true,
    ...(u.is_bot ? { bot_id: u.bot_id, api_app_id: u.app_id, always_active: true } : {}),
    team: w.team.id,
  };
  return {
    id: u.id,
    team_id: w.team.id,
    name: u.name,
    deleted: u.deleted,
    color: "9f69e7",
    real_name: u.real_name,
    tz: u.tz,
    ...zone(u.tz),
    profile,
    is_admin: u.is_admin,
    is_owner: u.is_owner,
    is_primary_owner: false,
    is_restricted: u.is_restricted,
    is_ultra_restricted: u.is_ultra_restricted,
    is_bot: u.is_bot,
    is_app_user: false,
    updated: 1_740_000_000,
    is_email_confirmed: !u.is_bot,
    who_can_share_contact_card: "EVERYONE",
    ...(locale ? { locale: "en-US" } : {}),
  };
}

// ───────────────────────────── what fctl posts ─────────────────────────────

/** A message's `blocks` argument as Slack takes it, or its refusal. */
function blocksArg(v, surface = "message") {
  if (v === undefined || v === null || v === "") return undefined;
  const b = parsed(v);
  if (b === Symbol.for("unparsed") || !Array.isArray(b)) refuse("invalid_blocks_format");
  const problems = B.blockProblems(b, surface);
  if (problems.length) {
    const messages = B.messages(problems);
    refuse("invalid_blocks", {
      errors: messages.map((m) => m.replace(/^\[ERROR\] /, "")),
      response_metadata: { messages },
    });
  }
  return b;
}

/** A message's words: `text`, `blocks` or `markdown_text`, as Slack takes them together. */
function content(ctx, args) {
  const text = str(args.text);
  const markdown = str(args.markdown_text);
  const blocks = blocksArg(args.blocks);
  if (markdown !== undefined && (text !== undefined || blocks !== undefined))
    refuse("markdown_text_conflict");
  if (markdown !== undefined && [...markdown].length > 12_000) refuse("msg_too_long");
  if (!text && !markdown && !blocks?.length) refuse("no_text");
  let t = text ?? "";
  if ([...t].length > 40_000) {
    t = [...t].slice(0, 40_000).join("");
    ctx.warnings.push("message_truncated");
  }
  if (markdown !== undefined) return { text: markdown, blocks: [{ type: "markdown", text: markdown }] };
  return { text: t, blocks };
}

/** Where a message goes: a conversation's id, or someone's id for their messages with the app. */
function postTarget(ctx, channel) {
  const id = str(channel) ?? "";
  if (/^[UW]/.test(id)) {
    const u = W.user(ctx.w, id);
    const dm = u && !u.is_bot && W.dmOf(ctx.w, id);
    if (!dm) refuse("channel_not_found");
    return dm;
  }
  const c = find(ctx, id);
  if (!W.isMember(c, ctx.record.user)) {
    // `chat:write.public` lets an app post in public channels it isn't in.
    const open = !c.is_private && !c.is_im && ctx.record.scopes.includes("chat:write.public");
    if (!open) refuse(c.is_private ? "channel_not_found" : "not_in_channel");
  }
  if (c.is_archived) refuse("is_archived");
  return c;
}

/** The thread a reply goes in, as `thread_ts` names it: Slack wants its first message's ts. */
function threadOf(ctx, c, threadTs) {
  if (threadTs === undefined || threadTs === "") return undefined;
  const parent = W.message(ctx.w, c.id, threadTs);
  if (!parent) {
    ctx.problems.push(`thread_ts ${threadTs} names no message in ${c.id}: Slack posts it as a new message`);
    return undefined;
  }
  const root = W.rootOf(ctx.w, parent);
  if (root !== parent) {
    ctx.problems.push(`thread_ts ${threadTs} is a reply's: Slack wants its thread's first message's ts`);
  }
  return root.ts;
}

/** An app's message as Slack posts it, and the event it sends apps in the conversation. */
function post(ctx, c, input) {
  const m = W.addMessage(ctx.w, {
    channel: c.id,
    user: ctx.record.user,
    bot: ctx.record.kind === "bot" ? W.BOT_ID : undefined,
    app: W.APP_ID,
    text: input.text,
    blocks: input.blocks ? B.withIds(input.blocks) : undefined,
    thread_ts: input.thread_ts,
    subtype: input.broadcast ? "thread_broadcast" : undefined,
  });
  if (input.stream) m.stream = input.stream;
  ctx.changed = true;
  // Slack sends an app its own messages too: fctl must tell them from people's.
  if (!input.stream) ctx.events.push({ kind: "message", channel: c.id, ts: m.ts });
  return m;
}

// ───────────────────────────── the methods ─────────────────────────────

export const METHODS = {
  "auth.test": (ctx) => {
    const u = W.user(ctx.w, ctx.record.user);
    return {
      ok: true,
      url: `https://${ctx.w.team.domain}.slack.com/`,
      team: ctx.w.team.name,
      user: u.name,
      team_id: ctx.w.team.id,
      user_id: u.id,
      ...(ctx.record.kind === "bot" ? { bot_id: W.BOT_ID } : {}),
      is_enterprise_install: false,
    };
  },

  "team.info": (ctx, args) => {
    need(ctx, "team:read");
    const team = str(args.team);
    if (team && team !== ctx.w.team.id) refuse("team_not_found");
    const icon = { image_default: false };
    for (const size of [34, 44, 68, 88, 102, 132, 230]) icon[`image_${size}`] = W.avatar(ctx.w.team.id, size);
    icon.image_original = W.avatar(ctx.w.team.id, 1024);
    return {
      ok: true,
      team: {
        id: ctx.w.team.id,
        name: ctx.w.team.name,
        url: `https://${ctx.w.team.domain}.slack.com/`,
        domain: ctx.w.team.domain,
        email_domain: "acme.test",
        icon,
        avatar_base_url: "https://ca.slack-edge.com/",
        is_verified: false,
        lob_sales_home_enabled: false,
      },
    };
  },

  "users.info": (ctx, args) => {
    need(ctx, "users:read");
    const u = W.user(ctx.w, str(args.user) ?? "");
    if (!u) refuse("user_not_found");
    const email = ctx.record.scopes.includes("users:read.email");
    return { ok: true, user: userJson(ctx.w, u, { email, locale: bool(args.include_locale) }) };
  },

  "conversations.info": (ctx, args) => {
    const c = find(ctx, args.channel);
    need(ctx, scopeFor(c, "read"));
    return {
      ok: true,
      channel: conversationJson(ctx.w, c, ctx.record.user, { members: bool(args.include_num_members) }),
    };
  },

  "conversations.list": (ctx, args) => {
    const types = (str(args.types) || "public_channel").split(",").map((t) => t.trim());
    const known = {
      public_channel: "channels:read",
      private_channel: "groups:read",
      im: "im:read",
      mpim: "mpim:read",
    };
    for (const t of types) {
      if (!known[t]) invalid(`unknown conversation type ${t}`);
      need(ctx, known[t]);
    }
    const limit = int(args.limit, 100);
    if (Number.isNaN(limit) || limit < 1 || limit >= 1000) refuse("invalid_limit");
    const me = ctx.record.user;
    const all = ctx.w.conversations.filter(
      (c) =>
        (types.includes("public_channel") && !c.is_im && !c.is_private) ||
        (types.includes("private_channel") && c.is_private && W.isMember(c, me)) ||
        (types.includes("im") && c.is_im && W.isMember(c, me)),
    );
    const from = decodeCursor(str(args.cursor));
    const page = all.slice(from, from + limit);
    const next = from + limit < all.length ? encodeCursor(from + limit) : "";
    // Slack filters after it pages: a page may come short.
    const shown = page.filter((c) => !(bool(args.exclude_archived) && c.is_archived));
    return {
      ok: true,
      channels: shown.map((c) => conversationJson(ctx.w, c, me, { members: true })),
      response_metadata: { next_cursor: next },
    };
  },

  "conversations.replies": (ctx, args) => {
    const c = find(ctx, args.channel);
    need(ctx, scopeFor(c, "history"));
    inside(ctx, c);
    const ts = str(args.ts);
    if (!ts) invalid("missing required field: ts");
    const m = W.message(ctx.w, c.id, ts);
    if (!m) refuse("thread_not_found");
    const root = W.rootOf(ctx.w, m);
    const [parent, ...rest] = W.thread(ctx.w, root);
    const replies = rest.filter(inWindow(args));
    const limit = Math.min(int(args.limit, 1000), ctx.maxLimit ?? 1000);
    if (Number.isNaN(limit) || limit < 1) refuse("invalid_limit");
    const from = decodeCursor(str(args.cursor));
    // Every page starts with the thread's first message, then as many of
    // its replies as the page holds besides.
    const per = Math.max(limit - 1, 1);
    const page = replies.slice(from, from + per);
    const more = from + per < replies.length;
    return {
      ok: true,
      messages: [parent, ...page].map((x) => W.messageJson(ctx.w, x)),
      has_more: more,
      ...(more ? { response_metadata: { next_cursor: encodeCursor(from + per) } } : {}),
    };
  },

  "conversations.history": (ctx, args) => {
    const c = find(ctx, args.channel);
    need(ctx, scopeFor(c, "history"));
    inside(ctx, c);
    const top = ctx.w.messages
      .filter(
        (m) =>
          m.channel === c.id && (!m.thread_ts || m.thread_ts === m.ts || m.subtype === "thread_broadcast"),
      )
      .filter(inWindow(args))
      .reverse();
    const limit = Math.min(int(args.limit, 100), 999, ctx.maxLimit ?? 999);
    if (Number.isNaN(limit) || limit < 1) refuse("invalid_limit");
    const from = decodeCursor(str(args.cursor));
    const page = top.slice(from, from + limit);
    const more = from + limit < top.length;
    return {
      ok: true,
      messages: page.map((m) => W.messageJson(ctx.w, m)),
      has_more: more,
      pin_count: 0,
      channel_actions_ts: null,
      channel_actions_count: 0,
      ...(more ? { response_metadata: { next_cursor: encodeCursor(from + limit) } } : {}),
    };
  },

  "conversations.join": (ctx, args) => {
    need(ctx, "channels:join");
    const c = find(ctx, args.channel);
    if (c.is_im || c.is_private) refuse("method_not_supported_for_channel_type");
    if (c.is_archived) refuse("is_archived");
    const already = W.isMember(c, ctx.record.user);
    if (already) ctx.warnings.push("already_in_channel");
    else {
      W.join(ctx.w, c, ctx.record.user);
      ctx.changed = true;
      ctx.notes.push(`fctl joined #${c.name}`);
    }
    return { ok: true, channel: conversationJson(ctx.w, c, ctx.record.user) };
  },

  "conversations.open": (ctx, args) => {
    need(ctx, "im:write");
    const channel = str(args.channel);
    if (channel) {
      const c = find(ctx, channel);
      return { ok: true, no_op: true, already_open: true, channel: { id: c.id } };
    }
    const users = (str(args.users) ?? "")
      .split(",")
      .map((u) => u.trim())
      .filter(Boolean);
    if (users.length === 0) refuse("users_list_not_supplied");
    if (users.length > 1) {
      ctx.problems.push(
        "conversations.open with several people opens a group DM, which the stand-in doesn't have",
      );
      refuse("standin_unsupported");
    }
    const u = W.user(ctx.w, users[0]);
    if (!u) refuse("user_not_found");
    if (u.is_bot) refuse("cannot_dm_bot");
    const dm = W.dmOf(ctx.w, u.id);
    const full = bool(args.return_im);
    return {
      ok: true,
      no_op: true,
      already_open: true,
      channel: full ? conversationJson(ctx.w, dm, ctx.record.user) : { id: dm.id },
    };
  },

  "chat.postMessage": (ctx, args) => {
    need(ctx, "chat:write");
    const c = postTarget(ctx, args.channel);
    const { text, blocks } = content(ctx, args);
    const thread_ts = threadOf(ctx, c, str(args.thread_ts));
    const m = post(ctx, c, { text, blocks, thread_ts, broadcast: thread_ts && bool(args.reply_broadcast) });
    return { ok: true, channel: c.id, ts: m.ts, message: W.messageJson(ctx.w, m) };
  },

  "chat.update": (ctx, args) => {
    need(ctx, "chat:write");
    const c = find(ctx, args.channel);
    inside(ctx, c);
    const m = W.message(ctx.w, c.id, str(args.ts) ?? "");
    if (!m) refuse("message_not_found");
    if (m.user !== ctx.record.user) refuse("cant_update_message");
    const text = str(args.text);
    const markdown = str(args.markdown_text);
    const blocks = blocksArg(args.blocks);
    if (markdown !== undefined && (text !== undefined || blocks !== undefined))
      refuse("markdown_text_conflict");
    if (text === undefined && blocks === undefined && markdown === undefined) refuse("no_text");
    const before = W.messageJson(ctx.w, m);
    if (markdown !== undefined) {
      m.text = markdown;
      m.blocks = [{ type: "markdown", text: markdown }];
    } else {
      if (text !== undefined) m.text = text;
      // Text without blocks takes the blocks away.
      if (blocks !== undefined) m.blocks = B.withIds(blocks);
      else delete m.blocks;
    }
    ctx.changed = true;
    ctx.events.push({ kind: "changed", channel: c.id, ts: m.ts, before });
    return { ok: true, channel: c.id, ts: m.ts, text: m.text, message: W.messageJson(ctx.w, m) };
  },

  "chat.postEphemeral": (ctx, args) => {
    need(ctx, "chat:write");
    const c = find(ctx, args.channel);
    inside(ctx, c);
    const to = W.user(ctx.w, str(args.user) ?? "");
    if (!to) refuse("user_not_found");
    if (!W.isMember(c, to.id)) refuse("user_not_in_channel");
    const { text, blocks } = content(ctx, args);
    let thread_ts = str(args.thread_ts) || undefined;
    if (thread_ts && !W.message(ctx.w, c.id, thread_ts)) {
      ctx.problems.push(`thread_ts ${thread_ts} names no message in ${c.id}`);
      thread_ts = undefined;
    }
    const e = W.addEphemeral(ctx.w, {
      channel: c.id,
      to: to.id,
      text,
      blocks: blocks && B.withIds(blocks),
      thread_ts,
    });
    ctx.changed = true;
    return { ok: true, message_ts: e.ts };
  },

  "chat.getPermalink": (ctx, args) => {
    const c = find(ctx, args.channel);
    const m = W.message(ctx.w, c.id, str(args.message_ts) ?? "");
    if (!m) refuse("message_not_found");
    return { ok: true, channel: c.id, permalink: W.permalink(ctx.w, c.id, m.ts, m.thread_ts) };
  },

  "reactions.add": (ctx, args) => {
    need(ctx, "reactions:write");
    const c = find(ctx, args.channel);
    inside(ctx, c);
    const name = str(args.name) ?? "";
    if (!/^[a-z0-9_+'-]+(::skin-tone-[2-6])?$/.test(name)) refuse("invalid_name");
    const ts = str(args.timestamp);
    if (!ts) refuse("bad_timestamp");
    const m = W.message(ctx.w, c.id, ts);
    if (!m) refuse("message_not_found");
    let r = m.reactions.find((x) => x.name === name);
    if (r?.users.includes(ctx.record.user)) refuse("already_reacted");
    if (!r) {
      r = { name, users: [] };
      m.reactions.push(r);
    }
    r.users.push(ctx.record.user);
    ctx.changed = true;
    return { ok: true };
  },

  "views.open": (ctx, args) => {
    const trigger = str(args.trigger_id) ?? str(args.interactivity_pointer);
    if (!trigger) invalid("missing required field: trigger_id");
    const t = ctx.triggers.get(trigger);
    if (!t) refuse("invalid_trigger_id");
    if (t.used) refuse("exchanged_trigger_id");
    if (Date.now() - t.at > (ctx.triggerTtlMs ?? TRIGGER_TTL_MS)) refuse("expired_trigger_id");
    const view = parsed(args.view);
    if (!view || view === Symbol.for("unparsed")) invalid("missing required field: view");
    if (view.type !== "modal") invalid("views.open opens modals only [json-pointer:/view/type]");
    const problems = B.viewProblems(view);
    if (problems.length)
      refuse("invalid_arguments", { response_metadata: { messages: B.messages(problems) } });
    t.used = true;
    // A new modal takes the place of one still open for them.
    for (const v of W.openModals(ctx.w, t.user)) {
      v.open = false;
      v.closed_at = new Date().toISOString();
      v.closed = "replaced";
    }
    const v = newView(ctx, view, t.user, trigger);
    return { ok: true, view: viewJson(ctx.w, v) };
  },

  "views.publish": (ctx, args) => {
    const userId = str(args.user_id);
    if (!userId) invalid("missing required field: user_id");
    const u = W.user(ctx.w, userId);
    if (!u || u.is_bot) refuse("user_not_found");
    const view = parsed(args.view);
    if (!view || view === Symbol.for("unparsed")) invalid("missing required field: view");
    if (view.type !== "home") invalid("views.publish publishes the Home tab only [json-pointer:/view/type]");
    const problems = B.viewProblems(view);
    if (problems.length)
      refuse("invalid_arguments", { response_metadata: { messages: B.messages(problems) } });
    const old = W.home(ctx.w, userId);
    const hash = str(args.hash);
    if (hash && old && hash !== old.hash) refuse("hash_conflict");
    if (old) ctx.w.views.splice(ctx.w.views.indexOf(old), 1);
    const v = newView(ctx, view, userId, null);
    return { ok: true, view: viewJson(ctx.w, v) };
  },

  "agents.sessions.setStatus": (ctx, args) => {
    need(ctx, "chat:write");
    const channel = str(args.channel_id);
    if (!channel) refuse("channel_not_found");
    const c = find(ctx, channel);
    if (!W.isMember(c, ctx.record.user)) refuse("not_authorized");
    const status = str(args.status);
    if (!["active", "processing", "suspended", "closed"].includes(status)) refuse("invalid_status");
    const thread_ts = str(args.thread_ts);
    if (!thread_ts) refuse("thread_ts_required");
    if (!W.message(ctx.w, c.id, thread_ts)) {
      ctx.problems.push(
        `agents.sessions.setStatus names thread ${thread_ts}, which has no message in ${c.id}`,
      );
    }
    const title = str(args.title);
    if (title !== undefined && [...title].length > 200) invalid("title must be at most 200 characters");
    const initiator = str(args.initiator_user_id);
    if (initiator && !W.user(ctx.w, initiator)) refuse("user_not_found");
    const s = W.setAgentSession(ctx.w, { channel: c.id, thread_ts, status, title, initiator });
    ctx.changed = true;
    return { ok: true, status: s.status, agent_status: s.status, title: s.title };
  },

  "chat.startStream": (ctx, args) => {
    need(ctx, "chat:write");
    const c = find(ctx, args.channel);
    inside(ctx, c);
    const thread_ts = str(args.thread_ts);
    if (!thread_ts) refuse("invalid_thread_ts");
    const root = W.message(ctx.w, c.id, thread_ts);
    if (!root) refuse("invalid_thread_ts");
    if (!c.is_im && !(str(args.recipient_user_id) && str(args.recipient_team_id))) {
      invalid("recipient_user_id and recipient_team_id are required to stream in a channel");
    }
    const { mode, text } = streamed(args);
    if ([...text].length > STREAM_CHARS) refuse("msg_too_long");
    const m = post(ctx, c, {
      text,
      thread_ts: W.rootOf(ctx.w, root).ts,
      stream: { state: "streaming", started_at: Date.now(), mode },
    });
    if (!W.agentSession(ctx.w, c.id, m.thread_ts)) {
      W.setAgentSession(ctx.w, {
        channel: c.id,
        thread_ts: m.thread_ts,
        status: "processing",
        initiator: W.rootOf(ctx.w, root).user,
      });
    }
    return { ok: true, channel: c.id, ts: m.ts };
  },

  "chat.appendStream": (ctx, args) => {
    need(ctx, "chat:write");
    const { c, m } = stream(ctx, args);
    if (m.stream.state === "stopped_by_user") refuse("stopped_by_user");
    if (m.stream.state !== "streaming") refuse("message_not_in_streaming_state");
    const { mode, text } = streamed(args);
    if (mode !== m.stream.mode) refuse("streaming_mode_mismatch");
    if ([...m.text].length + [...text].length > STREAM_CHARS) refuse("msg_too_long");
    m.text += text;
    ctx.changed = true;
    return { ok: true, channel: c.id, ts: m.ts };
  },

  "chat.stopStream": (ctx, args) => {
    need(ctx, "chat:write");
    const { c, m } = stream(ctx, args);
    if (m.stream.state !== "streaming" && m.stream.state !== "stopped_by_user") {
      refuse("message_not_in_streaming_state");
    }
    const status = str(args.session_status) ?? "active";
    if (!["active", "processing", "suspended", "closed"].includes(status)) refuse("invalid_status");
    if (args.markdown_text !== undefined || args.chunks !== undefined) {
      const { mode, text } = streamed(args);
      if (mode !== m.stream.mode) refuse("streaming_mode_mismatch");
      if ([...m.text].length + [...text].length > STREAM_CHARS) refuse("msg_too_long");
      m.text += text;
    }
    const blocks = blocksArg(args.blocks);
    if (blocks) m.blocks = B.withIds(blocks);
    m.stream.state = "stopped";
    m.stream.stopped_at = Date.now();
    W.setAgentSession(ctx.w, { channel: c.id, thread_ts: m.thread_ts, status });
    ctx.changed = true;
    return { ok: true, channel: c.id, ts: m.ts, message: W.messageJson(ctx.w, m) };
  },

  "assistant.search.context": (ctx, args) => {
    need(ctx, "search:read.public");
    const query = (str(args.query) ?? "").trim();
    if (!query) invalid("missing required field: query");
    const token = str(args.action_token);
    if (ctx.record.kind === "bot") {
      if (!token) invalid("missing required field: action_token");
      if (!ctx.w.actionTokens[token]) refuse("invalid_action_token");
    }
    const limit = int(args.limit, 20);
    if (Number.isNaN(limit) || limit < 1 || limit > 20) refuse("invalid_limit");
    const types = (str(args.channel_types) || "public_channel").split(",");
    const contentTypes = (str(args.content_types) || "messages").split(",");
    const words = query
      .toLowerCase()
      .split(/\s+/)
      .map((x) => x.replace(/^"|"$/g, ""))
      .filter(Boolean);
    const bots = bool(args.include_bots);
    const found = contentTypes.includes("messages")
      ? ctx.w.messages
          .filter((m) => {
            const c = W.conversation(ctx.w, m.channel);
            if (!types.includes("public_channel") || c.is_im || c.is_private || c.is_ext_shared) return false;
            if (m.subtype || (!bots && m.bot_id)) return false;
            const t = m.text.toLowerCase();
            return words.every((x) => t.includes(x));
          })
          .reverse()
          .slice(0, limit)
      : [];
    return {
      ok: true,
      results: {
        messages: found.map((m) => {
          const c = W.conversation(ctx.w, m.channel);
          return {
            author_user_id: m.user,
            author_name: W.user(ctx.w, m.user)?.real_name ?? "someone",
            team_id: ctx.w.team.id,
            channel_id: c.id,
            channel_name: c.name,
            message_ts: m.ts,
            ...(m.thread_ts && m.thread_ts !== m.ts ? { thread_ts: m.thread_ts } : {}),
            content: m.text,
            is_author_bot: Boolean(m.bot_id),
            permalink: W.permalink(ctx.w, c.id, m.ts, m.thread_ts),
            ...(bool(args.include_message_blocks) && m.blocks ? { blocks: m.blocks } : {}),
          };
        }),
      },
      response_metadata: { next_cursor: "" },
    };
  },
};

/** What a stream call adds: its markdown, or its chunks' (never both). */
function streamed(args) {
  const markdown = str(args.markdown_text);
  const chunks = parsed(args.chunks);
  if (markdown !== undefined && chunks !== undefined) invalid("markdown_text and chunks don't go together");
  if (chunks !== undefined) {
    if (!Array.isArray(chunks)) invalid("chunks must be an array");
    const text = chunks
      .filter((x) => x?.type === "markdown_text")
      .map((x) => x.text ?? "")
      .join("");
    return { mode: "chunks", text };
  }
  return { mode: "markdown", text: markdown ?? "" };
}

/** The stream a call names (`channel`, `ts`), past ten minutes over. */
function stream(ctx, args) {
  const c = find(ctx, args.channel);
  inside(ctx, c);
  const m = W.message(ctx.w, c.id, str(args.ts) ?? "");
  if (!m) refuse("message_not_found");
  if (!m.stream || m.user !== ctx.record.user) refuse("message_not_owned_by_app");
  timeOut(ctx.w, m);
  return { c, m };
}

/** A stream past Slack's ten minutes ends: what it said stays, and its agent session is active. */
export function timeOut(w, m, now = Date.now()) {
  if (m.stream?.state === "streaming" && now - m.stream.started_at > STREAM_MS) {
    m.stream.state = "timed_out";
    W.setAgentSession(w, { channel: m.channel, thread_ts: m.thread_ts, status: "active" });
  }
}

function newView(ctx, view, userId, trigger) {
  const id = W.newId(ctx.w, "V0", 11);
  const v = {
    id,
    team_id: ctx.w.team.id,
    type: view.type,
    blocks: B.withIds(view.blocks),
    private_metadata: view.private_metadata ?? "",
    callback_id: view.callback_id ?? "",
    state: { values: {} },
    hash: `${Math.floor(Date.now() / 1000)}.${randomBytes(4).toString("hex")}`,
    title: view.title ?? { type: "plain_text", text: "fctl", emoji: true },
    clear_on_close: Boolean(view.clear_on_close),
    notify_on_close: Boolean(view.notify_on_close),
    close: view.close ?? null,
    submit: view.submit ?? null,
    previous_view_id: null,
    root_view_id: id,
    app_id: W.APP_ID,
    external_id: view.external_id ?? "",
    app_installed_team_id: ctx.w.team.id,
    bot_id: W.BOT_ID,
    user: userId,
    trigger,
    open: view.type === "modal",
    at: new Date().toISOString(),
  };
  ctx.w.views.push(v);
  ctx.changed = true;
  ctx.notes.push(
    view.type === "modal"
      ? `fctl opened the form "${v.title.text}" for ${W.user(ctx.w, userId).real_name}`
      : `fctl published the Home tab for ${W.user(ctx.w, userId).real_name}`,
  );
  return v;
}

/** A view as Slack gives it to the app, without what the stand-in keeps of it. */
export function viewJson(_w, v) {
  const { user, trigger, open, at, closed, closed_at, errors, ...view } = v;
  return view;
}

/** Messages between `oldest` and `latest`, as `conversations.history` and `.replies` take them. */
function inWindow(args) {
  const oldest = args.oldest !== undefined && args.oldest !== "" ? micros(args.oldest) : null;
  const latest = args.latest !== undefined && args.latest !== "" ? micros(args.latest) : null;
  if (args.oldest !== undefined && args.oldest !== "" && oldest === null) refuse("invalid_ts_oldest");
  if (args.latest !== undefined && args.latest !== "" && latest === null) refuse("invalid_ts_latest");
  const inclusive = bool(args.inclusive);
  return (m) => {
    const t = micros(m.ts);
    if (oldest !== null && (inclusive ? t < oldest : t <= oldest)) return false;
    if (latest !== null && (inclusive ? t > latest : t >= latest)) return false;
    return true;
  };
}

/**
 * Answers a Web API call with a token (`ctx.record`): Slack's answer, a
 * `SlackError`'s, or `unknown_method` for what the stand-in doesn't fake.
 */
export function answer(ctx, method, args) {
  const run = METHODS[method];
  if (!run) {
    ctx.problems.push(
      `fctl called ${method}, which Slack doesn't have or the stand-in doesn't answer yet: add it to xtask/slack/lib/api.mjs`,
    );
    return { ok: false, error: "unknown_method" };
  }
  try {
    const body = run(ctx, args);
    if (ctx.warnings.length) {
      body.warning = ctx.warnings.join(",");
      body.response_metadata = { ...body.response_metadata, warnings: [...ctx.warnings] };
    }
    return body;
  } catch (e) {
    if (e instanceof SlackError) return { ok: false, error: e.error, ...e.extra };
    throw e;
  }
}

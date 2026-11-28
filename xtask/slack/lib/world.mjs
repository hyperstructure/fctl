// The Slack workspace the stand-in serves: Acme, with its people, channels
// and messages, fctl's bot, the tokens Slack handed out, and what fctl did
// there (its messages, the messages only one person sees, its views and
// its agent sessions). It is made up the first time and kept in the state
// directory from then on, as Slack keeps what fctl wrote there, so that the
// control plane's token and threads outlive a restart.

import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";

/** What a kept world must say to be read back; another starts afresh. */
const VERSION = 1;

export const TEAM = { id: "T0ACME", name: "Acme", domain: "acme" };

/** fctl's app in Acme: its bot user, which people mention, its bot and its app. */
export const BOT_USER = "U0FCTLBOT";
export const BOT_ID = "B0FCTL";
export const APP_ID = "A0FCTL";

/** The dev stack's owner, as `cargo xtask dev` signs in to fctl: the one who installs fctl's app. */
export const DEV_USER = "U0DEV";
export const GRACE = "U0GRACE";
export const GUEST = "U0GUEST";

/** Another app's bot, whose deploy notices fill #deploys. */
const DEPLOYS = "U0DEPLOYS";

const HOUR = 60 * 60 * 1000;
const MINUTE = 60 * 1000;

function person(id, handle, name, email, tz, more = {}) {
  const [first, ...rest] = name.split(" ");
  return {
    id,
    name: handle,
    real_name: name,
    first_name: first,
    last_name: rest.join(" "),
    email,
    tz,
    is_admin: false,
    is_owner: false,
    is_restricted: false,
    is_ultra_restricted: false,
    is_bot: false,
    deleted: false,
    ...more,
  };
}

function channel(id, name, members, flags = {}) {
  return {
    id,
    name,
    is_channel: true,
    is_group: false,
    is_im: false,
    is_mpim: false,
    is_private: false,
    is_ext_shared: false,
    is_archived: false,
    topic: "",
    purpose: "",
    creator: DEV_USER,
    members,
    ...flags,
  };
}

function dm(id, user) {
  return { id, is_im: true, user, members: [user, BOT_USER] };
}

/** Acme as it starts: three people, fctl's bot, channels of each kind, and a little history. */
export function seed(now = new Date()) {
  const created = Math.floor((now.getTime() - 400 * 24 * HOUR) / 1000);
  const users = [
    {
      id: BOT_USER,
      name: "fctl",
      real_name: "fctl",
      first_name: "",
      last_name: "",
      email: null,
      tz: "America/Los_Angeles",
      is_admin: false,
      is_owner: false,
      is_restricted: false,
      is_ultra_restricted: false,
      is_bot: true,
      bot_id: BOT_ID,
      app_id: APP_ID,
      deleted: false,
    },
    person(DEV_USER, "dev", "Dev Owner", "dev@fctl.test", "Europe/Stockholm"),
    person(GRACE, "grace", "Grace Hopper", "grace@acme.test", "America/New_York"),
    person(GUEST, "ada", "Ada Guest", "ada@guest.test", "Europe/London", { is_restricted: true }),
    {
      id: DEPLOYS,
      name: "deploys",
      real_name: "Deploys",
      first_name: "",
      last_name: "",
      email: null,
      tz: "America/Los_Angeles",
      is_admin: false,
      is_owner: false,
      is_restricted: false,
      is_ultra_restricted: false,
      is_bot: true,
      bot_id: "B0DEPLOYS",
      app_id: "A0DEPLOYS",
      deleted: false,
    },
  ];
  const conversations = [
    channel("C0WEB", "web-team", [DEV_USER, GRACE, GUEST], {
      topic: "The web app: its tests, its pull requests, its releases",
    }),
    channel("C0INC", "incidents", [DEV_USER, GRACE], { topic: "What's broken right now" }),
    channel("C0DEP", "deploys", [DEV_USER, GRACE, DEPLOYS], { topic: "Every deploy, as it happens" }),
    channel("C0RAND", "random", [DEV_USER, GRACE]),
    channel("G0SECRET", "secret", [DEV_USER, GRACE], {
      is_channel: false,
      is_group: true,
      is_private: true,
    }),
    channel("C0PART", "partners", [DEV_USER, GRACE], {
      is_ext_shared: true,
      topic: "Shared with Globex",
    }),
    dm("D0DEV", DEV_USER),
    dm("D0GRACE", GRACE),
    dm("D0GUEST", GUEST),
  ].map((c) => ({ ...c, created }));
  const w = {
    version: VERSION,
    team: { ...TEAM },
    users,
    conversations,
    messages: [],
    ephemeral: [],
    views: [],
    agentSessions: [],
    tokens: {},
    actionTokens: {},
    lastMicros: 0,
    counter: 0,
  };
  const at = (ago) => now.getTime() - ago;
  const say = (channel, user, text, ago, thread) =>
    addMessage(w, { channel, user, text, thread_ts: thread?.ts }, at(ago));
  // #incidents: a deploy that fails, and a thread about it.
  say("C0INC", DEV_USER, "Resolved: the queue backlog from last night cleared at 07:10.", 26 * HOUR);
  const failing = say(
    "C0INC",
    GRACE,
    'Deploys of web to production fail at the migrations step since this morning: `relation "slack_threads" already exists`. Can someone look?',
    3 * HOUR,
  );
  say(
    "C0INC",
    DEV_USER,
    "Looks like 0077 ran twice: the first deploy timed out after it applied.",
    170 * MINUTE,
    failing,
  );
  say("C0INC", GRACE, "It holds up the release of the new sign-in page.", 160 * MINUTE, failing);
  // #deploys: two days of deploys.
  const deploy = (text, ago) =>
    addMessage(w, { channel: "C0DEP", user: DEPLOYS, text, app: "A0DEPLOYS", bot: "B0DEPLOYS" }, at(ago));
  deploy("Deployed web build 409 to production.", 47 * HOUR);
  deploy("Deployed api 1.38.0 to staging.", 40 * HOUR);
  say("C0DEP", GRACE, "Holding production deploys until the sign-in fix lands.", 30 * HOUR);
  deploy("Rolled back web build 411 in production: the sign-in page answered 500.", 20 * HOUR);
  say("C0DEP", DEV_USER, "Deploying web build 412 to production.", 4 * HOUR);
  deploy("The deploy of web build 412 to production failed at the migrations step.", 3 * HOUR + 5 * MINUTE);
  // #web-team, #random and the others.
  const flaky = say("C0WEB", GRACE, "The flaky login test failed twice more on main today.", 26 * HOUR);
  say("C0WEB", DEV_USER, "I'll ask fctl to look at it.", 25 * HOUR, flaky);
  say("C0RAND", GRACE, "Lunch at the new ramen place on Friday?", 50 * HOUR);
  say("G0SECRET", DEV_USER, "The draft of next year's plan is in the shared drive.", 10 * HOUR);
  say("C0PART", DEV_USER, "Welcome to our shared channel with Globex!", 8 * HOUR);
  return w;
}

// ───────────────────────────── ids and times ─────────────────────────────

/**
 * A new `ts` (`1764000000.000100`: seconds and microseconds): for now, later
 * than every one before; for a time past (the seed's history), at that time.
 * Unique across Acme either way, so that a `ts` alone names a message.
 */
export function nextTs(w, ms = Date.now()) {
  let micros = Math.round(ms) * 1000;
  if (micros >= w.lastMicros - 1000) micros = Math.max(micros, w.lastMicros + 1);
  while (taken(w, fmt(micros))) micros += 1;
  w.lastMicros = Math.max(w.lastMicros, micros);
  return fmt(micros);
}

const fmt = (micros) => `${Math.floor(micros / 1e6)}.${String(micros % 1e6).padStart(6, "0")}`;
const taken = (w, ts) => w.messages.some((m) => m.ts === ts) || w.ephemeral.some((m) => m.ts === ts);

/** A `ts` as milliseconds since the epoch. */
export function tsMillis(ts) {
  const [s, us = "0"] = String(ts).split(".");
  return Number(s) * 1000 + Math.floor(Number(us.padEnd(6, "0").slice(0, 6)) / 1000);
}

/** A unique id of Slack's shape: `prefix` and upper-case letters and digits. */
export function newId(w, prefix, length = 9) {
  w.counter += 1;
  const n = `${Date.now().toString(36)}${w.counter.toString(36)}`.toUpperCase();
  return `${prefix}${n.slice(-(length - prefix.length)).padStart(length - prefix.length, "0")}`;
}

// ───────────────────────────── people and conversations ─────────────────────────────

export const user = (w, id) => w.users.find((u) => u.id === id);

/** A person (not a bot) by their id, handle, name or first name, as `as` names them; `guest` is Ada. */
export function personByName(w, name) {
  const n = String(name ?? "")
    .trim()
    .replace(/^@/, "")
    .toLowerCase();
  if (n === "guest") return user(w, GUEST);
  return w.users.find(
    (u) =>
      !u.is_bot &&
      (u.id === String(name).trim() ||
        u.name === n ||
        u.real_name.toLowerCase() === n ||
        u.first_name.toLowerCase() === n),
  );
}

export const conversation = (w, id) => w.conversations.find((c) => c.id === id);

/** A conversation by its id, or a channel by its name (with or without `#`). */
export function conversationByName(w, name) {
  const n = String(name ?? "")
    .trim()
    .replace(/^#/, "");
  return w.conversations.find((c) => c.id === n || (!c.is_im && c.name === n.toLowerCase()));
}

/** The direct messages of a person with fctl's app. */
export const dmOf = (w, userId) => w.conversations.find((c) => c.is_im && c.user === userId);

/** What kind of conversation it is, in a word: `public`, `private`, `shared` or `dm`. */
export function kind(c) {
  if (c.is_im) return "dm";
  if (c.is_ext_shared) return "shared";
  if (c.is_private) return "private";
  return "public";
}

export const isMember = (c, userId) => c.members.includes(userId);

/** The `channel_type` Slack gives a conversation's message events. */
export function channelType(c) {
  if (c.is_im) return "im";
  if (c.is_mpim) return "mpim";
  if (c.is_private) return "group";
  return "channel";
}

/** Someone joins a channel: a member, and Slack's line in it (`channel_join`). */
export function join(w, c, userId, inviter = null, ms = Date.now()) {
  if (isMember(c, userId)) return null;
  c.members.push(userId);
  const text = `<@${userId}> has joined the channel`;
  return addMessage(w, { channel: c.id, user: userId, text, subtype: "channel_join", inviter }, ms);
}

// ───────────────────────────── messages ─────────────────────────────

/**
 * Posts a message: a person's, or an app's (`bot`, `app`). A reply names
 * its thread's first message (`thread_ts`).
 */
export function addMessage(w, input, ms = Date.now()) {
  const ts = nextTs(w, ms);
  const m = {
    channel: input.channel,
    ts,
    user: input.user,
    text: input.text ?? "",
    ...(input.blocks ? { blocks: input.blocks } : {}),
    ...(input.thread_ts ? { thread_ts: input.thread_ts } : {}),
    ...(input.subtype ? { subtype: input.subtype } : {}),
    ...(input.inviter ? { inviter: input.inviter } : {}),
    ...(input.bot ? { bot_id: input.bot, app_id: input.app } : {}),
    ...(input.files?.length ? { files: input.files } : {}),
    ...(input.client_msg_id ? { client_msg_id: input.client_msg_id } : {}),
    reactions: [],
  };
  w.messages.push(m);
  return m;
}

export const message = (w, channel, ts) => w.messages.find((m) => m.channel === channel && m.ts === ts);

/** A message anywhere, by its `ts`, which the stand-in keeps unique across Acme. */
export const messageAnywhere = (w, ts) => w.messages.find((m) => m.ts === ts);

/** The first message of the thread `m` is in, or `m` itself. */
export function rootOf(w, m) {
  if (m.thread_ts && m.thread_ts !== m.ts) return message(w, m.channel, m.thread_ts) ?? m;
  return m;
}

/** The replies to a thread's first message, oldest first. */
export const replies = (w, root) =>
  w.messages.filter((m) => m.channel === root.channel && m.thread_ts === root.ts && m.ts !== root.ts);

/** A thread: its first message and its replies. */
export function thread(w, root) {
  return [root, ...replies(w, root)];
}

/**
 * A message as Slack's Web API gives it (`conversations.history`,
 * `.replies`, events): a thread's first message with what its replies
 * add up to, a reply with its thread's author, an app's with its bot.
 */
export function messageJson(w, m) {
  const out = { type: "message", user: m.user, text: m.text, ts: m.ts };
  if (m.subtype) out.subtype = m.subtype;
  if (m.inviter) out.inviter = m.inviter;
  if (m.client_msg_id) out.client_msg_id = m.client_msg_id;
  if (m.bot_id) {
    out.bot_id = m.bot_id;
    out.app_id = m.app_id;
    const bot = user(w, m.user);
    out.bot_profile = {
      id: m.bot_id,
      app_id: m.app_id,
      user_id: m.user,
      name: bot?.real_name ?? "an app",
      icons: { image_36: avatar(m.user, 36), image_48: avatar(m.user, 48), image_72: avatar(m.user, 72) },
      deleted: false,
      updated: Math.floor(tsMillis(m.ts) / 1000),
      team_id: TEAM.id,
    };
  }
  out.team = TEAM.id;
  if (m.blocks) out.blocks = m.blocks;
  if (m.files) out.files = m.files;
  if (m.edited) out.edited = m.edited;
  const thread = m.thread_ts && m.thread_ts !== m.ts ? message(w, m.channel, m.thread_ts) : null;
  if (thread) {
    out.thread_ts = m.thread_ts;
    out.parent_user_id = thread.user;
  } else {
    const rs = replies(w, m);
    if (rs.length > 0) {
      const users = [...new Set(rs.map((r) => r.user))];
      out.thread_ts = m.ts;
      out.reply_count = rs.length;
      out.reply_users_count = users.length;
      out.reply_users = users.slice(0, 5);
      out.latest_reply = rs.at(-1).ts;
      out.is_locked = false;
      out.subscribed = false;
    }
  }
  if (m.reactions.length) {
    out.reactions = m.reactions.map((r) => ({ name: r.name, users: r.users, count: r.users.length }));
  }
  return out;
}

/** A link to a message, as Slack writes it: in its thread when it is a reply. */
export function permalink(w, channel, ts, threadTs) {
  const p = `https://${w.team.domain}.slack.com/archives/${channel}/p${ts.replace(".", "")}`;
  return threadTs && threadTs !== ts ? `${p}?thread_ts=${threadTs}&cid=${channel}` : p;
}

/** Reads a link to a message (`https://acme.slack.com/archives/C…/p…`): its channel, ts and thread. */
export function readLink(link) {
  let u;
  try {
    u = new URL(String(link).trim());
  } catch {
    return null;
  }
  const m = /^\/archives\/([A-Z0-9]+)\/p(\d{7,})$/.exec(u.pathname);
  if (!m) return null;
  const ts = `${m[2].slice(0, -6)}.${m[2].slice(-6)}`;
  return { channel: m[1], ts, thread_ts: u.searchParams.get("thread_ts") ?? null };
}

/** A message only `to` sees, which Slack doesn't keep: in the stand-in's log, for them. */
export function addEphemeral(w, input, ms = Date.now()) {
  const e = {
    channel: input.channel,
    ts: nextTs(w, ms),
    to: input.to,
    text: input.text ?? "",
    ...(input.blocks ? { blocks: input.blocks } : {}),
    ...(input.thread_ts ? { thread_ts: input.thread_ts } : {}),
    via: input.via ?? "chat.postEphemeral",
    at: new Date(ms).toISOString(),
  };
  w.ephemeral.push(e);
  return e;
}

// ───────────────────────────── agent sessions and views ─────────────────────────────

/** Slack's agent session of a thread, where the app set one. */
export const agentSession = (w, channel, threadTs) =>
  w.agentSessions.find((s) => s.channel === channel && s.thread_ts === threadTs);

/** How long Slack keeps an agent session `processing` without hearing again. */
export const PROCESSING_MS = HOUR;

/** An agent session's status as Slack shows it: `processing` lapses to `active` after an hour. */
export function sessionStatus(s, now = Date.now()) {
  const last = s.history.at(-1);
  if (s.status === "processing" && last && now - Date.parse(last.at) > PROCESSING_MS) return "active";
  return s.status;
}

/** Sets a thread's agent session, made as it is first set: its title and initiator only then. */
export function setAgentSession(w, { channel, thread_ts, status, title, initiator }, ms = Date.now()) {
  let s = agentSession(w, channel, thread_ts);
  const at = new Date(ms).toISOString();
  if (!s) {
    s = {
      channel,
      thread_ts,
      status,
      title: title ?? null,
      initiator: initiator ?? null,
      created_at: at,
      stopped: [],
      history: [],
    };
    w.agentSessions.push(s);
  }
  s.status = status;
  s.history.push({ status, at });
  return s;
}

export const view = (w, id) => w.views.find((v) => v.id === id);

/** The modals open for someone, newest last, and the Home tab published for them. */
export const openModals = (w, userId) =>
  w.views.filter((v) => v.user === userId && v.type === "modal" && v.open);
export const home = (w, userId) => w.views.find((v) => v.user === userId && v.type === "home");

// ───────────────────────────── avatars ─────────────────────────────

/** Where the stand-in serves someone's picture (`/standin/avatar/<id>.svg`), set as it starts. */
let avatarBase = "http://127.0.0.1:8791";
export function setAvatarBase(base) {
  avatarBase = base;
}
export const avatar = (id, size) => `${avatarBase}/standin/avatar/${id}.svg?s=${size}`;

/** A picture of someone's initials, for `/standin/avatar/<id>.svg`. */
export function avatarSvg(w, id) {
  const u = user(w, id);
  const name = u?.real_name ?? (id === w.team.id ? w.team.name : "?");
  const initials = name
    .split(/\s+/)
    .map((p) => p[0] ?? "")
    .join("")
    .slice(0, 2)
    .toUpperCase();
  const colors = ["#007eff", "#ff00d8", "#171717", "#4cb782", "#f2994a"];
  const color = colors[[...id].reduce((a, c) => a + c.charCodeAt(0), 0) % colors.length];
  return `<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72" viewBox="0 0 72 72"><rect width="72" height="72" rx="12" fill="${color}"/><text x="36" y="45" font-family="system-ui,sans-serif" font-size="28" font-weight="600" fill="#fafafa" text-anchor="middle">${initials}</text></svg>`;
}

// ───────────────────────────── keeping it ─────────────────────────────

/** The world kept in `dir`, or a new one when there is none (or an older kind). */
export function load(dir) {
  const file = dir && path.join(dir, "world.json");
  if (file && existsSync(file)) {
    try {
      const kept = JSON.parse(readFileSync(file, "utf8"));
      if (kept.version === VERSION) return kept;
    } catch {
      // A world cut short as it was written: Acme starts afresh.
    }
  }
  return seed();
}

/** Keeps the world in `dir`: written whole, then moved into place. */
export function save(w, dir) {
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "world.json");
  writeFileSync(`${file}.new`, JSON.stringify(w, null, 2));
  renameSync(`${file}.new`, file);
}

/**
 * Text as Slack's client writes it when someone picks a mention: `@grace`
 * as `<@U0GRACE>`, `@fctl` as fctl's bot, and `#web-team` as
 * `<#C0WEB|web-team>` (a private channel's without its name).
 */
export function markup(w, text) {
  return String(text)
    .replace(/(^|[\s(])@([a-z0-9][a-z0-9_-]*)/gi, (all, pre, handle) => {
      const u = w.users.find((x) => x.name === handle.toLowerCase());
      return u ? `${pre}<@${u.id}>` : all;
    })
    .replace(/(^|[\s(])#([a-z0-9][a-z0-9_-]*)/gi, (all, pre, name) => {
      const c = w.conversations.find((x) => !x.is_im && x.name === name.toLowerCase());
      return c ? `${pre}<#${c.id}|${c.is_private ? "" : c.name}>` : all;
    });
}

/** Slack's markup as people read it: `<@U0GRACE>` as `@Grace Hopper`, `<#C0INC|incidents>` as `#incidents`. */
export function readable(w, text) {
  return String(text ?? "")
    .replace(/<@([A-Z0-9]+)(?:\|[^>]*)?>/g, (all, id) => {
      const u = user(w, id);
      return u ? `@${u.is_bot ? u.name : u.real_name}` : all;
    })
    .replace(/<#([A-Z0-9]+)(?:\|([^>]*))?>/g, (all, id, name) => {
      const c = conversation(w, id);
      return c?.name || name ? `#${c?.name ?? name}` : all;
    });
}

// The stand-in's HTTP server: Slack's endpoints (the install and sign-in
// pages, the Web API under /api/, the OpenID keys), the response_url fctl
// answers interactions and commands at (/standin/respond/<id>), and its
// own under /standin/, which act as people in Slack do (mention fctl, write
// to it, reply in a thread, press a button, run /fctl, use a shortcut, send
// a form, open the Home tab, press stop, uninstall the app) and send fctl
// what Slack would, signed, and which show what fctl did there. What Slack
// would refuse, or fctl failed to do in time, is a problem: printed as it
// happens, and listed at /standin/problems.

import { randomBytes, randomUUID } from "node:crypto";
import { appendFileSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import * as A from "./api.mjs";
import * as B from "./blocks.mjs";
import * as D from "./deliveries.mjs";
import * as O from "./oauth.mjs";
import * as W from "./world.mjs";

/** Slack waits this long for an answer to what it sends: an event it then tries again, a person sees an error. */
const ANSWER_WITHIN_MS = 3000;

/** Slack tries an event again at once, after a minute and after five (docs.slack.dev/apis/events-api). */
const RETRY_DELAYS_MS = [1000, 60_000, 300_000];

/** A response_url takes five messages within half an hour. */
const RESPONSE_USES = 5;
const RESPONSE_TTL_MS = 30 * 60 * 1000;

/** How long an action waits by default for what fctl does about it, where it does something at once. */
const WAIT_MS = 5000;

/** How many deliveries and calls the stand-in keeps to show; its log keeps them all. */
const KEPT = 5000;

/**
 * The stand-in, for fctl's app as `config` has it (`clientId`,
 * `clientSecret`, `signingSecret`, and fctl's addresses: `events`,
 * `interactivity`, `commands`), with its state kept in `state` (a
 * directory) when given.
 */
export function createStandin({
  state,
  config,
  out = console,
  retryDelaysMs = RETRY_DELAYS_MS,
  triggerTtlMs = A.TRIGGER_TTL_MS,
  answerWithinMs = ANSWER_WITHIN_MS,
  botEvents = D.BOT_EVENTS,
}) {
  const w = W.load(state);
  W.save(w, state);
  const key = O.signingKey();
  const codes = new Map();
  const triggers = new Map();
  const responses = new Map();
  const problems = [];
  const warnings = [];
  const warned = new Set();
  const deliveries = [];
  const calls = [];
  const inflight = new Set();
  const waiters = new Set();
  const switches = { rateLimit: null, fail: [] };
  let count = 0;
  let server;

  const base = () => config.self ?? `http://127.0.0.1:${server?.address()?.port ?? 8791}`;
  const save = () => W.save(w, state);
  const log = (entry) => {
    if (!state) return;
    mkdirSync(state, { recursive: true });
    appendFileSync(
      path.join(state, "log.jsonl"),
      `${JSON.stringify({ at: new Date().toISOString(), ...entry })}\n`,
    );
  };
  const problem = (kind, message, details = {}) => {
    problems.push({ at: new Date().toISOString(), kind, message, ...details });
    out.error(`[slack] problem: ${message}`);
  };
  const warn = (method, warning) => {
    warnings.push({ at: new Date().toISOString(), method, warning });
    if (!warned.has(`${method}:${warning}`)) {
      warned.add(`${method}:${warning}`);
      out.error(`[slack] warning: Slack warns of ${method}: ${warning}`);
    }
  };
  const keep = (list, entry) => {
    list.push(entry);
    if (list.length > KEPT) list.splice(0, list.length - KEPT);
  };
  const name = (id) => W.user(w, id)?.real_name ?? id;
  const where = (c) => (c.is_im ? `${name(c.user)}'s messages with fctl` : `#${c.name}`);
  const track = (p) => {
    inflight.add(p);
    p.finally(() => inflight.delete(p));
    return p;
  };
  /** Tells whoever waits what fctl just did (a call, a response). */
  const fire = (what) => {
    for (const f of [...waiters]) f(what);
  };
  /** Waits up to `ms` for what fctl does that `pred` takes: it, or null. */
  const waitFor = (pred, ms) => {
    let done;
    const promise = new Promise((resolve) => {
      if (!ms) return resolve(null);
      const timer = setTimeout(() => done(null), ms);
      const f = (what) => pred(what) && done(what);
      done = (v) => {
        clearTimeout(timer);
        waiters.delete(f);
        resolve(v);
      };
      waiters.add(f);
    });
    return { promise, cancel: () => done?.(null) };
  };

  // ───────────────────────────── what Slack sends fctl ─────────────────────────────

  const urls = {
    event: () => config.events,
    interaction: () => config.interactivity,
    command: () => config.commands,
  };

  /** One try at a delivery: Slack tries an event again when fctl fails it, unless it says not to. */
  const attempt = async (record, body, contentType, retry) => {
    const timeoutMs = record.kind === "event" ? answerWithinMs : 30_000;
    const answer = await D.deliver(record.url, config.signingSecret, body, contentType, { timeoutMs, retry });
    record.attempts.push({ at: new Date().toISOString(), retry, status: answer.status, ms: answer.ms });
    record.answer = { status: answer.status, body: answer.body, ms: answer.ms };
    log({ kind: "delivery", n: record.n, what: record.what, to: record.kind, retry, answer: record.answer });
    const ok = answer.status >= 200 && answer.status < 300;
    const tries = retry?.num ?? 0;
    const again =
      record.kind === "event" && record.retries && !answer.noRetry && tries < retryDelaysMs.length;
    if (!ok) {
      const said = answer.status
        ? `answered ${record.what} with ${answer.status}: ${answer.body.slice(0, 300)}`
        : `didn't answer ${record.what}: ${answer.body}`;
      const then =
        record.kind !== "event"
          ? ": the person sees an error"
          : again
            ? ": Slack tries again"
            : ": Slack gives up";
      problem("delivery", `fctl ${said}${then}`, { delivery: record.n });
      if (again) {
        const next = { num: tries + 1, reason: D.retryReason(answer) };
        track(
          new Promise((resolve) => {
            const t = setTimeout(
              () => attempt(record, body, contentType, next).then(resolve, resolve),
              retryDelaysMs[tries],
            );
            t.unref?.();
          }),
        );
      }
    } else if (answer.ms > answerWithinMs) {
      problem(
        "delivery",
        `fctl took ${answer.ms} ms to answer ${record.what}: Slack waits ${answerWithinMs}, and the person sees an error`,
        { delivery: record.n },
      );
    }
    return answer;
  };

  /** Sends fctl what Slack would (`kind`: `event`, `interaction`, `command`), signed: its first answer. */
  const send = (kind, what, payload, { retries = true } = {}) =>
    track(
      (async () => {
        const body =
          kind === "event"
            ? JSON.stringify(payload)
            : kind === "interaction"
              ? new URLSearchParams({ payload: JSON.stringify(payload) }).toString()
              : payload.toString();
        const contentType = kind === "event" ? "application/json" : "application/x-www-form-urlencoded";
        count += 1;
        const record = {
          n: count,
          at: new Date().toISOString(),
          kind,
          what,
          url: urls[kind](),
          payload: kind === "command" ? Object.fromEntries(payload) : payload,
          attempts: [],
          retries,
        };
        keep(deliveries, record);
        log({ kind: "deliver", n: record.n, what, to: kind, payload: record.payload });
        return attempt(record, body, contentType, null);
      })(),
    );

  /** Whether fctl's app hears messages in `c` (`message.channels`, `.groups`, `.im`). */
  const hears = (c) =>
    botEvents.includes(
      c.is_im
        ? "message.im"
        : c.is_mpim
          ? "message.mpim"
          : c.is_private
            ? "message.groups"
            : "message.channels",
    );
  const event = (inner, what, opts) => send("event", what, D.envelope(w, inner, opts));

  /** The events of what fctl did through the Web API: its own messages, as Slack sends apps theirs. */
  const echo = (ctx) => {
    for (const e of ctx.events) {
      const c = W.conversation(w, e.channel);
      const m = W.message(w, e.channel, e.ts);
      if (!c || !m || !hears(c) || !W.isMember(c, W.BOT_USER)) continue;
      const inner = e.kind === "changed" ? D.changedEvent(w, m, e.before) : D.messageEvent(w, m);
      event(inner, `the event of fctl's own message in ${where(c)}`, { shared: c.is_ext_shared });
    }
  };

  const answerJson = (a) => (a ? { status: a.status, body: a.body, ms: a.ms } : null);
  const delivered = (a) => a && a.status >= 200 && a.status < 300;

  // ───────────────────────────── Slack's Web API ─────────────────────────────

  /** Answers a Web API call as Slack does: HTTP 200, with `ok`, and the token's scopes. */
  const slack = (res, body, record) => {
    if (res.headersSent || res.destroyed) return;
    res.writeHead(200, {
      "Content-Type": "application/json; charset=utf-8",
      "X-Slack-Req-Id": randomBytes(16).toString("hex"),
      ...(record ? { "X-OAuth-Scopes": record.scopes.join(",") } : {}),
    });
    res.end(JSON.stringify(body));
  };

  /** A switch that rate-limits a method: the seconds to wait, or null to go on. */
  const limited = (method) => {
    const r = switches.rateLimit;
    if (!r?.methods.includes(method)) return null;
    if (r.perMinute) {
      const now = Date.now();
      r.seen[method] = (r.seen[method] ?? []).filter((t) => now - t < 60_000);
      if (r.seen[method].length >= r.perMinute) {
        return Math.max(1, Math.ceil((60_000 - (now - r.seen[method][0])) / 1000));
      }
      r.seen[method].push(now);
      return null;
    }
    if (r.remaining !== null) {
      if (r.remaining <= 0) return null;
      r.remaining -= 1;
    }
    return r.retryAfter;
  };

  /** A switch that fails a call with an error: the error, or null to go on. */
  const failing = (method, args) => {
    for (const [i, rule] of switches.fail.entries()) {
      if (rule.method && rule.method !== method) continue;
      if (rule.blocks) {
        let blocks = args.blocks;
        if (typeof blocks === "string") {
          try {
            blocks = JSON.parse(blocks);
          } catch {
            blocks = null;
          }
        }
        if (!Array.isArray(blocks) || !blocks.some((b) => rule.blocks.includes(b?.type))) continue;
      }
      if (rule.remaining !== null) {
        rule.remaining -= 1;
        if (rule.remaining <= 0) switches.fail.splice(i, 1);
      }
      return rule.error;
    }
    return null;
  };

  const firstLine = (t) => {
    const line = String(t ?? "")
      .split("\n")
      .find((l) => l.trim());
    return line && line.length > 120 ? `${line.slice(0, 119)}…` : (line ?? "");
  };

  /** What fctl did, in a line for the terminal, for the calls that change Acme. */
  const describe = (method, args, answer) => {
    const c = W.conversation(w, answer.channel ?? args.channel ?? args.channel_id);
    const at = c ? where(c) : "Slack";
    switch (method) {
      case "chat.postMessage": {
        const m = W.message(w, answer.channel, answer.ts);
        const thread = m?.thread_ts ? " in a thread" : "";
        return `fctl posted in ${at}${thread}: ${firstLine(B.shown(m?.blocks) || m?.text)}`;
      }
      case "chat.update": {
        const m = W.message(w, answer.channel, answer.ts);
        return `fctl changed its message in ${at}: ${firstLine(B.shown(m?.blocks) || m?.text)}`;
      }
      case "chat.postEphemeral": {
        const e = w.ephemeral.find((x) => x.ts === answer.message_ts);
        return `fctl told ${name(args.user)} in ${at}, only them: ${firstLine(B.shown(e?.blocks) || e?.text)}`;
      }
      case "reactions.add":
        return `fctl reacted :${args.name}: in ${at}`;
      case "agents.sessions.setStatus":
        return `the agent session of a thread in ${at} is ${args.status}`;
      case "chat.startStream":
        return `fctl started a stream in ${at}`;
      case "chat.stopStream":
        return `fctl ended its stream in ${at}`;
      default:
        return null;
    }
  };

  const api = async (req, res, url, raw) => {
    const method = decodeURIComponent(url.pathname.slice("/api/".length));
    const type = String(req.headers["content-type"] ?? "").toLowerCase();
    const extra = [];
    let args = Object.fromEntries([...url.searchParams].filter(([k]) => k !== "token"));
    let formToken;
    if (req.method === "POST" && type.startsWith("application/json")) {
      let body;
      try {
        body = raw.trim() ? JSON.parse(raw) : {};
      } catch {
        problem("api", `${method}: Slack refuses it: invalid_json`);
        return slack(res, { ok: false, error: "invalid_json" });
      }
      if (body === null || typeof body !== "object" || Array.isArray(body)) {
        problem("api", `${method}: Slack refuses it: json_not_object`);
        return slack(res, { ok: false, error: "json_not_object" });
      }
      if (!type.includes("charset")) extra.push("missing_charset");
      if (A.JSON_METHODS.has(method)) args = { ...args, ...body };
      else if (Object.keys(body).length) {
        problem("api", `fctl sent ${method} a JSON body, which Slack ignores for it: it takes a form`);
      }
    } else if (req.method === "POST") {
      const form = new URLSearchParams(raw);
      formToken = form.get("token") ?? undefined;
      args = { ...args, ...Object.fromEntries([...form].filter(([k]) => k !== "token")) };
    }
    const form = () => new URLSearchParams(Object.entries(args).map(([k, v]) => [k, String(v)]));
    let answer;
    let actor = null;
    switch (method) {
      case "oauth.v2.access": {
        const r = O.access(w, config, codes, form(), req.headers.authorization);
        if (r.changed) save();
        if (r.body.ok) out.log(`[slack] ${name(r.installer)} installed fctl's app in Acme`);
        answer = r.body;
        break;
      }
      case "openid.connect.token": {
        const r = O.oidcToken(w, config, codes, key, form(), req.headers.authorization);
        if (r.changed) save();
        if (r.body.ok) out.log(`[slack] ${name(r.user)} signed in to fctl with Slack`);
        answer = r.body;
        break;
      }
      case "apps.uninstall": {
        const r = O.appsUninstall(w, config, form(), req.headers.authorization);
        answer = r.body;
        if (r.body.ok) {
          save();
          out.log("[slack] fctl uninstalled its app from Acme");
          event(D.appUninstalled(), "app_uninstalled, after fctl uninstalled its app");
          event(D.tokensRevoked(r.gone), "tokens_revoked, after fctl uninstalled its app");
        }
        break;
      }
      case "apps.connections.open":
        // Events come over HTTP here: the stand-in has no Socket Mode.
        warn(method, "the stand-in has no Socket Mode: leave SLACK_APP_TOKEN unset");
        answer = { ok: false, error: "not_allowed" };
        break;
      default: {
        const who = O.actorOf(w, req.headers.authorization, formToken);
        if (who.error) {
          answer = { ok: false, error: who.error };
          break;
        }
        actor = who;
        const wait = limited(method);
        if (wait !== null) {
          log({ kind: "api", method, by: who.record.user, args, status: 429 });
          out.log(`[slack] ${method}: rate-limited, Retry-After ${wait}`);
          if (!res.headersSent) {
            res.writeHead(429, {
              "Content-Type": "application/json; charset=utf-8",
              "Retry-After": String(wait),
            });
            res.end(JSON.stringify({ ok: false, error: "ratelimited" }));
          }
          return;
        }
        const failed = failing(method, args);
        if (failed) {
          out.log(`[slack] ${method}: failed as the stand-in was asked, with ${failed}`);
          answer = { ok: false, error: failed, simulated: true };
          break;
        }
        if (method === "auth.revoke") {
          if (args.test === "1" || args.test === true || args.test === "true") {
            answer = { ok: true, revoked: false };
          } else {
            const gone = O.revoke(w, who.token);
            save();
            answer = { ok: true, revoked: true };
            if (gone.bot.length) {
              out.log("[slack] fctl revoked its bot token");
              event(D.tokensRevoked(gone), "tokens_revoked, after fctl revoked its bot token");
            }
          }
          break;
        }
        if (method === "openid.connect.userInfo") {
          answer = O.userInfo(w, who.record);
          break;
        }
        const r = switches.rateLimit;
        const ctx = {
          w,
          record: who.record,
          token: who.token,
          triggers,
          triggerTtlMs,
          maxLimit: r?.maxLimit && r.methods.includes(method) ? r.maxLimit : undefined,
          events: [],
          notes: [],
          warnings: [],
          problems: [],
          changed: false,
        };
        answer = A.answer(ctx, method, args);
        if (ctx.changed) save();
        for (const p of ctx.problems) problem("api", `${method}: ${p}`);
        for (const n of ctx.notes) out.log(`[slack] ${n}`);
        for (const x of ctx.warnings) warn(method, x);
        echo(ctx);
      }
    }
    if (extra.length) {
      for (const x of extra) warn(method, x);
      answer.warning = [answer.warning, ...extra].filter(Boolean).join(",");
      answer.response_metadata = {
        ...answer.response_metadata,
        warnings: [...(answer.response_metadata?.warnings ?? []), ...extra],
      };
    }
    if (answer.ok === false && A.BAD_REQUESTS.has(answer.error) && !answer.simulated) {
      const detail = answer.response_metadata?.messages?.join("; ") ?? answer.needed ?? "";
      problem("api", `${method}: Slack refuses it: ${answer.error}${detail ? ` (${detail})` : ""}`, { args });
    }
    delete answer.simulated;
    const by = actor?.record.user ?? null;
    log({ kind: "api", method, by, args, ok: answer.ok, error: answer.error });
    count += 1;
    keep(calls, {
      n: count,
      at: new Date().toISOString(),
      method,
      by,
      args,
      ok: answer.ok,
      error: answer.error,
    });
    if (answer.ok) {
      const note = describe(method, args, answer);
      if (note) out.log(`[slack] ${note}`);
      fire({ method, args, answer, by });
    }
    return slack(res, answer, actor?.record);
  };

  // ───────────────────────────── response_url ─────────────────────────────

  /** A response_url for an interaction or a command: where it is, and what it answers for. */
  const newResponse = (r) => {
    const id = randomBytes(12).toString("hex");
    responses.set(id, { id, ...r, created: Date.now(), uses: [] });
    return { id, url: `${base()}/standin/respond/${id}` };
  };

  const responseJson = (r) => ({
    id: r.id,
    kind: r.kind,
    user: r.user,
    by: name(r.user),
    channel: r.channel,
    message: r.message ?? null,
    uses: r.uses,
  });

  const yes = (v) => v === true || v === "true";

  /** fctl answers at a response_url: a message only the person sees, one in the channel, or one that replaces or deletes theirs. */
  const respond = (req, res, id, raw) => {
    const r = responses.get(id);
    if (req.method === "GET") {
      return r ? json(res, 200, responseJson(r)) : json(res, 404, { error: "No such response_url" });
    }
    if (!r || Date.now() - r.created > RESPONSE_TTL_MS) {
      problem("respond", "fctl posted to a response_url that is unknown, or older than half an hour");
      return json(res, 404, { ok: false, error: "expired_url" });
    }
    if (r.uses.length >= RESPONSE_USES) {
      problem("respond", `fctl posted to a response_url more than ${RESPONSE_USES} times`);
      return json(res, 404, { ok: false, error: "used_url" });
    }
    let body;
    try {
      body = JSON.parse(raw);
    } catch {
      body = null;
    }
    if (body === null || typeof body !== "object" || Array.isArray(body)) {
      problem("respond", "fctl posted to a response_url something other than a JSON message");
      return json(res, 400, { ok: false, error: "invalid_payload" });
    }
    if (body.blocks !== undefined) {
      const found = B.blockProblems(body.blocks);
      if (found.length) {
        problem(
          "respond",
          `fctl posted blocks Slack refuses to a response_url: ${B.messages(found).join("; ")}`,
        );
        return json(res, 400, { ok: false, error: "invalid_blocks" });
      }
    }
    const c = W.conversation(w, r.channel);
    const text = String(body.text ?? "");
    const blocks = body.blocks && B.withIds(body.blocks);
    const original = r.message
      ? r.message.ephemeral
        ? w.ephemeral.find((e) => e.ts === r.message.ts && !e.deleted)
        : W.message(w, r.channel, r.message.ts)
      : null;
    let effect;
    if (yes(body.delete_original) || yes(body.replace_original)) {
      if (!original) {
        problem(
          "respond",
          `fctl asked a response_url to ${yes(body.delete_original) ? "delete" : "replace"} the original message, and there is none (a command's, or one already gone)`,
        );
        effect = "nothing";
      } else if (yes(body.delete_original)) {
        effect = "deleted the original";
        if (r.message.ephemeral) original.deleted = true;
        else {
          const before = W.messageJson(w, original);
          w.messages.splice(w.messages.indexOf(original), 1);
          if (hears(c) && W.isMember(c, W.BOT_USER)) {
            event(D.deletedEvent(w, original, before), `the deletion of a message in ${where(c)}`);
          }
        }
      } else {
        effect = "replaced the original";
        const before = r.message.ephemeral ? null : W.messageJson(w, original);
        original.text = text;
        if (blocks) original.blocks = blocks;
        else delete original.blocks;
        if (before && hears(c) && W.isMember(c, W.BOT_USER)) {
          event(D.changedEvent(w, original, before), `the change of a message in ${where(c)}`);
        }
      }
    } else if (body.response_type === "in_channel") {
      effect = "posted in the channel";
      const m = W.addMessage(w, {
        channel: c.id,
        user: W.BOT_USER,
        bot: W.BOT_ID,
        app: W.APP_ID,
        text,
        blocks,
        thread_ts: body.thread_ts || undefined,
      });
      if (hears(c) && W.isMember(c, W.BOT_USER)) {
        event(D.messageEvent(w, m), `the event of fctl's own message in ${where(c)}`);
      }
    } else {
      effect = `told ${name(r.user)}, only them`;
      W.addEphemeral(w, {
        channel: c.id,
        to: r.user,
        text,
        blocks,
        thread_ts: body.thread_ts || r.thread_ts || undefined,
        via: "response_url",
      });
    }
    r.uses.push({ at: new Date().toISOString(), body, effect });
    save();
    log({ kind: "respond", id, body, effect });
    out.log(
      `[slack] fctl answered at a response_url (${r.kind} of ${name(r.user)}): ${effect}: ${firstLine(B.shown(blocks) || text)}`,
    );
    fire({ response: id, effect });
    return json(res, 200, { ok: true });
  };

  // ───────────────────────────── as people in Slack ─────────────────────────────

  const toInstalled = () => {
    if (!O.installed(w)) {
      throw new Refusal(
        409,
        "fctl's app isn't installed in Acme: restart cargo xtask dev, or connect Acme on the workspace's Slack tab",
      );
    }
  };
  const who = (n) => {
    const u = n ? W.personByName(w, n) : W.user(w, W.DEV_USER);
    if (!u) throw new Refusal(400, `Acme has nobody called ${n}: dev, grace or ada (a guest)`);
    return u;
  };
  const sees = (c, by) => (c.is_im ? c.user === by.id : W.isMember(c, by.id));
  const channels = () =>
    w.conversations
      .filter((c) => !c.is_im)
      .map((c) => c.name)
      .join(", ");
  /** The conversation a request names (`dm`: the person's messages with fctl), where they are. */
  const channelFor = (n, by, fallback) => {
    const asked = n ?? fallback;
    if (!asked) throw new Refusal(400, `Name a channel: ${channels()}, or dm`);
    const c = asked === "dm" ? W.dmOf(w, by.id) : W.conversationByName(w, asked);
    if (!c) throw new Refusal(404, `Acme has no channel ${asked}: ${channels()}, or dm`);
    if (!sees(c, by)) {
      throw new Refusal(
        403,
        c.is_im ? `Those are ${name(c.user)}'s messages with fctl` : `${by.real_name} isn't in #${c.name}`,
      );
    }
    return c;
  };
  /**
   * The message a request names (`ts`, `thread` or `link`: a ts, or a link
   * to it), else the newest that is fctl's or mentions it, in `channel`
   * when given, where the person sees it.
   */
  const messageFor = (b, by, channel = null) => {
    const ref = b.ts ?? b.thread ?? b.link;
    let m;
    if (ref) {
      const link = W.readLink(ref);
      m = link
        ? W.message(w, link.channel, link.ts)
        : channel
          ? W.message(w, channel.id, String(ref))
          : W.messageAnywhere(w, String(ref));
      if (!m) throw new Refusal(404, `Acme has no message ${ref}${channel ? ` in ${where(channel)}` : ""}`);
    } else {
      m = w.messages.findLast(
        (x) =>
          (!channel || x.channel === channel.id) &&
          sees(W.conversation(w, x.channel), by) &&
          ((x.user === W.BOT_USER && x.subtype !== "channel_join") || x.text.includes(`<@${W.BOT_USER}>`)),
      );
      if (!m) {
        throw new Refusal(
          404,
          `There's no thread with fctl${channel ? ` in ${where(channel)}` : ""} yet: mention it first`,
        );
      }
    }
    const c = W.conversation(w, m.channel);
    if (!sees(c, by)) throw new Refusal(403, `${by.real_name} doesn't see ${where(c)}`);
    return m;
  };
  const words = (b, required) => {
    const t = String(b.text ?? "").trim();
    if (!t && required) throw new Refusal(400, "Say what the message says (text)");
    return W.markup(w, t);
  };

  /** A token Slack gives an app with an event, for its Real-time Search. */
  const actionToken = (m) => {
    const t = D.triggerId();
    w.actionTokens[t] = { user: m.user, channel: m.channel, ts: m.ts, at: new Date().toISOString() };
    return t;
  };

  /**
   * A person's message, and the events Slack sends for it: `app_mention`
   * where it mentions fctl (inviting fctl first, where it isn't in the
   * channel and `invite`), `message` where fctl hears the conversation.
   */
  const say = async ({ by, c, text, root, invite = true, what }) => {
    const m = W.addMessage(w, {
      channel: c.id,
      user: by.id,
      text,
      blocks: B.richText(text),
      thread_ts: root?.ts,
      client_msg_id: randomUUID(),
    });
    const mentions = text.includes(`<@${W.BOT_USER}>`);
    const member = W.isMember(c, W.BOT_USER);
    let invited = false;
    if (!c.is_im && mentions && !member && invite) {
      // Slack offers to invite the app it was mentioned in; they do.
      W.join(w, c, W.BOT_USER, by.id);
      invited = true;
    }
    const token = mentions || c.is_im ? actionToken(m) : undefined;
    save();
    const sent = [];
    const shared = { shared: c.is_ext_shared };
    if (mentions && !c.is_im && (member || invited)) {
      sent.push(["app_mention", event(D.mentionEvent(w, m, { actionToken: token }), what, shared)]);
    }
    if (member && hears(c)) {
      const inner = D.messageEvent(w, m, { actionToken: token });
      sent.push(["message", event(inner, sent.length ? `the message event of ${what}` : what, shared)]);
    }
    const answers = await Promise.all(sent.map(([, p]) => p));
    const all = sent.map(([e], i) => ({ event: e, ...answerJson(answers[i]) }));
    const heard = all.length
      ? `fctl answered ${all.map((d) => d.status).join(" and ")}`
      : "Slack sent fctl nothing";
    out.log(`[slack] ${what}${invited ? ", inviting fctl" : ""}: ${heard}`);
    log({ kind: "action", what, ts: m.ts });
    return {
      what,
      channel: channelJson(c),
      ts: m.ts,
      thread_ts: root?.ts ?? m.ts,
      link: W.permalink(w, c.id, m.ts, root?.ts),
      invited,
      delivery: all[0] ?? null,
      deliveries: all,
      thread: threadJson(root ?? m),
    };
  };

  /** A trigger for opening a view, which works once, within Slack's three seconds. */
  const newTrigger = (by, kind) => {
    const id = D.triggerId();
    const now = Date.now();
    for (const [k, t] of triggers) if (now - t.at > 10 * 60_000) triggers.delete(k);
    triggers.set(id, { user: by.id, at: now, used: false, kind });
    return id;
  };

  /** What fctl did about an action within its wait: what it posted at its response_url, the view it opened, its messages. */
  const followUp = (since, { response, trigger, channels: cs = [] }) => ({
    ...(response ? { responses: responses.get(response)?.uses ?? [] } : {}),
    ...(trigger ? { view: viewJson(w.views.find((v) => v.trigger === trigger)) } : {}),
    posted: w.messages
      .filter(
        (m) =>
          m.user === W.BOT_USER && cs.includes(m.channel) && m.ts > since && m.subtype !== "channel_join",
      )
      .map(messageJson),
  });

  /** `{ thread }`: the thread a message heads, for an action's answer, where there is one. */
  const threadOf = (m) => (m ? { thread: threadJson(W.rootOf(w, m)) } : {});

  /** The elements a person could press: in a message, one only they see, their Home tab or a form of theirs. */
  const findControl = (b, by) => {
    if ([b.answer, b.project, b.action, b.label].every((x) => x === undefined)) {
      throw new Refusal(
        400,
        "Say what to press: answer (its index), project, action (its action_id) or label",
      );
    }
    const places = [];
    if (b.home) {
      const v = W.home(w, by.id);
      if (!v) throw new Refusal(404, `fctl published no Home tab for ${by.real_name}: open it first (home)`);
      places.push({ view: v, blocks: v.blocks });
    } else {
      const channel = b.channel ? channelFor(b.channel, by) : null;
      let scope = (m) => !channel || m.channel === channel.id;
      if (b.ts || b.thread || b.link) {
        const m = (() => {
          try {
            return messageFor(b, by, channel);
          } catch (e) {
            // Only they see it: a message fctl told them alone.
            const e2 = w.ephemeral.find(
              (x) => x.ts === String(b.ts ?? b.thread) && x.to === by.id && !x.deleted,
            );
            if (e2) return e2;
            throw e;
          }
        })();
        const root = m.to ? null : W.rootOf(w, m);
        scope = b.ts
          ? (x) => x.ts === m.ts
          : (x) => x.channel === m.channel && (x.ts === root?.ts || x.thread_ts === root?.ts);
      }
      const seen = [
        ...w.messages
          .filter((m) => sees(W.conversation(w, m.channel), by) && scope(m))
          .map((m) => ({ message: m })),
        ...w.ephemeral
          .filter((e) => e.to === by.id && !e.deleted && scope(e))
          .map((e) => ({ message: e, ephemeral: true })),
      ].sort((x, y) => (x.message.ts < y.message.ts ? 1 : -1));
      places.push(...seen.map((s) => ({ ...s, blocks: s.message.blocks })));
      if (!b.ts && !b.thread && !b.link && !channel) {
        for (const v of W.openModals(w, by.id).reverse()) places.push({ view: v, blocks: v.blocks });
        const h = W.home(w, by.id);
        if (h) places.push({ view: h, blocks: h.blocks });
      }
    }
    const want = (s) => String(s ?? "").toLowerCase();
    const label = (e) => want(e.text?.text);
    const project = (o, value) =>
      want(o.value).endsWith(`|${want(value)}`) ||
      want(o.value) === want(value) ||
      want(o.text?.text) === want(value) ||
      want(o.text?.text).startsWith(`${want(value)} (`);
    for (const place of places) {
      for (const { block_id, element: e } of B.controls(place.blocks)) {
        let chosen;
        if (b.answer !== undefined) {
          if (e.action_id !== `fctl_answer_${b.answer}`) continue;
        } else if (b.project !== undefined) {
          if (!String(e.action_id).startsWith("fctl_project")) continue;
          if (e.type === "button" && !project({ value: e.value, text: e.text }, b.project)) continue;
          if (e.type === "static_select") {
            chosen = (e.options ?? []).find((o) => project(o, b.project));
            if (!chosen) continue;
          }
        } else {
          if (b.action !== undefined && e.action_id !== b.action) continue;
          if (b.label !== undefined && label(e) !== want(b.label)) continue;
          if (e.type === "static_select" && b.value !== undefined) {
            chosen = (e.options ?? []).find(
              (o) => want(o.value) === want(b.value) || want(o.text?.text) === want(b.value),
            );
            if (!chosen) continue;
          }
        }
        return { ...place, block_id, element: e, chosen };
      }
    }
    const offered = places
      .flatMap((p) => B.controls(p.blocks).map((c) => c.element.text?.text ?? c.element.action_id))
      .filter(Boolean)
      .slice(0, 12);
    throw new Refusal(
      404,
      `${by.real_name} sees nothing like that to press${offered.length ? `: they see ${offered.join(", ")}` : ""}`,
    );
  };

  /** What a form's inputs hold as someone sends it: `project`, `task`, `updates`, or `values` by block id. */
  const formValues = (v, b) => {
    const values = {};
    const asked = { ...(b.values ?? {}) };
    if (b.project !== undefined) asked.project ??= b.project;
    if (b.task !== undefined) asked.task ??= b.task;
    if (b.updates !== undefined) asked.updates ??= b.updates;
    for (const { block_id, element: e, block } of B.controls(v.blocks)) {
      if (block.type !== "input") continue;
      const given = asked[block_id];
      let chosen;
      if (given !== undefined) {
        if (e.type === "static_select") {
          const g = String(given).toLowerCase();
          chosen = (e.options ?? []).find(
            (o) =>
              o.value.toLowerCase() === g ||
              o.text.text.toLowerCase() === g ||
              o.text.text.toLowerCase().startsWith(`${g} (`),
          );
          if (!chosen) {
            const offered = (e.options ?? []).map((o) => o.text.text).join(", ");
            throw new Refusal(400, `The form offers no ${block_id} called ${given}: ${offered}`);
          }
        } else if (e.type === "checkboxes") {
          chosen =
            given === true
              ? e.options
              : given === false
                ? []
                : (e.options ?? []).filter((o) => [given].flat().includes(o.value));
        } else chosen = String(given);
      }
      const s = B.stateOf(e, chosen);
      if (!s) continue;
      values[block_id] ??= {};
      values[block_id][e.action_id] = s;
    }
    return values;
  };

  /** What fctl keeps in a view (`private_metadata`), where it is JSON. */
  const metadata = (v) => {
    try {
      return JSON.parse(v.private_metadata || "{}") ?? {};
    } catch {
      return {};
    }
  };

  const parseAnswer = (a) => {
    if (!a?.body?.trim()) return null;
    try {
      return JSON.parse(a.body);
    } catch {
      return { text: a.body };
    }
  };

  const actions = {
    post: async (b) => {
      toInstalled();
      const by = who(b.as);
      const c = channelFor(b.channel, by);
      const root =
        b.thread || b.link ? W.rootOf(w, messageFor({ thread: b.thread, link: b.link }, by, c)) : null;
      const what = root
        ? `${by.real_name} replied in a thread in ${where(c)}`
        : `${by.real_name} wrote in ${where(c)}`;
      return say({ by, c, text: words(b, true), root, invite: b.invite !== false, what });
    },
    mention: async (b) => {
      toInstalled();
      const by = who(b.as);
      let c;
      let root = null;
      if (b.thread || b.link) {
        root = W.rootOf(w, messageFor(b, by, b.channel ? channelFor(b.channel, by) : null));
        c = W.conversation(w, root.channel);
      } else c = channelFor(b.channel, by, "web-team");
      if (c.is_im)
        throw new Refusal(400, "In their messages with fctl, people write to it: dm, rather than mention");
      let text = words(b, false) || "Can you take a look?";
      if (!text.includes(`<@${W.BOT_USER}>`)) text = `<@${W.BOT_USER}> ${text}`;
      const what = `${by.real_name} mentioned fctl in ${root ? "a thread in " : ""}${where(c)}`;
      return say({ by, c, text, root, invite: b.invite !== false, what });
    },
    dm: async (b) => {
      toInstalled();
      const by = who(b.as);
      const c = W.dmOf(w, by.id);
      const root = b.thread || b.link ? W.rootOf(w, messageFor(b, by, c)) : null;
      const what = `${by.real_name} wrote to fctl${root ? " in a thread" : ""}`;
      return say({ by, c, text: words(b, true), root, what });
    },
    reply: async (b) => {
      toInstalled();
      const by = who(b.as);
      const root = W.rootOf(w, messageFor(b, by, b.channel ? channelFor(b.channel, by) : null));
      const c = W.conversation(w, root.channel);
      const what = c.is_im
        ? `${by.real_name} replied to fctl in a thread`
        : `${by.real_name} replied in a thread in ${where(c)}`;
      return say({ by, c, text: words(b, true), root, what });
    },
    press: async (b) => {
      toInstalled();
      const by = who(b.as);
      const found = findControl(b, by);
      const e = found.element;
      const trigger = newTrigger(by, "press");
      const m = found.message;
      const response = found.view
        ? null
        : newResponse({
            kind: "action",
            user: by.id,
            channel: m.channel,
            message: { ts: m.ts, ephemeral: Boolean(found.ephemeral) },
            thread_ts: m.thread_ts,
          });
      const chosen = {};
      if (found.chosen)
        chosen[found.block_id] = { [e.action_id]: e.type === "checkboxes" ? [found.chosen] : found.chosen };
      if (b.remember) {
        for (const x of B.controls(found.blocks)) {
          if (x.element.action_id === "fctl_remember")
            chosen[x.block_id] = { fctl_remember: x.element.options };
        }
      }
      const state = B.stateValues(found.blocks, chosen);
      const action = D.actionOf(e, found.block_id, found.chosen);
      const payload = D.blockActions(w, {
        user: by,
        actions: [action],
        triggerId: trigger,
        responseUrl: response?.url,
        state,
        message: m,
        ephemeral: found.ephemeral,
        view: found.view,
      });
      const shown = e.text?.text ?? found.chosen?.text?.text ?? e.action_id;
      const c = m && W.conversation(w, m.channel);
      const what = `${by.real_name} pressed “${shown}” ${found.view ? `in ${found.view.type === "home" ? "fctl's Home tab" : "a form"}` : `in ${where(c)}${found.ephemeral ? ", in a message only they see" : ""}`}`;
      const since = W.nextTs(w);
      const waiting = waitFor(
        (x) =>
          (response && x.response === response.id) ||
          (x.method === "views.open" && x.args.trigger_id === trigger) ||
          (x.method?.startsWith("chat.") && c && x.answer.channel === c.id),
        b.wait ?? 0,
      );
      const delivery = await send("interaction", what, payload);
      if (!delivered(delivery)) waiting.cancel();
      await waiting.promise;
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return {
        what,
        delivery: answerJson(delivery),
        pressed: {
          action_id: e.action_id,
          block_id: found.block_id,
          value: e.value ?? found.chosen?.value ?? null,
        },
        url: e.url ?? null,
        trigger_id: trigger,
        response_url: response?.url ?? null,
        ...followUp(since, { response: response?.id, trigger, channels: c ? [c.id] : [] }),
        ...threadOf(
          m && !found.ephemeral ? W.rootOf(w, m) : m?.thread_ts && W.message(w, m.channel, m.thread_ts),
        ),
      };
    },
    /** `press` an answer to the agent's question: `{ answer: n }`. */
    answer: async (b) => {
      if (b.answer === undefined) throw new Refusal(400, "Name the answer by its place, from 0: answer");
      return actions.press(b);
    },
    /** `press` a project in fctl's choice of one: `{ project, remember }`. */
    project: async (b) => {
      if (b.project === undefined) throw new Refusal(400, "Name the project: project");
      return actions.press(b);
    },
    command: async (b) => {
      toInstalled();
      const by = who(b.as);
      if (b.thread || b.link) throw new Refusal(400, "Slack runs apps' commands outside threads only");
      const c = channelFor(b.channel, by, "web-team");
      const text = W.markup(w, String(b.text ?? "").trim());
      const trigger = newTrigger(by, "command");
      const response = newResponse({ kind: "command", user: by.id, channel: c.id });
      const form = D.command(w, {
        user: by,
        channel: c,
        text,
        responseUrl: response.url,
        triggerId: trigger,
      });
      const what = `${by.real_name} ran /fctl${text ? ` ${text}` : ""} in ${where(c)}`;
      const dm = W.dmOf(w, by.id);
      const since = W.nextTs(w);
      const waiting = waitFor(
        (x) =>
          x.response === response.id ||
          (x.method === "views.open" && x.args.trigger_id === trigger) ||
          (x.method === "chat.postMessage" && [c.id, dm.id].includes(x.answer.channel)),
        b.wait ?? WAIT_MS,
      );
      const delivery = await send("command", what, form);
      // fctl's answer at once shows as a message only they see.
      const answer = delivered(delivery) ? parseAnswer(delivery) : null;
      if (answer) {
        waiting.cancel();
        if (answer.response_type === "in_channel") {
          W.addMessage(w, {
            channel: c.id,
            user: W.BOT_USER,
            bot: W.BOT_ID,
            app: W.APP_ID,
            text: answer.text ?? "",
            blocks: answer.blocks && B.withIds(answer.blocks),
          });
        } else {
          W.addEphemeral(w, {
            channel: c.id,
            to: by.id,
            text: answer.text ?? "",
            blocks: answer.blocks && B.withIds(answer.blocks),
            via: "the command's answer",
          });
        }
        save();
      }
      if (!delivered(delivery)) waiting.cancel();
      await waiting.promise;
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return {
        what,
        delivery: answerJson(delivery),
        answer,
        trigger_id: trigger,
        response_url: response.url,
        ...followUp(since, { response: response.id, trigger, channels: [c.id, dm.id] }),
        ...threadOf(
          w.messages.find((m) => m.user === W.BOT_USER && [c.id, dm.id].includes(m.channel) && m.ts > since),
        ),
      };
    },
    shortcut: async (b) => {
      toInstalled();
      const by = who(b.as);
      const m = messageFor(b, by, b.channel ? channelFor(b.channel, by) : null);
      const c = W.conversation(w, m.channel);
      const trigger = newTrigger(by, "shortcut");
      const response = newResponse({ kind: "shortcut", user: by.id, channel: c.id, thread_ts: m.thread_ts });
      const payload = D.messageAction(w, {
        user: by,
        message: m,
        callbackId: b.callback_id ?? "fctl_send_message",
        triggerId: trigger,
        responseUrl: response.url,
      });
      const what = `${by.real_name} used Send to fctl… on a message in ${where(c)}`;
      const dm = W.dmOf(w, by.id);
      const since = W.nextTs(w);
      const waiting = waitFor(
        (x) =>
          x.response === response.id ||
          (x.method === "views.open" && x.args.trigger_id === trigger) ||
          (x.method === "chat.postMessage" && x.answer.channel === dm.id),
        b.wait ?? WAIT_MS,
      );
      const delivery = await send("interaction", what, payload);
      if (!delivered(delivery)) waiting.cancel();
      await waiting.promise;
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return {
        what,
        delivery: answerJson(delivery),
        trigger_id: trigger,
        response_url: response.url,
        ...followUp(since, { response: response.id, trigger, channels: [dm.id] }),
      };
    },
    "global-shortcut": async (b) => {
      toInstalled();
      const by = who(b.as);
      const trigger = newTrigger(by, "shortcut");
      const payload = D.shortcut(w, {
        user: by,
        callbackId: b.callback_id ?? "fctl_start_session",
        triggerId: trigger,
      });
      const what = `${by.real_name} used the shortcut Start an fctl session`;
      const dm = W.dmOf(w, by.id);
      const since = W.nextTs(w);
      const waiting = waitFor(
        (x) =>
          (x.method === "views.open" && x.args.trigger_id === trigger) ||
          (x.method === "chat.postMessage" && x.answer.channel === dm.id),
        b.wait ?? WAIT_MS,
      );
      const delivery = await send("interaction", what, payload);
      if (!delivered(delivery)) waiting.cancel();
      await waiting.promise;
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return {
        what,
        delivery: answerJson(delivery),
        trigger_id: trigger,
        ...followUp(since, { trigger, channels: [dm.id] }),
      };
    },
    submit: async (b) => {
      toInstalled();
      const by = who(b.as);
      const v = b.view ? W.view(w, b.view) : W.openModals(w, by.id).at(-1);
      if (!v || v.user !== by.id || !v.open) {
        throw new Refusal(
          404,
          `${by.real_name} has no form of fctl's open${b.view ? ` called ${b.view}` : ""}`,
        );
      }
      const values = formValues(v, b);
      const trigger = newTrigger(by, "submit");
      const payload = D.viewSubmission(w, { user: by, view: v, values, triggerId: trigger });
      const what = `${by.real_name} sent the form “${v.title.text}”`;
      const dm = W.dmOf(w, by.id);
      const since = W.nextTs(w);
      const waiting = waitFor((x) => x.method === "chat.postMessage", b.wait ?? 0);
      const delivery = await send("interaction", what, payload);
      const answer = delivered(delivery) ? parseAnswer(delivery) : null;
      v.state = { values };
      if (delivered(delivery)) {
        // An empty answer closes the form; `errors` keeps it open with them; `clear` closes every one.
        const close = (x, why) => {
          x.open = false;
          x.closed = why;
          x.closed_at = new Date().toISOString();
        };
        switch (answer?.response_action) {
          case "errors":
            v.errors = answer.errors ?? {};
            break;
          case "update":
            Object.assign(v, {
              blocks: B.withIds(answer.view?.blocks ?? []),
              title: answer.view?.title ?? v.title,
            });
            delete v.errors;
            break;
          case "clear":
            for (const x of W.openModals(w, by.id)) close(x, "cleared");
            break;
          default:
            close(v, "submitted");
        }
        save();
      } else waiting.cancel();
      await waiting.promise;
      out.log(
        `[slack] ${what}: fctl answered ${delivery.status}${answer?.response_action ? ` (${answer.response_action})` : ""}`,
      );
      return {
        what,
        delivery: answerJson(delivery),
        answer,
        view: viewJson(v),
        ...followUp(since, { channels: [dm.id, metadata(v).c].filter(Boolean) }),
      };
    },
    home: async (b) => {
      toInstalled();
      const by = who(b.as);
      const what = `${by.real_name} opened fctl's Home tab`;
      const waiting = waitFor(
        (x) => x.method === "views.publish" && x.args.user_id === by.id,
        b.wait ?? WAIT_MS,
      );
      const delivery = await event(D.homeOpened(w, by.id, W.home(w, by.id)), what);
      if (!delivered(delivery)) waiting.cancel();
      await waiting.promise;
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return { what, delivery: answerJson(delivery), view: viewJson(W.home(w, by.id)) };
    },
    stop: async (b) => {
      toInstalled();
      const by = who(b.as);
      const root = W.rootOf(w, messageFor(b, by, b.channel ? channelFor(b.channel, by) : null));
      const c = W.conversation(w, root.channel);
      const s = W.agentSession(w, c.id, root.ts);
      if (!s) {
        throw new Refusal(
          409,
          `fctl set no agent session on that thread in ${where(c)}, so Slack shows no stop button there`,
        );
      }
      const streaming = [];
      for (const m of W.thread(w, root)) {
        A.timeOut(w, m);
        if (m.stream?.state === "streaming") {
          m.stream.state = "stopped_by_user";
          streaming.push(m.ts);
        }
      }
      s.stopped.push({ by: by.id, at: new Date().toISOString() });
      save();
      const what = `${by.real_name} pressed stop on fctl's agent session in ${where(c)}`;
      const delivery = await event(D.sessionStopped(c.id, root.ts, by.id, streaming), what);
      out.log(`[slack] ${what}: fctl answered ${delivery.status}`);
      return { what, delivery: answerJson(delivery), thread: threadJson(root) };
    },
    invite: async (b) => {
      toInstalled();
      const by = who(b.as);
      const c = channelFor(b.channel, by);
      if (c.is_im) throw new Refusal(400, "fctl is in everyone's messages with it already");
      if (W.isMember(c, W.BOT_USER))
        return { what: `fctl is in ${where(c)} already`, channel: channelJson(c) };
      W.join(w, c, W.BOT_USER, by.id);
      save();
      const what = `${by.real_name} invited fctl to ${where(c)}`;
      out.log(`[slack] ${what}`);
      return { what, channel: channelJson(c) };
    },
    kick: async (b) => {
      const by = who(b.as);
      const c = channelFor(b.channel, by);
      if (c.is_im) throw new Refusal(400, "Nobody takes fctl out of someone's messages with it");
      c.members = c.members.filter((m) => m !== W.BOT_USER);
      save();
      const what = `${by.real_name} took fctl out of ${where(c)}`;
      out.log(`[slack] ${what}`);
      return { what, channel: channelJson(c) };
    },
    uninstall: async () => {
      toInstalled();
      const gone = O.uninstall(w);
      save();
      const what = "Acme uninstalled fctl's app";
      const [a, t] = await Promise.all([
        event(D.appUninstalled(), what),
        event(D.tokensRevoked(gone), "tokens_revoked, as Acme uninstalled fctl's app"),
      ]);
      out.log(`[slack] ${what}: fctl answered ${a.status} and ${t.status}`);
      return {
        what,
        delivery: answerJson(a),
        deliveries: [
          { event: "app_uninstalled", ...answerJson(a) },
          { event: "tokens_revoked", ...answerJson(t) },
        ],
      };
    },
    verify: async () => {
      const challenge = randomBytes(24).toString("base64url");
      const what = "Slack's check of fctl's events address (url_verification)";
      const delivery = await send("event", what, D.urlVerification(challenge), { retries: false });
      const body = delivery.body ?? "";
      let said = body.trim();
      try {
        const parsed = JSON.parse(body);
        said = parsed?.challenge ?? said;
      } catch {
        said = new URLSearchParams(body).get("challenge") ?? said;
      }
      const ok = delivered(delivery) && said === challenge;
      if (!ok && delivered(delivery))
        problem("delivery", `fctl answered ${what} without the challenge: ${body.slice(0, 200)}`);
      return { what, ok, delivery: answerJson(delivery) };
    },
    "rate-limit": async (b) => {
      if (b.off || (Array.isArray(b.methods) && b.methods.length === 0)) switches.rateLimit = null;
      else {
        switches.rateLimit = {
          methods: b.methods ?? ["conversations.replies", "conversations.history"],
          retryAfter: Number(b.retryAfter ?? 30),
          remaining: b.calls === undefined ? null : Number(b.calls),
          perMinute: b.perMinute === undefined ? null : Number(b.perMinute),
          maxLimit: b.maxLimit === undefined ? null : Number(b.maxLimit),
          seen: {},
        };
      }
      out.log(`[slack] rate limit: ${switches.rateLimit ? JSON.stringify(rateLimitJson()) : "off"}`);
      return { rateLimit: rateLimitJson() };
    },
    fail: async (b) => {
      if (b.off) switches.fail = [];
      else {
        if (!b.method && !b.blocks)
          throw new Refusal(400, "Say which calls fail: method, or blocks (block types)");
        switches.fail.push({
          method: b.method ?? null,
          error: b.error ?? (b.blocks ? "invalid_blocks" : "internal_error"),
          blocks: b.blocks ?? null,
          remaining: b.calls === undefined ? null : Number(b.calls),
        });
      }
      return { fail: switches.fail };
    },
  };

  const rateLimitJson = () => {
    const r = switches.rateLimit;
    if (!r) return null;
    const { seen, ...shown } = r;
    return { ...shown, calls: shown.remaining };
  };

  // ───────────────────────────── what /standin/ shows ─────────────────────────────

  const channelJson = (c) => ({
    id: c.id,
    name: c.is_im ? null : c.name,
    kind: W.kind(c),
    ...(c.is_im ? { with: name(c.user) } : {}),
    fctl: W.isMember(c, W.BOT_USER),
  });

  const messageJson = (m) => ({
    ts: m.ts,
    thread_ts: m.thread_ts && m.thread_ts !== m.ts ? m.thread_ts : null,
    channel: m.channel,
    user: m.user,
    by: name(m.user),
    fctl: m.user === W.BOT_USER,
    bot: Boolean(m.bot_id),
    ...(m.subtype ? { subtype: m.subtype } : {}),
    text: m.text,
    blocks: m.blocks ?? null,
    shown: W.readable(w, m.bot_id ? B.shown(m.blocks) || m.text : m.text),
    reactions: m.reactions.map((r) => ({ name: r.name, by: r.users.map(name) })),
    ...(!m.thread_ts || m.thread_ts === m.ts ? { replies: W.replies(w, m).length } : {}),
    ...(m.stream ? { stream: m.stream.state } : {}),
    link: W.permalink(w, m.channel, m.ts, m.thread_ts),
  });

  const ephemeralJson = (e) => ({
    ts: e.ts,
    thread_ts: e.thread_ts ?? null,
    channel: e.channel,
    to: name(e.to),
    user: e.to,
    text: e.text,
    blocks: e.blocks ?? null,
    shown: W.readable(w, B.shown(e.blocks) || e.text),
    via: e.via,
    at: e.at,
    ...(e.deleted ? { deleted: true } : {}),
  });

  const sessionJson = (s) =>
    s && {
      channel: s.channel,
      thread_ts: s.thread_ts,
      status: W.sessionStatus(s),
      title: s.title,
      initiator: s.initiator ? name(s.initiator) : null,
      stopped: s.stopped.map((x) => ({ by: name(x.by), at: x.at })),
      history: s.history,
    };

  const viewJson = (v) =>
    v
      ? {
          id: v.id,
          type: v.type,
          user: v.user,
          for: name(v.user),
          callback_id: v.callback_id,
          title: v.title?.text ?? null,
          private_metadata: v.private_metadata,
          blocks: v.blocks,
          shown: B.shown(v.blocks),
          state: v.state,
          open: v.type === "modal" ? v.open : null,
          ...(v.closed ? { closed: v.closed, closed_at: v.closed_at } : {}),
          ...(v.errors ? { errors: v.errors } : {}),
          at: v.at,
        }
      : null;

  /** A thread: its messages, what fctl told people alone there, and its agent session. */
  const threadJson = (root) => {
    const c = W.conversation(w, root.channel);
    const all = W.thread(w, root);
    for (const m of all) A.timeOut(w, m);
    return {
      channel: channelJson(c),
      ts: root.ts,
      link: W.permalink(w, c.id, root.ts),
      messages: all.map(messageJson),
      ephemeral: w.ephemeral.filter((e) => e.channel === c.id && e.thread_ts === root.ts).map(ephemeralJson),
      agentSession: sessionJson(W.agentSession(w, c.id, root.ts)) ?? null,
    };
  };

  const channelState = (c) => ({
    channel: channelJson(c),
    members: c.members.map(name),
    messages: w.messages.filter((m) => m.channel === c.id).map(messageJson),
    ephemeral: w.ephemeral.filter((e) => e.channel === c.id).map(ephemeralJson),
    agentSessions: w.agentSessions.filter((s) => s.channel === c.id).map(sessionJson),
  });

  const worldJson = () => ({
    team: w.team,
    installed: O.installed(w),
    bot: { user_id: W.BOT_USER, bot_id: W.BOT_ID, app_id: W.APP_ID },
    people: w.users
      .filter((u) => !u.is_bot)
      .map((u) => ({
        id: u.id,
        name: u.real_name,
        handle: u.name,
        email: u.email,
        guest: u.is_restricted,
        dm: W.dmOf(w, u.id)?.id ?? null,
        tz: u.tz,
      })),
    channels: w.conversations
      .filter((c) => !c.is_im)
      .map((c) => ({ ...channelJson(c), members: c.members.map(name) })),
  });

  // ───────────────────────────── the server ─────────────────────────────

  const authorizePage = (res, kind, params, outcome) => {
    const hidden = [...params]
      .filter(([k]) => !["decision", "user", "auto"].includes(k))
      .map(([k, v]) => `<input type="hidden" name="${esc(k)}" value="${esc(v)}">`)
      .join("");
    const people = w.users.filter((u) => !u.is_bot && (kind === "sign-in" || !u.is_restricted));
    const chosen = outcome.consent?.person ?? W.DEV_USER;
    const picker = people
      .map(
        (u) =>
          `<label><input type="radio" name="user" value="${u.id}"${u.id === chosen ? " checked" : ""}> ${esc(u.real_name)} <small>${esc(u.email ?? "")}${u.is_restricted ? " · guest" : ""}</small></label><br>`,
      )
      .join("");
    const scopes = (outcome.consent?.scopes ?? []).map((s) => `<li><code>${esc(s)}</code></li>`).join("");
    const action = kind === "install" ? "/oauth/v2/authorize" : "/openid/connect/authorize";
    const title = kind === "install" ? "fctl is asking to join Acme" : "Sign in to fctl with Slack";
    const content =
      `<h1>${title}</h1>${outcome.problem ? `<p class="problem">${esc(outcome.problem)}</p>` : ""}` +
      `<form method="post" action="${action}">${hidden}` +
      `<h2>${kind === "install" ? "Installed by" : "Signed in to Acme as"}</h2>${picker}` +
      `<h2>${kind === "install" ? "It will be able to" : "fctl will see"}</h2><ul>${scopes}</ul>` +
      `<p><button name="decision" value="allow">${kind === "install" ? "Allow" : "Continue"}</button> ` +
      `<button name="decision" value="deny">Cancel</button></p></form>` +
      "<p><small>The Slack stand-in of cargo xtask dev: add <code>auto=1</code> (and <code>user=U0GRACE</code>) to the address to skip this page.</small></p>";
    return page(res, outcome.problem && !outcome.consent ? 400 : 200, title, content);
  };

  const authorizeRoute = (req, res, url, raw, kind) => {
    const params = new URLSearchParams(url.searchParams);
    if (req.method === "POST") for (const [k, v] of new URLSearchParams(raw)) params.set(k, v);
    const decision = params.get("decision");
    const outcome = O.authorize(w, config, codes, kind, params, decision);
    log({ kind: kind === "install" ? "authorize" : "sign-in", query: Object.fromEntries(params), outcome });
    if (outcome.problem && outcome.location) problem("oauth", `the ${kind} page ${outcome.problem}`);
    else if (outcome.problem && !outcome.consent) problem("oauth", `the ${kind} page: ${outcome.problem}`);
    if (outcome.location) {
      res.writeHead(302, { Location: outcome.location });
      return res.end();
    }
    return authorizePage(res, kind, params, outcome);
  };

  // Whatever a client sends, the stand-in answers it, or turns it down,
  // and stays up for the rest of cargo xtask dev.
  server = createServer(async (req, res) => {
    let url;
    try {
      url = new URL(req.url, "http://127.0.0.1");
    } catch {
      problem("unknown", `a request Slack has no answer for: ${req.method} ${req.url}`);
      return json(res, 400, { error: "Slack answers no such address" });
    }
    const route = `${req.method} ${url.pathname}`;
    let raw;
    try {
      raw = await read(req);
    } catch {
      // The client went away before its request ended: nobody waits for an answer.
      return res.destroy();
    }
    try {
      if (url.pathname.startsWith("/api/") && (req.method === "POST" || req.method === "GET")) {
        return await api(req, res, url, raw);
      }
      if (url.pathname === "/oauth/v2/authorize" && (req.method === "GET" || req.method === "POST")) {
        return authorizeRoute(req, res, url, raw, "install");
      }
      if (url.pathname === "/openid/connect/authorize" && (req.method === "GET" || req.method === "POST")) {
        return authorizeRoute(req, res, url, raw, "sign-in");
      }
      if (route === "GET /openid/connect/keys") return json(res, 200, { keys: [key.jwk] });
      if (route === "GET /.well-known/openid-configuration") return json(res, 200, O.discovery(base()));
      if (url.pathname.startsWith("/standin/respond/")) {
        return respond(req, res, decodeURIComponent(url.pathname.split("/").pop()), raw);
      }
      if (req.method === "GET" && url.pathname.startsWith("/standin/avatar/")) {
        res.writeHead(200, { "Content-Type": "image/svg+xml", "Cache-Control": "max-age=3600" });
        return res.end(
          W.avatarSvg(
            w,
            url.pathname
              .split("/")
              .pop()
              .replace(/\.svg$/, ""),
          ),
        );
      }
      if (route === "GET /") return page(res, 200, "Acme in the Slack stand-in", overview());
      if (route === "GET /standin/health") {
        return json(res, 200, { ok: true, team: w.team, installed: O.installed(w), bot: worldJson().bot });
      }
      if (route === "GET /standin/problems") return json(res, 200, { problems, warnings });
      if (route === "GET /standin/world") return json(res, 200, worldJson());
      if (route === "GET /standin/rate-limit")
        return json(res, 200, { rateLimit: rateLimitJson(), fail: switches.fail });
      if (route === "GET /standin/deliveries") {
        const since = Number(url.searchParams.get("since") ?? 0);
        return json(res, 200, { deliveries: deliveries.filter((d) => d.n > since) });
      }
      if (route === "GET /standin/calls") {
        const since = Number(url.searchParams.get("since") ?? 0);
        const method = url.searchParams.get("method");
        return json(res, 200, {
          calls: calls.filter((c) => c.n > since && (!method || c.method === method)),
        });
      }
      if (route === "GET /standin/views") {
        const as = url.searchParams.get("as") ?? url.searchParams.get("user");
        const by = as ? who(as) : null;
        return json(res, 200, { views: w.views.filter((v) => !by || v.user === by.id).map(viewJson) });
      }
      if (req.method === "GET" && url.pathname.startsWith("/standin/channels/")) {
        const asked = decodeURIComponent(url.pathname.split("/").pop());
        // `dm`: someone's messages with fctl (`?as=`, the dev owner's by default).
        const c =
          asked === "dm" ? W.dmOf(w, who(url.searchParams.get("as")).id) : W.conversationByName(w, asked);
        return c ? json(res, 200, channelState(c)) : json(res, 404, { error: "Acme has no such channel" });
      }
      if (
        req.method === "GET" &&
        (url.pathname === "/standin/thread" || url.pathname.startsWith("/standin/threads/"))
      ) {
        const parts = url.pathname.split("/").slice(3).map(decodeURIComponent);
        const ref = url.searchParams.get("link") ?? url.searchParams.get("ts") ?? parts.at(-1);
        const channel = url.searchParams.get("channel") ?? (parts.length > 1 ? parts[0] : null);
        const c = channel ? W.conversationByName(w, channel) : null;
        const link = ref && W.readLink(ref);
        const m = link
          ? W.message(w, link.channel, link.ts)
          : ref && (c ? W.message(w, c.id, ref) : W.messageAnywhere(w, ref));
        return m
          ? json(res, 200, threadJson(W.rootOf(w, m)))
          : json(res, 404, { error: "Acme has no such thread" });
      }
      const action = url.pathname.slice("/standin/".length);
      if (req.method === "POST" && url.pathname.startsWith("/standin/") && Object.hasOwn(actions, action)) {
        let input;
        try {
          input = raw ? JSON.parse(raw) : {};
        } catch {
          input = undefined;
        }
        if (input === null || typeof input !== "object" || Array.isArray(input)) {
          throw new Refusal(400, "The body must be a JSON object");
        }
        return json(res, 200, await actions[action](input));
      }
      problem("unknown", `a request Slack has no answer for: ${route}`);
      return json(res, 404, { error: `Slack has no ${route}` });
    } catch (e) {
      if (e instanceof Refusal) return json(res, e.status, { error: e.message });
      problem("standin", `the stand-in failed on ${route}: ${e.stack ?? e}`);
      return json(res, 500, { error: String(e.message ?? e) });
    }
  });
  server.on("clientError", (_e, socket) => socket.destroy());
  server.on("listening", () => W.setAvatarBase(base()));

  /** Acme's channels, and what fctl did in each, for a browser. */
  const overview = () => {
    const line = (m) => {
      const shown = W.readable(w, m.bot_id ? B.shown(m.blocks) || m.text : m.text);
      const reactions = m.reactions.map((r) => `:${esc(r.name)}: ${r.users.length}`).join(" ");
      const stream = m.stream ? ` · stream ${esc(m.stream.state)}` : "";
      return `<li${m.user === W.BOT_USER ? ' class="fctl"' : ""}><small>${esc(name(m.user))}${stream}${reactions ? ` · ${reactions}` : ""}</small><br>${esc(shown)}`;
    };
    const sections = w.conversations.map((c) => {
      const tops = w.messages
        .filter((m) => m.channel === c.id && (!m.thread_ts || m.thread_ts === m.ts))
        .slice(-15);
      const items = tops.map((m) => {
        const s = W.agentSession(w, c.id, m.ts);
        const session = s ? `<br><small>agent session: ${esc(W.sessionStatus(s))}</small>` : "";
        const replies = W.replies(w, m).map((r) => `${line(r)}</li>`);
        const eph = w.ephemeral
          .filter((e) => e.channel === c.id && e.thread_ts === m.ts && !e.deleted)
          .map(
            (e) =>
              `<li class="eph"><small>only ${esc(name(e.to))} sees this</small><br>${esc(W.readable(w, B.shown(e.blocks) || e.text))}</li>`,
          );
        const inner = [...replies, ...eph].join("");
        return `${line(m)}${session}${inner ? `<ol>${inner}</ol>` : ""}</li>`;
      });
      const loose = w.ephemeral
        .filter((e) => e.channel === c.id && !e.thread_ts && !e.deleted)
        .map(
          (e) =>
            `<li class="eph"><small>only ${esc(name(e.to))} sees this</small><br>${esc(W.readable(w, B.shown(e.blocks) || e.text))}</li>`,
        );
      const head = c.is_im ? `${esc(name(c.user))} and fctl` : `#${esc(c.name)}`;
      const kind = `${W.kind(c)}${!c.is_im ? (W.isMember(c, W.BOT_USER) ? " · fctl is in it" : " · fctl isn't in it") : ""}`;
      return `<h2>${head} <small>${kind}</small></h2><ol>${items.join("")}${loose.join("")}</ol>`;
    });
    return `<meta http-equiv="refresh" content="3"><h1>Acme</h1><p>The Slack workspace of the stand-in that <code>cargo xtask dev</code> runs${O.installed(w) ? ", with fctl's app installed" : ": fctl's app isn't installed"}. <code>cargo xtask slack mention "…"</code> mentions fctl in #web-team.</p>${sections.join("")}`;
  };

  /** Resolves once every delivery under way (retries included) is done: for tests. */
  const settled = async () => {
    while (inflight.size) await Promise.allSettled([...inflight]);
  };

  return { server, world: w, problems, warnings, deliveries, calls, settled, switches };
}

/** A request the stand-in turns down: its status and why. */
class Refusal extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

function read(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => resolve(Buffer.concat(chunks).toString()));
    req.on("error", reject);
  });
}

function json(res, status, value) {
  if (res.headersSent || res.destroyed) return;
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(value));
}

const esc = (text) =>
  String(text ?? "").replace(
    /[<>&"]/g,
    (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;" })[c],
  );

function page(res, status, title, content) {
  res.writeHead(status, { "Content-Type": "text/html; charset=utf-8" });
  res.end(
    `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>${esc(title)}</title>` +
      `<style>body{font:15px/1.5 system-ui,sans-serif;max-width:760px;margin:2rem auto;padding:0 1rem;color:#171717}` +
      `h2{font-size:1rem;margin:1.5rem 0 .25rem}ol{padding-left:1.25rem}small{color:#666}code{font-size:.9em}` +
      `li{white-space:pre-wrap}.fctl{border-left:3px solid #007eff;padding-left:.5rem}.eph{color:#666;font-style:italic}` +
      `.problem{color:#b00}button{font:inherit;padding:.4rem 1rem;margin-right:.5rem}</style>${content}`,
  );
}

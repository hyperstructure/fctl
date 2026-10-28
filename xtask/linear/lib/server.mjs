// The stand-in's HTTP server: Linear's endpoints (/oauth/authorize,
// /oauth/token, /oauth/revoke, /graphql), and its own under /standin/,
// which act as people in Linear do (delegate an issue to fctl, mention it,
// reply in its agent session, press stop, uninstall its app) and send fctl
// the delivery Linear would, and which show what fctl did there. What
// Linear would refuse, or fctl failed to do in time, is a problem: printed
// as it happens, and listed at /standin/problems.

import { appendFileSync, mkdirSync } from "node:fs";
import { createServer } from "node:http";
import path from "node:path";
import * as D from "./deliveries.mjs";
import * as G from "./graphql.mjs";
import * as O from "./oauth.mjs";
import * as W from "./world.mjs";

/** Linear calls an agent session unresponsive when the app says nothing for this long after `created`. */
const ANSWER_WITHIN_MS = 10_000;

/** Linear gives up on a delivery its receiver doesn't answer this soon. */
const DELIVERY_WITHIN_MS = 5_000;

/**
 * The stand-in, for fctl's app as `config` has it (`clientId`,
 * `clientSecret`, `webhookSecret`, `webhook`: fctl's webhook URL), with
 * its state kept in `state` (a directory) when given.
 */
export function createStandin({ schema, state, config, out = console, answerWithinMs = ANSWER_WITHIN_MS }) {
  const w = W.load(state);
  W.save(w, state);
  const codes = new Map();
  const problems = [];
  const deprecations = new Set();
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
    out.error(`[linear] problem: ${message}`);
  };
  /** Whether fctl's app is installed in Acme: it has a token that works, or renews. */
  const installed = () => Object.values(w.tokens).some((t) => t.app && !t.revoked && t.kind === "refresh");
  /** People delegate to, mention and talk with fctl's app only while it is installed. */
  const toInstalled = () => {
    if (!installed()) {
      throw new Refusal(
        409,
        "fctl's app isn't installed in Acme: restart cargo xtask dev, or connect Acme on the workspace's Linear tab",
      );
    }
  };

  // ───────────────────────────── as people in Linear ─────────────────────────────

  const who = (name) => {
    const u = name ? W.person(w, name) : W.user(w, W.DEV_USER);
    if (!u) throw new Refusal(400, `Acme has nobody called ${name}: dev or grace`);
    return u;
  };
  /** The issue a request names (`issue`), or a new one of `team` (ENG) titled `title`. */
  const issueFor = (body, by) => {
    if (body.issue) {
      const i = W.issue(w, body.issue);
      if (!i) throw new Refusal(404, `Acme has no issue ${body.issue}`);
      return i;
    }
    if (!body.title?.trim())
      throw new Refusal(400, "Name an issue, like ENG-1, or give a title for a new one");
    const t = W.teamByIdOrKey(w, body.team ?? "ENG");
    if (!t) throw new Refusal(404, `Acme has no team ${body.team}`);
    return W.addIssue(w, {
      teamId: t.id,
      title: body.title.trim(),
      description: body.description,
      creatorId: by.id,
    });
  };
  /** The agent session a request names (`session`), or the newest on its issue. */
  const sessionFor = (body) => {
    if (body.session) {
      const s = W.session(w, body.session) ?? w.sessions.find((x) => x.id.startsWith(body.session));
      if (!s) throw new Refusal(404, `Acme has no agent session ${body.session}`);
      return s;
    }
    const i = body.issue && W.issue(w, body.issue);
    if (!i) throw new Refusal(404, `Acme has no issue ${body.issue ?? "(none named)"}`);
    const s = w.sessions.findLast((x) => x.issueId === i.id);
    if (!s)
      throw new Refusal(
        404,
        `${body.issue} has no agent session: delegate it to fctl, or mention fctl in it`,
      );
    return s;
  };
  const send = async (payload, what) => {
    const type = payload.type === "OAuthApp" ? "OAuthAppWebhookPayload" : "AgentSessionEventWebhookPayload";
    for (const d of G.differences(schema, type, payload)) {
      problem("standin", `the stand-in's delivery (${what}) isn't what Linear sends: ${d}`);
    }
    let answer;
    try {
      answer = await D.deliver(config.webhook, config.webhookSecret, payload);
    } catch (e) {
      problem("delivery", `fctl didn't take the delivery (${what}): ${e.message}`);
      return { status: 0, body: String(e.message), ms: 0 };
    }
    log({ kind: "delivery", what, payload, answer });
    if (answer.status < 200 || answer.status >= 300) {
      problem("delivery", `fctl answered the delivery (${what}) with ${answer.status}: ${answer.body}`);
    } else if (answer.ms > DELIVERY_WITHIN_MS) {
      problem(
        "delivery",
        `fctl took ${answer.ms} ms to answer the delivery (${what}): Linear waits ${DELIVERY_WITHIN_MS}`,
      );
    }
    return answer;
  };
  /** A new agent session: delivered, and watched for fctl's first answer. */
  const begin = async (s, what) => {
    save();
    const delivery = await send(D.created(w, config, s), what);
    out.log(`[linear] ${what}, in agent session ${s.id.slice(0, 8)}: fctl answered ${delivery.status}`);
    const since = new Date().toISOString();
    setTimeout(() => {
      const answered = s.activities.some((a) => a.userId === W.APP_USER) || s.externalUrls.length > 0;
      if (!answered && delivery.status >= 200 && delivery.status < 300) {
        problem(
          "unresponsive",
          `fctl said nothing in agent session ${s.id.slice(0, 8)} (${what}) within ${answerWithinMs / 1000} seconds of ${since}: Linear calls it unresponsive`,
        );
      }
    }, answerWithinMs).unref();
    return { what, session: sessionJson(w, s), delivery };
  };
  /** A person's prompt in an agent session: delivered. */
  const prompt = async (s, by, activity, did) => {
    const a = W.addActivity(s, { userId: by.id, ...activity });
    save();
    const what = `${by.name} ${did} in ${W.identifier(w, W.issue(w, s.issueId))}`;
    const delivery = await send(D.prompted(w, config, s, a), what);
    out.log(`[linear] ${what}, in agent session ${s.id.slice(0, 8)}: fctl answered ${delivery.status}`);
    return { what, session: sessionJson(w, s), delivery };
  };
  const actions = {
    delegate: async (body) => {
      toInstalled();
      const by = who(body.as);
      const i = issueFor(body, by);
      i.delegateId = W.APP_USER;
      i.assigneeId ??= by.id;
      i.updatedAt = new Date().toISOString();
      const s = W.addSession(w, { issue: i, creatorId: by.id, commentId: null });
      return begin(s, `${by.name} delegated ${W.identifier(w, i)} to fctl`);
    },
    mention: async (body) => {
      toInstalled();
      const by = who(body.as);
      const i = issueFor(body, by);
      let text = body.body?.trim() || "Can you take a look?";
      if (!/@fctl\b/.test(text)) text = `@fctl ${text}`;
      const c = W.addComment(w, { issueId: i.id, userId: by.id, body: text });
      const s = W.addSession(w, { issue: i, creatorId: by.id, commentId: c.id });
      return begin(s, `${by.name} mentioned fctl in ${W.identifier(w, i)}`);
    },
    reply: async (body) => {
      toInstalled();
      const by = who(body.as);
      const s = sessionFor(body);
      const text = body.body?.trim();
      if (!text) throw new Refusal(400, "Say what the reply says");
      // Linear keeps a reply in the session's thread as a comment, and hands it to the agent as a prompt.
      const c = W.addComment(w, { issueId: s.issueId, userId: by.id, body: text, parentId: s.commentId });
      return prompt(s, by, { content: { type: "prompt", body: text }, sourceCommentId: c.id }, "replied");
    },
    stop: async (body) => {
      toInstalled();
      const by = who(body.as);
      const s = sessionFor(body);
      return prompt(s, by, { content: { type: "prompt", body: "" }, signal: "stop" }, "pressed stop");
    },
    uninstall: async () => {
      toInstalled();
      O.uninstall(w);
      save();
      const what = "Acme uninstalled fctl's app";
      const delivery = await send(D.revoked(config), what);
      out.log(`[linear] ${what}: fctl answered ${delivery.status}`);
      return { what, delivery };
    },
  };

  // ───────────────────────────── Linear's endpoints ─────────────────────────────

  const graphql = async (req, res, body) => {
    const actor = O.actorOf(w, req.headers.authorization);
    if (!actor) {
      problem("auth", "a GraphQL request without a token Linear takes (unknown, revoked or expired)");
      return json(res, 401, {
        errors: [
          {
            message: "Authentication required, not authenticated",
            extensions: {
              type: "authentication error",
              code: "AUTHENTICATION_ERROR",
              userPresentableMessage: "You need to authenticate to access this operation.",
            },
          },
        ],
      });
    }
    let request;
    try {
      request = JSON.parse(body);
    } catch {
      problem("graphql", "a GraphQL request that isn't JSON");
      return json(res, 400, {
        errors: [{ message: "The body must be JSON", extensions: { code: "BAD_REQUEST" } }],
      });
    }
    const effects = { changed: false, notes: [] };
    const answer = await G.answer(schema, w, actor, request, effects);
    if (effects.changed) save();
    const by = W.user(w, actor.userId)?.name;
    log({ kind: "graphql", by, request, status: answer.status, answer: answer.body });
    for (const note of effects.notes) out.log(`[linear] ${note}`);
    for (const e of answer.body.errors ?? []) {
      if (!G.isNotFound(e)) {
        const said = e.extensions?.userPresentableMessage;
        const what = answer.fields?.join(", ") || "a GraphQL request";
        problem(
          "graphql",
          `${what}: Linear would refuse this: ${e.message}${said && said !== e.message ? ` (${said})` : ""}`,
          { query: request.query, variables: request.variables },
        );
      }
    }
    for (const d of answer.deprecated) {
      if (!deprecations.has(d)) {
        deprecations.add(d);
        out.error(`[linear] fctl uses what Linear deprecated: ${d}`);
      }
    }
    return json(res, answer.status, answer.body);
  };

  // Whatever a client sends, the stand-in answers it, or turns it down,
  // and stays up for the rest of cargo xtask dev.
  const server = createServer(async (req, res) => {
    let url;
    try {
      url = new URL(req.url, "http://127.0.0.1");
    } catch {
      problem("unknown", `a request Linear has no answer for: ${req.method} ${req.url}`);
      return json(res, 400, { error: "Linear answers no such address" });
    }
    const route = `${req.method} ${url.pathname}`;
    let body;
    try {
      body = await read(req);
    } catch {
      // The client went away before its request ended: nobody waits for an answer.
      return res.destroy();
    }
    try {
      if (route === "GET /oauth/authorize") {
        const answer = O.authorize(config, codes, url.searchParams);
        log({ kind: "authorize", query: Object.fromEntries(url.searchParams), answer });
        if (answer.problem) problem("oauth", `the authorize page ${answer.problem}`);
        if (!answer.location) return page(res, 400, "Linear", `<p>${esc(answer.problem)}</p>`);
        res.writeHead(302, { Location: answer.location });
        return res.end();
      }
      if (route === "POST /oauth/token" || route === "POST /oauth/revoke") {
        const form = new URLSearchParams(body);
        const answer =
          route === "POST /oauth/token"
            ? O.exchange(w, config, codes, form, req.headers.authorization)
            : O.revoke(w, form);
        if (answer.changed) save();
        log({
          kind: url.pathname.slice(1),
          grant: form.get("grant_type"),
          status: answer.status,
          error: answer.body.error,
        });
        if (answer.status !== 200)
          problem("oauth", `${url.pathname} refused: ${answer.body.error_description}`);
        return json(res, answer.status, answer.body);
      }
      if (route === "POST /graphql") return await graphql(req, res, body);
      if (route === "GET /") return page(res, 200, "Acme in the Linear stand-in", overview(w));
      if (route === "GET /standin/health") {
        return json(res, 200, { ok: true, organization: W.ORGANIZATION, installed: installed() });
      }
      if (route === "GET /standin/problems") return json(res, 200, { problems });
      if (route === "GET /standin/sessions") {
        const i = url.searchParams.get("issue") && W.issue(w, url.searchParams.get("issue"));
        const sessions = w.sessions.filter((s) => !i || s.issueId === i.id).map((s) => sessionJson(w, s));
        return json(res, 200, { sessions });
      }
      if (req.method === "GET" && url.pathname.startsWith("/standin/issues/")) {
        const i = W.issue(w, decodeURIComponent(url.pathname.split("/").pop()));
        return i ? json(res, 200, issueJson(w, i)) : json(res, 404, { error: "Acme has no such issue" });
      }
      if (req.method === "GET" && url.pathname.startsWith("/standin/sessions/")) {
        const s = W.session(w, url.pathname.split("/").pop());
        return s ? json(res, 200, sessionJson(w, s)) : json(res, 404, { error: "No such agent session" });
      }
      const name = url.pathname.slice("/standin/".length);
      if (req.method === "POST" && url.pathname.startsWith("/standin/") && Object.hasOwn(actions, name)) {
        let input;
        try {
          input = body ? JSON.parse(body) : {};
        } catch {
          input = undefined;
        }
        if (input === null || typeof input !== "object" || Array.isArray(input)) {
          throw new Refusal(400, "The body must be a JSON object");
        }
        return json(res, 200, await actions[name](input));
      }
      problem("unknown", `a request Linear has no answer for: ${route}`);
      return json(res, 404, { error: `Linear has no ${route}` });
    } catch (e) {
      if (e instanceof Refusal) return json(res, e.status, { error: e.message });
      problem("standin", `the stand-in failed on ${route}: ${e.stack ?? e}`);
      return json(res, 500, { error: String(e.message ?? e) });
    }
  });
  server.on("clientError", (_e, socket) => socket.destroy());
  return { server, world: w, problems };
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
      `h2{font-size:1rem;margin:1.5rem 0 .25rem}ol{padding-left:1.25rem}small{color:#666}code{font-size:.9em}</style>${content}`,
  );
}

/** An issue as /standin/ shows it: by names rather than ids, with its comments. */
export function issueJson(w, i) {
  const name = (id) => W.user(w, id)?.name ?? null;
  return {
    id: i.id,
    key: W.identifier(w, i),
    title: i.title,
    description: i.description,
    state: W.state(w, i.stateId)?.name,
    priority: W.priorityLabel(i.priority),
    labels: i.labelIds.map((id) => W.label(w, id)?.name).filter(Boolean),
    project: W.project(w, i.projectId)?.name ?? null,
    parent: i.parentId ? W.identifier(w, W.issue(w, i.parentId)) : null,
    children: w.issues.filter((c) => c.parentId === i.id).map((c) => W.identifier(w, c)),
    creator: name(i.creatorId),
    assignee: name(i.assigneeId),
    delegate: name(i.delegateId),
    comments: w.comments
      .filter((c) => c.issueId === i.id)
      .map((c) => ({ id: c.id, by: name(c.userId), body: c.body, parent: c.parentId })),
    attachments: i.attachments.map(({ title, url }) => ({ title, url })),
  };
}

/** An agent session as /standin/ shows it: its issue, links and plan, and its activities in order. */
export function sessionJson(w, s) {
  const i = W.issue(w, s.issueId);
  return {
    id: s.id,
    issue: W.identifier(w, i),
    title: i.title,
    state: W.state(w, i.stateId)?.name,
    delegate: W.user(w, i.delegateId)?.name ?? null,
    attachments: i.attachments.map(({ title, url }) => ({ title, url })),
    creator: W.user(w, s.creatorId)?.name ?? null,
    mention: W.comment(w, s.commentId)?.body ?? null,
    status: W.sessionStatus(s),
    createdAt: s.createdAt,
    externalUrls: s.externalUrls,
    plan: s.plan,
    activities: s.activities.map((a) => ({
      id: a.id,
      at: a.createdAt,
      by: W.user(w, a.userId)?.name ?? null,
      app: a.userId === W.APP_USER,
      ...a.content,
      ephemeral: a.ephemeral,
      signal: a.signal,
      signalMetadata: a.signalMetadata,
    })),
  };
}

/** Acme's issues, and what fctl did in each, for a browser. */
function overview(w) {
  const issues = [...w.issues].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  const items = issues.map((i) => {
    const sessions = w.sessions
      .filter((s) => s.issueId === i.id)
      .map((s) => {
        const j = sessionJson(w, s);
        const lines = j.activities.map((a) => {
          const text =
            a.type === "action" ? `${a.action} ${a.parameter}${a.result ? ` → ${a.result}` : ""}` : a.body;
          const extra = [a.ephemeral && "ephemeral", a.signal].filter(Boolean).join(", ");
          // What Linear shows with an elicitation: the button that links an account, or the choices.
          const button = a.signalMetadata?.url
            ? ` <a href="${esc(a.signalMetadata.url)}">${esc(a.signalMetadata.providerName ?? "Link")}</a>`
            : "";
          const choices = (a.signalMetadata?.options ?? []).map((o) => esc(o.label ?? o.value)).join(" · ");
          return `<li><small>${esc(a.by)} · ${esc(a.type)}${extra ? ` (${esc(extra)})` : ""}</small><br>${esc(text)}${button}${choices ? `<br><small>${choices}</small>` : ""}</li>`;
        });
        const links = j.externalUrls.map((l) => `<a href="${esc(l.url)}">${esc(l.label)}</a>`).join(" · ");
        return `<p><small>Agent session ${esc(s.id.slice(0, 8))} by ${esc(j.creator)}, ${esc(j.status)}${links ? ` · ${links}` : ""}</small></p><ol>${lines.join("")}</ol>`;
      });
    const state = W.state(w, i.stateId)?.name;
    const delegate = W.user(w, i.delegateId)?.name;
    return `<h2>${esc(W.identifier(w, i))} ${esc(i.title)}</h2><small>${esc(state)}${delegate ? ` · delegated to ${esc(delegate)}` : ""}</small>${sessions.join("")}`;
  });
  return `<meta http-equiv="refresh" content="3"><h1>Acme</h1><p>The Linear workspace of the stand-in that <code>cargo xtask dev</code> runs. <code>cargo xtask linear delegate ENG-1</code> delegates an issue to fctl.</p>${items.join("")}`;
}

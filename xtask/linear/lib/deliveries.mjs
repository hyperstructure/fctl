// The webhook deliveries Linear sends fctl's app (linear.app/developers/
// agent-interaction), shaped as Linear's schema has them
// (AgentSessionEventWebhookPayload, OAuthAppWebhookPayload) and signed as
// Linear signs them: AgentSessionEvent `created` when someone delegates an
// issue to fctl or mentions it, `prompted` when they reply in its agent
// session or press stop, and OAuthApp `revoked` when fctl's app is
// uninstalled.

import { createHmac, randomUUID } from "node:crypto";
import * as W from "./world.mjs";

/** The `Linear-Signature` of a body: its HMAC-SHA256 with the webhook's secret, in hex. */
export function sign(secret, body) {
  return createHmac("sha256", secret).update(body).digest("hex");
}

const userChild = (u) => ({
  id: u.id,
  name: u.name,
  email: u.email,
  avatarUrl: u.avatarUrl,
  url: `https://linear.app/${W.ORGANIZATION.urlKey}/profiles/${u.displayName}`,
});

const issueChild = (w, i) => {
  const t = W.team(w, i.teamId);
  return {
    id: i.id,
    identifier: W.identifier(w, i),
    title: i.title,
    description: i.description,
    url: W.issueUrl(w, i),
    teamId: t.id,
    team: { id: t.id, key: t.key, name: t.name },
  };
};

const commentChild = (c) => ({ id: c.id, body: c.body, userId: c.userId, issueId: c.issueId });

function sessionPayload(w, s) {
  const i = W.issue(w, s.issueId);
  const c = W.comment(w, s.commentId);
  const creator = W.user(w, s.creatorId);
  return {
    id: s.id,
    type: "commentThread",
    status: W.sessionStatus(s),
    url: `${W.issueUrl(w, i)}#agent-session-${s.id.slice(0, 8)}`,
    organizationId: W.ORGANIZATION.id,
    appUserId: s.appUserId,
    issueId: i.id,
    issue: issueChild(w, i),
    commentId: c?.id ?? null,
    comment: c ? commentChild(c) : null,
    creatorId: creator?.id ?? null,
    creator: creator ? userChild(creator) : null,
    createdAt: s.createdAt,
    updatedAt: s.updatedAt,
    startedAt: null,
    endedAt: null,
    archivedAt: null,
    summary: null,
  };
}

const xml = (text) =>
  String(text ?? "").replace(/[<>&]/g, (c) => ({ "<": "&lt;", ">": "&gt;", "&": "&amp;" })[c]);

function thread(w, tag, root) {
  const replies = w.comments.filter((c) => c.parentId === root.id);
  const lines = [root, ...replies].map(
    (c) =>
      `<comment author="${xml(W.user(w, c.userId)?.name)}" created-at="${c.createdAt}">${xml(c.body)}</comment>`,
  );
  return `<${tag} comment-id="${root.id}">\n${lines.join("\n")}\n</${tag}>`;
}

/**
 * What Linear writes for an agent about an agent session's issue: the issue,
 * the thread that asked (for a mention), and the issue's other threads.
 */
export function promptContext(w, s) {
  const i = W.issue(w, s.issueId);
  const t = W.team(w, i.teamId);
  const lines = [`<issue identifier="${W.identifier(w, i)}">`, `<title>${xml(i.title)}</title>`];
  if (i.description) lines.push(`<description>${xml(i.description)}</description>`);
  lines.push(`<team name="${xml(t.name)}"/>`);
  for (const id of i.labelIds) lines.push(`<label>${xml(W.label(w, id)?.name)}</label>`);
  const parent = W.issue(w, i.parentId);
  if (parent) {
    lines.push(`<parent-issue identifier="${W.identifier(w, parent)}">${xml(parent.title)}</parent-issue>`);
  }
  const project = W.project(w, i.projectId);
  if (project) lines.push(`<project name="${xml(project.name)}"/>`);
  lines.push("</issue>");
  const asked = W.comment(w, s.commentId);
  // A delegation's thread starts with Linear's own words, which ask nothing.
  if (asked?.body && !asked.anchor) lines.push(thread(w, "primary-directive-thread", asked));
  for (const c of w.comments.filter((c) => c.issueId === i.id && !c.parentId && c.id !== asked?.id)) {
    lines.push(thread(w, "other-thread", c));
  }
  return lines.join("\n");
}

function envelope(config, now) {
  return {
    createdAt: now.toISOString(),
    organizationId: W.ORGANIZATION.id,
    oauthClientId: config.clientId,
    webhookTimestamp: now.getTime(),
    webhookId: randomUUID(),
  };
}

/** `created`: someone delegated agent session `s`'s issue to fctl, or mentioned it. */
export function created(w, config, s, now = new Date()) {
  const asked = W.comment(w, s.commentId);
  const i = W.issue(w, s.issueId);
  return {
    type: "AgentSessionEvent",
    action: "created",
    ...envelope(config, now),
    appUserId: s.appUserId,
    agentSession: sessionPayload(w, s),
    promptContext: promptContext(w, s),
    previousComments: w.comments
      .filter((c) => c.issueId === i.id && c.id !== asked?.id && c.createdAt <= s.createdAt)
      .map(commentChild),
    guidance: [],
  };
}

/** `prompted`: someone replied in agent session `s` (`a`, their prompt), or pressed stop. */
export function prompted(w, config, s, a, now = new Date()) {
  return {
    type: "AgentSessionEvent",
    action: "prompted",
    ...envelope(config, now),
    appUserId: s.appUserId,
    agentSession: sessionPayload(w, s),
    agentActivity: {
      id: a.id,
      agentSessionId: s.id,
      content: a.content,
      signal: a.signal,
      signalMetadata: a.signalMetadata,
      sourceCommentId: a.sourceCommentId ?? null,
      userId: a.userId,
      user: userChild(W.user(w, a.userId)),
      createdAt: a.createdAt,
      updatedAt: a.createdAt,
      archivedAt: null,
    },
  };
}

/** `revoked`: fctl's app was uninstalled from the Linear workspace. */
export function revoked(config, now = new Date()) {
  return { type: "OAuthApp", action: "revoked", ...envelope(config, now) };
}

/**
 * Sends a delivery to fctl's webhook, signed: fctl's answer, and how long
 * it took (Linear waits five seconds).
 */
export async function deliver(url, secret, payload) {
  const body = JSON.stringify(payload);
  const started = Date.now();
  const res = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "User-Agent": "Linear-Webhook",
      "Linear-Delivery": randomUUID(),
      "Linear-Event": payload.type,
      "Linear-Signature": sign(secret, body),
      "Linear-Timestamp": String(payload.webhookTimestamp),
    },
    body,
    signal: AbortSignal.timeout(30_000),
  });
  const text = await res.text();
  return { status: res.status, body: text, ms: Date.now() - started };
}

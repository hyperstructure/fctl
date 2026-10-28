import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { test } from "node:test";
import { created, promptContext, prompted, revoked, sign } from "./deliveries.mjs";
import { differences, loadSchema } from "./graphql.mjs";
import * as W from "./world.mjs";

const schema = loadSchema();
const config = { clientId: "fctl-dev-linear" };

test("signs a delivery's body as fctl_domain::linear::signed checks it", () => {
  const body = JSON.stringify({ type: "AgentSessionEvent" });
  assert.equal(sign("secret", body), createHmac("sha256", "secret").update(body).digest("hex"));
  assert.match(sign("secret", body), /^[0-9a-f]{64}$/);
});

test("delivers what Linear's schema says its webhooks hold", () => {
  const w = W.seed();
  const i = W.issue(w, "ENG-1");
  const delegated = W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: null });
  const c = W.addComment(w, { issueId: i.id, userId: W.DEV_USER, body: "@fctl what's left?" });
  const mentioned = W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: c.id });
  const reply = W.addActivity(mentioned, { userId: W.DEV_USER, content: { type: "prompt", body: "Go on" } });
  const stop = W.addActivity(mentioned, {
    userId: W.DEV_USER,
    content: { type: "prompt", body: "" },
    signal: "stop",
  });
  for (const payload of [
    created(w, config, delegated),
    created(w, config, mentioned),
    prompted(w, config, mentioned, reply),
    prompted(w, config, mentioned, stop),
  ]) {
    assert.deepEqual(differences(schema, "AgentSessionEventWebhookPayload", payload), []);
    assert.ok(Math.abs(payload.webhookTimestamp - Date.now()) < 5_000);
  }
  assert.deepEqual(differences(schema, "OAuthAppWebhookPayload", revoked(config)), []);
  // And the check finds what isn't so.
  const wrong = { ...created(w, config, delegated), agentSession: { id: 1 }, extra: true };
  assert.deepEqual(differences(schema, "AgentSessionEventWebhookPayload", wrong).slice(0, 3), [
    "AgentSessionEventWebhookPayload.extra isn't a field of AgentSessionEventWebhookPayload",
    "AgentSessionEventWebhookPayload.agentSession.appUserId is missing",
    "AgentSessionEventWebhookPayload.agentSession.createdAt is missing",
  ]);
});

test("a delegation comes with Linear's own thread, and a mention with what the person wrote", () => {
  const w = W.seed();
  const i = W.issue(w, "ENG-1");
  const anchor = W.addAnchor(w, i);
  const s = W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: anchor.id });
  const delegated = created(w, config, s);
  assert.equal(delegated.agentSession.comment.body, "This thread is for an agent session with fctl.");
  assert.equal(delegated.agentSession.issue.identifier, "ENG-1");
  assert.equal(delegated.agentSession.creator.email, "dev@fctl.test");
  // Linear's words ask nothing: they aren't what the agent is asked.
  assert.doesNotMatch(promptContext(w, s), /primary-directive-thread|agent session with/);
  const c = W.addComment(w, { issueId: i.id, userId: W.DEV_USER, body: "@fctl what's left?" });
  const mentioned = created(w, config, W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: c.id }));
  assert.equal(mentioned.agentSession.comment.body, "@fctl what's left?");
});

test("writes the issue, the thread that asked and the others for the agent, as Linear does", () => {
  const w = W.seed();
  const i = W.issue(w, "ENG-2");
  const c = W.addComment(w, { issueId: i.id, userId: W.DEV_USER, body: "@fctl add <the> tests" });
  const context = promptContext(w, W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: c.id }));
  assert.match(context, /^<issue identifier="ENG-2">\n<title>Add a section on the tests<\/title>/);
  assert.match(
    context,
    /<parent-issue identifier="ENG-1">Notes on what the playground holds<\/parent-issue>/,
  );
  assert.match(
    context,
    /<primary-directive-thread comment-id="[^"]+">\n<comment author="Dev Owner" [^>]+>@fctl add &lt;the&gt; tests<\/comment>/,
  );
  const notes = promptContext(w, W.addSession(w, { issue: W.issue(w, "ENG-1"), creatorId: W.DEV_USER }));
  assert.match(
    notes,
    /<other-thread [^>]+>\n<comment author="Grace Hopper" [^>]+>The README says nothing about the tests yet.<\/comment>/,
  );
  assert.match(notes, /<label>Feature<\/label>/);
});

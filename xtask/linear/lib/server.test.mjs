import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:http";
import { test } from "node:test";
import { loadSchema } from "./graphql.mjs";
import { createStandin } from "./server.mjs";
import * as W from "./world.mjs";

const schema = loadSchema();
const quiet = { log: () => {}, error: () => {} };

/** fctl's webhook, as far as the stand-in sees it: it checks the signature, keeps what came, and answers. */
async function fctl(secret) {
  const got = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (c) => {
      body += c;
    });
    req.on("end", () => {
      const signed =
        createHmac("sha256", secret).update(body).digest("hex") === req.headers["linear-signature"];
      got.push({ signed, event: req.headers["linear-event"], payload: JSON.parse(body) });
      res.writeHead(signed ? 200 : 401, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ ok: signed }));
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  return { got, server, url: `http://127.0.0.1:${server.address().port}/api/v1/linear/webhook` };
}

async function standin(webhook) {
  const config = { clientId: "id", clientSecret: "secret", webhookSecret: "hook", webhook };
  const s = createStandin({ schema, config, out: quiet, answerWithinMs: 200 });
  s.server.listen(0, "127.0.0.1");
  await once(s.server, "listening");
  const base = `http://127.0.0.1:${s.server.address().port}`;
  const call = async (method, path, body) => {
    const res = await fetch(`${base}${path}`, {
      method,
      headers: { "Content-Type": "application/json" },
      body: body && JSON.stringify(body),
    });
    return { status: res.status, body: await res.json() };
  };
  return { ...s, base, call };
}

test("people's delegations, mentions, replies and stops reach fctl's webhook, signed", async (t) => {
  const hook = await fctl("hook");
  const s = await standin(hook.url);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const delegated = await s.call("POST", "/standin/delegate", { issue: "ENG-1" });
  assert.equal(delegated.status, 200);
  assert.equal(delegated.body.delivery.status, 200);
  const id = delegated.body.session.id;
  assert.equal(W.issue(s.world, "ENG-1").delegateId, W.APP_USER);
  const mention = await s.call("POST", "/standin/mention", {
    title: "Tidy up",
    body: "what's left?",
    as: "grace",
  });
  assert.equal(mention.body.session.issue, "ENG-6");
  assert.equal(mention.body.session.mention, "@fctl what's left?");
  await s.call("POST", "/standin/reply", { issue: "ENG-1", body: "Go on" });
  await s.call("POST", "/standin/stop", { session: id.slice(0, 8) });
  assert.deepEqual(
    hook.got.map((g) => [g.signed, g.event, g.payload.action, g.payload.agentSession.issue.identifier]),
    [
      [true, "AgentSessionEvent", "created", "ENG-1"],
      [true, "AgentSessionEvent", "created", "ENG-6"],
      [true, "AgentSessionEvent", "prompted", "ENG-1"],
      [true, "AgentSessionEvent", "prompted", "ENG-1"],
    ],
  );
  assert.equal(hook.got[1].payload.agentSession.creator.name, "Grace Hopper");
  assert.equal(hook.got[2].payload.agentActivity.content.body, "Go on");
  assert.equal(hook.got[3].payload.agentActivity.signal, "stop");
  const sessions = await s.call("GET", "/standin/sessions?issue=ENG-1");
  assert.deepEqual(
    sessions.body.sessions[0].activities.map((a) => [a.by, a.type, a.body, a.signal]),
    [
      ["Dev Owner", "prompt", "Go on", null],
      ["Dev Owner", "prompt", "", "stop"],
    ],
  );
  assert.equal((await s.call("POST", "/standin/reply", { issue: "DES-1", body: "Hi" })).status, 404);
  assert.equal((await s.call("POST", "/standin/delegate", { issue: "ENG-1", as: "ada" })).status, 400);
  // Neither agent session heard from fctl, which only says it took the deliveries.
  await new Promise((r) => setTimeout(r, 300));
  const { problems } = (await s.call("GET", "/standin/problems")).body;
  assert.deepEqual(
    problems.map((p) => p.kind),
    ["unresponsive", "unresponsive"],
  );
});

test("what Linear wouldn't answer, or fctl's webhook refused, is a problem", async (t) => {
  const hook = await fctl("another secret");
  const s = await standin(hook.url);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  await s.call("POST", "/standin/delegate", { issue: "ENG-1" });
  assert.equal((await s.call("GET", "/oauth/userinfo")).status, 404);
  const graphql = await fetch(`${s.base}/graphql`, { method: "POST", body: "{}" });
  assert.equal(graphql.status, 401);
  const { problems } = (await s.call("GET", "/standin/problems")).body;
  assert.deepEqual(
    problems.map((p) => p.kind),
    ["delivery", "unknown", "auth"],
  );
  assert.match(
    problems[0].message,
    /fctl answered the delivery \(Dev Owner delegated ENG-1 to fctl\) with 401/,
  );
});

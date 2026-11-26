import assert from "node:assert/strict";
import { createHmac, createPublicKey, verify } from "node:crypto";
import { once } from "node:events";
import { createServer } from "node:http";
import { connect } from "node:net";
import { test } from "node:test";
import { CARD, CHOOSE, HOME, QUESTION, SEND_FORM, START_FORM } from "./fixtures.mjs";
import { createStandin } from "./server.mjs";
import * as W from "./world.mjs";

const quiet = { log: () => {}, error: () => {} };
const SECRET = "signing-secret";
const SCOPES =
  "app_mentions:read,assistant:write,channels:history,channels:join,channels:read,chat:write,commands,groups:history,groups:read,im:history,im:read,im:write,reactions:write,search:read.public,team:read,users:read,users:read.email";

/**
 * fctl's three addresses, as far as the stand-in sees them: each checks
 * Slack's signature (as fctl_domain::slack::signed does), keeps what came,
 * and answers as `behave` says, then does what it says after (as fctl does
 * once it answered).
 */
async function fctl(behave = {}) {
  const got = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (c) => {
      body += c;
    });
    req.on("end", async () => {
      const ts = req.headers["x-slack-request-timestamp"];
      const mac = `v0=${createHmac("sha256", SECRET).update(`v0:${ts}:${body}`).digest("hex")}`;
      const signed =
        mac === req.headers["x-slack-signature"] && Math.abs(Date.now() / 1000 - Number(ts)) < 300;
      const kind = req.url.split("/").pop();
      const form = new URLSearchParams(body);
      const payload =
        kind === "events"
          ? JSON.parse(body)
          : kind === "interactivity"
            ? JSON.parse(form.get("payload"))
            : Object.fromEntries(form);
      const entry = { kind, signed, headers: req.headers, payload, type: req.headers["content-type"] };
      got.push(entry);
      if (!signed) {
        res.writeHead(401);
        return res.end("unsigned");
      }
      const answer = (await behave[kind]?.(payload, entry)) ?? {};
      res.writeHead(answer.status ?? 200, { "Content-Type": "application/json", ...answer.headers });
      res.end(answer.body ?? "");
      await answer.after?.();
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const base = `http://127.0.0.1:${server.address().port}/api/v1/slack`;
  const events = (type) => got.filter((g) => g.kind === "events" && g.payload.event?.type === type);
  return {
    got,
    server,
    events,
    urls: { events: `${base}/events`, interactivity: `${base}/interactivity`, commands: `${base}/commands` },
  };
}

async function standin(hook, { install = true, ...options } = {}) {
  const config = { clientId: "id", clientSecret: "secret", signingSecret: SECRET, ...hook.urls };
  const s = createStandin({ config, out: quiet, retryDelaysMs: [20, 40, 60], ...options });
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
  const out = { ...s, base, call, token: null };
  /** The Web API as fctl calls it: JSON with its bot token for writes, forms for reads. */
  out.write = async (method, args, token = out.token) => {
    const res = await fetch(`${base}/api/${method}`, {
      method: "POST",
      headers: { "Content-Type": "application/json; charset=utf-8", Authorization: `Bearer ${token}` },
      body: JSON.stringify(args),
    });
    return { status: res.status, headers: res.headers, ...(await res.json()) };
  };
  out.read = async (method, fields, token = out.token) => {
    const res = await fetch(`${base}/api/${method}`, {
      method: "POST",
      headers: { "Content-Type": "application/x-www-form-urlencoded", Authorization: `Bearer ${token}` },
      body: new URLSearchParams(fields),
    });
    return { status: res.status, headers: res.headers, ...(await res.json()) };
  };
  if (install) out.token = await installApp(base);
  return out;
}

/** Installs fctl's app as an admin's browser does: the install page, back to fctl, and oauth.v2.access. */
async function installApp(base) {
  const redirect = "http://localhost:8787/api/v1/slack/installed";
  const page = new URL(`${base}/oauth/v2/authorize`);
  for (const [k, v] of Object.entries({
    client_id: "id",
    scope: SCOPES,
    redirect_uri: redirect,
    state: "st",
    auto: "1",
  })) {
    page.searchParams.set(k, v);
  }
  const back = new URL((await fetch(page, { redirect: "manual" })).headers.get("location"));
  const res = await fetch(`${base}/api/oauth.v2.access`, {
    method: "POST",
    body: new URLSearchParams({
      client_id: "id",
      client_secret: "secret",
      code: back.searchParams.get("code"),
      redirect_uri: redirect,
    }),
  });
  const answer = await res.json();
  assert.equal(answer.ok, true);
  return answer.access_token;
}

const fctlMessages = (thread) => thread.messages.filter((m) => m.fctl);

test("a mention reaches fctl signed, as Slack sends it, and what fctl posts shows in the thread", async (t) => {
  const hook = await fctl();
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const mention = await s.call("POST", "/standin/mention", { text: "@fctl fix the flaky login test" });
  assert.equal(mention.status, 200);
  assert.equal(mention.body.delivery.status, 200);
  // fctl wasn't in #web-team: Dev Owner invites it, and only the mention comes.
  assert.equal(mention.body.invited, true);
  assert.deepEqual(
    mention.body.deliveries.map((d) => d.event),
    ["app_mention"],
  );
  const [e] = hook.events("app_mention");
  assert.ok(e.signed);
  assert.equal(e.type, "application/json");
  assert.equal(e.payload.type, "event_callback");
  assert.equal(e.payload.event.text, `<@${W.BOT_USER}> fix the flaky login test`);
  assert.equal(e.payload.event.user, "U0DEV");
  assert.equal(e.payload.event.channel, "C0WEB");
  assert.ok(e.payload.event.action_token);
  assert.equal(e.payload.event.blocks[0].type, "rich_text");
  const ts = mention.body.ts;
  assert.equal(mention.body.link, `https://acme.slack.com/archives/C0WEB/p${ts.replace(".", "")}`);

  // fctl takes it up: a reaction, its agent session, its card in the thread.
  assert.equal((await s.write("reactions.add", { channel: "C0WEB", timestamp: ts, name: "eyes" })).ok, true);
  const status = await s.write("agents.sessions.setStatus", {
    channel_id: "C0WEB",
    thread_ts: ts,
    status: "processing",
    title: "Fix the flaky login test",
    initiator_user_id: "U0DEV",
  });
  assert.equal(status.ok, true);
  const card = await s.write("chat.postMessage", {
    channel: "C0WEB",
    thread_ts: ts,
    text: "web#1: Fix the flaky login test (Working)",
    blocks: CARD,
    unfurl_links: false,
    unfurl_media: false,
  });
  assert.equal(card.ok, true);
  assert.equal(card.headers.get("x-oauth-scopes"), SCOPES);
  // Slack sends an app its own messages too.
  await s.settled();
  const own = hook.events("message").find((g) => g.payload.event.bot_id === W.BOT_ID);
  assert.equal(own.payload.event.ts, card.ts);
  const thread = (await s.call("GET", `/standin/threads/C0WEB/${ts}`)).body;
  assert.equal(thread.channel.fctl, true);
  assert.deepEqual(thread.messages[0].reactions, [{ name: "eyes", by: ["fctl"] }]);
  assert.equal(fctlMessages(thread).length, 1);
  assert.match(fctlMessages(thread)[0].shown, /Fix the flaky login test/);
  assert.match(fctlMessages(thread)[0].shown, /\[x\] Reproduce it/);
  assert.deepEqual([thread.agentSession.status, thread.agentSession.initiator], ["processing", "Dev Owner"]);

  // Grace replies, then mentions fctl in the thread: a message event, then both events.
  const reply = await s.call("POST", "/standin/reply", {
    thread: ts,
    text: "it fails on main too",
    as: "grace",
  });
  assert.deepEqual(
    reply.body.deliveries.map((d) => [d.event, d.status]),
    [["message", 200]],
  );
  const replied = hook.events("message").at(-1).payload.event;
  assert.deepEqual(
    [replied.thread_ts, replied.parent_user_id, replied.channel_type, replied.user],
    [ts, "U0DEV", "channel", "U0GRACE"],
  );
  const again = await s.call("POST", "/standin/mention", {
    thread: ts,
    text: "can you also check the docs?",
    as: "grace",
  });
  assert.deepEqual(again.body.deliveries.map((d) => d.event).sort(), ["app_mention", "message"]);
  assert.equal(hook.events("app_mention").at(-1).payload.event.thread_ts, ts);

  // In someone's messages with fctl: message.im.
  const dm = await s.call("POST", "/standin/dm", { text: "what runs now?" });
  const im = hook.events("message").at(-1).payload.event;
  assert.deepEqual(
    [dm.body.channel.id, im.channel, im.channel_type, im.user],
    ["D0DEV", "D0DEV", "im", "U0DEV"],
  );
  assert.ok(im.action_token);
  const mine = (await s.call("GET", "/standin/channels/dm")).body;
  assert.deepEqual(
    [mine.channel.id, mine.channel.with, mine.messages.at(-1).text],
    ["D0DEV", "Dev Owner", "what runs now?"],
  );
  assert.equal((await s.call("GET", "/standin/channels/dm?as=grace")).body.channel.id, "D0GRACE");
  // A guest, a stranger and a channel nobody named are turned down.
  assert.equal((await s.call("POST", "/standin/mention", { channel: "incidents", as: "ada" })).status, 403);
  assert.equal((await s.call("POST", "/standin/mention", { as: "nobody" })).status, 400);
  assert.equal((await s.call("POST", "/standin/post", { channel: "general", text: "hi" })).status, 404);
  // A mention in a channel shared with another organization says so.
  await s.call("POST", "/standin/mention", { channel: "partners" });
  assert.equal(hook.events("app_mention").at(-1).payload.is_ext_shared_channel, true);
  assert.deepEqual((await s.call("GET", "/standin/problems")).body.problems, []);
});

test("people write where fctl is, and fctl hears it; where it isn't, Slack sends nothing", async (t) => {
  const hook = await fctl();
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  assert.equal(
    (await s.call("POST", "/standin/invite", { channel: "deploys", as: "grace" })).body.channel.fctl,
    true,
  );
  const posted = await s.call("POST", "/standin/post", {
    channel: "deploys",
    text: "Deploying web build 413",
    as: "grace",
  });
  assert.deepEqual(
    posted.body.deliveries.map((d) => d.event),
    ["message"],
  );
  const e = hook.events("message").at(-1).payload.event;
  assert.deepEqual(
    [e.text, e.channel_type, e.thread_ts, e.action_token],
    ["Deploying web build 413", "channel", undefined, undefined],
  );
  await s.call("POST", "/standin/kick", { channel: "deploys" });
  const unheard = await s.call("POST", "/standin/post", { channel: "deploys", text: "Done", as: "grace" });
  assert.deepEqual([unheard.body.deliveries, unheard.body.delivery], [[], null]);
  // Nobody invites fctl where it was mentioned: Slack sends it nothing.
  const ignored = await s.call("POST", "/standin/mention", { channel: "random", invite: false });
  assert.deepEqual([ignored.body.invited, ignored.body.deliveries], [false, []]);
  assert.equal((await s.call("GET", "/standin/channels/random")).body.channel.fctl, false);
  assert.equal((await s.call("POST", "/standin/press", {})).status, 400);
  const world = (await s.call("GET", "/standin/world")).body;
  assert.deepEqual(
    world.people.map((p) => [p.id, p.handle, p.guest, p.dm]),
    [
      ["U0DEV", "dev", false, "D0DEV"],
      ["U0GRACE", "grace", false, "D0GRACE"],
      ["U0GUEST", "ada", true, "D0GUEST"],
    ],
  );
  assert.deepEqual(
    world.channels.map((c) => [c.id, c.name, c.kind]),
    [
      ["C0WEB", "web-team", "public"],
      ["C0INC", "incidents", "public"],
      ["C0DEP", "deploys", "public"],
      ["C0RAND", "random", "public"],
      ["G0SECRET", "secret", "private"],
      ["C0PART", "partners", "shared"],
    ],
  );
});

test("buttons, a choice only one person sees, and what fctl answers at the response_url", async (t) => {
  const hook = await fctl({
    interactivity: async (p) => {
      const action = p.actions?.[0];
      if (action?.action_id === "fctl_answer_1") {
        return {
          after: async () => {
            await fetch(p.response_url, {
              method: "POST",
              headers: { "Content-Type": "application/json; charset=utf-8" },
              body: JSON.stringify({
                response_type: "ephemeral",
                replace_original: false,
                text: "Answered.",
              }),
            });
          },
        };
      }
      if (action?.action_id.startsWith("fctl_project")) {
        return {
          after: async () => {
            await fetch(p.response_url, { method: "POST", body: JSON.stringify({ delete_original: true }) });
          },
        };
      }
      return {};
    },
  });
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const asked = await s.call("POST", "/standin/mention", { channel: "incidents", text: "ship it?" });
  const ts = asked.body.ts;
  const q = await s.write("chat.postMessage", {
    channel: "C0INC",
    thread_ts: ts,
    text: "The agent asks",
    blocks: QUESTION,
  });
  const pressed = await s.call("POST", "/standin/answer", { thread: ts, answer: 1, wait: 2000 });
  assert.equal(pressed.status, 200);
  assert.equal(pressed.body.thread.ts, ts, "the thread the answer is in, to follow");
  assert.equal(pressed.body.delivery.status, 200);
  assert.deepEqual(pressed.body.pressed, {
    action_id: "fctl_answer_1",
    block_id: "fctl_q:ses_1:1700",
    value: "1",
  });
  assert.equal(pressed.body.responses[0].effect, "told Dev Owner, only them");
  const p = hook.got.findLast((g) => g.kind === "interactivity").payload;
  assert.equal(p.type, "block_actions");
  assert.deepEqual(p.container, {
    type: "message",
    message_ts: q.ts,
    channel_id: "C0INC",
    is_ephemeral: false,
  });
  assert.equal(p.message.thread_ts, ts);
  assert.equal(p.channel.name, "incidents");
  assert.deepEqual([p.user.id, p.team.id], ["U0DEV", "T0ACME"]);
  assert.match(p.response_url, /\/standin\/respond\/[0-9a-f]+$/);
  assert.ok(p.trigger_id);
  // fctl replaces the question with its answer.
  await s.write("chat.update", {
    channel: "C0INC",
    ts: q.ts,
    text: "Dev Owner answered: Not yet",
    blocks: QUESTION.slice(0, 1),
  });
  const thread = (await s.call("GET", `/standin/threads/C0INC/${ts}`)).body;
  assert.equal(thread.ephemeral[0].to, "Dev Owner");
  assert.equal(thread.ephemeral[0].text, "Answered.");
  assert.equal(thread.messages.at(-1).blocks.length, 1);

  // Which project? Only Grace sees the choice; she remembers it for the channel.
  const graces = await s.call("POST", "/standin/mention", {
    channel: "incidents",
    text: "look at the deploy",
    as: "grace",
  });
  const only = await s.write("chat.postEphemeral", {
    channel: "C0INC",
    user: "U0GRACE",
    thread_ts: graces.body.ts,
    text: "Which project should work on this?",
    blocks: CHOOSE,
  });
  assert.equal(only.ok, true);
  assert.equal(
    (await s.call("POST", "/standin/press", { project: "web" })).status,
    404,
    "Dev Owner sees no choice",
  );
  const chose = await s.call("POST", "/standin/project", {
    as: "grace",
    project: "web",
    remember: true,
    wait: 2000,
  });
  assert.equal(chose.status, 200);
  assert.equal(chose.body.thread.ts, graces.body.ts);
  const c = hook.got.findLast((g) => g.kind === "interactivity").payload;
  assert.deepEqual(c.container, {
    type: "message",
    message_ts: only.message_ts,
    channel_id: "C0INC",
    is_ephemeral: true,
  });
  assert.equal(c.message, undefined);
  assert.equal(c.actions[0].value, "T0ACME:C0WEB:1.2|prj_2");
  assert.deepEqual(
    c.state.values.fctl_remember.fctl_remember.selected_options.map((o) => o.value),
    ["remember"],
  );
  assert.equal(chose.body.responses[0].effect, "deleted the original");
  const after = (await s.call("GET", `/standin/threads/C0INC/${graces.body.ts}`)).body;
  assert.equal(after.ephemeral[0].deleted, true);
  // A link button tells fctl too, and says where it goes.
  const open = await s.call("POST", "/standin/press", { channel: "incidents", label: "Open in fctl" });
  assert.equal(open.status, 404, "nothing to open in #incidents yet");
  await s.write("chat.postMessage", { channel: "C0INC", thread_ts: ts, text: "card", blocks: CARD });
  const link = await s.call("POST", "/standin/press", { channel: "incidents", label: "Open in fctl" });
  assert.equal(link.body.url, "https://fctl.acme.dev/s/ses_1");
  assert.deepEqual((await s.call("GET", "/standin/problems")).body.problems, []);
});

test("/fctl, Send to fctl…, the forms they open, and the Home tab", async (t) => {
  let s;
  const hook = await fctl({
    commands: async (p) => {
      if (!p.text)
        return {
          body: JSON.stringify({ response_type: "ephemeral", text: "/fctl <what to do> starts a session" }),
        };
      return {
        status: 200,
        after: async () => {
          await fetch(p.response_url, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ response_type: "ephemeral", text: "Nothing waits for you." }),
          });
        },
      };
    },
    interactivity: async (p) => {
      if (p.type === "message_action") {
        return { after: () => s.write("views.open", { trigger_id: p.trigger_id, view: SEND_FORM }) };
      }
      if (p.type === "shortcut")
        return { after: () => s.write("views.open", { trigger_id: p.trigger_id, view: START_FORM }) };
      if (p.type === "view_submission") {
        const task = p.view.state.values.task.task.value;
        if (!task)
          return {
            body: JSON.stringify({
              response_action: "errors",
              errors: { task: "Say what the agent should do" },
            }),
          };
        return { body: JSON.stringify({ response_action: "clear" }) };
      }
      if (p.type === "block_actions" && p.actions[0].action_id === "fctl_start") {
        return { after: () => s.write("views.open", { trigger_id: p.trigger_id, view: START_FORM }) };
      }
      return {};
    },
    events: async (p) => {
      if (p.event.type === "app_home_opened")
        return { after: () => s.write("views.publish", { user_id: p.event.user, view: HOME }) };
      return {};
    },
  });
  s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const status = await s.call("POST", "/standin/command", { text: "status" });
  assert.equal(status.body.delivery.status, 200);
  assert.equal(status.body.answer, null);
  assert.equal(status.body.thread, undefined, "fctl started no thread for /fctl status");
  assert.equal((await s.call("POST", "/standin/answer", {})).status, 400);
  assert.equal(status.body.responses[0].body.text, "Nothing waits for you.");
  const form = hook.got.findLast((g) => g.kind === "commands");
  assert.equal(form.type, "application/x-www-form-urlencoded");
  assert.deepEqual(
    [
      form.payload.command,
      form.payload.text,
      form.payload.channel_id,
      form.payload.channel_name,
      form.payload.user_id,
    ],
    ["/fctl", "status", "C0WEB", "web-team", "U0DEV"],
  );
  const help = await s.call("POST", "/standin/command", { text: "", channel: "dm" });
  assert.equal(help.body.answer.text, "/fctl <what to do> starts a session");
  assert.equal(hook.got.findLast((g) => g.kind === "commands").payload.channel_name, "directmessage");
  assert.equal((await s.call("POST", "/standin/command", { text: "x", thread: "1.2" })).status, 400);

  // Send to fctl… on the message about the failing deploy, then its form.
  const incident = (await s.call("GET", "/standin/channels/incidents")).body.messages.find(
    (m) => m.replies === 2,
  );
  const shortcut = await s.call("POST", "/standin/shortcut", { ts: incident.ts });
  assert.equal(shortcut.body.delivery.status, 200);
  assert.equal(shortcut.body.view.callback_id, "fctl_send");
  const action = hook.got.findLast((g) => g.kind === "interactivity").payload;
  assert.deepEqual(
    [action.type, action.callback_id, action.message_ts, action.channel.id],
    ["message_action", "fctl_send_message", incident.ts, "C0INC"],
  );
  assert.match(action.message.text, /migrations step/);
  const empty = await s.call("POST", "/standin/submit", { project: "Web" });
  assert.deepEqual(empty.body.answer, {
    response_action: "errors",
    errors: { task: "Say what the agent should do" },
  });
  assert.equal(empty.body.view.open, true);
  assert.equal((await s.call("POST", "/standin/submit", { project: "Elsewhere", task: "x" })).status, 400);
  const sent = await s.call("POST", "/standin/submit", {
    project: "Web",
    task: "Fix the deploy",
    updates: false,
  });
  assert.equal(sent.body.view.open, false);
  const submission = hook.got.findLast((g) => g.kind === "interactivity").payload;
  const values = submission.view.state.values;
  assert.deepEqual(
    [
      submission.type,
      submission.view.callback_id,
      values.project.project.selected_option.value,
      values.task.task.value,
    ],
    ["view_submission", "fctl_send", "prj_2", "Fix the deploy"],
  );
  assert.deepEqual(values.updates.updates.selected_options, []);
  assert.equal((await s.call("POST", "/standin/submit", {})).status, 404, "no form open any more");

  // The global shortcut, and the Home tab with its button.
  const global = await s.call("POST", "/standin/global-shortcut", { as: "grace" });
  assert.equal(global.body.view.callback_id, "fctl_start");
  assert.equal(
    hook.got.findLast((g) => g.kind === "interactivity").payload.callback_id,
    "fctl_start_session",
  );
  const home = await s.call("POST", "/standin/home", {});
  assert.equal(home.body.delivery.status, 200);
  assert.match(home.body.view.shown, /Needs you/);
  assert.equal(hook.events("app_home_opened")[0].payload.event.tab, "home");
  const start = await s.call("POST", "/standin/press", { home: true, action: "fctl_start", wait: 2000 });
  assert.equal(start.body.view.callback_id, "fctl_start");
  const press = hook.got.findLast((g) => g.kind === "interactivity").payload;
  assert.deepEqual(press.container, { type: "view", view_id: home.body.view.id });
  assert.equal(press.response_url, undefined);
  const views = (await s.call("GET", "/standin/views?as=dev")).body.views;
  assert.deepEqual(
    views.map((v) => [v.type, v.callback_id, v.open]),
    [
      ["modal", "fctl_send", false],
      ["home", "", null],
      ["modal", "fctl_start", true],
    ],
  );
  assert.deepEqual((await s.call("GET", "/standin/problems")).body.problems, []);
});

test("stop ends the streams in a thread and tells fctl; an uninstall ends its token", async (t) => {
  const hook = await fctl();
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const dm = await s.call("POST", "/standin/dm", { text: "explain the tests" });
  assert.equal(
    (await s.call("POST", "/standin/stop", { thread: dm.body.ts })).status,
    409,
    "no agent session yet",
  );
  const stream = await s.write("chat.startStream", {
    channel: "D0DEV",
    thread_ts: dm.body.ts,
    markdown_text: "Reading",
  });
  assert.equal(stream.ok, true);
  const stop = await s.call("POST", "/standin/stop", {});
  assert.equal(stop.body.delivery.status, 200);
  const e = hook.events("agent_session_stopped")[0].payload.event;
  assert.deepEqual(
    [e.channel, e.thread_ts, e.user, e.streaming_message_ts],
    ["D0DEV", dm.body.ts, "U0DEV", [stream.ts]],
  );
  const append = await s.write("chat.appendStream", {
    channel: "D0DEV",
    ts: stream.ts,
    markdown_text: " more",
  });
  assert.equal(append.error, "stopped_by_user");
  assert.equal(
    (await s.write("chat.stopStream", { channel: "D0DEV", ts: stream.ts, session_status: "active" })).ok,
    true,
  );
  assert.deepEqual(
    stop.body.thread.agentSession.stopped.map((x) => x.by),
    ["Dev Owner"],
  );

  const gone = await s.call("POST", "/standin/uninstall", {});
  assert.deepEqual(
    gone.body.deliveries.map((d) => [d.event, d.status]),
    [
      ["app_uninstalled", 200],
      ["tokens_revoked", 200],
    ],
  );
  assert.deepEqual(hook.events("tokens_revoked")[0].payload.event.tokens, { oauth: [], bot: [W.BOT_USER] });
  assert.equal((await s.write("auth.test", {})).error, "token_revoked");
  assert.equal((await s.call("POST", "/standin/mention", {})).status, 409);
  assert.equal((await s.call("GET", "/standin/health")).body.installed, false);
});

test("what Slack refuses, what fctl fails to answer, and the switches that rate-limit and fail calls", async (t) => {
  let failures = 1;
  const hook = await fctl({
    events: async (p) => {
      if (p.type === "url_verification") return { body: JSON.stringify({ challenge: p.challenge }) };
      if (p.event?.channel_type === "im" && failures-- > 0) return { status: 500, body: "down" };
      if (p.event?.text?.includes("never")) return { status: 500, headers: { "X-Slack-No-Retry": "1" } };
      return {};
    },
  });
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  // fctl fails an event once: Slack tries again, and says why.
  await s.call("POST", "/standin/dm", { text: "hello" });
  await s.settled();
  const tries = hook.events("message").filter((g) => g.payload.event.text === "hello");
  assert.deepEqual(
    tries.map((g) => [g.headers["x-slack-retry-num"], g.headers["x-slack-retry-reason"]]),
    [
      [undefined, undefined],
      ["1", "http_error"],
    ],
  );
  assert.equal(tries[0].payload.event_id, tries[1].payload.event_id);
  // Unless it asks Slack not to.
  await s.call("POST", "/standin/dm", { text: "never" });
  await s.settled();
  assert.equal(hook.events("message").filter((g) => g.payload.event.text === "never").length, 1);
  assert.equal((await s.call("POST", "/standin/verify", {})).body.ok, true);

  // Calls Slack would refuse are problems; Slack's state isn't.
  assert.equal(
    (await s.read("conversations.replies", { channel: "C0WEB", ts: "1.1" }, "")).error,
    "not_authed",
  );
  const json = await fetch(`${s.base}/api/conversations.info`, {
    method: "POST",
    headers: { "Content-Type": "application/json; charset=utf-8", Authorization: `Bearer ${s.token}` },
    body: JSON.stringify({ channel: "C0WEB" }),
  });
  assert.equal((await json.json()).error, "channel_not_found");
  const charset = await fetch(`${s.base}/api/auth.test`, {
    method: "POST",
    headers: { "Content-Type": "application/json", Authorization: `Bearer ${s.token}` },
    body: "{}",
  });
  assert.equal((await charset.json()).warning, "missing_charset");
  assert.equal(
    (await s.write("chat.postMessage", { channel: "D0DEV", blocks: [{ type: "carrot" }] })).error,
    "invalid_blocks",
  );
  assert.equal((await s.write("chat.postMessage", { channel: "C0WEB", text: "hi" })).error, "not_in_channel");
  const { problems, warnings } = (await s.call("GET", "/standin/problems")).body;
  assert.deepEqual(
    problems.map((p) => p.kind),
    ["delivery", "delivery", "api", "api", "api"],
  );
  assert.match(
    problems[0].message,
    /^fctl answered Dev Owner wrote to fctl with 500: down: Slack tries again$/,
  );
  assert.match(problems[1].message, /Slack gives up/);
  assert.match(problems[3].message, /fctl sent conversations\.info a JSON body, which Slack ignores/);
  assert.deepEqual(
    warnings.map((x) => x.warning),
    ["missing_charset"],
  );

  // Slack slows fctl down: the next call of conversations.replies is refused with Retry-After.
  await s.call("POST", "/standin/invite", { channel: "incidents" });
  const thread = (await s.call("GET", "/standin/channels/incidents")).body.messages.find(
    (m) => m.replies === 2,
  );
  const limit = await s.call("POST", "/standin/rate-limit", { calls: 1, retryAfter: 7 });
  assert.deepEqual(limit.body.rateLimit.methods, ["conversations.replies", "conversations.history"]);
  const limited = await s.read("conversations.replies", { channel: "C0INC", ts: thread.ts });
  assert.deepEqual(
    [limited.status, limited.headers.get("retry-after"), limited.error],
    [429, "7", "ratelimited"],
  );
  assert.equal((await s.read("conversations.replies", { channel: "C0INC", ts: thread.ts })).ok, true);
  // As Slack limits apps outside its Marketplace: a call a minute, 15 messages each.
  await s.call("POST", "/standin/rate-limit", { perMinute: 1, maxLimit: 2 });
  const first = await s.read("conversations.history", { channel: "C0INC", limit: "50" });
  assert.deepEqual([first.messages.length, first.has_more], [2, true]);
  const second = await s.read("conversations.history", { channel: "C0INC" });
  assert.equal(second.status, 429);
  assert.ok(Number(second.headers.get("retry-after")) > 50);
  await s.call("POST", "/standin/rate-limit", { off: true });
  assert.equal((await s.read("conversations.history", { channel: "C0INC" })).ok, true);
  // Slack refuses the plan block: fctl posts its card without it.
  await s.call("POST", "/standin/fail", { blocks: ["plan"] });
  assert.equal(
    (await s.write("chat.postMessage", { channel: "C0INC", text: "card", blocks: CARD })).error,
    "invalid_blocks",
  );
  assert.equal(
    (await s.write("chat.postMessage", { channel: "C0INC", text: "card", blocks: CARD.slice(0, 2) })).ok,
    true,
  );
  await s.call("POST", "/standin/fail", { method: "chat.update", error: "message_not_found", calls: 1 });
  const card = await s.write("chat.postMessage", {
    channel: "C0INC",
    text: "card",
    blocks: CARD.slice(0, 2),
  });
  assert.equal(
    (await s.write("chat.update", { channel: "C0INC", ts: card.ts, text: "x" })).error,
    "message_not_found",
  );
  assert.equal((await s.write("chat.update", { channel: "C0INC", ts: card.ts, text: "x" })).ok, true);
  assert.equal(
    (await s.call("GET", "/standin/problems")).body.problems.length,
    5,
    "what the switches fail is no problem",
  );
  assert.ok((await s.call("GET", "/standin/calls?method=chat.update")).body.calls.length >= 2);
});

test("signs people in with Slack through its pages, with an ID token its keys verify", async (t) => {
  const hook = await fctl();
  const s = await standin(hook, { install: false });
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const linked = "http://localhost:8787/api/v1/slack/linked";
  const query = new URLSearchParams({
    response_type: "code",
    scope: "openid profile email",
    client_id: "id",
    redirect_uri: linked,
    state: "st",
    nonce: "n",
  });
  // The page asks who signs in; its form sends the browser back.
  const page = await fetch(`${s.base}/openid/connect/authorize?${query}`);
  const html = await page.text();
  assert.equal(page.status, 200);
  assert.match(html, /Grace Hopper/);
  assert.match(html, /value="allow"/);
  const posted = await fetch(`${s.base}/openid/connect/authorize`, {
    method: "POST",
    body: new URLSearchParams({ ...Object.fromEntries(query), user: "U0GRACE", decision: "allow" }),
    redirect: "manual",
  });
  assert.equal(posted.status, 302);
  const back = new URL(posted.headers.get("location"));
  assert.equal(back.searchParams.get("state"), "st");
  const tokens = await (
    await fetch(`${s.base}/api/openid.connect.token`, {
      method: "POST",
      body: new URLSearchParams({
        client_id: "id",
        client_secret: "secret",
        code: back.searchParams.get("code"),
        redirect_uri: linked,
        grant_type: "authorization_code",
      }),
    })
  ).json();
  const [head, claims, sig] = tokens.id_token.split(".");
  const { keys } = await (await fetch(`${s.base}/openid/connect/keys`)).json();
  const key = keys.find((k) => k.kid === JSON.parse(Buffer.from(head, "base64url")).kid);
  assert.ok(
    verify(
      "sha256",
      Buffer.from(`${head}.${claims}`),
      createPublicKey({ key, format: "jwk" }),
      Buffer.from(sig, "base64url"),
    ),
  );
  assert.equal(JSON.parse(Buffer.from(claims, "base64url"))["https://slack.com/user_id"], "U0GRACE");
  const info = await s.read("openid.connect.userInfo", {}, tokens.access_token);
  assert.deepEqual(
    [info.ok, info.email, info["https://slack.com/team_name"]],
    [true, "grace@acme.test", "Acme"],
  );
  assert.deepEqual(
    await s.read("auth.revoke", {}, tokens.access_token).then(({ ok, revoked }) => ({ ok, revoked })),
    {
      ok: true,
      revoked: true,
    },
  );
  assert.equal((await s.read("openid.connect.userInfo", {}, tokens.access_token)).error, "token_revoked");
  const discovery = await (await fetch(`${s.base}/.well-known/openid-configuration`)).json();
  assert.equal(discovery.jwks_uri, `${s.base}/openid/connect/keys`);
  // The install page, with its Allow button.
  const install = await fetch(
    `${s.base}/oauth/v2/authorize?client_id=id&scope=chat:write&redirect_uri=${encodeURIComponent(linked)}`,
  );
  assert.match(await install.text(), /Allow/);
  assert.equal((await s.call("GET", "/standin/health")).body.installed, false);
  assert.equal((await s.call("POST", "/standin/mention", {})).status, 409);
});

test("stays up whatever a client sends", async (t) => {
  const hook = await fctl();
  const s = await standin(hook);
  t.after(() => {
    hook.server.close();
    s.server.close();
  });
  const send = async (raw, { close = false } = {}) => {
    const socket = connect(s.server.address().port, "127.0.0.1");
    await once(socket, "connect");
    let got = "";
    socket.on("data", (d) => {
      got += d;
    });
    socket.write(raw);
    if (close) {
      socket.destroy();
      return got;
    }
    socket.setTimeout(2_000, () => socket.destroy());
    await once(socket, "close");
    return got;
  };
  assert.match(await send("GET // HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"), /^HTTP\/1.1 400/);
  await send('POST /standin/mention HTTP/1.1\r\nHost: x\r\nContent-Length: 100\r\n\r\n{"text"', {
    close: true,
  });
  await send("GET /oauth/v2/authorize HTTP/1.1\r\nHost x\r\n\r\n");
  for (const body of ["{", "null", "5", "[]"]) {
    const bad = await fetch(`${s.base}/standin/mention`, { method: "POST", body });
    assert.equal(bad.status, 400, body);
    assert.match((await bad.json()).error, /a JSON object/);
  }
  assert.equal((await fetch(`${s.base}/standin/toString`, { method: "POST" })).status, 404);
  assert.equal(
    (
      await fetch(`${s.base}/api/chat.postMessage`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: "{",
      })
    ).status,
    200,
  );
  assert.equal((await fetch(`${s.base}/standin/respond/nope`, { method: "POST", body: "{}" })).status, 404);
  assert.equal((await s.call("GET", "/standin/health")).body.ok, true);
  assert.match(await (await fetch(`${s.base}/`)).text(), /#web-team/);
  assert.match(await (await fetch(`${s.base}/standin/avatar/U0GRACE.svg`)).text(), /GH/);
  const { problems } = (await s.call("GET", "/standin/problems")).body;
  assert.deepEqual(
    problems.filter((p) => p.kind === "standin"),
    [],
  );
});

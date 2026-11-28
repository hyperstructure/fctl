import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { test } from "node:test";
import * as D from "./deliveries.mjs";
import * as W from "./world.mjs";

test("signs as Slack does: Slack's own example (docs.slack.dev, verifying requests)", () => {
  const secret = "8f742231b10e8888abcd99yyyzzz85a5";
  const body =
    "token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
  assert.equal(
    D.sign(secret, "1531420618", body),
    "v0=a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503",
  );
  // As fctl_domain::slack::signed checks it: the hex HMAC of `v0:<timestamp>:<body>`.
  assert.equal(D.sign("s", "1", "{}"), `v0=${createHmac("sha256", "s").update("v0:1:{}").digest("hex")}`);
});

test("an app_mention comes in Slack's envelope, with its thread and the action token", () => {
  const w = W.seed();
  const root = W.messageAnywhere(
    w,
    w.messages.find((m) => m.channel === "C0INC" && !m.thread_ts && m.user === W.GRACE).ts,
  );
  const m = W.addMessage(w, {
    channel: "C0INC",
    user: W.DEV_USER,
    text: `<@${W.BOT_USER}> what broke?`,
    thread_ts: root.ts,
    client_msg_id: "c1",
  });
  const e = D.envelope(w, D.mentionEvent(w, m, { actionToken: "tok" }));
  assert.equal(e.type, "event_callback");
  assert.equal(e.team_id, "T0ACME");
  assert.equal(e.api_app_id, W.APP_ID);
  assert.match(e.event_id, /^Ev[0-9A-Z]{9}$/);
  assert.ok(Math.abs(e.event_time - Date.now() / 1000) < 5);
  assert.deepEqual(e.authorizations, [
    {
      enterprise_id: null,
      team_id: "T0ACME",
      user_id: W.BOT_USER,
      is_bot: true,
      is_enterprise_install: false,
    },
  ]);
  assert.equal(e.is_ext_shared_channel, false);
  assert.deepEqual(
    [
      e.event.type,
      e.event.user,
      e.event.channel,
      e.event.thread_ts,
      e.event.parent_user_id,
      e.event.action_token,
    ],
    ["app_mention", W.DEV_USER, "C0INC", root.ts, W.GRACE, "tok"],
  );
  assert.equal(e.event.event_ts, m.ts);
  assert.equal(e.event.channel_type, undefined);
  // Another event, another id: fctl tells retries by it.
  assert.notEqual(D.envelope(w, D.mentionEvent(w, m)).event_id, e.event_id);
});

test("message events say where they are: channel, group (private) or im", () => {
  const w = W.seed();
  const at = (channel) => W.addMessage(w, { channel, user: W.DEV_USER, text: "hi" });
  assert.equal(D.messageEvent(w, at("C0WEB")).channel_type, "channel");
  assert.equal(D.messageEvent(w, at("G0SECRET")).channel_type, "group");
  const dm = D.messageEvent(w, at("D0DEV"), { actionToken: "tok" });
  assert.deepEqual(
    [dm.type, dm.channel_type, dm.action_token, dm.user],
    ["message", "im", "tok", W.DEV_USER],
  );
  // A person's message in a channel shared with another organization says whose team they are of.
  const shared = D.messageEvent(w, at("C0PART"));
  assert.equal(shared.user_team, "T0ACME");
  assert.equal(shared.user_profile.real_name, "Dev Owner");
  // An app's own message carries its bot, as fctl must tell it from people's.
  const own = W.addMessage(w, {
    channel: "C0WEB",
    user: W.BOT_USER,
    bot: W.BOT_ID,
    app: W.APP_ID,
    text: "card",
  });
  const e = D.messageEvent(w, own);
  assert.deepEqual(
    [e.bot_id, e.app_id, e.bot_profile.name, e.subtype],
    [W.BOT_ID, W.APP_ID, "fctl", undefined],
  );
  const before = W.messageJson(w, own);
  own.text = "card, changed";
  const changed = D.changedEvent(w, own, before);
  assert.deepEqual(
    [changed.subtype, changed.hidden, changed.message.text, changed.previous_message.text],
    ["message_changed", true, "card, changed", "card"],
  );
});

test("interactions carry what fctl reads: the actor, the container, the actions and the state", () => {
  const w = W.seed();
  const dev = W.user(w, W.DEV_USER);
  const card = W.addMessage(w, {
    channel: "C0WEB",
    user: W.BOT_USER,
    bot: W.BOT_ID,
    app: W.APP_ID,
    text: "asks",
    thread_ts: w.messages.find((m) => m.channel === "C0WEB").ts,
  });
  const action = D.actionOf(
    { type: "button", action_id: "fctl_answer_1", text: { type: "plain_text", text: "No" }, value: "1" },
    "fctl_q:ses_1:17",
    undefined,
  );
  const p = D.blockActions(w, {
    user: dev,
    actions: [action],
    triggerId: "t",
    responseUrl: "http://r",
    state: {},
    message: card,
  });
  assert.deepEqual(p.user, { id: W.DEV_USER, username: "dev", name: "dev", team_id: "T0ACME" });
  assert.deepEqual(p.team, { id: "T0ACME", domain: "acme" });
  assert.deepEqual(p.container, {
    type: "message",
    message_ts: card.ts,
    channel_id: "C0WEB",
    is_ephemeral: false,
  });
  assert.equal(p.message.thread_ts, card.thread_ts);
  assert.deepEqual(
    [p.actions[0].action_id, p.actions[0].block_id, p.actions[0].value],
    ["fctl_answer_1", "fctl_q:ses_1:17", "1"],
  );
  // In a message only they see, Slack sends no message.
  const e = W.addEphemeral(w, { channel: "C0WEB", to: W.DEV_USER, text: "choose" });
  const only = D.blockActions(w, {
    user: dev,
    actions: [],
    triggerId: "t",
    responseUrl: "r",
    state: {},
    message: e,
    ephemeral: true,
  });
  assert.equal(only.container.is_ephemeral, true);
  assert.equal(only.message, undefined);
  const select = D.actionOf(
    { type: "static_select", action_id: "fctl_project", options: [] },
    "fctl_choose",
    { text: { type: "plain_text", text: "Web" }, value: "T0ACME:C0WEB:1.2|prj_1" },
  );
  assert.equal(select.selected_option.value, "T0ACME:C0WEB:1.2|prj_1");

  const form = D.command(w, {
    user: dev,
    channel: W.conversation(w, "G0SECRET"),
    text: "status",
    responseUrl: "http://r",
    triggerId: "t",
  });
  assert.equal(form.get("command"), "/fctl");
  assert.equal(form.get("channel_name"), "privategroup");
  assert.equal(form.get("is_enterprise_install"), "false");
  const shortcut = D.messageAction(w, {
    user: dev,
    message: card,
    callbackId: "fctl_send_message",
    triggerId: "t",
    responseUrl: "r",
  });
  assert.deepEqual(
    [shortcut.type, shortcut.channel.name, shortcut.message_ts, shortcut.message.text],
    ["message_action", "web-team", card.ts, "asks"],
  );
  assert.match(D.triggerId(), /^\d{10}\.\d{12}\.[0-9a-f]{32}$/);
});

test("sends what it signs, and says when fctl doesn't answer in time", async (t) => {
  const { createServer } = await import("node:http");
  const { once } = await import("node:events");
  const got = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (c) => {
      body += c;
    });
    req.on("end", () => {
      got.push({ headers: req.headers, body });
      if (req.url === "/slow") return setTimeout(() => res.end("late"), 300);
      res.writeHead(200, { "X-Slack-No-Retry": "1" });
      res.end("{}");
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => server.close());
  const url = `http://127.0.0.1:${server.address().port}`;
  const a = await D.deliver(`${url}/fast`, "s", "{}", "application/json", {
    retry: { num: 1, reason: "http_timeout" },
  });
  assert.deepEqual([a.status, a.body, a.noRetry], [200, "{}", true]);
  const h = got[0].headers;
  assert.equal(h["x-slack-signature"], D.sign("s", h["x-slack-request-timestamp"], "{}"));
  assert.ok(Math.abs(Number(h["x-slack-request-timestamp"]) - Date.now() / 1000) < 5);
  assert.deepEqual([h["x-slack-retry-num"], h["x-slack-retry-reason"]], ["1", "http_timeout"]);
  const late = await D.deliver(`${url}/slow`, "s", "{}", "application/json", { timeoutMs: 100 });
  assert.deepEqual([late.status, late.reason], [0, "http_timeout"]);
  const down = await D.deliver("http://127.0.0.1:1/x", "s", "{}", "application/json");
  assert.deepEqual([down.status, down.reason], [0, "connection_failed"]);
});

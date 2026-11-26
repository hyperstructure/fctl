import assert from "node:assert/strict";
import { test } from "node:test";
import * as A from "./api.mjs";
import { CARD, CHOOSE, HOME, plain, QUESTION, START_FORM } from "./fixtures.mjs";
import * as O from "./oauth.mjs";
import * as W from "./world.mjs";

const config = { clientId: "id", clientSecret: "secret" };
const SCOPES =
  "app_mentions:read,assistant:write,channels:history,channels:join,channels:read,chat:write,commands,groups:history,groups:read,im:history,im:read,im:write,reactions:write,search:read.public,team:read,users:read,users:read.email";

/** Acme with fctl's app installed, and a way to call the Web API with its bot token. */
function acme({ scope = SCOPES } = {}) {
  const w = W.seed();
  const codes = new Map();
  const redirect = "http://localhost:8787/api/v1/slack/installed";
  const params = new URLSearchParams({ client_id: "id", scope, redirect_uri: redirect, auto: "1" });
  const code = new URL(O.authorize(w, config, codes, "install", params).location).searchParams.get("code");
  const token = O.access(
    w,
    config,
    codes,
    new URLSearchParams({
      ...config,
      client_id: "id",
      client_secret: "secret",
      code,
      redirect_uri: redirect,
    }),
  ).body.access_token;
  const triggers = new Map();
  const call = (method, args = {}, more = {}) => {
    const ctx = {
      w,
      record: w.tokens[token],
      token,
      triggers,
      events: [],
      notes: [],
      warnings: [],
      problems: [],
      changed: false,
      ...more,
    };
    return { ...A.answer(ctx, method, args), ctx };
  };
  return { w, call, triggers, token };
}

const top = (w, channel) => w.messages.find((m) => m.channel === channel && !m.thread_ts);
const root = (w, channel) => w.messages.find((m) => m.channel === channel && W.replies(w, m).length > 0);

test("says who fctl's bot is, and what Acme and its people are", () => {
  const { call } = acme();
  const me = call("auth.test");
  assert.deepEqual(
    [me.ok, me.url, me.team, me.team_id, me.user_id, me.bot_id],
    [true, "https://acme.slack.com/", "Acme", "T0ACME", W.BOT_USER, W.BOT_ID],
  );
  const team = call("team.info", { team: "T0ACME" }).team;
  assert.deepEqual([team.name, team.domain], ["Acme", "acme"]);
  assert.ok(team.icon.image_88 && !team.icon.image_88.includes("default"));
  assert.equal(call("team.info", { team: "T0OTHER" }).error, "team_not_found");
  const ada = call("users.info", { user: "U0GUEST" }).user;
  assert.deepEqual(
    [ada.real_name, ada.is_restricted, ada.profile.email, ada.profile.display_name, ada.tz],
    ["Ada Guest", true, "ada@guest.test", "ada", "Europe/London"],
  );
  assert.ok(ada.profile.image_72.endsWith("/standin/avatar/U0GUEST.svg?s=72"));
  assert.equal(typeof ada.tz_offset, "number");
  const bot = call("users.info", { user: W.BOT_USER }).user;
  assert.deepEqual([bot.is_bot, bot.profile.bot_id, bot.profile.email], [true, W.BOT_ID, undefined]);
  assert.equal(call("users.info", { user: "U0NOBODY" }).error, "user_not_found");
  // Without users:read.email, no addresses; without team:read, no team.
  const narrow = acme({ scope: "users:read,chat:write" });
  assert.equal(narrow.call("users.info", { user: "U0DEV" }).user.profile.email, undefined);
  const missing = narrow.call("team.info");
  assert.deepEqual([missing.error, missing.needed], ["missing_scope", "team:read"]);
});

test("conversations: what fctl's bot may see of each kind, and the channels it lists, page by page", () => {
  const { w, call } = acme();
  const web = call("conversations.info", { channel: "C0WEB" }).channel;
  assert.deepEqual(
    [web.name, web.is_channel, web.is_private, web.is_ext_shared, web.is_member],
    ["web-team", true, false, false, false],
  );
  assert.equal(call("conversations.info", { channel: "G0SECRET" }).error, "channel_not_found");
  W.join(w, W.conversation(w, "G0SECRET"), W.BOT_USER);
  const secret = call("conversations.info", { channel: "G0SECRET" }).channel;
  assert.deepEqual(
    [secret.is_private, secret.is_group, secret.is_channel, secret.is_member],
    [true, true, false, true],
  );
  assert.equal(call("conversations.info", { channel: "C0PART" }).channel.is_ext_shared, true);
  const dm = call("conversations.info", { channel: "D0DEV" }).channel;
  assert.deepEqual([dm.is_im, dm.user], [true, "U0DEV"]);
  assert.equal(call("conversations.info", { channel: "C0NOPE" }).error, "channel_not_found");

  const names = [];
  let cursor = "";
  for (let i = 0; i < 10; i++) {
    const page = call("conversations.list", {
      types: "public_channel",
      exclude_archived: "true",
      limit: "2",
      cursor,
    });
    names.push(...page.channels.map((c) => c.name));
    cursor = page.response_metadata.next_cursor;
    if (!cursor) break;
  }
  assert.deepEqual(names, ["web-team", "incidents", "deploys", "random", "partners"]);
  assert.equal(call("conversations.list", { cursor: "bogus" }).error, "invalid_cursor");
  assert.equal(call("conversations.list", { limit: "1000" }).error, "invalid_limit");
  assert.deepEqual(
    call("conversations.list", { types: "private_channel" }).channels.map((c) => c.name),
    ["secret"],
  );
});

test("reads history and threads only where fctl's bot is, newest first and oldest first", () => {
  const { w, call } = acme();
  assert.equal(call("conversations.history", { channel: "C0DEP" }).error, "not_in_channel");
  const joined = call("conversations.join", { channel: "C0DEP" });
  assert.equal(joined.channel.is_member, true);
  assert.equal(call("conversations.join", { channel: "C0DEP" }).warning, "already_in_channel");
  assert.equal(
    call("conversations.join", { channel: "D0DEV" }).error,
    "method_not_supported_for_channel_type",
  );
  const history = call("conversations.history", { channel: "C0DEP", limit: "50" });
  assert.equal(history.messages[0].subtype, "channel_join", "the newest first: fctl's joining");
  const deploys = history.messages.filter((m) => m.bot_id === "B0DEPLOYS");
  assert.equal(deploys.length, 4);
  assert.equal(deploys[0].bot_profile.name, "Deploys");
  const since = history.messages.at(-2).ts;
  const newer = call("conversations.history", { channel: "C0DEP", oldest: since });
  assert.equal(newer.messages.length, history.messages.length - 2);
  const paged = call("conversations.history", { channel: "C0DEP", limit: "3" });
  assert.equal(paged.messages.length, 3);
  assert.equal(paged.has_more, true);
  const next = call("conversations.history", {
    channel: "C0DEP",
    limit: "3",
    cursor: paged.response_metadata.next_cursor,
  });
  assert.equal(next.messages[0].ts, history.messages[3].ts);

  W.join(w, W.conversation(w, "C0INC"), W.BOT_USER);
  const thread = root(w, "C0INC");
  const replies = call("conversations.replies", { channel: "C0INC", ts: thread.ts, limit: "50" });
  assert.equal(replies.messages.length, 3);
  assert.deepEqual(
    [replies.messages[0].ts, replies.messages[0].reply_count, replies.messages[0].thread_ts],
    [thread.ts, 2, thread.ts],
  );
  assert.deepEqual(replies.messages[0].reply_users, ["U0DEV", "U0GRACE"]);
  assert.deepEqual(
    [replies.messages[1].thread_ts, replies.messages[1].parent_user_id],
    [thread.ts, "U0GRACE"],
  );
  assert.equal(replies.has_more, false);
  // By any message of the thread, and a page at a time.
  const byReply = call("conversations.replies", { channel: "C0INC", ts: replies.messages[2].ts });
  assert.equal(byReply.messages[0].ts, thread.ts);
  const first = call("conversations.replies", { channel: "C0INC", ts: thread.ts, limit: "2" });
  assert.deepEqual([first.messages.length, first.has_more], [2, true]);
  const rest = call("conversations.replies", {
    channel: "C0INC",
    ts: thread.ts,
    limit: "2",
    cursor: first.response_metadata.next_cursor,
  });
  // Slack starts every page with the thread's first message.
  assert.deepEqual(
    [...first.messages, ...rest.messages].map((m) => m.ts),
    [thread.ts, replies.messages[1].ts, thread.ts, replies.messages[2].ts],
  );
  assert.equal(rest.has_more, false);
  assert.equal(call("conversations.replies", { channel: "C0INC", ts: "1.000001" }).error, "thread_not_found");
  // Slack's limit for apps outside the Marketplace: 15 messages a call.
  assert.equal(call("conversations.history", { channel: "C0DEP" }, { maxLimit: 2 }).messages.length, 2);
});

test("posts, changes and reacts where fctl's bot is, and tells it its own messages", () => {
  const { w, call } = acme();
  assert.equal(call("chat.postMessage", { channel: "C0WEB", text: "hi" }).error, "not_in_channel");
  assert.equal(call("chat.postMessage", { channel: "G0SECRET", text: "hi" }).error, "channel_not_found");
  W.join(w, W.conversation(w, "C0WEB"), W.BOT_USER);
  const flaky = top(w, "C0WEB");
  const posted = call("chat.postMessage", {
    channel: "C0WEB",
    thread_ts: flaky.ts,
    text: "card",
    blocks: CARD,
  });
  assert.equal(posted.ok, true);
  assert.deepEqual(
    [posted.message.bot_id, posted.message.app_id, posted.message.thread_ts],
    [W.BOT_ID, W.APP_ID, flaky.ts],
  );
  assert.equal(posted.message.blocks[2].type, "plan");
  assert.deepEqual(posted.ctx.events, [{ kind: "message", channel: "C0WEB", ts: posted.ts }]);
  // A form's blocks as a string, as a form body has them.
  const asked = call("chat.postMessage", {
    channel: "C0WEB",
    thread_ts: flaky.ts,
    text: "q",
    blocks: JSON.stringify(QUESTION),
  });
  assert.equal(W.message(w, "C0WEB", asked.ts).blocks[1].block_id, "fctl_q:ses_1:1700");
  // In someone's messages with the app, by their id too.
  assert.equal(call("chat.postMessage", { channel: "U0GRACE", text: "hi" }).channel, "D0GRACE");
  assert.equal(call("chat.postMessage", { channel: "C0WEB" }).error, "no_text");
  const bad = call("chat.postMessage", {
    channel: "C0WEB",
    blocks: [{ type: "header", text: plain("x".repeat(151)) }],
  });
  assert.equal(bad.error, "invalid_blocks");
  assert.deepEqual(bad.response_metadata.messages, [
    "[ERROR] must be less than 151 characters [json-pointer:/blocks/0/text/text]",
  ]);
  assert.equal(call("chat.postMessage", { channel: "C0WEB", blocks: "{" }).error, "invalid_blocks_format");
  assert.equal(
    call("chat.postMessage", { channel: "C0WEB", text: "a", markdown_text: "b" }).error,
    "markdown_text_conflict",
  );
  // A reply's ts for a thread: Slack wants its first message's.
  const odd = call("chat.postMessage", { channel: "C0WEB", thread_ts: posted.ts, text: "again" });
  assert.equal(odd.message.thread_ts, flaky.ts);
  assert.match(odd.ctx.problems[0], /is a reply's/);

  const changed = call("chat.update", {
    channel: "C0WEB",
    ts: posted.ts,
    text: "card 2",
    blocks: CARD.slice(0, 2),
  });
  assert.equal(changed.message.blocks.length, 2);
  assert.equal(changed.ctx.events[0].kind, "changed");
  assert.equal(changed.ctx.events[0].before.text, "card");
  // Text without blocks takes the blocks away.
  call("chat.update", { channel: "C0WEB", ts: posted.ts, text: "plain" });
  assert.equal(W.message(w, "C0WEB", posted.ts).blocks, undefined);
  assert.equal(
    call("chat.update", { channel: "C0WEB", ts: flaky.ts, text: "x" }).error,
    "cant_update_message",
  );
  assert.equal(
    call("chat.update", { channel: "C0WEB", ts: "1.000001", text: "x" }).error,
    "message_not_found",
  );

  assert.equal(call("reactions.add", { channel: "C0WEB", timestamp: flaky.ts, name: "eyes" }).ok, true);
  assert.equal(
    call("reactions.add", { channel: "C0WEB", timestamp: flaky.ts, name: "eyes" }).error,
    "already_reacted",
  );
  assert.equal(
    call("reactions.add", { channel: "C0WEB", timestamp: flaky.ts, name: ":eyes:" }).error,
    "invalid_name",
  );
  assert.deepEqual(flaky.reactions, [{ name: "eyes", users: [W.BOT_USER] }]);

  const link = call("chat.getPermalink", { channel: "C0WEB", message_ts: posted.ts }).permalink;
  const p = posted.ts.replace(".", "");
  assert.equal(link, `https://acme.slack.com/archives/C0WEB/p${p}?thread_ts=${flaky.ts}&cid=C0WEB`);

  const only = call("chat.postEphemeral", {
    channel: "C0WEB",
    user: "U0GRACE",
    thread_ts: flaky.ts,
    text: "choose",
    blocks: CHOOSE,
  });
  const e = w.ephemeral.find((x) => x.ts === only.message_ts);
  assert.deepEqual([e.to, e.thread_ts, e.blocks[2].block_id], ["U0GRACE", flaky.ts, "fctl_choose"]);
  assert.equal(
    call("chat.postEphemeral", { channel: "C0WEB", user: "U0NOBODY", text: "x" }).error,
    "user_not_found",
  );
  W.join(w, W.conversation(w, "C0INC"), W.BOT_USER);
  assert.equal(
    call("chat.postEphemeral", { channel: "C0INC", user: "U0GUEST", text: "x" }).error,
    "user_not_in_channel",
  );
  assert.equal(call("conversations.open", { users: "U0GRACE" }).channel.id, "D0GRACE");
  assert.equal(call("conversations.open", { users: W.BOT_USER }).error, "cannot_dm_bot");
  assert.equal(call("conversations.open", { users: "U0GRACE,U0DEV" }).error, "standin_unsupported");
});

test("opens a form once per trigger, within its seconds, and publishes the Home tab", () => {
  const { w, call, triggers } = acme();
  triggers.set("t1", { user: "U0DEV", at: Date.now(), used: false });
  assert.equal(call("views.open", { trigger_id: "nope", view: START_FORM }).error, "invalid_trigger_id");
  const long = call("views.open", {
    trigger_id: "t1",
    view: { ...START_FORM, title: plain("Start a session right now") },
  });
  assert.equal(long.error, "invalid_arguments");
  assert.deepEqual(long.response_metadata.messages, [
    "[ERROR] must be less than 25 characters [json-pointer:/view/title/text]",
  ]);
  const opened = call("views.open", { trigger_id: "t1", view: JSON.stringify(START_FORM) });
  assert.equal(opened.ok, true);
  assert.match(opened.view.id, /^V0/);
  assert.deepEqual(
    [opened.view.callback_id, opened.view.private_metadata, opened.view.type],
    ["fctl_start", '{"channel":null}', "modal"],
  );
  assert.equal(opened.view.user, undefined, "the stand-in's own fields stay its own");
  assert.equal(W.openModals(w, "U0DEV").length, 1);
  assert.equal(call("views.open", { trigger_id: "t1", view: START_FORM }).error, "exchanged_trigger_id");
  triggers.set("t2", { user: "U0DEV", at: Date.now() - 4000, used: false });
  assert.equal(call("views.open", { trigger_id: "t2", view: START_FORM }).error, "expired_trigger_id");
  triggers.set("t3", { user: "U0DEV", at: Date.now(), used: false });
  call("views.open", { trigger_id: "t3", view: START_FORM });
  assert.equal(W.openModals(w, "U0DEV").length, 1, "a new form takes the place of the one open");

  const home = call("views.publish", { user_id: "U0GRACE", view: HOME });
  assert.deepEqual([home.ok, home.view.type], [true, "home"]);
  assert.equal(W.home(w, "U0GRACE").id, home.view.id);
  call("views.publish", { user_id: "U0GRACE", view: HOME });
  assert.equal(w.views.filter((v) => v.type === "home").length, 1);
  assert.equal(call("views.publish", { user_id: "U0GRACE", view: START_FORM }).error, "invalid_arguments");
  assert.equal(call("views.publish", { user_id: W.BOT_USER, view: HOME }).error, "user_not_found");
});

test("sets a thread's agent session, where fctl's bot is, and streams into a thread of a DM", () => {
  const { w, call } = acme();
  const flaky = top(w, "C0WEB");
  const status = (args) =>
    call("agents.sessions.setStatus", { channel_id: "C0WEB", thread_ts: flaky.ts, ...args });
  assert.equal(status({ status: "processing" }).error, "not_authorized");
  W.join(w, W.conversation(w, "C0WEB"), W.BOT_USER);
  assert.equal(status({ status: "working" }).error, "invalid_status");
  assert.equal(
    call("agents.sessions.setStatus", { channel_id: "C0WEB", status: "active" }).error,
    "thread_ts_required",
  );
  const set = status({
    status: "processing",
    title: "Fix the flaky login test",
    initiator_user_id: "U0GRACE",
  });
  assert.deepEqual(
    [set.ok, set.status, set.agent_status, set.title],
    [true, "processing", "processing", "Fix the flaky login test"],
  );
  status({ status: "suspended", title: "Another title" });
  const s = W.agentSession(w, "C0WEB", flaky.ts);
  assert.deepEqual([s.status, s.title, s.initiator], ["suspended", "Fix the flaky login test", "U0GRACE"]);
  assert.deepEqual(
    s.history.map((h) => h.status),
    ["processing", "suspended"],
  );

  const ask = W.addMessage(w, { channel: "D0DEV", user: "U0DEV", text: "fix the tests" });
  const start = call("chat.startStream", { channel: "D0DEV", thread_ts: ask.ts, markdown_text: "Looking" });
  assert.equal(start.ok, true);
  assert.equal(W.agentSession(w, "D0DEV", ask.ts).initiator, "U0DEV");
  assert.equal(
    call("chat.appendStream", { channel: "D0DEV", ts: start.ts, markdown_text: " at it" }).ok,
    true,
  );
  assert.equal(W.message(w, "D0DEV", start.ts).text, "Looking at it");
  const big = call("chat.appendStream", {
    channel: "D0DEV",
    ts: start.ts,
    markdown_text: "x".repeat(12_000),
  });
  assert.equal(big.error, "msg_too_long");
  const stop = call("chat.stopStream", { channel: "D0DEV", ts: start.ts, session_status: "active" });
  assert.equal(stop.ok, true);
  assert.equal(W.agentSession(w, "D0DEV", ask.ts).status, "active");
  assert.equal(
    call("chat.appendStream", { channel: "D0DEV", ts: start.ts, markdown_text: "more" }).error,
    "message_not_in_streaming_state",
  );
  // A stream that someone stopped, or that ran past Slack's ten minutes.
  const again = call("chat.startStream", { channel: "D0DEV", thread_ts: ask.ts, markdown_text: "Again" });
  W.message(w, "D0DEV", again.ts).stream.state = "stopped_by_user";
  assert.equal(
    call("chat.appendStream", { channel: "D0DEV", ts: again.ts, markdown_text: "x" }).error,
    "stopped_by_user",
  );
  const old = call("chat.startStream", { channel: "D0DEV", thread_ts: ask.ts, markdown_text: "Old" });
  W.message(w, "D0DEV", old.ts).stream.started_at -= 11 * 60 * 1000;
  assert.equal(
    call("chat.appendStream", { channel: "D0DEV", ts: old.ts, markdown_text: "x" }).error,
    "message_not_in_streaming_state",
  );
  assert.equal(W.message(w, "D0DEV", old.ts).stream.state, "timed_out");
  // In a channel, a stream names whom it is for.
  const inChannel = call("chat.startStream", { channel: "C0WEB", thread_ts: flaky.ts, markdown_text: "x" });
  assert.equal(inChannel.error, "invalid_arguments");
  assert.equal(call("chat.startStream", { channel: "D0DEV", markdown_text: "x" }).error, "invalid_thread_ts");
});

test("searches public channels with an event's action token, and says what it doesn't answer", () => {
  const { w, call } = acme();
  w.actionTokens.tok = { user: "U0DEV", channel: "C0WEB", ts: "1.1", at: new Date().toISOString() };
  const search = (args) => call("assistant.search.context", { query: "flaky login", ...args });
  assert.equal(search({}).error, "invalid_arguments");
  assert.equal(search({ action_token: "nope" }).error, "invalid_action_token");
  assert.equal(search({ action_token: "tok", limit: 21 }).error, "invalid_limit");
  const found = search({
    action_token: "tok",
    channel_types: "public_channel",
    content_types: "messages",
    limit: 20,
  });
  assert.equal(found.results.messages.length, 1);
  const m = found.results.messages[0];
  assert.deepEqual([m.channel_name, m.author_name, m.is_author_bot], ["web-team", "Grace Hopper", false]);
  assert.match(m.permalink, /^https:\/\/acme\.slack\.com\/archives\/C0WEB\/p\d+$/);
  // Private channels stay out, and so do other apps' messages unless asked for.
  assert.equal(search({ action_token: "tok", query: "draft plan" }).results.messages.length, 0);
  assert.equal(search({ action_token: "tok", query: "migrations step" }).results.messages.length, 1);
  assert.equal(
    search({ action_token: "tok", query: "migrations step", include_bots: true }).results.messages.length,
    2,
  );
  const unknown = call("chat.scheduleMessage", {});
  assert.equal(unknown.error, "unknown_method");
  assert.match(unknown.ctx.problems[0], /doesn't answer yet/);
});

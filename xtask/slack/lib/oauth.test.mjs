import assert from "node:assert/strict";
import { createPublicKey, verify } from "node:crypto";
import { test } from "node:test";
import * as O from "./oauth.mjs";
import * as W from "./world.mjs";

const config = { clientId: "fctl-dev-slack", clientSecret: "secret" };
const installed = "http://localhost:8787/api/v1/slack/installed";
const linked = "http://localhost:8787/api/v1/slack/linked";
/** fctl_domain::slack::BOT_SCOPES, as fctl asks for them. */
const FCTL_SCOPES =
  "app_mentions:read,assistant:write,channels:history,channels:join,channels:read,chat:write,commands,groups:history,groups:read,im:history,im:read,im:write,reactions:write,search:read.public,team:read,users:read,users:read.email";

const form = (fields) =>
  new URLSearchParams({ client_id: config.clientId, client_secret: config.clientSecret, ...fields });

/** The install page as fctl sends an admin there (fctl_domain::slack::authorize_url), allowed at once. */
function install(w, codes, more = {}) {
  const params = new URLSearchParams({
    client_id: config.clientId,
    scope: FCTL_SCOPES,
    redirect_uri: installed,
    state: "the-state",
    auto: "1",
    ...more,
  });
  return O.authorize(w, config, codes, "install", params);
}

const codeOf = (location) => new URL(location).searchParams.get("code");

test("installs fctl's app: the page sends the browser back with a code, which buys the bot token", () => {
  const w = W.seed();
  const codes = new Map();
  const page = install(w, codes);
  const back = new URL(page.location);
  assert.equal(back.origin + back.pathname, installed);
  assert.equal(back.searchParams.get("state"), "the-state");
  const r = O.access(w, config, codes, form({ code: codeOf(page.location), redirect_uri: installed }));
  assert.equal(r.body.ok, true);
  assert.match(r.body.access_token, /^xoxb-\d+-\d+-[\w-]+$/);
  assert.deepEqual(
    {
      token_type: r.body.token_type,
      bot_user_id: r.body.bot_user_id,
      app_id: r.body.app_id,
      team: r.body.team,
      enterprise: r.body.enterprise,
      is_enterprise_install: r.body.is_enterprise_install,
      authed_user: r.body.authed_user,
    },
    {
      token_type: "bot",
      bot_user_id: W.BOT_USER,
      app_id: W.APP_ID,
      team: { id: "T0ACME", name: "Acme" },
      enterprise: null,
      is_enterprise_install: false,
      authed_user: { id: W.DEV_USER },
    },
  );
  assert.equal(r.body.scope, FCTL_SCOPES);
  assert.ok(O.installed(w));
  const who = O.actorOf(w, `Bearer ${r.body.access_token}`);
  assert.deepEqual([who.record.kind, who.record.user], ["bot", W.BOT_USER]);
  // A code works once.
  assert.equal(
    O.access(w, config, codes, form({ code: codeOf(page.location), redirect_uri: installed })).body.error,
    "invalid_code",
  );
  // Installing again keeps the token, with what it adds; Grace may install it too.
  const again = O.access(
    w,
    config,
    codes,
    form({
      code: codeOf(install(w, codes, { scope: "chat:write.public", user: "grace" }).location),
      redirect_uri: installed,
    }),
  );
  assert.equal(again.body.access_token, r.body.access_token);
  assert.ok(again.body.scope.endsWith(",chat:write.public"));
  assert.deepEqual(again.body.authed_user, { id: W.GRACE });
});

test("refuses what Slack refuses: another client, a wrong secret or redirect, unknown scopes, JSON", () => {
  const w = W.seed();
  const codes = new Map();
  assert.match(
    O.authorize(w, config, codes, "install", new URLSearchParams({ client_id: "other" })).problem,
    /invalid_client_id/,
  );
  const scoped = install(w, codes, { scope: "chat:write,chat:everything" });
  assert.equal(new URL(scoped.location).searchParams.get("error"), "invalid_scope");
  assert.equal(new URL(scoped.location).searchParams.get("state"), "the-state");
  // Sign in with Slack's scopes go in a flow of their own.
  const mixed = install(w, codes, { user_scope: "openid" });
  assert.equal(new URL(mixed.location).searchParams.get("error"), "invalid_scope");
  // A guest can't install apps.
  assert.match(install(w, codes, { user: "ada" }).problem, /guest/);
  const denied = O.authorize(
    w,
    config,
    codes,
    "install",
    new URLSearchParams({
      client_id: config.clientId,
      scope: "chat:write",
      redirect_uri: installed,
      state: "s",
    }),
    "deny",
  );
  assert.equal(new URL(denied.location).searchParams.get("error"), "access_denied");
  // Without auto=1 or a decision, the page asks first.
  const asks = O.authorize(
    w,
    config,
    codes,
    "install",
    new URLSearchParams({ client_id: config.clientId, scope: "chat:write", redirect_uri: installed }),
  );
  assert.deepEqual(asks.consent.scopes, ["chat:write"]);
  const code = codeOf(install(w, codes).location);
  assert.equal(
    O.access(w, config, codes, form({ code, client_secret: "nope" })).body.error,
    "bad_client_secret",
  );
  const other = codeOf(install(w, codes).location);
  assert.equal(
    O.access(w, config, codes, form({ code: other, redirect_uri: `${installed}/elsewhere` })).body.error,
    "bad_redirect_uri",
  );
  // HTTP Basic works as well as the form.
  const basic = `Basic ${Buffer.from(`${config.clientId}:${config.clientSecret}`).toString("base64")}`;
  const third = codeOf(install(w, codes).location);
  assert.equal(
    O.access(w, config, codes, new URLSearchParams({ code: third, redirect_uri: installed }), basic).body.ok,
    true,
  );
  // Arguments in a JSON body don't count for oauth.v2.access: nothing names the client.
  assert.equal(O.access(w, config, codes, new URLSearchParams()).body.error, "invalid_client_id");
});

test("signs someone in with Slack: an RS256 ID token, with the nonce, that the stand-in's key verifies", () => {
  const w = W.seed();
  const codes = new Map();
  const key = O.signingKey();
  const params = new URLSearchParams({
    response_type: "code",
    scope: "openid profile email",
    client_id: config.clientId,
    redirect_uri: linked,
    state: "st",
    nonce: "n-1",
    team: "T0ACME",
    user: "U0GRACE",
    auto: "1",
  });
  const page = O.authorize(w, config, codes, "sign-in", params);
  const back = new URL(page.location);
  assert.equal(back.searchParams.get("state"), "st");
  const r = O.oidcToken(
    w,
    config,
    codes,
    key,
    form({ code: back.searchParams.get("code"), redirect_uri: linked, grant_type: "authorization_code" }),
  );
  assert.equal(r.body.ok, true);
  assert.equal(r.body.token_type, "Bearer");
  assert.match(r.body.access_token, /^xoxp-/);
  const [head, claims, sig] = r.body.id_token.split(".");
  assert.deepEqual(JSON.parse(Buffer.from(head, "base64url")), { alg: "RS256", typ: "JWT", kid: key.kid });
  const ok = verify(
    "sha256",
    Buffer.from(`${head}.${claims}`),
    createPublicKey({ key: key.jwk, format: "jwk" }),
    Buffer.from(sig, "base64url"),
  );
  assert.ok(ok, "the ID token verifies with the key /openid/connect/keys serves");
  const c = JSON.parse(Buffer.from(claims, "base64url"));
  assert.deepEqual(
    [c.iss, c.aud, c.sub, c["https://slack.com/user_id"], c["https://slack.com/team_id"], c.nonce],
    ["https://slack.com", config.clientId, "U0GRACE", "U0GRACE", "T0ACME", "n-1"],
  );
  assert.deepEqual(
    [c.name, c.email, c.email_verified, c["https://slack.com/team_name"], c["https://slack.com/team_domain"]],
    ["Grace Hopper", "grace@acme.test", true, "Acme", "acme"],
  );
  assert.equal(c.exp - c.iat, 300);
  // userInfo, with the person's token, says the same.
  const info = O.userInfo(w, O.actorOf(w, `Bearer ${r.body.access_token}`).record);
  assert.deepEqual(
    [info.ok, info.sub, info.email, info["https://slack.com/team_domain"]],
    [true, "U0GRACE", "grace@acme.test", "acme"],
  );
  // fctl keeps no token of the person's: it revokes it.
  assert.deepEqual(O.revoke(w, r.body.access_token), { oauth: ["U0GRACE"], bot: [] });
  assert.equal(O.actorOf(w, `Bearer ${r.body.access_token}`).error, "token_revoked");
  // A sign-in's code doesn't install the app.
  const another = codeOf(O.authorize(w, config, codes, "sign-in", params).location);
  assert.equal(
    O.access(w, config, codes, form({ code: another, redirect_uri: linked })).body.error,
    "invalid_code",
  );
  const noOpenid = O.authorize(
    w,
    config,
    codes,
    "sign-in",
    new URLSearchParams({ ...Object.fromEntries(params), scope: "profile" }),
  );
  assert.equal(new URL(noOpenid.location).searchParams.get("error"), "invalid_scope");
});

test("tokens: none is not_authed, an unknown one invalid_auth, and an uninstall revokes the bot's", () => {
  const w = W.seed();
  const codes = new Map();
  const token = O.access(
    w,
    config,
    codes,
    form({ code: codeOf(install(w, codes).location), redirect_uri: installed }),
  ).body.access_token;
  assert.equal(O.actorOf(w, undefined).error, "not_authed");
  assert.equal(O.actorOf(w, "Bearer xoxb-nope").error, "invalid_auth");
  assert.equal(O.actorOf(w, undefined, token).token, token);
  W.join(w, W.conversation(w, "C0WEB"), W.BOT_USER);
  const r = O.appsUninstall(w, config, form({}), `Bearer ${token}`);
  assert.deepEqual(r.body, { ok: true });
  assert.deepEqual(r.gone, { oauth: [], bot: [W.BOT_USER] });
  assert.equal(O.installed(w), false);
  assert.equal(O.actorOf(w, `Bearer ${token}`).error, "token_revoked");
  // Slack takes fctl's bot out of its channels.
  assert.equal(W.isMember(W.conversation(w, "C0WEB"), W.BOT_USER), false);
  // Installed again: a new token.
  const fresh = O.access(
    w,
    config,
    codes,
    form({ code: codeOf(install(w, codes).location), redirect_uri: installed }),
  ).body;
  assert.notEqual(fresh.access_token, token);
  assert.equal(
    O.appsUninstall(w, config, form({ client_secret: "no" }), `Bearer ${fresh.access_token}`).body.error,
    "bad_client_secret",
  );
});

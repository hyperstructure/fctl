// Slack's sign-ins, as fctl goes through them (docs.slack.dev/authentication):
// the OAuth v2 install of fctl's app (`/oauth/v2/authorize`, then
// `oauth.v2.access` for the bot token), Sign in with Slack (OpenID Connect:
// `/openid/connect/authorize`, `openid.connect.token` with an RS256 ID token
// whose key `/openid/connect/keys` serves, and `openid.connect.userInfo`),
// `auth.revoke` and `apps.uninstall`. Tokens are kept in the world, so that
// the control plane's bot token outlives a restart.

import { generateKeyPairSync, randomBytes, randomUUID, sign } from "node:crypto";
import * as W from "./world.mjs";

/** Slack's bot scopes (docs.slack.dev/reference/scopes), which the install page takes. */
export const BOT_SCOPES = new Set([
  "app_mentions:read",
  "assistant:write",
  "bookmarks:read",
  "bookmarks:write",
  "calls:read",
  "calls:write",
  "canvases:read",
  "canvases:write",
  "channels:history",
  "channels:join",
  "channels:manage",
  "channels:read",
  "channels:write.invites",
  "channels:write.topic",
  "chat:write",
  "chat:write.customize",
  "chat:write.public",
  "code_channels:manage",
  "commands",
  "conversations.connect:manage",
  "conversations.connect:read",
  "conversations.connect:write",
  "dnd:read",
  "emoji:read",
  "files:read",
  "files:write",
  "groups:history",
  "groups:read",
  "groups:write",
  "groups:write.invites",
  "groups:write.topic",
  "im:history",
  "im:read",
  "im:write",
  "im:write.topic",
  "incoming-webhook",
  "links.embed:write",
  "links:read",
  "links:write",
  "lists:read",
  "lists:write",
  "mcp:connect",
  "metadata.message:read",
  "mpim:history",
  "mpim:read",
  "mpim:write",
  "mpim:write.topic",
  "pins:read",
  "pins:write",
  "reactions:read",
  "reactions:write",
  "reminders:read",
  "reminders:write",
  "remote_files:read",
  "remote_files:share",
  "remote_files:write",
  "search:read.files",
  "search:read.public",
  "search:read.users",
  "team.billing:read",
  "team.preferences:read",
  "team:read",
  "triggers:read",
  "triggers:write",
  "usergroups:read",
  "usergroups:write",
  "users.profile:read",
  "users:read",
  "users:read.email",
  "users:write",
  "workflow.steps:execute",
]);

/** Sign in with Slack's scopes, which go in a flow of their own. */
export const OIDC_SCOPES = new Set(["openid", "profile", "email"]);

/** Slack's user scopes, besides Sign in with Slack's. */
const USER_SCOPES = new Set([
  ...BOT_SCOPES,
  "channels:write",
  "dnd:write",
  "identify",
  "identity.avatar",
  "identity.basic",
  "identity.email",
  "identity.team",
  "search:read",
  "search:read.im",
  "search:read.mpim",
  "search:read.private",
  "stars:read",
  "stars:write",
  "users.profile:write",
]);

/** How long an authorization code works: ten minutes. */
const CODE_TTL_MS = 10 * 60 * 1000;

/** How long an ID token says it works: Slack's examples give five minutes. */
const ID_TOKEN_TTL_S = 300;

const digits = (n) => [...randomBytes(n)].map((b) => b % 10).join("");
const token = (prefix) => `${prefix}-${digits(11)}-${digits(13)}-${randomBytes(18).toString("base64url")}`;

/** The key the stand-in signs ID tokens with, and its public half as a JWK. */
export function signingKey() {
  const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const kid = randomUUID();
  return { privateKey, kid, jwk: { ...publicKey.export({ format: "jwk" }), kid, alg: "RS256", use: "sig" } };
}

/** Slack's OpenID Connect discovery document, with the stand-in's addresses. */
export function discovery(base) {
  return {
    issuer: "https://slack.com",
    authorization_endpoint: `${base}/openid/connect/authorize`,
    token_endpoint: `${base}/api/openid.connect.token`,
    userinfo_endpoint: `${base}/api/openid.connect.userInfo`,
    jwks_uri: `${base}/openid/connect/keys`,
    scopes_supported: ["openid", "profile", "email"],
    response_types_supported: ["code"],
    response_modes_supported: ["query"],
    grant_types_supported: ["authorization_code"],
    subject_types_supported: ["public"],
    id_token_signing_alg_values_supported: ["RS256"],
    claims_supported: ["sub", "auth_time", "iss"],
    claims_parameter_supported: false,
    request_parameter_supported: false,
    request_uri_parameter_supported: true,
    token_endpoint_auth_methods_supported: ["client_secret_post", "client_secret_basic"],
  };
}

const fail = (error, more = {}) => ({ ok: false, error, ...more });

// ───────────────────────────── the authorize pages ─────────────────────────────

/**
 * What `/oauth/v2/authorize` or `/openid/connect/authorize` makes of a
 * request (`kind`: `install` or `sign-in`): `{ location }` to send the
 * browser back with a code or an error, `{ consent }` to ask the person
 * first, or `{ problem }` to show on the page, where Slack can't send the
 * browser back at all. `decision` is the page's answer (`allow`, `deny`);
 * `auto=1` allows at once, as `cargo xtask` asks.
 */
export function authorize(w, config, codes, kind, params, decision = null) {
  const get = (k) => params.get(k) ?? "";
  if (get("client_id") !== config.clientId) {
    return { problem: `Slack doesn't know the app ${JSON.stringify(get("client_id"))}: invalid_client_id` };
  }
  let back;
  try {
    back = new URL(get("redirect_uri"));
  } catch {
    return { problem: "The redirect_uri must be an absolute URL: bad_redirect_uri" };
  }
  if (!["http:", "https:"].includes(back.protocol) || back.hash) {
    return { problem: "The redirect_uri must be an http(s) URL without a fragment: bad_redirect_uri" };
  }
  const answer = (pairs, problem) => {
    for (const [k, v] of pairs) back.searchParams.set(k, v);
    if (params.has("state")) back.searchParams.set("state", get("state"));
    return { location: back.toString(), ...(problem ? { problem } : {}) };
  };
  const refuse = (error) => answer([["error", error]], `refused the ${kind}: ${error}`);
  let grant;
  if (kind === "install") {
    const scopes = get("scope")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    const userScopes = get("user_scope")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    if (scopes.length + userScopes.length === 0) return refuse("invalid_scope");
    if (scopes.some((s) => !BOT_SCOPES.has(s))) return refuse("invalid_scope");
    // Sign in with Slack's scopes go in a flow of their own.
    if (userScopes.some((s) => !USER_SCOPES.has(s) || OIDC_SCOPES.has(s))) return refuse("invalid_scope");
    grant = { kind, scopes, userScopes };
  } else {
    if (get("response_type") !== "code") return refuse("unsupported_response_type");
    const scopes = get("scope").split(/[ +]/).filter(Boolean);
    if (!scopes.includes("openid") || scopes.some((s) => !OIDC_SCOPES.has(s))) return refuse("invalid_scope");
    grant = { kind, scopes, nonce: params.has("nonce") ? get("nonce") : null };
  }
  const person = W.personByName(w, get("user") || W.DEV_USER);
  const team = get("team");
  const consent = {
    kind,
    scopes: [...grant.scopes, ...(grant.userScopes ?? [])],
    // Slack drops a hint of another workspace's, as it may: there's only Acme.
    team: team === w.team.id ? team : null,
    person: person?.id ?? null,
  };
  if (decision === "deny") return refuse("access_denied");
  if (decision !== "allow" && get("auto") !== "1") return { consent };
  if (!person) return { consent, problem: `Acme has nobody called ${JSON.stringify(get("user"))}` };
  if (kind === "install" && person.is_restricted) {
    return { consent, problem: `${person.real_name} is a guest of Acme: guests can't install apps there` };
  }
  const code = `${digits(10)}.${digits(13)}.${randomBytes(16).toString("hex")}`;
  codes.set(code, {
    ...grant,
    user: person.id,
    clientId: config.clientId,
    redirectUri: params.has("redirect_uri") ? get("redirect_uri") : null,
    expiresAt: Date.now() + CODE_TTL_MS,
  });
  return answer([["code", code]]);
}

// ───────────────────────────── the token endpoints ─────────────────────────────

/** The client's id and secret, from HTTP Basic or the form, and whether they are fctl's app's. */
function client(config, form, authorization) {
  let id = form.get("client_id");
  let secret = form.get("client_secret");
  if (/^Basic /i.test(authorization ?? "")) {
    const [i, ...s] = Buffer.from(authorization.slice(6).trim(), "base64").toString().split(":");
    id = decodeURIComponent(i);
    secret = decodeURIComponent(s.join(":"));
  }
  if (!id || id !== config.clientId) return fail("invalid_client_id");
  if (secret !== config.clientSecret) return fail("bad_client_secret");
  return null;
}

/** Takes a code once: the grant it stands for, or Slack's refusal. */
function redeem(codes, form, kind) {
  const code = form.get("code") ?? "";
  const grant = codes.get(code);
  codes.delete(code);
  if (!grant || grant.kind !== kind || grant.expiresAt < Date.now()) return { error: fail("invalid_code") };
  // Sent to authorize, the same redirect_uri must come here.
  if (grant.redirectUri !== null && form.get("redirect_uri") !== grant.redirectUri) {
    return { error: fail("bad_redirect_uri") };
  }
  return { grant };
}

/**
 * `oauth.v2.access`: an install's code for the bot's token. Slack keeps one
 * bot token for the app in Acme: installing again keeps it, with the
 * scopes added; after an uninstall, a new one.
 */
export function access(w, config, codes, form, authorization) {
  const refused = client(config, form, authorization);
  if (refused) return { body: refused };
  const grantType = form.get("grant_type");
  if (grantType === "refresh_token") return { body: fail("invalid_refresh_token") };
  if (grantType && grantType !== "authorization_code") return { body: fail("invalid_grant_type") };
  const { grant, error } = redeem(codes, form, "install");
  if (error) return { body: error };
  const now = new Date().toISOString();
  let bot = w.installation && w.tokens[w.installation.token];
  if (!bot || bot.revoked) {
    const t = token("xoxb");
    bot = { kind: "bot", user: W.BOT_USER, scopes: [], revoked: false, created: now };
    w.tokens[t] = bot;
    w.installation = { token: t, installer: grant.user, installed_at: now };
  }
  bot.scopes = [...new Set([...bot.scopes, ...grant.scopes])];
  w.installation.installer = grant.user;
  w.installation.updated_at = now;
  const authed = { id: grant.user };
  if (grant.userScopes.length) {
    const t = token("xoxp");
    w.tokens[t] = { kind: "user", user: grant.user, scopes: grant.userScopes, revoked: false, created: now };
    Object.assign(authed, { scope: grant.userScopes.join(","), access_token: t, token_type: "user" });
  }
  return {
    changed: true,
    installer: grant.user,
    body: {
      ok: true,
      app_id: W.APP_ID,
      authed_user: authed,
      scope: bot.scopes.join(","),
      token_type: "bot",
      access_token: w.installation.token,
      bot_user_id: W.BOT_USER,
      team: { id: w.team.id, name: w.team.name },
      enterprise: null,
      is_enterprise_install: false,
    },
  };
}

/** What Sign in with Slack says of someone, as its ID token and userInfo give it, by its scopes. */
export function claims(w, u, scopes) {
  const c = { sub: u.id, "https://slack.com/user_id": u.id, "https://slack.com/team_id": w.team.id };
  if (scopes.includes("email") && u.email) {
    Object.assign(c, {
      email: u.email,
      email_verified: true,
      date_email_verified: Math.floor(Date.parse("2025-03-01T09:00:00Z") / 1000),
    });
  }
  if (scopes.includes("profile")) {
    Object.assign(c, {
      name: u.real_name,
      picture: W.avatar(u.id, 512),
      given_name: u.first_name,
      family_name: u.last_name,
      locale: "en-US",
    });
    for (const size of [24, 32, 48, 72, 192, 512, 1024]) {
      c[`https://slack.com/user_image_${size}`] = W.avatar(u.id, size);
    }
  }
  Object.assign(c, {
    "https://slack.com/team_name": w.team.name,
    "https://slack.com/team_domain": w.team.domain,
    "https://slack.com/team_image_default": false,
  });
  for (const size of [34, 44, 68, 88, 102, 132, 230]) {
    c[`https://slack.com/team_image_${size}`] = W.avatar(w.team.id, size);
  }
  return c;
}

const b64url = (v) => Buffer.from(typeof v === "string" ? v : JSON.stringify(v)).toString("base64url");

/** An RS256 JWT of `payload`, signed with the stand-in's key. */
export function jwt(key, payload) {
  const head = `${b64url({ alg: "RS256", typ: "JWT", kid: key.kid })}.${b64url(payload)}`;
  return `${head}.${sign("sha256", Buffer.from(head), key.privateKey).toString("base64url")}`;
}

/** `openid.connect.token`: a Sign in with Slack's code for the person's token and an ID token. */
export function oidcToken(w, config, codes, key, form, authorization) {
  const refused = client(config, form, authorization);
  if (refused) return { body: refused };
  const grantType = form.get("grant_type");
  if (grantType === "refresh_token") return { body: fail("invalid_refresh_token") };
  if (grantType && grantType !== "authorization_code") return { body: fail("invalid_grant_type") };
  const { grant, error } = redeem(codes, form, "sign-in");
  if (error) return { body: error };
  const u = W.user(w, grant.user);
  const t = token("xoxp");
  const now = Math.floor(Date.now() / 1000);
  w.tokens[t] = {
    kind: "user",
    user: u.id,
    scopes: grant.scopes,
    revoked: false,
    created: new Date().toISOString(),
    signIn: true,
  };
  const payload = {
    iss: "https://slack.com",
    aud: config.clientId,
    exp: now + ID_TOKEN_TTL_S,
    iat: now,
    auth_time: now,
    ...(grant.nonce !== null ? { nonce: grant.nonce } : {}),
    at_hash: randomBytes(16).toString("base64url"),
    ...claims(w, u, grant.scopes),
  };
  return {
    changed: true,
    user: u.id,
    body: { ok: true, access_token: t, token_type: "Bearer", id_token: jwt(key, payload) },
  };
}

/** `openid.connect.userInfo`, for a Sign in with Slack's token. */
export function userInfo(w, record) {
  if (record.kind !== "user" || !record.scopes.includes("openid")) return fail("not_allowed_token_type");
  return { ok: true, ...claims(w, W.user(w, record.user), record.scopes) };
}

// ───────────────────────────── tokens ─────────────────────────────

/**
 * Who a request's token is: `{ token, record }`, or Slack's error for it
 * (`not_authed` without one, `invalid_auth` for one Slack never gave,
 * `token_revoked` for one it took back).
 */
export function actorOf(w, authorization, formToken) {
  const bearer = /^Bearer\s+(.+)$/i.exec(authorization ?? "")?.[1]?.trim() || formToken;
  if (!bearer) return { error: "not_authed" };
  const record = w.tokens[bearer];
  if (!record) return { error: "invalid_auth" };
  if (record.revoked) return { error: "token_revoked" };
  return { token: bearer, record };
}

/** Whether fctl's app is installed in Acme: its bot token works. */
export const installed = (w) => Boolean(w.installation && w.tokens[w.installation.token]?.revoked === false);

/**
 * Takes a token back (`auth.revoke`). The bot's takes fctl's bot out of
 * every channel, as Slack does; the app is uninstalled once no token of
 * it works. Whose tokens went, for `tokens_revoked`.
 */
export function revoke(w, token) {
  const record = w.tokens[token];
  if (!record || record.revoked) return { oauth: [], bot: [] };
  record.revoked = true;
  if (record.kind === "bot") {
    for (const c of w.conversations) {
      if (!c.is_im) c.members = c.members.filter((m) => m !== W.BOT_USER);
    }
    return { oauth: [], bot: [record.user] };
  }
  return { oauth: [record.user], bot: [] };
}

/** Uninstalls fctl's app from Acme: every token of it goes. Whose tokens went. */
export function uninstall(w) {
  const gone = { oauth: [], bot: [] };
  for (const [t, record] of Object.entries(w.tokens)) {
    if (record.revoked) continue;
    const g = revoke(w, t);
    gone.oauth.push(...g.oauth);
    gone.bot.push(...g.bot);
  }
  gone.oauth = [...new Set(gone.oauth)];
  w.installation = null;
  return gone;
}

/** `apps.uninstall`: the app's own uninstall, with its client's id and secret and a token of it. */
export function appsUninstall(w, config, form, authorization) {
  const refused = client(config, form, null);
  if (refused) return { body: refused };
  const who = actorOf(w, authorization, form.get("token"));
  if (who.error) return { body: fail(who.error) };
  return { body: { ok: true }, gone: uninstall(w), changed: true };
}

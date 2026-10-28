// Linear's OAuth sign-in, as fctl goes through it: the authorize page,
// which consents at once (as the app's installer, with actor=app, or as
// the person signing in), the token endpoint, with PKCE and refresh tokens
// that change at each use, and revocation. Tokens are kept in the world, so
// that the control plane's outlive a restart.

import { createHash, randomBytes } from "node:crypto";
import { APP_USER, DEV_USER } from "./world.mjs";

/** What fctl's app may ask for (linear.app/developers/oauth-2-0-authentication). */
const SCOPES = [
  "read",
  "write",
  "issues:create",
  "comments:create",
  "timeSchedule:write",
  "admin",
  "app:assignable",
  "app:mentionable",
];

/** How long Linear's access tokens last, in seconds: a day. */
export const ACCESS_TTL = 86_399;

/** How long a refresh token Linear already used answers the same, as fctl_control::linear says. */
const REUSE_MS = 30 * 60 * 1000;

const CODE_TTL_MS = 10 * 60 * 1000;

const token = (kind) => `lin_${kind}_${randomBytes(24).toString("base64url")}`;

/** A refusal, as OAuth words them: its status and body. */
const refuse = (status, error, description) => ({
  status,
  body: { error, error_description: description },
});

/**
 * `GET /oauth/authorize`: where the browser goes back to, with a code, or
 * why it can't (`problem`, shown on the page, as Linear shows it).
 */
export function authorize(config, codes, params) {
  const get = (k) => params.get(k) ?? "";
  if (get("client_id") !== config.clientId) return { problem: "That client_id isn't fctl's app's" };
  let back;
  try {
    back = new URL(get("redirect_uri"));
  } catch {
    return { problem: "The redirect_uri must be an absolute URL" };
  }
  const fail = (error) => {
    back.searchParams.set("error", error);
    if (params.has("state")) back.searchParams.set("state", get("state"));
    return { location: back.toString(), problem: `refused the sign-in: ${error}` };
  };
  if (get("response_type") !== "code") return fail("unsupported_response_type");
  const scopes = get("scope").split(/[ ,]/).filter(Boolean);
  const app = get("actor") === "app";
  if (scopes.length === 0 || scopes.some((s) => !SCOPES.includes(s))) return fail("invalid_scope");
  // Agents are installed as the app: delegating and mentioning need actor=app.
  if (!app && scopes.some((s) => s.startsWith("app:"))) return fail("invalid_scope");
  const challenge = get("code_challenge");
  const method = get("code_challenge_method") || (challenge ? "plain" : "");
  if (challenge && method !== "S256" && method !== "plain") return fail("invalid_request");
  const code = randomBytes(16).toString("hex");
  codes.set(code, {
    userId: app ? APP_USER : DEV_USER,
    app,
    scopes,
    challenge,
    method,
    redirectUri: get("redirect_uri"),
    expiresAt: Date.now() + CODE_TTL_MS,
  });
  back.searchParams.set("code", code);
  if (params.has("state")) back.searchParams.set("state", get("state"));
  return { location: back.toString() };
}

/**
 * Issues an access token and its refresh token, as Linear answers them.
 * Linear keeps one installation for an app's user: installing it again
 * (`installed`) ends the tokens of the one before.
 */
function issue(w, grant, installed = false) {
  if (installed && grant.app) {
    for (const t of Object.values(w.tokens)) {
      if (t.app) t.revoked = true;
    }
  }
  const access = token("oauth");
  const refresh = token("refresh");
  const kept = { userId: grant.userId, app: grant.app, scopes: grant.scopes, revoked: false };
  w.tokens[access] = { ...kept, kind: "access", expiresAt: Date.now() + ACCESS_TTL * 1000, pair: refresh };
  w.tokens[refresh] = { ...kept, kind: "refresh", pair: access };
  return {
    access_token: access,
    token_type: "Bearer",
    expires_in: ACCESS_TTL,
    scope: grant.scopes.join(" "),
    refresh_token: refresh,
  };
}

/** `POST /oauth/token`: a code, or a refresh token, for tokens. Its status and body. */
export function exchange(w, config, codes, form, authorization) {
  let id = form.get("client_id");
  let secret = form.get("client_secret");
  if (authorization?.startsWith("Basic ")) {
    [id, secret] = Buffer.from(authorization.slice(6), "base64").toString().split(":");
  }
  if (id !== config.clientId || secret !== config.clientSecret) {
    return refuse(401, "invalid_client", "The client_id and client_secret aren't fctl's app's");
  }
  switch (form.get("grant_type")) {
    case "authorization_code": {
      const code = codes.get(form.get("code") ?? "");
      codes.delete(form.get("code") ?? "");
      if (!code || code.expiresAt < Date.now())
        return refuse(400, "invalid_grant", "The code is unknown or expired");
      if (form.get("redirect_uri") !== code.redirectUri) {
        return refuse(400, "invalid_grant", "The redirect_uri isn't the one the code was issued for");
      }
      if (code.challenge) {
        const verifier = form.get("code_verifier") ?? "";
        const derived =
          code.method === "S256" ? createHash("sha256").update(verifier).digest("base64url") : verifier;
        if (derived !== code.challenge) {
          return refuse(400, "invalid_grant", "The code_verifier doesn't match the code_challenge");
        }
      }
      return { status: 200, body: issue(w, code, true), changed: true };
    }
    case "refresh_token": {
      const old = w.tokens[form.get("refresh_token") ?? ""];
      if (old?.kind !== "refresh" || old.revoked) {
        return refuse(400, "invalid_grant", "The refresh token is unknown or revoked");
      }
      if (old.used) {
        // Two renewals that race end alike, for a while.
        if (Date.now() - old.used.at < REUSE_MS) return { status: 200, body: old.used.answer };
        return refuse(400, "invalid_grant", "The refresh token was used already");
      }
      const answer = issue(w, old);
      old.used = { at: Date.now(), answer };
      return { status: 200, body: answer, changed: true };
    }
    default:
      return refuse(400, "unsupported_grant_type", "Use authorization_code or refresh_token");
  }
}

/** `POST /oauth/revoke`: ends a token and its pair. */
export function revoke(w, form) {
  const t = w.tokens[form.get("token") ?? ""];
  if (!t) return refuse(400, "invalid_token", "The token is unknown");
  t.revoked = true;
  if (w.tokens[t.pair]) w.tokens[t.pair].revoked = true;
  return { status: 200, body: {}, changed: true };
}

/** Who a request's bearer token is, while it works: `{ userId, app, scopes }`. */
export function actorOf(w, authorization) {
  const bearer = /^Bearer (.+)$/i.exec(authorization ?? "")?.[1]?.trim();
  const t = bearer && w.tokens[bearer];
  if (t?.kind !== "access" || t.revoked || t.expiresAt < Date.now()) return null;
  return { userId: t.userId, app: t.app, scopes: t.scopes };
}

/** Ends every token of fctl's app, as uninstalling it in Linear does. */
export function uninstall(w) {
  for (const t of Object.values(w.tokens)) {
    if (t.app) t.revoked = true;
  }
}

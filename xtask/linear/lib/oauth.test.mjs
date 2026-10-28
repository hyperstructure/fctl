import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import { test } from "node:test";
import { actorOf, authorize, exchange, revoke, uninstall } from "./oauth.mjs";
import * as W from "./world.mjs";

const config = { clientId: "fctl-dev-linear", clientSecret: "secret" };
const redirect = "http://localhost:8787/api/v1/linear/installed";

/** A sign-in through the authorize page, as fctl sends people there: the page's answer and the verifier. */
function signIn(codes, { app = true, scope, ...more } = {}) {
  const verifier = randomBytes(32).toString("base64url");
  const params = new URLSearchParams({
    client_id: config.clientId,
    redirect_uri: redirect,
    response_type: "code",
    scope: scope ?? (app ? "read,write,app:assignable,app:mentionable" : "read"),
    state: "the-state",
    code_challenge: createHash("sha256").update(verifier).digest("base64url"),
    code_challenge_method: "S256",
    prompt: "consent",
    ...(app ? { actor: "app" } : {}),
    ...more,
  });
  return { ...authorize(config, codes, params), verifier };
}

const form = (fields) =>
  new URLSearchParams({ client_id: config.clientId, client_secret: config.clientSecret, ...fields });

function tokens(w, codes, signedIn, verifier = signedIn.verifier) {
  const code = new URL(signedIn.location).searchParams.get("code");
  return exchange(
    w,
    config,
    codes,
    form({ grant_type: "authorization_code", code, redirect_uri: redirect, code_verifier: verifier }),
  );
}

test("installs fctl's app with PKCE, and its token is the app's", () => {
  const w = W.seed();
  const codes = new Map();
  const signedIn = signIn(codes);
  const back = new URL(signedIn.location);
  assert.equal(back.origin + back.pathname, redirect);
  assert.equal(back.searchParams.get("state"), "the-state");
  const answer = tokens(w, codes, signedIn);
  assert.equal(answer.status, 200);
  assert.equal(answer.body.scope, "read write app:assignable app:mentionable");
  assert.equal(answer.body.expires_in, 86_399);
  assert.deepEqual(actorOf(w, `Bearer ${answer.body.access_token}`), {
    userId: W.APP_USER,
    app: true,
    scopes: ["read", "write", "app:assignable", "app:mentionable"],
  });
  // A code works once.
  assert.equal(tokens(w, codes, signedIn).body.error, "invalid_grant");
});

test("refuses a sign-in whose verifier, client or scopes aren't right", () => {
  const w = W.seed();
  const codes = new Map();
  assert.equal(tokens(w, codes, signIn(codes), "another-verifier").body.error, "invalid_grant");
  const signedIn = signIn(codes);
  const code = new URL(signedIn.location).searchParams.get("code");
  const wrongSecret = exchange(
    w,
    config,
    codes,
    form({ grant_type: "authorization_code", code, redirect_uri: redirect, client_secret: "nope" }),
  );
  assert.equal(wrongSecret.status, 401);
  // A person's sign-in can't take the app's scopes.
  const asPerson = signIn(codes, { app: false, scope: "read,app:assignable" });
  assert.equal(new URL(asPerson.location).searchParams.get("error"), "invalid_scope");
  assert.match(authorize(config, codes, new URLSearchParams({ client_id: "other" })).problem, /client_id/);
});

test("a person's sign-in is the dev owner's, and fctl revokes it", () => {
  const w = W.seed();
  const codes = new Map();
  const answer = tokens(w, codes, signIn(codes, { app: false }));
  const bearer = `Bearer ${answer.body.access_token}`;
  assert.deepEqual(actorOf(w, bearer), { userId: W.DEV_USER, app: false, scopes: ["read"] });
  assert.equal(revoke(w, new URLSearchParams({ token: answer.body.access_token })).status, 200);
  assert.equal(actorOf(w, bearer), null);
  assert.equal(revoke(w, new URLSearchParams({ token: "unknown" })).status, 400);
});

test("refresh tokens change at each use, and a use again soon answers the same", () => {
  const w = W.seed();
  const codes = new Map();
  const first = tokens(w, codes, signIn(codes)).body;
  const refresh = () =>
    exchange(w, config, codes, form({ grant_type: "refresh_token", refresh_token: first.refresh_token }));
  const renewed = refresh();
  assert.equal(renewed.status, 200);
  assert.notEqual(renewed.body.refresh_token, first.refresh_token);
  assert.ok(actorOf(w, `Bearer ${renewed.body.access_token}`));
  assert.deepEqual(refresh().body, renewed.body);
});

test("installing the app again, or uninstalling it, ends the tokens it had", () => {
  const w = W.seed();
  const codes = new Map();
  const first = tokens(w, codes, signIn(codes)).body;
  const second = tokens(w, codes, signIn(codes)).body;
  assert.equal(actorOf(w, `Bearer ${first.access_token}`), null);
  assert.ok(actorOf(w, `Bearer ${second.access_token}`));
  uninstall(w);
  assert.equal(actorOf(w, `Bearer ${second.access_token}`), null);
});

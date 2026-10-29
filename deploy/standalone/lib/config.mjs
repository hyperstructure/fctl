// The server's configuration, from the environment: the instance's
// variables and secrets under the names the Worker reads on Cloudflare
// (crates/fctl-control/src/config.rs), where it listens and where it keeps
// its data. Any of them can come from a file instead, `<NAME>_FILE`, as
// Docker and Kubernetes secrets do.

import { createPrivateKey, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Passed to the Worker as they are, when set. */
const VARS = [
  "FCTL_INSTANCE_NAME",
  "FCTL_SEARCH_ENGINES",
  "GITHUB_APP_ID",
  "GITHUB_APP_SLUG",
  "GITHUB_CLIENT_ID",
  "GITHUB_CLIENT_SECRET",
  "GITHUB_DEFAULT_OWNER",
  "GITHUB_WEBHOOK_SECRET",
  "GITHUB_REVIEW_APP_ID",
  "GITHUB_REVIEW_APP_SLUG",
  "GOOGLE_CLIENT_ID",
  "GOOGLE_CLIENT_SECRET",
  "LINEAR_CLIENT_ID",
  "LINEAR_CLIENT_SECRET",
  "LINEAR_WEBHOOK_SECRET",
  "RESEND_API_KEY",
  "FCTL_EMAIL_FROM",
];

const MASTER_KEY = /^[A-Za-z0-9+/]{43}=$/;

export class ConfigError extends Error {}

/** `name` from the environment, or from the file `<name>_FILE` names; trimmed, `undefined` when empty. */
export function value(env, name) {
  let v = env[name];
  const file = env[`${name}_FILE`];
  if ((v === undefined || v === "") && file) {
    try {
      v = readFileSync(file, "utf8");
    } catch (e) {
      throw new ConfigError(`${name}_FILE: can't read ${file} (${e.code ?? e.message})`);
    }
  }
  v = v?.trim();
  return v ? v : undefined;
}

/** `https://fctl.example.com/` → `https://fctl.example.com`; anything but an origin is an error. */
export function publicUrl(raw) {
  let url;
  try {
    url = new URL(raw);
  } catch {
    url = undefined;
  }
  if (!url || !["http:", "https:"].includes(url.protocol) || url.pathname !== "/" || url.search || url.hash) {
    throw new ConfigError(
      `FCTL_PUBLIC_URL must be the address people open the app at, like https://fctl.example.com or http://192.168.1.20:8787 (got ${raw})`,
    );
  }
  return url.origin;
}

/** Whether browsers count the app at `origin` as a secure context (https, or localhost). */
export function secureContext(origin) {
  const { protocol, hostname } = new URL(origin);
  return (
    protocol === "https:" ||
    hostname === "localhost" ||
    hostname.endsWith(".localhost") ||
    /^127\.\d+\.\d+\.\d+$/.test(hostname) ||
    hostname === "[::1]"
  );
}

function port(raw) {
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1 || n > 65535) {
    throw new ConfigError(`FCTL_PORT must be a port number (got ${raw})`);
  }
  return n;
}

/** How many reverse proxies in front add to X-Forwarded-For: `0` (none), `1`, `true` (one), … */
function trustProxy(raw) {
  if (raw === undefined || raw === "false" || raw === "no") return 0;
  if (raw === "true" || raw === "yes") return 1;
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 0) {
    throw new ConfigError(`FCTL_TRUST_PROXY must be how many proxies are in front, like 1 (got ${raw})`);
  }
  return n;
}

/**
 * A GitHub App's private key as PKCS#8 PEM, which is what WebCrypto
 * imports. GitHub hands out PKCS#1 (`BEGIN RSA PRIVATE KEY`); a key pasted
 * into one line works too, with `\n` for its line breaks, spaces or
 * nothing. `name`: the variable it came from, which an error names.
 */
export function githubKey(pem, name = "GITHUB_APP_PRIVATE_KEY") {
  const text = lines(pem.includes("\n") ? pem : pem.replaceAll("\\n", "\n"));
  try {
    return createPrivateKey(text).export({ type: "pkcs8", format: "pem" });
  } catch {
    throw new ConfigError(`${name} is not a PEM private key`);
  }
}

/** A PEM key on one line, its line breaks turned into spaces or dropped, with them again. */
function lines(pem) {
  if (pem.includes("\n")) return pem;
  const m = /^(-----BEGIN [A-Z ]+-----)(.+?)(-----END [A-Z ]+-----)$/.exec(pem.trim());
  if (!m) return pem;
  const body = m[2].replace(/\s+/g, "").match(/.{1,64}/g) ?? [];
  return [m[1], ...body, m[3], ""].join("\n");
}

/**
 * The master key: FCTL_MASTER_KEY, else the one this server made in its
 * data directory the first time. `made` says it was just made.
 */
export function masterKey(env, dataDir) {
  const path = join(dataDir, "master-key");
  let stored;
  try {
    stored = readFileSync(path, "utf8").trim();
  } catch (e) {
    if (e.code !== "ENOENT") throw e;
  }
  const given = value(env, "FCTL_MASTER_KEY");
  if (given) {
    if (!MASTER_KEY.test(given)) throw new ConfigError("FCTL_MASTER_KEY must be 32 bytes in base64");
    if (stored !== undefined && stored !== given) {
      throw new ConfigError(
        `FCTL_MASTER_KEY isn't the key in ${path}, which this server made before and encrypted credentials with: use that one, or remove the file`,
      );
    }
    return { key: given, made: false };
  }
  if (stored !== undefined) {
    if (!MASTER_KEY.test(stored))
      throw new ConfigError(`${path} doesn't hold a master key (32 bytes in base64)`);
    return { key: stored, made: false, path };
  }
  const key = randomBytes(32).toString("base64");
  writeFileSync(path, `${key}\n`, { mode: 0o600, flag: "wx" });
  return { key, made: true, path };
}

/** Everything but the master key, which lives in the data directory (`masterKey`). */
export function readConfig(env) {
  const url = publicUrl(value(env, "FCTL_PUBLIC_URL") ?? "http://localhost:8787");
  const vars = { FCTL_PUBLIC_URL: url };
  for (const name of VARS) {
    const v = value(env, name);
    if (v !== undefined) vars[name] = v;
  }
  for (const name of ["GITHUB_APP_PRIVATE_KEY", "GITHUB_REVIEW_APP_PRIVATE_KEY"]) {
    const key = value(env, name);
    if (key) vars[name] = githubKey(key, name);
  }
  // Made up when not given: it only matters until the first account exists.
  const setupToken = value(env, "FCTL_SETUP_TOKEN") ?? randomBytes(24).toString("base64url");
  vars.FCTL_SETUP_TOKEN = setupToken;
  return {
    publicUrl: url,
    listen: value(env, "FCTL_LISTEN") ?? "0.0.0.0",
    port: port(value(env, "FCTL_PORT") ?? value(env, "PORT") ?? "8787"),
    dataDir: value(env, "FCTL_DATA_DIR") ?? join(process.cwd(), "fctl-data"),
    trustProxy: trustProxy(value(env, "FCTL_TRUST_PROXY")),
    setupToken,
    vars,
  };
}

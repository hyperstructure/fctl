// The server's configuration, from the environment: the instance's
// variables and secrets under the names the Worker reads on Cloudflare
// (crates/fctl-control/src/config.rs), where it listens and where it keeps
// its data. Any of them can come from a file instead, `<NAME>_FILE`, as
// Docker and Kubernetes secrets do.

import { createPrivateKey, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { isIP } from "node:net";
import { join } from "node:path";
import { rootCertificates } from "node:tls";

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
  "SLACK_CLIENT_ID",
  "SLACK_CLIENT_SECRET",
  "SLACK_SIGNING_SECRET",
  "SLACK_APP_TOKEN",
  "RESEND_API_KEY",
  "FCTL_EMAIL_FROM",
  "FCTL_WEBHOOK_HTTP",
  "FCTL_WEBHOOK_PRIVATE_HOSTS",
  "FCTL_ADMIN_TOKENS",
  "FCTL_METRICS_TOKEN",
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
 * into one line works too, as the Worker reads it (`fctl_domain::private_key`).
 * `name`: the variable it came from, which an error names.
 */
export function githubKey(pem, name = "GITHUB_APP_PRIVATE_KEY") {
  try {
    return createPrivateKey(armored(pem)).export({ type: "pkcs8", format: "pem" });
  } catch {
    throw new ConfigError(`${name} is not a PEM private key`);
  }
}

/**
 * A PEM key in lines, as OpenSSL reads it, however it came: on one line, its
 * line breaks turned into spaces, dropped or written as `\n` (or `\r\n`), in
 * quotes, after other text, or the base64 alone, which is PKCS#8's.
 */
function armored(pem) {
  const m = /-----BEGIN ([A-Z ]+)-----([\s\S]*?)-----END \1-----/.exec(pem);
  if (!m && pem.includes("-----")) return pem;
  const [label, body] = m ? [m[1], m[2]] : ["PRIVATE KEY", pem];
  // Base64 has no backslash: one is a line break written as `\n`.
  const base64 = body.replace(/\\[rn]/g, "").replace(/\s+/g, "");
  const lines = base64.match(/.{1,64}/g) ?? [];
  return [`-----BEGIN ${label}-----`, ...lines, `-----END ${label}-----`, ""].join("\n");
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

const LABEL = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/i;

/**
 * One entry of FCTL_WEBHOOK_PRIVATE_HOSTS, as the Worker reads it (fctl_domain::hooks::Private):
 * an IP range (`10.0.0.0/8`, `fd00::/8`), an address, a host name (`hooks.lan`) or the names under
 * a domain (`*.corp.internal`).
 */
export function privateHost(entry) {
  const bare = (s) => s.replace(/^\[/, "").replace(/\]$/, "");
  const slash = entry.indexOf("/");
  if (slash >= 0) {
    const family = isIP(bare(entry.slice(0, slash)));
    const bits = entry.slice(slash + 1);
    return family !== 0 && /^\d{1,3}$/.test(bits) && Number(bits) <= (family === 4 ? 32 : 128);
  }
  if (isIP(bare(entry)) !== 0) return true;
  const name = entry.replace(/\.$/, "").replace(/^\*\./, "");
  return name.length <= 253 && name.split(".").every((l) => LABEL.test(l));
}

/**
 * Where webhooks may post besides public https:// addresses: FCTL_WEBHOOK_HTTP is 1 (or true)
 * or 0, and FCTL_WEBHOOK_PRIVATE_HOSTS lists only entries the Worker reads, separated by commas or
 * spaces; anything else stops the server, rather than the Worker leaving it out.
 */
export function webhookReach(env) {
  const http = value(env, "FCTL_WEBHOOK_HTTP");
  if (http !== undefined && !["0", "1", "true", "false"].includes(http))
    throw new ConfigError(`FCTL_WEBHOOK_HTTP must be 1 or 0 (got ${http})`);
  const hosts = (value(env, "FCTL_WEBHOOK_PRIVATE_HOSTS") ?? "").split(/[\s,]+/).filter(Boolean);
  const unread = hosts.filter((h) => !privateHost(h));
  if (unread.length > 0)
    throw new ConfigError(
      `FCTL_WEBHOOK_PRIVATE_HOSTS lists IP ranges (10.0.0.0/8), addresses, host names (hooks.lan) and the names under a domain (*.corp.internal): fctl can't read ${unread.join(", ")}`,
    );
}

/**
 * The network webhooks post through, the Worker's FCTL_WEBHOOK_NETWORK binding
 * (crates/fctl-control/src/do_hooks.rs): workerd connects it only to public addresses and the IP
 * ranges FCTL_WEBHOOK_PRIVATE_HOSTS lists, checking every address a host name resolves to as it
 * connects, so that a name someone points at this server's own network reaches nothing there. The
 * host names it lists the Worker reaches through its own fetch, to where they resolve. Its TLS
 * trusts what the Worker's own requests do (Miniflare's): browsers' authorities, Node's on Windows,
 * and the certificates NODE_EXTRA_CA_CERTS names.
 */
export function webhookNetwork(env, extraCerts = process.env.NODE_EXTRA_CA_CERTS) {
  const bare = (s) => s.replace(/^\[/, "").replace(/\]$/, "");
  const ranges = (value(env, "FCTL_WEBHOOK_PRIVATE_HOSTS") ?? "")
    .split(/[\s,]+/)
    .filter(Boolean)
    .flatMap((entry) => {
      const slash = entry.indexOf("/");
      const ip = bare(slash >= 0 ? entry.slice(0, slash) : entry);
      const family = isIP(ip);
      if (family === 0) return [];
      return [`${ip}/${slash >= 0 ? entry.slice(slash + 1) : family === 4 ? 32 : 128}`];
    });
  const trustedCertificates = process.platform === "win32" ? [...rootCertificates] : [];
  if (extraCerts) {
    try {
      const pem = readFileSync(extraCerts, "utf8");
      trustedCertificates.push(
        ...(pem.match(/-----BEGIN CERTIFICATE-----[\s\S]+?-----END CERTIFICATE-----/g) ?? []),
      );
    } catch {
      // As Miniflare does: a file it can't read adds nothing.
    }
  }
  return {
    network: {
      allow: ["public", ...ranges],
      deny: [],
      tlsOptions: { trustBrowserCas: true, trustedCertificates },
    },
  };
}

/**
 * Whether instance admins make admin tokens, which provision people: FCTL_ADMIN_TOKENS is 1 (or
 * true) or 0; anything else stops the server, rather than the Worker reading it as off.
 */
export function adminTokens(env) {
  const on = value(env, "FCTL_ADMIN_TOKENS");
  if (on !== undefined && !["0", "1", "true", "false"].includes(on))
    throw new ConfigError(`FCTL_ADMIN_TOKENS must be 1 or 0 (got ${on})`);
  return on === "1" || on === "true";
}

/** Everything but the master key, which lives in the data directory (`masterKey`). */
export function readConfig(env) {
  webhookReach(env);
  adminTokens(env);
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
    webhookNetwork: webhookNetwork(env),
    listen: value(env, "FCTL_LISTEN") ?? "0.0.0.0",
    port: port(value(env, "FCTL_PORT") ?? value(env, "PORT") ?? "8787"),
    dataDir: value(env, "FCTL_DATA_DIR") ?? join(process.cwd(), "fctl-data"),
    trustProxy: trustProxy(value(env, "FCTL_TRUST_PROXY")),
    setupToken,
    vars,
  };
}

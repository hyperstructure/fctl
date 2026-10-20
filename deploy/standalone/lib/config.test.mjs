import assert from "node:assert/strict";
import { generateKeyPairSync } from "node:crypto";
import { mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { ConfigError, githubKey, masterKey, publicUrl, readConfig, secureContext, value } from "./config.mjs";

const dir = () => mkdtempSync(join(tmpdir(), "fctl-config-"));

test("the public URL is an origin", () => {
  assert.equal(publicUrl("https://fctl.example.com/"), "https://fctl.example.com");
  assert.equal(publicUrl("http://192.168.1.20:8787"), "http://192.168.1.20:8787");
  for (const bad of ["fctl.example.com", "https://fctl.example.com/app", "ftp://x", "https://x/?a=1"]) {
    assert.throws(() => publicUrl(bad), ConfigError, bad);
  }
});

test("secure contexts are https and localhost", () => {
  assert.ok(secureContext("https://fctl.example.com"));
  assert.ok(secureContext("http://localhost:8787"));
  assert.ok(secureContext("http://127.0.0.1:8787"));
  assert.ok(!secureContext("http://192.168.1.20:8787"));
});

test("values come from <NAME>_FILE too", () => {
  const d = dir();
  writeFileSync(join(d, "secret"), "from-a-file\n");
  assert.equal(value({ A_FILE: join(d, "secret") }, "A"), "from-a-file");
  assert.equal(value({ A: "set", A_FILE: join(d, "secret") }, "A"), "set");
  assert.equal(value({ A: "  " }, "A"), undefined);
  assert.throws(() => value({ A_FILE: join(d, "missing") }, "A"), ConfigError);
});

test("the Worker gets the variables that are set, and a setup token", () => {
  const cfg = readConfig({
    FCTL_PUBLIC_URL: "https://fctl.example.com",
    FCTL_INSTANCE_NAME: "Acme",
    PORT: "9000",
    FCTL_TRUST_PROXY: "true",
    FCTL_DATA_DIR: "/data",
  });
  assert.equal(cfg.port, 9000);
  assert.equal(cfg.trustProxy, 1);
  assert.equal(cfg.vars.FCTL_PUBLIC_URL, "https://fctl.example.com");
  assert.equal(cfg.vars.FCTL_INSTANCE_NAME, "Acme");
  assert.equal(cfg.vars.FCTL_SETUP_TOKEN, cfg.setupToken);
  assert.ok(cfg.setupToken.length >= 32);
  assert.equal(cfg.vars.GITHUB_APP_ID, undefined);
  assert.equal(readConfig({ FCTL_SETUP_TOKEN: "given" }).setupToken, "given");
  assert.throws(() => readConfig({ FCTL_PORT: "http" }), ConfigError);
  assert.throws(() => readConfig({ FCTL_TRUST_PROXY: "-1" }), ConfigError);
});

test("GitHub's PKCS#1 keys become PKCS#8", () => {
  const { privateKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
  const pkcs1 = privateKey.export({ type: "pkcs1", format: "pem" });
  const pkcs8 = githubKey(pkcs1);
  assert.match(pkcs8, /^-----BEGIN PRIVATE KEY-----\n/);
  assert.equal(githubKey(pkcs1.trim().replaceAll("\n", "\\n")), pkcs8);
  assert.throws(() => githubKey("not a key"), ConfigError);
});

test("the master key is made once and kept", () => {
  const d = dir();
  const made = masterKey({}, d);
  assert.ok(made.made);
  assert.equal(Buffer.from(made.key, "base64").length, 32);
  assert.equal(statSync(made.path).mode & 0o777, 0o600);
  assert.equal(readFileSync(made.path, "utf8").trim(), made.key);
  const again = masterKey({}, d);
  assert.deepEqual([again.key, again.made], [made.key, false]);
  // One from the environment must be that one: credentials are encrypted with it.
  assert.equal(masterKey({ FCTL_MASTER_KEY: made.key }, d).key, made.key);
  const other = Buffer.alloc(32, 7).toString("base64");
  assert.throws(() => masterKey({ FCTL_MASTER_KEY: other }, d), ConfigError);
  assert.equal(masterKey({ FCTL_MASTER_KEY: other }, dir()).key, other);
  assert.throws(() => masterKey({ FCTL_MASTER_KEY: "short" }, dir()), ConfigError);
});

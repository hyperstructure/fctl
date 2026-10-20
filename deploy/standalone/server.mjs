#!/usr/bin/env node
// The fctl control plane outside Cloudflare (docs/self-hosting.md): the same
// Worker (crates/fctl-control) and web app (web/dist) that run on Cloudflare,
// run by workerd, Cloudflare's open-source Workers runtime, through
// Miniflare, with D1 and the Durable Objects kept on disk in FCTL_DATA_DIR.
// The container image (Dockerfile) runs it, and so does `cargo xtask serve`.
//
// It does what Cloudflare does around the Worker: applies the D1 migrations
// before it starts, sets the client's address (lib/proxy.mjs) and runs the
// hourly cron trigger (lib/schedule.mjs). It also keeps a second server off
// the same data (lib/lock.mjs), and exits when the runtime stops answering,
// so that whatever runs it starts it again.

import { execFile, spawn } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { ConfigError, masterKey, readConfig, secureContext } from "./lib/config.mjs";
import { acquire, LockError } from "./lib/lock.mjs";
import { createProxy } from "./lib/proxy.mjs";
import { hourly } from "./lib/schedule.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../..");
const control = join(root, "crates/fctl-control");
const build = join(control, "build");
// The Worker's bindings are the ones `wrangler dev` runs it with. Its name
// and D1 database id there also name the files the state lives in.
const wranglerToml = join(control, "wrangler.toml");
const NAME = "fctl-control";

const require = createRequire(import.meta.url);
const wranglerDir = dirname(require.resolve("wrangler/package.json"));
// The Miniflare this wrangler runs, whatever else is installed.
const { Miniflare, Log, LogLevel, convertV4MiniflareOptions } = createRequire(
  join(wranglerDir, "package.json"),
)("miniflare");

const log = (...args) => console.log(...args);

/** What to undo on the way out, last first. */
const cleanup = [];
let stopping = false;

/** Undoes what is up, then exits. */
async function exit(code) {
  if (stopping) return;
  stopping = true;
  setTimeout(() => process.exit(code), 10_000).unref();
  for (const step of cleanup.reverse()) {
    try {
      await step();
    } catch (e) {
      console.error(e?.message ?? String(e));
    }
  }
  process.exit(code);
}

/** Ours, in place of anyone else's: Miniflare's exit at once, without the steps above. */
function onSignals() {
  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
    process.removeAllListeners(signal);
    process.on(signal, () => {
      if (!stopping) log(`${signal}: stopping`);
      exit(0);
    });
  }
}

/** Runs `node <args>` to completion and returns its output, which goes on ours when it fails. */
function node(args, env) {
  return new Promise((done, fail) => {
    const child = spawn(process.execPath, args, { stdio: ["ignore", "pipe", "pipe"], env });
    let out = "";
    child.stdout.on("data", (d) => {
      out += d;
    });
    child.stderr.on("data", (d) => {
      out += d;
    });
    child.on("error", fail);
    child.on("close", (code, signal) => {
      if (code === 0) return done(out);
      process.stderr.write(out);
      fail(new Error(`${args.slice(1, 4).join(" ")} failed (${signal ?? code})`));
    });
  });
}

/** What wrangler needs from the environment, and nothing secret. Its own files go in a temporary directory. */
function wranglerEnv() {
  const env = {
    CI: "1",
    NO_COLOR: "1",
    WRANGLER_SEND_METRICS: "false",
    XDG_CONFIG_HOME: join(tmpdir(), "fctl-wrangler"),
    WRANGLER_LOG_PATH: join(tmpdir(), "fctl-wrangler", "logs"),
  };
  for (const name of ["PATH", "HOME", "TMPDIR", "LANG", "TZ"]) {
    if (process.env[name] !== undefined) env[name] = process.env[name];
  }
  return env;
}

async function main() {
  onSignals();
  const cfg = readConfig(process.env);
  const state = join(cfg.dataDir, "state");
  try {
    mkdirSync(state, { recursive: true });
  } catch (e) {
    if (e.code !== "EACCES" && e.code !== "EROFS" && e.code !== "EPERM") throw e;
    // A directory from the host, mounted at /data, keeps the host's owner.
    throw new ConfigError(
      `Can't write to ${cfg.dataDir} (${e.code}): make it writable for user ${process.getuid?.() ?? "?"}, which runs this server`,
    );
  }

  const lock = await acquire(cfg.dataDir, {
    log,
    onLost(other) {
      console.error(`${cfg.dataDir} was taken over by ${other}: stopping`);
      exit(1);
    },
  });
  cleanup.push(() => lock.release());
  const key = masterKey(process.env, cfg.dataDir);
  if (key.made) {
    log(
      `Made a master key in ${key.path}. It encrypts stored credentials and secrets: keep it with the data (or set FCTL_MASTER_KEY to it).`,
    );
  }

  // As `cargo xtask deploy` does before it deploys. The database's
  // d1_migrations table records the ones applied.
  const env = wranglerEnv();
  const migrated = await node(
    [
      join(wranglerDir, "bin/wrangler.js"),
      "d1",
      "migrations",
      "apply",
      "DB",
      "--local",
      "--persist-to",
      state,
      "--config",
      wranglerToml,
    ],
    env,
  );
  // wrangler prints a table of them after each one; a count says enough.
  const applied = /About to apply (\d+) migration/.exec(migrated);
  if (applied) log(`Applied ${applied[1]} database migrations`);
  const workerOptions = JSON.parse(
    (
      await promisify(execFile)(process.execPath, [join(here, "lib/worker-options.mjs"), wranglerToml], {
        env,
        maxBuffer: 16 * 1024 * 1024,
      })
    ).stdout,
  );
  if (!lock.held()) throw new LockError(`${cfg.dataDir} was taken over by another server`);

  const mf = new Miniflare(
    convertV4MiniflareOptions({
      host: "127.0.0.1",
      port: 0,
      resourcePersistencePath: join(state, "v3"),
      // Cloudflare's request metadata (`request.cf`: city, country) isn't
      // there to be had, and Miniflare would fetch a sample of it.
      cf: {},
      log: new Log(LogLevel.WARN),
      logRequests: false,
      handleStructuredLogs: ({ level, message }) =>
        (level === "error" || level === "warn" ? console.error : console.log)(message),
      workers: [
        {
          ...workerOptions,
          name: NAME,
          // worker-build's output: the module and its WebAssembly.
          modulesRules: undefined,
          modulesRoot: build,
          modules: [
            { type: "ESModule", path: join(build, "index.js") },
            { type: "CompiledWasm", path: join(build, "index_bg.wasm") },
          ],
          // In place of wrangler.toml's [vars], which are for development.
          bindings: { ...cfg.vars, FCTL_MASTER_KEY: key.key },
        },
      ],
    }),
  );
  onSignals();
  cleanup.push(() => mf.dispose());
  const ready = await mf.ready;
  const origin = ready.origin;

  const proxy = createProxy({
    upstream: { host: ready.hostname, port: Number(ready.port) },
    trustProxy: cfg.trustProxy,
    onError: (e) => console.error(`control plane: ${e.message}`),
  });
  await new Promise((done, fail) => {
    proxy.once("error", fail);
    proxy.listen(cfg.port, cfg.listen, done);
  });
  cleanup.push(() => proxy.closeAll());

  for (const cron of workerOptions.cronTriggers ?? []) {
    const job = hourly(cron, async (c, at) => {
      try {
        // Asked for each time: a workerd that restarted after a crash
        // invalidates the handles Miniflare gave out before.
        const worker = await mf.getWorker();
        await worker.scheduled({ cron: c, scheduledTime: new Date(at) });
      } catch (e) {
        console.error(`housekeeping failed: ${e.message}`);
      }
    });
    cleanup.push(() => job.stop());
  }

  // Exits when the runtime stops answering, for whatever runs this to start
  // it again: Miniflare restarts workerd after a crash, but not forever.
  let misses = 0;
  const watchdog = setInterval(async () => {
    try {
      const res = await fetch(`${origin}/api/v1/health`, { signal: AbortSignal.timeout(10_000) });
      misses = res.ok ? 0 : misses + 1;
    } catch {
      misses += 1;
    }
    if (misses >= 4) {
      console.error("The control plane stopped answering: exiting");
      exit(1);
    }
  }, 30_000);
  cleanup.push(() => clearInterval(watchdog));

  const { version } = await (await fetch(`${origin}/api/v1/health`)).json();
  log(
    `fctl control plane ${version} at ${cfg.publicUrl}, listening on ${cfg.listen}:${cfg.port}, data in ${cfg.dataDir}`,
  );
  if (!secureContext(cfg.publicUrl)) {
    log(
      "Over plain http, anyone on the network in between can read sign-ins and tokens, and browsers offer passkeys, notifications and installing the app only over https (or on localhost): keep it to a network you trust, or put a reverse proxy with TLS in front.",
    );
  }
  const instance = await (await fetch(`${origin}/api/v1/instance`)).json();
  if (instance.setup_required) {
    log(
      `Create the primary instance owner's account (the link works once):\n  ${cfg.publicUrl}/setup#token=${cfg.setupToken}`,
    );
  }
}

main().catch((e) => {
  console.error(e instanceof ConfigError || e instanceof LockError ? e.message : (e?.stack ?? String(e)));
  exit(1);
});

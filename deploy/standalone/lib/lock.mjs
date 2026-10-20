// One server per data directory. Two would each run the same Durable
// Objects, with two fleets handing the same sessions to runners, so a
// server holds `server.lock` in the directory and touches it every few
// seconds. A lock nobody touched for `staleMs` was left by a server that
// stopped without removing it (a crash, a killed container): the next one
// takes it over.

import { randomBytes } from "node:crypto";
import { readFileSync, statSync, unlinkSync, utimesSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export class LockError extends Error {}

function read(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return {};
  }
}

function describe(owner) {
  return owner.pid ? `process ${owner.pid} on ${owner.host ?? "another machine"}` : "another server";
}

/**
 * Takes the lock in `dir`, waiting up to `waitMs` for a holder that stops
 * touching it. `onLost` runs when another server takes it over after all,
 * which only happens when this one stalled for longer than `staleMs`.
 */
export async function acquire(dir, { staleMs = 30_000, waitMs = 45_000, beatMs = 5_000, log, onLost }) {
  const path = join(dir, "server.lock");
  const me = { pid: process.pid, host: hostname(), nonce: randomBytes(12).toString("hex") };
  const deadline = Date.now() + waitMs;
  let told = false;
  for (;;) {
    try {
      writeFileSync(path, `${JSON.stringify({ ...me, since: new Date().toISOString() })}\n`, { flag: "wx" });
      break;
    } catch (e) {
      if (e.code !== "EEXIST") throw e;
    }
    let age;
    try {
      age = Date.now() - statSync(path).mtimeMs;
    } catch {
      continue;
    }
    const owner = read(path);
    if (age > staleMs) {
      log(
        `Taking over ${path}: ${describe(owner)} left it ${Math.round(age / 1000)}s ago without removing it`,
      );
      try {
        unlinkSync(path);
      } catch {}
      continue;
    }
    if (Date.now() > deadline) {
      throw new LockError(
        `${dir} is in use by ${describe(owner)}: stop that server first (two would each run the same sessions)`,
      );
    }
    if (!told) {
      log(`Waiting for ${describe(owner)} to stop using ${dir}`);
      told = true;
    }
    await sleep(1_000);
  }

  const ours = () => read(path).nonce === me.nonce;
  const beat = setInterval(() => {
    if (!ours()) {
      clearInterval(beat);
      onLost?.(describe(read(path)));
      return;
    }
    const now = new Date();
    try {
      utimesSync(path, now, now);
    } catch {}
  }, beatMs);
  beat.unref();

  return {
    path,
    /** Whether this server still holds it. */
    held: ours,
    release() {
      clearInterval(beat);
      if (ours()) {
        try {
          unlinkSync(path);
        } catch {}
      }
    },
  };
}

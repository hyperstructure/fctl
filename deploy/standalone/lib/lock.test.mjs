import assert from "node:assert/strict";
import { existsSync, mkdtempSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { acquire, LockError } from "./lock.mjs";

const dir = () => mkdtempSync(join(tmpdir(), "fctl-lock-"));
const quiet = () => {};

test("one server at a time", async () => {
  const d = dir();
  const first = await acquire(d, { log: quiet });
  assert.ok(first.held());
  await assert.rejects(acquire(d, { log: quiet, waitMs: 300 }), LockError);
  first.release();
  assert.ok(!existsSync(join(d, "server.lock")));
  const second = await acquire(d, { log: quiet, waitMs: 300 });
  assert.ok(second.held());
  second.release();
});

test("a lock nobody touches any more is taken over", async () => {
  const d = dir();
  const path = join(d, "server.lock");
  writeFileSync(path, JSON.stringify({ pid: 1, host: "gone", nonce: "old" }));
  const old = new Date(Date.now() - 60_000);
  utimesSync(path, old, old);
  const said = [];
  const lock = await acquire(d, { log: (m) => said.push(m), waitMs: 300 });
  assert.ok(lock.held());
  assert.match(said[0], /process 1 on gone left it/);
  lock.release();
});

test("the holder hears when it is taken over", async () => {
  const d = dir();
  let lost;
  const lock = await acquire(d, { log: quiet, beatMs: 20, onLost: (who) => (lost = who) });
  writeFileSync(join(d, "server.lock"), JSON.stringify({ pid: 2, host: "elsewhere", nonce: "new" }));
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(lost, "process 2 on elsewhere");
  assert.ok(!lock.held());
  // Releasing leaves the other server's lock alone.
  lock.release();
  assert.ok(existsSync(join(d, "server.lock")));
});

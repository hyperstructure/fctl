import assert from "node:assert/strict";
import { test } from "node:test";
import { minuteOf, nextRun } from "./schedule.mjs";

test("hourly crons have a minute", () => {
  assert.equal(minuteOf("17 * * * *"), 17);
  assert.equal(minuteOf("0 * * * *"), 0);
  assert.equal(minuteOf("*/5 * * * *"), undefined);
  assert.equal(minuteOf("17 3 * * *"), undefined);
  assert.equal(minuteOf("75 * * * *"), undefined);
});

test("the next run is later in this hour or in the next", () => {
  const at = (hh, mm, ss = 0) => Date.UTC(2026, 9, 20, hh, mm, ss);
  assert.equal(nextRun(at(10, 5), 17), at(10, 17));
  assert.equal(nextRun(at(10, 17), 17), at(11, 17));
  assert.equal(nextRun(at(10, 17, 1), 17), at(11, 17));
  assert.equal(nextRun(at(23, 59), 17), at(0, 17) + 86_400_000);
});

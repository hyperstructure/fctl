import { describe, expect, it } from "vitest";
import type { Item } from "./transcript";
import { cameAfter, newestSeq } from "./unseen";

const prose = (seq: number): Item => ({ kind: "prose", key: `p${seq}`, seq, ts: seq, text: "…" });
const sending: Item = {
  kind: "user",
  key: "pending-1",
  seq: Number.MAX_SAFE_INTEGER,
  ts: 0,
  text: "Go on",
  source: "web",
  pending: true,
};

describe("what the reader hasn't seen", () => {
  it("counts the items after an event, not those loaded before it", () => {
    const now = [prose(10), prose(11)];
    const from = newestSeq(now);
    expect(from).toBe(11);
    // Earlier activity loads above, and two items come after.
    const later = [prose(3), prose(4), ...now, prose(12), prose(15)];
    expect(cameAfter(later, from)).toBe(2);
  });

  it("leaves out a message on its way, which isn't stored yet", () => {
    expect(newestSeq([prose(7), sending])).toBe(7);
    expect(cameAfter([prose(7), prose(8), sending], 7)).toBe(1);
    expect(newestSeq([])).toBe(0);
  });
});

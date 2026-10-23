import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { agoShort } from "./format";
import { RECENT_MS, sections } from "./sessions";

const hour = 3_600_000;
const now = Date.UTC(2026, 9, 23, 12);

const session = (
  number: number,
  status: G.SessionStatus,
  at: { created: number; moved?: number; updated?: number },
): G.Session =>
  ({
    id: `ses_${number}`,
    number,
    status,
    created_at: at.created,
    last_activity_at: at.moved,
    updated_at: at.updated ?? at.moved ?? at.created,
  }) as G.Session;

const numbers = (list: G.Session[]) => list.map((s) => s.number);

describe("a project's session sections", () => {
  it("put the sessions that haven't ended first, then the ones that moved in the last day", () => {
    const { working, recent, older } = sections(
      [
        [
          session(9, "running", { created: now - hour }),
          session(8, "idle", { created: now - 2 * hour }),
          session(7, "stopped", { created: now - 3 * hour, moved: now - 2 * hour }),
          session(6, "failed", { created: now - 30 * hour, moved: now - 23 * hour }),
          session(5, "stopped", { created: now - 30 * hour, moved: now - 25 * hour }),
          // Resumed by a comment on its pull request.
          session(4, "queued", { created: now - 300 * hour, moved: now - 400 * hour }),
          session(3, "stopped", { created: now - 300 * hour }),
        ],
      ],
      now,
    );
    expect(numbers(working)).toEqual([9, 8, 4]);
    expect(numbers(recent)).toEqual([7, 6]);
    expect(numbers(older)).toEqual([5, 3]);
  });

  it("list the ones at work newest first, and the ended ones by when they last moved", () => {
    const { working, recent, older } = sections(
      [
        [
          session(9, "running", { created: now - hour, moved: now - 5 * hour }),
          session(8, "running", { created: now - 2 * hour, moved: now }),
          session(7, "stopped", { created: now - 3 * hour, moved: now - 20 * hour }),
          session(6, "stopped", { created: now - 4 * hour, moved: now - 4 * hour }),
          session(5, "stopped", { created: now - 50 * hour, moved: now - 48 * hour }),
          session(4, "stopped", { created: now - 60 * hour, moved: now - 48 * hour }),
          // Days old, it came back for a comment on its pull request an hour ago.
          session(3, "stopped", { created: now - 300 * hour, moved: now - hour }),
          session(2, "stopped", { created: now - 400 * hour, moved: now - 30 * hour }),
        ],
      ],
      now,
    );
    expect(numbers(working)).toEqual([9, 8]);
    expect(numbers(recent)).toEqual([3, 6, 7]);
    // The newer first where they moved at the same time.
    expect(numbers(older)).toEqual([2, 5, 4]);
  });

  it("keep as recent the sessions whose time reads in minutes or hours", () => {
    const at = (moved: number) => sections([[session(1, "stopped", { created: 0, moved })]], now);
    expect(agoShort(now - RECENT_MS + 1000, now)).toBe("24h");
    expect(numbers(at(now - RECENT_MS + 1000).recent)).toEqual([1]);
    expect(agoShort(now - RECENT_MS, now)).toBe("1d");
    expect(numbers(at(now - RECENT_MS).older)).toEqual([1]);
  });

  it("take each session once from several lists, newest first, as its newest summary has it", () => {
    // It ended: the list of active sessions still has it, marked by the feed, as the list of
    // ended ones comes again with it.
    const ended = session(2, "stopped", { created: now - 5 * hour, moved: now - hour, updated: now });
    const stale = { ...ended, status: "running", updated_at: now - hour } as G.Session;
    const { working, recent } = sections(
      [
        [session(3, "running", { created: now - 4 * hour }), stale],
        [ended, session(1, "failed", { created: now - 6 * hour })],
      ],
      now,
    );
    expect(numbers(working)).toEqual([3]);
    expect(numbers(recent)).toEqual([2, 1]);
  });

  it("are empty until the lists come", () => {
    expect(sections([undefined, undefined], now)).toEqual({ working: [], recent: [], older: [] });
  });
});

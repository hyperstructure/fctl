import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { exportEvery, exported, exportHint, nextExport } from "./data-export";
import { clock, day } from "./format";

describe("data export", () => {
  const now = Date.UTC(2026, 10, 7, 12, 0, 0);
  const hour = 3_600_000;
  const week: G.DataExport = { interval_hours: 168 };

  it("says how often, as the CLI does", () => {
    expect(exportEvery(0)).toBe("at any time");
    expect(exportEvery(1)).toBe("once an hour");
    expect(exportEvery(12)).toBe("once every 12 hours");
    expect(exportEvery(24)).toBe("once a day");
    expect(exportEvery(72)).toBe("once every 3 days");
    expect(exportEvery(168)).toBe("once a week");
    expect(exportEvery(336)).toBe("once every 2 weeks");
    expect(exportEvery(720)).toBe("once every 30 days");
    expect(exportEvery(8760)).toBe("once a year");
  });

  it("waits only until the next one may start", () => {
    expect(nextExport(undefined, now)).toBeUndefined();
    expect(nextExport(week, now)).toBeUndefined();
    const waiting = { ...week, last_at: now - 48 * hour, next_at: now + 120 * hour };
    expect(nextExport(waiting, now)).toBe(now + 120 * hour);
    // The page stayed open past it.
    expect(nextExport(waiting, now + 121 * hour)).toBeUndefined();
  });

  it("tells you how often, and while you wait, until when", () => {
    expect(exportHint(undefined, now)).toBeUndefined();
    expect(exportHint({ interval_hours: 0, last_at: now - hour }, now)).toBeUndefined();
    expect(exportHint(week, now)).toBe("You can export it once a week.");
    const next = now + 120 * hour;
    expect(exportHint({ ...week, last_at: now - 48 * hour, next_at: next }, now)).toBe(
      `You exported it 2 days ago. You can export it once a week: again from ${day(next)}, ${clock(next)}.`,
    );
    const later = now + 20 * 24 * hour;
    expect(exportHint({ interval_hours: 720, last_at: now - 10 * 24 * hour, next_at: later }, now)).toBe(
      `You exported it on ${day(now - 10 * 24 * hour)}. You can export it once every 30 days: again from ${day(later)}, ${clock(later)}.`,
    );
  });

  it("counts an export from its start", () => {
    const me = { export: week } as G.Me;
    expect(exported(me, now).export).toEqual({
      interval_hours: 168,
      last_at: now,
      next_at: now + 168 * hour,
    });
    const anyTime = { export: { interval_hours: 0, next_at: undefined } } as G.Me;
    expect(exported(anyTime, now).export).toEqual({ interval_hours: 0, last_at: now, next_at: undefined });
    // From a control plane before it: nothing to count.
    const older = {} as G.Me;
    expect(exported(older, now)).toBe(older);
  });
});

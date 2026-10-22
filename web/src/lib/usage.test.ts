import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { clock } from "./format";
import {
  inSentence,
  limitLabel,
  percent,
  pressing,
  reportedAt,
  resetTime,
  share,
  shortLabel,
  standing,
} from "./usage";

const limit = (p: Partial<G.PlanLimit>): G.PlanLimit => ({ window: "five_hour", observed_at: 0, ...p });

describe("plan usage", () => {
  // Noon, local time: the day boundaries below are local too.
  const now = new Date(2026, 9, 17, 12, 0).getTime();
  const hour = 3_600_000;

  it("names limits", () => {
    expect(limitLabel("five_hour")).toBe("5-hour limit");
    expect(limitLabel("seven_day")).toBe("Weekly limit");
    expect(limitLabel("seven_day_opus")).toBe("Weekly Opus limit");
    expect(limitLabel("seven_day_overage_included")).toBe("Weekly Fable limit");
    expect(limitLabel("seven_day_new_model")).toBe("Weekly New model limit");
    expect(limitLabel("thirty_day")).toBe("Thirty day limit");
    expect(shortLabel("five_hour")).toBe("5-hour");
    expect(shortLabel("seven_day_sonnet")).toBe("Weekly Sonnet");
  });

  it("says where a limit stands", () => {
    expect(standing(limit({ utilization: 0.42, resets_at: now + hour }), now)).toBe("fine");
    expect(standing(limit({}), now)).toBe("fine");
    expect(standing(limit({ utilization: 0.85, resets_at: now + hour }), now)).toBe("close");
    expect(standing(limit({ utilization: 0.9, resets_at: now + hour }), now)).toBe("critical");
    // Claude Code warns about pace: a quarter of the week in its first day.
    expect(standing(limit({ utilization: 0.25, status: "allowed_warning" }), now)).toBe("fine");
    expect(standing(limit({ utilization: 0.93, status: "allowed_warning" }), now)).toBe("critical");
    expect(standing(limit({ utilization: 1.03, resets_at: now + hour }), now)).toBe("reached");
    expect(standing(limit({ status: "rejected", resets_at: now + hour }), now)).toBe("reached");
    // Once the window starts over, the reading says nothing about it.
    expect(standing(limit({ utilization: 1, status: "rejected", resets_at: now }), now)).toBe("reset");
  });

  it("fills meters", () => {
    expect(share(limit({ utilization: 0.42, resets_at: now + hour }), now)).toBe(0.42);
    expect(share(limit({ utilization: 1.3 }), now)).toBe(1);
    expect(share(limit({ status: "rejected" }), now)).toBe(1);
    expect(share(limit({ utilization: 0.9, resets_at: now - 1 }), now)).toBe(0);
    expect(percent(limit({ utilization: 0.416 }))).toBe("42%");
    expect(percent(limit({}))).toBe("");
  });

  it("finds the limit to warn about", () => {
    const fine = limit({ window: "five_hour", utilization: 0.2 });
    const warned = limit({ window: "seven_day", utilization: 0.25, status: "allowed_warning" });
    const close = limit({ window: "five_hour", utilization: 0.8, resets_at: now + hour });
    const critical = limit({ window: "seven_day", utilization: 0.9, resets_at: now + hour });
    const reached = limit({ window: "seven_day_opus", status: "rejected", resets_at: now + hour });
    expect(pressing([fine], now)).toBeUndefined();
    expect(pressing([fine, warned], now)).toBeUndefined();
    expect(pressing([fine, close], now)).toBe(close);
    expect(pressing([close, critical], now)).toBe(critical);
    expect(pressing([close, critical, reached], now)).toBe(reached);
    expect(reportedAt([limit({ observed_at: 5 }), limit({ observed_at: 9 })])).toBe(9);
  });

  it("says when a limit resets", () => {
    expect(resetTime(now + 2 * hour, now)).toBe(`at ${clock(now + 2 * hour)}`);
    expect(resetTime(now + 20 * hour, now)).toBe(`tomorrow at ${clock(now + 20 * hour)}`);
    expect(resetTime(now + 3 * 24 * hour, now)).toMatch(
      new RegExp(`^\\S+ at ${clock(now + 3 * 24 * hour)}$`),
    );
    expect(resetTime(now + 10 * 24 * hour, now)).toMatch(/^on /);
    expect(inSentence("Weekly Opus limit")).toBe("weekly Opus limit");
    expect(inSentence("5-hour limit")).toBe("5-hour limit");
  });
});

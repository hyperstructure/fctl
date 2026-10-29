import { describe, expect, it } from "vitest";
import { floorLines, floorPlaceholder, PROJECT_LINES, pick, shipWhen } from "./placeholders";

describe("placeholders", () => {
  it("ship at the time of day it is", () => {
    expect(shipWhen(0)).toBe("tonight");
    expect(shipWhen(4)).toBe("tonight");
    expect(shipWhen(5)).toBe("this morning");
    expect(shipWhen(10)).toBe("this morning");
    expect(shipWhen(11)).toBe("over lunch");
    expect(shipWhen(13)).toBe("over lunch");
    expect(shipWhen(14)).toBe("today");
    expect(shipWhen(16)).toBe("today");
    expect(shipWhen(17)).toBe("this evening");
    expect(shipWhen(20)).toBe("this evening");
    expect(shipWhen(21)).toBe("tonight");
    expect(shipWhen(23)).toBe("tonight");
  });

  it("put the time of day in the Agent Floor's line about shipping", () => {
    expect(floorLines(8)).toContain("What should we ship this morning?");
    expect(floorLines(12)).toContain("What should we ship over lunch?");
    expect(floorLines(22)).toContain("What should we ship tonight?");
  });

  it("are picked from the whole list", () => {
    const lines = ["a", "b", "c"];
    expect(pick(lines, () => 0)).toBe("a");
    expect(pick(lines, () => 0.5)).toBe("b");
    expect(pick(lines, () => 0.9999)).toBe("c");
    expect(pick(lines, () => 1)).toBe("c");
    expect(floorPlaceholder(new Date(2026, 9, 29, 9), () => 0)).toBe("What should an agent build next?");
  });

  it("name the project", () => {
    for (const line of PROJECT_LINES) expect(line("fctl")).toContain("fctl");
    expect(PROJECT_LINES[0]?.("fctl")).toBe("What should an agent do in fctl?");
  });

  it("are all different", () => {
    expect(new Set(floorLines(9)).size).toBe(floorLines(9).length);
    expect(new Set(PROJECT_LINES.map((l) => l("x"))).size).toBe(PROJECT_LINES.length);
  });
});

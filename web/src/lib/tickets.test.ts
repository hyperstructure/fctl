import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { LINEAR_OUTCOMES, linearOutcome, stateTone, TRACKERS, ticketLabel, trackerOf } from "./tickets";

describe("ticket trackers", () => {
  it("offers none, GitHub Issues and Linear, in that order", () => {
    expect(TRACKERS.map((t) => t.value)).toEqual(["none", "github", "linear"]);
  });

  it("reads a project from before trackers as GitHub Issues, as its agents always worked", () => {
    expect(trackerOf({} as G.ProjectSettings)).toBe("github");
    expect(trackerOf({ tickets: { tracker: "none" } } as G.ProjectSettings)).toBe("none");
  });

  it("has a word for every way back from Linear", () => {
    for (const o of LINEAR_OUTCOMES) expect(linearOutcome(o).text).not.toBe("");
    expect(linearOutcome("linked").tone).toBe("ok");
    expect(linearOutcome("not_app").text).toContain("admin of the Linear workspace");
  });

  it("colors states by how far along they are", () => {
    expect(stateTone("started")).toBe("text-blue");
    expect(stateTone("completed")).toBe("text-accent");
    expect(stateTone("canceled")).toBe("text-ink-4");
    expect(stateTone(undefined)).toBe("text-ink-3");
  });

  it("names a session's ticket with its tracker", () => {
    expect(
      ticketLabel({
        tracker: "linear",
        id: "i",
        key: "ENG-1",
        title: "Fix it",
        url: "https://linear.app/a/issue/ENG-1",
      }),
    ).toBe("ENG-1: Fix it, on Linear");
  });
});

import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { canDelete, merged, outcomeLabel, parts, total } from "./retrospectives";
import type { Item } from "./transcript";

const retro = (over: Partial<G.Retrospective> = {}): G.Retrospective => ({
  id: "rtr_1",
  project_id: "prj_1",
  session: { id: "ses_1", number: 1, title: "Fix it" },
  started_by: { id: "usr_ada", name: "Ada" },
  created_at: 10,
  outcome: "completed",
  worked_on: "The login redirect.",
  ...over,
});

const notice = (r: G.Retrospective, seq: number): Item =>
  ({
    kind: "notice",
    key: `n-${seq}`,
    seq,
    ts: r.created_at,
    notice: { level: "info", code: "retrospective", message: "Recorded", retrospective: r },
  }) as Item;

describe("parts", () => {
  it("leaves out the parts with nothing to say", () => {
    const r = retro({ approach: " ", insights: "Use the helper." });
    expect(parts(r).map((p) => p.label)).toEqual(["Worked on", "Insights"]);
  });
});

describe("merged", () => {
  it("adds what the transcript has to what was listed, newest first, once each", () => {
    const listed = [retro({ id: "rtr_1", created_at: 10 })];
    const items = [
      notice(retro({ id: "rtr_1", created_at: 10 }), 1),
      notice(retro({ id: "rtr_2", created_at: 20 }), 2),
    ];
    expect(merged(listed, items).map((r) => r.id)).toEqual(["rtr_2", "rtr_1"]);
    expect(merged(undefined, []).length).toBe(0);
  });
});

describe("canDelete", () => {
  it("lets who started the session and the project's managers delete one", () => {
    const r = retro();
    expect(canDelete(r, "editor", "usr_ada")).toBe(true);
    expect(canDelete(r, "editor", "usr_bob")).toBe(false);
    expect(canDelete(r, "maintainer", "usr_bob")).toBe(true);
    expect(canDelete(r, "viewer", undefined)).toBe(false);
  });
});

describe("outcomes", () => {
  it("names them as people say them, and counts them", () => {
    expect(outcomeLabel("partial")).toBe("Partly done");
    expect(total({ completed: 3, partial: 1, blocked: 2 })).toBe(6);
    expect(total(undefined)).toBe(0);
  });
});

import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { canDelete, changedAt, outcomeLabel, parts, total, unlisted } from "./retrospectives";
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

describe("unlisted", () => {
  it("names the transcript's retrospectives the list doesn't have", () => {
    const listed = [retro({ id: "rtr_1", created_at: 10 })];
    const items = [
      notice(retro({ id: "rtr_1", created_at: 10 }), 1),
      notice(retro({ id: "rtr_2", created_at: 20 }), 2),
    ];
    expect(unlisted(listed, items)).toEqual(["rtr_2@20"]);
    expect(unlisted(undefined, [])).toEqual([]);
  });

  it("names updates newer than the list's, once for each", () => {
    const recorded = retro({ id: "rtr_1", created_at: 10, outcome: "blocked" });
    const updated = retro({ id: "rtr_1", created_at: 10, updated_at: 30 });
    const items = [notice(recorded, 1), notice(updated, 2)];
    expect(unlisted([recorded], items)).toEqual(["rtr_1@30"]);
    // The list has caught up, or is newer still.
    expect(unlisted([updated], items)).toEqual([]);
    expect(unlisted([retro({ id: "rtr_1", created_at: 10, updated_at: 40 })], items)).toEqual([]);
  });
});

describe("changedAt", () => {
  it("is when the agent last said something", () => {
    expect(changedAt(retro({ created_at: 10 }))).toBe(10);
    expect(changedAt(retro({ created_at: 10, updated_at: 30 }))).toBe(30);
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

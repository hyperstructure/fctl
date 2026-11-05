import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { movesLists } from "./feed";

const summary = (more: Partial<G.Session> = {}) =>
  ({ id: "ses_1", status: "running", archived: false, title: "Fix the build", ...more }) as G.Session;

describe("a session's summary from the feed", () => {
  it("refreshes lists when the session is new to it, moves on, or is archived or brought back", () => {
    expect(movesLists(undefined, summary())).toBe(true);
    expect(movesLists(summary(), summary({ status: "idle" }))).toBe(true);
    expect(movesLists(summary({ status: "stopping" }), summary({ status: "stopped" }))).toBe(true);
    expect(movesLists(summary({ status: "stopped" }), summary({ status: "stopped", archived: true }))).toBe(
      true,
    );
    expect(movesLists(summary({ archived: true }), summary())).toBe(true);
  });

  it("leaves them to the patch for the rest, as an agent's steps change it", () => {
    expect(movesLists(summary(), summary({ title: "Fix the flaky build", last_activity_at: 2 }))).toBe(false);
  });
});

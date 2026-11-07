import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { cameBack, hideable, staysHidden } from "./hidden-sessions";

const session = (status: G.SessionStatus, turns = 2, question?: string, updated_at = 20) => ({
  id: "ses_1",
  status,
  question: question ? ({ text: question } as G.Session["question"]) : undefined,
  updated_at,
  stats: { turns },
});

describe("hiding a session from the sidebar", () => {
  it("is for one whose agent rests and asks nothing", () => {
    expect(hideable(session("idle"))).toBe(true);
    expect(hideable(session("stopped"))).toBe(true);
    expect(hideable(session("failed"))).toBe(true);
    expect(hideable(session("idle", 2, "Which branch?"))).toBe(false);
    for (const status of ["queued", "provisioning", "preparing", "running", "paused", "stopping"] as const)
      expect(hideable(session(status)), status).toBe(false);
  });

  it("lasts while it rests, as it stops too", () => {
    const hidden = { turns: 2, updated: 10, at: 0 };
    expect(staysHidden(session("idle"), hidden)).toBe(true);
    expect(staysHidden(session("stopping"), hidden)).toBe(true);
    expect(staysHidden(session("stopped"), hidden)).toBe(true);
    expect(staysHidden(session("idle"), undefined)).toBe(false);
  });

  it("ends once its agent works again, asks something, or has ended another turn", () => {
    const hidden = { turns: 2, updated: 10, at: 0 };
    for (const status of ["queued", "provisioning", "preparing", "running", "paused"] as const)
      expect(staysHidden(session(status), hidden), status).toBe(false);
    expect(staysHidden(session("idle", 2, "Which branch?"), hidden)).toBe(false);
    // It worked while nobody looked, and rests again.
    expect(staysHidden(session("idle", 3), hidden)).toBe(false);
    expect(staysHidden(session("stopped", 3), hidden)).toBe(false);
  });

  it("is forgotten on a summary newer than the one it was hidden from", () => {
    const hidden = { turns: 2, updated: 10, at: 0 };
    // At work again, past the turn it was hidden after, or asking.
    expect(cameBack(session("running", 2), hidden)).toBe(true);
    expect(cameBack(session("idle", 3), hidden)).toBe(true);
    expect(cameBack(session("idle", 2, "Which branch?"), hidden)).toBe(true);
    // Another tab whose lists are behind still sees it at work on the turn it was hidden after: one
    // that stopped or failed during it ended no turn, so it has as many as then.
    expect(cameBack(session("running", 2, undefined, 9), hidden)).toBe(false);
    expect(cameBack(session("running", 1, undefined, 9), hidden)).toBe(false);
    expect(cameBack(session("idle", 1, "Which branch?", 9), hidden)).toBe(false);
    // Still resting, or not hidden.
    expect(cameBack(session("idle", 2), hidden)).toBe(false);
    expect(cameBack(session("running", 2), undefined)).toBe(false);
  });
});

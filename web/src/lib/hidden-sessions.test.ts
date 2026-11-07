import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { hideable, staysHidden } from "./hidden-sessions";

const session = (status: G.SessionStatus, turns = 2, question?: string) => ({
  id: "ses_1",
  status,
  question: question ? ({ text: question } as G.Session["question"]) : undefined,
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
    const hidden = { turns: 2, at: 0 };
    expect(staysHidden(session("idle"), hidden)).toBe(true);
    expect(staysHidden(session("stopping"), hidden)).toBe(true);
    expect(staysHidden(session("stopped"), hidden)).toBe(true);
    expect(staysHidden(session("idle"), undefined)).toBe(false);
  });

  it("ends once its agent works again, asks something, or has ended another turn", () => {
    const hidden = { turns: 2, at: 0 };
    for (const status of ["queued", "provisioning", "preparing", "running", "paused"] as const)
      expect(staysHidden(session(status), hidden), status).toBe(false);
    expect(staysHidden(session("idle", 2, "Which branch?"), hidden)).toBe(false);
    // It worked while nobody looked, and rests again.
    expect(staysHidden(session("idle", 3), hidden)).toBe(false);
    expect(staysHidden(session("stopped", 3), hidden)).toBe(false);
  });
});

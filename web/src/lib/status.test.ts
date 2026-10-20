import { describe, expect, it } from "vitest";
import type { Stall } from "./stall";
import { stallLabel } from "./stall";
import { statusNotes, statusReason, waitsForYou } from "./status";

describe("status", () => {
  const background = { status: "idle", status_reason: "Waiting for 2 background tasks" } as const;

  it("shows the reasons the status pill doesn't give", () => {
    expect(statusReason({ status: "idle", status_reason: "Waiting for a message" })).toBeUndefined();
    expect(statusReason(background)).toBe("Waiting for 2 background tasks");
    expect(
      statusReason({ status: "running", status_reason: "Resuming after the usage limit reset" }),
    ).toBeUndefined();
    expect(statusReason({ status: "stopped", status_reason: "Stopped after being idle" })).toBe(
      "Stopped after being idle",
    );
    expect(statusReason({ status: "queued" })).toBeUndefined();
  });

  it("tells sessions that wait for you from ones that wait for background work", () => {
    expect(waitsForYou({ status: "idle", status_reason: "Waiting for a message" })).toBe(true);
    expect(waitsForYou({ status: "idle" })).toBe(true);
    expect(waitsForYou(background)).toBe(false);
    expect(waitsForYou({ status: "running" })).toBe(false);
  });

  it("puts what is most pressing first beside the status", () => {
    const now = Date.UTC(2026, 9, 20, 12);
    const paused = {
      status: "paused",
      status_reason: "Waiting for the usage limit to reset",
      resume_at: now + 4 * 60_000,
    } as const;
    const quiet = { reconnecting: false };
    expect(statusNotes(paused, quiet, now)).toEqual([
      { text: "resumes in 4 min", warn: true },
      { text: "Waiting for the usage limit to reset" },
    ]);
    const stall: Stall = {
      kind: "away",
      runner: "dev-host",
      status: "paused",
      since: now - 60_000,
      showFrom: 0,
    };
    expect(statusNotes(paused, { reconnecting: true, stall }, now).map((n) => n.text)).toEqual([
      "Reconnecting…",
      stallLabel(stall, now),
      "resumes in 4 min",
      "Waiting for the usage limit to reset",
    ]);
    // The pill says it all.
    expect(statusNotes({ status: "idle", status_reason: "Waiting for a message" }, quiet, now)).toEqual([]);
    expect(
      statusNotes({ status: "provisioning", status_reason: "Creating the sandbox" }, quiet, now),
    ).toEqual([{ text: "Creating the sandbox" }]);
  });
});

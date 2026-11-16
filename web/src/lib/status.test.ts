import { describe, expect, it } from "vitest";
import type { Stall } from "./stall";
import { stallLabel } from "./stall";
import { besidePill, statusNotes, statusReason, waitsForYou } from "./status";

describe("status", () => {
  const background = { status: "idle", status_reason: "Waiting for 2 background tasks" } as const;

  it("shows the reasons the status pill doesn't give", () => {
    expect(statusReason({ status: "idle", status_reason: "Waiting for a message" })).toBeUndefined();
    expect(
      statusReason({ status: "idle", status_reason: "Cut off twice: waiting for a message" }),
    ).toBeUndefined();
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
    // Claude Code was cut off twice, and waits for a message rather than carry on.
    expect(waitsForYou({ status: "idle", status_reason: "Cut off twice: waiting for a message" })).toBe(true);
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
    // A runner short of disk comes after the work that doesn't come through, in red once nearly out.
    const disk = { room: "out" as const, free_mb: 800, min_free_mb: 20_480, since: now, runner: "dev-host" };
    expect(statusNotes({ status: "running" }, { reconnecting: false, stall, disk }, now)).toEqual([
      { text: stallLabel(stall, now), warn: true },
      { text: "Runner nearly out of disk: 800 MiB free", warn: true, danger: true },
    ]);
    expect(
      statusNotes({ status: "running" }, { reconnecting: false, disk: { ...disk, room: "low" } }, now),
    ).toEqual([{ text: "Runner low on disk: 800 MiB free", warn: true, danger: false }]);
    // The pill says it all.
    expect(statusNotes({ status: "idle", status_reason: "Waiting for a message" }, quiet, now)).toEqual([]);
    expect(
      statusNotes({ status: "provisioning", status_reason: "Creating the sandbox" }, quiet, now),
    ).toEqual([{ text: "Creating the sandbox" }]);
  });

  it("leaves out the word the status pill says already", () => {
    expect(besidePill("Stopped after being idle", "Stopped")).toBe("after being idle");
    expect(besidePill("Stopping: studio is draining", "Stopping")).toBe("studio is draining");
    expect(besidePill("Waiting for 2 background tasks", "Waiting")).toBe("for 2 background tasks");
    // Others are as they are: another word, a word the pill's starts, or the pill's word alone.
    expect(besidePill("Waiting for a runner", "Queued")).toBe("Waiting for a runner");
    expect(besidePill("Stoppedness", "Stopped")).toBe("Stoppedness");
    expect(besidePill("Stopped", "Stopped")).toBe("Stopped");
    expect(besidePill("resumes in 4m", "Paused")).toBe("resumes in 4m");
  });
});

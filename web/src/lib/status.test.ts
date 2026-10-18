import { describe, expect, it } from "vitest";
import { statusReason, waitsForYou } from "./status";

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
});

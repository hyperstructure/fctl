import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { clock } from "./format";
import { AWAY_GRACE_MS, diskLabel, diskOf, diskText, since, stallLabel, stallOf, stallText } from "./stall";

const session = (status: G.SessionStatus): G.Session =>
  ({ status, runner: { id: "rnr_1", name: "now" } }) as G.Session;

const connected = { connection: "live" as const, runnerConnected: true };

function must<T>(v: T | undefined): T {
  if (v === undefined) throw new Error("expected a stall");
  return v;
}

describe("stalls", () => {
  // Noon, local time.
  const now = new Date(2026, 9, 18, 12, 0).getTime();
  const minute = 60_000;

  it("say when a runner went away, once it has been gone for a few seconds", () => {
    const away = {
      connection: "live" as const,
      runnerConnected: false,
      runnerAway: { lastSeenAt: now - 20_000, noticedAt: now },
    };
    const st = stallOf(session("running"), away);
    expect(st).toMatchObject({ kind: "away", runner: "now", since: now - 20_000 });
    // A host that restarts connects again within the grace.
    expect(st?.showFrom).toBe(now + AWAY_GRACE_MS);
    expect(stallText(must(st), now)).toEqual({
      title: `Runner now hasn't been heard from since ${clock(now - 20_000)}.`,
      detail: "The agent's work shows up here when it's back, and messages you send reach the agent then.",
    });
    expect(stallLabel(must(st), now)).toBe(`Runner not heard from since ${clock(now - 20_000)}`);
  });

  it("show a runner gone for long at once", () => {
    const st = stallOf(session("idle"), {
      connection: "live",
      runnerConnected: false,
      runnerAway: { lastSeenAt: now - 10 * minute, noticedAt: now },
    });
    expect(st?.showFrom).toBe(now - 9 * minute);
  });

  it("say what the runner says about reaching the agent", () => {
    const st = stallOf(session("running"), {
      ...connected,
      agentUnreachable: { since: now - 2 * minute, reason: "nothing came from fctl-agent for a minute" },
    });
    expect(st).toMatchObject({ kind: "unreachable", showFrom: 0 });
    expect(stallText(must(st), now)).toEqual({
      title: `Runner now hasn't been able to reach the agent in its sandbox since ${clock(now - 2 * minute)}: nothing came from fctl-agent for a minute.`,
      detail:
        "It keeps trying. The agent's work shows up here when it gets through, and messages you send reach the agent then.",
    });
    expect(stallLabel(must(st), now)).toBe(`Agent unreachable since ${clock(now - 2 * minute)}`);
  });

  it("leave messages out of a session that takes none, and end the sentence once", () => {
    const st = stallOf(session("stopping"), {
      ...connected,
      agentUnreachable: { since: now, reason: "`docker exec` did not finish within 60 seconds." },
    });
    expect(stallText(must(st), now)).toEqual({
      title: `Runner now hasn't been able to reach the agent in its sandbox since ${clock(now)}: \`docker exec\` did not finish within 60 seconds.`,
      detail: "It keeps trying. The agent's work shows up here when it gets through.",
    });
  });

  it("are none while the work comes through, the session has ended, or the page itself reconnects", () => {
    expect(stallOf(session("running"), connected)).toBeUndefined();
    const away = { runnerConnected: false, runnerAway: { lastSeenAt: now, noticedAt: now } };
    expect(stallOf(session("stopped"), { ...away, connection: "live" })).toBeUndefined();
    expect(stallOf(session("failed"), { ...away, connection: "live" })).toBeUndefined();
    expect(stallOf(session("queued"), { ...away, connection: "live" })).toBeUndefined();
    expect(stallOf(session("running"), { ...away, connection: "reconnecting" })).toBeUndefined();
    // Not connected yet in this run: nothing was heard to miss.
    expect(stallOf(session("provisioning"), { connection: "live", runnerConnected: false })).toBeUndefined();
    // The runner is away: what it said about the agent is out of date.
    expect(
      stallOf(session("running"), {
        connection: "live",
        runnerConnected: false,
        agentUnreachable: { since: now, reason: "x" },
      }),
    ).toBeUndefined();
  });

  it("name days other than today", () => {
    const yesterday = new Date(2026, 9, 17, 23, 10).getTime();
    expect(since(now - minute, now)).toBe(`since ${clock(now - minute)}`);
    expect(since(yesterday, now)).toBe(`since yesterday at ${clock(yesterday)}`);
    expect(since(new Date(2026, 9, 14, 9, 12).getTime(), now)).not.toContain("yesterday");
  });

  it("say when the runner paused the sandbox for want of disk, and what follows", () => {
    const st = stallOf(session("running"), {
      ...connected,
      sandboxPaused: { since: now - 3 * minute, reason: "runner now is nearly out of disk: 800 MiB free" },
      // A paused sandbox doesn't answer: that is no news.
      agentUnreachable: { since: now, reason: "nothing came from fctl-agent for a minute" },
    });
    expect(st).toMatchObject({ kind: "paused", showFrom: 0, since: now - 3 * minute });
    expect(stallText(must(st), now)).toEqual({
      title: `Runner now paused this session's sandbox since ${clock(now - 3 * minute)}: runner now is nearly out of disk: 800 MiB free.`,
      detail:
        "The sandbox keeps what the agent wrote. The agent carries on where it was once there is room, and messages you send reach the agent then.",
    });
    expect(stallLabel(must(st), now)).toBe(`Sandbox paused since ${clock(now - 3 * minute)}`);
    // Its runner's disk goes without saying then.
    const disk = { room: "out" as const, free_mb: 800, min_free_mb: 20_480, since: now - 4 * minute };
    expect(diskOf(session("running"), { connection: "live", runnerDisk: disk }, st)).toBeUndefined();
  });

  it("say when the runner is low on disk, while the agent's work comes through", () => {
    const low = { room: "low" as const, free_mb: 12_595, min_free_mb: 20_480, since: now - 5 * minute };
    // Not a stall: the work in progress shows.
    expect(stallOf(session("running"), { ...connected })).toBeUndefined();
    const d = diskOf(session("running"), { connection: "live", runnerDisk: low });
    expect(d).toMatchObject({ room: "low", runner: "now" });
    expect(diskText(must(d), now)).toEqual({
      title: `Runner now has been low on disk since ${clock(now - 5 * minute)}: 12.3 GiB free, below the 20 GiB its host keeps free.`,
      detail:
        "No new sessions start on it until there is room, and its host removes stopped sandboxes to make some. If its disk fills up, the agent's writes fail.",
    });
    expect(diskLabel(must(d))).toBe("Runner low on disk: 12.3 GiB free");
  });

  it("say when the runner is nearly out of disk", () => {
    const out = { room: "out" as const, free_mb: 800, min_free_mb: 20_480, since: now - minute };
    const d = must(diskOf(session("idle"), { connection: "live", runnerDisk: out }));
    expect(diskText(d, now)).toEqual({
      title: `Runner now has been nearly out of disk since ${clock(now - minute)}: 800 MiB free.`,
      detail:
        "The agent's writes are about to fail, until its host has removed stopped sandboxes or space is freed on it.",
    });
    expect(diskLabel(d)).toBe("Runner nearly out of disk: 800 MiB free");
  });

  it("say nothing of the disk of a session that has ended, or while the page reconnects", () => {
    const low = { room: "low" as const, free_mb: 1, min_free_mb: 2, since: now };
    expect(diskOf(session("stopped"), { connection: "live", runnerDisk: low })).toBeUndefined();
    expect(diskOf(session("running"), { connection: "reconnecting", runnerDisk: low })).toBeUndefined();
    expect(diskOf(session("running"), { connection: "live" })).toBeUndefined();
  });

  it("say the runner when the session has none", () => {
    const st = stallOf({ status: "running" } as G.Session, {
      ...connected,
      agentUnreachable: { since: now, reason: "x" },
    });
    expect(stallText(must(st), now).title).toMatch(/^The runner hasn't been able/);
  });
});

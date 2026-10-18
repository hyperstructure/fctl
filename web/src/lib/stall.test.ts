import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { clock } from "./format";
import { AWAY_GRACE_MS, since, stallLabel, stallOf, stallText } from "./stall";

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

  it("say the runner when the session has none", () => {
    const st = stallOf({ status: "running" } as G.Session, {
      ...connected,
      agentUnreachable: { since: now, reason: "x" },
    });
    expect(stallText(must(st), now).title).toMatch(/^The runner hasn't been able/);
  });
});

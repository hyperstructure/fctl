// When the agent's work isn't reaching the session page: its runner is away,
// the runner can't reach the agent in its sandbox, or it paused the sandbox
// for want of disk. The agent may well be working, or holds its place; its
// events wait in the sandbox until they get through. The page says so, and
// shows nothing as in progress meanwhile.
//
// Apart from those, the page says when the session's runner is short of
// room on its disk: the agent's work still comes through, but its writes
// fail if the disk fills up.

import { useEffect, useState } from "react";
import type * as G from "@/gen";
import { clock, memory, stamp } from "./format";
import type { LiveState } from "./live";

/**
 * How long a runner may be gone before the page says so: a host that
 * restarts, or a resumed session's new run, connects again within seconds.
 */
export const AWAY_GRACE_MS = 10_000;
/** A runner gone this long when the page learns of it is reported at once. */
const LONG_GONE_MS = 60_000;

/** Sessions that have a runner working for them. */
const ACTIVE = new Set<G.SessionStatus>([
  "provisioning",
  "preparing",
  "running",
  "idle",
  "paused",
  "stopping",
]);

export type Stall = {
  /** The runner's name. */
  runner?: string;
  status: G.SessionStatus;
  /** Since when the agent's work hasn't come through. */
  since: number;
  /** When the page may say so. */
  showFrom: number;
} & ({ kind: "away" } | { kind: "unreachable"; reason: string } | { kind: "paused"; reason: string });

type RunnerState = Pick<
  LiveState,
  "connection" | "runnerConnected" | "runnerAway" | "agentUnreachable" | "sandboxPaused"
>;

/** Why the agent's work isn't coming through right now, if it isn't. */
export function stallOf(session: G.Session | undefined, s: RunnerState): Stall | undefined {
  // The page's own connection is down: the header says it reconnects.
  if (!session || !ACTIVE.has(session.status) || s.connection !== "live") return undefined;
  const runner = session.runner?.name;
  if (!s.runnerConnected) {
    const away = s.runnerAway;
    if (!away) return undefined;
    const showFrom = Math.min(away.noticedAt + AWAY_GRACE_MS, away.lastSeenAt + LONG_GONE_MS);
    return { kind: "away", runner, status: session.status, since: away.lastSeenAt, showFrom };
  }
  // Its host paused the sandbox until it has room on its disk.
  const p = s.sandboxPaused;
  if (p)
    return { kind: "paused", runner, status: session.status, since: p.since, reason: p.reason, showFrom: 0 };
  const a = s.agentUnreachable;
  // The runner waited a few seconds before it said so.
  return (
    a && {
      kind: "unreachable",
      runner,
      status: session.status,
      since: a.since,
      reason: a.reason,
      showFrom: 0,
    }
  );
}

/** The stall the page shows, once it may. */
export function useStall(session: G.Session | undefined, s: RunnerState): Stall | undefined {
  const stall = stallOf(session, s);
  const showFrom = stall?.showFrom ?? 0;
  const [, rerender] = useState(0);
  useEffect(() => {
    const wait = showFrom - Date.now();
    if (wait <= 0) return;
    const t = setTimeout(() => rerender((n) => n + 1), wait);
    return () => clearTimeout(t);
  }, [showFrom]);
  return stall && Date.now() >= showFrom ? stall : undefined;
}

/** "since 02:26", "since yesterday at 23:10", "since 14 Oct 2026, 09:12". */
export function since(ms: number, now = Date.now()): string {
  const day = (t: number) => new Date(t).setHours(0, 0, 0, 0);
  const days = Math.round((day(now) - day(ms)) / 86_400_000);
  if (days <= 0) return `since ${clock(ms)}`;
  if (days === 1) return `since yesterday at ${clock(ms)}`;
  return `since ${stamp(ms)}`;
}

/** Statuses in which messages are taken (and wait for the agent). */
const TAKES_MESSAGES = new Set<G.SessionStatus>(["provisioning", "preparing", "running", "idle", "paused"]);

/** What the page says about a stall: what is wrong, and what happens next. */
export function stallText(st: Stall, now = Date.now()): { title: string; detail: string } {
  const who = st.runner ? `Runner ${st.runner}` : "The runner";
  const messages = TAKES_MESSAGES.has(st.status) ? ", and messages you send reach the agent then" : "";
  if (st.kind === "away") {
    return {
      title: `${who} hasn't been heard from ${since(st.since, now)}.`,
      detail: `The agent's work shows up here when it's back${messages}.`,
    };
  }
  const reason = st.reason.replace(/[.\s]+$/, "");
  if (st.kind === "paused") {
    return {
      title: `${who} paused this session's sandbox ${since(st.since, now)}: ${reason}.`,
      detail: `The sandbox keeps what the agent wrote. The agent carries on where it was once there is room${messages}.`,
    };
  }
  return {
    title: `${who} hasn't been able to reach the agent in its sandbox ${since(st.since, now)}: ${reason}.`,
    detail: `It keeps trying. The agent's work shows up here when it gets through${messages}.`,
  };
}

/** A few words for the session's header: "Runner not heard from since 02:26". */
export function stallLabel(st: Stall, now = Date.now()): string {
  if (st.kind === "away") return `Runner not heard from ${since(st.since, now)}`;
  if (st.kind === "paused") return `Sandbox paused ${since(st.since, now)}`;
  return `Agent unreachable ${since(st.since, now)}`;
}

/** The session's runner is short of room on its disk. */
export type DiskShort = G.RunnerDisk & {
  /** The runner's name. */
  runner?: string;
};

/**
 * How short of disk the session's runner is, while the session runs there. A sandbox the runner paused for it
 * says so in its stall.
 */
export function diskOf(
  session: G.Session | undefined,
  s: Pick<LiveState, "connection" | "runnerDisk">,
  stall?: Stall,
): DiskShort | undefined {
  if (!session || !ACTIVE.has(session.status) || s.connection !== "live" || !s.runnerDisk) return undefined;
  if (stall?.kind === "paused") return undefined;
  return { ...s.runnerDisk, runner: session.runner?.name };
}

/** What the page says about a runner short of disk, and what follows. */
export function diskText(d: DiskShort, now = Date.now()): { title: string; detail: string } {
  const who = d.runner ? `Runner ${d.runner}` : "The runner";
  if (d.room === "out") {
    return {
      title: `${who} has been nearly out of disk ${since(d.since, now)}: ${memory(d.free_mb)} free.`,
      detail:
        "The agent's writes are about to fail, until its host has removed stopped sandboxes or space is freed on it.",
    };
  }
  return {
    title: `${who} has been low on disk ${since(d.since, now)}: ${memory(d.free_mb)} free, below the ${memory(d.min_free_mb)} its host keeps free.`,
    detail:
      "No new sessions start on it until there is room, and its host removes stopped sandboxes to make some. If its disk fills up, the agent's writes fail.",
  };
}

/** A few words for the session's header: "Runner low on disk: 12.3 GiB free". */
export function diskLabel(d: DiskShort): string {
  return `Runner ${d.room === "out" ? "nearly out of" : "low on"} disk: ${memory(d.free_mb)} free`;
}

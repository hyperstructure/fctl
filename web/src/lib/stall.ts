// When the agent's work isn't reaching the session page: its runner is away,
// or the runner can't reach the agent in its sandbox. The agent may well be
// working; its events wait in the sandbox until they get through. The page
// says so, and shows nothing as in progress meanwhile.

import { useEffect, useState } from "react";
import type * as G from "@/gen";
import { clock, stamp } from "./format";
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
} & ({ kind: "away" } | { kind: "unreachable"; reason: string });

type RunnerState = Pick<LiveState, "connection" | "runnerConnected" | "runnerAway" | "agentUnreachable">;

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
  return {
    title: `${who} hasn't been able to reach the agent in its sandbox ${since(st.since, now)}: ${reason}.`,
    detail: `It keeps trying. The agent's work shows up here when it gets through${messages}.`,
  };
}

/** A few words for the session's header: "Runner not heard from since 02:26". */
export function stallLabel(st: Stall, now = Date.now()): string {
  return st.kind === "away"
    ? `Runner not heard from ${since(st.since, now)}`
    : `Agent unreachable ${since(st.since, now)}`;
}

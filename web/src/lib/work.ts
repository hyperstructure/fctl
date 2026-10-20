// A session's work so far, with the turn under way.

import type * as G from "@/gen";

/** Turns taken, the one under way included. */
export function turnsTaken(st: G.SessionStats): number {
  return st.turns + (st.turn_started_at != null ? 1 : 0);
}

/**
 * Milliseconds Claude worked by `now`: the turns that ended, and the one
 * under way so far. `until` is when the agent's work stopped coming
 * through, and the turn's time with it.
 */
export function agentTime(st: G.SessionStats, now: number, until?: number): number {
  const start = st.turn_started_at;
  if (start == null) return st.busy_ms;
  return st.busy_ms + Math.max(0, Math.min(now, until ?? now) - start);
}

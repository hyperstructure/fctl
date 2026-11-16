// A session's work so far, with the turn under way.

import type * as G from "@/gen";
import { num } from "./format";

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

/** How full Claude's context is, as the session page shows it. */
export type ContextUse = {
  /** Tokens in the context. */
  tokens: number;
  /** The most it holds, once Claude has said. */
  window?: number;
  /** Where Claude compacts the conversation on its own, below the window. */
  compactAt?: number;
  /**
   * Where a fresh session takes over from an autopilot's own, below where Claude compacts (or the
   * window): only for the session it works in now, when its handoff comes first.
   */
  handoffAt?: number;
  /**
   * The meter's parts, split where a fresh session takes over and where Claude compacts: the
   * tokens of the window each spans, and the share of them used. None without the window.
   */
  parts: { span: number; used: number }[];
  /** Nine tenths of the way to the first of those, or to the end of the window: the meter's amber. */
  close: boolean;
  /** Nine tenths of the way to where Claude compacts, or to the end of the window: its note's amber. */
  closeToCompact: boolean;
  /** Times Claude compacted the conversation. */
  compactions: number;
};

const share = (n: number, of: number) => Math.min(1, Math.max(0, n / of));

/** Nine tenths of the way to `limit`. */
const nearly = (tokens: number, limit?: number) => limit != null && tokens >= limit * 0.9;

/**
 * How full Claude's context is, once the session has made a call. `handoffAt` is where a fresh
 * session takes over, for the session an autopilot works in now: it counts when it comes before
 * Claude compacts, since past a compaction the context holds too little to reach it.
 */
export function contextUse(st: G.SessionStats, handoffAt?: number): ContextUse | undefined {
  const tokens = st.context_tokens;
  if (tokens == null) return undefined;
  const window = st.context_window || undefined;
  const compactAt = window && st.compact_at && st.compact_at < window ? st.compact_at : undefined;
  const handoff =
    handoffAt && handoffAt < (compactAt ?? window ?? Number.POSITIVE_INFINITY) ? handoffAt : undefined;
  // The meter's bounds, from empty to the window, each split between two.
  const bounds = window ? [0, handoff, compactAt, window].filter((b) => b != null) : [];
  const parts = bounds.slice(1).map((end, i) => {
    const start = bounds[i] ?? 0;
    return { span: end - start, used: share(tokens - start, end - start) };
  });
  const limit = compactAt ?? window;
  return {
    tokens,
    window,
    compactAt,
    handoffAt: handoff,
    parts,
    close: nearly(tokens, handoff ?? limit),
    closeToCompact: nearly(tokens, limit),
    compactions: st.compactions,
  };
}

/**
 * How much is left until a fresh session takes over from an autopilot's own: "137K left until a
 * fresh session takes over at 400K", or once past it, that one does at the next wake between turns.
 */
export function handoffNote(use: ContextUse): string | undefined {
  const at = use.handoffAt;
  if (at == null) return undefined;
  return use.tokens < at
    ? `${num(at - use.tokens)} left until a fresh session takes over at ${num(at)}`
    : `Past ${num(at)}: a fresh session takes over at the next wake between turns`;
}

/** What the meter doesn't say: "Compacts at 967K · compacted twice". */
export function contextNote(use: ContextUse): string {
  const n = use.compactions;
  const compacted = n === 0 ? "" : `compacted ${n === 1 ? "once" : n === 2 ? "twice" : `${n} times`}`;
  const note = [use.compactAt != null && `Compacts at ${num(use.compactAt)}`, compacted]
    .filter(Boolean)
    .join(" · ");
  return note.charAt(0).toUpperCase() + note.slice(1);
}

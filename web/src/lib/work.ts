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
   * The meter's parts, split where Claude compacts: the tokens of the
   * window each spans, and the share of them used. None without the window.
   */
  parts: { span: number; used: number }[];
  /** Nine tenths of the way to where Claude compacts, or to the end of the window. */
  close: boolean;
  /** Times Claude compacted the conversation. */
  compactions: number;
};

const share = (n: number, of: number) => Math.min(1, Math.max(0, n / of));

/** How full Claude's context is, once the session has made a call. */
export function contextUse(st: G.SessionStats): ContextUse | undefined {
  const tokens = st.context_tokens;
  if (tokens == null) return undefined;
  const window = st.context_window || undefined;
  const compactAt = window && st.compact_at && st.compact_at < window ? st.compact_at : undefined;
  const parts = !window
    ? []
    : compactAt
      ? [
          { span: compactAt, used: share(tokens, compactAt) },
          { span: window - compactAt, used: share(tokens - compactAt, window - compactAt) },
        ]
      : [{ span: window, used: share(tokens, window) }];
  const limit = compactAt ?? window;
  return {
    tokens,
    window,
    compactAt,
    parts,
    close: limit != null && tokens >= limit * 0.9,
    compactions: st.compactions,
  };
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

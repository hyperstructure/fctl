import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { agentTime, contextNote, contextUse, handoffNote, turnsTaken } from "./work";

const stats = (s: Partial<G.SessionStats>): G.SessionStats => ({
  turns: 0,
  input_tokens: 0,
  output_tokens: 0,
  cache_read_tokens: 0,
  tool_calls: 0,
  files_changed: 0,
  additions: 0,
  deletions: 0,
  busy_ms: 0,
  compactions: 0,
  ...s,
});

describe("work", () => {
  it("counts the turn under way", () => {
    expect(turnsTaken(stats({}))).toBe(0);
    expect(turnsTaken(stats({ turn_started_at: 1000 }))).toBe(1);
    expect(turnsTaken(stats({ turns: 2, turn_started_at: 1000 }))).toBe(3);
    expect(turnsTaken(stats({ turns: 2 }))).toBe(2);
  });

  it("adds the turn under way to the agent's time so far", () => {
    expect(agentTime(stats({ busy_ms: 5000 }), 99_000)).toBe(5000);
    expect(agentTime(stats({ busy_ms: 5000, turn_started_at: 90_000 }), 99_000)).toBe(14_000);
    // A clock behind the server's counts no time yet.
    expect(agentTime(stats({ busy_ms: 5000, turn_started_at: 90_000 }), 89_000)).toBe(5000);
  });

  it("stops where the agent's work stopped coming through", () => {
    expect(agentTime(stats({ turn_started_at: 90_000 }), 99_000, 95_000)).toBe(5000);
    expect(agentTime(stats({ turn_started_at: 90_000 }), 99_000, 80_000)).toBe(0);
  });

  it("shows nothing of the context before the first call", () => {
    expect(contextUse(stats({}))).toBeUndefined();
    expect(contextUse(stats({ context_window: 200_000, compact_at: 167_000 }))).toBeUndefined();
  });

  it("splits the window where Claude compacts", () => {
    const use = contextUse(stats({ context_tokens: 83_500, context_window: 200_000, compact_at: 167_000 }));
    expect(use).toMatchObject({ tokens: 83_500, window: 200_000, compactAt: 167_000, close: false });
    expect(use?.parts).toEqual([
      { span: 167_000, used: 0.5 },
      { span: 33_000, used: 0 },
    ]);
    // Past it, until the next call says what the compaction left.
    const past = contextUse(stats({ context_tokens: 183_500, context_window: 200_000, compact_at: 167_000 }));
    expect(past?.parts).toEqual([
      { span: 167_000, used: 1 },
      { span: 33_000, used: 0.5 },
    ]);
  });

  it("fills one part when Claude compacts only once the window is full", () => {
    const use = contextUse(stats({ context_tokens: 250_000, context_window: 1_000_000 }));
    expect(use?.compactAt).toBeUndefined();
    expect(use?.parts).toEqual([{ span: 1_000_000, used: 0.25 }]);
    // A threshold at or past the window is no threshold.
    expect(
      contextUse(stats({ context_tokens: 1, context_window: 200_000, compact_at: 200_000 }))?.parts,
    ).toHaveLength(1);
  });

  it("counts the tokens without the window from older agents", () => {
    const use = contextUse(stats({ context_tokens: 41_000 }));
    expect(use).toMatchObject({ tokens: 41_000, parts: [], close: false });
    expect(use?.window).toBeUndefined();
  });

  it("is close nine tenths of the way to where Claude compacts", () => {
    const at = (tokens: number, compact_at?: number) =>
      contextUse(stats({ context_tokens: tokens, context_window: 200_000, compact_at }))?.close;
    expect(at(150_299, 167_000)).toBe(false);
    expect(at(150_300, 167_000)).toBe(true);
    expect(at(180_000, 167_000)).toBe(true);
    // Or to the end of the window.
    expect(at(179_999)).toBe(false);
    expect(at(180_000)).toBe(true);
  });

  it("says where Claude compacts and how often it did", () => {
    const note = (s: Partial<G.SessionStats>) => {
      const use = contextUse(stats({ context_tokens: 1000, context_window: 200_000, ...s }));
      return use && contextNote(use);
    };
    expect(note({})).toBe("");
    expect(note({ compact_at: 167_000 })).toBe("Compacts at 167K");
    expect(note({ compact_at: 167_000, compactions: 1 })).toBe("Compacts at 167K · compacted once");
    expect(note({ compactions: 2 })).toBe("Compacted twice");
    expect(note({ compact_at: 167_000, compactions: 3 })).toBe("Compacts at 167K · compacted 3 times");
  });

  it("splits the window where an autopilot's fresh session takes over, before Claude compacts", () => {
    const st = stats({ context_tokens: 263_000, context_window: 600_000, compact_at: 567_000 });
    const use = contextUse(st, 400_000);
    expect(use).toMatchObject({ handoffAt: 400_000, compactAt: 567_000, close: false });
    expect(use?.parts.map((p) => p.span)).toEqual([400_000, 167_000, 33_000]);
    expect(use?.parts[0]?.used).toBeCloseTo(0.6575);
    expect(use?.parts.slice(1).map((p) => p.used)).toEqual([0, 0]);
    // A handoff at or past where Claude compacts never comes: the compaction leaves less.
    expect(contextUse(st, 567_000)?.handoffAt).toBeUndefined();
    expect(contextUse(st, 567_000)?.parts).toHaveLength(2);
    // Without a threshold, before the end of the window.
    const whole = stats({ context_tokens: 263_000, context_window: 1_000_000 });
    expect(contextUse(whole, 400_000)?.parts.map((p) => p.span)).toEqual([400_000, 600_000]);
    expect(contextUse(whole, 1_000_000)?.handoffAt).toBeUndefined();
    // Without the window, from older agents: no meter, but the handoff still counts.
    expect(contextUse(stats({ context_tokens: 263_000 }), 400_000)).toMatchObject({
      handoffAt: 400_000,
      parts: [],
    });
  });

  it("is close nine tenths of the way to the handoff, and its note only near the compaction", () => {
    const at = (tokens: number) =>
      contextUse(stats({ context_tokens: tokens, context_window: 600_000, compact_at: 567_000 }), 400_000);
    expect(at(359_999)).toMatchObject({ close: false, closeToCompact: false });
    expect(at(360_000)).toMatchObject({ close: true, closeToCompact: false });
    expect(at(510_300)).toMatchObject({ close: true, closeToCompact: true });
  });

  it("says how much is left until a fresh session takes over", () => {
    const note = (tokens: number, handoff?: number) => {
      const use = contextUse(
        stats({ context_tokens: tokens, context_window: 600_000, compact_at: 567_000 }),
        handoff,
      );
      return use && handoffNote(use);
    };
    expect(note(263_000)).toBeUndefined();
    expect(note(263_000, 400_000)).toBe("137K left until a fresh session takes over at 400K");
    expect(note(400_000, 400_000)).toBe(
      "Past 400K: a fresh session takes over at the next wake between turns",
    );
    expect(note(263_000, 600_000)).toBeUndefined();
  });
});

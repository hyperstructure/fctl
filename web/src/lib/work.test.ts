import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { agentTime, turnsTaken } from "./work";

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
});

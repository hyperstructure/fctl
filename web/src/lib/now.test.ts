import { describe, expect, it } from "vitest";
import { doing, nowOf } from "./now";
import type { Stall } from "./stall";
import { emptyLive, type Item, type LiveTurn } from "./transcript";

let seq = 0;
const base = (ts = 1000 * ++seq) => ({ key: `k${seq}`, seq, ts });
const ask = (): Item => ({ ...base(), kind: "user", text: "Go", source: "web" });
const call = (name: string, input: Record<string, unknown>, done = false): Item => ({
  ...base(),
  kind: "tool",
  id: `toolu_${seq}`,
  name,
  input: input as Record<string, never>,
  result: done ? { text: "", isError: false } : undefined,
  children: 0,
});
const thinking = (tokens?: number) => (tokens ? `Thinking · ${tokens}` : "Thinking");
const at = (items: Item[], live: LiveTurn = emptyLive(), more: Partial<Parameters<typeof nowOf>[0]> = {}) =>
  nowOf({ status: "running", items, live, thinking, cwd: "/home/agent/workspace", ...more });

describe("what the agent does now", () => {
  it("says what it writes, thinks or prepares, as it streams in", () => {
    const live = (type: "text" | "thinking" | "tool_use", extra = {}): LiveTurn => ({
      blocks: [{ index: 0, type, text: "", done: false, ...extra }],
      updatedAt: 5,
    });
    expect(at([ask()], live("text"))?.text).toBe("Writing");
    expect(at([ask()], live("thinking", { tokens: 1200 }))?.text).toBe("Thinking · 1200");
    expect(at([ask()], live("tool_use", { name: "Edit" }))).toMatchObject({
      kind: "preparing",
      text: "Preparing an edit",
    });
    expect(at([ask()], live("tool_use", { name: "mcp__linear__create_issue" }))?.text).toBe(
      "Preparing linear · create issue",
    );
  });

  it("names the call it waits on, with how long it has run", () => {
    const items = [ask(), call("Bash", { command: "npm test", description: "Run the tests" })];
    const id = (items[1] as Extract<Item, { kind: "tool" }>).id;
    const now = at(items, { blocks: [], tool: { id, name: "Bash", seconds: 42 }, updatedAt: 1 });
    expect(now).toMatchObject({ kind: "tool", text: "Run the tests", tool: "Bash", seconds: 42 });
    expect(at([ask(), call("Read", { file_path: "/home/agent/workspace/src/a.ts" })])).toMatchObject({
      text: "Reading",
      target: { kind: "path", text: "src/a.ts" },
    });
  });

  it("says it works between steps, since its turn started", () => {
    const items = [ask(), call("Read", { file_path: "/a" }, true)];
    expect(at(items)).toMatchObject({ kind: "working", text: "Working", since: items[0]?.ts });
  });

  it("says why its work isn't coming through, and how its sandbox is set up", () => {
    const stall: Stall = { kind: "away", runner: "kohlrabi", status: "running", since: 0, showFrom: 0 };
    expect(at([ask()], emptyLive(), { stall })?.kind).toBe("stalled");
    const setup: Item = {
      ...base(),
      kind: "setup",
      steps: [{ status: "provisioning", reason: "Pulling the sandbox image", ts: 1 }],
      logs: [],
      done: false,
      endTs: 1,
    };
    expect(at([ask(), setup], emptyLive(), { status: "provisioning" })?.text).toBe(
      "Pulling the sandbox image",
    );
  });

  it("says nothing while it waits for people, and what it waits for in the background", () => {
    expect(at([ask()], emptyLive(), { status: "idle" })).toBeUndefined();
    expect(at([ask()], emptyLive(), { status: "idle", reason: "Waiting for a message" })).toBeUndefined();
    expect(at([ask()], emptyLive(), { status: "idle", reason: "Waiting for 2 background tasks" })?.kind).toBe(
      "background",
    );
    expect(at([ask()], emptyLive(), { status: "stopped" })).toBeUndefined();
  });
});

describe("a call in progress, in words", () => {
  it("says what it does as it does it", () => {
    expect(doing("Bash", { command: "cargo test\n--quiet" })).toEqual({
      verb: "Running",
      target: { kind: "code", text: "cargo test" },
    });
    expect(doing("Edit", { file_path: "/home/agent/workspace/x.ts" }, "/home/agent/workspace")).toEqual({
      verb: "Editing",
      target: { kind: "path", text: "x.ts" },
    });
    expect(doing("Grep", { pattern: "sto-1" }).verb).toBe("Searching for");
    expect(doing("Glob", { pattern: "*.json" }).verb).toBe("Listing");
    expect(doing("Agent", { description: "Check the copy" })).toEqual({
      verb: "Delegating",
      target: { kind: "text", text: "Check the copy" },
    });
    expect(doing("mcp__claude_ai_Linear__get_issue", {}).verb).toBe("Using Linear · get issue");
  });
});

import { describe, expect, it } from "vitest";
import { doing, nowOf, taskLabel, waitingCount, waitingOf } from "./now";
import type { Stall } from "./stall";
import { type BackgroundTask, emptyLive, type Item, type LiveTurn, type Tool } from "./transcript";

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
const at = (items: Item[], live: LiveTurn = emptyLive(), more: Partial<Parameters<typeof nowOf>[0]> = {}) =>
  nowOf({ status: "running", items, live, cwd: "/home/agent/workspace", ...more });
const task = (id: string, more: Partial<BackgroundTask> = {}): BackgroundTask => ({
  id,
  description: "",
  ...base(),
  ...more,
});

describe("what the agent does now", () => {
  it("says what it writes, thinks or prepares, as it streams in", () => {
    const live = (type: "text" | "thinking" | "tool_use", extra = {}): LiveTurn => ({
      blocks: [{ index: 0, type, text: "", done: false, ...extra }],
      updatedAt: 5,
    });
    expect(at([ask()], live("text"))?.text).toBe("Writing");
    // A thought's tokens go on as it does: its step is the same thought.
    expect(at([ask()], live("thinking", { tokens: 1200 }))).toMatchObject({
      kind: "thinking",
      text: "Thinking",
      tokens: 1200,
    });
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

  it("names the one task it waits for, since it started, where the transcript knows it", () => {
    const one = task("b1", { description: "Start the dev stack", type: "local_bash" });
    const reason = "Waiting for a background task";
    const waiting = waitingOf([one], [], reason);
    expect(at([ask()], emptyLive(), { status: "idle", reason, waiting })).toEqual({
      kind: "background",
      text: "Waiting for",
      target: { kind: "text", text: "Start the dev stack" },
      since: one.ts,
    });
    // More than one, or one the transcript doesn't know: what its status says.
    const two = "Waiting for 2 background tasks";
    expect(
      at([ask()], emptyLive(), { status: "idle", reason: two, waiting: waitingOf([one], [], two) }),
    ).toEqual({
      kind: "background",
      text: two,
    });
  });
});

describe("what it waits for in the background", () => {
  it("counts the tasks its status says it waits for", () => {
    expect(waitingCount("Waiting for a background task")).toBe(1);
    expect(waitingCount("Waiting for 12 background tasks")).toBe(12);
    expect(waitingCount("Waiting for a message")).toBeUndefined();
    expect(waitingCount(undefined)).toBeUndefined();
  });

  it("lists those that run, the newest as many as its status counts, and how many more it misses", () => {
    const shell = call("Bash", { command: "npm run dev\n# and wait", run_in_background: true }) as Tool;
    const ended = task("a", { ended: "completed" });
    const old = task("b");
    const dev = task("c", { call: shell.key });
    const two = "Waiting for 2 background tasks";
    expect(waitingOf([ended, old, dev], [shell], two)).toEqual({
      tasks: [{ task: old }, { task: dev, call: shell }],
      more: 0,
    });
    // One the transcript didn't see start, before what loaded.
    expect(waitingOf([dev], [shell], two)?.more).toBe(1);
    // Fewer than the transcript thinks run: the newest.
    expect(
      waitingOf([old, dev], [shell], "Waiting for a background task")?.tasks.map((a) => a.task.id),
    ).toEqual(["c"]);
    // Not waiting for tasks.
    expect(waitingOf([dev], [shell], "Waiting for a message")).toBeUndefined();
  });

  it("still counts one that just ended, until the agent's next turn: its status says so a moment later", () => {
    const end: Item = { ...base(), kind: "turn", durationMs: 1, ok: true };
    const done = task("d", { ended: "completed", endSeq: end.seq + 1 });
    const one = "Waiting for a background task";
    expect(waitingOf([done], [end], one)).toEqual({ tasks: [{ task: done }], more: 0 });
    // Once a turn took up what came of it, it is done with.
    const next: Item = { ...base(), kind: "turn", durationMs: 1, ok: true };
    expect(waitingOf([{ ...done, endSeq: end.seq + 1 }], [end, next], one)).toEqual({ tasks: [], more: 1 });
  });

  it("finds a call in a subagent's work", () => {
    const inner = call("Bash", { command: "make watch" }) as Tool;
    const agent: Tool = { ...(call("Agent", {}) as Tool), children: 1, agent: { steps: [inner] } };
    expect(
      waitingOf([task("w", { call: inner.key })], [agent], "Waiting for a background task")?.tasks[0]?.call,
    ).toBe(inner);
  });

  it("names a task by its description, else its command, else its kind", () => {
    const shell = call("Bash", { command: "  tail -f log/dev.log\nexit" }) as Tool;
    expect(taskLabel({ task: task("1", { description: "Follow the log" }), call: shell })).toEqual({
      kind: "text",
      text: "Follow the log",
    });
    expect(taskLabel({ task: task("1"), call: shell })).toEqual({
      kind: "code",
      text: "tail -f log/dev.log",
    });
    expect(taskLabel({ task: task("1", { type: "monitor_mcp" }) })).toEqual({
      kind: "text",
      text: "a monitor",
    });
    expect(taskLabel({ task: task("1", { type: "something_new" }) })).toEqual({
      kind: "text",
      text: "a task",
    });
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

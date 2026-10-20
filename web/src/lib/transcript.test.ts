import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { applyStream, emptyLive, settleLive, TranscriptBuilder } from "./transcript";

let seq = 0;
const ev = (kind: string, data: unknown): G.SessionEvent =>
  ({ seq: ++seq, ts: 1_000 + seq, kind, data }) as unknown as G.SessionEvent;
const claude = (data: unknown) => ev("claude", data);
const assistant = (id: string, content: unknown[], parent: string | null = null) =>
  claude({ type: "assistant", message: { id, role: "assistant", content }, parent_tool_use_id: parent });
const toolResult = (id: string, text: string, isError = false) =>
  claude({
    type: "user",
    message: {
      role: "user",
      content: [{ type: "tool_result", tool_use_id: id, content: text, is_error: isError }],
    },
    parent_tool_use_id: null,
  });

describe("TranscriptBuilder", () => {
  it("groups setup steps and closes them when Claude starts", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      ev("status", { status: "queued" }),
      ev("status", { status: "provisioning", reason: "Starting a sandbox" }),
      ev("log", { source: "setup", level: "info", text: "npm ci" }),
      claude({ type: "system", subtype: "init", model: "claude-opus-5-5", claude_code_version: "2.1.302" }),
    ]);
    const setup = items.find((i) => i.kind === "setup");
    expect(setup?.kind === "setup" && setup.steps.map((s) => s.status)).toEqual(["queued", "provisioning"]);
    expect(setup?.kind === "setup" && setup.logs).toHaveLength(1);
    expect(setup?.kind === "setup" && setup.done).toBe(true);
    expect(b.model).toBe("claude-opus-5-5");
  });

  it("attaches tool results to their call and keeps other items identical", () => {
    const b = new TranscriptBuilder();
    const first = b.add([
      assistant("m1", [
        { type: "text", text: "Looking around." },
        { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } },
      ]),
    ]);
    const prose = first[0];
    const next = b.add([toolResult("t1", "README.md")]);
    expect(next).not.toBe(first);
    expect(next[0]).toBe(prose);
    const tool = next[1];
    expect(tool?.kind === "tool" && tool.result).toEqual({
      text: "README.md",
      isError: false,
      structured: undefined,
    });
  });

  it("merges consecutive text blocks of one message", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      assistant("m1", [
        { type: "text", text: "One." },
        { type: "text", text: "Two." },
      ]),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0]?.kind === "prose" && items[0].text).toBe("One.\n\nTwo.");
  });

  it("counts subagent tool calls on the parent tool", () => {
    const b = new TranscriptBuilder();
    b.add([assistant("m1", [{ type: "tool_use", id: "task", name: "Task", input: {} }])]);
    const items = b.add([
      assistant("s1", [{ type: "tool_use", id: "x", name: "Grep", input: {} }], "task"),
      assistant("s2", [{ type: "tool_use", id: "y", name: "Read", input: {} }], "task"),
    ]);
    expect(items).toHaveLength(1);
    expect(items[0]?.kind === "tool" && [items[0].children, items[0].childLast]).toEqual([2, "Read"]);
  });

  it("shows a state when a branch changes, and keeps the newest", () => {
    const b = new TranscriptBuilder();
    const git = (head: string, ahead: number, files: number, pushed = false): G.GitState => ({
      head,
      ahead,
      behind: 0,
      files: Array.from({ length: files }, (_, i) => ({ path: `f${i}`, status: "modified" })),
      uncommitted: 0,
      pushed,
    });
    // Edits while Claude works, then a commit, then the end of the turn.
    const items = b.add([
      ev("git", git("base", 0, 1)),
      ev("git", git("base", 0, 2)),
      ev("git", git("c1", 1, 2)),
      claude({ type: "result", subtype: "success", is_error: false, duration_ms: 5, result: "Done" }),
      ev("git", git("c1", 1, 3)),
      ev("git", git("c1", 1, 3, true)),
    ]);
    expect(items.map((i) => i.kind)).toEqual(["git", "turn", "git"]);
    expect(items[0]?.kind === "git" && [items[0].git.ahead, items[0].repos]).toEqual([1, [undefined]]);
    expect(items[2]?.kind === "git" && items[2].git.pushed).toBe(true);
    expect(b.latestGit?.files).toHaveLength(3);
  });

  it("ignores events it has already seen", () => {
    const b = new TranscriptBuilder();
    const e = ev("user_message", { id: "dlv_1", text: "hi", source: "web" });
    b.add([e]);
    expect(b.add([e])).toHaveLength(1);
  });

  it("turns rejected rate limits into items with millisecond reset times", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      claude({
        type: "rate_limit_event",
        rate_limit_info: { status: "rejected", resetsAt: 1_900_000_000, rateLimitType: "five_hour" },
      }),
    ]);
    expect(items[0]?.kind === "limit" && items[0].resetsAt).toBe(1_900_000_000_000);
  });
});

describe("live streaming", () => {
  it("accumulates deltas per block and settles when the message is stored", () => {
    let live = emptyLive();
    live = applyStream(live, {
      type: "stream_event",
      event: { type: "message_start", message: { id: "m1" } },
    });
    live = applyStream(live, {
      type: "stream_event",
      event: { type: "content_block_start", index: 0, content_block: { type: "text" } },
    });
    for (const text of ["Hel", "lo"]) {
      live = applyStream(live, {
        type: "stream_event",
        event: { type: "content_block_delta", index: 0, delta: { type: "text_delta", text } },
      });
    }
    expect(live.blocks[0]?.text).toBe("Hello");
    const unchanged = applyStream(live, { type: "stream_event", event: { type: "ping" } });
    expect(unchanged).toBe(live);
    live = applyStream(live, { type: "stream_event", event: { type: "content_block_stop", index: 0 } });
    const settled = settleLive(live, [assistant("m1", [{ type: "text", text: "Hello" }])]);
    expect(settled.blocks).toHaveLength(0);
  });

  it("ignores subagent streams", () => {
    const live = emptyLive();
    expect(applyStream(live, { type: "stream_event", parent_tool_use_id: "t1", event: {} })).toBe(live);
  });
});

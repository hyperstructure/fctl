import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  applyStream,
  emptyLive,
  type Item,
  settleLive,
  subagentReport,
  subagentRuns,
  type Tool,
  TranscriptBuilder,
} from "./transcript";

let seq = 0;
const ev = (kind: string, data: unknown): G.SessionEvent =>
  ({ seq: ++seq, ts: 1_000 + seq, kind, data }) as unknown as G.SessionEvent;
const claude = (data: unknown) => ev("claude", data);
const assistant = (id: string, content: unknown[], parent: string | null = null) =>
  claude({ type: "assistant", message: { id, role: "assistant", content }, parent_tool_use_id: parent });
const toolResult = (id: string, text: string, isError = false, parent: string | null = null) =>
  claude({
    type: "user",
    message: {
      role: "user",
      content: [{ type: "tool_result", tool_use_id: id, content: text, is_error: isError }],
    },
    parent_tool_use_id: parent,
  });
/** Claude Code's word on a task (`task_started`, `task_notification`), as 2.1.307 sends it. */
const task = (subtype: string, fields: Record<string, unknown>) =>
  claude({ type: "system", subtype, session_id: "s", uuid: `u${seq}`, ...fields });
/** A subagent's start: its call, and Claude Code saying it started. */
const delegate = (id: string, background = false) => [
  assistant(`m-${id}`, [
    {
      type: "tool_use",
      id,
      name: "Agent",
      input: {
        description: "Count lines in cn.ts",
        prompt: "Read web/src/lib/cn.ts and count its lines.",
        subagent_type: "general-purpose",
        ...(background ? { run_in_background: true } : {}),
      },
    },
  ]),
  task("task_started", {
    task_id: `task-${id}`,
    tool_use_id: id,
    description: "Count lines in cn.ts",
    task_type: "local_agent",
    is_backgrounded: background,
  }),
  // Its prompt, which the call has already.
  claude({
    type: "user",
    message: {
      role: "user",
      content: [{ type: "text", text: "Read web/src/lib/cn.ts and count its lines." }],
    },
    parent_tool_use_id: id,
  }),
];
const ended = (id: string, summary: string, status = "completed") =>
  task("task_notification", {
    task_id: `task-${id}`,
    tool_use_id: id,
    status,
    summary,
    output_file: "/tmp/tasks/a1.output",
    usage: { duration_ms: 12_107, tool_uses: 2, total_tokens: 21_365 },
  });
const call = (items: Item[], i = 0) => items[i] as Tool;

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
    const result = toolResult("t1", "README.md");
    const next = b.add([result]);
    expect(next).not.toBe(first);
    expect(next[0]).toBe(prose);
    const tool = next[1];
    // With when it returned, which says how long it took.
    expect(tool?.kind === "tool" && tool.result).toEqual({
      text: "README.md",
      isError: false,
      structured: undefined,
      ts: result.ts,
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

  it("keeps a subagent's steps under the call that started it", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      ...delegate("agent"),
      assistant(
        "s1",
        [{ type: "tool_use", id: "r1", name: "Read", input: { file_path: "web/src/lib/cn.ts" } }],
        "agent",
      ),
      toolResult("r1", '1\timport { clsx } from "clsx";', false, "agent"),
      assistant("s2", [{ type: "text", text: "It has three lines." }], "agent"),
      assistant(
        "s2",
        [{ type: "tool_use", id: "b1", name: "Bash", input: { command: "wc -l cn.ts" } }],
        "agent",
      ),
      toolResult("b1", "3 cn.ts", false, "agent"),
      ended("agent", "cn.ts has 3 lines."),
      claude({
        type: "user",
        message: {
          role: "user",
          content: [
            {
              type: "tool_result",
              tool_use_id: "agent",
              content: [
                { type: "text", text: "[Subagent hand-back] The report follows:\n  cn.ts has 3 lines." },
              ],
            },
          ],
        },
        parent_tool_use_id: null,
        tool_use_result: { status: "completed", content: [{ type: "text", text: "cn.ts has 3 lines." }] },
      }),
    ]);
    expect(items).toHaveLength(1);
    const t = call(items);
    expect(t.agent?.steps.map((s) => s.kind)).toEqual(["tool", "prose", "tool"]);
    expect(t.agent?.steps.map((s) => (s.kind === "tool" ? s.result?.text : s.text))).toEqual([
      '1\timport { clsx } from "clsx";',
      "It has three lines.",
      "3 cn.ts",
    ]);
    expect([t.children, t.childLast]).toEqual([2, "Bash"]);
    expect([t.agent?.background, t.agent?.ended, t.agent?.durationMs]).toEqual([false, "completed", 12_107]);
    // The report alone, without Claude Code's notes for Claude around it.
    expect(subagentReport(t)).toBe("cn.ts has 3 lines.");
    expect(subagentRuns(t, true, true)).toBe(false);
  });

  it("shows a subagent working until its call returns", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      ...delegate("agent"),
      assistant("s1", [{ type: "tool_use", id: "r1", name: "Read", input: {} }], "agent"),
    ]);
    const t = call(items);
    expect(subagentRuns(t, true, true)).toBe(true);
    // A turn that ended, or a session that stopped, takes its subagents with it.
    expect(subagentRuns(t, false, true)).toBe(false);
    const stopped = b.add([ev("status", { status: "stopped", reason: "Stopped by Ada" })]);
    expect(call(stopped).agent?.ended).toBe("stopped");
    expect(subagentReport(call(stopped))).toBeUndefined();
  });

  it("follows a subagent in the background after its call returned", () => {
    const b = new TranscriptBuilder();
    b.add([
      ...delegate("agent", true),
      claude({
        type: "user",
        message: {
          role: "user",
          content: [
            { type: "tool_result", tool_use_id: "agent", content: "Async agent launched successfully." },
          ],
        },
        parent_tool_use_id: null,
        tool_use_result: { isAsync: true, agentId: "a1", status: "async_launched" },
      }),
      claude({ type: "result", subtype: "success", is_error: false, duration_ms: 5, result: "Started it" }),
      assistant("s1", [{ type: "tool_use", id: "g1", name: "Grep", input: { pattern: "export" } }], "agent"),
    ]);
    let t = call(b.items);
    expect(t.agent?.background).toBe(true);
    // It works while the session waits for it, not once the session stopped.
    expect([subagentRuns(t, false, true), subagentRuns(t, false, false)]).toEqual([true, false]);
    expect(subagentReport(t)).toBeUndefined();
    b.add([
      toolResult("g1", "10:export function stopPinchZoom", false, "agent"),
      assistant("s2", [{ type: "text", text: "It has one export." }], "agent"),
      ended("agent", "It has one export."),
    ]);
    t = call(b.items);
    expect(t.agent?.steps.map((s) => s.kind)).toEqual(["tool", "prose"]);
    expect(subagentRuns(t, false, true)).toBe(false);
    expect(subagentReport(t)).toBe("It has one export.");
  });

  it("ends subagents in the background when Claude Code restarts or the session stops", () => {
    const restarted = new TranscriptBuilder();
    restarted.add([
      ...delegate("agent", true),
      ev("notice", { level: "warn", code: "claude_restarted", message: "Claude exited unexpectedly" }),
    ]);
    expect(call(restarted.items).agent?.ended).toBe("stopped");
    const failed = new TranscriptBuilder();
    failed.add([
      ...delegate("agent", true),
      ev("status", { status: "failed", reason: "The runner went away" }),
    ]);
    expect(subagentRuns(call(failed.items), false, true)).toBe(false);
    // A subagent that ended keeps how.
    const done = new TranscriptBuilder();
    done.add([...delegate("agent", true), ended("agent", "Done."), ev("status", { status: "stopped" })]);
    expect(call(done.items).agent?.ended).toBe("completed");
  });

  it("says a subagent failed without taking what it said for its report", () => {
    const b = new TranscriptBuilder();
    b.add([...delegate("agent", true), ended("agent", "API Error: 500", "failed")]);
    expect(call(b.items).agent?.ended).toBe("failed");
    expect(subagentReport(call(b.items))).toBeUndefined();
  });

  it("puts the steps of a subagent's own subagent under its call", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      ...delegate("outer"),
      assistant(
        "s1",
        [{ type: "tool_use", id: "inner", name: "Agent", input: { description: "Look deeper" } }],
        "outer",
      ),
      assistant("s2", [{ type: "tool_use", id: "r1", name: "Read", input: {} }], "inner"),
      toolResult("r1", "deep", false, "inner"),
      toolResult("inner", "Found it deep down.", false, "outer"),
    ]);
    expect(items).toHaveLength(1);
    const inner = call(items).agent?.steps[0] as Tool;
    expect([inner.name, inner.result?.text, inner.children]).toEqual(["Agent", "Found it deep down.", 1]);
    expect((inner.agent?.steps[0] as Tool | undefined)?.result?.text).toBe("deep");
    expect(subagentReport(inner)).toBe("Found it deep down.");
  });

  it("leaves out the work of a subagent whose call isn't loaded, and other tasks", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      assistant("s1", [{ type: "tool_use", id: "r1", name: "Read", input: {} }], "earlier"),
      toolResult("r1", "text", false, "earlier"),
      // A shell command in the background is a task, not a subagent.
      assistant("m1", [{ type: "tool_use", id: "bg", name: "Bash", input: { command: "sleep 9" } }]),
      task("task_started", {
        task_id: "b1",
        tool_use_id: "bg",
        task_type: "local_bash",
        is_backgrounded: true,
      }),
      task("task_notification", { task_id: "b1", tool_use_id: "bg", status: "completed", summary: "done" }),
    ]);
    expect(items).toHaveLength(1);
    expect(call(items).agent).toBeUndefined();
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

  it("leaves out events it can't read", () => {
    const b = new TranscriptBuilder();
    const items = b.add([
      claude({ type: "assistant" }),
      claude({
        type: "assistant",
        message: { id: "m0", content: [{ type: "text", text: 7 }] },
        parent_tool_use_id: null,
      }),
      claude({ type: "user", message: null, parent_tool_use_id: null }),
      claude({ type: "rate_limit_event" }),
      ev("git", null),
      assistant("m1", [{ type: "text", text: "Still here." }]),
    ]);
    expect(items.map((i) => i.kind)).toEqual(["prose"]);
  });

  it("keeps what fctl's messages are about: a pull request's follow-up, a review's findings", () => {
    const repo = { owner: "acme", name: "web" };
    const url = "https://github.com/acme/web/pull/12";
    const followUp: G.FollowUp = { repo, number: 12, url, events: [{ kind: "closed" }] };
    const review: G.PassedOnReview = { repo, number: 12, url, sha: "3f2a1c9", session: "ses_r", findings: [] };
    const items = new TranscriptBuilder().add([
      ev("user_message", { id: "dlv_1", text: "On GitHub, …", source: "system", follow_up: followUp }),
      ev("user_message", { id: "dlv_2", text: "fctl's review agent …", source: "system", review }),
    ]);
    expect(items.map((i) => i.kind === "user" && [i.followUp, i.review])).toEqual([
      [followUp, undefined],
      [undefined, review],
    ]);
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

  it("says nothing of usage that Claude only reports or warns about", () => {
    const b = new TranscriptBuilder();
    const windows = {
      five_hour: { resetsAt: 1_792_675_200, utilization: 0.12 },
      seven_day: { resetsAt: 1_793_192_400, utilization: 0.25 },
    };
    const items = b.add([
      claude({ type: "rate_limit_event", rate_limit_info: { status: "allowed", unifiedWindows: windows } }),
      // As Claude Code 2.1 sent it with a quarter of the week used in its first day.
      claude({
        type: "rate_limit_event",
        rate_limit_info: {
          status: "allowed_warning",
          rateLimitType: "seven_day",
          resetsAt: 1_793_192_400,
          utilization: 0.25,
          isUsingOverage: false,
          overageStatus: "rejected",
          overageDisabledReason: "org_level_disabled",
          unifiedWindows: windows,
        },
      }),
    ]);
    expect(items).toEqual([]);
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

  it("counts the tokens of the thought streaming in, as Claude Code says how far it got", () => {
    const progress = (estimated_tokens: number) =>
      ({ type: "system", subtype: "thinking_tokens", estimated_tokens }) as const;
    const thought = (index: number) =>
      ({
        type: "stream_event",
        event: { type: "content_block_start", index, content_block: { type: "thinking" } },
      }) as const;
    let live = applyStream(emptyLive(), {
      type: "stream_event",
      event: { type: "message_start", message: { id: "m1" } },
    });
    // Before a thought starts, there is nothing to count.
    expect(applyStream(live, progress(50))).toBe(live);
    live = applyStream(live, thought(0));
    live = applyStream(live, progress(50));
    live = applyStream(live, progress(1200));
    expect(live.blocks[0]?.tokens).toBe(1200);
    expect(applyStream(live, progress(1200))).toBe(live);
    // Each thought counts from the start.
    live = applyStream(live, { type: "stream_event", event: { type: "content_block_stop", index: 0 } });
    live = applyStream(live, thought(1));
    expect(live.blocks.map((b) => b.tokens)).toEqual([1200, undefined]);
    live = applyStream(live, progress(5));
    expect(live.blocks.map((b) => b.tokens)).toEqual([1200, 5]);
    // The stored thoughts take their place.
    const settled = settleLive(live, [assistant("m1", [{ type: "thinking", thinking: "" }])]);
    expect(settled.blocks.map((b) => b.tokens)).toEqual([5]);
  });

  it("keeps the agent's tool running through a subagent's tool results", () => {
    const live = applyStream(emptyLive(), {
      type: "tool_progress",
      tool_use_id: "t1",
      tool_name: "Bash",
      elapsed_time_seconds: 40,
    });
    expect(settleLive(live, [toolResult("r1", "text", false, "agent")]).tool?.seconds).toBe(40);
    expect(settleLive(live, [toolResult("t1", "built")]).tool).toBeUndefined();
  });
});

import { expect, describe as group, it } from "vitest";
import type { Json } from "./claude-types";
import { aside, describe, folds, lastRun, parts, runTook, shownPath, summary, took, toolName } from "./runs";
import type { Item, Tool } from "./transcript";

let seq = 0;
const call = (
  name: string,
  input: Record<string, Json> = {},
  result?: { ts: number; error?: boolean },
): Tool => ({
  kind: "tool",
  key: `t${++seq}`,
  seq,
  ts: seq * 1000,
  id: `toolu_${seq}`,
  name,
  input,
  result: result && { text: "", isError: !!result.error, ts: result.ts },
  children: 0,
});
const thought = (): Item => ({ kind: "thinking", key: `th${++seq}`, seq, ts: seq * 1000, text: "Hmm." });
const prose = (): Item => ({ kind: "prose", key: `p${++seq}`, seq, ts: seq * 1000, text: "Done." });
const bash = (command = "ls") => call("Bash", { command });
const read = (file_path: string) => call("Read", { file_path });

group("runs", () => {
  it("fold tool calls and the thoughts among them, between the things the agent says", () => {
    const items = [prose(), bash(), thought(), read("a.ts"), prose(), bash()];
    const ps = parts(items);
    expect(ps.map((p) => p.kind)).toEqual(["item", "run", "item", "run"]);
    const run = ps[1];
    expect(run?.kind === "run" && run.steps).toEqual(items.slice(1, 4));
    // It keeps its first step's key as it grows, and says where it starts.
    expect(run?.key).toBe(items[1]?.key);
    expect(run?.index).toBe(1);
  });

  it("go on past what comes aside of the agent's work, which goes after them", () => {
    const git: Item = {
      kind: "git",
      key: `g${++seq}`,
      seq,
      ts: 0,
      git: { branch: "fctl/1-x", repos: [] } as unknown as Extract<Item, { kind: "git" }>["git"],
      repos: [undefined],
    };
    const crashed: Item = {
      kind: "notice",
      key: `n${++seq}`,
      seq,
      ts: 0,
      notice: { level: "error", code: "claude_crashed", message: "Claude Code crashed" },
    };
    const commit = bash("git commit");
    const push = bash("git push");
    const ps = parts([commit, git, push]);
    expect(ps.map((p) => (p.kind === "run" ? p.steps.length : p.item.kind))).toEqual([2, "git"]);
    expect(lastRun(ps)?.steps).toEqual([commit, push]);
    expect(aside(git)).toBe(true);
    // What says something went wrong ends a run, as what the agent says does.
    expect(aside(crashed)).toBe(false);
    const broken = parts([commit, crashed, push]);
    expect(broken.map((p) => p.kind)).toEqual(["run", "item", "run"]);
    expect(lastRun(parts([commit, prose()]))).toBeUndefined();
  });

  it("leave subagents and the plan on lines of their own", () => {
    const agent = call("Agent", { description: "Audit the webhooks" });
    const plan = call("TodoWrite", { todos: [] });
    const ps = parts([bash(), agent, bash(), plan, bash()]);
    expect(ps.map((p) => p.kind)).toEqual(["run", "item", "run", "item", "run"]);
    expect(folds(agent)).toBe(false);
    expect(folds(plan)).toBe(false);
  });
});

group("a run's line", () => {
  it("says what its calls did, kind by kind in the order they came", () => {
    expect(summary([bash(), read("a.ts"), bash(), call("Grep", { pattern: "TODO" })])).toBe(
      "Ran 2 commands, read 1 file, searched for 1 pattern",
    );
    expect(
      summary([read("a.ts"), call("Edit", { file_path: "a.ts" }), call("Write", { file_path: "b.ts" })]),
    ).toBe("Read 1 file, edited 1 file, wrote 1 file");
  });

  it("counts a file once however often it was read or edited", () => {
    const edits = [call("Edit", { file_path: "a.ts" }), call("MultiEdit", { file_path: "a.ts" })];
    expect(summary([read("a.ts"), read("a.ts"), read("b.ts"), ...edits])).toBe("Read 2 files, edited 1 file");
  });

  it("says what is still going on as it happens", () => {
    expect(summary([bash(), bash(), read("a.ts")], true)).toBe("Running 2 commands, reading 1 file");
  });

  it("names searches of the web and the tools it doesn't know", () => {
    expect(summary([call("WebSearch", { query: "a" })])).toBe("Searched the web");
    expect(summary([call("WebSearch", { query: "a" }), call("WebSearch", { query: "b" })])).toBe(
      "Searched the web 2 times",
    );
    expect(summary([call("mcp__linear__create_issue"), call("Skill"), call("WebFetch", { url: "x" })])).toBe(
      "Used 2 tools, fetched 1 page",
    );
    expect(summary([call("Glob", { pattern: "*.md" }), call("Grep", { pattern: "x" })])).toBe(
      "Searched for 2 patterns",
    );
  });
});

group("a tool call in words", () => {
  it("says what a command is for when Claude said so, else what it ran", () => {
    expect(describe("Bash", { command: "git log -5", description: "Show the recent commits" })).toEqual({
      verb: "Show the recent commits",
    });
    expect(describe("Bash", { command: "git log -5\ngit status" })).toEqual({
      verb: "Ran",
      target: { kind: "code", text: "git log -5" },
    });
    expect(describe("Bash", {})).toEqual({ verb: "Ran a command" });
  });

  it("shows files from the root of the repository", () => {
    const cwd = "/home/agent/workspace";
    expect(describe("Read", { file_path: "/home/agent/workspace/web/src/lib/cn.ts" }, cwd).target).toEqual({
      kind: "path",
      text: "web/src/lib/cn.ts",
    });
    expect(describe("Edit", { file_path: "/srv/app/main.rs" }, "/srv/app").target?.text).toBe("main.rs");
  });

  it("reads a search of the web as words and an MCP tool by its server", () => {
    expect(describe("WebSearch", { query: "workerd sockets" }).target).toEqual({
      kind: "text",
      text: "workerd sockets",
    });
    expect(describe("mcp__claude_ai_Linear__create_issue", {})).toEqual({ verb: "Linear · create issue" });
    expect(toolName("Skill")).toBe("Skill");
  });

  it("takes what isn't text in its input for nothing", () => {
    expect(describe("Read", { file_path: 42 })).toEqual({ verb: "Read", target: undefined });
    expect(describe("Grep", { pattern: ["x"] })).toEqual({ verb: "Searched for", target: undefined });
  });
});

group("paths", () => {
  it("start at the repository's root, else at the home directory", () => {
    expect(shownPath("/home/agent/workspace/README.md")).toBe("README.md");
    expect(shownPath("/home/agent/repos/hyperstructure/design/src/tokens.css")).toBe(
      "~/repos/hyperstructure/design/src/tokens.css",
    );
    expect(shownPath("/tmp/out.log")).toBe("/tmp/out.log");
    expect(shownPath("notes.md")).toBe("notes.md");
    // Claude Code's working directory, wherever it is.
    expect(shownPath("/data/s1/workspace/notes.md", "/data/s1/workspace/")).toBe("notes.md");
  });
});

group("how long", () => {
  it("a call took, once it returned, and a run, to its last result", () => {
    const done = call("Bash", { command: "cargo test" }, { ts: 0 });
    done.ts = 1_000;
    if (done.result) done.result.ts = 13_500;
    expect(took(done)).toBe(12_500);
    expect(took(bash())).toBeUndefined();
    const first = { ...bash(), ts: 500 };
    expect(runTook([first, done])).toBe(13_000);
  });
});

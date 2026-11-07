import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { parts } from "./runs";
import type { Item } from "./transcript";
import { factsOf, firstLine, lastTs, laterBy, turnAt, turnsOf, turnTitle, turnTook } from "./turns";

let seq = 0;
const base = (ts = seq * 1000) => ({ key: `k${++seq}`, seq, ts });
const ask = (text: string, ts?: number): Item => ({
  ...base(ts),
  kind: "user",
  text,
  source: "web",
  author: { id: "usr_ada", name: "Ada" },
});
const prose = (text: string): Item => ({ ...base(), kind: "prose", text });
const call = (
  name: string,
  input: Record<string, unknown> = {},
  result: { isError?: boolean } | null = {},
): Item => ({
  ...base(),
  kind: "tool",
  id: `toolu_${seq}`,
  name,
  input: input as Record<string, never>,
  result: result ? { text: "", isError: !!result.isError } : undefined,
  children: 0,
});
const result = (durationMs: number, ok = true): Item => ({ ...base(), kind: "turn", durationMs, ok });
const status = (s: G.SessionStatus): Item => ({ ...base(), kind: "status", status: s });
const git = (): Item => ({
  ...base(),
  kind: "git",
  git: { ahead: 1, behind: 0, files: [], uncommitted: 0, pushed: true },
  repos: [undefined],
});

const turns = (items: Item[]) => turnsOf(parts(items));

describe("turns", () => {
  it("start with each message once the agent ended the turn before", () => {
    const t = turns([
      ask("Fix the login test"),
      call("Bash", { command: "npm test" }),
      prose("Fixed."),
      result(4000),
      ask("Now the signup test"),
      prose("Done."),
      result(2000),
    ]);
    expect(t.map((x) => x.ask?.text)).toEqual(["Fix the login test", "Now the signup test"]);
    expect(t[0]?.body.map((p) => (p.kind === "run" ? "run" : p.item.kind))).toEqual(["run", "prose"]);
    expect(t[0]?.end).toMatchObject({ kind: "result", ok: true, durationMs: 4000 });
    expect(t[1]?.end).toMatchObject({ kind: "result", durationMs: 2000 });
  });

  it("keep a message that comes while the agent works in the turn it works on", () => {
    const t = turns([
      ask("Translate the shop"),
      prose("On it."),
      ask("Danish too"),
      prose("Both."),
      result(9000),
    ]);
    expect(t).toHaveLength(1);
    expect(t[0]?.body.map((p) => (p.kind === "item" ? p.item.kind : "run"))).toEqual([
      "prose",
      "user",
      "prose",
    ]);
  });

  it("keep what comes after the end with the turn it follows, and the agent's own work starts a turn", () => {
    const t = turns([
      ask("Ship it"),
      prose("Shipped."),
      result(1000),
      git(),
      status("stopped"),
      prose("The build ended."),
    ]);
    expect(t).toHaveLength(2);
    expect(t[0]?.after.map((p) => (p.kind === "item" ? p.item.kind : "run"))).toEqual(["git", "status"]);
    expect(t[1]?.ask).toBeUndefined();
    expect(t[1]?.body).toHaveLength(1);
  });

  it("end with the session when it stops or fails before the agent ended the turn, which says so in a line", () => {
    const t = turns([ask("Migrate"), prose("Working on it."), status("failed")]);
    expect(t[0]?.end?.kind).toBe("failed");
    expect(t[0]?.after.map((p) => (p.kind === "item" ? p.item.kind : "run"))).toEqual(["status"]);
    // A message then starts the next turn.
    expect(turns([ask("Migrate"), status("stopped"), ask("Go on")])).toHaveLength(2);
  });

  it("go on while the agent works, and start without an ask where earlier activity isn't loaded", () => {
    const t = turns([
      prose("…so the test passes."),
      result(1000),
      ask("Thanks"),
      call("Read", { file_path: "/a" }),
    ]);
    expect(t[0]?.ask).toBeUndefined();
    expect(t[0]?.end?.kind).toBe("result");
    expect(t[1]?.end).toBeUndefined();
  });

  it("keep their keys as they grow", () => {
    const items = [ask("Go"), prose("One")];
    const before = turns(items);
    const after = turns([...items, call("Bash"), result(1)]);
    expect(after[0]?.key).toBe(before[0]?.key);
  });
});

describe("a turn's facts", () => {
  it("count its calls and failures, and the files its edits changed, once each", () => {
    const [t] = turns([
      ask("Edit"),
      call("Edit", {
        file_path: "/home/agent/workspace/src/a.ts",
        old_string: "a\nb",
        new_string: "a\nc\nd",
      }),
      call("Bash", { command: "false" }, { isError: true }),
      call("Edit", { file_path: "/home/agent/workspace/src/a.ts", old_string: "x", new_string: "y" }),
      call("Write", { file_path: "/home/agent/workspace/README.md", content: "one\ntwo\n" }),
      prose("Done: a.ts and the README."),
      result(1),
    ]);
    if (!t) throw new Error("no turn");
    const f = factsOf(t, "/home/agent/workspace");
    expect(f.calls).toBe(4);
    expect(f.failed).toBe(1);
    expect(f.files.map((x) => [x.path, x.lines.added, x.lines.removed])).toEqual([
      ["src/a.ts", 3, 2],
      ["README.md", 2, 0],
    ]);
    expect(f.lines).toEqual({ added: 5, removed: 2 });
    expect(f.answer).toBe("Done: a.ts and the README.");
  });

  it("leave out edits that failed or haven't returned", () => {
    const [t] = turns([
      ask("Edit"),
      call("Edit", { file_path: "/w/a.ts", old_string: "a", new_string: "b" }, { isError: true }),
      call("Edit", { file_path: "/w/b.ts", old_string: "a", new_string: "b" }, null),
    ]);
    if (!t) throw new Error("no turn");
    expect(factsOf(t).files).toEqual([]);
  });

  it("take a turn's time from its result, or from its ask until now", () => {
    const [done] = turns([ask("Go", 1000), prose("Done"), result(4200)]);
    const [going] = turns([ask("Go", 1000), prose("Doing")]);
    if (!done || !going) throw new Error("no turn");
    expect(turnTook(done, factsOf(done), 99_000)).toBe(4200);
    expect(turnTook(going, factsOf(going), 61_000)).toBe(60_000);
  });
});

describe("turnAt", () => {
  it("finds the last turn that starts above a point", () => {
    expect(turnAt([0, 500, 1200], -10)).toBe(-1);
    expect(turnAt([0, 500, 1200], 0)).toBe(0);
    expect(turnAt([0, 500, 1200], 800)).toBe(1);
    expect(turnAt([0, 500, 1200], 5000)).toBe(2);
    expect(turnAt([], 10)).toBe(-1);
  });
});

describe("firstLine", () => {
  it("says a message's first line without Markdown's marks, cut short", () => {
    expect(firstLine("\n## Fix the *login* test\n\nIt fails on CI.")).toBe("Fix the login test");
    expect(firstLine("- `npm test` fails")).toBe("npm test fails");
    expect(firstLine("a".repeat(20), 10)).toBe(`${"a".repeat(9)}…`);
    expect(firstLine("  ")).toBe("");
  });
});

describe("between turns", () => {
  it("says how much later the next one came, once it is half an hour or more", () => {
    expect(laterBy(29 * 60_000)).toBeUndefined();
    expect(laterBy(35 * 60_000)).toBe("35 minutes later");
    expect(laterBy(8 * 3_600_000)).toBe("8 hours later");
    expect(laterBy(30 * 3_600_000)).toBe("30 hours later");
    expect(laterBy(40 * 3_600_000)).toBe("2 days later");
  });

  it("knows when a turn last moved", () => {
    const [t] = turns([ask("Go", 1000), prose("Done"), result(9)]);
    if (!t) throw new Error("no turn");
    expect(lastTs(t)).toBe(t.end?.ts);
  });
});

describe("a turn's title", () => {
  it("is its ask's first line, or what fctl passed on", () => {
    const t = turns([ask("## Fix the *login* test\nIt fails.")]);
    expect(t[0] && turnTitle(t[0], true, false)).toBe("Fix the login test");
    const ticket: Item = {
      ...base(),
      kind: "user",
      text: "Tomás delegated SHOP-419 to fctl in Linear: …",
      source: "system",
      ticket: {
        ticket: {
          tracker: "linear",
          id: "x",
          key: "SHOP-419",
          title: "Danish for the shop",
          url: "https://l/x",
        },
        event: "delegated",
      },
    };
    const [d] = turns([ticket]);
    expect(d && turnTitle(d, true, false)).toBe("SHOP-419: Danish for the shop");
    const [loose] = turns([prose("…and the tests pass.")]);
    expect(loose && turnTitle(loose, true, true)).toBe("Earlier activity");
    expect(loose && turnTitle(loose, false, false)).toBe("The agent went on without a message");
  });
});

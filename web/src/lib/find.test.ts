import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Markdown } from "@/components/markdown";
import type * as G from "@/gen";
import type { Json } from "./claude-types";
import {
  type Findable,
  findables,
  fold,
  needle,
  occurrences,
  plainText,
  rangesIn,
  regionText,
  revealing,
  search,
} from "./find";
import type { Item, Tool } from "./transcript";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let seq = 0;
const call = (name: string, input: Record<string, Json> = {}, extra: Partial<Tool> = {}): Tool => ({
  kind: "tool",
  key: `t${++seq}`,
  seq,
  ts: seq,
  id: `toolu_${seq}`,
  name,
  input,
  children: 0,
  ...extra,
});
const prose = (text: string): Item => ({ kind: "prose", key: `p${++seq}`, seq, ts: seq, text });
const thought = (text: string): Item => ({ kind: "thinking", key: `th${++seq}`, seq, ts: seq, text });
const message = (text: string, more: Partial<Extract<Item, { kind: "user" }>> = {}): Item => ({
  kind: "user",
  key: `u${++seq}`,
  seq,
  ts: seq,
  text,
  source: "web",
  ...more,
});
const notice = (n: Partial<G.Notice>): Item => ({
  kind: "notice",
  key: `n${++seq}`,
  seq,
  ts: seq,
  notice: { level: "info", code: "x", message: "", ...n } as G.Notice,
});

const ids = (fs: Findable[]) => fs.map((f) => f.id.replace(/:.*$/, ""));
const of = (fs: Findable[], id: string) => fs.find((f) => f.id === id);

describe("folded text", () => {
  it("compares whatever the case, accents and whitespace", () => {
    expect(fold("Le CAFÉ\n\t  est  là").text).toBe("le cafe est la");
    expect(needle("  Café  Résumé ")).toBe("cafe resume");
    // Combining accents too.
    expect(fold("café").text).toBe("cafe");
  });

  it("leaves the marks of other scripts, which make other letters", () => {
    expect(occurrences(fold("か"), needle("が"))).toEqual([]);
    expect(occurrences(fold("がか"), needle("が"))).toEqual([[0, 1]]);
    // Korean syllables stay whole.
    expect(occurrences(fold("한국"), needle("하"))).toEqual([]);
    expect(occurrences(fold("किताब"), needle("क"))).toEqual([[0, 1]]);
    expect(occurrences(fold("कि"), needle("कि"))).toEqual([[0, 2]]);
    expect(occurrences(fold("क"), needle("कि"))).toEqual([]);
  });

  it("says where each match is in the original", () => {
    const text = "Un CAFÉ,\n\nun  café";
    const found = occurrences(fold(text), needle("un cafe"));
    expect(found.map(([s, e]) => text.slice(s, e))).toEqual(["Un CAFÉ", "un  café"]);
    expect(occurrences(fold("aaaa"), "aa")).toEqual([
      [0, 2],
      [2, 4],
    ]);
    expect(occurrences(fold("abc"), "")).toEqual([]);
    // Characters outside the basic plane count as JavaScript counts them.
    const emoji = "😀 bug";
    const [[s, e] = [0, 0]] = occurrences(fold(emoji), "bug");
    expect(emoji.slice(s, e)).toBe("bug");
  });
});

describe("Markdown's text", () => {
  const roots: (() => void)[] = [];
  afterEach(() => {
    for (const r of roots.splice(0)) r();
  });

  /** What `Markdown` shows of `text`, as finding reads the page. */
  function shown(text: string): string {
    const el = document.createElement("div");
    document.body.append(el);
    const root = createRoot(el);
    act(() => root.render(createElement(Markdown, { text })));
    roots.push(() => {
      act(() => root.unmount());
      el.remove();
    });
    return fold(regionText(el).text).text.trim();
  }

  it("is what the page shows of it", () => {
    for (const md of [
      "Fixed **the login** redirect in `auth.rs`, see [the docs](https://example.com/docs).",
      "# Done\n\nTwo things:\n\n- one *thing*\n- two\n\n1. first\n2. second",
      "| a | b |\n|---|---|\n| one | two |",
      "> quoted\n> text\n\nline one  \nline two",
      "```sh\ncargo test -p fctl\n```\n\nAfter the code.",
      "A &amp; B &lt;tag&gt; ~~gone~~ \\*kept\\*",
      "- [ ] a task\n- [x] done",
      "<b>html</b> stays text",
    ]) {
      expect(fold(plainText(md)).text.trim(), md).toBe(shown(md));
    }
  });

  it("shows no text for pictures", () => {
    expect(plainText("Look: ![the header](https://example.com/a.png)")).toBe("Look: ");
  });
});

describe("the transcript's regions", () => {
  it("are what it says, in the order it shows them", () => {
    const task = message("Fix the login redirect");
    const reply = prose("Found it in **auth.rs**.");
    const run = [
      call("Bash", { command: "cargo test", description: "Run the tests" }),
      thought("The cookie expires first."),
      call("Read", { file_path: "/home/agent/workspace/src/auth.rs" }),
    ];
    const done = prose("Fixed.");
    const fs = findables([task, reply, ...run, done], "/home/agent/workspace");
    expect(ids(fs)).toEqual(["msg", "prose", "line", "cmd", "thought", "line", "prose"]);
    expect(of(fs, `prose:${reply.key}`)?.text).toBe("Found it in auth.rs.");
    expect(of(fs, `line:${run[0]?.key}`)?.text).toBe("Run the tests");
    expect(of(fs, `cmd:${run[0]?.key}`)?.text).toBe("cargo test");
    expect(of(fs, `line:${run[2]?.key}`)?.text).toBe("src/auth.rs");
    // Each in the part of the page it is in.
    expect(fs.map((f) => f.part)).toEqual([
      task.key,
      reply.key,
      run[0]?.key,
      run[0]?.key,
      run[0]?.key,
      run[0]?.key,
      done.key,
    ]);
  });

  it("say what opens to show them", () => {
    const [bash, think, read] = [
      call("Bash", { command: "ls -la", description: "List the files" }),
      thought("Hmm."),
      call("Read", { file_path: "a.ts" }),
    ];
    const fs = findables([bash as Item, think as Item, read as Item]);
    // A folded run shows a step's line on its own, and opens it for what it shows open.
    expect(of(fs, `line:${bash?.key}`)?.reveal).toEqual([[bash?.key, "show"]]);
    expect(of(fs, `cmd:${bash?.key}`)?.reveal).toEqual([[bash?.key, "open"]]);
    expect(of(fs, `thought:${think?.key}`)?.reveal).toEqual([[think?.key, "open"]]);
    // A call on its own isn't folded.
    const alone = call("Bash", { command: "make\nmake test" });
    const one = findables([alone]);
    expect(of(one, `line:${alone.key}`)?.reveal).toEqual([]);
    expect(of(one, `line:${alone.key}`)?.text).toBe("make");
    expect(of(one, `cmd:${alone.key}`)?.reveal).toEqual([[alone.key, "open"]]);
    // A command its line shows whole is found there only.
    const short = call("Bash", { command: "ls" });
    expect(ids(findables([short]))).toEqual(["line"]);
  });

  it("go into subagents' work", () => {
    const inner = call("Grep", { pattern: "redirect" });
    const innerToo = call("Read", { file_path: "b.ts" });
    const agent = call(
      "Agent",
      { description: "Find the redirect", prompt: "Look for the redirect loop" },
      {
        agent: {
          steps: [prose("Looking.") as never, inner, innerToo],
          ended: "completed",
          report: "It is in b.ts",
        },
        result: { text: "It is in b.ts", isError: false },
      },
    );
    const fs = findables([agent]);
    expect(ids(fs)).toEqual(["line", "prompt", "prose", "line", "line", "report"]);
    expect(of(fs, `prompt:${agent.key}`)?.reveal).toEqual([
      [agent.key, "open"],
      [`prompt:${agent.key}`, "open"],
    ]);
    expect(of(fs, `line:${inner.key}`)?.reveal).toEqual([
      [agent.key, "open"],
      [inner.key, "show"],
    ]);
  });

  it("include what fctl said and kept", () => {
    const followUp = message("On GitHub, Ada commented: try the cache", {
      source: "system",
      followUp: { kind: "comment" } as unknown as G.FollowUp,
    });
    const findings = message("fctl's review agent reviewed commit abc: one finding", { source: "system" });
    const note =
      'Ada started this session on Monday 26 October 2026 at 09:00 (UTC) from the project\'s schedule "Nightly".';
    const scheduled = message(`Check the deploy\n\n---\n${note}`);
    const memory = notice({ code: "memory_added", memory: { text: "Run **npm ci** first" } as G.Memory });
    const retro = notice({
      code: "retrospective",
      retrospective: { worked_on: "The redirect", insights: "Cookies expire" } as G.Retrospective,
    });
    const plain = notice({ message: "Claude Code restarted" });
    const shot = notice({ message: "Published a.png", published: {} as G.PublishedFile });
    const stopped: Item = {
      kind: "status",
      key: `s${++seq}`,
      seq,
      ts: seq,
      status: "failed",
      reason: "Out of disk",
    };
    const fs = findables([followUp, findings, scheduled, memory, retro, plain, shot, stopped]);
    expect(fs.map((f) => f.id.split(":").slice(0, 1).concat(f.id.split(":").slice(2)).join(":"))).toEqual([
      "told",
      "msg",
      "msg",
      "memory",
      "retro:worked_on",
      "retro:insights",
      "notice",
      "status",
    ]);
    expect(of(fs, `told:${followUp.key}`)?.reveal).toEqual([[`told:${followUp.key}`, "open"]]);
    expect(of(fs, `msg:${findings.key}`)?.reveal).toEqual([[`long:${findings.key}`, "open"]]);
    // A schedule's run shows its task, then fctl's note.
    expect(of(fs, `msg:${scheduled.key}`)?.text).toBe(`Check the deploy\n${note}`);
    expect(of(fs, `memory:${memory.key}`)?.text).toBe("Run npm ci first");
  });
});

describe("matches", () => {
  it("are each occurrence, in the order the page shows them", () => {
    const fs = findables([message("Cookie, cookie!"), prose("The *cookie* expired.")]);
    const ms = search(fs, "COOKIE");
    expect(ms.map((m) => m.key)).toEqual([`${fs[0]?.id}#0`, `${fs[0]?.id}#1`, `${fs[1]?.id}#0`]);
    expect(search(fs, "   ")).toEqual([]);
    expect(search(fs, "zebra")).toEqual([]);
  });

  it("gather what has to open to show them", () => {
    const bash = call("Bash", { command: "npm test", description: "Test it" });
    const read = call("Read", { file_path: "test.ts" });
    const ms = search(findables([bash, read]), "test");
    const reveal = revealing(new Map(), ...ms);
    // Opening shows too.
    expect([...reveal]).toEqual([
      [bash.key, "open"],
      [read.key, "show"],
    ]);
    expect(revealing(new Map([[read.key, "open"]]), ...ms).get(read.key)).toBe("open");
  });
});

describe("the page's matches", () => {
  it("are ranges of its text, across elements and lines", () => {
    const el = document.createElement("div");
    el.innerHTML =
      "<p>The <strong>login</strong> red<em>irect</em></p><p>loops.</p><button>Copy login redirect</button>" +
      '<span data-find-skip="">login redirect</span>';
    const ranges = rangesIn(el, needle("login redirect"));
    expect(ranges.map((r) => r.toString())).toEqual(["login redirect"]);
    // Blocks break the text: no match runs from one paragraph into the next.
    expect(rangesIn(el, needle("redirect loops"))).toHaveLength(1);
    expect(rangesIn(el, needle("redirectloops"))).toHaveLength(0);
    expect(rangesIn(el, needle("copy"))).toHaveLength(0);
  });
});

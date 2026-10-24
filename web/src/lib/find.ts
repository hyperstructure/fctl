// Finding words in a session's transcript, as the session page does (⌘F):
// what of it is searched, where each match is, and what has to open to show
// it.
//
// The transcript folds much of what it says: a run of the agent's tool
// calls into one line, its thoughts, a subagent's work, the command behind
// a call's description, what fctl told the agent. A browser's find sees only
// what is open on the page. This looks in all of it, each piece as the page
// shows it once open (a "region", which the page marks with `data-find`),
// and says what has to open for a match to show: the page opens it as the
// reader steps to the match. It finds what was typed, as typed but for
// case, accents and how much whitespace separates words, as the search of
// all sessions does (fctl_domain::search).

import { lexer, type Token, type Tokens } from "marked";
import type * as G from "@/gen";
import type { Json } from "./claude-types";
import { parts as retrospectiveParts } from "./retrospectives";
import { callsIn, describe, parts, type Work } from "./runs";
import { splitRunNote } from "./schedules";
import { findingsBefore, type Item, isSubagent, subagentReport, type Tool } from "./transcript";

/**
 * What has to open for a region to show: a step a folded run shows on its
 * own (`show`), or a line that opens into it (`open`), by its key.
 */
export type Reveal = readonly [key: string, how: "show" | "open"];

/** A piece of the transcript that finding looks in. */
export interface Findable {
  /** The `data-find` of the element that shows it. */
  id: string;
  /** The part of the transcript it is in (`data-part`), which is on the page even while it is folded. */
  part: string;
  /** The event it comes from. */
  seq: number;
  /** Its text, as the page shows it. */
  text: string;
  /** What has to open for it to show, the outermost first. */
  reveal: readonly Reveal[];
}

/** An occurrence of what was typed: the `at`th in its region. */
export interface Match {
  /** Its region's id and `at`, which stay the same as the transcript grows. */
  key: string;
  findable: Findable;
  at: number;
}

// ───────────────────────────── text ─────────────────────────────

/** Text as finding compares it, with where each of its characters comes from. */
export interface Folded {
  /** Lowercase, without accents, and with each run of whitespace as one space. */
  text: string;
  /** For each UTF-16 code unit of `text`: where its character starts in the original, and where it ends. */
  start: number[];
  end: number[];
}

const MARKS = /\p{M}/gu;
const SPACE = /\s/;

/** `s` as finding compares it ([`Folded`]). */
export function fold(s: string): Folded {
  let text = "";
  const start: number[] = [];
  const end: number[] = [];
  let i = 0;
  for (const c of s) {
    const n = c.length;
    const code = c.charCodeAt(0);
    if (code === 32 || code === 10 || code === 9 || code === 13 || (code > 127 && SPACE.test(c))) {
      if (text.endsWith(" ")) end[end.length - 1] = i + n;
      else if (text) {
        text += " ";
        start.push(i);
        end.push(i + n);
      }
    } else {
      const f =
        code < 128
          ? code >= 65 && code <= 90
            ? String.fromCharCode(code + 32)
            : c
          : c.normalize("NFD").replace(MARKS, "").toLowerCase();
      for (let k = 0; k < f.length; k++) {
        text += f[k];
        start.push(i);
        end.push(i + n);
      }
    }
    i += n;
  }
  return { text, start, end };
}

/** What was typed, as finding compares it: [`fold`]ed, without spaces at its ends. */
export const needle = (query: string) => fold(query).text.trim();

/** Where `needle` is in `hay`: the ranges `[start, end)` of the original text, which don't overlap. */
export function occurrences(hay: Folded, needle: string): [number, number][] {
  const out: [number, number][] = [];
  if (!needle) return out;
  let at = hay.text.indexOf(needle);
  while (at >= 0) {
    out.push([hay.start[at] ?? 0, hay.end[at + needle.length - 1] ?? 0]);
    at = hay.text.indexOf(needle, at + needle.length);
  }
  return out;
}

const folds = new Map<string, Folded>();

/** [`fold`], remembered for the texts of the transcript, which finding goes over at each key. */
function folded(text: string): Folded {
  let f = folds.get(text);
  if (!f) {
    if (folds.size > 20_000) folds.clear();
    f = fold(text);
    folds.set(text, f);
  }
  return f;
}

/** Markdown's text as `Markdown` (components/markdown.tsx) shows it: its blocks on lines of their own. */
export function plainText(markdown: string): string {
  let tokens: Token[];
  try {
    tokens = lexer(markdown, { gfm: true, breaks: false });
  } catch {
    return markdown;
  }
  return tokens
    .map(blockText)
    .filter((t) => t)
    .join("\n");
}

function blockText(t: Token): string {
  switch (t.type) {
    case "heading":
      return inlineText((t as Tokens.Heading).tokens);
    case "paragraph":
      return inlineText((t as Tokens.Paragraph).tokens);
    case "code":
      return (t as Tokens.Code).text;
    case "blockquote":
      return (t as Tokens.Blockquote).tokens.map(blockText).join("\n");
    case "list":
      return (t as Tokens.List).items
        .map((it) =>
          it.tokens
            .map((x) => (x.type === "text" ? inlineText((x as Tokens.Text).tokens ?? [x]) : blockText(x)))
            .join("\n"),
        )
        .join("\n");
    case "table": {
      const tb = t as Tokens.Table;
      return [tb.header, ...tb.rows]
        .flat()
        .map((c) => inlineText(c.tokens))
        .join("\n");
    }
    case "html":
      return (t as Tokens.HTML).text;
    case "text": {
      const tx = t as Tokens.Text;
      return tx.tokens ? inlineText(tx.tokens) : decode(tx.text);
    }
    default:
      return "";
  }
}

function inlineText(tokens: Token[] | undefined): string {
  if (!tokens) return "";
  return tokens
    .map((t) => {
      switch (t.type) {
        case "strong":
        case "em":
        case "del":
          return inlineText((t as Tokens.Strong).tokens);
        case "codespan":
          return (t as Tokens.Codespan).text;
        case "br":
          return "\n";
        case "link":
          return inlineText((t as Tokens.Link).tokens);
        case "image": {
          // A picture shows no text.
          const im = t as Tokens.Image;
          return /^https:/i.test(im.href) ? "" : im.text;
        }
        case "text": {
          const tx = t as Tokens.Text;
          return tx.tokens ? inlineText(tx.tokens) : decode(tx.text);
        }
        case "escape":
          return (t as Tokens.Escape).text;
        default:
          return "raw" in t ? decode(String(t.raw)) : "";
      }
    })
    .join("");
}

function decode(s: string): string {
  return s.replace(
    /&(amp|lt|gt|quot|#39);/g,
    (_, e) => ({ amp: "&", lt: "<", gt: ">", quot: '"', "#39": "'" })[e as string] ?? _,
  );
}

const plains = new Map<string, string>();

/** [`plainText`], remembered. */
function plain(markdown: string): string {
  let p = plains.get(markdown);
  if (p === undefined) {
    if (plains.size > 5_000) plains.clear();
    p = plainText(markdown);
    plains.set(markdown, p);
  }
  return p;
}

// ───────────────────────────── the transcript ─────────────────────────────

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

/**
 * The transcript's regions, in the order the page shows them: people's
 * messages, the agent's replies, the lines of its work (what a call is
 * about, a command, the plan), its thoughts, what its subagents were asked
 * and did and reported, the notes it kept, its retrospectives, notices and
 * why the session stopped. Not what calls returned, files or diffs.
 * `cwd`: where Claude Code works, which paths start from.
 */
export function findables(items: Item[], cwd?: string): Findable[] {
  const out: Findable[] = [];
  for (const p of parts(items)) {
    if (p.kind === "run") run(out, p.key, p.steps, [], cwd);
    else item(out, p.key, p.item, [], cwd);
  }
  return out;
}

type Out = Findable[];

function push(out: Out, id: string, part: string, seq: number, text: string, reveal: readonly Reveal[]) {
  if (text.trim()) out.push({ id, part, seq, text, reveal });
}

/** A run of the agent's work: two calls or more fold it into a line, which shows a step on its own. */
function run(out: Out, part: string, steps: Work[], outer: readonly Reveal[], cwd?: string) {
  const fold = callsIn(steps).length > 1;
  for (const s of steps) {
    const show: readonly Reveal[] = fold ? [...outer, [s.key, "show"]] : outer;
    const open: readonly Reveal[] = [...outer, [s.key, "open"]];
    if (s.kind === "thinking") push(out, `thought:${s.key}`, part, s.seq, s.text, open);
    else tool(out, part, s, show, open, cwd);
  }
}

/** A tool call: its line (`show` shows it), and what opening it (`open`) shows. */
function tool(
  out: Out,
  part: string,
  t: Tool,
  show: readonly Reveal[],
  open: readonly Reveal[],
  cwd?: string,
) {
  const { verb, target } = describe(t.name, t.input, cwd);
  push(out, `line:${t.key}`, part, t.seq, target ? target.text : verb, show);
  if (t.name === "Bash") {
    // The line shows its description, or its first line.
    const command = str(t.input.command);
    if (target?.text !== command.trim()) push(out, `cmd:${t.key}`, part, t.seq, command, open);
  }
  if (t.name === "TodoWrite") {
    const todos = Array.isArray(t.input.todos) ? (t.input.todos as { content?: Json }[]) : [];
    push(out, `todo:${t.key}`, part, t.seq, todos.map((x) => str(x?.content)).join("\n"), open);
  }
  if (!isSubagent(t)) return;
  push(out, `prompt:${t.key}`, part, t.seq, str(t.input.prompt), [...open, [`prompt:${t.key}`, "open"]]);
  for (const p of parts(t.agent?.steps ?? [])) {
    if (p.kind === "run") run(out, part, p.steps, open, cwd);
    else if (p.item.kind === "tool") tool(out, part, p.item, open, [...open, [p.item.key, "open"]], cwd);
    else if (p.item.kind === "prose")
      push(out, `prose:${p.item.key}`, part, p.item.seq, plain(p.item.text), open);
  }
  const report = subagentReport(t);
  const steps = t.agent?.steps ?? [];
  const last = steps[steps.length - 1];
  // From the background, its last words are its report, which shows once.
  if (report && !(last?.kind === "prose" && last.text.trim() === report.trim()))
    push(out, `report:${t.key}`, part, t.seq, plain(report), open);
}

function item(out: Out, part: string, it: Item, outer: readonly Reveal[], cwd?: string) {
  switch (it.kind) {
    case "user": {
      // From fctl about a pull request, a review or a ticket: what it told the agent, a click away.
      if (it.followUp || it.ticket || it.review) {
        push(out, `told:${it.key}`, part, it.seq, it.text.trim(), [[`told:${it.key}`, "open"]]);
        return;
      }
      if (findingsBefore(it)) {
        push(out, `msg:${it.key}`, part, it.seq, it.text, [[`long:${it.key}`, "open"]]);
        return;
      }
      const { task, note } = splitRunNote(it.text);
      push(out, `msg:${it.key}`, part, it.seq, note ? `${task}\n${note}` : task, []);
      return;
    }
    case "prose":
      push(out, `prose:${it.key}`, part, it.seq, plain(it.text), outer);
      return;
    case "tool":
      tool(out, part, it, outer, [...outer, [it.key, "open"]], cwd);
      return;
    case "notice":
      notice(out, part, it, it.notice);
      return;
    case "status":
      if (it.reason) push(out, `status:${it.key}`, part, it.seq, it.reason, []);
      return;
    default:
      return;
  }
}

/** A notice as the transcript shows it: a note the agent kept, its retrospective, or what it says. */
function notice(out: Out, part: string, it: Item, n: G.Notice) {
  if (n.memory) {
    push(out, `memory:${it.key}`, part, it.seq, plain(n.memory.text), []);
    return;
  }
  if (n.retrospective) {
    for (const p of retrospectiveParts(n.retrospective))
      push(out, `retro:${it.key}:${p.key}`, part, it.seq, plain(p.text), []);
    return;
  }
  // Screenshots, reviews, follow-ups held and tickets show as lines of their own.
  if (n.published || n.review || n.follow_up || n.ticket) return;
  push(out, `notice:${it.key}`, part, it.seq, n.message, []);
}

// ───────────────────────────── matches ─────────────────────────────

/** The matches of `query` among `findables`, in the order the page shows them. */
export function search(findables: Findable[], query: string): Match[] {
  const n = needle(query);
  const out: Match[] = [];
  if (!n) return out;
  for (const f of findables) {
    const count = occurrences(folded(f.text), n).length;
    for (let at = 0; at < count; at++) out.push({ key: `${f.id}#${at}`, findable: f, at });
  }
  return out;
}

/** The reveals of `matches`, merged into `into`: what has to open to show them, `open` over `show`. */
export function revealing(into: ReadonlyMap<string, Reveal[1]>, ...matches: Match[]): Map<string, Reveal[1]> {
  const next = new Map(into);
  for (const m of matches)
    for (const [key, how] of m.findable.reveal) if (next.get(key) !== "open") next.set(key, how);
  return next;
}

// ───────────────────────────── the page ─────────────────────────────

/** Elements that show no text of the transcript's: controls, and what marks itself so. */
const SKIPPED = new Set(["BUTTON", "INPUT", "SELECT", "TEXTAREA", "SVG"]);
/** Elements on lines of their own, between which the page's text breaks. */
const BLOCKS = new Set([
  "ADDRESS",
  "BLOCKQUOTE",
  "BR",
  "DD",
  "DIV",
  "DL",
  "DT",
  "H1",
  "H2",
  "H3",
  "H4",
  "H5",
  "H6",
  "HR",
  "LI",
  "OL",
  "P",
  "PRE",
  "SECTION",
  "TABLE",
  "TD",
  "TH",
  "TR",
  "UL",
]);

/** A region's text as the page shows it, with where each text node starts in it. */
export function regionText(el: Element): { text: string; nodes: { node: Text; at: number }[] } {
  let text = "";
  const nodes: { node: Text; at: number }[] = [];
  const walk = (parent: Node) => {
    for (let c = parent.firstChild; c; c = c.nextSibling) {
      if (c.nodeType === 3) {
        const node = c as Text;
        nodes.push({ node, at: text.length });
        text += node.data;
      } else if (c.nodeType === 1) {
        const e = c as Element;
        const tag = e.tagName.toUpperCase();
        if (SKIPPED.has(tag) || e.hasAttribute("data-find-skip")) continue;
        const block = BLOCKS.has(tag);
        if (block) text += "\n";
        walk(e);
        if (block) text += "\n";
      }
    }
  };
  walk(el);
  return { text, nodes };
}

/** The node and offset of `at` in a region's text: the start of the next text node, or the end of the last, between them. */
function point(nodes: { node: Text; at: number }[], at: number, end: boolean): [Text, number] | undefined {
  let lo = 0;
  let hi = nodes.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const n = nodes[mid];
    if (n && n.at <= at) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  const n = nodes[found];
  if (n && at <= n.at + n.node.data.length && (end || at < n.at + n.node.data.length))
    return [n.node, at - n.at];
  // Between text nodes: a line break of the page's own.
  if (end) return n ? [n.node, n.node.data.length] : undefined;
  const next = nodes[found + 1];
  return next ? [next.node, 0] : undefined;
}

/** The ranges of the page where `needle` ([`needle`]) shows in a region. */
export function rangesIn(el: Element, needle: string): Range[] {
  if (!needle) return [];
  const { text, nodes } = regionText(el);
  const doc = el.ownerDocument;
  const out: Range[] = [];
  for (const [s, e] of occurrences(fold(text), needle)) {
    const a = point(nodes, s, false);
    const b = point(nodes, e, true);
    if (!a || !b) continue;
    const r = doc.createRange();
    try {
      r.setStart(a[0], a[1]);
      r.setEnd(b[0], b[1]);
    } catch {
      continue;
    }
    if (!r.collapsed) out.push(r);
  }
  return out;
}

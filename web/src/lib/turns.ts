// A session's transcript in turns, as the session page (routes/session.tsx) shows it:
// what a person or fctl asked, what the agent did about it, and how that ended. Each turn is a
// chapter of the page, which the reader steps through and finds their way back to.
//
// A turn starts with a message, once the agent has ended the one before: a message that comes
// while it works is part of the turn it works on, as the agent reads it at its next step. The
// agent's work after a turn ended, with no message to start it (it went on when something it
// started in the background ended), starts a turn of its own. What only comes after the end (the
// branch's new state, a stop after being idle) stays with the turn it follows.

import { wokeLine } from "./autopilot";
import type { Json } from "./claude-types";
import { parts as followUpParts, happenedText, pullRef } from "./follow-ups";
import { changed, type Lines, type Part, shownPath } from "./runs";
import type { Item, Step, Tool } from "./transcript";

type Ask = Extract<Item, { kind: "user" }>;

export interface Turn {
  /** Stays the same as the turn grows: the key of its first part. */
  key: string;
  /**
   * The message that started it, a person's or fctl's: none for a turn whose start isn't loaded
   * yet, or that the agent started on its own.
   */
  ask?: Ask;
  /** The ask's part, which finding in the transcript knows by its key. */
  askPart?: Part<Item>;
  /** What came after the ask, until the turn ended: the agent's work and words, milestones, messages. */
  body: Part<Item>[];
  /** How it ended, once it did: its result, or what cut it short. */
  end?: TurnEnd;
  /** What came after it ended, before the next turn: the branch's new state, a stop. */
  after: Part<Item>[];
}

export type TurnEnd = {
  ts: number;
  seq: number;
  key: string;
} /** The agent's result: whether the turn went well, and how long it took (ms). */ & (
  | { kind: "result"; ok: boolean; durationMs: number }
  /** The session stopped or failed before the agent ended the turn. */
  | { kind: "stopped" | "failed" }
  /** Claude Code restarted under it, or fctl cut it off. */
  | { kind: "cut" }
);

/** Statuses that end a turn the agent hadn't: its session no longer runs. */
const STOPS = new Set(["stopped", "failed"]);

/** Notices that say the turn the agent worked on is over: Claude Code restarted, or fctl cut it off. */
const CUTS = new Set(["claude_restarted", "turn_cut_off"]);

/** Whether an item is the agent at work or speaking, which starts a turn of its own after one ended. */
const agentWork = (it: Item) => it.kind === "prose" || it.kind === "tool" || it.kind === "thinking";

/** How an item ends the turn it is in, if it does. */
function endOf(it: Item): TurnEnd | undefined {
  const at = { ts: it.ts, seq: it.seq, key: it.key };
  if (it.kind === "turn") return { ...at, kind: "result", ok: it.ok, durationMs: it.durationMs };
  if (it.kind === "status" && STOPS.has(it.status))
    return { ...at, kind: it.status === "failed" ? "failed" : "stopped" };
  if (it.kind === "notice" && it.notice.code && CUTS.has(it.notice.code)) return { ...at, kind: "cut" };
  return undefined;
}

/** The transcript's parts (`parts` in lib/runs.ts) in turns. */
export function turnsOf(ps: Part<Item>[]): Turn[] {
  const out: Turn[] = [];
  const start = (p: Part<Item>, ask?: Ask): Turn => {
    const t: Turn = { key: p.key, ask, askPart: ask ? p : undefined, body: [], after: [] };
    out.push(t);
    return t;
  };
  let turn: Turn | undefined;
  for (const p of ps) {
    if (p.kind === "run") {
      if (!turn || turn.end) turn = start(p);
      turn.body.push(p);
      continue;
    }
    const it = p.item;
    if (it.kind === "user") {
      // While the agent works on a turn, a message is part of it.
      if (!turn || turn.end) turn = start(p, it);
      else turn.body.push(p);
      continue;
    }
    if (turn?.end && !agentWork(it)) {
      turn.after.push(p);
      continue;
    }
    if (!turn || turn.end) turn = start(p);
    const end = endOf(it);
    if (end) {
      turn.end = end;
      // The result is said by the turn's end; a stop or a cut also shows as the line it is.
      if (it.kind !== "turn") turn.after.push(p);
    } else turn.body.push(p);
  }
  return out;
}

/** A file the agent changed with its edit tools in a turn, by its path as people know it. */
export interface EditedFile {
  path: string;
  /** Its path as the agent wrote it, from the root of the sandbox. */
  full: string;
  lines: Lines;
}

/** What a turn did, as its end and the turns' outline say. */
export interface TurnFacts {
  /** The tool calls the agent made, its subagents' too. */
  calls: number;
  /** Those that failed. */
  failed: number;
  /** The files it changed with its edit tools (not those its commands changed), in the order it first did. */
  files: EditedFile[];
  /** Lines its edits added and took out, in all of them. */
  lines: Lines;
  /** The agent's last words in it, if it said any. */
  answer?: string;
  /** When it started: its ask, else its first part. */
  start: number;
}

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

/** The path a call that edits a file edits, if it is one that does. */
function editedPath(t: Tool): string | undefined {
  switch (t.name) {
    case "Edit":
    case "MultiEdit":
    case "Write":
      return str(t.input.file_path) || undefined;
    case "NotebookEdit":
      return str(t.input.notebook_path) || undefined;
    default:
      return undefined;
  }
}

/** A turn's facts ([`TurnFacts`]); `cwd`, where Claude Code works, is where paths start. */
export function factsOf(turn: Turn, cwd?: string): TurnFacts {
  const facts: TurnFacts = {
    calls: 0,
    failed: 0,
    files: [],
    lines: { added: 0, removed: 0 },
    start: turn.ask?.ts ?? firstTs(turn) ?? 0,
  };
  const files = new Map<string, EditedFile>();
  const call = (t: Tool) => {
    facts.calls++;
    if (t.result?.isError || t.agent?.ended === "failed") facts.failed++;
    const full = editedPath(t);
    const lines = full ? changed(t) : undefined;
    if (full && lines) {
      const f = files.get(full) ?? { path: shownPath(full, cwd), full, lines: { added: 0, removed: 0 } };
      f.lines = { added: f.lines.added + lines.added, removed: f.lines.removed + lines.removed };
      files.set(full, f);
      facts.lines = { added: facts.lines.added + lines.added, removed: facts.lines.removed + lines.removed };
    }
    for (const s of t.agent?.steps ?? []) step(s);
  };
  const step = (s: Step) => {
    if (s.kind === "tool") call(s);
  };
  for (const p of turn.body) {
    if (p.kind === "run") {
      for (const s of p.steps) if (s.kind === "tool") call(s);
    } else if (p.item.kind === "tool") call(p.item);
    else if (p.item.kind === "prose") facts.answer = p.item.text;
  }
  facts.files = [...files.values()];
  return facts;
}

/** When a turn's first part came. */
function firstTs(turn: Turn): number | undefined {
  const p = turn.body[0] ?? turn.after[0];
  if (!p) return undefined;
  return p.kind === "run" ? p.steps[0]?.ts : p.item.ts;
}

/** How long a turn took: its result says so; else from its start until `now`, or until its end. */
export function turnTook(turn: Turn, facts: TurnFacts, now = Date.now()): number {
  if (turn.end?.kind === "result") return turn.end.durationMs;
  const until = turn.end?.ts ?? now;
  return Math.max(0, until - facts.start);
}

/** The turn the reader is in: the last one whose start is above `y` (a page coordinate), from their tops. */
export function turnAt(tops: readonly number[], y: number): number {
  let lo = 0;
  let hi = tops.length - 1;
  let at = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if ((tops[mid] ?? 0) <= y) {
      at = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return at;
}

/** A message's first line, as a line of a list shows it: the rest of it, and Markdown's marks, left out. */
export function firstLine(text: string, max = 160): string {
  const line =
    text
      .split("\n")
      .map((l) =>
        l
          .replace(/^\s*(#{1,6}\s+|[-*+]\s+|>\s*|\d+\.\s+)/, "")
          .replace(/[*_`~]+/g, "")
          .trim(),
      )
      .find((l) => l.length > 0) ?? "";
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

/**
 * What a turn is about, in a line, as the bar under the header and the list of turns say it: what
 * its ask says, or for a message from fctl, what happened that it passed on.
 */
export function turnTitle(turn: Turn, first: boolean, hasMoreBefore: boolean): string {
  const ask = turn.ask;
  if (!ask) return first && hasMoreBefore ? "Earlier activity" : "The agent went on without a message";
  if (ask.ticket) {
    const { ticket, event, body } = ask.ticket;
    const what = event === "delegated" ? ticket.title : body && firstLine(body);
    return what ? `${ticket.key}: ${what}` : ticket.key;
  }
  if (ask.followUp) {
    const f = ask.followUp;
    const { wrote, happened } = followUpParts(f);
    const first = happened[0];
    const what = first
      ? happenedText(f, first)
      : wrote.length === 1
        ? "A comment"
        : `${wrote.length} comments`;
    return `${pullRef(f)}: ${what}`;
  }
  if (ask.review) {
    const r = ask.review;
    const n = r.findings.length;
    return `${r.repo.name}#${r.number}: the review found ${n === 1 ? "1 issue" : `${n} issues`}`;
  }
  // An autopilot woke: what for, rather than its briefing's first line.
  if (ask.wake) {
    const briefed = ask.text.startsWith("You are the autopilot of ");
    return briefed ? "Woke as it was put on" : wokeLine(ask.wake);
  }
  return firstLine(ask.text) || (ask.attachments?.length ? "Sent files" : "An empty message");
}

/** When a turn's last part came, or its end: the latest the page knows of it. */
export function lastTs(turn: Turn): number | undefined {
  const p = turn.after[turn.after.length - 1] ?? turn.body[turn.body.length - 1];
  const part = p ? (p.kind === "run" ? p.steps[p.steps.length - 1]?.ts : p.item.ts) : undefined;
  return Math.max(part ?? 0, turn.end?.ts ?? 0, turn.ask?.ts ?? 0) || undefined;
}

/**
 * How much later a turn came than the one before ended, when it was long enough to say: "35
 * minutes later", "8 hours later", "2 days later". Nothing within half an hour.
 */
export function laterBy(ms: number): string | undefined {
  const minutes = Math.round(ms / 60_000);
  if (minutes < 30) return undefined;
  if (minutes < 90) return `${minutes} minutes later`;
  const hours = Math.round(minutes / 60);
  if (hours < 36) return `${hours} hours later`;
  const days = Math.round(hours / 24);
  return days === 1 ? "A day later" : `${days} days later`;
}

/** Where a turn starts, as a link to it says: its ask's event, else its first part's. */
export function turnSeq(turn: Turn): number | undefined {
  if (turn.ask) return turn.ask.seq;
  const p = turn.body[0] ?? turn.after[0];
  return p ? (p.kind === "run" ? p.steps[0]?.seq : p.item.seq) : undefined;
}

/**
 * A turn as Markdown, to paste where people talk about it: who asked what, when, what the agent
 * said, and how the turn ended, with the files its edits changed. `when` says a time as the page
 * does; `took` how long the turn took.
 */
export function turnMarkdown(
  turn: Turn,
  facts: TurnFacts,
  { title, when, took }: { title: string; when: (ms: number) => string; took: string },
): string {
  const out: string[] = [];
  const ask = turn.ask;
  if (ask) {
    out.push(`**${ask.author?.name ?? "fctl"}** · ${when(ask.ts)}`);
    const text = ask.followUp || ask.ticket || ask.review ? title : ask.text.trim();
    if (text)
      out.push(
        text
          .split("\n")
          .map((l) => `> ${l}`.trimEnd())
          .join("\n"),
      );
  }
  for (const p of turn.body) if (p.kind === "item" && p.item.kind === "prose") out.push(p.item.text.trim());
  const end = turn.end;
  const how =
    end?.kind === "result"
      ? `${end.ok ? "Finished in" : "Ended after"} ${took}`
      : end
        ? `Worked for ${took}`
        : `Working for ${took}`;
  const steps = facts.calls ? ` · ${facts.calls} ${facts.calls === 1 ? "step" : "steps"}` : "";
  const files = facts.files.length
    ? ` · edited ${facts.files.map((f) => `\`${f.path}\` (+${f.lines.added} −${f.lines.removed})`).join(", ")}`
    : "";
  out.push(`_${how}${steps}${files}_`);
  return out.join("\n\n");
}

/**
 * The turn an event is in: the last that starts at it or before, as a link to a turn finds it even
 * when the turn's start has loaded since the link was made. -1 before the first.
 */
export function turnOfSeq(turns: readonly Turn[], seq: number): number {
  let at = -1;
  for (const [i, t] of turns.entries()) {
    const start = turnSeq(t);
    if (start !== undefined && start <= seq) at = i;
  }
  return at;
}

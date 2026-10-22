// The agent's tool calls in words, and the runs it strings them into.
//
// Between the things it says, an agent mostly works: it runs commands,
// reads and searches files, and thinks in between. The transcript folds such
// a run into one line that says what it did ("Ran 6 commands, read 2
// files"), as Claude Code does, and opens it into its steps.

import { diffLines } from "diff";
import type { Json } from "./claude-types";
import { type Item, isSubagent, type Tool } from "./transcript";

/** A step a run folds: a tool call or a thought. */
export type Work = Extract<Item, { kind: "tool" | "thinking" }>;

/**
 * Whether an item is work a run folds. Subagents and the plan stay on their
 * own lines: what a subagent does has a line of its own already, and the
 * plan is there to be read.
 */
export function folds(it: Item): it is Work {
  if (it.kind === "thinking") return true;
  return it.kind === "tool" && !isSubagent(it) && it.name !== "TodoWrite";
}

/**
 * Whether an item comes on its own time, beside the agent's work rather
 * than as a step of it: a branch that changed, a note the agent kept, a
 * screenshot it published, its retrospective, the host's logs. One that
 * comes while a run is under way goes after it rather than breaking it.
 */
export function aside(it: Item): boolean {
  switch (it.kind) {
    case "git":
    case "logs":
    case "remote":
      return true;
    case "notice":
      return !!(it.notice.memory || it.notice.published || it.notice.retrospective);
    default:
      return false;
  }
}

/** A part of the transcript: an item of its own, or a run of work, from `index` in the list. */
export type Part<T extends Item> =
  | { kind: "item"; key: string; index: number; item: T }
  | { kind: "run"; key: string; index: number; steps: (T & Work)[] };

/**
 * The transcript, or a subagent's steps, in parts: each run of work, and
 * the other items one by one, those that came aside of a run after it. A
 * run keeps the key of its first step, so that it stays the same part as it
 * grows.
 */
export function parts<T extends Item>(items: T[]): Part<T>[] {
  const out: Part<T>[] = [];
  let run: Extract<Part<T>, { kind: "run" }> | undefined;
  let after: Part<T>[] = [];
  const close = () => {
    out.push(...after);
    after = [];
    run = undefined;
  };
  items.forEach((it, index) => {
    if (folds(it)) {
      if (run) run.steps.push(it);
      else {
        run = { kind: "run", key: it.key, index, steps: [it] };
        out.push(run);
      }
    } else if (run && aside(it)) {
      after.push({ kind: "item", key: it.key, index, item: it });
    } else {
      close();
      out.push({ kind: "item", key: it.key, index, item: it });
    }
  });
  close();
  return out;
}

/**
 * The run that may go on: the last one in `ps`, unless something other than
 * what comes aside of runs came after it.
 */
export function lastRun<T extends Item>(ps: Part<T>[]): Extract<Part<T>, { kind: "run" }> | undefined {
  for (let i = ps.length - 1; i >= 0; i--) {
    const p = ps[i];
    if (p?.kind === "run") return p;
    if (!p || !aside(p.item)) return undefined;
  }
  return undefined;
}

/** The tool calls among a run's steps. */
export const callsIn = (steps: Work[]): Tool[] => steps.filter((s) => s.kind === "tool");

/** What a run's line counts its tool calls as. */
export type Kind =
  | "command"
  | "read"
  | "edit"
  | "write"
  | "notebook"
  | "search"
  | "list"
  | "fetch"
  | "web"
  | "other";

export function kindOf(name: string): Kind {
  switch (name) {
    case "Bash":
      return "command";
    case "Read":
      return "read";
    case "Edit":
    case "MultiEdit":
      return "edit";
    case "Write":
      return "write";
    case "NotebookEdit":
      return "notebook";
    case "Grep":
    case "Glob":
      return "search";
    case "LS":
      return "list";
    case "WebFetch":
      return "fetch";
    case "WebSearch":
      return "web";
    default:
      return "other";
  }
}

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

/** What a kind of call counts: the files or directories it was about, or its calls. */
function counted(t: Tool): string {
  switch (kindOf(t.name)) {
    case "read":
    case "edit":
    case "write":
      return `file ${str(t.input.file_path)}`;
    case "notebook":
      return `file ${str(t.input.notebook_path)}`;
    case "list":
      return `dir ${str(t.input.path)}`;
    default:
      return `call ${t.id}`;
  }
}

/** Each kind's verb, done and in progress, and what it counts, one and many. */
const PHRASES: Record<Exclude<Kind, "web">, [string, string, string, string]> = {
  command: ["ran", "running", "command", "commands"],
  read: ["read", "reading", "file", "files"],
  edit: ["edited", "editing", "file", "files"],
  write: ["wrote", "writing", "file", "files"],
  notebook: ["edited", "editing", "notebook", "notebooks"],
  search: ["searched for", "searching for", "pattern", "patterns"],
  list: ["listed", "listing", "directory", "directories"],
  fetch: ["fetched", "fetching", "page", "pages"],
  other: ["used", "using", "tool", "tools"],
};

function phrase(kind: Kind, n: number, doing: boolean): string {
  if (kind === "web") {
    if (doing) return "searching the web";
    return n === 1 ? "searched the web" : `searched the web ${n} times`;
  }
  const [done, now, one, many] = PHRASES[kind];
  return `${doing ? now : done} ${n} ${n === 1 ? one : many}`;
}

/**
 * What tool calls did, in a line, kind by kind in the order they first
 * came: "Ran 3 commands, read 2 files". Files count once however often they
 * were read or edited. `doing` says it of calls still going on: "Running 3
 * commands, reading 2 files".
 */
export function summary(calls: Tool[], doing = false): string {
  const kinds = new Map<Kind, Set<string>>();
  for (const t of calls) {
    const kind = kindOf(t.name);
    const seen = kinds.get(kind) ?? new Set<string>();
    seen.add(counted(t));
    kinds.set(kind, seen);
  }
  const text = [...kinds].map(([kind, seen]) => phrase(kind, seen.size, doing)).join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** Lines an edit added and took out. */
export interface Lines {
  added: number;
  removed: number;
}

const linesOf = new WeakMap<Tool, Lines | null>();

/**
 * The lines a call that changed a file added and took out, once it did: an
 * edit's, or a new file's lines. Nothing for other calls.
 */
export function changed(t: Tool): Lines | undefined {
  if (!t.result || t.result.isError) return undefined;
  let lines = linesOf.get(t);
  if (lines === undefined) {
    lines = count(t);
    linesOf.set(t, lines);
  }
  return lines ?? undefined;
}

function count(t: Tool): Lines | null {
  const diff = (before: Json | undefined, after: Json | undefined, into: Lines) => {
    for (const p of diffLines(str(before), str(after))) {
      if (p.added) into.added += p.count ?? 0;
      if (p.removed) into.removed += p.count ?? 0;
    }
  };
  const lines = { added: 0, removed: 0 };
  switch (t.name) {
    case "Edit":
      diff(t.input.old_string, t.input.new_string, lines);
      return lines;
    case "MultiEdit":
      for (const e of Array.isArray(t.input.edits) ? t.input.edits : []) {
        if (e && typeof e === "object" && !Array.isArray(e)) diff(e.old_string, e.new_string, lines);
      }
      return lines;
    case "Write": {
      const content = str(t.input.content);
      return { added: content ? content.replace(/\n$/, "").split("\n").length : 0, removed: 0 };
    }
    default:
      return null;
  }
}

/** The lines the calls among `calls` that changed files added and took out, if any did. */
export function changedIn(calls: Tool[]): Lines | undefined {
  let total: Lines | undefined;
  for (const t of calls) {
    const lines = changed(t);
    if (lines)
      total = { added: (total?.added ?? 0) + lines.added, removed: (total?.removed ?? 0) + lines.removed };
  }
  return total;
}

/** How long a tool call took, once it returned. */
export function took(t: Tool): number | undefined {
  const end = t.result?.ts;
  return end === undefined ? undefined : Math.max(0, end - t.ts);
}

/** How long a run took: from its first step to its last, or the last of its calls' results. */
export function runTook(steps: Work[]): number {
  const start = steps[0]?.ts ?? 0;
  let end = start;
  for (const s of steps) end = Math.max(end, s.ts, (s.kind === "tool" && s.result?.ts) || 0);
  return end - start;
}

/**
 * A file's path as people know it: from the root of the repository Claude
 * Code works in (`cwd`, else the sandbox's `~/workspace`), from `~`
 * elsewhere in the home directory, else as it is.
 */
export function shownPath(path: string, cwd?: string): string {
  const root = cwd?.replace(/\/+$/, "");
  if (root && path.startsWith(`${root}/`)) return path.slice(root.length + 1);
  const home = /^\/home\/[^/]+\//.exec(path)?.[0] ?? (path.startsWith("/root/") ? "/root/" : undefined);
  if (!home) return path;
  const rest = path.slice(home.length);
  return rest.startsWith("workspace/") ? rest.slice("workspace/".length) : `~/${rest}`;
}

/** What a tool call is about: a file, code (a command, a pattern, an address), or words. */
export type Target = { kind: "path" | "code" | "text"; text: string };

/**
 * A tool call in words: what it did, and to what. A command says what it
 * is for when Claude said so (its description), which reads better than the
 * command itself.
 */
export function describe(
  name: string,
  input: Record<string, Json>,
  cwd?: string,
): { verb: string; target?: Target } {
  const path = (p: string): Target | undefined => (p ? { kind: "path", text: shownPath(p, cwd) } : undefined);
  const code = (c: string): Target | undefined => (c ? { kind: "code", text: c } : undefined);
  switch (name) {
    case "Bash": {
      const description = str(input.description).trim();
      if (description) return { verb: description };
      const command = str(input.command).trim().split("\n")[0];
      return command ? { verb: "Ran", target: code(command) } : { verb: "Ran a command" };
    }
    case "Read":
      return { verb: "Read", target: path(str(input.file_path)) };
    case "Write":
      return { verb: "Wrote", target: path(str(input.file_path)) };
    case "Edit":
    case "MultiEdit":
      return { verb: "Edited", target: path(str(input.file_path)) };
    case "NotebookEdit":
      return { verb: "Edited", target: path(str(input.notebook_path)) };
    case "Grep":
      return { verb: "Searched for", target: code(str(input.pattern)) };
    case "Glob":
      return { verb: "Listed", target: code(str(input.pattern)) };
    case "LS":
      return { verb: "Listed", target: path(str(input.path)) };
    case "WebFetch":
      return { verb: "Fetched", target: code(str(input.url).replace(/^https?:\/\//, "")) };
    case "WebSearch": {
      const query = str(input.query);
      return query
        ? { verb: "Searched the web for", target: { kind: "text", text: query } }
        : { verb: "Searched the web" };
    }
    case "Task":
    case "Agent": {
      const description = str(input.description);
      return { verb: "Delegated", target: { kind: "text", text: description || "a task" } };
    }
    case "TodoWrite":
      return { verb: "Updated the plan" };
    default:
      return { verb: toolName(name) };
  }
}

/** A tool's name to read: an MCP server's tool as "Linear · create issue". */
export function toolName(name: string): string {
  const [, server, tool] = /^mcp__(.+?)__(.+)$/.exec(name) ?? [];
  if (!server || !tool) return name;
  const words = (s: string) => s.replace(/_+/g, " ").trim();
  return `${words(server.replace(/^claude_ai_/, ""))} · ${words(tool)}`;
}

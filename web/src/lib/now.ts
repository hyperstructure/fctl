// What the agent is doing at this moment, in a few words, for the line the new session page
// (routes/session-next.tsx) keeps above its message box while the agent works: what it writes
// or thinks, the call it waits on, the sandbox it sets up, or why its work isn't coming through.

import type * as G from "@/gen";
import type { Json } from "./claude-types";
import { describe, kindOf, type Target, toolName } from "./runs";
import { type Stall, stallLabel } from "./stall";
import { type BackgroundTask, type Item, isSubagent, type LiveTurn, type Tool } from "./transcript";

export type NowKind =
  /** Its sandbox is set up, or it waits its turn for one. */
  | "setup"
  | "thinking"
  /** It writes its reply. */
  | "writing"
  /** It writes a call, which runs next. */
  | "preparing"
  /** A call it made runs. */
  | "tool"
  /** Between steps. */
  | "working"
  /** It waits for work it left in the background. */
  | "background"
  | "paused"
  | "stopping"
  /** Its work isn't coming through. */
  | "stalled";

export interface Now {
  kind: NowKind;
  /** What it does: "Running the tests", "Thinking". */
  text: string;
  /** How far a thought got, as Claude Code estimates it: its tokens so far. */
  tokens?: number;
  /** What it does it to: a command, a file, a pattern. */
  target?: Target;
  /** The tool it uses, for its icon. */
  tool?: string;
  /** Since when it has done it, when the page knows. */
  since?: number;
  /** For a call in progress: how long it has run, as Claude Code reports it (seconds). */
  seconds?: number;
}

/** A call in progress in words, as it happens: "Reading src/lib/money.ts", or what its command is for. */
export function doing(
  name: string,
  input: Record<string, Json>,
  cwd?: string,
): { verb: string; target?: Target } {
  const str = (v: Json | undefined) => (typeof v === "string" ? v.trim() : "");
  switch (name) {
    case "Bash": {
      const description = str(input.description);
      if (description) return { verb: description };
      const command = str(input.command).split("\n")[0] ?? "";
      return command
        ? { verb: "Running", target: { kind: "code", text: command } }
        : { verb: "Running a command" };
    }
    case "Task":
    case "Agent":
      return { verb: "Delegating", target: { kind: "text", text: str(input.description) || "a task" } };
    case "TodoWrite":
      return { verb: "Updating the plan" };
    case "Glob":
    case "LS":
      return { verb: "Listing", target: describe(name, input, cwd).target };
  }
  const { target } = describe(name, input, cwd);
  switch (kindOf(name)) {
    case "command":
      return { verb: "Running", target };
    case "read":
      return { verb: "Reading", target };
    case "edit":
    case "notebook":
      return { verb: "Editing", target };
    case "write":
      return { verb: "Writing", target };
    case "search":
      return { verb: "Searching for", target };
    case "fetch":
      return { verb: "Fetching", target };
    case "web":
      return target ? { verb: "Searching the web for", target } : { verb: "Searching the web" };
    default:
      return { verb: `Using ${toolName(name)}` };
  }
}

/** What a call that streams in is, before its input has: "Preparing an edit". */
function preparing(name?: string): string {
  if (!name) return "Preparing a tool";
  if (name === "Task" || name === "Agent") return "Preparing a subagent";
  switch (kindOf(name)) {
    case "command":
      return "Preparing a command";
    case "edit":
    case "notebook":
      return "Preparing an edit";
    case "write":
      return "Preparing a file";
    default:
      return `Preparing ${toolName(name)}`;
  }
}

/** The last call of the turn the agent works on that hasn't returned: what it waits on. */
function waitingOn(items: Item[]): Tool | undefined {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (!it) continue;
    if (it.kind === "turn" || it.kind === "status") return undefined;
    // Subagents in the background return at once and work on: their own lines say so.
    if (it.kind === "tool" && !it.result && !(isSubagent(it) && it.agent?.background)) return it;
  }
  return undefined;
}

/** When the turn the agent works on started: the message that started it, else its first step. */
function turnStart(items: Item[]): number | undefined {
  let start: number | undefined;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (!it || it.kind === "turn") break;
    start = it.ts;
    if (it.kind === "user") break;
  }
  return start;
}

/** A task the session waits for, with the call that started it where the transcript has that. */
export interface Awaited {
  task: BackgroundTask;
  call?: Tool;
}

/** What an idle session waits for in the background, as its status says and the transcript tells. */
export interface Waiting {
  /** The tasks the transcript saw start and not end, oldest first. */
  tasks: Awaited[];
  /** How many more its status counts: they started before the activity loaded here. */
  more: number;
}

/** How many tasks fctl-agent says an idle session waits for: "Waiting for 2 background tasks". */
export function waitingCount(reason?: string): number | undefined {
  if (reason === "Waiting for a background task") return 1;
  const n = reason?.match(/^Waiting for (\d+) background tasks$/)?.[1];
  return n ? Number(n) : undefined;
}

/** The call `key`, in the transcript or in the work of the subagents in it. */
function callOf(items: readonly Item[], key: string): Tool | undefined {
  for (const it of items) {
    if (it.kind !== "tool") continue;
    if (it.key === key) return it;
    const steps = it.agent?.steps;
    const inner = steps && callOf(steps, key);
    if (inner) return inner;
  }
  return undefined;
}

/**
 * What an idle session waits for, while its status says it waits for background tasks: those the
 * transcript saw start and not end, the newest as many as the status counts, with their calls.
 */
export function waitingOf(
  tasks: readonly BackgroundTask[],
  items: readonly Item[],
  reason?: string,
): Waiting | undefined {
  const count = waitingCount(reason);
  if (count === undefined) return undefined;
  const running = tasks.filter((t) => !t.ended).slice(-count);
  return {
    tasks: running.map((task) => ({ task, call: task.call ? callOf(items, task.call) : undefined })),
    more: count - running.length,
  };
}

/** What kind of work a task is, in a word or two: "shell command", "subagent". */
export function taskKind(type?: string): string | undefined {
  switch (type) {
    case "local_bash":
      return "shell command";
    case "local_agent":
      return "subagent";
    case "remote_agent":
      return "remote agent";
    case "in_process_teammate":
      return "teammate";
    case "local_workflow":
      return "workflow";
    case "mcp_task":
      return "MCP task";
  }
  return type?.startsWith("monitor") ? "monitor" : undefined;
}

/** A task as a line says it: its description, else its command, else what kind of work it is. */
export function taskLabel({ task, call }: Awaited): Target {
  if (task.description) return { kind: "text", text: task.description };
  const command = call?.name === "Bash" && typeof call.input.command === "string" ? call.input.command : "";
  const line = command.trim().split("\n")[0];
  if (line) return { kind: "code", text: line };
  const kind = taskKind(task.type);
  return { kind: "text", text: kind ? `a ${kind}` : "a task" };
}

/**
 * What the agent is doing now, while its session is at it: the step it is at, the call it waits
 * on, or else that it works. Nothing while it waits for people or has stopped.
 */
export function nowOf({
  status,
  items,
  live,
  stall,
  cwd,
  reason,
  resumeAt,
  waiting,
}: {
  status?: G.SessionStatus;
  items: Item[];
  live: LiveTurn;
  stall?: Stall;
  cwd?: string;
  /** What the session's status says beyond it (`Session.status_reason`). */
  reason?: string;
  /** When a paused session resumes. */
  resumeAt?: number;
  /** What it waits for in the background, while it does ([`waitingOf`]). */
  waiting?: Waiting;
}): Now | undefined {
  if (stall) return { kind: "stalled", text: stallLabel(stall), since: stall.since };
  switch (status) {
    case "queued":
    case "provisioning":
    case "preparing": {
      const setup = items.findLast((it) => it.kind === "setup");
      const step = setup?.kind === "setup" && !setup.done ? setup.steps[setup.steps.length - 1] : undefined;
      return {
        kind: "setup",
        text: step?.reason ?? (status === "queued" ? "Waiting for a runner" : "Setting up the sandbox"),
        since: setup?.ts,
      };
    }
    case "paused":
      return { kind: "paused", text: reason ?? "Paused", since: resumeAt };
    case "stopping":
      return { kind: "stopping", text: "Stopping" };
    case "idle": {
      // Waiting for its background work, which its status says, rather than for a message: one
      // task the transcript knows by what it does, and since when it runs.
      if (!reason || reason === "Waiting for a message") return undefined;
      const one = waiting?.tasks.length === 1 && !waiting.more ? waiting.tasks[0] : undefined;
      return one
        ? { kind: "background", text: "Waiting for", target: taskLabel(one), since: one.task.ts }
        : { kind: "background", text: reason };
    }
    case "running":
      break;
    default:
      return undefined;
  }
  const block = live.blocks.findLast((b) => !b.done);
  if (block?.type === "text") return { kind: "writing", text: "Writing" };
  if (block?.type === "thinking") return { kind: "thinking", text: "Thinking", tokens: block.tokens };
  if (block?.type === "tool_use") return { kind: "preparing", text: preparing(block.name), tool: block.name };
  const call = waitingOn(items);
  if (call) {
    const { verb, target } = doing(call.name, call.input, cwd);
    return {
      kind: "tool",
      text: verb,
      target,
      tool: call.name,
      since: call.ts,
      seconds: live.tool?.id === call.id ? live.tool.seconds : undefined,
    };
  }
  return { kind: "working", text: "Working", since: turnStart(items) };
}

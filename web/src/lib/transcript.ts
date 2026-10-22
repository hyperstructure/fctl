// Turns the session event log into transcript items, incrementally.
//
// The builder keeps item objects stable: adding events copies the items
// array but only replaces items that changed, so memoized rows re-render
// only when their own content moves.

import type * as G from "@/gen";
import {
  type ClaudeMessage,
  isAssistant,
  isInit,
  isRateLimit,
  isResult,
  isTask,
  isUserTool,
  type Json,
  resultText,
  type TaskMessage,
  type ToolResultBlock,
} from "./claude-types";
import { branchChanges } from "./git";

interface Base {
  key: string;
  seq: number;
  ts: number;
}

export type Item =
  | (Base & {
      kind: "user";
      author?: G.UserRef;
      text: string;
      source: G.MessageSource;
      attachments?: G.Attachment[];
      /** Local previews of a message not stored yet, by attachment id. */
      previews?: Record<string, string>;
      pending?: boolean;
    })
  | (Base & { kind: "prose"; text: string })
  | (Base & { kind: "thinking"; text: string })
  | (Base & {
      kind: "tool";
      id: string;
      name: string;
      input: Record<string, Json>;
      result?: { text: string; isError: boolean; structured?: unknown };
      /** Tool calls a subagent made inside this one. */
      children: number;
      childLast?: string;
      /** The subagent this call started, once it does something. */
      agent?: Subagent;
    })
  | (Base & { kind: "turn"; durationMs: number; ok: boolean; text?: string })
  | (Base & {
      kind: "setup";
      steps: { status: G.SessionStatus; reason?: string; ts: number }[];
      logs: G.LogLine[];
      done: boolean;
      endTs: number;
    })
  | (Base & { kind: "status"; status: G.SessionStatus; reason?: string; resumeAt?: number })
  | (Base & { kind: "logs"; lines: G.LogLine[] })
  | (Base & { kind: "notice"; notice: G.Notice })
  /** The repositories whose branch changed (`branchChanges`), in a state. */
  | (Base & { kind: "git"; git: G.GitState; repos: (string | undefined)[] })
  | (Base & { kind: "remote"; rc: G.RemoteControlState })
  | (Base & { kind: "interrupt"; by?: G.UserRef })
  | (Base & { kind: "init"; model: string; version?: string })
  | (Base & { kind: "limit"; status: string; resetsAt?: number; limit?: string });

export type Tool = Extract<Item, { kind: "tool" }>;

/** What an agent does, in the transcript or under the call that started a subagent. */
export type Step = Extract<Item, { kind: "prose" | "thinking" | "tool" }>;

/**
 * A subagent (the `Agent` tool, `Task` before it). Its messages name the
 * call that started it (`parent_tool_use_id`), and Claude Code says when it
 * starts and ends (`system/task_started`, `system/task_notification`).
 */
export interface Subagent {
  /** Its prose, thinking and tool calls, in order. */
  steps: Step[];
  /** It works in the background: its call returns at once, and it goes on after the turn. */
  background?: boolean;
  /** How it ended: `completed`, `failed`, `stopped`. */
  ended?: string;
  /** Its last words, as Claude Code passed them on when it ended. */
  report?: string;
  /** How long it worked, once it ended. */
  durationMs?: number;
}

/** Whether a tool call starts a subagent, whose steps it shows. */
export const isSubagent = (t: Tool) => t.name === "Agent" || t.name === "Task" || !!t.agent;

/**
 * Whether the subagent a call started still works, as far as the page knows:
 * in the turn its call is in, or, in the background, while Claude Code runs
 * (`up`).
 */
export function subagentRuns(t: Tool, running: boolean, up: boolean): boolean {
  if (t.agent?.ended) return false;
  if (t.agent?.background) return up;
  return !t.result && running;
}

/**
 * A subagent's final report: what its call returned, or, from the
 * background, what Claude Code passed on when it ended.
 */
export function subagentReport(t: Tool): string | undefined {
  const r = t.result;
  if (r?.isError) return undefined;
  const s = r?.structured as { isAsync?: boolean; content?: unknown } | undefined;
  const async = t.agent?.background || t.input.run_in_background === true || s?.isAsync === true;
  if (!r || async) return t.agent?.report;
  // Claude Code's own copy of the result has the report alone, without its notes for Claude.
  const parts = Array.isArray(s?.content)
    ? s.content.flatMap((c: { text?: unknown }) => (typeof c?.text === "string" ? [c.text] : []))
    : [];
  return parts.join("\n\n").trim() || t.agent?.report || r.text;
}

/** Statuses shown in the transcript after setup (others live in the header). */
const SHOWN_STATUSES = new Set<G.SessionStatus>(["paused", "stopped", "failed", "queued"]);

export class TranscriptBuilder {
  /** Only ever appended to or replaced in place: `tools` holds indices into it. */
  items: Item[] = [];
  latestGit?: G.GitState;
  model?: string;
  /** Where each tool call is: in the transcript, or in the steps of the subagent `parent` started. */
  private tools = new Map<string, { index: number; parent?: string }>();
  /** The calls of subagents that Claude Code reported started, by task id. */
  private tasks = new Map<string, string>();
  /** The calls of subagents that started and haven't ended. */
  private open = new Set<string>();
  private seen = new Set<number>();
  private pullRequests = new Set<string>();
  private setupOpen = true;

  /** Appends events (in seq order); returns the new items array. */
  add(events: G.SessionEvent[]): Item[] {
    if (!events.length) return this.items;
    const items = this.items.slice();
    for (const ev of events) {
      if (this.seen.has(ev.seq)) continue;
      this.seen.add(ev.seq);
      // Events come from the sandbox too, where anything can pose as the
      // agent: one this app can't read is left out, not the page broken.
      try {
        this.apply(items, ev);
      } catch {
        // Left out.
      }
    }
    this.items = items;
    return items;
  }

  private replace(items: Item[], i: number, next: Item) {
    items[i] = next;
  }

  /** The tool call `id`, in the transcript or in a subagent's steps. */
  private tool(items: Item[], id: string): Tool | undefined {
    const at = this.tools.get(id);
    if (!at) return undefined;
    const list = at.parent === undefined ? items : this.tool(items, at.parent)?.agent?.steps;
    const t = list?.[at.index];
    return t?.kind === "tool" ? t : undefined;
  }

  /** Replaces the tool call `id` with what `fn` makes of it, and the calls of the subagents it is in. */
  private updateTool(items: Item[], id: string, fn: (t: Tool) => Tool) {
    const at = this.tools.get(id);
    const t = this.tool(items, id);
    if (!at || !t) return;
    if (at.parent === undefined) {
      this.replace(items, at.index, fn(t));
      return;
    }
    this.updateAgent(items, at.parent, (a) => ({ ...a, steps: a.steps.with(at.index, fn(t)) }));
  }

  private updateAgent(items: Item[], id: string, fn: (a: Subagent) => Subagent) {
    this.updateTool(items, id, (t) => ({ ...t, agent: fn(t.agent ?? { steps: [] }) }));
  }

  /** The last step in the transcript, or of the subagent `parent` started. */
  private lastStep(items: Item[], parent: string | null): Item | undefined {
    const list = parent ? this.tool(items, parent)?.agent?.steps : items;
    return list?.[list.length - 1];
  }

  /** Adds a step to the transcript, or to the subagent `parent` started. */
  private addStep(items: Item[], parent: string | null, step: Step) {
    if (!parent) {
      if (step.kind === "tool") this.tools.set(step.id, { index: items.length });
      items.push(step);
      return;
    }
    const call = this.tool(items, parent);
    if (!call) return;
    if (step.kind === "tool") this.tools.set(step.id, { index: call.agent?.steps.length ?? 0, parent });
    this.updateTool(items, parent, (t) => ({
      ...t,
      agent: { ...t.agent, steps: [...(t.agent?.steps ?? []), step] },
      children: step.kind === "tool" ? t.children + 1 : t.children,
      childLast: step.kind === "tool" ? step.name : t.childLast,
    }));
  }

  private replaceLastStep(items: Item[], parent: string | null, step: Step) {
    if (!parent) this.replace(items, items.length - 1, step);
    else this.updateAgent(items, parent, (a) => ({ ...a, steps: a.steps.with(-1, step) }));
  }

  /** Subagents still working when their Claude Code stops end with it. */
  private stopSubagents(items: Item[]) {
    for (const id of this.open) this.updateAgent(items, id, (a) => ({ ...a, ended: a.ended ?? "stopped" }));
    this.open.clear();
  }

  private setup(items: Item[], ev: G.SessionEvent): Extract<Item, { kind: "setup" }> | undefined {
    if (!this.setupOpen) return undefined;
    const i = items.findIndex((x) => x.kind === "setup");
    if (i >= 0) return items[i] as Extract<Item, { kind: "setup" }>;
    const s: Item = {
      kind: "setup",
      key: `setup-${ev.seq}`,
      seq: ev.seq,
      ts: ev.ts,
      steps: [],
      logs: [],
      done: false,
      endTs: ev.ts,
    };
    items.push(s);
    return s;
  }

  private updateSetup(
    items: Item[],
    fn: (s: Extract<Item, { kind: "setup" }>) => Extract<Item, { kind: "setup" }>,
  ) {
    const i = items.findIndex((x) => x.kind === "setup");
    if (i >= 0) this.replace(items, i, fn(items[i] as Extract<Item, { kind: "setup" }>));
  }

  private closeSetup(items: Item[], ts: number) {
    if (!this.setupOpen) return;
    this.setupOpen = false;
    this.updateSetup(items, (s) => ({ ...s, done: true, endTs: ts }));
  }

  private apply(items: Item[], ev: G.SessionEvent) {
    const base = { seq: ev.seq, ts: ev.ts };
    switch (ev.kind) {
      case "user_message":
        items.push({
          ...base,
          kind: "user",
          key: `u-${ev.seq}`,
          author: ev.data.author,
          text: ev.data.text,
          source: ev.data.source,
          attachments: ev.data.attachments,
        });
        return;
      case "status": {
        const st = ev.data.status;
        if (this.setupOpen && (st === "queued" || st === "provisioning" || st === "preparing")) {
          if (!this.setup(items, ev)) return;
          this.updateSetup(items, (s) => ({
            ...s,
            steps: [...s.steps, { status: st, reason: ev.data.reason, ts: ev.ts }],
            endTs: ev.ts,
          }));
          return;
        }
        if (st === "running" || st === "idle") {
          this.closeSetup(items, ev.ts);
          return;
        }
        if (SHOWN_STATUSES.has(st)) {
          if (st === "queued") this.setupOpen = true; // a resume sets up again
          items.push({
            ...base,
            kind: "status",
            key: `s-${ev.seq}`,
            status: st,
            reason: ev.data.reason,
            resumeAt: ev.data.resume_at,
          });
          if (st === "failed" || st === "stopped") {
            this.closeSetup(items, ev.ts);
            this.stopSubagents(items);
          }
        }
        return;
      }
      case "log": {
        if (this.setupOpen && this.setup(items, ev)) {
          this.updateSetup(items, (s) => ({ ...s, logs: [...s.logs, ev.data], endTs: ev.ts }));
          return;
        }
        const last = items[items.length - 1];
        if (last?.kind === "logs") {
          this.replace(items, items.length - 1, { ...last, lines: [...last.lines, ev.data] });
        } else {
          items.push({ ...base, kind: "logs", key: `l-${ev.seq}`, lines: [ev.data] });
        }
        return;
      }
      case "notice":
        // A restarted Claude Code doesn't follow the last one's subagents.
        if (ev.data.code === "claude_restarted") this.stopSubagents(items);
        items.push({ ...base, kind: "notice", key: `n-${ev.seq}`, notice: ev.data });
        return;
      case "git": {
        // A line when a branch changed: a commit, a push or a new pull request.
        const repos = branchChanges(this.latestGit, ev.data, this.pullRequests);
        this.latestGit = ev.data;
        if (repos.length) items.push({ ...base, kind: "git", key: `g-${ev.seq}`, git: ev.data, repos });
        return;
      }
      case "remote_control":
        items.push({ ...base, kind: "remote", key: `r-${ev.seq}`, rc: ev.data });
        return;
      case "interrupt":
        items.push({ ...base, kind: "interrupt", key: `i-${ev.seq}`, by: ev.data.by });
        return;
      case "claude":
        this.claude(items, ev, ev.data as ClaudeMessage);
        return;
    }
  }

  private claude(items: Item[], ev: G.SessionEvent, m: ClaudeMessage) {
    const base = { seq: ev.seq, ts: ev.ts };
    if (isInit(m)) {
      this.closeSetup(items, ev.ts);
      this.model = m.model;
      if (!items.some((x) => x.kind === "init")) {
        items.push({
          ...base,
          kind: "init",
          key: `init-${ev.seq}`,
          model: m.model,
          version: m.claude_code_version,
        });
      }
      return;
    }
    if (isAssistant(m)) {
      // A subagent's work goes under the call that started it, when that is loaded.
      const parent = m.parent_tool_use_id;
      if (parent && !this.tools.has(parent)) return;
      if (!parent) this.closeSetup(items, ev.ts);
      m.message.content.forEach((b, n) => {
        const key = `${m.message.id}-${ev.seq}-${n}`;
        if (b.type === "text" && "text" in b && b.text.trim()) {
          const last = this.lastStep(items, parent);
          // Consecutive text blocks of one message read as one passage.
          if (last?.kind === "prose" && last.key.startsWith(m.message.id)) {
            this.replaceLastStep(items, parent, { ...last, text: `${last.text}\n\n${b.text}` });
          } else {
            this.addStep(items, parent, { ...base, kind: "prose", key, text: b.text });
          }
        } else if (b.type === "thinking" && "thinking" in b && b.thinking.trim()) {
          this.addStep(items, parent, { ...base, kind: "thinking", key, text: b.thinking });
        } else if (b.type === "tool_use" && "id" in b) {
          this.addStep(items, parent, {
            ...base,
            kind: "tool",
            key,
            id: b.id,
            name: b.name,
            input: b.input ?? {},
            children: 0,
          });
        }
      });
      if (!parent && m.error === "rate_limit") {
        items.push({ ...base, kind: "limit", key: `lim-${ev.seq}`, status: "rejected" });
      }
      return;
    }
    if (isUserTool(m)) {
      // Results of the agent's tool calls and of its subagents' (whose prompt this skips).
      const content = m.message.content;
      if (typeof content === "string") return;
      for (const b of content) {
        if (b.type !== "tool_result") continue;
        const r = b as ToolResultBlock;
        this.updateTool(items, r.tool_use_id, (t) => ({
          ...t,
          result: { text: resultText(r.content), isError: !!r.is_error, structured: m.tool_use_result },
        }));
        // A subagent whose call returned has ended, unless it works in the background.
        if (!this.tool(items, r.tool_use_id)?.agent?.background) this.open.delete(r.tool_use_id);
      }
      return;
    }
    if (isTask(m)) {
      this.task(items, m);
      return;
    }
    if (isResult(m)) {
      items.push({
        ...base,
        kind: "turn",
        key: `t-${ev.seq}`,
        durationMs: m.duration_ms,
        ok: !m.is_error,
        text: m.result,
      });
      return;
    }
    if (isRateLimit(m)) {
      const info = m.rate_limit_info;
      if (info.status !== "allowed") {
        items.push({
          ...base,
          kind: "limit",
          key: `lim-${ev.seq}`,
          status: info.status,
          resetsAt: info.resetsAt ? (info.resetsAt < 1e11 ? info.resetsAt * 1000 : info.resetsAt) : undefined,
          limit: info.rateLimitType,
        });
      }
    }
  }

  /** Follows Claude Code's word on a subagent: that it started, in the background or not, and how it ended. */
  private task(items: Item[], m: TaskMessage) {
    const id = m.tool_use_id ?? (m.task_id ? this.tasks.get(m.task_id) : undefined);
    const t = id ? this.tool(items, id) : undefined;
    if (!id || !t) return;
    if (m.subtype === "task_started") {
      // Shell commands and monitors are tasks too.
      if (m.task_type !== "local_agent" && !isSubagent(t)) return;
      if (m.task_id) this.tasks.set(m.task_id, id);
      this.open.add(id);
      this.updateAgent(items, id, (a) => ({ ...a, background: m.is_backgrounded === true }));
    } else if (m.subtype === "task_notification" && t.agent) {
      this.open.delete(id);
      const ended = m.status ?? "completed";
      this.updateAgent(items, id, (a) => ({
        ...a,
        ended,
        report: (ended === "completed" && m.summary) || a.report,
        durationMs: m.usage?.duration_ms ?? a.durationMs,
      }));
    }
  }
}

/** The text being streamed right now (from `stream_event`s), per block. */
export interface LiveBlock {
  index: number;
  type: "text" | "thinking" | "tool_use";
  text: string;
  name?: string;
  done: boolean;
}

export interface LiveTurn {
  messageId?: string;
  blocks: LiveBlock[];
  /** Tool heartbeats: which tool is running and for how long. */
  tool?: { id: string; name: string; seconds: number };
  updatedAt: number;
}

export const emptyLive = (): LiveTurn => ({ blocks: [], updatedAt: 0 });

/** Applies one `stream_event` or `tool_progress` to the live turn (returns a new object when it changed). */
export function applyStream(
  live: LiveTurn,
  e: {
    type: string;
    event?: unknown;
    parent_tool_use_id?: string | null;
    tool_use_id?: string;
    tool_name?: string;
    elapsed_time_seconds?: number;
  },
): LiveTurn {
  if (e.parent_tool_use_id) return live;
  const now = Date.now();
  if (e.type === "tool_progress") {
    return {
      ...live,
      tool: { id: e.tool_use_id ?? "", name: e.tool_name ?? "tool", seconds: e.elapsed_time_seconds ?? 0 },
      updatedAt: now,
    };
  }
  const ev = e.event as
    | {
        type: string;
        index?: number;
        message?: { id: string };
        content_block?: { type: string; name?: string };
        delta?: { type: string; text?: string; thinking?: string; partial_json?: string };
      }
    | undefined;
  if (!ev) return live;
  switch (ev.type) {
    case "message_start":
      return { messageId: ev.message?.id, blocks: [], updatedAt: now };
    case "content_block_start": {
      const t = ev.content_block?.type;
      if (t !== "text" && t !== "thinking" && t !== "tool_use") return live;
      return {
        ...live,
        tool: t === "tool_use" ? undefined : live.tool,
        blocks: [
          ...live.blocks,
          { index: ev.index ?? 0, type: t, text: "", name: ev.content_block?.name, done: false },
        ],
        updatedAt: now,
      };
    }
    case "content_block_delta": {
      const d = ev.delta;
      const piece = d?.text ?? d?.thinking ?? d?.partial_json ?? "";
      if (!piece) return live;
      return {
        ...live,
        blocks: live.blocks.map((b) =>
          b.index === ev.index && !b.done ? { ...b, text: b.text + piece } : b,
        ),
        updatedAt: now,
      };
    }
    case "content_block_stop":
      return {
        ...live,
        blocks: live.blocks.map((b) => (b.index === ev.index ? { ...b, done: true } : b)),
        updatedAt: now,
      };
    default:
      return live;
  }
}

/** Drops finished live blocks once their stored message arrived. */
export function settleLive(live: LiveTurn, events: G.SessionEvent[]): LiveTurn {
  if (!live.blocks.length && !live.tool) return live;
  let next = live;
  for (const ev of events) {
    if (ev.kind !== "claude") continue;
    const m = ev.data as ClaudeMessage;
    if (isAssistant(m) && !m.parent_tool_use_id) {
      next = { ...next, blocks: next.blocks.filter((b) => !b.done) };
    }
    // A subagent's tool results don't end the agent's own tool call.
    if ((isUserTool(m) && !m.parent_tool_use_id) || isResult(m)) next = { ...next, tool: undefined };
    if (isResult(m)) next = { ...next, blocks: [] };
  }
  return next;
}

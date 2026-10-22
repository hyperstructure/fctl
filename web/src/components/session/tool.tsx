// A tool call in the transcript: one quiet line that opens into its details.
// A subagent's opens into what it was asked and what it did, step by step.

import { diffLines } from "diff";
import {
  Bot,
  ChevronRight,
  FilePen,
  FilePlus2,
  FileText,
  FolderSearch,
  Globe,
  ListChecks,
  MessageSquareText,
  NotebookPen,
  Search,
  SquareTerminal,
  Wrench,
} from "lucide-react";
import { memo, type ReactNode, useMemo, useState } from "react";
import { CodeBlock, Markdown } from "@/components/markdown";
import { Spinner } from "@/components/ui";
import type { Json } from "@/lib/claude-types";
import { cn } from "@/lib/cn";
import { duration } from "@/lib/format";
import { langOf } from "@/lib/highlight";
import {
  isSubagent,
  type Step,
  type Subagent,
  subagentReport,
  subagentRuns,
  type Tool,
} from "@/lib/transcript";
import { Thinking } from "./thinking";

const ICONS: Record<string, ReactNode> = {
  Bash: <SquareTerminal />,
  Read: <FileText />,
  Write: <FilePlus2 />,
  Edit: <FilePen />,
  MultiEdit: <FilePen />,
  NotebookEdit: <NotebookPen />,
  Grep: <Search />,
  Glob: <FolderSearch />,
  WebFetch: <Globe />,
  WebSearch: <Search />,
  Task: <Bot />,
  Agent: <Bot />,
  TodoWrite: <ListChecks />,
};

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

function shortPath(p: string) {
  const parts = p.split("/");
  return parts.length > 3 ? `…/${parts.slice(-3).join("/")}` : p;
}

/** The verb and the object of a tool call, for its one-line summary. */
export function describe(name: string, input: Record<string, Json>): { verb: string; target: string } {
  switch (name) {
    case "Bash":
      return {
        verb: "Ran",
        target: str(input.description) || str(input.command).split("\n")[0] || "a command",
      };
    case "Read":
      return { verb: "Read", target: shortPath(str(input.file_path)) };
    case "Write":
      return { verb: "Wrote", target: shortPath(str(input.file_path)) };
    case "Edit":
    case "MultiEdit":
      return { verb: "Edited", target: shortPath(str(input.file_path)) };
    case "Grep":
      return { verb: "Searched for", target: str(input.pattern) };
    case "Glob":
      return { verb: "Listed", target: str(input.pattern) };
    case "WebFetch":
      return { verb: "Fetched", target: str(input.url).replace(/^https?:\/\//, "") };
    case "WebSearch":
      return { verb: "Searched the web for", target: str(input.query) };
    case "Task":
    case "Agent":
      return { verb: "Delegated", target: str(input.description) || "a task" };
    case "TodoWrite":
      return { verb: "Updated", target: "the plan" };
    default:
      return { verb: name.replace(/^mcp__/, "").replace(/__/g, " · "), target: "" };
  }
}

function DiffView({ before, after, path }: { before: string; after: string; path?: string }) {
  const parts = useMemo(() => diffLines(before, after), [before, after]);
  let add = 0;
  let del = 0;
  for (const p of parts) {
    if (p.added) add += p.count ?? 0;
    if (p.removed) del += p.count ?? 0;
  }
  return (
    <div className="overflow-hidden rounded-md border border-line bg-sunken">
      {path && (
        <div className="flex items-center justify-between border-b border-line px-3 py-1.5 font-mono text-[11.5px] text-ink-3">
          <span className="truncate">{path}</span>
          <span className="shrink-0 tabular-nums">
            <span className="text-accent">+{add}</span> <span className="text-danger">−{del}</span>
          </span>
        </div>
      )}
      <pre className="max-h-[420px] overflow-auto py-1.5 font-mono text-[12.25px] leading-[1.6]">
        {parts.map((p, i) =>
          p.value
            .replace(/\n$/, "")
            .split("\n")
            .map((line, j) => (
              <div
                key={`${i}-${j}`}
                className={cn(
                  "px-3 whitespace-pre",
                  p.added && "bg-accent-soft text-ink",
                  p.removed && "bg-danger-soft text-ink-2",
                  !p.added && !p.removed && "text-ink-3",
                )}
              >
                <span className="mr-3 inline-block w-2 select-none text-ink-4">
                  {p.added ? "+" : p.removed ? "−" : " "}
                </span>
                {line || " "}
              </div>
            )),
        )}
      </pre>
    </div>
  );
}

function Output({ text, error }: { text: string; error: boolean }) {
  const [all, setAll] = useState(false);
  const lines = text.split("\n");
  const long = lines.length > 24;
  const shown = all || !long ? text : lines.slice(0, 24).join("\n");
  if (!text.trim()) return <div className="text-[12.5px] text-ink-4 italic">No output</div>;
  return (
    <div
      className={cn(
        "overflow-hidden rounded-md border bg-sunken",
        error ? "border-danger/25" : "border-line",
      )}
    >
      <pre
        className={cn(
          "max-h-[480px] overflow-auto px-3 py-2 font-mono text-[12.25px] leading-[1.6] whitespace-pre-wrap break-words",
          error ? "text-danger" : "text-ink-2",
        )}
      >
        {shown}
      </pre>
      {long && (
        <button
          type="button"
          onClick={() => setAll(!all)}
          className="w-full border-t border-line py-1 text-[11.5px] text-ink-4 hover:text-ink"
        >
          {all ? "Show less" : `Show ${lines.length - 24} more lines`}
        </button>
      )}
    </div>
  );
}

function Todos({ input }: { input: Record<string, Json> }) {
  const todos = Array.isArray(input.todos)
    ? (input.todos as { content?: string; status?: string; activeForm?: string }[])
    : [];
  return (
    <ul className="flex flex-col gap-1.5 py-1">
      {todos.map((t, i) => (
        <li key={i} className="flex items-start gap-2.5 text-[13.5px]">
          <span
            className={cn(
              "mt-[3px] flex size-3.5 shrink-0 items-center justify-center rounded-full border",
              t.status === "completed"
                ? "border-accent bg-accent text-accent-ink"
                : t.status === "in_progress"
                  ? "border-blue"
                  : "border-line-strong",
            )}
          >
            {t.status === "completed" && (
              <svg viewBox="0 0 12 12" className="size-2.5" aria-hidden="true">
                <path
                  d="M2.5 6.2 5 8.5l4.5-5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            )}
            {t.status === "in_progress" && (
              <span className="size-1.5 animate-pulse-soft rounded-full bg-blue" />
            )}
          </span>
          <span
            className={cn(
              t.status === "completed" ? "text-ink-3 line-through decoration-ink-4" : "text-ink-2",
            )}
          >
            {t.content}
          </span>
        </li>
      ))}
    </ul>
  );
}

function Details({ t }: { t: Tool }) {
  const input = t.input;
  const result = t.result;
  switch (t.name) {
    case "Edit":
      return (
        <div className="flex flex-col gap-2">
          <DiffView
            before={str(input.old_string)}
            after={str(input.new_string)}
            path={str(input.file_path)}
          />
          {result?.isError && <Output text={result.text} error />}
        </div>
      );
    case "MultiEdit": {
      const edits = Array.isArray(input.edits)
        ? (input.edits as { old_string?: string; new_string?: string }[])
        : [];
      return (
        <div className="flex flex-col gap-2">
          {edits.map((e, i) => (
            <DiffView
              key={i}
              before={e.old_string ?? ""}
              after={e.new_string ?? ""}
              path={i === 0 ? str(input.file_path) : undefined}
            />
          ))}
        </div>
      );
    }
    case "Write": {
      const content = str(input.content);
      const lines = content.split("\n");
      return (
        <div className="flex flex-col gap-1.5">
          <div className="font-mono text-[11.5px] text-ink-3">
            {str(input.file_path)} · {lines.length} lines
          </div>
          <CodeBlock
            code={lines.slice(0, 80).join("\n") + (lines.length > 80 ? "\n…" : "")}
            lang={langOf(str(input.file_path))}
          />
        </div>
      );
    }
    case "Bash":
      return (
        <div className="flex flex-col gap-2">
          <CodeBlock code={str(input.command)} lang="bash" />
          {result && <Output text={result.text} error={result.isError} />}
        </div>
      );
    case "TodoWrite":
      return <Todos input={input} />;
    case "Read":
      return result ? (
        <div className="text-[12.5px] text-ink-3">
          {result.isError ? (
            <Output text={result.text} error />
          ) : (
            `${result.text.split("\n").length} lines read`
          )}
        </div>
      ) : null;
    default:
      return (
        <div className="flex flex-col gap-2">
          <CodeBlock code={JSON.stringify(input, null, 2)} lang="json" />
          {result && <Output text={result.text} error={result.isError} />}
        </div>
      );
  }
}

/** What the agent asked of a subagent: a line that opens into it. */
function Prompt({ text, agent }: { text: string; agent: string }) {
  const [open, setOpen] = useState(false);
  if (!text.trim()) return null;
  return (
    <div className="py-1">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="flex items-center gap-2 text-[12.5px] text-ink-4 transition-colors hover:text-ink-2"
      >
        <MessageSquareText className="size-3.5" />
        Prompt
        {agent && <span className="font-mono text-[11.5px]">· {agent}</span>}
        <ChevronRight className={cn("size-3 transition-transform", open && "rotate-90")} />
      </button>
      {open && (
        <div className="mt-1.5 mb-1 max-h-[420px] animate-rise overflow-auto rounded-md border border-line bg-sunken px-3 py-2 text-[13px] leading-relaxed whitespace-pre-wrap wrap-anywhere text-ink-2">
          {text}
        </div>
      )}
    </div>
  );
}

function Prose({ text }: { text: string }) {
  return (
    <div className="py-1.5 text-[13.5px] text-ink-2">
      <Markdown text={text} className="prose-note" />
    </div>
  );
}

/** How a subagent ended, and after how long. */
function Ended({ agent }: { agent?: Subagent }) {
  if (!agent?.ended) return null;
  const took = agent.durationMs ? duration(agent.durationMs) : "";
  const failed = agent.ended === "failed";
  const text =
    agent.ended === "completed"
      ? `Finished${took && ` in ${took}`}`
      : `${failed ? "Failed" : "Stopped"}${took && ` after ${took}`}`;
  return <div className={cn("py-1 text-[11.5px]", failed ? "text-danger" : "text-ink-4")}>{text}</div>;
}

/**
 * What a subagent was asked and what it did: its prose and tool calls, as
 * they come while it works, then its report.
 */
function SubagentDetails({ t, running, up }: { t: Tool; running: boolean; up: boolean }) {
  const steps = t.agent?.steps ?? [];
  const last = steps[steps.length - 1];
  const report = subagentReport(t);
  // From the background, its last words are its report.
  const reported = report && !(last?.kind === "prose" && last.text.trim() === report.trim());
  const waiting = running && !(last?.kind === "tool" && !last.result);
  return (
    <div className="flex flex-col">
      <Prompt text={str(t.input.prompt)} agent={str(t.input.subagent_type)} />
      {(steps.length > 0 || reported || waiting) && (
        <div className="flex flex-col border-l border-line pl-3">
          {steps.map((s) => (
            <StepView key={s.key} s={s} running={running} up={up} />
          ))}
          {reported && <Prose text={report} />}
          {waiting && <div className="text-shimmer py-1.5 text-[12.5px] text-ink-3">Working</div>}
        </div>
      )}
      {t.result?.isError && (
        <div className="mt-1.5">
          <Output text={t.result.text} error />
        </div>
      )}
      <Ended agent={t.agent} />
    </div>
  );
}

function StepView({ s, running, up }: { s: Step; running: boolean; up: boolean }) {
  if (s.kind === "tool") return <ToolRow t={s} running={running} up={up} />;
  if (s.kind === "thinking") return <Thinking text={s.text} />;
  return <Prose text={s.text} />;
}

export const ToolRow = memo(function ToolRow({
  t,
  running,
  up = false,
  seconds,
}: {
  t: Tool;
  /** Whether the agent works on the turn this call is in, as far as the page knows. */
  running: boolean;
  /** Whether its Claude Code runs, as far as the page knows: subagents in the background work on after their turn. */
  up?: boolean;
  seconds?: number;
}) {
  const [open, setOpen] = useState(t.name === "TodoWrite");
  const { verb, target } = describe(t.name, t.input);
  const agent = isSubagent(t);
  const error = t.result?.isError || t.agent?.ended === "failed";
  const pending = agent ? subagentRuns(t, running, up) : !t.result && running;
  return (
    <div className="group/tool relative">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className={cn(
          "flex w-full items-center gap-2.5 rounded-md py-1 pr-2 pl-1 text-left text-[13.5px] transition-colors",
          "hover:bg-hover/60",
          error ? "text-danger" : "text-ink-2",
        )}
      >
        <span
          className={cn(
            "flex size-6 shrink-0 items-center justify-center rounded-md border transition-colors [&_svg]:size-3.5",
            error
              ? "border-danger/30 bg-danger-soft text-danger"
              : "border-line bg-raised text-ink-3 group-hover/tool:text-ink-2",
          )}
        >
          {pending ? <Spinner className="size-3.5 text-accent" /> : (ICONS[t.name] ?? <Wrench />)}
        </span>
        <span className="min-w-0 flex-1 truncate">
          <span className={cn(pending && "text-shimmer")}>{verb}</span>
          {target && <span className="ml-1.5 font-mono text-[12.5px] text-ink-3">{target}</span>}
        </span>
        {t.children > 0 && (
          <span className="shrink-0 text-[11.5px] text-ink-4">
            {t.children} {t.children === 1 ? "step" : "steps"}
            {t.childLast ? ` · ${t.childLast}` : ""}
          </span>
        )}
        {pending && seconds !== undefined && seconds > 2 && (
          <span className="shrink-0 text-[11.5px] text-ink-4 tabular-nums">{Math.round(seconds)}s</span>
        )}
        <ChevronRight
          className={cn(
            "size-3.5 shrink-0 text-ink-4 transition-transform duration-200",
            open && "rotate-90",
          )}
        />
      </button>
      {open && (
        <div className="mt-1 mb-2 ml-9 animate-rise">
          {agent ? <SubagentDetails t={t} running={pending} up={up} /> : <Details t={t} />}
        </div>
      )}
    </div>
  );
});

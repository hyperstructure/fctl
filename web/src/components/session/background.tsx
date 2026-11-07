// What the agent waits for in the background, in the list the line above the new session page's
// message box opens into (now-line.tsx) while it does: each task Claude Code runs on its own, what
// it is and does, and for how long, which takes the reader to the call that started it.

import { Bot, Hourglass, SquareTerminal } from "lucide-react";
import { type CSSProperties, useEffect } from "react";
import { DotWave } from "@/components/ui";
import { cn } from "@/lib/cn";
import { type Awaited, doing, taskKind, taskLabel, type Waiting } from "@/lib/now";
import type { Tool } from "@/lib/transcript";
import { toolIcon } from "./tool";

/** How long something has run: "12s", "4m 3s", "1h 2m". */
export function ran(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** A command's first line, as a list shows it. */
const commandOf = (call?: Tool) =>
  call?.name === "Bash" && typeof call.input.command === "string"
    ? (call.input.command.trim().split("\n")[0] ?? "")
    : "";

/**
 * What a task's second line says: a shell command's command, what a subagent does at the moment
 * (its newest call), or else what kind of work it is.
 */
function detailOf(a: Awaited, cwd?: string): { text: string; code?: boolean; live?: boolean } | undefined {
  const { task, call } = a;
  const kind = taskKind(task.type);
  const command = commandOf(call);
  // The line names it by its command when it has no description: its kind, then.
  if (command && task.description) return { text: command, code: true };
  const steps = call?.agent?.steps ?? [];
  const last = steps.findLast((s) => s.kind === "tool");
  if (last?.kind === "tool") {
    const { verb, target } = doing(last.name, last.input, cwd);
    return { text: target ? `${verb} ${target.text}` : verb, live: !last.result };
  }
  return kind ? { text: capital(kind) } : undefined;
}

/** A task's icon: its call's, or else its kind's. */
function iconOf({ task, call }: Awaited) {
  if (call) return toolIcon(call.name);
  if (task.type === "local_bash") return <SquareTerminal />;
  if (task.type === "local_agent") return <Bot />;
  return <Hourglass />;
}

function TaskRow({
  a,
  now,
  cwd,
  i,
  onGo,
}: {
  a: Awaited;
  now: number;
  cwd?: string;
  /** Its place in the list, for the rows to settle in one after another. */
  i: number;
  onGo: (call: string) => void;
}) {
  const label = taskLabel(a);
  const detail = detailOf(a, cwd);
  const call = a.call;
  const body = (
    <>
      <span className="flex size-7 shrink-0 items-center justify-center rounded-lg border border-line bg-sunken text-ink-2 [&_svg]:size-3.5">
        {iconOf(a)}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex min-w-0 items-baseline gap-3">
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px] text-ink",
              label.kind === "code" && "font-mono text-[12px]",
            )}
          >
            {label.text}
          </span>
          <span className="shrink-0 text-[11.5px] text-ink-4 tabular-nums">{ran(now - a.task.ts)}</span>
        </span>
        {detail && (
          <span
            className={cn(
              "min-w-0 truncate text-[11.5px] text-ink-4",
              detail.code && "font-mono text-[11px]",
              detail.live && "text-shimmer",
            )}
          >
            {detail.text}
          </span>
        )}
      </span>
    </>
  );
  const style = { "--i": i } as CSSProperties;
  return call ? (
    <li style={style} className="task-row">
      <button
        type="button"
        onClick={() => onGo(call.key)}
        title="Show where it started"
        className="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left transition-colors duration-150 hover:bg-hover focus-visible:outline-offset-0 active:bg-hover"
      >
        {body}
      </button>
    </li>
  ) : (
    <li style={style} className="task-row flex items-center gap-3 px-2 py-2">
      {body}
    </li>
  );
}

/**
 * The list the line opens into: the tasks the agent waits for, oldest first, each going to the
 * call that started it; and those its status counts that started before the activity loaded
 * here, which it looks for further back by itself.
 */
export function BackgroundList({
  id,
  waiting,
  now,
  cwd,
  earlier,
  looking,
  onLookBack,
  onGo,
}: {
  id: string;
  waiting: Waiting;
  /** The time, which moves each second while the list shows. */
  now: number;
  cwd?: string;
  /** The session has activity before what loaded here. */
  earlier: boolean;
  /** It loads some now. */
  looking: boolean;
  /** Loads more of it, while the list looks for tasks; unset once it has looked far enough. */
  onLookBack?: () => void;
  onGo: (call: string) => void;
}) {
  const { tasks, more } = waiting;
  // Tasks it misses started further back: it looks for them there while it shows.
  const look = more > 0 && earlier && !looking ? onLookBack : undefined;
  useEffect(() => {
    look?.();
  }, [look]);
  const total = tasks.length + more;
  return (
    <div
      id={id}
      role="dialog"
      aria-label="In the background"
      className="task-list absolute bottom-full left-0 z-20 mb-2 flex max-h-[min(60vh,26rem)] w-[min(24rem,calc(100vw-2rem))] origin-bottom-left animate-listbox-in flex-col overflow-hidden rounded-xl border border-line-strong bg-raised/95 shadow-[0_18px_60px_rgb(0_0_0/0.35)] backdrop-blur-xl [--listbox-y:6px]"
    >
      <div className="flex items-baseline justify-between gap-3 px-3.5 pt-3 pb-1.5">
        <span className="text-[12.5px] font-medium text-ink">In the background</span>
        <span className="text-[11.5px] text-ink-4 tabular-nums">{total} running</span>
      </div>
      <ul className="min-h-0 flex-auto overflow-y-auto overscroll-contain px-1.5 pb-1.5">
        {tasks.map((a, i) => (
          <TaskRow key={a.task.id} a={a} now={now} cwd={cwd} i={i} onGo={onGo} />
        ))}
        {more > 0 && (
          <li
            style={{ "--i": tasks.length } as CSSProperties}
            className="task-row flex items-center gap-3 px-2 py-2 text-[12px] text-ink-3"
          >
            <span className="flex size-7 shrink-0 items-center justify-center">
              {looking || look ? (
                <DotWave className="text-accent" />
              ) : (
                <Hourglass className="size-3.5 text-ink-4" />
              )}
            </span>
            <span className="min-w-0">
              {looking || look
                ? `Looking further back for ${tasks.length ? `${more} more` : more === 1 ? "it" : "them"}`
                : earlier
                  ? `${more} ${tasks.length ? "more " : ""}started further back`
                  : `${more} ${tasks.length ? "more " : ""}the transcript doesn't show`}
            </span>
          </li>
        )}
      </ul>
      <p className="border-t border-line px-3.5 py-2.5 text-[11.5px] leading-snug text-ink-4">
        When one ends, the agent takes up its result.
      </p>
    </div>
  );
}

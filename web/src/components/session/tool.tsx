// A tool call in the transcript: one quiet line that opens into its details.
// A subagent's opens into what it was asked and what it did, step by step.
// Calls in a row fold into a run: a line that says what they did, which
// opens into them.

import { diffLines } from "diff";
import {
  Bot,
  FilePenLine,
  FilePlus,
  FileText,
  FolderOpen,
  FolderSearch,
  Globe,
  Link,
  ListTodo,
  MessageSquareText,
  NotebookPen,
  Plug,
  Terminal,
  TextSearch,
  Wrench,
} from "lucide-react";
import {
  type CSSProperties,
  createContext,
  type MouseEvent,
  memo,
  type ReactNode,
  useCallback,
  useContext,
  useId,
  useLayoutEffect,
  useMemo,
  useState,
} from "react";
import { CodeBlock, Markdown } from "@/components/markdown";
import { plainClick, Spinner, ThoughtIcon, useLightbox } from "@/components/ui";
import type * as G from "@/gen";
import type { Json } from "@/lib/claude-types";
import { cn } from "@/lib/cn";
import { bytes, duration } from "@/lib/format";
import { langOf } from "@/lib/highlight";
import { formatOf, imageName, KEPT_DAYS, keptImageUrl, keptNow, type ToolImage } from "@/lib/kept-images";
import {
  callsIn,
  changed,
  changedIn,
  describe,
  type Lines,
  parts,
  runTook,
  summary,
  type Target,
  took,
  type Work,
} from "@/lib/runs";
import {
  isSubagent,
  type LiveBlock,
  type Step,
  type Subagent,
  subagentReport,
  subagentRuns,
  type Tool,
} from "@/lib/transcript";
import { useRevealed, useWanted } from "./find";
import { OpenLine, RailIcon, Thread, Tile, Unreadable } from "./lines";
import { Thinking, thinkingNow } from "./thinking";

const ICONS: Record<string, ReactNode> = {
  Bash: <Terminal />,
  Read: <FileText />,
  Write: <FilePlus />,
  Edit: <FilePenLine />,
  MultiEdit: <FilePenLine />,
  NotebookEdit: <NotebookPen />,
  Grep: <TextSearch />,
  Glob: <FolderSearch />,
  LS: <FolderOpen />,
  WebFetch: <Link />,
  WebSearch: <Globe />,
  Task: <Bot />,
  Agent: <Bot />,
  TodoWrite: <ListTodo />,
};

/** A tool's icon: its own, a plug for an MCP server's, else a wrench. */
const toolIcon = (name: string) => ICONS[name] ?? (name.startsWith("mcp__") ? <Plug /> : <Wrench />);

/** The directory Claude Code works in, which paths in the transcript start from. */
export const WorkDir = createContext<string | undefined>(undefined);

/** The session whose transcript this is, which keeps the images its agent read. */
export const SessionImages = createContext<string | undefined>(undefined);

const str = (v: Json | undefined) => (typeof v === "string" ? v : "");

/** How long something has run, by the second: "12 s", "1m 4s". */
const ranFor = (s: number) => (s < 60 ? `${Math.round(s)} s` : duration(s * 1000));

/** A call that took this long (ms) says so on its line. */
const SLOW = 10_000;

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

function Todos({ input, find }: { input: Record<string, Json>; find: string }) {
  const todos = Array.isArray(input.todos)
    ? (input.todos as { content?: string; status?: string; activeForm?: string }[])
    : [];
  return (
    <ul data-find={find} className="flex flex-col gap-1.5 py-1">
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
          <div data-find={`cmd:${t.key}`}>
            <CodeBlock code={str(input.command)} lang="bash" />
          </div>
          {result && <Output text={result.text} error={result.isError} />}
        </div>
      );
    case "TodoWrite":
      return <Todos input={input} find={`todo:${t.key}`} />;
    case "Read":
      return result ? (
        <div className="text-[12.5px] text-ink-3">
          {result.isError ? (
            <Output text={result.text} error />
          ) : result.images?.length ? (
            <ImageFacts images={result.images} />
          ) : (
            `${result.text.split("\n").length} lines read`
          )}
        </div>
      ) : null;
    default:
      return (
        <div className="flex flex-col gap-2">
          <CodeBlock code={JSON.stringify(input, null, 2)} lang="json" />
          {result && (result.text.trim() || !result.images?.length) && (
            <Output text={result.text} error={result.isError} />
          )}
          {result?.images?.length ? (
            <div className="text-[12.5px] text-ink-3">
              <ImageFacts images={result.images} />
            </div>
          ) : null}
        </div>
      );
  }
}

/** What the images a call read are, a line each, and those the session no longer keeps say so. */
function ImageFacts({ images }: { images: ToolImage[] }) {
  return images.map((image, i) => {
    const k = image.kept;
    const facts = [
      `${formatOf(image.mediaType) ?? "An"} image`,
      k && `${k.width}×${k.height}`,
      image.size !== undefined && bytes(image.size),
    ].filter(Boolean);
    return (
      <div key={k?.id ?? i}>
        {facts.join(", ")}
        {k && !keptNow(image) && <span className="text-ink-4"> · deleted after {KEPT_DAYS} days</span>}
      </div>
    );
  });
}

/**
 * An image a call read, small, which a click shows at full size over the page among the transcript's other pictures;
 * with a modifier key, the browser opens it in a new tab.
 */
function ImageThumb({ kept, name, session }: { kept: G.KeptImage; name: string; session: string }) {
  const [failed, setFailed] = useState(false);
  const src = keptImageUrl(session, kept.id);
  const { ref, open, lightbox } = useLightbox<HTMLAnchorElement>(
    failed
      ? undefined
      : {
          file: {
            name,
            media_type: kept.media_type,
            size: kept.size,
            width: kept.width,
            height: kept.height,
          },
          src,
          download: true,
        },
  );
  // Deleted sooner: the session had more than it keeps.
  if (failed) return null;
  const view = (e: MouseEvent) => {
    if (!plainClick(e)) return;
    e.preventDefault();
    open();
  };
  return (
    <>
      <a
        ref={ref}
        href={src}
        target="_blank"
        rel="noreferrer"
        onClick={view}
        aria-label={`View ${name}`}
        title={name}
        className="block cursor-zoom-in overflow-hidden rounded-md border border-line bg-raised/50 transition-[border-color] duration-150 hover:border-line-strong"
      >
        <img
          src={src}
          alt={name}
          width={kept.width}
          height={kept.height}
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
          className="h-24 w-auto max-w-[min(100%,18rem)] object-cover"
        />
      </a>
      {lightbox}
    </>
  );
}

/** The images calls read, while the session keeps them: under a call's line, or under a run's while it is folded. */
function ReadImages({ calls, nested }: { calls: Tool[]; nested?: boolean }) {
  const session = useContext(SessionImages);
  const now = Date.now();
  const shown = calls.flatMap((t) =>
    (t.result?.images ?? []).flatMap((image, i) => {
      const kept = keptNow(image, now);
      return kept ? [{ kept, name: imageName(t.input, kept.media_type, i) }] : [];
    }),
  );
  if (!session || !shown.length) return null;
  return (
    <div className={cn("mt-0.5 mb-1.5 flex flex-wrap items-start gap-2", nested ? "ml-[22px]" : "ml-[34px]")}>
      {shown.map((s) => (
        <ImageThumb key={s.kept.id} kept={s.kept} name={s.name} session={session} />
      ))}
    </div>
  );
}

/** What the agent asked of a subagent: a line that opens into it. `call`: the key of the call that started it. */
function Prompt({ text, agent, call }: { text: string; agent: string; call: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  useRevealed(`prompt:${call}`, () => setOpen(true));
  if (!text.trim()) return null;
  return (
    <div>
      <OpenLine
        icon={
          <RailIcon className="text-ink-4 group-hover/line:text-ink-3">
            <MessageSquareText />
          </RailIcon>
        }
        open={open}
        onToggle={() => setOpen(!open)}
        controls={id}
      >
        <span className="shrink-0 text-ink-3 group-hover/line:text-ink-2">Prompt</span>
        {agent && <span className="min-w-0 truncate font-mono text-[12px] text-ink-4">{agent}</span>}
      </OpenLine>
      {open && (
        <div
          id={id}
          data-find={`prompt:${call}`}
          className="mt-1 mb-2 ml-[34px] max-h-[420px] animate-rise overflow-auto rounded-md border border-line bg-sunken px-3 py-2 text-[13px] leading-relaxed whitespace-pre-wrap wrap-anywhere text-ink-2"
        >
          {text}
        </div>
      )}
    </div>
  );
}

function Prose({ text, find }: { text: string; find: string }) {
  return (
    <div data-find={find} className="py-1.5 text-[13.5px] text-ink-2">
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
 * What a subagent was asked and what it did: its prose and its work, in
 * runs, as they come while it works, then its report.
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
      <Prompt text={str(t.input.prompt)} agent={str(t.input.subagent_type)} call={t.key} />
      <WorkList steps={steps} running={running} up={up} />
      {reported && <Prose text={report} find={`report:${t.key}`} />}
      {waiting && <div className="text-shimmer self-start py-1.5 text-[12.5px] text-ink-3">Working</div>}
      {t.result?.isError && (
        <div className="mt-1.5">
          <Output text={t.result.text} error />
        </div>
      )}
      <Ended agent={t.agent} />
    </div>
  );
}

/** A subagent's work: what it said, and its tool calls and thoughts in runs. */
function WorkList({ steps, running, up }: { steps: Step[]; running: boolean; up: boolean }) {
  const ps = parts(steps);
  return ps.map((p, n) => (
    <Unreadable key={p.key}>
      {p.kind === "run" ? (
        <Run steps={p.steps} running={running} active={running && n === ps.length - 1} up={up} />
      ) : p.item.kind === "tool" ? (
        <ToolRow t={p.item} running={running} up={up} />
      ) : (
        <Prose text={p.item.text} find={`prose:${p.item.key}`} />
      )}
    </Unreadable>
  ));
}

/**
 * What a call is about: a file (its name brighter than its directory, which gives way first), code or words.
 * `find`: what finding in the transcript knows it as.
 */
function TargetText({ target, find }: { target: Target; find: string }) {
  if (target.kind === "path") {
    const cut = target.text.lastIndexOf("/") + 1;
    return (
      <span data-find={find} className="flex min-w-0 font-mono text-[0.925em]">
        {cut > 0 && (
          <span className="min-w-0 shrink-[1000] truncate text-ink-4">{target.text.slice(0, cut)}</span>
        )}
        <span className="min-w-0 truncate text-ink-3">{target.text.slice(cut)}</span>
      </span>
    );
  }
  return (
    <span
      data-find={find}
      className={cn("min-w-0 truncate text-ink-3", target.kind === "code" && "font-mono text-[0.925em]")}
    >
      {target.text}
    </span>
  );
}

/** Lines added and taken out: "+12 −3", in color when there are any. */
function LinesChanged({ lines }: { lines: Lines }) {
  return (
    <span className="font-mono text-[11px]">
      <span className={cn(lines.added > 0 && "text-accent/80")}>+{lines.added}</span>{" "}
      <span className={cn(lines.removed > 0 && "text-danger/80")}>−{lines.removed}</span>
    </span>
  );
}

/** What a line says about itself at its right, in order, with dots between. */
function Facts({ children }: { children: ReactNode[] }) {
  const shown = children.filter(Boolean);
  if (!shown.length) return null;
  return (
    <span className="flex items-center gap-1.5">
      {shown.map((c, i) => (
        <span key={i} className="flex items-center gap-1.5">
          {i > 0 && <span aria-hidden="true">·</span>}
          {c}
        </span>
      ))}
    </span>
  );
}

export const ToolRow = memo(function ToolRow({
  t,
  running,
  up = false,
  seconds: elapsed,
  open: shown,
  onToggle,
  nested,
}: {
  t: Tool;
  /** Whether the agent works on the turn this call is in, as far as the page knows. */
  running: boolean;
  /** Whether its Claude Code runs, as far as the page knows: subagents in the background work on after their turn. */
  up?: boolean;
  /** How long it has run, as Claude Code reports it while it does. */
  seconds?: number;
  /** Whether it is open, when the run it is in keeps that; `onToggle` gets its key. */
  open?: boolean;
  onToggle?: (key: string) => void;
  /** In a run: smaller, its icon bare. */
  nested?: boolean;
}) {
  const [own, setOwn] = useState(t.name === "TodoWrite");
  const open = shown ?? own;
  const id = useId();
  // Its own line, not one a run keeps open: finding opens it to a match in it.
  useRevealed(shown === undefined ? t.key : undefined, () => setOwn(true));
  const { verb, target } = describe(t.name, t.input, useContext(WorkDir));
  const agent = isSubagent(t);
  const error = !!t.result?.isError || t.agent?.ended === "failed";
  const pending = agent ? subagentRuns(t, running, up) : !t.result && running;
  const lines = changed(t);
  const ms = took(t);
  const command = t.name === "Bash" ? str(t.input.command) : "";
  return (
    <div>
      <OpenLine
        icon={
          <Tile nested={nested} error={error}>
            {pending ? <Spinner className="size-3.5 text-accent" /> : toolIcon(t.name)}
          </Tile>
        }
        open={open}
        onToggle={onToggle ? () => onToggle(t.key) : () => setOwn(!open)}
        controls={id}
        nested={nested}
        error={error}
        // A command's line says what it is for; the command shows where the pointer rests.
        title={command && command !== verb ? command.slice(0, 500) : undefined}
        meta={
          <Facts>
            {[
              t.children > 0 &&
                `${t.children} ${t.children === 1 ? "step" : "steps"}${t.childLast ? ` · ${t.childLast}` : ""}`,
              lines && <LinesChanged lines={lines} />,
              pending
                ? elapsed !== undefined && elapsed > 2 && ranFor(elapsed)
                : !agent && ms !== undefined && ms >= SLOW && duration(ms),
            ]}
          </Facts>
        }
      >
        <span
          data-find={target ? undefined : `line:${t.key}`}
          className={cn(target ? "shrink-0" : "min-w-0 truncate", pending && "text-shimmer")}
        >
          {verb}
        </span>
        {target && <TargetText target={target} find={`line:${t.key}`} />}
      </OpenLine>
      {!agent && <ReadImages calls={[t]} nested={nested} />}
      {open &&
        (agent ? (
          <Thread id={id} nested={nested} className="mt-0.5 mb-2 animate-rise">
            <SubagentDetails t={t} running={pending} up={up} />
          </Thread>
        ) : (
          <div id={id} className={cn("mt-1 mb-2 animate-rise", nested ? "ml-[22px]" : "ml-[34px]")}>
            <Details t={t} />
          </div>
        ))}
    </div>
  );
});

const none: ReadonlySet<string> = new Set();

/** A thought or a call that streams in, in the run it goes in, until it is stored. */
function Coming({ block }: { block: Pick<LiveBlock, "type" | "name" | "tokens"> }) {
  return (
    <div className="step-in flex min-h-7 items-center gap-2 py-1 text-[13px] text-ink-3">
      {block.type === "thinking" ? (
        <ThoughtIcon className="size-3.5 shrink-0 text-violet/80" />
      ) : (
        <Spinner className="size-3.5 shrink-0 text-accent" />
      )}
      <span className="text-shimmer">
        {block.type === "thinking" ? thinkingNow(block.tokens) : `Preparing ${block.name ?? "a tool"}`}
      </span>
    </div>
  );
}

/**
 * A run of the agent's work: tool calls and thoughts in a row. Two calls or
 * more fold into a line that says what they did and opens into them; while
 * the run goes on, its newest step shows under that line as it happens.
 * Steps the reader opens stay in view, open or not, until the run folds
 * again.
 */
export const Run = memo(
  function Run({
    steps,
    running,
    active,
    up = false,
    progress,
    coming = [],
  }: {
    steps: Work[];
    /** Whether the agent works on the turn the run is in, as far as the page knows. */
    running: boolean;
    /** The run goes on: it is the newest work of the turn the agent works on. */
    active: boolean;
    /** Whether Claude Code runs, as far as the page knows (for subagents in the background). */
    up?: boolean;
    /** How long its call in progress has run, as Claude Code reports it. */
    progress?: { id: string; seconds: number };
    /** The thoughts and calls streaming in, which will be its next steps. */
    coming?: Pick<LiveBlock, "index" | "type" | "name" | "tokens">[];
  }) {
    const [open, setOpen] = useState(false);
    const [shown, setShown] = useState(none);
    const [opened, setOpened] = useState(none);
    const id = useId();
    // Finding shows the steps it gets to on their own, and opens them for a match in what they open into.
    const keys = useMemo(() => steps.map((s) => s.key), [steps]);
    const wanted = useWanted(keys);
    useLayoutEffect(() => {
      const how = wanted.split(" ");
      const show = keys.filter((_, i) => how[i]);
      if (!show.length) return;
      const open = keys.filter((_, i) => how[i] === "open");
      const add = (more: string[]) => (set: ReadonlySet<string>) =>
        more.every((k) => set.has(k)) ? set : new Set([...set, ...more]);
      setShown(add(show));
      if (open.length) setOpened(add(open));
    }, [wanted, keys]);
    const toggle = useCallback((key: string) => {
      setOpened((o) => {
        const next = new Set(o);
        if (!next.delete(key)) next.add(key);
        return next;
      });
      setShown((s) => (s.has(key) ? s : new Set(s).add(key)));
    }, []);
    const calls = callsIn(steps);
    const step = (s: Work, nested: boolean) => (
      <Unreadable key={s.key}>
        {s.kind === "tool" ? (
          <ToolRow
            t={s}
            running={running}
            up={up}
            seconds={progress?.id === s.id ? progress.seconds : undefined}
            open={opened.has(s.key)}
            onToggle={toggle}
            nested={nested}
          />
        ) : (
          <Thinking
            text={s.text}
            open={opened.has(s.key)}
            onToggle={() => toggle(s.key)}
            nested={nested}
            find={`thought:${s.key}`}
          />
        )}
      </Unreadable>
    );
    // A call on its own is a line of its own.
    if (calls.length < 2) return steps.map((s) => step(s, false));
    // Folded, a run that goes on shows what it does now: its calls in
    // progress, what streams in, or else its newest step.
    const pending = running ? calls.filter((t) => !t.result) : [];
    const now = active ? (pending.length || coming.length ? pending : steps.slice(-1)) : [];
    // Its line says what its calls did, and only while none has, what they do.
    const done = pending.length ? calls.filter((t) => !pending.includes(t)) : calls;
    const newest = new Set(now.map((s) => s.key));
    const visible = open ? steps : steps.filter((s) => shown.has(s.key) || newest.has(s.key));
    const failed = calls.filter((t) => t.result?.isError).length;
    const lines = changedIn(calls);
    const ms = runTook(steps);
    return (
      <div>
        <OpenLine
          icon={<Tile stacked>{toolIcon(calls[0]?.name ?? "")}</Tile>}
          open={open}
          onToggle={() => {
            setOpen(!open);
            if (open) {
              setShown(none);
              setOpened(none);
            }
          }}
          controls={id}
          meta={
            <Facts>
              {[
                lines && <LinesChanged lines={lines} />,
                failed > 0 && <span className="text-danger/80">{failed} failed</span>,
                !active && ms >= 1000 && duration(ms),
              ]}
            </Facts>
          }
          // What a run did is all its line says: it wraps rather than lose any of it.
          wrap={{
            text: done.length ? summary(done) : summary(pending, true),
            className: cn(active && "text-shimmer"),
          }}
        />
        {/* Folded, it shows the images its calls read: those of the steps it shows are under them. */}
        {!open && <ReadImages calls={calls.filter((t) => !visible.includes(t))} />}
        {(visible.length > 0 || coming.length > 0) && (
          <Thread id={id} className="mb-1">
            {visible.map((s, i) => (
              <div key={s.key} className="step-in" style={{ "--i": i } as CSSProperties}>
                {step(s, true)}
              </div>
            ))}
            {coming.map((b) => (
              <Coming key={`coming-${b.index}`} block={b} />
            ))}
          </Thread>
        )}
      </div>
    );
  },
  (a, b) =>
    a.running === b.running &&
    a.active === b.active &&
    a.up === b.up &&
    a.progress?.id === b.progress?.id &&
    a.progress?.seconds === b.progress?.seconds &&
    a.steps.length === b.steps.length &&
    a.steps.every((s, i) => s === b.steps[i]) &&
    (a.coming?.length ?? 0) === (b.coming?.length ?? 0) &&
    (a.coming ?? []).every((c, i) => c.index === b.coming?.[i]?.index && c.type === b.coming[i]?.type),
);

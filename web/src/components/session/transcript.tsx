// The transcript: people's messages, the agent's prose, its tool calls, and
// the session's milestones, with the turn in progress streaming at the end.

import {
  BrainCircuit,
  ChevronRight,
  CircleAlert,
  CircleStop,
  GitBranch,
  Hourglass,
  Info,
  Radio,
  Sparkles,
  TriangleAlert,
  Unplug,
} from "lucide-react";
import { memo, type ReactNode, useState } from "react";
import { Markdown } from "@/components/markdown";
import { Avatar, Spinner } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { clock, duration, until } from "@/lib/format";
import { repoChanges } from "@/lib/git";
import { claudeAppLink } from "@/lib/remote-control";
import { type Stall, stallText } from "@/lib/stall";
import type { Item, LiveTurn } from "@/lib/transcript";
import { MessageFiles } from "./attachments";
import { ToolRow } from "./tool";

function Milestone({
  icon,
  children,
  tone = "muted",
}: {
  icon: ReactNode;
  children: ReactNode;
  tone?: "muted" | "warn" | "error" | "info" | "ok";
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 py-1.5 text-[12.5px] [&>svg]:size-3.5 [&>svg]:shrink-0",
        tone === "muted" && "text-ink-4",
        tone === "warn" && "text-amber",
        tone === "error" && "text-danger",
        tone === "info" && "text-blue",
        tone === "ok" && "text-accent",
      )}
    >
      {icon}
      <span className="min-w-0">{children}</span>
    </div>
  );
}

const UserMessage = memo(function UserMessage({
  item,
  sessionId,
}: {
  item: Extract<Item, { kind: "user" }>;
  sessionId?: string;
}) {
  const author = item.author;
  const files = item.attachments ?? [];
  return (
    <div className={cn("group/msg animate-rise py-3", item.pending && "opacity-70")}>
      <div className="mb-1.5 flex items-center gap-2 text-[12.5px]">
        {author ? <Avatar user={author} size={20} /> : <span className="size-5 rounded-full bg-raised" />}
        <span className="font-medium text-ink">{author?.name ?? "fctl"}</span>
        <span className="text-ink-4 opacity-0 transition-opacity group-hover/msg:opacity-100">
          {item.pending ? "Sending…" : clock(item.ts)}
          {item.source === "cli" && " · from the CLI"}
          {item.source === "remote_control" && " · from the Claude app"}
        </span>
      </div>
      {item.text && (
        <div className="ml-7 rounded-xl rounded-tl-sm border border-line bg-raised/50 px-4 py-2.5 text-[14.5px] leading-[1.6] whitespace-pre-wrap wrap-anywhere text-ink">
          {item.text}
        </div>
      )}
      {files.length > 0 && (
        <div className={cn("ml-7", item.text && "mt-2")}>
          <MessageFiles sessionId={sessionId} files={files} previews={item.previews} pending={item.pending} />
        </div>
      )}
    </div>
  );
});

function Thinking({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="py-1">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 text-[12.5px] text-violet/80 transition-colors hover:text-violet"
      >
        <BrainCircuit className="size-3.5" />
        Thought it through
        <ChevronRight className={cn("size-3 transition-transform", open && "rotate-90")} />
      </button>
      {open && (
        <div className="mt-2 ml-5.5 animate-rise border-l border-violet/25 pl-4 font-display text-[15px] leading-relaxed whitespace-pre-wrap text-ink-3 italic">
          {text}
        </div>
      )}
    </div>
  );
}

function Setup({ item, stalled }: { item: Extract<Item, { kind: "setup" }>; stalled?: boolean }) {
  const [open, setOpen] = useState(false);
  const last = item.steps[item.steps.length - 1];
  const took = item.endTs - item.ts;
  return (
    <div className="py-2">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 text-[12.5px] text-ink-4 transition-colors hover:text-ink-2"
      >
        {item.done ? (
          <Sparkles className="size-3.5 text-accent/80" />
        ) : stalled ? (
          <Hourglass className="size-3.5 text-amber" />
        ) : (
          <Spinner className="size-3.5 text-amber" />
        )}
        {item.done ? (
          <>Sandbox ready in {duration(took)}</>
        ) : (
          <span className={cn(!stalled && "text-shimmer")}>{last?.reason ?? "Setting up the sandbox"}</span>
        )}
        {(item.logs.length > 0 || item.steps.length > 1) && (
          <ChevronRight className={cn("size-3 transition-transform", open && "rotate-90")} />
        )}
      </button>
      {open && (
        <div className="mt-2 ml-5.5 animate-rise space-y-1 border-l border-line pl-4 font-mono text-[11.75px] leading-relaxed text-ink-3">
          {item.steps.map((s, i) => (
            <div key={`s${i}`} className="text-ink-2">
              <span className="mr-2 text-ink-4">{clock(s.ts)}</span>
              {s.reason ?? s.status}
            </div>
          ))}
          {item.logs.map((l, i) => (
            <div
              key={`l${i}`}
              className={cn(
                "whitespace-pre-wrap break-words",
                l.level === "error" && "text-danger",
                l.level === "warn" && "text-amber",
              )}
            >
              {l.text}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Logs({ lines }: { lines: G.LogLine[] }) {
  const [open, setOpen] = useState(false);
  const warn = lines.some((l) => l.level === "warn" || l.level === "error");
  const first = lines[0]?.text ?? "";
  return (
    <div className="py-1">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className={cn(
          "flex max-w-full items-center gap-2 text-[12.5px] transition-colors hover:text-ink-2",
          warn ? "text-amber/80" : "text-ink-4",
        )}
      >
        <Info className="size-3.5 shrink-0" />
        <span className="truncate">
          {lines.length === 1 ? first : `${first} (+${lines.length - 1} more)`}
        </span>
        {lines.length > 1 && (
          <ChevronRight className={cn("size-3 shrink-0 transition-transform", open && "rotate-90")} />
        )}
      </button>
      {open && (
        <div className="mt-2 ml-5.5 animate-rise space-y-0.5 border-l border-line pl-4 font-mono text-[11.75px] text-ink-3">
          {lines.map((l, i) => (
            <div key={i} className="whitespace-pre-wrap break-words">
              {l.text}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/** One line per repository with changes: the primary one, then the others by name. */
function GitItem({ git }: { git: G.GitState }) {
  const repos = repoChanges(git).filter((r) => r.files.length > 0 || r.pr);
  return repos.map((r) => (
    <Milestone key={r.name ?? ""} icon={<GitBranch />}>
      {r.name && <span className="font-mono text-ink-3">{r.name}: </span>}
      {r.files.length} {r.files.length === 1 ? "file" : "files"} changed{" "}
      <span className="text-accent/80">+{r.additions}</span>{" "}
      <span className="text-danger/80">−{r.deletions}</span>
      {r.ahead > 0 && (
        <>
          {" "}
          · {r.ahead} {r.ahead === 1 ? "commit" : "commits"}
        </>
      )}
      {r.pushed && <> · pushed</>}
      {r.pr && (
        <>
          {" · "}
          <a
            href={r.pr.url}
            target="_blank"
            rel="noreferrer"
            className="text-ink-2 underline decoration-line-strong underline-offset-2 hover:text-ink"
          >
            PR #{r.pr.number}
          </a>
        </>
      )}
    </Milestone>
  ));
}

const ItemView = memo(function ItemView({
  item,
  live,
  sessionId,
  running,
  stalled,
}: {
  item: Item;
  live?: LiveTurn;
  sessionId?: string;
  /** For tool calls: whether the agent works on its turn, as far as the page knows. */
  running?: boolean;
  /** For the setup: whether its runner can't be heard. */
  stalled?: boolean;
}) {
  switch (item.kind) {
    case "user":
      return <UserMessage item={item} sessionId={sessionId} />;
    case "prose":
      return (
        <div className="animate-fade py-2">
          <Markdown text={item.text} />
        </div>
      );
    case "thinking":
      return <Thinking text={item.text} />;
    case "tool":
      return (
        <ToolRow
          t={item}
          running={!item.result && !!running}
          seconds={live?.tool?.id === item.id ? live.tool.seconds : undefined}
        />
      );
    case "turn":
      return (
        <div className="flex items-center gap-3 py-4 text-[11.5px] text-ink-4">
          <div className="rule-fade flex-1" />
          <span className={cn(!item.ok && "text-amber")}>
            {item.ok ? "Turn finished" : "Turn ended"} · {duration(item.durationMs)}
          </span>
          <div className="rule-fade flex-1" />
        </div>
      );
    case "setup":
      return <Setup item={item} stalled={stalled} />;
    case "status":
      return item.status === "failed" ? (
        <Milestone icon={<CircleAlert />} tone="error">
          Failed{item.reason ? ` — ${item.reason}` : ""}
        </Milestone>
      ) : item.status === "paused" ? (
        <Milestone icon={<Hourglass />} tone="warn">
          {item.reason ?? "Paused"}
          {item.resumeAt ? ` · resumes in ${until(item.resumeAt)}` : ""}
        </Milestone>
      ) : item.status === "queued" ? (
        <Milestone icon={<Hourglass />}>{item.reason ?? "Queued"}</Milestone>
      ) : (
        <Milestone icon={<CircleStop />}>{item.reason ?? "Stopped"}</Milestone>
      );
    case "logs":
      return <Logs lines={item.lines} />;
    case "notice":
      return (
        <Milestone
          icon={
            item.notice.level === "error" ? (
              <CircleAlert />
            ) : item.notice.level === "warn" ? (
              <TriangleAlert />
            ) : (
              <Info />
            )
          }
          tone={item.notice.level === "error" ? "error" : item.notice.level === "warn" ? "warn" : "info"}
        >
          {item.notice.message}
          {item.notice.until ? ` (until ${clock(item.notice.until)})` : ""}
        </Milestone>
      );
    case "git":
      return <GitItem git={item.git} />;
    case "remote":
      return item.rc.session_url ? (
        <Milestone icon={<Radio />} tone="ok">
          Followable in the Claude apps ·{" "}
          <a {...claudeAppLink(item.rc.session_url)} className="underline underline-offset-2">
            open
          </a>
        </Milestone>
      ) : null;
    case "interrupt":
      return (
        <Milestone icon={<CircleStop />} tone="warn">
          Interrupted by {item.by?.name ?? "someone"}
        </Milestone>
      );
    case "init":
      return (
        <Milestone icon={<Sparkles />}>
          Claude Code {item.version ? `${item.version} · ` : ""}
          {item.model}
        </Milestone>
      );
    case "limit":
      return (
        <Milestone icon={<Hourglass />} tone="warn">
          Usage limit reached{item.limit ? ` (${item.limit.replace(/_/g, " ")})` : ""}
          {item.resetsAt ? ` · resets ${clock(item.resetsAt)}` : ""}
        </Milestone>
      );
  }
});

function LiveText({ text, writing }: { text: string; writing: boolean }) {
  return (
    <div
      className={cn(
        "relative py-2",
        writing &&
          "after:ml-0.5 after:inline-block after:h-[1.05em] after:w-[2px] after:translate-y-[3px] after:animate-caret after:bg-accent after:content-['']",
      )}
    >
      <Markdown text={text} className="[&>*:last-child]:inline" />
    </div>
  );
}

/** The agent's work isn't coming through: says why, in place of work in progress. */
function StallNotice({ stall }: { stall: Stall }) {
  const { title, detail } = stallText(stall);
  return (
    <div role="status" className="flex animate-fade gap-2.5 py-2 text-[13px] leading-relaxed">
      <Unplug className="mt-[3px] size-3.5 shrink-0 text-amber" />
      <div className="min-w-0">
        <p className="text-amber">{title}</p>
        <p className="text-ink-3">{detail}</p>
      </div>
    </div>
  );
}

export function LiveTurnView({
  live,
  status,
  since,
  stall,
}: {
  live: LiveTurn;
  status?: G.SessionStatus;
  since?: number;
  stall?: Stall;
}) {
  const blocks = live.blocks;
  const working = status === "running" && !stall;
  return (
    <>
      {blocks.map((b) =>
        b.type === "text" ? (
          <LiveText key={`live-${b.index}`} text={b.text} writing={!stall} />
        ) : b.type === "thinking" ? (
          <div
            key={`live-${b.index}`}
            className="flex items-center gap-2 py-1.5 text-[12.5px] text-violet/80"
          >
            <BrainCircuit className="size-3.5" />
            <span className={cn(!stall && "text-shimmer")}>Thinking</span>
          </div>
        ) : (
          <div
            key={`live-${b.index}`}
            className="flex items-center gap-2.5 py-1 pl-1 text-[13.5px] text-ink-3"
          >
            <span className="flex size-6 items-center justify-center rounded-md border border-line bg-raised">
              {stall ? (
                <Hourglass className="size-3.5 text-ink-4" />
              ) : (
                <Spinner className="size-3.5 text-accent" />
              )}
            </span>
            <span className={cn(!stall && "text-shimmer")}>Preparing {b.name ?? "a tool"}</span>
          </div>
        ),
      )}
      {stall && <StallNotice stall={stall} />}
      {working && !blocks.some((b) => !b.done) && (
        <div className="flex items-center gap-2 py-2 text-[12.5px] text-ink-3">
          <span className="relative flex size-2">
            <span className="absolute inset-0 animate-ping rounded-full bg-accent/60" />
            <span className="relative size-2 rounded-full bg-accent" />
          </span>
          <span className="text-shimmer">Working{since ? ` · ${duration(Date.now() - since)}` : ""}</span>
        </div>
      )}
    </>
  );
}

export const Transcript = memo(function Transcript({
  sessionId,
  items,
  live,
  status,
  stall,
  onLoadOlder,
  hasMoreBefore,
  loadingOlder,
}: {
  sessionId: string;
  items: Item[];
  live: LiveTurn;
  status?: G.SessionStatus;
  /** Why the agent's work isn't coming through right now. */
  stall?: Stall;
  onLoadOlder: () => void;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
}) {
  // A tool call without a result runs only while the agent works on a turn
  // and its work comes through; after a stop or a crash it never will.
  const running = status === "running" && !stall;
  // Tool rows sit in a quiet column; everything else spans the reading width.
  return (
    <div className="flex flex-col">
      {hasMoreBefore && (
        <button
          type="button"
          onClick={onLoadOlder}
          className="mx-auto mb-4 flex items-center gap-2 rounded-full border border-line px-3 py-1 text-[12px] text-ink-3 hover:text-ink"
        >
          {loadingOlder && <Spinner className="size-3" />}
          Show earlier activity
        </button>
      )}
      {items.map((it) => (
        <div
          key={it.key}
          className={cn(
            it.kind !== "user" && it.kind !== "prose" && "cv-auto",
            it.kind === "tool" && "[contain-intrinsic-size:auto_34px]",
          )}
        >
          <ItemView
            item={it}
            live={it.kind === "tool" ? live : undefined}
            sessionId={sessionId}
            running={it.kind === "tool" ? running : undefined}
            stalled={it.kind === "setup" ? !!stall : undefined}
          />
        </div>
      ))}
      <LiveTurnView live={live} status={status} stall={stall} />
    </div>
  );
});

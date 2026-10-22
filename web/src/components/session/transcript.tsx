// The transcript: people's messages, the agent's prose, its tool calls, and
// the session's milestones, with the turn in progress streaming at the end.

import { useMutation } from "@tanstack/react-query";
import {
  Archive,
  BookMarked,
  BrainCircuit,
  ChevronRight,
  CircleAlert,
  CircleStop,
  GitBranch,
  Hourglass,
  ImageUp,
  Info,
  Radio,
  ScrollText,
  Sparkles,
  Trash2,
  TriangleAlert,
  Unplug,
} from "lucide-react";
import { Component, type MouseEvent, memo, type ReactNode, useMemo, useRef, useState } from "react";
import { ImageSizes, Markdown } from "@/components/markdown";
import { NoteLink } from "@/components/project/memory";
import { OutcomeBadge, RetrospectiveParts } from "@/components/retrospective";
import {
  Avatar,
  Button,
  CopyButton,
  Dialog,
  Gallery,
  plainClick,
  Spinner,
  toast,
  useLightbox,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { bytes, clock, day, duration, until } from "@/lib/format";
import { repoChanges } from "@/lib/git";
import { useKeepPlace } from "@/lib/keep-place";
import { isPublishedLink, linkToken, markdown } from "@/lib/published";
import { claudeAppLink, remoteControlUrl } from "@/lib/remote-control";
import { type Stall, stallText } from "@/lib/stall";
import type { Item, LiveTurn } from "@/lib/transcript";
import { MessageFiles } from "./attachments";
import { Thinking } from "./thinking";
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

/**
 * One item of the transcript, or a line in its place when it can't be shown: events come from the session's sandbox,
 * where anything can pose as the agent, and one this app can't read mustn't take the page down for everyone.
 */
class Unreadable extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    return this.state.failed ? (
      <Milestone icon={<CircleAlert />} tone="warn">
        This event couldn't be shown
      </Milestone>
    ) : (
      this.props.children
    );
  }
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

/**
 * A note the agent kept in the project's memory, or marked stale (a
 * duplicate too), as it was then, with a link to it in the project's Memory
 * tab.
 */
function MemoryNotice({
  memory,
  stale,
  projectSlug,
}: {
  memory: G.Memory;
  stale: boolean;
  projectSlug?: string;
}) {
  const duplicateOf = stale ? memory.stale?.duplicate_of : undefined;
  return (
    <div className="animate-rise py-1.5">
      <Milestone icon={stale ? <Archive /> : <BookMarked />} tone={stale ? "muted" : "ok"}>
        {duplicateOf
          ? "Marked a note a duplicate"
          : stale
            ? "Marked a note stale"
            : "Remembered for later agents"}
        {projectSlug && (
          <>
            {" · "}
            <NoteLink slug={projectSlug} id={memory.id} className="text-ink-3">
              Memory
            </NoteLink>
          </>
        )}
      </Milestone>
      <div
        className={cn(
          "mt-0.5 ml-5.5 border-l border-line-strong pl-3 text-[13.5px]",
          stale ? "text-ink-3" : "text-ink-2",
        )}
      >
        <Markdown text={memory.text} className="prose-note" />
        {stale && (memory.stale?.replaced_by || duplicateOf || memory.stale?.reason) && (
          <p className="mt-1 text-[12.5px] text-ink-4 [overflow-wrap:anywhere]">
            {memory.stale?.replaced_by ? (
              "A newer note takes its place."
            ) : duplicateOf ? (
              <>
                It repeats{" "}
                {projectSlug ? (
                  <NoteLink
                    slug={projectSlug}
                    id={duplicateOf}
                    arrow={false}
                    className="underline underline-offset-2"
                  >
                    another note
                  </NoteLink>
                ) : (
                  "another note"
                )}
                .
              </>
            ) : (
              memory.stale?.reason
            )}
          </p>
        )}
      </div>
    </div>
  );
}

/** A retrospective the agent recorded as it finished a piece of work. */
function RetrospectiveNotice({ r }: { r: G.Retrospective }) {
  return (
    <div className="my-2 animate-rise rounded-xl border border-line bg-raised/30 px-4 py-3.5">
      <div className="mb-3 flex items-center gap-2 text-[12.5px] text-ink-3">
        <ScrollText className="size-3.5 shrink-0" />
        <span>Retrospective</span>
        <OutcomeBadge outcome={r.outcome} />
      </div>
      <RetrospectiveParts r={r} wide />
    </div>
  );
}

/** Whether a published screenshot is this instance's to show (`isPublishedLink`); others show as the notice's text. */
const shown = (file: G.PublishedFile) => isPublishedLink(file.url, location.origin, import.meta.env.DEV);

/**
 * A screenshot the agent published for a pull request: the image while its
 * link works, which a click shows at full size over the page, and for
 * people who manage the session, deleting it sooner.
 */
function PublishedShot({ file, canManage }: { file: G.PublishedFile; canManage?: boolean }) {
  const [state, setState] = useState<"shown" | "offline" | "deleted">(() =>
    file.expires_at > Date.now() ? "shown" : "offline",
  );
  const [confirm, setConfirm] = useState(false);
  const { ref, open, lightbox } = useLightbox<HTMLAnchorElement>(
    state === "shown" ? { file, src: file.url, download: true } : undefined,
  );
  // With a modifier key, the browser opens the link in a new tab instead.
  const view = (e: MouseEvent) => {
    if (!plainClick(e)) return;
    e.preventDefault();
    open();
  };
  const token = linkToken(file.url);
  const del = useMutation({
    mutationFn: () => api.unpublish(token ?? ""),
    onSuccess: () => {
      setState("deleted");
      setConfirm(false);
      toast(`Deleted ${file.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const facts = [file.width && file.height ? `${file.width}×${file.height}` : "", bytes(file.size)]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="py-1.5">
      <Milestone icon={<ImageUp />} tone="info">
        Published{" "}
        {state === "shown" ? (
          <a
            href={file.url}
            target="_blank"
            rel="noreferrer"
            onClick={view}
            className="underline underline-offset-2"
          >
            {file.name}
          </a>
        ) : (
          file.name
        )}
        {state === "shown"
          ? ` · anyone with the link can open it until ${day(file.expires_at)}`
          : state === "deleted"
            ? " · deleted"
            : " · its link no longer works"}
      </Milestone>
      {state === "shown" && (
        <div className="mt-1 ml-5.5 flex flex-wrap items-end gap-3">
          <a
            ref={ref}
            href={file.url}
            target="_blank"
            rel="noreferrer"
            onClick={view}
            aria-label={`View ${file.name}`}
            className="cursor-zoom-in overflow-hidden rounded-lg border border-line bg-raised/50 transition-[border-color] duration-150 hover:border-line-strong"
          >
            <img
              src={file.url}
              alt={file.name}
              width={file.width}
              height={file.height}
              loading="lazy"
              decoding="async"
              onError={() => setState("offline")}
              className="h-36 w-auto max-w-[min(100%,20rem)] object-cover"
            />
          </a>
          <div className="flex flex-col items-start gap-0.5">
            <span className="px-2 font-mono text-[10.5px] text-ink-4">{facts}</span>
            <CopyButton text={markdown(file)} label="Copy Markdown" />
            {canManage && token && (
              <button
                type="button"
                onClick={() => setConfirm(true)}
                className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-danger-soft hover:text-danger"
              >
                <Trash2 className="size-3.5" />
                Delete
              </button>
            )}
          </div>
        </div>
      )}
      {lightbox}
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title={`Delete ${file.name}?`}
        description="Its link stops working for everyone, in pull requests too. This can't be undone."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setConfirm(false)}>
            Cancel
          </Button>
          <Button variant="danger" loading={del.isPending} onClick={() => del.mutate()}>
            Delete
          </Button>
        </div>
      </Dialog>
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

/** One line per repository whose branch changed: the primary one, then the others by name. */
function GitItem({ git, repos }: { git: G.GitState; repos: (string | undefined)[] }) {
  const changed = repoChanges(git).filter((r) => repos.includes(r.name));
  return changed.map((r) => (
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
  up,
  stalled,
  canManage,
  claudeUrl,
  projectSlug,
}: {
  item: Item;
  live?: LiveTurn;
  sessionId?: string;
  /** For tool calls: whether the agent works on its turn, as far as the page knows. */
  running?: boolean;
  /** For subagents: whether Claude Code runs, as far as the page knows. */
  up?: boolean;
  /** For the setup: whether its runner can't be heard. */
  stalled?: boolean;
  /** The viewer manages the session (deletes its published screenshots). */
  canManage?: boolean;
  /** For Remote Control: its link, when it is the viewer's to open. */
  claudeUrl?: string;
  /** The session's project, which the notes its agent keeps link to. */
  projectSlug?: string;
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
          up={up}
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
      if (item.notice.published && shown(item.notice.published))
        return <PublishedShot file={item.notice.published} canManage={canManage} />;
      if (item.notice.retrospective) return <RetrospectiveNotice r={item.notice.retrospective} />;
      if (item.notice.memory)
        return (
          <MemoryNotice
            memory={item.notice.memory}
            stale={item.notice.code === "memory_stale"}
            projectSlug={projectSlug}
          />
        );
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
      return <GitItem git={item.git} repos={item.repos} />;
    case "remote":
      return claudeUrl ? (
        <Milestone icon={<Radio />} tone="ok">
          Followable in the Claude apps ·{" "}
          <a {...claudeAppLink(claudeUrl)} className="underline underline-offset-2">
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
  canManage,
  startedBy,
  me,
  projectSlug,
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
  canManage?: boolean;
  /** Who started the session, and who is signed in: a Remote Control link is only for its person. */
  startedBy: G.UserRef;
  me?: G.UserId;
  /** The session's project, which the notes its agent keeps link to. */
  projectSlug?: string;
  onLoadOlder: () => void;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
}) {
  // A tool call without a result runs only in the turn the agent works on,
  // while its work comes through. One from a turn that ended (with a result,
  // a stop or a failure) never gets one.
  const running = status === "running" && !stall;
  const turnStart = items.findLastIndex((it) => it.kind === "turn" || it.kind === "status");
  // Subagents in the background work on after their turn, while the
  // session waits for them, until they end or the session stops.
  const up = (status === "running" || status === "idle") && !stall;
  const list = useRef<HTMLDivElement>(null);
  // Earlier activity goes in above the reader, who stays where they are.
  const keepPlace = useKeepPlace(list, loadingOlder);
  // The screenshots the agent published, which its answers show: they take
  // their room before they load. A new map only when it publishes another.
  const shots = items.flatMap((it) =>
    it.kind === "notice" && it.notice.published && shown(it.notice.published) ? [it.notice.published] : [],
  );
  const shotUrls = shots.map((f) => f.url).join(" ");
  // biome-ignore lint/correctness/useExhaustiveDependencies: shotUrls stands for shots
  const sizes = useMemo(() => new Map(shots.map((f) => [f.url, f])), [shotUrls]);
  // Tool rows sit in a quiet column; everything else spans the reading width.
  // The pictures people attach, the agent publishes and its answers show
  // open in one lightbox, which steps through them all.
  return (
    <ImageSizes value={sizes}>
      <Gallery>
        <div ref={list} className="flex flex-col">
          {hasMoreBefore && (
            <button
              type="button"
              onClick={() => {
                keepPlace();
                onLoadOlder();
              }}
              className="mx-auto mb-4 flex items-center gap-2 rounded-full border border-line px-3 py-1 text-[12px] text-ink-3 hover:text-ink"
            >
              {loadingOlder && <Spinner className="size-3" />}
              Show earlier activity
            </button>
          )}
          {items.map((it, i) => (
            <div key={it.key} data-item>
              <Unreadable>
                <ItemView
                  item={it}
                  live={it.kind === "tool" ? live : undefined}
                  sessionId={sessionId}
                  running={it.kind === "tool" ? running && i > turnStart : undefined}
                  up={it.kind === "tool" && it.agent ? up : undefined}
                  stalled={it.kind === "setup" ? !!stall : undefined}
                  canManage={canManage}
                  claudeUrl={it.kind === "remote" ? remoteControlUrl(it.rc, startedBy, me) : undefined}
                  projectSlug={projectSlug}
                />
              </Unreadable>
            </div>
          ))}
          <LiveTurnView live={live} status={status} stall={stall} />
        </div>
      </Gallery>
    </ImageSizes>
  );
});

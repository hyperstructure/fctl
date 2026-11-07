// The new session page's transcript (routes/session-next.tsx): the session in turns, each a
// chapter of it. What someone asked opens a turn, in a card. What the agent did about it hangs
// from the card on a thread down the rail its steps' icons sit in, with what it said beside the
// thread; and where the thread ends, the turn says how it went: how long it took, how many steps,
// and the files it edited, which open in the changes.
//
// The parts of the transcript stay the classic transcript's (`PartView`), one element each in one
// list, with the keys they have there: finding, earlier activity coming in above the reader and
// what the reader opened all work as they do on the classic page.

import {
  ArrowUpRight,
  Check,
  CircleCheck,
  CircleSlash,
  Copy,
  Ellipsis,
  FilePenLine,
  Link2,
  OctagonX,
  Sparkle,
  TriangleAlert,
} from "lucide-react";
import {
  type CSSProperties,
  memo,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { ImageSizes } from "@/components/markdown";
import { Avatar, Gallery, Logo, Menu, MenuItem, toast } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { duration, stamp, when } from "@/lib/format";
import { useKeepPlace } from "@/lib/keep-place";
import type { Part } from "@/lib/runs";
import { splitRunNote } from "@/lib/schedules";
import type { DiskShort, Stall } from "@/lib/stall";
import { type BackgroundTask, findingsBefore, type Item, type LiveTurn } from "@/lib/transcript";
import {
  type EditedFile,
  factsOf,
  lastTs,
  laterBy,
  type Turn,
  type TurnEnd,
  type TurnFacts,
  turnMarkdown,
  turnSeq,
  turnsOf,
  turnTitle,
  turnTook,
} from "@/lib/turns";
import { MessageFiles } from "./attachments";
import type { DiffGroup, DiffTarget } from "./diff-files";
import { Chevron, Milestone, RailIcon, Unreadable } from "./lines";
import { CallTasks, SessionImages, WorkDir } from "./tool";
import {
  EarlierActivity,
  LiveTurnView,
  LongText,
  PartView,
  type TranscriptParts,
  transcriptParts,
  UserMessage,
  useShotSizes,
} from "./transcript";

type Ask = Extract<Item, { kind: "user" }>;

/** How many stretches of a turn's thread its light runs down as it ends, and for how long it may. */
const SWEPT = 10;
const SWEEP_MS = 2500;

/** What the parts of the transcript need to know of the session and the reader. */
interface Ctx {
  t: TranscriptParts;
  sessionId: string;
  stall?: Stall;
  canManage?: boolean;
  startedBy: G.UserRef;
  me?: G.UserId;
  projectSlug?: string;
}

/**
 * Items that stand apart from the thread's rail, as the agent's words do: they start where the
 * thread's lines' words do, beside it.
 */
function besideThread(it: Item): boolean {
  if (it.kind === "prose") return true;
  if (it.kind !== "notice") return false;
  const n = it.notice;
  return !!(n.retrospective || n.review || n.summarized);
}

/**
 * What the conversation alone keeps (`quiet`): people's messages, what the agent said and asked,
 * its retrospectives, the screenshots it published and reviews it posted, and failures.
 */
function inConversation(p: Part<Item>): boolean {
  if (p.kind === "run") return false;
  const it = p.item;
  switch (it.kind) {
    case "user":
    case "prose":
      return true;
    case "status":
      return it.status === "failed";
    case "notice": {
      const n = it.notice;
      return !!(
        n.question ||
        n.retrospective ||
        n.published ||
        n.review ||
        n.summarized ||
        n.level === "error"
      );
    }
    default:
      return false;
  }
}

/** Which file of the session's changes an edit was to, if it is one of them. */
export function changeOf(file: EditedFile, groups: readonly DiffGroup[]): DiffTarget | undefined {
  // Other repositories are cloned under ~/repos/<owner>/<name>, as fctl tells agents to.
  const other = /^~\/repos\/([^/]+\/[^/]+)\/(.+)$/.exec(file.path);
  for (const g of groups) {
    const path = g.repo ? (other?.[1] === g.repo ? other[2] : undefined) : other ? undefined : file.path;
    if (path && g.files.some((f) => f.path === path)) return { repo: g.repo, path };
  }
  return undefined;
}

export const TurnTranscript = memo(function TurnTranscript({
  sessionId,
  items,
  live,
  status,
  stall,
  disk,
  canManage,
  startedBy,
  me,
  projectSlug,
  onLoadOlder,
  hasMoreBefore,
  loadingOlder,
  olderFailed = false,
  cover,
  quiet,
  newFrom,
  diffGroups,
  onDiff,
  tasks,
}: {
  sessionId: string;
  items: Item[];
  live: LiveTurn;
  status?: G.SessionStatus;
  /** Why the agent's work isn't coming through right now. */
  stall?: Stall;
  /** How short of disk the session's runner is. */
  disk?: DiskShort;
  canManage?: boolean;
  /** Who started the session, and who is signed in: a Remote Control link is only for its person. */
  startedBy: G.UserRef;
  me?: G.UserId;
  /** The session's project, which the notes its agent keeps link to. */
  projectSlug?: string;
  onLoadOlder: () => void;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
  olderFailed?: boolean;
  /** What opens the session, once its start is loaded: its title, and who started it when. */
  cover?: ReactNode;
  /** Only the conversation: the agent's work and the milestones are left out. */
  quiet?: boolean;
  /** The first part that came while the reader was away, which a line marks. */
  newFrom?: string;
  /** The session's changes, which the files a turn edited open in. */
  diffGroups: readonly DiffGroup[];
  onDiff: (at: DiffTarget) => void;
  /** What Claude Code ran on its own: the calls that left work in the background say how it goes. */
  tasks?: readonly BackgroundTask[];
}) {
  // What the agent does at this moment shows in the line above the message box (now-line.tsx):
  // what streams in, a thought or a call it writes, shows there, and here once it is done.
  const t = { ...transcriptParts(items, live, status, stall), coming: [] };
  // What came after the page first showed the session is new to the reader: its thread draws.
  const shownUpTo = useShownUpTo(sessionId, items);
  // When the page saw each turn end that came while it showed: its light runs for a moment.
  const endsSeen = useRef(new Map<string, number>());
  const justEnded = (end: TurnEnd) => {
    if (end.seq <= shownUpTo) return false;
    const seen = endsSeen.current.get(end.key) ?? Date.now();
    endsSeen.current.set(end.key, seen);
    return Date.now() - seen < SWEEP_MS;
  };
  const turns = turnsOf(t.ps);
  const list = useRef<HTMLDivElement>(null);
  // Earlier activity goes in above the reader, who stays where they are.
  const keepPlace = useKeepPlace(list, loadingOlder);
  useLayoutEffect(() => {
    if (loadingOlder) keepPlace();
  }, [loadingOlder, keepPlace]);
  const sizes = useShotSizes(items);
  const byCall = useMemo(
    () => new Map((tasks ?? []).flatMap((task) => (task.call ? [[task.call, task] as const] : []))),
    [tasks],
  );
  const ctx: Ctx = { t, sessionId, stall, canManage, startedBy, me, projectSlug };
  const last = turns[turns.length - 1];
  // The agent works on the last turn: the work that streams in goes on its thread.
  const open = !!last && !last.end;

  const rows: ReactNode[] = [];
  // Where what came while the reader was away starts: the line goes before the first row from
  // there on that shows (the conversation alone leaves the agent's work out).
  let lineDue = false;
  // Once: a stop's key is the turn's end's and a part after it.
  let lined = false;
  const reached = (key: string) => {
    if (key === newFrom && !lined) lineDue = true;
  };
  const lineBefore = (thread: boolean) => {
    if (!lineDue) return;
    lineDue = false;
    lined = true;
    rows.push(<NewLine key="new-line" thread={thread} />);
  };
  const arrived = (seq: number | undefined) => seq !== undefined && seq > shownUpTo;
  const part = (p: Part<Item>, thread: boolean, sweep?: CSSProperties) => {
    reached(p.key);
    if (quiet && !inConversation(p)) return;
    lineBefore(thread);
    const beside = p.kind === "item" && besideThread(p.item);
    const seq = p.kind === "run" ? p.steps[0]?.seq : p.item.seq;
    rows.push(
      <div
        key={p.key}
        data-item
        data-part={p.key}
        className={cn(thread && "thread", thread && arrived(seq) && "arrive", sweep && "sweep")}
        style={sweep}
      >
        <Unreadable>
          {beside ? (
            <div className="pl-[34px]">
              <PartView p={p} {...ctx} />
            </div>
          ) : (
            <PartView p={p} {...ctx} />
          )}
        </Unreadable>
      </div>,
    );
  };
  turns.forEach((turn, i) => {
    const first = i === 0;
    // When the turn came long after the one before ended: how much later.
    const before = turns[i - 1];
    const since = before && lastTs(before);
    const start = turn.ask?.ts ?? firstPartTs(turn);
    const later = since && start ? laterBy(start - since) : undefined;
    const gap = later && (
      <p className="turn-gap pb-6 text-[11.5px] text-ink-4" title={stamp(start ?? 0)}>
        {later}
      </p>
    );
    // Where it starts, for the bar that says which turn the reader is in.
    if (turn.ask && turn.askPart) {
      // What came while the reader was away starts with this turn: the line says so above its card.
      reached(turn.askPart.key);
      const isNew = lineDue;
      lineDue = false;
      if (isNew) lined = true;
      rows.push(
        <div
          key={turn.askPart.key}
          data-item
          data-part={turn.askPart.key}
          data-turn={turn.key}
          className={cn(first ? (cover ? "pt-2" : "pt-1") : gap || isNew ? "pt-10" : "pt-12")}
        >
          {gap}
          {isNew && <NewLine thread={false} className="-mt-2 mb-3" />}
          <Unreadable>
            <AskCard item={turn.ask} sessionId={sessionId} />
          </Unreadable>
        </div>,
      );
    } else if (first && hasMoreBefore) {
      // Its start isn't loaded: earlier activity above it has it.
      rows.push(<div key={`start-${turn.key}`} data-turn={turn.key} />);
    } else {
      lineBefore(false);
      rows.push(
        <div key={`start-${turn.key}`} data-turn={turn.key} className={cn(!first && "pt-10")}>
          {gap}
          <div className="thread thread-start">
            <Milestone icon={<Sparkle />}>The agent went on without a message</Milestone>
          </div>
        </div>,
      );
    }
    // A turn that ends while the reader watches: a light runs down the last of its thread.
    const shownBody = quiet ? turn.body.filter(inConversation) : turn.body;
    const sweeping = !!turn.end && justEnded(turn.end);
    const n = Math.min(SWEPT, shownBody.length + 1);
    const sweepOf = (k: number): CSSProperties | undefined =>
      sweeping && k < n ? ({ "--k": k, "--n": n } as CSSProperties) : undefined;
    const fromEnd = new Map(shownBody.map((p, j) => [p.key, shownBody.length - j]));
    for (const p of turn.body) part(p, true, sweepOf(fromEnd.get(p.key) ?? SWEPT));
    const isLast = i === turns.length - 1;
    if (isLast && open) {
      lineBefore(true);
      rows.push(
        // The thread trails off under the newest step, where the next comes.
        <div key="live" className="thread thread-live min-h-8">
          <LiveTurnView
            live={live}
            status={status}
            stall={stall}
            disk={disk}
            inRun={t.folded}
            textClassName="pl-[34px]"
            nowLine
          />
        </div>,
      );
    }
    if (turn.end) {
      // The turn's result is no part of it, but what came while away may start there.
      reached(turn.end.key);
      lineBefore(true);
      rows.push(
        <div
          key={`end-${turn.end.key}`}
          className={cn("thread thread-end", arrived(turn.end.seq) && "arrive", sweeping && "sweep")}
          style={sweepOf(0)}
        >
          <Outcome
            turn={turn}
            facts={factsOf(turn, t.cwd)}
            title={turnTitle(turn, i === 0, hasMoreBefore)}
            sessionId={sessionId}
            groups={diffGroups}
            onDiff={onDiff}
          />
        </div>,
      );
    }
    for (const p of turn.after) part(p, false);
  });
  if (!open) {
    // The agent went on after the last turn ended, without a message yet: what streams in.
    rows.push(
      <div key="live">
        <LiveTurnView live={live} status={status} stall={stall} disk={disk} inRun={t.folded} nowLine />
      </div>,
    );
  }

  return (
    <ImageSizes value={sizes}>
      <Gallery>
        <div ref={list} className="turns flex flex-col">
          {hasMoreBefore ? (
            <EarlierActivity
              onLoad={() => {
                keepPlace();
                onLoadOlder();
              }}
              loading={loadingOlder}
              failed={olderFailed}
            />
          ) : (
            cover
          )}
          <WorkDir value={t.cwd}>
            <SessionImages value={sessionId}>
              <CallTasks.Provider value={byCall}>{rows}</CallTasks.Provider>
            </SessionImages>
          </WorkDir>
        </div>
      </Gallery>
    </ImageSizes>
  );
});

/** When a turn's first part came. */
function firstPartTs(turn: Turn): number | undefined {
  const p = turn.body[0] ?? turn.after[0];
  return p ? (p.kind === "run" ? p.steps[0]?.ts : p.item.ts) : undefined;
}

/**
 * The newest event the page had when it first showed the session: what comes after it is new to
 * the reader, and draws as it comes. Earlier activity that loads later is older than it.
 */
function useShownUpTo(session: string, items: Item[]): number {
  const [at, setAt] = useState<{ session: string; seq: number }>();
  const newest = items.reduce((n, it) => (it.seq !== Number.MAX_SAFE_INTEGER && it.seq > n ? it.seq : n), 0);
  if (items.length > 0 && at?.session !== session) setAt({ session, seq: newest });
  return at?.session === session ? at.seq : Number.MAX_SAFE_INTEGER;
}

/** Where what came while the reader was away starts. */
function NewLine({ thread, className }: { thread: boolean; className?: string }) {
  return (
    <div className={cn("new-line flex items-center gap-3 py-2", thread && "thread", className)}>
      <span className="h-px flex-1 bg-gradient-to-r from-transparent via-pink/60 to-pink/60" />
      <span className="text-[11.5px] font-medium text-pink">New</span>
      <span className="h-px w-6 bg-pink/60" />
    </div>
  );
}

/**
 * What someone asked, as the card that opens a turn: who, when and from where, and what they
 * wrote, with what they attached. Messages from fctl about a pull request, a review or a ticket
 * keep the classic transcript's look, which shows what happened there.
 */
export function AskCard({ item, sessionId }: { item: Ask; sessionId: string }) {
  if (item.followUp || item.ticket || item.review) return <UserMessage item={item} sessionId={sessionId} />;
  const author = item.author;
  const files = item.attachments ?? [];
  const { task, note } = splitRunNote(item.text);
  return (
    <article
      className={cn(
        "ask-card group/ask animate-rise rounded-[18px] rounded-bl-[7px] border border-line bg-surface px-4 pt-3 pb-3.5 md:px-5 md:pt-3.5 md:pb-4",
        item.pending && "opacity-70",
      )}
    >
      <header className="flex min-w-0 items-center gap-2 text-[12.5px] leading-5">
        <span className="flex size-5 shrink-0 items-center justify-center">
          {author ? <Avatar user={author} size={20} /> : <Logo withWord={false} className="[&_svg]:size-5" />}
        </span>
        <span className="truncate font-medium text-ink">{author?.name ?? "fctl"}</span>
        <span className="min-w-0 truncate text-ink-4" title={item.pending ? undefined : stamp(item.ts)}>
          {item.pending ? "Sending…" : when(item.ts)}
          {item.source === "cli" && " · from the CLI"}
          {item.source === "remote_control" && " · from the Claude app"}
        </span>
        {item.text && !item.pending && <CopyAsk text={item.text} />}
      </header>
      {item.text && (
        <div className="mt-2 text-[15px] leading-[1.6] whitespace-pre-wrap wrap-anywhere text-ink md:text-[15.5px]">
          {findingsBefore(item) ? (
            <LongText text={item.text} findKey={item.key} />
          ) : (
            <div data-find={`msg:${item.key}`}>
              {task}
              {note && (
                <p className="mt-2.5 border-t border-line pt-2.5 text-[12.5px] leading-relaxed text-ink-3">
                  {note}
                </p>
              )}
            </div>
          )}
        </div>
      )}
      {files.length > 0 && (
        <div className={cn(item.text && "mt-3")}>
          <MessageFiles sessionId={sessionId} files={files} previews={item.previews} pending={item.pending} />
        </div>
      )}
    </article>
  );
}

/**
 * Copies what someone asked, to ask it again or elsewhere: at the card's corner, where a pointer
 * rests on the card. Not on touch screens, where nothing rests on it, and selecting the text does.
 */
function CopyAsk({ text }: { text: string }) {
  const [done, setDone] = useState(false);
  const t = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(t.current), []);
  return (
    <button
      type="button"
      aria-label={done ? "Copied" : "Copy the message"}
      title="Copy the message"
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setDone(true);
        clearTimeout(t.current);
        t.current = setTimeout(() => setDone(false), 1600);
      }}
      className="-my-1 -mr-2 ml-auto flex size-7 shrink-0 items-center justify-center rounded-md text-ink-4 opacity-0 transition-[opacity,color,background-color] duration-150 group-hover/ask:opacity-100 hover:bg-hover hover:text-ink focus-visible:opacity-100 [@media(hover:none)]:hidden"
    >
      {done ? <Check className="size-3.5 text-accent" /> : <Copy className="size-3.5" />}
    </button>
  );
}

/** How a turn ended, as its end says it, after how long. */
function endText(end: TurnEnd, took: number): { icon: ReactNode; text: string; tone: string } {
  const d = duration(took);
  switch (end.kind) {
    case "result":
      return end.ok
        ? { icon: <CircleCheck />, text: `Finished in ${d}`, tone: "text-accent" }
        : { icon: <TriangleAlert />, text: `Ended after ${d}`, tone: "text-amber" };
    case "failed":
      return { icon: <OctagonX />, text: `Worked for ${d}`, tone: "text-danger" };
    case "cut":
      return { icon: <TriangleAlert />, text: `Cut off after ${d}`, tone: "text-amber" };
    default:
      return { icon: <CircleSlash />, text: `Worked for ${d}`, tone: "text-ink-4" };
  }
}

/** Lines added and taken out: "+12 −3", in color when there are any. */
function LinesChanged({ added, removed }: { added: number; removed: number }) {
  return (
    <span className="font-mono text-[11px] tabular-nums">
      <span className={cn(added > 0 ? "text-added/80" : "text-ink-4")}>+{added}</span>{" "}
      <span className={cn(removed > 0 ? "text-danger/80" : "text-ink-4")}>−{removed}</span>
    </span>
  );
}

/**
 * Where a turn's thread ends: how it ended and after how long, its steps, and the files its edits
 * changed, which open into a list of them, each a click from its diff.
 */
function Outcome({
  turn,
  facts,
  title,
  sessionId,
  groups,
  onDiff,
}: {
  turn: Turn;
  facts: TurnFacts;
  /** What the turn is about, in a line. */
  title: string;
  sessionId: string;
  groups: readonly DiffGroup[];
  onDiff: (at: DiffTarget) => void;
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  if (!turn.end) return null;
  const took = turnTook(turn, facts);
  const { icon, text, tone } = endText(turn.end, took);
  const files = facts.files;
  return (
    <div className="py-1.5">
      <div className="flex items-start gap-2 text-[12.5px] leading-5 text-ink-3">
        <RailIcon className={tone}>{icon}</RailIcon>
        {/* Its facts wrap beside the icon, apart without dots, which would start a line. */}
        <div className="flex min-h-6 min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-0.5">
          <span className="font-medium text-ink-2">{text}</span>
          {facts.calls > 0 && (
            <span>
              {facts.calls} {facts.calls === 1 ? "step" : "steps"}
            </span>
          )}
          {facts.failed > 0 && <span className="text-danger/80">{facts.failed} failed</span>}
          {files.length > 0 && (
            <button
              type="button"
              aria-expanded={open}
              aria-controls={open ? id : undefined}
              onClick={() => setOpen(!open)}
              className="group/line -mx-1.5 inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 transition-colors hover:bg-hover hover:text-ink-2"
            >
              Edited {files.length} {files.length === 1 ? "file" : "files"}
              <LinesChanged {...facts.lines} />
              <Chevron open={open} className="-ml-0.5 size-3" />
            </button>
          )}
        </div>
        <TurnMenu turn={turn} facts={facts} title={title} took={took} sessionId={sessionId} />
      </div>
      {open && (
        <ul id={id} className="mt-1 ml-[34px] flex animate-rise flex-col py-0.5">
          {files.map((f, i) => (
            <EditedRow key={f.full} file={f} at={changeOf(f, groups)} onDiff={onDiff} i={i} />
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * What a turn can be taken away as, from its end: in Markdown, to paste where people talk about
 * it, or as a link that opens the session at it.
 */
function TurnMenu({
  turn,
  facts,
  title,
  took,
  sessionId,
}: {
  turn: Turn;
  facts: TurnFacts;
  title: string;
  took: number;
  sessionId: string;
}) {
  // A link to a turn names the message it starts with: a turn whose start isn't loaded has none
  // yet, and its first part loaded so far would be inside it once earlier activity loads.
  const seq = turn.ask ? turnSeq(turn) : undefined;
  return (
    <Menu
      // Up, over the turn: each part of a thread stacks on its own, and the next would cover it.
      side="top"
      className="-my-1 ml-auto shrink-0"
      trigger={(toggle, open) => (
        <button
          type="button"
          aria-label="This turn"
          aria-haspopup="menu"
          aria-expanded={open}
          onClick={toggle}
          className={cn(
            "flex size-7 items-center justify-center rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink",
            open && "bg-hover text-ink",
          )}
        >
          <Ellipsis className="size-4" />
        </button>
      )}
    >
      {(close) => (
        <>
          <MenuItem
            icon={<Copy />}
            onClick={() => {
              close();
              void copied(turnMarkdown(turn, facts, { title, when, took: duration(took) }), "The turn");
            }}
          >
            Copy as Markdown
          </MenuItem>
          {seq !== undefined && (
            <MenuItem
              icon={<Link2 />}
              onClick={() => {
                close();
                void copied(`${location.origin}/s/${sessionId}?turn=${seq}`, "The link to the turn");
              }}
            >
              Copy link to this turn
            </MenuItem>
          )}
        </>
      )}
    </Menu>
  );
}

/**
 * Copies `text`, and says whether it did: browsers refuse the clipboard on plain http, and without
 * the page's focus.
 */
async function copied(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text);
    toast(`${what} is copied`);
  } catch {
    toast(`${what} didn't copy`, "error");
  }
}

/** A file a turn edited: its path, its lines, and its diff a click away while it is among the changes. */
function EditedRow({
  file,
  at,
  onDiff,
  i,
}: {
  file: EditedFile;
  at?: DiffTarget;
  onDiff: (at: DiffTarget) => void;
  i: number;
}) {
  const cut = file.path.lastIndexOf("/") + 1;
  const body = (
    <>
      <FilePenLine aria-hidden="true" className="size-3.5 shrink-0 text-ink-4" />
      <span className="flex min-w-0 flex-1 font-mono text-[12px]">
        {cut > 0 && (
          <span className="min-w-0 shrink-[1000] truncate text-ink-4">{file.path.slice(0, cut)}</span>
        )}
        <span className="min-w-0 truncate text-ink-2">{file.path.slice(cut)}</span>
      </span>
      <LinesChanged {...file.lines} />
    </>
  );
  return (
    <li className="step-in" style={{ "--i": i } as CSSProperties}>
      {at ? (
        <button
          type="button"
          onClick={() => onDiff(at)}
          title="Open its changes"
          className="group/file -mx-2 flex w-[calc(100%+1rem)] items-center gap-2.5 rounded-md px-2 py-1 text-left transition-colors hover:bg-hover"
        >
          {body}
          <ArrowUpRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-opacity group-hover/file:opacity-100 max-md:hidden" />
        </button>
      ) : (
        <div className="flex items-center gap-2.5 py-1" title="Not among the session's changes now">
          {body}
        </div>
      )}
    </li>
  );
}

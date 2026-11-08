// Finding one's way through the new session page's turns (routes/session-next.tsx): which turn
// the reader is in, the bar under the header that says so once the turn's start has scrolled away,
// the list of every turn it opens, and the keys that go from turn to turn.

import {
  ChevronsUp,
  CircleCheck,
  CircleSlash,
  ListTree,
  OctagonX,
  Sparkle,
  TriangleAlert,
} from "lucide-react";
import { type ReactNode, type RefObject, useCallback, useEffect, useRef, useState } from "react";
import { Avatar, Logo, Menu } from "@/components/ui";
import { cn } from "@/lib/cn";
import { findables } from "@/lib/find";
import { agoShort, duration, stamp } from "@/lib/format";
import type { Item } from "@/lib/transcript";
import { type Turn, type TurnFacts, turnTitle, turnTook } from "@/lib/turns";
import type { Revealer } from "./find";

/** Where the reader is among the turns: the one they are in, and whether its start is out of sight above. */
export interface TurnPlace {
  /** Its index among the turns on the page, or -1 above the first. */
  at: number;
  /** Its start has scrolled up under the header. */
  pinned: boolean;
}

/** The turns' starts on the page (`data-turn`), in order. */
const marksIn = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>("[data-turn]")];

/** Where the page's header ends, in the window: what is above it is out of sight. */
const viewTop = (header: HTMLElement | null) => header?.getBoundingClientRect().bottom ?? 0;

/**
 * Which turn the reader is in, from where the turns of `root` start (`data-turn`) and where the
 * header ends: the last one that starts above the top of what they see. Measured as they scroll,
 * and as the page changes.
 */
export function useTurnPlace(
  root: RefObject<HTMLElement | null>,
  header: RefObject<HTMLElement | null>,
): TurnPlace {
  const [place, setPlace] = useState<TurnPlace>({ at: -1, pinned: false });
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    let raf = 0;
    const measure = () => {
      raf = 0;
      const top = viewTop(header.current);
      const marks = marksIn(el);
      let at = -1;
      for (const [i, m] of marks.entries()) {
        if (m.getBoundingClientRect().top > top + 24) break;
        at = i;
      }
      const start = marks[at];
      // The ask's card, or the line that starts a turn, is all above the header.
      const pinned = !!start && start.getBoundingClientRect().bottom < top + 4;
      setPlace((p) => (p.at === at && p.pinned === pinned ? p : { at, pinned }));
    };
    const on = () => {
      if (!raf) raf = requestAnimationFrame(measure);
    };
    window.addEventListener("scroll", on, { passive: true });
    window.addEventListener("resize", on, { passive: true });
    const grown = new ResizeObserver(on);
    grown.observe(el);
    measure();
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("scroll", on);
      window.removeEventListener("resize", on);
      grown.disconnect();
    };
  }, [root, header]);
  return place;
}

/**
 * Goes to the turn `i` of `root`: its start comes in under the header. `letGo` lets go of the
 * page's bottom first, which it keeps in view while the agent works.
 */
export function useGoToTurn(
  root: RefObject<HTMLElement | null>,
  header: RefObject<HTMLElement | null>,
  letGo: () => void,
) {
  return useCallback(
    (i: number) => {
      const el = root.current && marksIn(root.current)[i];
      if (!el) return;
      letGo();
      const reduce = matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      // The start's card sits a little below the header; a line that starts a turn, by its padding.
      const top = el.getBoundingClientRect().top + window.scrollY - viewTop(header.current) - 8;
      window.scrollTo({ top: Math.max(0, top), behavior: reduce ? "instant" : "smooth" });
    },
    [root, header, letGo],
  );
}

/** Lights a line up for a moment: where the page went. */
function light(el: HTMLElement) {
  const done = (e: AnimationEvent) => {
    if (e.target !== el) return;
    el.classList.remove("animate-landed");
    el.removeEventListener("animationend", done);
  };
  el.classList.remove("animate-landed");
  void el.offsetWidth;
  el.classList.add("animate-landed");
  el.addEventListener("animationend", done);
}

/** How many frames the page waits for a line to show once what folds it opens. */
const FRAMES = 30;

/**
 * Goes to the call `key` in the transcript of `root`: what folds it opens (its run, the subagent
 * whose work it is), and so does the call, to what it ran and what came of it; then its line comes
 * in under the header, a little of what came before above it, and lights up for a moment. `letGo`
 * lets go of the page's bottom first, which it keeps in view while the agent works.
 */
export function useGoToCall(
  root: RefObject<HTMLElement | null>,
  header: RefObject<HTMLElement | null>,
  letGo: () => void,
  reveal: Revealer,
  items: Item[],
  cwd?: string,
) {
  const latest = useRef({ items, cwd });
  useEffect(() => {
    latest.current = { items, cwd };
  });
  return useCallback(
    (key: string) => {
      const { items, cwd } = latest.current;
      const line = `line:${key}`;
      const found = findables(items, cwd).find((f) => f.id === line);
      const before = reveal.all;
      const wanted = new Map(before);
      for (const [k, how] of found?.reveal ?? []) if (wanted.get(k) !== "open") wanted.set(k, how);
      wanted.set(key, "open");
      reveal.set(wanted);
      let frames = 0;
      const go = () => {
        const el = root.current?.querySelector<HTMLElement>(`[data-find="${CSS.escape(line)}"]`);
        if (!el && frames++ < FRAMES) {
          requestAnimationFrame(go);
          return;
        }
        // Opened, the lines keep what they show: finding can have the reveals back.
        if (reveal.all === wanted) reveal.set(before);
        if (!el) return;
        const row = el.closest<HTMLElement>("button") ?? el;
        letGo();
        const reduce = matchMedia?.("(prefers-reduced-motion: reduce)").matches;
        const top = row.getBoundingClientRect().top + window.scrollY - viewTop(header.current) - 40;
        window.scrollTo({ top: Math.max(0, top), behavior: reduce ? "instant" : "smooth" });
        light(row);
      };
      requestAnimationFrame(go);
    },
    [root, header, letGo, reveal],
  );
}

/** Who started a turn, as its lines say: a person, fctl, or the agent going on by itself. */
function Starter({ turn, size = 18 }: { turn: Turn; size?: number }) {
  const author = turn.ask?.author;
  if (author) return <Avatar user={author} size={size} />;
  if (turn.ask) return <Logo withWord={false} className="[&_svg]:size-[18px]" />;
  return (
    <span className="flex items-center justify-center text-ink-4" style={{ width: size, height: size }}>
      <Sparkle className="size-3.5" />
    </span>
  );
}

/**
 * The bar under the header while the reader is inside a turn whose start has scrolled away: who
 * asked what, which takes them back to it, and the list of turns.
 */
export function TurnBar({
  turn,
  title,
  shown,
  onBack,
  outline,
}: {
  turn?: Turn;
  title: string;
  /** It shows; otherwise it hides above, under the header. */
  shown: boolean;
  onBack: () => void;
  /** The list of turns, which it opens. */
  outline: ReactNode;
}) {
  return (
    <div
      inert={!shown}
      className={cn(
        "turn-bar absolute inset-x-0 top-full border-b border-line bg-canvas/85 backdrop-blur-xl transition-[translate,opacity] duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
        shown ? "translate-y-0 opacity-100" : "pointer-events-none -translate-y-2 opacity-0",
      )}
    >
      <div className="mx-auto flex h-10 w-full max-w-[808px] items-center gap-1 px-2 md:px-5">
        {turn && (
          <button
            type="button"
            onClick={onBack}
            title="Back to where this turn starts"
            className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left text-[12.5px] transition-colors hover:bg-hover"
          >
            <Starter turn={turn} />
            {turn.ask && (
              <span className="shrink-0 font-medium text-ink-2">{turn.ask.author?.name ?? "fctl"}</span>
            )}
            <span className="min-w-0 truncate text-ink-3">{title}</span>
          </button>
        )}
        {outline}
      </div>
    </div>
  );
}

/** How a turn ended, as an icon in the list of turns; the turn the agent works on pulses. */
function EndMark({ turn, live }: { turn: Turn; live: boolean }) {
  const end = turn.end;
  if (!end)
    return live ? (
      <span className="relative flex size-2">
        <span className="absolute inset-0 animate-ping rounded-full bg-accent/60" />
        <span className="relative size-2 rounded-full bg-accent" />
      </span>
    ) : (
      <span className="size-2 rounded-full bg-line-strong" />
    );
  if (end.kind === "result")
    return end.ok ? (
      <CircleCheck className="size-3.5 text-accent" />
    ) : (
      <TriangleAlert className="size-3.5 text-amber" />
    );
  if (end.kind === "failed") return <OctagonX className="size-3.5 text-danger" />;
  if (end.kind === "cut") return <TriangleAlert className="size-3.5 text-amber" />;
  return <CircleSlash className="size-3.5 text-ink-4" />;
}

/**
 * Every turn on the page, in a list that drops from its button: who asked what and when, how it
 * went, how long it took and what it edited. The one the reader is in is marked; a turn opens
 * where it starts.
 */
export function TurnOutline({
  turns,
  facts,
  at,
  live,
  hasMoreBefore,
  onGo,
  className,
  buttonClassName,
}: {
  turns: Turn[];
  /** Each turn's facts, by its key, worked out as the list opens. */
  facts: (turn: Turn) => TurnFacts;
  /** The turn the reader is in. */
  at: number;
  /** The agent works on the last turn. */
  live: boolean;
  hasMoreBefore: boolean;
  onGo: (i: number) => void;
  className?: string;
  buttonClassName?: string;
}) {
  return (
    <Menu
      className={className}
      trigger={(toggle, open) => (
        <button
          type="button"
          onClick={toggle}
          aria-haspopup="menu"
          aria-expanded={open}
          title="Turns"
          className={cn(
            "flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-[12.5px] text-ink-3 transition-colors hover:bg-hover hover:text-ink",
            open && "bg-hover text-ink",
            buttonClassName,
          )}
        >
          <ListTree className="size-4" />
          <span className="max-sm:hidden">Turns</span>
        </button>
      )}
    >
      {(close) => {
        const shares = sharesOf(turns, facts);
        return (
          <div className="turn-outline -m-1 max-h-[min(30rem,calc(100dvh-10rem))] w-[min(26rem,calc(100vw-1.5rem))] overflow-y-auto overscroll-contain p-1">
            {hasMoreBefore && (
              <p className="px-2.5 pt-1.5 pb-2 text-[12px] text-ink-4">
                Earlier turns load as you scroll up.
              </p>
            )}
            {turns.map((turn, i) => (
              <TurnRow
                key={turn.key}
                turn={turn}
                facts={facts(turn)}
                title={turnTitle(turn, i === 0, hasMoreBefore)}
                current={i === at}
                live={live && i === turns.length - 1}
                share={shares[i]}
                role="menuitem"
                onClick={() => {
                  close();
                  onGo(i);
                }}
              />
            ))}
          </div>
        );
      }}
    </Menu>
  );
}

/** Each turn's time as a share of the longest's, for the hairlines under them. */
function sharesOf(turns: Turn[], facts: (turn: Turn) => TurnFacts): number[] {
  const took = turns.map((t) => turnTook(t, facts(t)));
  const longest = Math.max(1, ...took);
  return took.map((ms) => ms / longest);
}

/**
 * A turn as the lists of turns show it: who asked what, when, its steps and the files it edited,
 * how long it took and how it ended. The turn the reader is in is marked.
 */
function TurnRow({
  turn,
  facts: f,
  title,
  current,
  live,
  role,
  compact,
  onClick,
  share,
}: {
  turn: Turn;
  facts: TurnFacts;
  title: string;
  current: boolean;
  /** The agent works on it. */
  live: boolean;
  role?: "menuitem";
  /** In the rail: a line of its title, less room. */
  compact?: boolean;
  /** Its time, as a share of the longest turn's: a hairline that long under it. */
  share?: number;
  onClick: () => void;
}) {
  const took = turnTook(turn, f);
  return (
    <button
      type="button"
      role={role}
      aria-current={current ? "true" : undefined}
      onClick={onClick}
      className={cn(
        "group/turn relative flex w-full items-start gap-2.5 rounded-md text-left transition-colors hover:bg-hover",
        compact ? "px-2 py-1.5" : "px-2.5 py-2",
        current && "bg-hover",
      )}
    >
      {current && (
        <span aria-hidden="true" className="absolute top-2 bottom-2 left-0 w-0.5 rounded-full bg-accent" />
      )}
      {share !== undefined && (
        <span
          aria-hidden="true"
          className={cn(
            "absolute bottom-0.5 left-[38px] h-px rounded-full",
            current ? "bg-accent/50" : "bg-line-strong",
          )}
          style={{ width: `calc((100% - 46px) * ${Math.max(0.02, share)})` }}
        />
      )}
      <span className="mt-px flex size-[18px] shrink-0 items-center justify-center">
        <Starter turn={turn} />
      </span>
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "text-ink",
            compact ? "line-clamp-1 text-[12.5px] leading-5" : "line-clamp-2 text-[13px] leading-[1.4]",
          )}
        >
          {title}
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-1.5 text-[11.5px] text-ink-4">
          <span title={stamp(f.start)}>{agoShort(f.start)}</span>
          {f.calls > 0 && (
            <>
              <span aria-hidden="true">·</span>
              {f.calls} {f.calls === 1 ? "step" : "steps"}
            </>
          )}
          {f.files.length > 0 && (
            <>
              <span aria-hidden="true">·</span>
              {f.files.length} {f.files.length === 1 ? "file" : "files"} edited
            </>
          )}
        </span>
      </span>
      <span className="mt-0.5 flex shrink-0 items-center gap-1.5 text-[11.5px] text-ink-4 tabular-nums">
        {(turn.end || live) && took >= 1000 && duration(took)}
        <span className="flex size-3.5 items-center justify-center">
          <EndMark turn={turn} live={live} />
        </span>
      </span>
    </button>
  );
}

/**
 * Every turn, in the details beside the transcript, as a table of contents: the one the reader is
 * in is marked as they read, and kept in view in the list, which scrolls on its own when long.
 */
/** How many of the newest turns the rail lists before the earlier ones are a click away. */
const LISTED = 6;

export function TurnList({
  turns,
  facts,
  at,
  live,
  hasMoreBefore,
  onGo,
}: {
  turns: Turn[];
  facts: (turn: Turn) => TurnFacts;
  at: number;
  live: boolean;
  hasMoreBefore: boolean;
  onGo: (i: number) => void;
}) {
  const shares = sharesOf(turns, facts);
  // In the rail, which scrolls: the newest turns, and the earlier ones a click away, rather than
  // a list that scrolls in it. The turn the reader is in always shows, the list growing back to it.
  const [all, setAll] = useState(false);
  const newest = turns.length - LISTED;
  const folded = !all && turns.length > LISTED + 2 ? (at >= 0 ? Math.min(newest, at) : newest) : 0;
  return (
    <div className="-mx-2">
      {hasMoreBefore && (
        <p className="px-2 pb-1 text-[11.5px] text-ink-4">Earlier turns load as you scroll up.</p>
      )}
      {folded > 0 && (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="mb-1 flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
        >
          <ChevronsUp aria-hidden="true" className="size-3.5 shrink-0" />
          Show {folded} earlier {folded === 1 ? "turn" : "turns"}
        </button>
      )}
      {turns.slice(folded).map((turn, j) => {
        const i = folded + j;
        return (
          <TurnRow
            key={turn.key}
            turn={turn}
            facts={facts(turn)}
            title={turnTitle(turn, i === 0, hasMoreBefore)}
            current={i === at}
            live={live && i === turns.length - 1}
            share={shares[i]}
            compact
            onClick={() => onGo(i)}
          />
        );
      })}
    </div>
  );
}

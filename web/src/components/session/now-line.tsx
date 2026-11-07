// Above the new session page's message box (routes/session-next.tsx): what the agent is doing at
// this moment, as it changes, and for how long, each new step rolling in from below as the last
// rolls away above; while it waits for work it left in the background, what that is, in a list the
// line opens into; and beside it, the way back to the newest work, with how much came since the
// reader left it, or up to what came while they were away.

import {
  ArrowDown,
  ArrowUp,
  Box,
  ChevronUp,
  CirclePause,
  Hourglass,
  MessageSquareReply,
  PenLine,
  Square,
  Unplug,
} from "lucide-react";
import {
  type CSSProperties,
  createContext,
  type ReactNode,
  type RefObject,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { DotWave, ThoughtIcon } from "@/components/ui";
import { cn } from "@/lib/cn";
import type { Now, Waiting } from "@/lib/now";
import type { Plan } from "@/lib/plan";
import { NEAR_BOTTOM } from "@/lib/stick-to-bottom";
import { BackgroundList, ran } from "./background";
import { Rolling } from "./diff-live";
import { PlanRing } from "./plan";
import { toolIcon } from "./tool";

/** How long a step shows at least, so that quick ones don't flicker past unread (ms). */
const STEADY_MS = 600;

/**
 * `value`, changing no more often than every `ms`: a change that comes sooner shows once the last
 * one has shown that long, unless another takes its place first. `id` says when it changed; a
 * value `settle` gives time to shows only once it has lasted that long (ms).
 */
function useSteady<T>(value: T, id: (v: T) => string, settle?: (v: T) => number, ms = STEADY_MS): T {
  const key = id(value);
  const [shownKey, setShownKey] = useState(key);
  // The step shown, as it was last: the same step's newer facts (its seconds) show as they come.
  const [held, setHeld] = useState(value);
  const since = useRef(0);
  if (key === shownKey && held !== value) setHeld(value);
  const lasts = settle?.(value) ?? 0;
  useEffect(() => {
    if (key === shownKey) return;
    const show = () => {
      since.current = Date.now();
      setShownKey(key);
    };
    const wait = Math.max(since.current + ms - Date.now(), lasts);
    if (wait <= 0) {
      show();
      return;
    }
    const t = setTimeout(show, wait);
    return () => clearTimeout(t);
  }, [key, shownKey, ms, lasts]);
  return key === shownKey ? value : held;
}

/**
 * How long the agent waits before the line says so (ms): between a background task's end and the
 * turn that takes up what came of it, its session waits for a message for a moment, which isn't
 * the reader's turn yet.
 */
const SETTLE_MS = 400;

/** The time, again each second while `on`. */
function useSeconds(on: boolean): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [on]);
  return now;
}

function icon(now: Now): ReactNode {
  switch (now.kind) {
    case "thinking":
      return <ThoughtIcon className="size-3.5 text-violet" />;
    case "writing":
      return <PenLine className="size-3.5 text-accent" />;
    case "preparing":
    case "working":
      return <DotWave className="text-accent" />;
    case "tool":
      return <span className="flex text-ink-2 [&_svg]:size-3.5">{toolIcon(now.tool ?? "")}</span>;
    case "setup":
      return <Box className="size-3.5 text-ink-2" />;
    case "background":
      return <Hourglass className="size-3.5 text-blue" />;
    case "paused":
      return <CirclePause className="size-3.5 text-pink" />;
    case "stopping":
      return <Square className="size-3 text-ink-3" />;
    default:
      return <Unplug className="size-3.5 text-amber" />;
  }
}

/** Kinds of moment the agent is busy in, whose words shimmer. */
const BUSY = new Set<Now["kind"]>(["thinking", "writing", "preparing", "tool", "setup", "working"]);

/** What tells a step from the next: what it does and to what, not its figures, which move in it. */
const stepOf = (n: Now) => `${n.kind}\u0000${n.text}\u0000${n.target?.text ?? ""}`;

/** What tells a step's icon from the next one's. */
const iconOf = (n: Now) => (n.kind === "tool" ? `tool\u0000${n.tool ?? ""}` : n.kind);

/** How long the line takes to glide to its new width (ms). */
const GLIDE_MS = 420;
/** By when what rolled away has gone, at the latest (ms): its animations take less. */
const GONE_MS = 600;

const reduced = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/**
 * The line glides to the width of its new step rather than jumping there: from the width it had
 * to the one it takes, its words kept whole on the way (styles/app.css), then free again to follow
 * them. A step that comes before it gets there glides on from where it is.
 */
function useGlide(ref: RefObject<HTMLElement | null>, step: string) {
  const width = useRef(0);
  const last = useRef(step);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) {
      width.current = 0;
      return;
    }
    const gliding = el.dataset.glide !== undefined;
    if (last.current === step) {
      // Between steps, it follows its words: the width to glide from next.
      if (!gliding) width.current = el.getBoundingClientRect().width;
      return;
    }
    last.current = step;
    const from = gliding ? el.getBoundingClientRect().width : width.current;
    clearTimeout(timer.current);
    el.style.width = "";
    delete el.dataset.glide;
    const to = el.getBoundingClientRect().width;
    width.current = to;
    if (!from || Math.abs(to - from) < 1 || reduced()) return;
    el.style.width = `${from}px`;
    el.dataset.glide = "";
    el.getBoundingClientRect();
    el.style.width = `${to}px`;
    timer.current = setTimeout(() => {
      el.style.width = "";
      delete el.dataset.glide;
      width.current = el.getBoundingClientRect().width;
    }, GLIDE_MS);
  });
}

/** What showed for a step that rolls away, and how wide it was. */
interface Gone {
  step: string;
  node: ReactNode;
  width?: number;
}

/**
 * `children` as they are for `step`: as it changes, the new roll in from below while the last roll
 * away above, as it was, out of the line, which clips them. The first shows at once, as the line
 * rises.
 */
function Roll({ step, className, children }: { step: string; className?: string; children: ReactNode }) {
  const [at, setAt] = useState(step);
  const [rolled, setRolled] = useState(false);
  const [gone, setGone] = useState<Gone[]>([]);
  // What showed for the step before, as it last was and as wide, to roll away just so.
  const shown = useRef<HTMLSpanElement>(null);
  const last = useRef<Gone>({ step, node: children });
  if (step !== at) {
    const before = last.current;
    setAt(step);
    setRolled(true);
    setGone((g) => [...g.filter((x) => x.step !== before.step), before].slice(-2));
  }
  useLayoutEffect(() => {
    last.current = { step, node: children, width: shown.current?.getBoundingClientRect().width };
  });
  // Gone by the time it has rolled away, whether its animation said so or not (hidden, it doesn't).
  useEffect(() => {
    if (!gone.length) return;
    const t = setTimeout(() => setGone([]), GONE_MS);
    return () => clearTimeout(t);
  }, [gone]);
  return (
    <span className={cn("relative flex min-w-0", className)}>
      <span
        ref={shown}
        key={step}
        className={cn("flex min-w-0 items-baseline gap-1.5", rolled && "animate-now-in")}
      >
        <Fresh>{children}</Fresh>
      </span>
      {gone.map((g) => (
        <span
          key={`gone\u0000${g.step}`}
          aria-hidden="true"
          style={{ width: g.width }}
          className="pointer-events-none absolute inset-y-0 left-0 flex min-w-0 animate-now-out items-baseline gap-1.5"
          onAnimationEnd={(e) => {
            if (e.target === e.currentTarget) setGone((x) => x.filter((y) => y !== g));
          }}
        >
          <WithStep.Provider value={true}>{g.node}</WithStep.Provider>
        </span>
      ))}
    </span>
  );
}

/**
 * Whether what is in a step came with it, as it rolled in: what comes later in the same step rolls
 * in by itself (a thought's count, once Claude Code says it).
 */
const WithStep = createContext(false);

function Fresh({ children }: { children: ReactNode }) {
  const [fresh, setFresh] = useState(true);
  useEffect(() => setFresh(false), []);
  return <WithStep.Provider value={fresh}>{children}</WithStep.Provider>;
}

/** A thought's tokens in a few figures that keep their places as they grow: "860", "4.2K", "12.0K". */
export function tokenFigures(n: number): string {
  if (n < 1000) return String(Math.max(0, Math.round(n)));
  if (n < 999_950) return `${(n / 1000).toFixed(1)}K`;
  return `${(n / 1e6).toFixed(1)}M`;
}

/** A figure in a slot of its own, which rolls to the next as it changes: up as the count grows. */
function Figure({ c, dir, enter }: { c: string; dir: 1 | -1; enter: boolean }) {
  const [shown, setShown] = useState(c);
  const [from, setFrom] = useState<string | undefined>(undefined);
  const [moved, setMoved] = useState(enter);
  if (c !== shown) {
    setFrom(shown);
    setShown(c);
    setMoved(true);
  }
  useEffect(() => {
    if (from === undefined) return;
    const t = setTimeout(() => setFrom(undefined), GONE_MS);
    return () => clearTimeout(t);
  }, [from]);
  const style = { "--dir": dir } as CSSProperties;
  return (
    <span className="relative inline-grid overflow-clip" style={style}>
      {from !== undefined && (
        <span
          key={`${from}\u0000${c}`}
          aria-hidden="true"
          className="absolute inset-0 animate-digit-out"
          onAnimationEnd={(e) => {
            if (e.target === e.currentTarget) setFrom(undefined);
          }}
        >
          {from}
        </span>
      )}
      <span key={c} className={cn(moved && "animate-digit-in")}>
        {c}
      </span>
    </span>
  );
}

/**
 * A count whose figures roll one by one as it changes, each in its slot, as an odometer's do:
 * only those that changed move. Slots count from the right, so the units stay put as it grows.
 */
function Odometer({ value, text }: { value: number; text: string }) {
  const [last, setLast] = useState(value);
  const [dir, setDir] = useState<1 | -1>(1);
  if (value !== last) {
    setLast(value);
    setDir(value > last ? 1 : -1);
  }
  // Slots that come once it shows roll in too.
  const shown = useRef(false);
  useEffect(() => {
    shown.current = true;
  }, []);
  const chars = [...text];
  return (
    <span className="inline-flex tabular-nums">
      {chars.map((c, i) => (
        <Figure key={chars.length - i} c={c} dir={dir} enter={shown.current} />
      ))}
    </span>
  );
}

/** How often a count changes at most (ms): each roll ends before the next, for it to be read. */
const COUNT_MS = 450;

/** `value`, changing at most every `ms`: the newest one when its time comes. */
function useThrottled<T>(value: T, ms: number): T {
  const [shown, setShown] = useState(value);
  const last = useRef(0);
  useEffect(() => {
    if (Object.is(value, shown)) return;
    const show = () => {
      last.current = Date.now();
      setShown(value);
    };
    const wait = last.current + ms - Date.now();
    if (wait <= 0) {
      show();
      return;
    }
    const t = setTimeout(show, wait);
    return () => clearTimeout(t);
  }, [value, shown, ms]);
  return shown;
}

/** How far a thought got: "4.2K tokens", its figures rolling as it goes on. */
function Tokens({ n }: { n: number }) {
  // Once its thought has shown for a while: it rolls in by itself.
  const [later] = useState(!useContext(WithStep));
  // Screen readers hear the step, not each figure of it.
  return (
    <span
      aria-hidden="true"
      className={cn("flex shrink-0 items-baseline gap-1.5", later && "animate-now-in")}
    >
      <span className="text-ink-4">·</span>
      <span className="text-ink-3">
        <Odometer value={n} text={tokenFigures(n)} /> {n === 1 ? "token" : "tokens"}
      </span>
    </span>
  );
}

/**
 * What the agent does now, in a line that stays put above the message box: the step it is at and
 * what it does it to, for how long, each new step rolling in as it moves on. While it waits for
 * its background work, what for, which the line opens into: each task, going to where it started.
 * Once it waits for the reader, that it's their turn, which takes them to the message box.
 */
export function NowLine({
  now,
  plan,
  onReply,
  background,
}: {
  now?: Now;
  /** The plan of the turn the agent works on. */
  plan?: Plan;
  /** It's the reader's turn while the agent does nothing: the line says so, and goes to their reply. */
  onReply?: () => void;
  /** What the agent waits for in the background, while it does, and how to go to each. */
  background?: {
    waiting: Waiting;
    cwd?: string;
    /** The session has activity before what loaded here. */
    earlier: boolean;
    /** It loads some now. */
    looking: boolean;
    /** Loads more of it, for tasks that started before; unset once it has looked far enough. */
    onLookBack?: () => void;
    /** Goes to the call that started a task. */
    onGo: (call: string) => void;
  };
}) {
  // Mounted whether the agent works or not, so that its last step holds as long as a step does;
  // once it does nothing, whose turn it is, when it's the reader's, once it has stayed so a moment.
  const line = useSteady<Now | "turn" | undefined>(
    now ?? (onReply ? "turn" : undefined),
    (l) => (l === undefined ? "" : l === "turn" ? "turn" : stepOf(l)),
    (l) => (l === undefined || l === "turn" ? SETTLE_MS : 0),
  );
  const shown = line === "turn" ? undefined : line;
  const turn = line === "turn";
  const busy = !!shown && BUSY.has(shown.kind);
  // The step of its plan the agent is at, in the words it says while it does it, leads the line.
  const step = busy ? plan?.current : undefined;
  const doing = step && (step.doing ?? step.text);
  const ring = busy && !!plan && plan.done < plan.steps.length;
  // What it waits for in the background, which the line opens into while it does.
  const waiting =
    shown?.kind === "background" && background && (background.waiting.tasks.length || background.waiting.more)
      ? background
      : undefined;
  const [open, setOpen] = useState(false);
  if (open && !waiting) setOpen(false);
  const listId = useId();
  const wrap = useRef<HTMLDivElement>(null);
  const toggle = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setOpen(false);
      toggle.current?.focus();
    };
    document.addEventListener("pointerdown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  const time = useSeconds((!!shown?.since && shown.kind !== "paused") || open);
  const took = !shown
    ? undefined
    : shown.seconds !== undefined && shown.seconds > 1
      ? ran(shown.seconds * 1000)
      : shown.since && shown.kind !== "paused"
        ? ran(time - shown.since)
        : undefined;
  // A thought's count, as often as it can be read.
  const counted = useThrottled(shown?.kind === "thinking" ? shown.tokens : undefined, COUNT_MS);
  const tokens = shown?.kind === "thinking" ? counted : undefined;
  const figures = tokens ? tokenFigures(tokens) : "";
  const pill = useRef<HTMLOutputElement>(null);
  // It glides to its new width as it changes shape: another step, a count that shows or grows a
  // figure, a time that does, the plan's ring.
  useGlide(
    pill,
    turn
      ? "turn"
      : shown
        ? [stepOf(shown), doing ?? "", figures.length, took?.length ?? 0, ring, !!waiting].join("\u0000")
        : "",
  );
  if (!shown && !turn) return null;
  const target = shown?.target;
  const words = shown ? (
    <>
      <span
        className={cn(
          "min-w-0 truncate",
          target && "max-w-[60%] shrink-0",
          doing
            ? "text-ink-3"
            : busy
              ? "text-shimmer"
              : shown.kind === "stalled"
                ? "text-amber"
                : "text-ink-2",
        )}
      >
        {shown.text}
      </span>
      {figures && tokens !== undefined && <Tokens n={tokens} />}
      {target && (
        <span
          className={cn(
            "min-w-0 truncate text-ink-3",
            target.kind !== "text" && "font-mono text-[11.5px]",
            target.kind === "path" && "[direction:rtl] text-left",
          )}
        >
          {/* A path keeps its file's name in view: it gives way from its start. */}
          {target.kind === "path" ? <bdi>{target.text}</bdi> : target.text}
        </span>
      )}
      {/* Screen readers hear the step, not each second of it. */}
      {took && (
        <span aria-hidden="true" className="shrink-0 text-[11.5px] text-ink-4 tabular-nums">
          {took}
        </span>
      )}
    </>
  ) : (
    <>
      <span className="shrink-0 font-medium text-ink">Your turn</span>
      <span className="min-w-0 truncate text-ink-3 max-sm:hidden">The agent waits for your reply</span>
    </>
  );
  return (
    <div ref={wrap} className="relative flex min-w-0 max-w-full">
      <output
        ref={pill}
        aria-live="polite"
        className={cn(
          "now-line pointer-events-auto relative flex h-8 max-w-full min-w-0 animate-rise overflow-hidden rounded-full border bg-surface/85 text-[12.5px] shadow-[0_6px_24px_rgb(0_0_0/0.18)] backdrop-blur-xl transition-[border-color] duration-300",
          turn ? "your-turn border-blue/30 hover:border-blue/60" : "border-line",
          waiting && "hover:border-line-strong",
          open && "border-line-strong",
          // The button over it takes the focus: the ring goes around the line.
          "has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent has-[:focus-visible]:outline-solid",
        )}
      >
        <span className="now-row flex h-full min-w-0 items-center gap-2 py-1 pr-3 pl-1">
          <span
            data-busy={busy || undefined}
            className={cn(
              "busy-ring relative flex size-6 shrink-0 items-center justify-center rounded-full transition-colors duration-300",
              turn ? "bg-blue-soft" : "bg-raised",
            )}
          >
            {/* The icon rolls inside its circle, which the ring goes around. */}
            <span className="absolute inset-0 flex items-center justify-center overflow-hidden rounded-full">
              <Roll step={shown ? iconOf(shown) : "turn"} className="justify-center">
                {shown ? icon(shown) : <MessageSquareReply className="size-3.5 text-blue" />}
              </Roll>
            </span>
          </span>
          {doing && (
            <Roll step={doing} className="shrink-[0.6]">
              <span className="min-w-0 truncate font-medium text-shimmer">{doing}</span>
            </Roll>
          )}
          <Roll
            step={shown ? stepOf(shown) : "turn"}
            // Beside the plan's step, the call at hand: where there's room.
            className={cn(doing && "border-l border-line pl-2 max-sm:hidden")}
          >
            {words}
          </Roll>
          {ring && plan && (
            <PlanRing
              done={plan.done}
              total={plan.steps.length}
              className="-mr-0.5 shrink-0 border-l border-line pl-2"
            />
          )}
          {waiting && (
            <ChevronUp
              aria-hidden="true"
              className={cn(
                "-mr-1 size-3.5 shrink-0 text-ink-4 transition-transform duration-200 ease-[var(--ease-out)]",
                open && "rotate-180",
              )}
            />
          )}
        </span>
        {/* Over the line, what it does when it does something: the line stays as it is under it,
            for its steps to roll on as they change. */}
        {turn ? (
          <button
            type="button"
            aria-label="Your turn: reply to the agent"
            onClick={onReply}
            className="absolute inset-0 rounded-full focus-visible:outline-none"
          />
        ) : (
          waiting && (
            <button
              ref={toggle}
              type="button"
              aria-label={`${shown?.text ?? ""}${target ? ` ${target.text}` : ""}`}
              aria-expanded={open}
              aria-controls={open ? listId : undefined}
              onClick={() => setOpen((o) => !o)}
              className="absolute inset-0 rounded-full focus-visible:outline-none"
            />
          )
        )}
      </output>
      {open && waiting && (
        <BackgroundList
          id={listId}
          waiting={waiting.waiting}
          now={time}
          cwd={waiting.cwd}
          earlier={waiting.earlier}
          looking={waiting.looking}
          onLookBack={waiting.onLookBack}
          onGo={(call) => {
            setOpen(false);
            waiting.onGo(call);
          }}
        />
      )}
    </div>
  );
}

/** Whether the page is scrolled to its bottom, or near it, and how far above it it is. */
export function useAtBottom(): boolean {
  return useSyncExternalStore(subscribeScroll, atBottom, () => true);
}

const atBottom = () =>
  document.documentElement.scrollHeight - window.scrollY - window.innerHeight <= NEAR_BOTTOM * 3;

function subscribeScroll(fn: () => void) {
  let raf = 0;
  const on = () => {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      fn();
    });
  };
  window.addEventListener("scroll", on, { passive: true });
  window.addEventListener("resize", on, { passive: true });
  // The page grows under a reader who stays where they are.
  const grown = new ResizeObserver(on);
  grown.observe(document.documentElement);
  return () => {
    cancelAnimationFrame(raf);
    window.removeEventListener("scroll", on);
    window.removeEventListener("resize", on);
    grown.disconnect();
  };
}

/** Scrolls the page down to its bottom, which then keeps in view as the agent works on. */
export function toBottom() {
  const reduce = matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  window.scrollTo({ top: document.documentElement.scrollHeight, behavior: reduce ? "instant" : "smooth" });
}

/**
 * The button back to the newest work, while the reader is away from it: how many parts came
 * since they left it, if any. Or, with `up`, up to where what came while they were away starts.
 */
export function JumpPill({ count, up, onClick }: { count: number; up?: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="jump-pill pointer-events-auto ml-auto flex h-8 shrink-0 animate-rise items-center gap-1.5 rounded-full border border-line-strong bg-raised/90 pr-3 pl-2.5 text-[12.5px] font-medium text-ink-2 shadow-[0_6px_24px_rgb(0_0_0/0.22)] backdrop-blur-xl transition-[color,background-color,scale] duration-150 hover:text-ink active:scale-95"
    >
      {up ? <ArrowUp className="size-3.5" /> : <ArrowDown className="size-3.5" />}
      {count > 0 ? (
        <>
          <span className="tabular-nums">
            <Rolling value={count} />
          </span>{" "}
          new
        </>
      ) : (
        "Latest"
      )}
    </button>
  );
}

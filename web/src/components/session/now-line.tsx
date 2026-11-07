// Above the new session page's message box (routes/session-next.tsx): what the agent is doing at
// this moment, as it changes, and for how long; and beside it, the way back to the newest work,
// with how much came since the reader left it, or up to what came while they were away.

import {
  ArrowDown,
  ArrowUp,
  Box,
  CirclePause,
  Hourglass,
  MessageSquareReply,
  PenLine,
  Square,
  Unplug,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { DotWave, ThoughtIcon } from "@/components/ui";
import { cn } from "@/lib/cn";
import { duration } from "@/lib/format";
import type { Now } from "@/lib/now";
import type { Plan } from "@/lib/plan";
import { NEAR_BOTTOM } from "@/lib/stick-to-bottom";
import { Rolling } from "./diff-live";
import { PlanRing } from "./plan";
import { toolIcon } from "./tool";

/** How long a step shows at least, so that quick ones don't flicker past unread (ms). */
const STEADY_MS = 600;

/**
 * `value`, changing no more often than every `ms`: a change that comes sooner shows once the last
 * one has shown that long, unless another takes its place first. `id` says when it changed.
 */
function useSteady<T>(value: T, id: (v: T) => string, ms = STEADY_MS): T {
  const key = id(value);
  const [shownKey, setShownKey] = useState(key);
  // The step shown, as it was last: the same step's newer facts (its seconds) show as they come.
  const [held, setHeld] = useState(value);
  const since = useRef(0);
  if (key === shownKey && held !== value) setHeld(value);
  useEffect(() => {
    if (key === shownKey) return;
    const show = () => {
      since.current = Date.now();
      setShownKey(key);
    };
    const wait = since.current + ms - Date.now();
    if (wait <= 0) {
      show();
      return;
    }
    const t = setTimeout(show, wait);
    return () => clearTimeout(t);
  }, [key, shownKey, ms]);
  return key === shownKey ? value : held;
}

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

/** How long something has run: "12s", "1m 4s". */
const ran = (ms: number) => {
  const s = Math.max(0, Math.round(ms / 1000));
  return s < 60 ? `${s}s` : duration(s * 1000);
};

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
      return <span className="text-ink-2 [&_svg]:size-3.5">{toolIcon(now.tool ?? "")}</span>;
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

/**
 * What the agent does now, in a line that stays put above the message box: the step it is at and
 * what it does it to, for how long, the words rolling in as it moves on. Nothing while it waits.
 */
export function NowLine({
  now,
  plan,
  idle,
}: {
  now?: Now;
  /** The plan of the turn the agent works on. */
  plan?: Plan;
  /** What the line says while the agent does nothing: that it's the reader's turn. */
  idle?: ReactNode;
}) {
  // Mounted whether the agent works or not, so that its last step holds as long as a step does.
  const shown = useSteady(now, (n) => (n ? `${n.kind}\u0000${n.text}\u0000${n.target?.text ?? ""}` : ""));
  const ticking = !!shown?.since && shown.kind !== "paused";
  const time = useSeconds(ticking);
  if (!shown) return idle ?? null;
  const busy = BUSY.has(shown.kind);
  // The step of its plan the agent is at, in the words it says while it does it, leads the line.
  const step = busy ? plan?.current : undefined;
  const doing = step && (step.doing ?? step.text);
  const took =
    shown.seconds !== undefined && shown.seconds > 1
      ? ran(shown.seconds * 1000)
      : ticking && shown.since
        ? ran(time - shown.since)
        : undefined;
  const target = shown.target;
  return (
    <output
      aria-live="polite"
      className="now-line pointer-events-auto flex h-8 max-w-full min-w-0 animate-rise items-center gap-2 rounded-full border border-line bg-surface/85 py-1 pr-3 pl-1 text-[12.5px] shadow-[0_6px_24px_rgb(0_0_0/0.18)] backdrop-blur-xl"
    >
      <span
        className={cn(
          "relative flex size-6 shrink-0 items-center justify-center rounded-full bg-raised",
          busy && "busy-ring",
        )}
      >
        {icon(shown)}
      </span>
      {doing && (
        <span key={doing} className="min-w-0 shrink-[0.6] animate-roll-in truncate font-medium text-shimmer">
          {doing}
        </span>
      )}
      <span
        key={`${shown.kind}\u0000${shown.text}\u0000${target?.text ?? ""}`}
        className={cn(
          "flex min-w-0 animate-roll-in items-baseline gap-1.5",
          // Beside the plan's step, the call at hand: where there's room.
          doing && "border-l border-line pl-2 max-sm:hidden",
        )}
      >
        <span
          className={cn(
            "min-w-0 truncate",
            target && "shrink-0 max-w-[60%]",
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
      </span>
      {/* Screen readers hear the step, not each second of it. */}
      {took && (
        <span aria-hidden="true" className="shrink-0 text-[11.5px] text-ink-4 tabular-nums">
          {took}
        </span>
      )}
      {busy && plan && plan.done < plan.steps.length && (
        <PlanRing
          done={plan.done}
          total={plan.steps.length}
          className="-mr-0.5 shrink-0 border-l border-line pl-2"
        />
      )}
    </output>
  );
}

/**
 * Above the message box once the agent has ended its turn and waits for someone to write: whose
 * turn it is, which takes them to the message box.
 */
export function YourTurn({ onReply }: { onReply: () => void }) {
  return (
    <button
      type="button"
      onClick={onReply}
      className="now-line your-turn pointer-events-auto flex h-8 max-w-full min-w-0 animate-rise items-center gap-2 rounded-full border border-blue/30 bg-surface/85 py-1 pr-3 pl-1 text-[12.5px] shadow-[0_6px_24px_rgb(0_0_0/0.18)] backdrop-blur-xl transition-[border-color] duration-150 hover:border-blue/60"
    >
      <span className="flex size-6 shrink-0 items-center justify-center rounded-full bg-blue-soft text-blue">
        <MessageSquareReply className="size-3.5" />
      </span>
      <span className="shrink-0 font-medium text-ink">Your turn</span>
      <span className="min-w-0 truncate text-ink-3 max-sm:hidden">The agent waits for your reply</span>
    </button>
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

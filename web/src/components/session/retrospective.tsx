// A retrospective in a session's transcript, as the agent recorded it or updated it then: how the
// work ended and the gist of each part in a line, which opens to all it says. An update marks
// what it changed since the transcript last had it.

import { ArrowRight, ChevronDown, Lightbulb, ListChecks, Mountain, Route, ScrollText } from "lucide-react";
import { Fragment, useId, useLayoutEffect, useRef, useState } from "react";
import { OutcomeBadge, RetrospectiveParts } from "@/components/retrospective";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { plainText } from "@/lib/find";
import { changes, parts } from "@/lib/retrospectives";
import { useRevealed } from "./find";

/** How long the card takes to open or close (ms). */
const GLIDE_MS = 320;

/** A dot beside a part an update changed. */
function Changed() {
  return (
    <span
      aria-hidden="true"
      className="inline-block size-1.5 shrink-0 rounded-full bg-accent"
      title="Changed in this update"
    />
  );
}

/** A part in one line, as its gist shows it: its words as they read, its lines run together. */
const inALine = (text: string) => plainText(text).replace(/\s+/g, " ").trim();

/** Each part's mark, which stands for its name where there's no room for it (a phone). */
const PART_ICON: Record<string, typeof ListChecks> = {
  worked_on: ListChecks,
  approach: Route,
  challenges: Mountain,
  insights: Lightbulb,
};

/**
 * Each part in a line beside its name (its mark on a phone): what the agent worked on leads, in
 * two lines where it takes them, the rest follow quieter, and in an update, what it changed stands out while the rest steps back.
 * Its columns and type are those of the parts in full, so nothing moves as it opens.
 */
function Gist({
  r,
  changed,
  onOpen,
}: {
  r: G.Retrospective;
  changed?: ReadonlySet<string>;
  onOpen: () => void;
}) {
  return (
    <div className="relative grid grid-cols-[auto_minmax(0,1fr)] gap-x-2.5 gap-y-1 sm:grid-cols-[5.75rem_minmax(0,1fr)] sm:gap-x-5">
      {parts(r).map((p) => {
        const marked = changed?.has(p.key);
        const Icon = PART_ICON[p.key] ?? ListChecks;
        return (
          <Fragment key={p.key}>
            {/* As tall as a line beside it, its mark centred on it where its name is for screen readers. */}
            <span className="flex h-[22px] items-center gap-1.5 self-start text-[12px] leading-[22px] whitespace-nowrap text-ink-4">
              <Icon aria-hidden="true" className="size-3.5 sm:hidden" />
              <span className="max-sm:sr-only">{p.label}</span>
              {marked && <Changed />}
              {marked && <span className="sr-only"> (changed)</span>}
            </span>
            <span
              className={cn(
                "text-[13.5px] leading-[22px] [overflow-wrap:anywhere]",
                // What it worked on leads, in two lines where it takes them.
                p.key === "worked_on" ? "line-clamp-2" : "truncate",
                changed
                  ? marked
                    ? "text-ink-2"
                    : "text-ink-4"
                  : p.key === "worked_on"
                    ? "text-ink-2"
                    : "text-ink-3",
              )}
            >
              {inALine(p.text)}
            </span>
          </Fragment>
        );
      })}
      {/* The gist opens it all to a pointer or a finger anywhere on it; keys use the header's button. */}
      <button
        type="button"
        tabIndex={-1}
        aria-hidden="true"
        onClick={onOpen}
        className="absolute -inset-x-4 -top-1 -bottom-3.5 cursor-pointer"
      />
    </div>
  );
}

/**
 * The retrospective a notice holds: its outcome, and each part in a line until the reader opens
 * it to all it says. Finding opens it to a match in it.
 */
export function RetrospectiveCard({
  r,
  before,
  findKey,
}: {
  r: G.Retrospective;
  /** How it was before this update, where the transcript has that. */
  before?: G.Retrospective;
  /** The notice's key, which finding knows its parts by. */
  findKey: string;
}) {
  const [open, setOpen] = useState(false);
  useRevealed(`retro:${findKey}`, () => setOpen(true));
  const id = useId();
  const changed = before ? changes(before, r) : undefined;
  // It glides from the height it had to the one it takes.
  const body = useRef<HTMLDivElement>(null);
  const from = useRef<number | undefined>(undefined);
  const toggle = (to: boolean) => {
    from.current = body.current?.getBoundingClientRect().height;
    setOpen(to);
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: it glides as it opens or closes
  useLayoutEffect(() => {
    const el = body.current;
    const start = from.current;
    from.current = undefined;
    if (!el || start === undefined || matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    const end = el.getBoundingClientRect().height;
    if (Math.abs(end - start) < 1) return;
    el.animate([{ height: `${start}px` }, { height: `${end}px` }], {
      duration: GLIDE_MS,
      easing: "cubic-bezier(0.16, 1, 0.3, 1)",
    });
  }, [open]);
  return (
    <section
      aria-label={r.updated_at ? "Retrospective updated" : "Retrospective"}
      className="retro my-2 animate-rise rounded-xl border border-line bg-raised/40"
    >
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => toggle(!open)}
        className="group/retro flex w-full min-w-0 flex-wrap items-center gap-x-2 gap-y-1 rounded-t-xl px-4 pt-3 pb-2 text-left"
      >
        <ScrollText aria-hidden="true" className="size-3.5 shrink-0 text-ink-3" />
        <span className="text-[12.5px] font-medium text-ink-2">
          {r.updated_at ? "Retrospective updated" : "Retrospective"}
        </span>
        {changed?.outcome && (
          <span className="flex items-center gap-1.5" title="How the work ended, before this update">
            <OutcomeBadge outcome={changed.outcome} className="opacity-60" />
            <ArrowRight aria-label="now" className="size-3 text-ink-4" />
          </span>
        )}
        <OutcomeBadge outcome={r.outcome} />
        <span className="ml-auto flex items-center gap-1 text-[12px] text-ink-4 transition-colors group-hover/retro:text-ink-2">
          <span className="max-sm:sr-only">{open ? "Show less" : "Show all"}</span>
          <ChevronDown
            aria-hidden="true"
            className={cn("size-3.5 transition-transform duration-200 ease-out", open && "rotate-180")}
          />
        </span>
      </button>
      <div ref={body} id={id} className="overflow-hidden px-4 pb-3.5">
        {open ? (
          <div className="animate-fade">
            <RetrospectiveParts r={r} wide findKey={findKey} changed={changed?.parts} />
          </div>
        ) : (
          <Gist r={r} changed={changed?.parts} onOpen={() => toggle(true)} />
        )}
      </div>
    </section>
  );
}

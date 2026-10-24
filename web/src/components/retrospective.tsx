// A retrospective, as the session page, its transcript and the project's
// Retrospectives tab show it: how the work ended, what the agent wrote of
// it, and when. An insight can go into the project's memory from here.

import { BookMarked, CircleCheck, CircleDashed, Hand } from "lucide-react";
import { useState } from "react";
import { Markdown } from "@/components/markdown";
import { RememberDialog } from "@/components/remember";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { ago, stamp } from "@/lib/format";
import { outcomeLabel, parts } from "@/lib/retrospectives";

const OUTCOME_ICON = { completed: CircleCheck, partial: CircleDashed, blocked: Hand } as const;
const OUTCOME_TONE = { completed: "text-accent", partial: "text-blue", blocked: "text-pink" } as const;
const OUTCOME_FILL = {
  completed: "border-accent/25 bg-accent-soft",
  partial: "border-blue/25 bg-blue-soft",
  blocked: "border-pink/25 bg-pink-soft",
} as const;

/** How a piece of work ended, as an icon in its color: identity rides on the label beside it. */
export function OutcomeIcon({ outcome, className }: { outcome: G.RetrospectiveOutcome; className?: string }) {
  const Icon = OUTCOME_ICON[outcome];
  return <Icon aria-hidden="true" className={cn("size-3.5 shrink-0", OUTCOME_TONE[outcome], className)} />;
}

/** The outcome as a small pill: its icon, and its name in ink. */
export function OutcomeBadge({
  outcome,
  className,
}: {
  outcome: G.RetrospectiveOutcome;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex h-5.5 shrink-0 items-center gap-1.5 rounded-full border px-2 text-[11.5px] font-medium text-ink",
        OUTCOME_FILL[outcome],
        className,
      )}
    >
      <OutcomeIcon outcome={outcome} className="size-3" />
      {outcomeLabel(outcome)}
    </span>
  );
}

/** When the agent recorded it, or last updated it, which it says. */
export function RetrospectiveTime({ r, className }: { r: G.Retrospective; className?: string }) {
  const at = r.updated_at ?? r.created_at;
  return (
    <time
      dateTime={new Date(at).toISOString()}
      title={
        r.updated_at ? `Recorded ${stamp(r.created_at)}, updated ${stamp(r.updated_at)}` : stamp(r.created_at)
      }
      className={cn("text-[12px] whitespace-nowrap text-ink-4", className)}
    >
      {r.updated_at ? `Updated ${ago(r.updated_at)}` : ago(r.created_at)}
    </time>
  );
}

/**
 * What the agent wrote: each part under its name, or beside it where there
 * is room (`wide`). With `remember`, the insights go into the project's
 * memory with a click.
 */
export function RetrospectiveParts({
  r,
  wide,
  remember,
  className,
  findKey,
}: {
  r: G.Retrospective;
  wide?: boolean;
  /** The project to remember insights in, for people who may edit its memory. */
  remember?: G.Project;
  className?: string;
  /** In a transcript: the notice's key, which finding in it knows the parts by. */
  findKey?: string;
}) {
  const [noting, setNoting] = useState(false);
  return (
    <dl
      className={cn(
        "grid gap-x-5 gap-y-2.5 text-[13.5px] leading-relaxed",
        wide && "sm:grid-cols-[5.75rem_1fr]",
        className,
      )}
    >
      {parts(r).map((p) => (
        // Beside its name where there is room, under it on phones.
        <div key={p.key} className={wide ? "block sm:contents" : "block"}>
          <dt className={cn("text-[12px] font-medium text-ink-3", wide && "sm:pt-[3px]")}>{p.label}</dt>
          <dd className={cn("mt-0.5 min-w-0 text-ink-2 [overflow-wrap:anywhere]", wide && "sm:mt-0")}>
            <div data-find={findKey && `retro:${findKey}:${p.key}`}>
              <Markdown text={p.text} className="prose-note" />
            </div>
            {p.key === "insights" && remember && (
              <button
                type="button"
                onClick={() => setNoting(true)}
                className="mt-1 -ml-2 inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
              >
                <BookMarked className="size-3.5" />
                Remember for later agents
              </button>
            )}
          </dd>
        </div>
      ))}
      {remember && noting && (
        <RememberDialog
          project={remember}
          open={noting}
          onClose={() => setNoting(false)}
          initial={r.insights ?? ""}
          from={`From the retrospective of #${r.session.number}`}
        />
      )}
    </dl>
  );
}

// The agent's plan on the session page (routes/session.tsx): beside the transcript, its
// steps as the agent last wrote them, the one it is at marked and those it finished checked off as
// it does, with how far it got; and above the message box, a ring that fills as it goes.

import { type CSSProperties, useEffect, useRef } from "react";
import { cn } from "@/lib/cn";
import type { Plan, PlanStep } from "@/lib/plan";

/**
 * How far the agent got in its plan, as a ring that fills, with the count beside it: "2/6". The
 * ring fills smoothly as steps get done.
 */
export function PlanRing({ done, total, className }: { done: number; total: number; className?: string }) {
  const r = 5.25;
  const length = 2 * Math.PI * r;
  const share = total ? done / total : 0;
  return (
    <span
      className={cn("inline-flex items-center gap-1.5 text-[11.5px] text-ink-3 tabular-nums", className)}
      title={`${done} of ${total} steps of its plan done`}
    >
      <svg viewBox="0 0 14 14" className="size-3.5 -rotate-90" aria-hidden="true">
        <circle cx="7" cy="7" r={r} fill="none" stroke="var(--c-line-strong)" strokeWidth="1.75" />
        <circle
          cx="7"
          cy="7"
          r={r}
          fill="none"
          stroke="var(--c-accent)"
          strokeWidth="1.75"
          strokeLinecap="round"
          strokeDasharray={length}
          strokeDashoffset={length * (1 - share)}
          className="transition-[stroke-dashoffset] duration-700 ease-[var(--ease-out)] motion-reduce:transition-none"
        />
      </svg>
      <span>
        {done}/{total}
      </span>
    </span>
  );
}

/** A step's mark: checked off, the one at hand (which pulses while the agent is at it), or still to come. */
function Mark({ status, fresh, live }: { status: PlanStep["status"]; fresh: boolean; live: boolean }) {
  if (status === "completed")
    return (
      <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-accent text-accent-ink">
        <svg viewBox="0 0 12 12" className="size-2.5" aria-hidden="true">
          <path
            d="M2.5 6.2 5 8.5l4.5-5"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            pathLength={1}
            strokeDasharray={1}
            className={cn(fresh && "animate-draw")}
          />
        </svg>
      </span>
    );
  if (status === "in_progress")
    return (
      <span className="plan-doing relative flex size-4 shrink-0 items-center justify-center rounded-full border-[1.5px] border-accent">
        <span className={cn("size-1.5 rounded-full bg-accent", live && "animate-pulse-soft")} />
      </span>
    );
  return <span className="block size-4 shrink-0 rounded-full border-[1.5px] border-line-strong" />;
}

/**
 * The plan beside the transcript: a bar of how far the agent got, then its steps, the one at hand
 * in the words the agent says while it does it. A step it just finished draws its check.
 */
export function PlanList({
  plan,
  live = true,
}: {
  plan: Plan;
  /** The agent works on it now; else it shows as it was left, its step at hand still. */
  live?: boolean;
}) {
  // The steps this list has seen finished: those that finish while it shows draw their checks.
  const seen = useRef<Set<string> | undefined>(undefined);
  const finished = plan.steps.filter((s) => s.status === "completed").map((s) => s.text);
  const fresh = new Set(seen.current ? finished.filter((t) => !seen.current?.has(t)) : []);
  useEffect(() => {
    seen.current = new Set(finished);
  });
  const total = plan.steps.length;
  return (
    <div>
      <div className="mb-3 flex items-center gap-3">
        <div className="h-1 flex-1 overflow-hidden rounded-full bg-line">
          <div
            className="h-full rounded-full bg-accent transition-[width] duration-700 ease-[var(--ease-out)] motion-reduce:transition-none"
            style={{ width: `${(plan.done / total) * 100}%` }}
          />
        </div>
        <span className="text-[11.5px] text-ink-4 tabular-nums">
          {plan.done} of {total}
        </span>
      </div>
      <ol className="flex flex-col gap-1.5">
        {plan.steps.map((s, i) => (
          <li
            key={`${i}-${s.text}`}
            className="flex items-start gap-2.5 text-[12.5px] leading-[1.45]"
            style={{ "--i": i } as CSSProperties}
          >
            <span className="mt-px flex">
              <Mark status={s.status} fresh={fresh.has(s.text)} live={live} />
            </span>
            <span
              className={cn(
                "min-w-0",
                s.status === "completed" && "text-ink-4",
                s.status === "in_progress" && (live ? "font-medium text-ink" : "text-ink-2"),
                s.status === "pending" && "text-ink-3",
              )}
            >
              {s.status === "in_progress" ? (s.doing ?? s.text) : s.text}
            </span>
          </li>
        ))}
      </ol>
    </div>
  );
}

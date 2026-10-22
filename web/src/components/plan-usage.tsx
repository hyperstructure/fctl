// Claude plan usage as meters: how much of each limit sessions last saw
// used, and when it starts over. Settings lists every limit of each
// credential; the home page shows the default credential's in one line.

import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import {
  inSentence,
  limitLabel,
  percent,
  pressing,
  reportedAt,
  resetTime,
  type Standing,
  share,
  shortLabel,
  standing,
} from "@/lib/usage";

const TONE: Record<Standing, { bar: string; text: string }> = {
  fine: { bar: "bg-ink-3", text: "text-ink-3" },
  close: { bar: "bg-amber", text: "text-amber" },
  critical: { bar: "bg-danger", text: "text-danger" },
  reached: { bar: "bg-danger", text: "text-danger" },
  reset: { bar: "bg-ink-4", text: "text-ink-4" },
};

/** The time, again every minute: a limit that resets meanwhile shows it. */
function useNow(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

/** A bar of the share used. The text beside it says the same for screen readers. */
function Meter({ limit, now, className }: { limit: G.PlanLimit; now: number; className?: string }) {
  const s = standing(limit, now);
  const filled = share(limit, now);
  return (
    <div aria-hidden className={cn("h-1 overflow-hidden rounded-full bg-line", className)}>
      <div
        className={cn(
          "h-full rounded-full transition-[width] duration-300 motion-reduce:transition-none",
          TONE[s].bar,
        )}
        style={{ width: `${filled * 100}%` }}
      />
    </div>
  );
}

/** What a limit's numbers say: "42% used · resets 3:40 PM". */
function detail(l: G.PlanLimit, now: number): string {
  const s = standing(l, now);
  if (s === "reset") return `Started over ${ago(l.resets_at)}`;
  const resets = l.resets_at !== undefined ? `resets ${resetTime(l.resets_at, now)}` : "";
  const used = s === "reached" ? "Used up" : percent(l) && `${percent(l)} used`;
  return [used, resets].filter(Boolean).join(" · ");
}

/** Every limit of a credential, for Settings. */
export function PlanUsageList({ usage }: { usage: G.PlanLimit[] }) {
  const now = useNow();
  return (
    <div>
      <ul className="grid grid-cols-[minmax(0,auto)_minmax(64px,1fr)] items-center gap-x-4 gap-y-2 sm:grid-cols-[minmax(0,auto)_minmax(64px,1fr)_auto]">
        {usage.map((l) => {
          const s = standing(l, now);
          return (
            <li key={l.window} className="contents">
              <span className="truncate text-[12.5px] text-ink-2">{limitLabel(l.window)}</span>
              <Meter limit={l} now={now} />
              <span
                className={cn(
                  "col-span-2 -mt-1 font-mono text-[11.5px] tabular-nums sm:col-span-1 sm:mt-0",
                  s === "fine" ? "text-ink-3" : TONE[s].text,
                )}
              >
                {detail(l, now)}
              </span>
            </li>
          );
        })}
      </ul>
      <p className="mt-2 text-[11.5px] text-ink-4">Reported by a session {ago(reportedAt(usage))}</p>
    </div>
  );
}

/** The default credential's limits in one line, under the home page's task box. Phones get
 * shorter meters and gaps, so two limits fit on the line from 390 px. */
export function PlanUsageStrip({ usage, className }: { usage: G.PlanLimit[]; className?: string }) {
  const now = useNow();
  const urgent = pressing(usage, now);
  const s = urgent && standing(urgent, now);
  const when = urgent?.resets_at !== undefined ? resetTime(urgent.resets_at, now) : undefined;
  return (
    <div className={cn("flex flex-col gap-1.5 px-1 text-[12px]", className)}>
      <Link
        to="/settings"
        search={{ linked: undefined, error: undefined }}
        title={`As a session reported it ${ago(reportedAt(usage))}`}
        className="flex flex-wrap items-center gap-x-4 gap-y-1.5 self-start rounded-md text-ink-3 transition-colors hover:text-ink-2 sm:gap-x-5"
      >
        <span className="text-ink-4">Claude plan</span>
        {usage.map((l) => {
          const st = standing(l, now);
          return (
            <span key={l.window} className="flex items-center gap-1.5 sm:gap-2">
              <span>{shortLabel(l.window)}</span>
              <Meter limit={l} now={now} className="w-10 sm:w-14" />
              <span className={cn("font-mono text-[11px] tabular-nums", st !== "fine" && TONE[st].text)}>
                {st === "reset" ? "reset" : st === "reached" ? "used up" : percent(l) || "–"}
              </span>
            </span>
          );
        })}
      </Link>
      {urgent && s === "reached" && (
        <p className={TONE[s].text}>
          Your {inSentence(limitLabel(urgent.window))} is used up. Sessions you start now pause until it
          resets
          {when ? ` ${when}` : ""}.
        </p>
      )}
      {urgent && (s === "close" || s === "critical") && (
        <p className={TONE[s].text}>
          {percent(urgent)} of your {inSentence(limitLabel(urgent.window))} is used
          {when ? `. It resets ${when}` : ""}.
        </p>
      )}
    </div>
  );
}

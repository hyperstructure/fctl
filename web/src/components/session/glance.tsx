// The session at a glance, on the new session page's cover (routes/session-next.tsx): how many
// turns it took, the agent's time, its steps and what it changed, in figures that roll as they
// move; its pull requests and how each stands; and its turns as a ribbon, each as wide as the
// work in it, colored by how it went, that goes to the turn it is.

import { type CSSProperties, useEffect, useState } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { duration } from "@/lib/format";
import { prRef, prStateText, prTitle, pullRequests } from "@/lib/pulls";
import { type Turn, type TurnFacts, turnTitle, turnTook } from "@/lib/turns";
import { agentTime, turnsTaken } from "@/lib/work";
import { Rolling } from "./diff-live";
import { PullRequestChecksIcon, PullRequestStateIcon } from "./pull-request";

/** The time, again each `ms` while `on`. */
function useNow(on: boolean, ms: number): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!on) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [on, ms]);
  return now;
}

/** A figure and what it counts, side by side with the others. */
function Figure({ value, label, children }: { value?: number; label: string; children?: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5 border-line px-4 first:pl-0 not-first:border-l md:px-6">
      <span className="font-display text-[17px] leading-6 font-medium tracking-[-0.01em] text-ink tabular-nums md:text-[19px]">
        {children ?? <Rolling value={value ?? 0} />}
      </span>
      <span className="text-[11.5px] text-ink-4">{label}</span>
    </div>
  );
}

/** How a turn went, as its stretch of the ribbon shows it. */
function tone(turn: Turn, live: boolean): string {
  const end = turn.end;
  if (!end) return live ? "ribbon-live bg-accent" : "bg-line-strong";
  if (end.kind === "result") return end.ok ? "bg-ink-4/70 hover:bg-ink-3" : "bg-amber/70 hover:bg-amber";
  if (end.kind === "failed") return "bg-danger/70 hover:bg-danger";
  if (end.kind === "cut") return "bg-amber/70 hover:bg-amber";
  return "bg-line-strong hover:bg-ink-4";
}

export function Glance({
  session,
  turns,
  facts,
  live,
  hasMoreBefore,
  onGo,
}: {
  session: G.Session;
  turns: Turn[];
  facts: (turn: Turn) => TurnFacts;
  /** The agent works on the last turn. */
  live: boolean;
  hasMoreBefore: boolean;
  onGo: (i: number) => void;
}) {
  const st = session.stats;
  const now = useNow(st.turn_started_at != null, 1000);
  const prs = pullRequests(session);
  const took = turns.map((t) => turnTook(t, facts(t), now));
  // Each turn as wide as the root of its time: an hour's turn shows beside a minute's.
  const weights = took.map((ms) => Math.sqrt(Math.max(ms, 20_000) / 1000));
  return (
    <div className="mt-6 flex flex-col gap-4">
      <div className="flex flex-wrap gap-y-3">
        <Figure value={turnsTaken(st)} label={turnsTaken(st) === 1 ? "turn" : "turns"} />
        <Figure label="agent time">{duration(agentTime(st, now))}</Figure>
        <Figure value={st.tool_calls} label={st.tool_calls === 1 ? "step" : "steps"} />
        <Figure label={st.files_changed === 1 ? "file changed" : "files changed"}>
          <span className="inline-flex items-baseline gap-2">
            <Rolling value={st.files_changed} />
            <span className="font-mono text-[11.5px] font-normal tracking-normal">
              <span className={cn(st.additions > 0 ? "text-added" : "text-ink-4")}>+{st.additions}</span>{" "}
              <span className={cn(st.deletions > 0 ? "text-danger" : "text-ink-4")}>−{st.deletions}</span>
            </span>
          </span>
        </Figure>
      </div>
      {turns.length > 0 && !hasMoreBefore && (
        <nav className="flex h-1.5 items-stretch gap-1" aria-label="Turns">
          {turns.map((turn, i) => (
            <button
              key={turn.key}
              type="button"
              onClick={() => onGo(i)}
              title={`${turnTitle(turn, i === 0, hasMoreBefore)} · ${duration(took[i] ?? 0)}`}
              aria-label={`${turnTitle(turn, i === 0, hasMoreBefore)}, ${duration(took[i] ?? 0)}`}
              className={cn(
                "ribbon-turn min-w-1.5 rounded-full transition-[background-color,transform] duration-150 hover:scale-y-150",
                tone(turn, live && i === turns.length - 1),
              )}
              style={{ flex: `${weights[i]} 1 0` } as CSSProperties}
            />
          ))}
        </nav>
      )}
      {prs.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {prs.map((pr) => (
            <a
              key={pr.url}
              href={pr.url}
              target="_blank"
              rel="noreferrer"
              title={prTitle(pr)}
              className="inline-flex h-7 min-w-0 items-center gap-1.5 rounded-full border border-line bg-surface/60 pr-3 pl-2 text-[12px] text-ink-2 transition-colors hover:border-line-strong hover:text-ink"
            >
              <PullRequestStateIcon pr={pr} className="size-3.5" />
              <span className="font-mono text-[11.5px]">{prRef(pr)}</span>
              <span className="truncate text-ink-3">{prStateText(pr)}</span>
              <PullRequestChecksIcon pr={pr} className="size-3" />
            </a>
          ))}
        </div>
      )}
    </div>
  );
}

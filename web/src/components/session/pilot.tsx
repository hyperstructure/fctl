// An autopilot's own session, in its details (the rail beside the transcript, the sheet on a
// phone): where the autopilot stands and what people do with it from here, what it asks them, its
// plan and the sessions it started, as its Autopilot tab shows them, so that following it in its
// session doesn't take going back and forth to the tab.

import { Link } from "@tanstack/react-router";
import { ArrowUpRight, CircleAlert, KeyRound, MessageCircleQuestion } from "lucide-react";
import { useMemo } from "react";
import {
  AutopilotTile,
  Clamped,
  CrewList,
  crewCount,
  MoveButtons,
  TONES,
  useAutopilot,
  useNow,
} from "@/components/project/autopilot";
import { Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { atWork, openAsks, phase, planDone } from "@/lib/autopilot";
import { cn } from "@/lib/cn";
import { agoShortWords, stamp } from "@/lib/format";
import { RailSection } from "./rail";

/** The asks it shows; the rest are a click away, on its tab. */
const ASKS_SHOWN = 3;

/** Its autopilot, its plan and its sessions, for an autopilot's own session (`role: "pilot"`). */
export function PilotDetails({ session }: { session: G.Session }) {
  const q = useAutopilot(session.autopilot?.project_slug ?? session.project_slug);
  const a = q.data;
  if (q.isPending)
    return (
      <div className="px-5 pt-4 pb-2" aria-hidden="true">
        <Skeleton className="h-[136px] w-full rounded-xl" />
      </div>
    );
  // Its tab says why it didn't load; the rest of the details go on without it.
  if (!a) return null;
  return (
    <>
      <Standing a={a} session={session} />
      {a.can.read && (
        <>
          <PlanSection a={a} />
          <RailSection title="Its sessions" meta={crewCount(a)}>
            <CrewList a={a} className="text-[12.5px]" />
          </RailSection>
        </>
      )}
    </>
  );
}

/**
 * Where it stands, as its tab's header says it, in a card the width of the rail: its mark and name
 * (a link to its tab), the word and sentence of its phase, what it asks, the moves of those who
 * steer it, and a problem across its foot.
 */
function Standing({ a, session }: { a: G.Autopilot; session: G.Session }) {
  const now = useNow(15_000);
  const p = phase(a, now, session.id);
  const tone = TONES[p.tone];
  const live = atWork(a);
  const asks = a.can.read ? openAsks(a) : [];
  // Wakes go to its current session: an earlier one says where it works now.
  const earlier = a.state !== "off" && a.session && a.session.id !== session.id ? a.session : undefined;
  const tab = { to: "/p/$slug", params: { slug: a.project_slug }, search: { tab: "autopilot" } } as const;
  return (
    <section aria-label="Its autopilot" className="px-5 pt-4 pb-2">
      <div className={cn("relative rounded-xl border bg-surface", tone.border)}>
        <div
          aria-hidden="true"
          className={cn("pointer-events-none absolute inset-0 rounded-[11px]", tone.glow)}
        />
        <div className="relative px-3.5 pt-3 pb-3.5">
          <div className="flex items-center gap-2.5">
            <AutopilotTile live={live} size={32} className="rounded-lg" />
            <div className="min-w-0 flex-1">
              <Link
                {...tab}
                className="block truncate text-[13.5px] leading-5 font-semibold text-ink underline-offset-2 hover:underline"
              >
                {a.scope === "workspace"
                  ? `${session.workspace?.name ?? "The workspace"}'s autopilot`
                  : "Autopilot"}
              </Link>
              <p aria-live="polite" className="flex items-center gap-1.5 text-[12.5px] leading-5">
                <span className="relative inline-flex size-1.5 shrink-0">
                  {live && (
                    <span
                      className={cn(
                        "absolute inset-0 animate-ping rounded-full opacity-60 [animation-duration:2s] motion-reduce:hidden",
                        tone.dot,
                      )}
                    />
                  )}
                  <span className={cn("relative inline-flex size-1.5 rounded-full", tone.dot)} />
                </span>
                <span className={cn("font-medium", tone.word)}>{p.word}</span>
              </p>
            </div>
            <Link
              {...tab}
              aria-label="Open its Autopilot tab"
              title="Open its Autopilot tab: its direction, notes and log"
              className="-mr-1.5 flex size-7 shrink-0 items-center justify-center self-start rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink"
            >
              <ArrowUpRight className="size-3.5" />
            </Link>
          </div>
          <p className="mt-2.5 text-[12.5px] leading-relaxed text-ink-3">{p.line}</p>
          {earlier && (
            <p className="mt-1.5 text-[12px] leading-relaxed text-ink-4">
              This is an earlier session of its own: fctl wakes it in{" "}
              <Link
                to="/s/$id"
                params={{ id: earlier.id }}
                className="font-mono text-ink-3 underline underline-offset-2 hover:text-ink"
              >
                #{earlier.number}
              </Link>{" "}
              now.
            </p>
          )}
          {asks.length > 0 && (
            <ul aria-label="What it asks you" className="mt-3 flex flex-col gap-1.5">
              {asks.slice(0, ASKS_SHOWN).map((ask) => (
                <li key={ask.id}>
                  <Link
                    {...tab}
                    title="Answer on its Autopilot tab"
                    className="flex items-start gap-2 rounded-lg border border-blue/25 bg-blue-soft px-2.5 py-1.5 text-[12.5px] leading-snug text-blue transition-colors hover:border-blue/45"
                  >
                    {ask.secret ? (
                      <KeyRound className="mt-[2px] size-3.5 shrink-0" aria-hidden="true" />
                    ) : (
                      <MessageCircleQuestion className="mt-[2px] size-3.5 shrink-0" aria-hidden="true" />
                    )}
                    <span className="line-clamp-2 min-w-0 [overflow-wrap:anywhere]">
                      {ask.secret ? `Needs the secret ${ask.secret.name}` : ask.text}
                    </span>
                  </Link>
                </li>
              ))}
              {asks.length > ASKS_SHOWN && (
                <li>
                  <Link {...tab} className="text-[12px] text-blue underline-offset-2 hover:underline">
                    {asks.length - ASKS_SHOWN} more on its tab
                  </Link>
                </li>
              )}
            </ul>
          )}
          {a.can.steer && a.state !== "off" && (
            <div className="mt-3 flex flex-wrap items-center gap-1.5">
              <MoveButtons a={a} />
            </div>
          )}
        </div>
        {a.problem && (
          <div className="relative flex items-start gap-2 rounded-b-[11px] border-t border-amber/25 bg-amber-soft px-3.5 py-2 text-[12px] leading-snug text-ink-2">
            <CircleAlert className="mt-px size-3.5 shrink-0 text-amber" aria-hidden="true" />
            {a.problem}
          </div>
        )}
      </div>
    </section>
  );
}

/** Its plan: how far its checklist is, as the agent's own plan shows it, then the Markdown, clamped. */
function PlanSection({ a }: { a: G.Autopilot }) {
  const done = useMemo(() => planDone(a.plan), [a.plan]);
  return (
    <RailSection
      title="Its plan"
      meta={a.plan_at && <span title={stamp(a.plan_at)}>Updated {agoShortWords(a.plan_at)}</span>}
    >
      {done && (
        <div className="mb-3 flex items-center gap-3">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-line">
            <div
              className="h-full rounded-full bg-accent transition-[width] duration-700 ease-[var(--ease-out)] motion-reduce:transition-none"
              style={{ width: `${(done.checked / done.of) * 100}%` }}
            />
          </div>
          <span className="text-[11.5px] text-ink-4 tabular-nums">
            {done.checked} of {done.of}
          </span>
        </div>
      )}
      {a.plan ? (
        <Clamped text={a.plan} lines={10} className="text-[12.5px]" />
      ) : (
        <p className="text-[12.5px] leading-relaxed text-ink-4">
          It writes one as it starts: what it will do, in order. It keeps it current as the work goes.
        </p>
      )}
    </RailSection>
  );
}

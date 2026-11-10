// How autopilots (experimental) show in sessions: the message fctl wakes an autopilot with, as a
// card of what woke it; what the autopilot did in its own session, a session it started showing
// there as that session, live; the task and messages it gives the sessions it starts, as the
// autopilot's rather than its owner's; and the link from any of its sessions back to it.

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ArrowUpRight,
  BookOpenCheck,
  ChevronDown,
  CircleAlert,
  FolderPlus,
  GitFork,
  KeyRound,
  ListChecks,
  MessageCircleQuestion,
  NotebookPen,
  Repeat,
  Send,
  Settings2,
} from "lucide-react";
import { type ReactNode, useId, useMemo, useState } from "react";
import { ProjectMark } from "@/components/project/project-icon";
import { AutopilotIcon, StatusDot, statusLabel } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { splitBriefing, splitCrewNote, wakeText } from "@/lib/autopilot";
import { cn } from "@/lib/cn";
import { stamp, when } from "@/lib/format";
import { prState, pullRequests } from "@/lib/pulls";
import { keys } from "@/lib/queries";
import type { Item } from "@/lib/transcript";
import { Milestone, Thread } from "./lines";

type UserItem = Extract<Item, { kind: "user" }>;

/** The autopilot's mark in a round tile, as a message's author shows it. */
function Mark({ size = 20, live }: { size?: number; live?: boolean }) {
  return (
    <span
      aria-hidden="true"
      style={{ width: size, height: size }}
      className="flex shrink-0 items-center justify-center rounded-full bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
    >
      <AutopilotIcon live={live} className={size > 22 ? "size-4" : "size-3"} />
    </span>
  );
}

/**
 * The message fctl woke an autopilot with, as the card that opens its turn: what woke it, the
 * notes, answers and sessions that did, and the message itself a click away. Its own session's
 * first message holds its briefing too, with the direction it set out on.
 */
export function WakeCard({ item }: { item: UserItem }) {
  const wake = item.wake;
  const { briefing, direction, news } = useMemo(() => splitBriefing(item.text), [item.text]);
  const [open, setOpen] = useState(false);
  const id = useId();
  const crew = wake?.crew ?? [];
  // A start or a resume alone says so in a sentence of its own.
  const lone = wake?.reasons.length === 1 ? wake.reasons[0] : undefined;
  return (
    <article className="ask-card animate-rise rounded-[18px] rounded-bl-[7px] border border-line bg-surface px-4 pt-3 pb-3.5 md:px-5 md:pt-3.5 md:pb-4">
      <header className="flex min-w-0 items-center gap-2 text-[12.5px] leading-5">
        <Mark />
        <span className="truncate font-medium text-ink">fctl woke the autopilot</span>
        <span className="min-w-0 truncate text-ink-4" title={stamp(item.ts)}>
          {when(item.ts)}
        </span>
      </header>
      <p className="mt-2 text-[15px] leading-[1.6] text-ink md:text-[15.5px]" data-find={`msg:${item.key}`}>
        {briefing
          ? "It was put on autopilot: it reads its brief, then plans the work."
          : lone === "resumed"
            ? "People resumed it: it reads what's new, then goes on."
            : lone === "started"
              ? "It was put on autopilot again: it reads what's new, then goes on."
              : lone === "woken"
                ? "Someone woke it: it reads what's new, then goes on."
                : `For ${wake ? wakeText(wake) : "its check-in"}.`}
      </p>
      {(crew.length > 0 || (wake?.notes ?? 0) > 0 || (wake?.answers ?? 0) > 0) && (
        <ul className="mt-2.5 flex flex-wrap gap-1.5" aria-label="What woke it">
          {(wake?.notes ?? 0) > 0 && (
            <Chip icon={<NotebookPen />}>
              {wake?.notes === 1 ? "A new note" : `${wake?.notes} new notes`}
            </Chip>
          )}
          {(wake?.answers ?? 0) > 0 && (
            <Chip icon={<MessageCircleQuestion />}>
              {wake?.answers === 1 ? "An answer" : `${wake?.answers} answers`}
            </Chip>
          )}
          {crew.map((s) => (
            <li key={s.id}>
              <Link
                to="/s/$id"
                params={{ id: s.id }}
                className="inline-flex max-w-[20rem] items-center gap-1.5 rounded-full border border-line px-2.5 py-0.5 text-[12px] text-ink-3 transition-colors hover:border-line-strong hover:text-ink"
              >
                <GitFork className="size-3 shrink-0" aria-hidden="true" />
                <span className="font-mono text-ink-4">#{s.number}</span>
                <span className="truncate">{s.title}</span>
              </Link>
            </li>
          ))}
        </ul>
      )}
      {direction && (
        <blockquote className="mt-3 border-l-2 border-line-strong pl-3 text-[13.5px] leading-relaxed whitespace-pre-wrap text-ink-2 [overflow-wrap:anywhere]">
          <span className="line-clamp-4">{direction}</span>
        </blockquote>
      )}
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        aria-controls={id}
        className="mt-2.5 inline-flex items-center gap-1 text-[12.5px] text-ink-3 transition-colors hover:text-ink"
      >
        <ChevronDown className={cn("size-3.5 transition-transform duration-200", open && "rotate-180")} />
        {open
          ? "Hide what fctl told it"
          : briefing
            ? "Its briefing, and what fctl told it"
            : "What fctl told it"}
      </button>
      {open && (
        <div
          id={id}
          className="mt-2 max-h-[28rem] overflow-y-auto rounded-xl bg-sunken/60 px-3.5 py-3 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink-2 [overflow-wrap:anywhere]"
        >
          {briefing ? `${briefing}\n\n---\n\n${news}` : item.text}
        </div>
      )}
    </article>
  );
}

function Chip({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <li className="inline-flex items-center gap-1.5 rounded-full border border-line px-2.5 py-0.5 text-[12px] text-ink-3 [&_svg]:size-3">
      {icon}
      {children}
    </li>
  );
}

/** A task or message an autopilot gave one of its sessions: its words, then fctl's note of who started it. */
export function AutopilotAuthorLine({
  author,
  owner,
  ts,
  pending,
}: {
  author: G.AutopilotAuthor;
  owner?: G.UserRef;
  ts: number;
  pending?: boolean;
}) {
  return (
    <header className="flex min-w-0 items-center gap-2 text-[12.5px] leading-5">
      <Mark />
      <Link
        to="/p/$slug"
        params={{ slug: author.project_slug }}
        search={{ tab: "autopilot" }}
        className="truncate font-medium text-ink hover:underline"
      >
        {author.project_slug}'s autopilot
      </Link>
      {owner && <span className="hidden truncate text-ink-4 sm:inline">for {owner.name}</span>}
      <span className="min-w-0 truncate text-ink-4" title={pending ? undefined : stamp(ts)}>
        {pending ? "Sending…" : when(ts)}
      </span>
    </header>
  );
}

/** What an autopilot asked one of its sessions to do, without fctl's note, which follows apart. */
export function crewTask(text: string): { task: string; note?: string } {
  return splitCrewNote(text);
}

const ACTION_ICONS: Partial<Record<G.AutopilotEntryKind, ReactNode>> = {
  session: <GitFork />,
  steered: <Send />,
  settings: <Settings2 />,
  secret: <KeyRound />,
  repository: <BookOpenCheck />,
  schedule: <Repeat />,
  project: <FolderPlus />,
  plan: <ListChecks />,
  problem: <CircleAlert />,
};

/**
 * What an autopilot did, in its own session's transcript: a session it started or steered, as
 * that session, live, a click from it; a change it made, as a line.
 */
export function AutopilotAction({ notice }: { notice: G.Notice }) {
  const a = notice.autopilot;
  if (!a) return null;
  const icon = ACTION_ICONS[a.kind] ?? <AutopilotIcon />;
  if (!a.session && a.kind === "project" && a.project_slug)
    return (
      <div className="animate-rise py-1.5">
        <Milestone icon={icon}>{notice.message}</Milestone>
        <Thread>
          <CreatedProject slug={a.project_slug} />
        </Thread>
      </div>
    );
  if (!a.session)
    return (
      <Milestone icon={icon} tone="info">
        {notice.message}
      </Milestone>
    );
  const verb =
    a.kind === "session"
      ? "Started a session"
      : (notice.message.split(` ${a.project_slug ?? ""}#`)[0] ?? notice.message);
  return (
    <div className="animate-rise py-1.5">
      <Milestone icon={icon}>
        {a.schedule && a.project_slug ? (
          <ScheduleNamed
            text={verb}
            schedule={a.schedule}
            slug={a.project_slug}
            className="text-ink-3 decoration-line-strong hover:text-ink hover:decoration-ink-3"
          />
        ) : (
          verb
        )}
      </Milestone>
      <Thread>
        <SpawnedSession session={a.session} projectSlug={a.project_slug} />
      </Thread>
    </div>
  );
}

/**
 * A line that names the schedule an autopilot ran ("Ran the schedule “Nightly deps” now"), its
 * name a link to the schedule, whose runs its Schedules tab lists; the line as it is once the
 * schedule is deleted, or when it doesn't name it.
 */
export function ScheduleNamed({
  text,
  schedule,
  slug,
  className,
}: {
  text: string;
  schedule: G.SessionSchedule;
  slug: string;
  className?: string;
}) {
  const quoted = schedule.name ? `“${schedule.name}”` : undefined;
  const at = quoted ? text.indexOf(quoted) : -1;
  if (!quoted || at < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, at)}
      <Link
        to="/p/$slug"
        params={{ slug }}
        search={{ tab: "schedules", schedule: schedule.id }}
        title="Open the schedule and its runs"
        className={cn("underline underline-offset-2 transition-colors", className)}
      >
        {quoted}
      </Link>
      {text.slice(at + quoted.length)}
    </>
  );
}

/** A project an autopilot created, as a card a click from it. */
function CreatedProject({ slug }: { slug: string }) {
  const p = useQuery({
    queryKey: keys.project(slug),
    queryFn: () => api.project(slug),
    staleTime: 30_000,
  }).data;
  return (
    <Link
      to="/p/$slug"
      params={{ slug }}
      className="group flex max-w-xl items-center gap-3 rounded-xl border border-line bg-surface px-3.5 py-2.5 transition-[border-color,background-color] duration-150 hover:border-line-strong hover:bg-hover/30"
    >
      {p ? (
        <ProjectMark project={p} tile={28} />
      ) : (
        <span className="size-7 shrink-0 rounded-[8px] bg-raised" aria-hidden="true" />
      )}
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-[14px] text-ink">{p?.name ?? slug}</span>
          <ArrowUpRight
            className="size-3.5 shrink-0 self-center text-ink-4 transition-transform duration-150 group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
            aria-hidden="true"
          />
        </span>
        <span className="mt-0.5 block truncate font-mono text-[12px] text-ink-3">{slug}</span>
      </span>
    </Link>
  );
}

const ACTIVE = new Set<G.SessionStatus>(["queued", "provisioning", "preparing", "running", "stopping"]);

/** A session an autopilot started, as a card that keeps its status, question and pull requests live. */
export function SpawnedSession({ session, projectSlug }: { session: G.SessionRef; projectSlug?: string }) {
  const live = useQuery({
    queryKey: keys.session(session.id),
    queryFn: () => api.session(session.id),
    refetchInterval: (q) => (q.state.data && ACTIVE.has(q.state.data.status) ? 5_000 : 30_000),
    staleTime: 4_000,
  });
  const s = live.data;
  const prs = s ? pullRequests(s) : [];
  return (
    <Link
      to="/s/$id"
      params={{ id: session.id }}
      className="group flex max-w-xl items-start gap-3 rounded-xl border border-line bg-surface px-3.5 py-2.5 transition-[border-color,background-color] duration-150 hover:border-line-strong hover:bg-hover/30"
    >
      {s ? (
        <StatusDot status={s.status} className="mt-[7px]" />
      ) : (
        <span className="mt-[7px] size-2 rounded-full bg-ink-4" />
      )}
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span className="min-w-0 flex-1 truncate text-[14px] text-ink">{s?.title ?? session.title}</span>
          <ArrowUpRight
            className="size-3.5 shrink-0 self-center text-ink-4 transition-transform duration-150 group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
            aria-hidden="true"
          />
        </span>
        <span className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 text-[12px] text-ink-4">
          <span className="font-mono text-ink-3">
            {projectSlug}#{session.number}
          </span>
          {s && <span>{statusLabel(s.status)}</span>}
          {prs.map((pr) => (
            <span key={pr.url} className="font-mono">
              #{pr.number} {prState(pr)}
            </span>
          ))}
        </span>
        {s?.question && (
          <span className="mt-1 flex items-center gap-1.5 text-[12.5px] text-blue">
            <MessageCircleQuestion className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="truncate">{s.question.text}</span>
          </span>
        )}
      </span>
    </Link>
  );
}

/**
 * Where a session stands with its autopilot, for its header: its own session ("Shop's autopilot")
 * or one it started ("Started by shop's autopilot"), a link to the autopilot's tab.
 */
export function AutopilotLink({
  session,
  className,
}: {
  session: Pick<G.Session, "autopilot">;
  className?: string;
}) {
  const a = session.autopilot;
  if (!a) return null;
  return (
    <Link
      to="/p/$slug"
      params={{ slug: a.project_slug }}
      search={{ tab: "autopilot" }}
      title={
        a.role === "pilot" ? "The autopilot works in this session" : "The autopilot started this session"
      }
      className={cn(
        "inline-flex min-w-0 items-center gap-1 text-ink-3 transition-colors hover:text-ink",
        className,
      )}
    >
      <AutopilotIcon className="size-3 shrink-0" />
      <span className="truncate">
        {a.role === "pilot" ? `${a.project_slug}'s autopilot` : `Started by ${a.project_slug}'s autopilot`}
      </span>
    </Link>
  );
}

/**
 * Who started a session of an autopilot's, for its cover: the autopilot, a link to its tab, and
 * the person it acts for, on whose subscription the session runs.
 */
export function AutopilotStarter({ session }: { session: Pick<G.Session, "autopilot" | "created_by"> }) {
  const a = session.autopilot;
  if (!a) return null;
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5">
      <span className="flex size-4 shrink-0 items-center justify-center rounded-full bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line-strong)]">
        <AutopilotIcon className="size-2.5" />
      </span>
      <span className="truncate">
        {a.role === "pilot" ? "" : "Started by "}
        <Link
          to="/p/$slug"
          params={{ slug: a.project_slug }}
          search={{ tab: "autopilot" }}
          className="text-ink-2 underline-offset-2 hover:text-ink hover:underline"
        >
          {a.project_slug}'s autopilot
        </Link>
        {a.role === "pilot"
          ? `, on ${session.created_by.name}'s subscription`
          : `, for ${session.created_by.name}`}
      </span>
    </span>
  );
}

// A session in a list (the home page, a workspace's page): its status, title,
// workspace and reference, its pull requests, who started it and when it
// last moved.

import { Link } from "@tanstack/react-router";
import { ArrowRight, Repeat } from "lucide-react";
import { Fragment } from "react";
import { Avatar, GithubIcon, LinearIcon, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { agoShort, stamp } from "@/lib/format";
import { prLabel, prRef, prTitle, pullRequests } from "@/lib/pulls";
import { statusReason } from "@/lib/status";
import { ticketLabel } from "@/lib/tickets";
import { PullRequestChecksIcon, PullRequestReviewMark, PullRequestStateIcon } from "./pull-request";

export function SessionRow({ s, workspace }: { s: G.Session; workspace?: G.WorkspaceRef }) {
  return (
    <div className="group relative isolate flex items-center gap-3.5 border-b border-line p-3 transition-colors last:border-b-0 hover:bg-hover/30">
      <StatusDot status={s.status} />
      <div className="min-w-0 flex-1">
        <SessionLink s={s} />
        <div className="mt-0.5 flex min-w-0 items-center gap-2 text-[12px] text-ink-4">
          {workspace && (
            // On a phone its avatar says which one, and the reference keeps the room.
            // There the avatar keeps its width, so that a long status can't push it into the reference.
            <span className="inline-flex min-w-0 shrink items-center gap-1.5 text-ink-3 max-sm:shrink-0">
              <WorkspaceAvatar workspace={workspace} size={14} />
              <span className="max-w-[10rem] truncate max-sm:hidden">{workspace.name}</span>
              <span className="text-ink-4 max-sm:hidden">/</span>
            </span>
          )}
          <span className="shrink-0 font-mono">
            {s.project_slug}#{s.number}
          </span>
          <ScheduledMark s={s} />
          <TicketMark s={s} />
          {statusReason(s) && <span className="truncate">· {statusReason(s)}</span>}
          {s.stats.files_changed > 0 && (
            <span className="shrink-0 max-sm:hidden">
              · <span className="text-accent/80">+{s.stats.additions}</span>{" "}
              <span className="text-danger/80">−{s.stats.deletions}</span>
            </span>
          )}
          <PullRequestLinks s={s} className="max-sm:hidden" />
        </div>
      </div>
      <Avatar user={s.created_by} size={20} />
      <time
        dateTime={new Date(s.last_activity_at ?? s.created_at).toISOString()}
        title={stamp(s.last_activity_at ?? s.created_at)}
        className="w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
      >
        {agoShort(s.last_activity_at ?? s.created_at)}
      </time>
      {/* It appears on hover; on a phone its room goes to the title. */}
      <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100 max-sm:hidden" />
    </div>
  );
}

/**
 * The session's title, as the link that opens it from anywhere on its row: a link can't hold the
 * pull requests' links, so its ::after covers the row (`relative isolate`) and they lie over that.
 * Its focus ring goes around the row.
 */
export function SessionLink({ s }: { s: G.Session }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      className="block truncate text-[14.5px] text-ink outline-none after:absolute after:inset-0 after:z-[1] after:rounded focus-visible:after:outline-2 focus-visible:after:outline-offset-2 focus-visible:after:outline-accent"
    >
      {s.title}
    </Link>
  );
}

/** That a schedule started the session: its symbol, with the schedule's name on hover. */
export function ScheduledMark({ s }: { s: Pick<G.Session, "schedule"> }) {
  if (!s.schedule) return null;
  const name = s.schedule.name;
  return (
    <span
      title={name ? `Started by the schedule ${name}` : "Started by a schedule, since deleted"}
      className="inline-flex shrink-0 items-center text-ink-4"
    >
      <Repeat className="size-3" aria-hidden="true" />
      <span className="sr-only">{name ? `From the schedule ${name}` : "From a schedule"}</span>
    </span>
  );
}

/**
 * The schedule a session started from, as a link to it in its project's Schedules tab; its name
 * alone once it is deleted.
 */
export function ScheduleLink({
  session,
  className,
}: {
  session: Pick<G.Session, "schedule" | "project_slug">;
  className?: string;
}) {
  const schedule = session.schedule;
  if (!schedule) return null;
  const content = (
    <>
      <Repeat className="size-3 shrink-0" aria-hidden="true" />
      <span className="truncate">{schedule.name ?? "A deleted schedule"}</span>
    </>
  );
  if (!schedule.name)
    return (
      <span className={cn("inline-flex min-w-0 items-center gap-1 text-ink-4", className)}>{content}</span>
    );
  return (
    <Link
      to="/p/$slug"
      params={{ slug: session.project_slug }}
      search={{ tab: "schedules", schedule: schedule.id }}
      title={`Started by the schedule ${schedule.name}`}
      className={cn(
        "inline-flex min-w-0 items-center gap-1 text-ink-3 transition-colors hover:text-ink",
        className,
      )}
    >
      {content}
    </Link>
  );
}

/**
 * "· fctl#120 design#14", each a link to it on GitHub after GitHub's icon for its state, with marks
 * for its checks and fctl's review of it while they matter, and what they say on hover; nothing
 * without any.
 */
export function PullRequestLinks({ s, className }: { s: G.Session; className?: string }) {
  const prs = pullRequests(s);
  if (prs.length === 0) return null;
  return (
    // The padding is room for the links' focus rings, which truncating would cut off.
    <span className={cn("-m-1 truncate p-1", className)}>
      ·
      {prs.map((pr) => (
        <Fragment key={pr.url}>
          {" "}
          <a
            href={pr.url}
            target="_blank"
            rel="noreferrer"
            title={prTitle(pr)}
            aria-label={prLabel(pr)}
            // Over the session's link, which covers the row.
            className="group/pr relative z-[2] mr-1 text-ink-3 transition-colors last:mr-0 hover:text-ink"
          >
            <PullRequestStateIcon pr={pr} className="mr-1 inline size-3 align-[-1.5px]" />
            <span className="underline-offset-2 group-hover/pr:underline">{prRef(pr)}</span>
            <PullRequestChecksIcon pr={pr} className="ml-1 inline size-3 align-[-1.5px]" />
            <PullRequestReviewMark pr={pr} className="ml-1 align-[-1.5px]" />
          </a>
        </Fragment>
      ))}
    </span>
  );
}

/** The ticket a session started from, as a link to it in its tracker: Linear's mark and its key. */
export function TicketLink({
  session,
  className,
}: {
  session: Pick<G.Session, "ticket">;
  className?: string;
}) {
  const t = session.ticket;
  if (!t) return null;
  return (
    <a
      href={t.url}
      target="_blank"
      rel="noreferrer"
      title={ticketLabel(t)}
      className={cn(
        "inline-flex min-w-0 items-center gap-1 font-mono text-[12px] text-ink-3 transition-colors hover:text-ink",
        className,
      )}
    >
      {t.tracker === "linear" ? (
        <LinearIcon className="size-3 shrink-0" />
      ) : (
        <GithubIcon className="size-3 shrink-0" />
      )}
      <span className="truncate">{t.key}</span>
    </a>
  );
}

/** That a ticket started the session, in a list: its key, a link over the session's own. */
export function TicketMark({ s }: { s: Pick<G.Session, "ticket"> }) {
  if (!s.ticket) return null;
  return <TicketLink session={s} className="relative z-[2] shrink-0" />;
}

// A session in a list (the Agent Floor, a workspace's page, a project's): its status, title,
// workspace and reference, its pull requests, who started it and when it last moved.

import { Link } from "@tanstack/react-router";
import { MessageCircleQuestion, Repeat } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import { AutopilotIcon, Avatar, GithubIcon, LinearIcon, StatusDot, statusLabel } from "@/components/ui";
import { ROW, ROW_ABOVE, ROW_COVER } from "@/components/ui/list-row";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { ago, agoShort, stamp } from "@/lib/format";
import { prLabel, prRef, prTitle, pullRequests } from "@/lib/pulls";
import { lastMoved } from "@/lib/sessions";
import { statusReason } from "@/lib/status";
import { ticketLabel } from "@/lib/tickets";
import { PullRequestChecksIcon, PullRequestReviewMark, PullRequestStateIcon } from "./pull-request";

/**
 * A session's row. `workspace` names its workspace, where the list has several; in its project's
 * list (`inProject`), its number says which it is, and its branch follows.
 */
export function SessionRow({
  s,
  workspace,
  inProject,
}: {
  s: G.Session;
  workspace?: G.WorkspaceRef;
  inProject?: boolean;
}) {
  const reason = statusReason(s);
  return (
    <div className={cn(ROW, "flex items-start gap-3 py-3")}>
      <StatusDot status={s.status} className="mt-[7px]" />
      <div className="min-w-0 flex-1">
        <SessionLink s={s}>
          <Moved s={s} />
        </SessionLink>
        <MovedText s={s} />
        {s.question && (
          // What its agent asks: it waits on someone.
          <p className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[12.5px] text-blue">
            <MessageCircleQuestion className="size-3.5 shrink-0" aria-hidden="true" />
            <span className="sr-only">Asks:</span>
            <span className="truncate">{s.question.text}</span>
          </p>
        )}
        <div className="mt-0.5 flex min-w-0 items-center gap-2 text-[12px] leading-[18px] text-ink-4">
          {workspace && (
            // On a phone its avatar says which one, and the reference keeps the room.
            // There the avatar keeps its width, so that a long status can't push it into the reference.
            <span className="inline-flex min-w-0 shrink items-center gap-1.5 text-ink-3 max-sm:shrink-0">
              <WorkspaceAvatar workspace={workspace} size={14} />
              <span className="max-w-[10rem] truncate max-sm:hidden">{workspace.name}</span>
              <span className="text-ink-4 max-sm:hidden">/</span>
            </span>
          )}
          <span className="shrink-0 font-mono text-ink-3">
            {inProject ? "" : s.project_slug}#{s.number}
          </span>
          <ScheduledMark s={s} />
          <TicketMark s={s} />
          {inProject && s.branch && (
            <>
              <Dot />
              <span className="min-w-0 truncate font-mono">{s.branch}</span>
            </>
          )}
          {reason && (
            <>
              <Dot />
              <span className="truncate">{reason}</span>
            </>
          )}
          {s.stats.files_changed > 0 && (
            <>
              <Dot className="max-sm:hidden" />
              <span className="shrink-0 tabular-nums max-sm:hidden">
                <span className="text-added/80">+{s.stats.additions}</span>{" "}
                <span className="text-danger/80">−{s.stats.deletions}</span>
              </span>
            </>
          )}
          {/* On a phone, a project's list has the room for them. */}
          <PullRequestLinks s={s} className={inProject ? undefined : "max-sm:hidden"} />
        </div>
      </div>
    </div>
  );
}

/** Between the parts of a row's second line. */
function Dot({ className }: { className?: string }) {
  return (
    <span aria-hidden="true" className={cn("shrink-0 text-ink-4", className)}>
      ·
    </span>
  );
}

/**
 * The session's title, as the link that opens it from anywhere on its row (`ROW_COVER`), and what
 * goes at the end of its line, in the link too.
 */
function SessionLink({ s, children }: { s: G.Session; children?: ReactNode }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      data-cover
      className={cn(ROW_COVER, "flex min-w-0 items-center gap-3")}
    >
      <span className="min-w-0 flex-1 truncate text-[14.5px] leading-[22px] text-ink">{s.title}</span>
      {children}
    </Link>
  );
}

/**
 * When the session last moved and who started it, at the end of its title's line: in the link, so
 * that a click there opens the session as anywhere else on its row, and over the row's cover, so
 * that each says it in full where the pointer rests. Screen readers read `MovedText` instead.
 */
function Moved({ s }: { s: G.Session }) {
  const at = lastMoved(s);
  return (
    <span aria-hidden="true" className={cn(ROW_ABOVE, "flex shrink-0 items-center gap-2.5")}>
      <time
        dateTime={new Date(at).toISOString()}
        title={`Last active ${stamp(at)}`}
        className="min-w-8 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
      >
        {agoShort(at)}
      </time>
      {s.autopilot ? (
        <span
          title={`Started by ${s.autopilot.project_slug}'s autopilot, for ${s.created_by.name}`}
          className="flex size-[18px] items-center justify-center rounded-full bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
        >
          <AutopilotIcon className="size-3" />
        </span>
      ) : (
        <Avatar user={s.created_by} size={18} title={`Started by ${s.created_by.name}`} />
      )}
    </span>
  );
}

/** What the session's status dot and `Moved` show, for screen readers, after its link. */
function MovedText({ s }: { s: G.Session }) {
  return (
    <span className="sr-only">
      {statusLabel(s.status)}. Started by{" "}
      {s.autopilot ? `${s.autopilot.project_slug}'s autopilot, for ${s.created_by.name}` : s.created_by.name},
      last active {ago(lastMoved(s))}.
    </span>
  );
}

/** That a schedule started the session: its symbol, with the schedule's name on hover. */
function ScheduledMark({ s }: { s: Pick<G.Session, "schedule"> }) {
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
function PullRequestLinks({ s, className }: { s: G.Session; className?: string }) {
  const prs = pullRequests(s);
  if (prs.length === 0) return null;
  return (
    <>
      <Dot className={className} />
      {/* The padding is room for the links' focus rings, which truncating would cut off. */}
      <span className={cn("-m-1 truncate p-1", className)}>
        {prs.map((pr, i) => (
          <Fragment key={pr.url}>
            {i > 0 && " "}
            <a
              href={pr.url}
              target="_blank"
              rel="noreferrer"
              title={prTitle(pr)}
              aria-label={prLabel(pr)}
              // Over the session's link, which covers the row.
              className={cn(ROW_ABOVE, "group/pr mr-1 text-ink-3 transition-colors last:mr-0 hover:text-ink")}
            >
              <PullRequestStateIcon pr={pr} className="mr-1 inline size-3 align-[-1.5px]" />
              <span className="underline-offset-2 group-hover/pr:underline">{prRef(pr)}</span>
              <PullRequestChecksIcon pr={pr} className="ml-1 inline size-3 align-[-1.5px]" />
              <PullRequestReviewMark pr={pr} className="ml-1 align-[-1.5px]" />
            </a>
          </Fragment>
        ))}
      </span>
    </>
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

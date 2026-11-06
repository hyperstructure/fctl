// A pull request as sessions show it: its state as GitHub's octicon for it, in GitHub's color, a
// mark for its checks, and how fctl's review of it stands.

import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowRight, CirclePause, Inbox, Play, RotateCw, Send } from "lucide-react";
import type { ReactNode } from "react";
import { ReviewStateIcon } from "@/components/review";
import { Button, ChecksIcon, PullRequestIcon, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import {
  type ChecksState,
  checksMark,
  type PrState,
  prReview,
  prReviewLook,
  prReviewText,
  prState,
  reviewAgainIn,
  reviewIssues,
} from "@/lib/pulls";
import { keys, queryClient, useProjects } from "@/lib/queries";
import { pullRef, shortSha } from "@/lib/reviews";

const STATE_COLORS: Record<PrState, string> = {
  open: "text-pr-open",
  draft: "text-pr-draft",
  queued: "text-pr-queued",
  merged: "text-pr-merged",
  closed: "text-pr-closed",
  unknown: "text-pr-unknown",
};

const CHECKS_COLORS: Record<ChecksState, string> = {
  passed: "text-checks-passed",
  pending: "text-checks-pending",
  failed: "text-checks-failed",
};

/** Its state: open, draft, queued, merged, closed, or unknown until someone looked it up. */
export function PullRequestStateIcon({ pr, className }: { pr: G.PullRequestRef; className?: string }) {
  const state = prState(pr);
  return <PullRequestIcon state={state} className={cn("shrink-0", STATE_COLORS[state], className)} />;
}

/** Its checks: passed, pending or failed. Nothing without any, or once it is merged or closed. */
export function PullRequestChecksIcon({ pr, className }: { pr: G.PullRequestRef; className?: string }) {
  const state = checksMark(pr);
  if (!state) return null;
  return <ChecksIcon state={state} className={cn("shrink-0", CHECKS_COLORS[state], className)} />;
}

/**
 * fctl's review of it, after its checks' mark, while the review says what they don't: an eye while it
 * waits for a runner, reading once it reviews, and a comment with how many issues it found. Nothing
 * once it found none (the checks' mark says so), stopped or failed: its line on the session's page
 * says so.
 */
export function PullRequestReviewMark({ pr, className }: { pr: G.PullRequestRef; className?: string }) {
  const review = prReview(pr);
  const look = review && prReviewLook(review);
  if (!review || (look !== "waiting" && look !== "reviewing" && look !== "commented")) return null;
  const n = look === "commented" ? reviewIssues(review) : 0;
  return (
    <span className={cn("inline-flex shrink-0 items-center gap-0.5", n > 0 && "text-blue", className)}>
      <ReviewStateIcon look={look} className="size-3" />
      {n > 0 && <span className="text-[11px] leading-none font-medium tabular-nums">{n}</span>}
    </span>
  );
}

/**
 * fctl's review of it, under it on the session's page: how it stands, in a line that opens the
 * review's session, where its agent's work and what it found show. Once it ended, a button beside
 * reviews the pull request again for those who may ask for reviews in the review's project. When
 * fctl held its findings back from the agent of the session whose pull request it is, or they went
 * to no session, the card ends with that (`HeldReview`, `UnsentReview`).
 */
export function PullRequestReviewLine({
  pr,
  review,
  passOnTo,
}: {
  pr: G.PullRequestRef;
  review: G.ReviewCheck;
  /** That session, where the person looking may pass on to its agent what fctl held back. */
  passOnTo?: string;
}) {
  const look = prReviewLook(review);
  const again = reviewAgainIn(review, useProjects().data?.projects);
  const id = review.session_id;
  const tone = look === "commented" && "text-ink-2";
  const held = review.held_after;
  const unsent = held == null && review.unsent === true;
  // The card's last line has its rounded corners.
  const last = held == null && !unsent && "rounded-b-[calc(var(--radius-md)-1px)]";
  return (
    <>
      <div
        className={cn(
          "group/review relative flex items-center gap-2.5 border-t border-line px-3 py-1.5 text-[11.5px] text-ink-3",
          // Its fill keeps to the card's rounded corners, inside its border, and leaves the button be.
          id && "transition-colors has-[a:hover]:bg-hover has-[a:hover]:text-ink",
          id && last,
        )}
      >
        {/* In the column of the pull request's icon. */}
        <span className="flex size-4 shrink-0 items-center justify-center">
          <ReviewStateIcon look={look} className="size-3.5" />
        </span>
        {id ? (
          <>
            <Link
              to="/s/$id"
              params={{ id }}
              title="Open the review's session"
              // The whole line opens it, but for the button.
              className={cn(
                "min-w-0 flex-1 outline-none after:absolute after:inset-0 focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-accent",
                last && "after:rounded-b-[calc(var(--radius-md)-1px)]",
                tone,
              )}
            >
              {prReviewText(review)}
            </Link>
            <ArrowRight className="size-3 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-has-[a:hover]/review:translate-x-0.5 group-has-[a:hover]/review:opacity-100" />
          </>
        ) : (
          <span className={cn("min-w-0 flex-1", tone)}>{prReviewText(review)}</span>
        )}
        {again && <ReviewAgain pr={pr} review={review} project={again} />}
      </div>
      {held != null && <HeldReview review={review} after={held} passOnTo={passOnTo} />}
      {unsent && <UnsentReview pr={pr} review={review} passOnTo={passOnTo} />}
    </>
  );
}

/** "5 review rounds", "1 more round". */
const rounds = (n: number, more = false) =>
  `${n} ${more ? "more " : "review "}${n === 1 ? "round" : "rounds"}`;

/**
 * What fctl held back of the review: its findings, which the agent didn't get, since it got those
 * of `after` reviews in a row with no message from a person. Those who may message the agent pass
 * them on, and it gets those of as many reviews in a row again; a message to it ends the pause
 * too, and leaves the findings to what it says.
 */
function HeldReview({
  review,
  after,
  passOnTo,
}: {
  review: G.ReviewCheck;
  after: number;
  passOnTo?: string;
}) {
  return (
    <PassOnStrip
      review={review}
      passOnTo={passOnTo}
      icon={<CirclePause aria-hidden="true" className="size-3.5 text-amber" />}
      title={`Paused after ${rounds(after)}`}
      text="fctl held these findings back, as nobody wrote to the agent in that time."
      action={{
        label: `Continue for ${rounds(after, true)}`,
        icon: <Play aria-hidden="true" className="size-3" />,
      }}
    />
  );
}

/**
 * The review's findings went to no agent: one of fctl's agents opened the pull request, but no session
 * took them when the review posted, as none listed it yet, say. fctl passes them on by itself once
 * the session that opened it lists it; those who may message this session's agent pass them on to
 * it, while the pull request is open.
 */
function UnsentReview({
  pr,
  review,
  passOnTo,
}: {
  pr: G.PullRequestRef;
  review: G.ReviewCheck;
  passOnTo?: string;
}) {
  const state = prState(pr);
  const done = state === "merged" || state === "closed";
  return (
    <PassOnStrip
      review={review}
      passOnTo={done ? undefined : passOnTo}
      icon={<Inbox aria-hidden="true" className="size-3.5 text-amber" />}
      title="Sent to no agent"
      text="These findings reached no agent: no session took them when the review posted."
      action={{ label: "Pass on to the agent", icon: <Send aria-hidden="true" className="size-3" /> }}
    />
  );
}

/**
 * The end of a pull request's card for findings of its review that the agent didn't get, and why,
 * with a button that passes them on to the agent of session `passOnTo` where there is one. The
 * button stays off once they went, until the card stops showing them.
 */
function PassOnStrip({
  review,
  passOnTo,
  icon,
  title,
  text,
  action,
}: {
  review: G.ReviewCheck;
  passOnTo?: string;
  icon: ReactNode;
  title: string;
  text: string;
  action: { label: string; icon: ReactNode };
}) {
  const pass = useMutation<G.SendMessageResponse, Error, { session: string; review: string }>({
    mutationFn: (v) => api.passOnReview(v.session, v.review),
    // Here, not in `mutate`: the card may stop showing this before the answer comes.
    onSuccess: () => toast("Findings passed on to the agent"),
    onError: (e) => toast(e.message, "error"),
  });
  const id = review.session_id;
  const done = pass.isSuccess && pass.variables.review === id;
  return (
    <div
      role="status"
      className="flex items-start gap-2.5 rounded-b-[calc(var(--radius-md)-1px)] border-t border-amber/20 bg-amber-soft px-3 pt-2 pb-2.5 text-[11.5px] leading-relaxed motion-safe:animate-rise"
    >
      {/* In the column of the pull request's icon, on the first line. */}
      <span className="flex h-[1lh] w-4 shrink-0 items-center justify-center">{icon}</span>
      <div className="min-w-0 flex-1">
        <p className="font-medium text-ink-2">{title}</p>
        <p className="text-ink-3">{text}</p>
        {passOnTo && id && (
          <Button
            size="sm"
            className="mt-2"
            loading={pass.isPending}
            disabled={done}
            icon={action.icon}
            onClick={() => pass.mutate({ session: passOnTo, review: id })}
          >
            {action.label}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * Reviews the pull request's newest commit again, on the subscription the project's review agents
 * run on, as the Reviews tab's "Review a pull request" does: for when what its review went by
 * changed since, an issue or an API it relies on. Its arrow turns while it asks, and it stays off
 * until the line shows the new review.
 */
function ReviewAgain({
  pr,
  review,
  project,
}: {
  pr: G.PullRequestRef;
  review: G.ReviewCheck;
  project: G.Project;
}) {
  // What it is asked with: the review it repeats.
  const start = useMutation<G.Review, Error, string | undefined>({
    mutationFn: () => api.startReview(project.id, { pull_request: pr.url }),
    // Here, not in `mutate`: the line may show the new review, without this button, before the
    // answer comes.
    onSuccess: (r) => {
      toast(`Reviewing ${pullRef(r)} at ${shortSha(r.head_sha)}`);
      void queryClient.invalidateQueries({ queryKey: keys.reviews(project.id) });
    },
    onError: (e) => toast(e.message, "error"),
  });
  const done = start.isSuccess && start.variables === review.session_id;
  return (
    <button
      type="button"
      aria-label="Review again"
      title="Review again"
      disabled={start.isPending || done}
      onClick={() => start.mutate(review.session_id)}
      // Off, it still takes the pointer: a click that went through would open the review's
      // session under it.
      className="relative -my-1 -mr-1.5 inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-4 transition-colors duration-150 enabled:hover:bg-hover enabled:hover:text-ink enabled:active:scale-95 disabled:cursor-default disabled:opacity-60"
    >
      <RotateCw className={cn("size-3", start.isPending && "motion-safe:animate-spin")} />
    </button>
  );
}

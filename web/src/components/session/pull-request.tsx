// A pull request as sessions show it: its state as GitHub's octicon for it, in GitHub's color, a
// mark for its checks, and how fctl's review of it stands.

import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowRight, RotateCw } from "lucide-react";
import { useState } from "react";
import { ReviewStateIcon } from "@/components/review";
import { ChecksIcon, PullRequestIcon, toast } from "@/components/ui";
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
 * reviews the pull request again for those who may ask for reviews in the review's project.
 */
export function PullRequestReviewLine({ pr, review }: { pr: G.PullRequestRef; review: G.ReviewCheck }) {
  const look = prReviewLook(review);
  const again = reviewAgainIn(review, useProjects().data?.projects);
  const id = review.session_id;
  const tone = look === "commented" && "text-ink-2";
  return (
    <div
      className={cn(
        "group/review relative flex items-center gap-2.5 border-t border-line px-3 py-1.5 text-[11.5px] text-ink-3",
        // Its fill keeps to the card's rounded corners, inside its border, and leaves the button be.
        id &&
          "rounded-b-[calc(var(--radius-md)-1px)] transition-colors has-[a:hover]:bg-hover has-[a:hover]:text-ink",
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
              "min-w-0 flex-1 outline-none after:absolute after:inset-0 after:rounded-b-[calc(var(--radius-md)-1px)] focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-accent",
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
  // The review it was asked to repeat: the line shows the new one once GitHub says so.
  const [asked, setAsked] = useState<string>();
  const start = useMutation({
    mutationFn: () => api.startReview(project.id, { pull_request: pr.url }),
    onError: (e) => toast(e.message, "error"),
  });
  const done = asked !== undefined && asked === review.session_id;
  return (
    <button
      type="button"
      aria-label="Review again"
      title="Review again"
      disabled={start.isPending || done}
      onClick={() => {
        const reviewed = review.session_id;
        start.mutate(undefined, {
          onSuccess: (r) => {
            setAsked(reviewed);
            toast(`Reviewing ${pullRef(r)} at ${shortSha(r.head_sha)}`);
            void queryClient.invalidateQueries({ queryKey: keys.reviews(project.id) });
          },
        });
      }}
      className="relative -my-1 -mr-1.5 inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-4 transition-colors duration-150 hover:bg-hover hover:text-ink active:scale-95 disabled:pointer-events-none disabled:opacity-60"
    >
      <RotateCw className={cn("size-3", start.isPending && "motion-safe:animate-spin")} />
    </button>
  );
}

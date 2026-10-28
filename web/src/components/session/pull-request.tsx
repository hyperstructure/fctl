// A pull request as sessions show it: its state as GitHub's octicon for it, in GitHub's color, a
// mark for its checks, and how fctl's review of it stands.

import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { ReviewStateIcon } from "@/components/review";
import { ChecksIcon, PullRequestIcon } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import {
  type ChecksState,
  checksMark,
  type PrState,
  prReview,
  prReviewLook,
  prReviewText,
  prState,
  reviewIssues,
} from "@/lib/pulls";

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
 * review's session, where its agent's work and what it found show.
 */
export function PullRequestReviewLine({ review }: { review: G.ReviewCheck }) {
  const look = prReviewLook(review);
  const cls =
    "group/review flex items-center gap-2.5 border-t border-line px-3 py-1.5 text-[11.5px] text-ink-3";
  const content = (
    <>
      {/* In the column of the pull request's icon. */}
      <span className="flex size-4 shrink-0 items-center justify-center">
        <ReviewStateIcon look={look} className="size-3.5" />
      </span>
      <span className={cn("min-w-0 flex-1", look === "commented" && "text-ink-2")}>
        {prReviewText(review)}
      </span>
    </>
  );
  if (!review.session_id) return <div className={cls}>{content}</div>;
  return (
    <Link
      to="/s/$id"
      params={{ id: review.session_id }}
      title="Open the review's session"
      className={cn(cls, "transition-colors hover:bg-hover hover:text-ink")}
    >
      {content}
      <ArrowRight className="size-3 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover/review:translate-x-0.5 group-hover/review:opacity-100" />
    </Link>
  );
}

// A pull request as sessions show it: its state as GitHub's octicon for it, in GitHub's color, and
// a mark for its checks.

import { ChecksIcon, PullRequestIcon } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { type ChecksState, checksState, type PrState, prState } from "@/lib/pulls";

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
  const state = checksState(pr);
  if (!state) return null;
  return <ChecksIcon state={state} className={cn("shrink-0", CHECKS_COLORS[state], className)} />;
}

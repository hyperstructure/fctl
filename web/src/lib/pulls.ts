// A session's pull requests: every one its agent found, in all of its
// project's repositories, and how the app names them: their state, as
// GitHub last told fctl, and their checks.

import type * as G from "@/gen";
import { ago, stamp } from "./format";

type Pulls = Pick<G.Session, "pull_request" | "pull_requests">;

/**
 * Whether a pull request's link is GitHub's own. Agents report their pull requests, and what runs in a sandbox can
 * pose as the agent; the control plane keeps only these now, but sessions may list others from before it did.
 */
export function isGithubPull(pr: Pick<G.PullRequestRef, "url">): boolean {
  return /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+$/.test(pr.url);
}

/** Every pull request of a session, in the order its agent found them: those GitHub's own links name. */
export function pullRequests(s: Pulls): G.PullRequestRef[] {
  return (s.pull_requests ?? (s.pull_request ? [s.pull_request] : [])).filter(isGithubPull);
}

/** Its repository's name and its number: "fctl#120" ("#120" when the repository is unknown). */
export function prRef(pr: G.PullRequestRef): string {
  return `${pr.repo?.name ?? ""}#${pr.number}`;
}

/** The same with the repository's owner: "hyperstructure/fctl#120". */
export function prFullRef(pr: G.PullRequestRef): string {
  return pr.repo ? `${pr.repo.owner}/${pr.repo.name}#${pr.number}` : `#${pr.number}`;
}

/** A pull request's state; unknown until someone looked it up. */
export type PrState = "open" | "draft" | "queued" | "merged" | "closed" | "unknown";

export const PR_STATES: Record<PrState, string> = {
  open: "Open",
  draft: "Draft",
  queued: "Queued",
  merged: "Merged",
  closed: "Closed",
  unknown: "Unknown",
};

export function prState(pr: G.PullRequestRef): PrState {
  switch (pr.state) {
    case "open":
    case "draft":
    case "queued":
    case "merged":
    case "closed":
      return pr.state;
    case "in_merge_queue":
      return "queued";
    default:
      return "unknown";
  }
}

/**
 * Its state, and when it took it where that says something: "Merged 2 hours ago", "Closed
 * yesterday", "Queued 5 minutes ago", "Open".
 */
export function prStateText(pr: G.PullRequestRef, now = Date.now()): string {
  const state = prState(pr);
  const when = ["merged", "closed", "queued"].includes(state) && pr.state_changed_at;
  return when ? `${PR_STATES[state]} ${ago(when, now)}` : PR_STATES[state];
}

/** Its checks all passed, one failed, or some are still to finish. */
export type ChecksState = "passed" | "failed" | "pending";

/** Its checks' state, while that matters: until it is merged or closed, and once it has any. */
export function checksState(pr: G.PullRequestRef): ChecksState | undefined {
  const c = pr.checks;
  if (!c || !["open", "draft", "queued"].includes(prState(pr))) return undefined;
  if (c.failed > 0) return "failed";
  if (c.pending > 0) return "pending";
  if (c.passed > 0) return "passed";
  return undefined;
}

/** "All 5 checks passed", "1 of 4 checks failed", "2 of 5 checks pending": what `checksState` says. */
export function checksText(pr: G.PullRequestRef): string | undefined {
  const state = checksState(pr);
  const c = pr.checks;
  if (!state || !c) return undefined;
  const total = c.passed + c.failed + c.pending;
  const checks = (n: number) => (n === 1 ? "check" : "checks");
  if (state === "passed") return total === 1 ? "1 check passed" : `All ${total} checks passed`;
  const n = state === "failed" ? c.failed : c.pending;
  return n === total ? `${n} ${checks(n)} ${state}` : `${n} of ${total} checks ${state}`;
}

/**
 * For a tooltip: "hyperstructure/fctl#120: Fix the flaky login test", and on a second line its
 * state and checks, once someone looked it up: "Merged Oct 21, 2026, 4:12 PM".
 */
export function prTitle(pr: G.PullRequestRef): string {
  const name = pr.title ? `${prFullRef(pr)}: ${pr.title}` : prFullRef(pr);
  const state = prState(pr);
  if (state === "unknown") return name;
  const when = ["merged", "closed", "queued"].includes(state) && pr.state_changed_at;
  const said = [when ? `${PR_STATES[state]} ${stamp(when)}` : PR_STATES[state], checksText(pr)];
  return `${name}\n${said.filter(Boolean).join(" · ")}`;
}

/** What a link to it says to screen readers: "fctl#120, merged 2 hours ago, all 5 checks passed". */
export function prLabel(pr: G.PullRequestRef, now = Date.now()): string {
  const state = prState(pr);
  const parts = [prRef(pr), state === "unknown" ? undefined : prStateText(pr, now), checksText(pr)];
  return parts
    .filter(Boolean)
    .map((p, i) => (i === 0 ? p : p?.charAt(0).toLowerCase() + (p?.slice(1) ?? "")))
    .join(", ");
}

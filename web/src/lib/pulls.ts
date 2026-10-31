// A session's pull requests: every one its agent found, in all of its
// project's repositories, and how the app names them: their state, as
// GitHub last told fctl, and their checks.

import type * as G from "@/gen";
import { ago, stamp } from "./format";
import { canAskForReview, issues, type ReviewLook } from "./reviews";

type Pulls = Pick<G.Session, "pull_request" | "pull_requests">;

/**
 * Whether a pull request's link is GitHub's own. Agents report their pull requests, and what runs in a sandbox can
 * pose as the agent; the control plane keeps only these now, but sessions may list others from before it did.
 */
export function isGithubPull(pr: Pick<G.PullRequestRef, "url">): boolean {
  return /^https:\/\/github\.com\/[\w.-]+\/[\w.-]+\/pull\/\d+$/.test(pr.url);
}

/**
 * A link to a page on GitHub, as the app shows the links of reviews and follow-ups as GitHub's own, or undefined for
 * one anywhere else: what runs in a sandbox can pose as its agent, whose notices carry links too.
 */
export function githubLink(url: string | undefined): string | undefined {
  return url && /^https:\/\/github\.com\/[^\s\\]*$/.test(url) ? url : undefined;
}

/**
 * Every pull request of a session, those GitHub's own links name: first those still under way (open, draft, queued),
 * then those nobody looked up yet, then the merged and closed ones, each in the order its agent found them. Lists
 * that run out of room cut off the end, which then holds those done with.
 */
export function pullRequests(s: Pulls): G.PullRequestRef[] {
  const all = (s.pull_requests ?? (s.pull_request ? [s.pull_request] : [])).filter(isGithubPull);
  return all.sort((a, b) => STATE_ORDER[prState(a)] - STATE_ORDER[prState(b)]);
}

/** Where pull requests in each state come in a session's list. */
const STATE_ORDER: Record<PrState, number> = {
  open: 0,
  draft: 0,
  queued: 0,
  unknown: 1,
  merged: 2,
  closed: 2,
};

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

/** Whether its checks matter: until it is merged or closed. */
const checksMatter = (pr: G.PullRequestRef) => ["open", "draft", "queued"].includes(prState(pr));

/**
 * Its checks' state, while that matters: until it is merged or closed, and once it has any. fctl's own review
 * isn't among them: `prReview` says how it went.
 */
export function checksState(pr: G.PullRequestRef): ChecksState | undefined {
  const c = pr.checks;
  if (!c || !checksMatter(pr)) return undefined;
  if (c.failed > 0) return "failed";
  if (c.pending > 0) return "pending";
  if (c.passed > 0) return "passed";
  return undefined;
}

/** The mark lists give its checks: their state, or passed where fctl's review passed and nothing else checks it. */
export function checksMark(pr: G.PullRequestRef): ChecksState | undefined {
  return checksState(pr) ?? (prReview(pr)?.state === "passed" ? "passed" : undefined);
}

/**
 * fctl's review of its head commit, where the project's review agents review it, while its checks matter. GitHub
 * counts its check as passed when it found issues (neutral): this says so instead.
 */
export function prReview(pr: G.PullRequestRef): G.ReviewCheck | undefined {
  return checksMatter(pr) ? pr.checks?.review : undefined;
}

/** How fctl's review shows: under way (waiting for a runner, or reviewing), or how it ended. */
export function prReviewLook(r: G.ReviewCheck): ReviewLook {
  if (r.state === "running") return r.waiting ? "waiting" : "reviewing";
  return r.state;
}

/**
 * Where someone may review a pull request again once fctl's review of it ended, as the Reviews tab's "Review a pull
 * request" does: the project whose review agents reviewed it, among `projects` (theirs), where they start sessions.
 * Nothing while it runs, and where the control plane didn't say whose review it was.
 */
export function reviewAgainIn(r: G.ReviewCheck, projects: G.Project[] | undefined): G.Project | undefined {
  if (r.state === "running" || !r.project_id) return undefined;
  const p = projects?.find((p) => p.id === r.project_id);
  return p && canAskForReview(p.role) && !p.archived_at ? p : undefined;
}

/** How many issues it points out: those it found, else those of earlier reviews still open. */
export const reviewIssues = (r: G.ReviewCheck) => (r.findings > 0 ? r.findings : r.still_open);

/**
 * What fctl's review says, as `fctl show` says it beside its checks (`fctl_domain::reviews::said`): "fctl's
 * review agent found 3 issues", "fctl's review agent is reviewing it".
 */
export function prReviewText(r: G.ReviewCheck): string {
  switch (prReviewLook(r)) {
    case "waiting":
      return "fctl's review agent waits for a runner";
    case "reviewing":
      return "fctl's review agent is reviewing it";
    case "passed":
      return "fctl's review agent found no issues";
    case "commented":
      if (r.findings > 0) return `fctl's review agent found ${issues(r.findings)}`;
      if (r.still_open > 0)
        return `${issues(r.still_open)} from fctl's earlier reviews ${r.still_open === 1 ? "is" : "are"} still open`;
      return "fctl's review agent found issues";
    case "failed":
      return "fctl's review failed";
    case "superseded":
    case "stopped":
      return "fctl's review stopped";
  }
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

/** What its checks and fctl's review of it say: "All 5 checks passed", "fctl's review agent found 2 issues". */
const checksSaid = (pr: G.PullRequestRef) => {
  const review = prReview(pr);
  return [checksText(pr), review && prReviewText(review)];
};

/**
 * For a tooltip: "hyperstructure/fctl#120: Fix the flaky login test", and on a second line its
 * state and checks, once someone looked it up: "Merged Oct 21, 2026, 4:12 PM".
 */
export function prTitle(pr: G.PullRequestRef): string {
  const name = pr.title ? `${prFullRef(pr)}: ${pr.title}` : prFullRef(pr);
  const state = prState(pr);
  if (state === "unknown") return name;
  const when = ["merged", "closed", "queued"].includes(state) && pr.state_changed_at;
  const said = [when ? `${PR_STATES[state]} ${stamp(when)}` : PR_STATES[state], ...checksSaid(pr)];
  return `${name}\n${said.filter(Boolean).join(" · ")}`;
}

/**
 * What a link to it says to screen readers: "fctl#120, merged 2 hours ago, all 5 checks passed",
 * "fctl#121, open, all 5 checks passed, fctl's review agent found 2 issues".
 */
export function prLabel(pr: G.PullRequestRef, now = Date.now()): string {
  const state = prState(pr);
  const parts = [prRef(pr), state === "unknown" ? undefined : prStateText(pr, now), ...checksSaid(pr)];
  return parts
    .filter(Boolean)
    .map((p, i) => (i === 0 ? p : p?.charAt(0).toLowerCase() + (p?.slice(1) ?? "")))
    .join(", ");
}

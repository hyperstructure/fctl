// A session's pull requests: every one its agent found, in all of its
// project's repositories, and how the app names them.

import type * as G from "@/gen";

type Pulls = Pick<G.Session, "pull_request" | "pull_requests">;

/** Every pull request of a session, in the order its agent found them. */
export function pullRequests(s: Pulls): G.PullRequestRef[] {
  return s.pull_requests ?? (s.pull_request ? [s.pull_request] : []);
}

/** Its repository's name and its number: "fctl#120" ("#120" when the repository is unknown). */
export function prRef(pr: G.PullRequestRef): string {
  return `${pr.repo?.name ?? ""}#${pr.number}`;
}

/** The same with the repository's owner: "hyperstructure/fctl#120". */
export function prFullRef(pr: G.PullRequestRef): string {
  return pr.repo ? `${pr.repo.owner}/${pr.repo.name}#${pr.number}` : `#${pr.number}`;
}

/** For a tooltip: "hyperstructure/fctl#120: Fix the flaky login test". */
export function prTitle(pr: G.PullRequestRef): string {
  return pr.title ? `${prFullRef(pr)}: ${pr.title}` : prFullRef(pr);
}

/** What session lists say: "PR fctl#120", "PRs fctl#120, design#14"; undefined without any. */
export function prsLabel(prs: G.PullRequestRef[]): string | undefined {
  if (prs.length === 0) return undefined;
  return `${prs.length === 1 ? "PR" : "PRs"} ${prs.map(prRef).join(", ")}`;
}

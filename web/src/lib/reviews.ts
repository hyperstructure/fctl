// Review agents: where a project turns them on, an agent reviews each new commit of its pull
// requests, on the Claude subscription of the person who turned them on, and fctl posts what it
// found on GitHub in one review, with a check on the commit. Here: how a review looks in lists,
// and who may change a project's review agents.

import type * as G from "@/gen";

/** How a review shows: under way (waiting for a runner, or reviewing), or how it ended. */
export type ReviewLook =
  | "waiting"
  | "reviewing"
  | "passed"
  | "commented"
  | "superseded"
  | "stopped"
  | "failed";

export function reviewLook(r: Pick<G.Review, "state" | "status">): ReviewLook {
  if (r.state === "running") return r.status === "queued" ? "waiting" : "reviewing";
  return r.state;
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What came of a review, in a few words: "No issues", "2 issues", "Reviewing"… */
export function reviewOutcome(r: Pick<G.Review, "state" | "status" | "findings" | "still_open">): string {
  switch (reviewLook(r)) {
    case "waiting":
      return "Waiting for a runner";
    case "reviewing":
      return "Reviewing";
    case "passed":
      return "No issues";
    case "commented":
      return r.findings > 0
        ? plural(r.findings, "issue", "issues")
        : plural(r.still_open, "earlier issue open", "earlier issues open");
    case "superseded":
      return "Superseded";
    case "stopped":
      return "Stopped";
    case "failed":
      return "Failed";
  }
}

/** What it did with earlier reviews' findings: "1 fixed", "2 still open"; nothing without any. */
export function earlierFindings(
  r: Pick<G.Review, "resolved" | "still_open" | "findings" | "state">,
): string[] {
  const parts: string[] = [];
  if (r.resolved > 0) parts.push(`${r.resolved} fixed`);
  // A review that found nothing new says so in its outcome already.
  if (r.still_open > 0 && r.findings > 0) parts.push(`${r.still_open} still open`);
  return parts;
}

/** A commit's first seven characters, as GitHub shows it. */
export const shortSha = (sha: string) => sha.slice(0, 7);

/** `web#12`: the pull request by its repository's name. */
export const pullRef = (r: Pick<G.Review, "repo" | "number">) => `${r.repo.name}#${r.number}`;

/** The commit on GitHub, in its pull request. */
export const commitUrl = (r: Pick<G.Review, "url" | "head_sha">) => `${r.url}/commits/${r.head_sha}`;

/** Whether `role` turns a project's review agents on or off and says how they review: its owners. */
export const canManageReviews = (role?: G.ProjectRole) => role === "owner";

/** Whether someone may ask for a review of a pull request: those who start sessions in the project. */
export const canAskForReview = (role?: G.ProjectRole) =>
  role === "owner" || role === "maintainer" || role === "editor";

/** "acme/web", "acme/web and acme/api", "acme/web, acme/api and 2 more". */
export function repoList(repos: G.RepoRef[]): string {
  const names = repos.map((r) => `${r.owner}/${r.name}`);
  if (names.length <= 2) return names.join(" and ");
  if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
}

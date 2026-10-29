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

/** "1 issue", "3 issues". */
export const issues = (n: number) => plural(n, "issue", "issues");

/** A finding's severity, as fctl says it on GitHub: "High", "Medium", "Low". */
export const severityLabel = (s: G.Severity) => `${s.charAt(0).toUpperCase()}${s.slice(1)}`;

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
        ? issues(r.findings)
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

/** `acme/web`. */
export const repoName = (r: G.RepoRef) => `${r.owner}/${r.name}`;

/** Whether `a` and `b` name the same repository: GitHub ignores case. */
export const sameRepo = (a: G.RepoRef, b: G.RepoRef) =>
  repoName(a).toLowerCase() === repoName(b).toLowerCase();

/**
 * The repositories a project's review agents review or skip (those its agents may write to), as its
 * Repositories tab lists them: its primary repository first, then the others by name.
 */
export function reviewRepos(
  agents: Pick<G.ReviewAgents, "repos" | "skipped_repos">,
  primary?: G.RepoRef,
): G.RepoRef[] {
  const key = (r: G.RepoRef) => `${primary && sameRepo(r, primary) ? 0 : 1}${repoName(r).toLowerCase()}`;
  return [...agents.repos, ...agents.skipped_repos].sort((a, b) =>
    key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0,
  );
}

/** Whether two lists of repositories (`owner/name`) are the same ones, in any order. */
export function sameRepos(a: readonly string[], b: readonly string[]): boolean {
  const x = new Set(a.map((r) => r.toLowerCase()));
  const y = new Set(b.map((r) => r.toLowerCase()));
  return x.size === y.size && [...x].every((r) => y.has(r));
}

/** The GitHub alert that fctl's summary of a pull request is in: a note, a warning or a caution. */
export type RiskAlert = "note" | "warning" | "caution";

/**
 * How a pull request's risk reads where fctl sums it up at the end of its description: in a note at
 * low and medium risk, a warning at high risk, and a caution where a serious security problem shows
 * at first sight, as on GitHub.
 */
export const RISKS: Record<G.Risk, { label: string; alert: RiskAlert }> = {
  low: { label: "Low risk", alert: "note" },
  medium: { label: "Medium risk", alert: "note" },
  high: { label: "High risk", alert: "warning" },
  critical: { label: "Critical risk", alert: "caution" },
};

/**
 * The summary as GitHub shows it in the alert, as Markdown: how risky the pull request looks and
 * why, what it does, and what deserves a closer look, each led in bold.
 */
export function summaryMarkdown(s: G.PullRequestSummary): string {
  const [first = "", ...rest] = s.overview.split("\n\n");
  const parts = [`**${RISKS[s.risk].label}.** ${s.risk_reason}`, `**What it does.** ${first}`, ...rest];
  if (s.highlight) {
    const stop = /[?!]$/.test(s.highlight.topic) ? "" : ".";
    parts.push(`**${s.highlight.topic}${stop}** ${s.highlight.text}`);
  }
  return parts.join("\n\n");
}

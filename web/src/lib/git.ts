// A session's changes per repository: the primary one, then the others the
// agent cloned (GitState.repos).
import type * as G from "@/gen";
import { isGithubPull } from "./pulls";

export type RepoChanges = {
  /** `owner/name`; absent for the primary repository. */
  name?: string;
  readOnly: boolean;
  files: G.FileChange[];
  additions: number;
  deletions: number;
  ahead: number;
  pushed: boolean;
  pr?: G.PullRequestRef;
  /** Once `pr` is merged: the commits after it. */
  unmerged?: number;
};

type State = {
  files: G.FileChange[];
  ahead: number;
  pushed: boolean;
  pull_request?: G.PullRequestRef;
  unmerged?: number;
};

function changes(name: string | undefined, readOnly: boolean, s: State): RepoChanges {
  return {
    name,
    readOnly,
    files: s.files,
    additions: s.files.reduce((n, f) => n + (f.additions ?? 0), 0),
    deletions: s.files.reduce((n, f) => n + (f.deletions ?? 0), 0),
    ahead: s.ahead,
    pushed: s.pushed,
    pr: s.pull_request && isGithubPull(s.pull_request) ? s.pull_request : undefined,
    unmerged: s.unmerged,
  };
}

/** The primary repository first, then the others. */
export function repoChanges(git: G.GitState): RepoChanges[] {
  return [
    changes(undefined, false, git),
    ...(git.repos ?? []).map((r) => changes(`${r.owner}/${r.name}`, r.access === "read", r)),
  ];
}

/** Whether a repository has changes or commits to show. The session lists its
 * pull requests on their own (lib/pulls). */
export const hasChanges = (r: RepoChanges) => r.files.length > 0 || r.ahead > 0;

/**
 * Where a repository's commits are: "pushed", "not pushed yet", or "kept in
 * the sandbox" for a read-only one. Once its branch's pull request is merged,
 * "#12 merged", and the commits after it: fctl-agent doesn't push a merged
 * branch.
 */
export function whereCommits(r: RepoChanges): string[] {
  if (r.pr?.state === "merged") {
    const merged = `#${r.pr.number} merged`;
    const n = r.unmerged ?? 0;
    if (n === 0) return [merged];
    return [merged, `${n} ${n === 1 ? "commit" : "commits"} since, ${r.pushed ? "pushed" : "not pushed"}`];
  }
  if (r.pushed) return ["pushed"];
  if (r.ahead === 0) return [];
  return [r.readOnly ? "kept in the sandbox" : "not pushed yet"];
}

/** What a git state says of one repository's branch. */
type Branch = { head?: string; ahead: number; pushed: boolean; pr?: string };

type BranchState = { head?: string; ahead: number; pushed: boolean; pull_request?: G.PullRequestRef };

function branches(git: G.GitState): Map<string | undefined, Branch> {
  const branch = (s: BranchState): Branch => ({
    head: s.head,
    ahead: s.ahead,
    pushed: s.pushed,
    pr: s.pull_request?.url,
  });
  return new Map<string | undefined, Branch>([
    [undefined, branch(git)],
    ...(git.repos ?? []).map((r) => [`${r.owner}/${r.name}`, branch(r)] as const),
  ]);
}

/**
 * The repositories whose branch changed since the state before, named as in
 * `repoChanges`: a commit of their own (the head moved, with commits the base
 * lacks), a push of them, or a pull request not in `seen`, which keeps the
 * pull requests' URLs from one call to the next. fctl-agent reports the state
 * while Claude works, after each turn and when the session stops, and most
 * reports only move files.
 */
export function branchChanges(
  before: G.GitState | undefined,
  after: G.GitState,
  seen: Set<string>,
): (string | undefined)[] {
  const was = before ? branches(before) : new Map<string | undefined, Branch>();
  const changed: (string | undefined)[] = [];
  for (const [name, b] of branches(after)) {
    const w = was.get(name);
    const committed = b.ahead > 0 && !!b.head && b.head !== w?.head;
    const pushed = b.ahead > 0 && b.pushed && !w?.pushed;
    const opened = !!b.pr && !seen.has(b.pr);
    if (b.pr) seen.add(b.pr);
    if (committed || pushed || opened) changed.push(name);
  }
  return changed;
}

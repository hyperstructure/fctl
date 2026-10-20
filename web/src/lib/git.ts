// A session's changes per repository: the primary one, then the others the
// agent cloned (GitState.repos).
import type * as G from "@/gen";

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
};

type State = { files: G.FileChange[]; ahead: number; pushed: boolean; pull_request?: G.PullRequestRef };

function changes(name: string | undefined, readOnly: boolean, s: State): RepoChanges {
  return {
    name,
    readOnly,
    files: s.files,
    additions: s.files.reduce((n, f) => n + (f.additions ?? 0), 0),
    deletions: s.files.reduce((n, f) => n + (f.deletions ?? 0), 0),
    ahead: s.ahead,
    pushed: s.pushed,
    pr: s.pull_request,
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

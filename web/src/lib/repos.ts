// A project's GitHub repositories in a few words: the one its agents start
// in, and how many more they may use.

import type * as G from "@/gen";

type Repos = Pick<G.Project, "repo" | "other_repos" | "all_repos_of">;

export interface RepoSummary {
  /** The primary repository ("hyperstructure/fctl"), or the account of an owner-wide grant without one. */
  name?: string;
  /** The others: "(+3 more)", "(+entire org)"; without a primary repository, "3 repositories". */
  more?: string;
}

/** What lists show of a project's repositories; undefined when it has none. */
export function repoSummary(p: Repos): RepoSummary | undefined {
  const n = p.other_repos ?? 0;
  if (p.repo) {
    const more = p.all_repos_of ? "(+entire org)" : n > 0 ? `(+${n} more)` : undefined;
    return { name: `${p.repo.owner}/${p.repo.name}`, more };
  }
  if (p.all_repos_of) return { name: p.all_repos_of, more: "(entire org)" };
  if (n > 0) return { more: `${n} ${n === 1 ? "repository" : "repositories"}` };
  return undefined;
}

/** The same in one line: "hyperstructure/fctl (+3 more)". */
export function repoLabel(p: Repos): string | undefined {
  const s = repoSummary(p);
  return s && [s.name, s.more].filter(Boolean).join(" ");
}

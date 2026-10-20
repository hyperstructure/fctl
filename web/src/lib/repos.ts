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

/**
 * Where a project's agents work, for a tooltip: "Agents work in
 * hyperstructure/fctl, in Hyperstructure, and may also use 3 more repositories".
 */
export function repoTitle(p: Repos, workspace?: string): string | undefined {
  const n = p.other_repos ?? 0;
  const where = workspace ? `, in ${workspace}` : "";
  const others = p.all_repos_of
    ? `read every repository of ${p.all_repos_of}`
    : n > 0
      ? `use ${n} ${p.repo ? "more " : ""}${n === 1 ? "repository" : "repositories"}`
      : undefined;
  if (p.repo) {
    return `Agents work in ${p.repo.owner}/${p.repo.name}${where}${others ? `, and may also ${others}` : ""}`;
  }
  return others && `Agents start in an empty directory${where}, and may ${others}`;
}

// What a session's agent may do on GitHub (G.GithubAccess): push, as every agent did before;
// comment, as review agents do (review, comment and label, push nothing); or only read, as
// planners do. A session asks for it as it starts, within its project's settings
// (ProjectSettings.github_access) and the access token that starts it, and GitHub holds the agent
// to it: its tokens carry those permissions.

import type * as G from "@/gen";

export interface GithubLevel {
  value: G.GithubAccess;
  /** What the composer and the session say. */
  label: string;
  /** What a project's settings say its agents do at most. */
  most: string;
  description: string;
}

const PUSH: GithubLevel = {
  value: "push",
  label: "Can push",
  most: "Push",
  description: "Commits, pushes its branch and opens pull requests.",
};

/** From the most to the least an agent may do. */
export const GITHUB_ACCESS: readonly GithubLevel[] = [
  PUSH,
  {
    value: "comment",
    label: "Comments only",
    most: "Comment",
    description:
      "Reviews and comments on pull requests and issues, and resolves review threads. Pushes nothing.",
  },
  {
    value: "read",
    label: "Read only",
    most: "Read",
    description: "Reads the code, pull requests, issues and CI results. Writes nothing to GitHub.",
  },
];

const RANK: Record<G.GithubAccess, number> = { read: 0, comment: 1, push: 2 };

/** The lower of two levels. */
export const lowerAccess = (a: G.GithubAccess, b: G.GithubAccess) => (RANK[a] <= RANK[b] ? a : b);

/** Whether `a` lets an agent do at most what `b` does. */
export const within = (a: G.GithubAccess, b: G.GithubAccess) => RANK[a] <= RANK[b];

export const githubLevel = (g: G.GithubAccess): GithubLevel =>
  GITHUB_ACCESS.find((l) => l.value === g) ?? PUSH;

/** The most a project's agents may do on GitHub. */
export const projectGithub = (p: Pick<G.Project, "settings">): G.GithubAccess =>
  p.settings.github_access ?? "push";

/** What a session's agent may do on GitHub now: what it started with, within its project's settings. */
export function sessionGithub(s: Pick<G.Session, "github_access">, project?: Pick<G.Project, "settings">) {
  const own = s.github_access ?? "push";
  return project ? lowerAccess(own, projectGithub(project)) : own;
}

/** The levels a session in `p` may ask for, the most first. */
export const githubOffers = (p: Pick<G.Project, "settings">) =>
  GITHUB_ACCESS.filter((l) => within(l.value, projectGithub(p)));

/** Whether a project's agents work on GitHub at all: it has a repository. */
export const onGithub = (p: Pick<G.Project, "repo" | "other_repos" | "all_repos_of">) =>
  !!p.repo || !!p.other_repos || !!p.all_repos_of;

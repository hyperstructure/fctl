// Which GitHub App posts review agents' reviews (`fctl_domain::review_app`): the instance's review
// app (fctl-review) where it's installed on the account and reaches the repository, so that people
// tell its findings apart from what agents write as the instance's GitHub App (fctl-sh), which posts
// them elsewhere.

import type * as G from "@/gen";

/** The app a review posts as: the review app, or the GitHub App. */
export type Poster = "review" | "github";

/** The review app's standing on the workspace's account `owner`. */
export function accountOf(r: G.WorkspaceReviewApp, owner: string): G.ReviewAppAccount | undefined {
  const o = owner.toLowerCase();
  return r.accounts.find((a) => a.account.toLowerCase() === o);
}

/** Which app posts the reviews of `repo`; `undefined` where GitHub didn't say. */
export function postsAs(r: G.WorkspaceReviewApp, repo: G.RepoRef): Poster | undefined {
  if (!r.configured || !r.app) return "github";
  const a = accountOf(r, repo.owner);
  if (!a) return "github";
  switch (a.state) {
    case "installed":
      return "review";
    case "partly": {
      const name = repo.name.toLowerCase();
      return a.unreached.some((u) => u.toLowerCase() === name) ? "github" : "review";
    }
    case "not_installed":
      return "github";
    default:
      return undefined;
  }
}

/** Of `repos`, those whose reviews post as the GitHub App, and the accounts they're on, where installing the review app or giving it more repositories changes that. */
export function postedAsGithubApp(
  r: G.WorkspaceReviewApp,
  repos: readonly G.RepoRef[],
): { repos: G.RepoRef[]; accounts: G.ReviewAppAccount[] } {
  const elsewhere = repos.filter((repo) => postsAs(r, repo) === "github");
  const accounts: G.ReviewAppAccount[] = [];
  for (const repo of elsewhere) {
    const a = accountOf(r, repo.owner);
    if (a && !accounts.includes(a)) accounts.push(a);
  }
  return { repos: elsewhere, accounts };
}

/** Which app posts all of `repos`' reviews, when one does; `undefined` when they're split or unknown. */
export function postsAllAs(r: G.WorkspaceReviewApp, repos: readonly G.RepoRef[]): Poster | undefined {
  const posters = new Set(repos.map((repo) => postsAs(r, repo)));
  if (posters.size !== 1) return undefined;
  return [...posters][0];
}

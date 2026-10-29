import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { postedAsGithubApp, postsAllAs, postsAs } from "./review-app";

const app = (slug: string): G.GithubAppRef => ({ slug });
const repo = (owner: string, name: string): G.RepoRef => ({ owner, name });

const r = (accounts: G.ReviewAppAccount[], configured = true): G.WorkspaceReviewApp => ({
  configured,
  app: configured ? app("fctl-review") : undefined,
  github_app: app("fctl-sh"),
  accounts,
});

const acme: G.ReviewAppAccount = { account: "Acme", state: "installed", unreached: [] };
const globex: G.ReviewAppAccount = { account: "globex", state: "partly", unreached: ["Docs"] };
const initech: G.ReviewAppAccount = { account: "initech", state: "not_installed", unreached: [] };
const hooli: G.ReviewAppAccount = { account: "hooli", state: "unknown", unreached: [] };

describe("which app posts a repository's reviews", () => {
  const all = r([acme, globex, initech, hooli]);

  it("is the review app where it's installed and reaches the repository", () => {
    expect(postsAs(all, repo("acme", "web"))).toBe("review");
    expect(postsAs(all, repo("globex", "web"))).toBe("review");
    expect(postsAs(all, repo("globex", "docs"))).toBe("github");
    expect(postsAs(all, repo("initech", "web"))).toBe("github");
    // GitHub didn't say, and an account the workspace doesn't work in gets no reviews.
    expect(postsAs(all, repo("hooli", "web"))).toBeUndefined();
    expect(postsAs(all, repo("umbrella", "web"))).toBe("github");
  });

  it("is the GitHub App without a review app", () => {
    expect(postsAs(r([], false), repo("acme", "web"))).toBe("github");
  });

  it("says where installing the review app would change it", () => {
    const repos = [repo("acme", "web"), repo("globex", "docs"), repo("initech", "a"), repo("initech", "b")];
    const elsewhere = postedAsGithubApp(all, repos);
    expect(elsewhere.repos.map((x) => `${x.owner}/${x.name}`)).toEqual([
      "globex/docs",
      "initech/a",
      "initech/b",
    ]);
    expect(elsewhere.accounts.map((a) => a.account)).toEqual(["globex", "initech"]);
    expect(postsAllAs(all, [repo("acme", "web"), repo("globex", "web")])).toBe("review");
    expect(postsAllAs(all, [repo("initech", "web")])).toBe("github");
    expect(postsAllAs(all, repos)).toBeUndefined();
    expect(postsAllAs(all, [repo("acme", "web"), repo("hooli", "web")])).toBeUndefined();
  });
});

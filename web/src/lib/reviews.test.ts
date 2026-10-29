import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  canAskForReview,
  canManageReviews,
  earlierFindings,
  pullRef,
  RISKS,
  repoList,
  reviewLook,
  reviewOutcome,
  reviewRepos,
  sameRepos,
  shortSha,
  summaryMarkdown,
} from "./reviews";

const review = (r: Partial<G.Review> = {}): G.Review => ({
  session_id: "ses_1" as G.SessionId,
  session_number: 45,
  status: "running",
  state: "running",
  repo: { owner: "acme", name: "web" },
  number: 12,
  url: "https://github.com/acme/web/pull/12",
  title: "Fix the login redirect",
  draft: false,
  head_sha: "3f2a1c9d00000000000000000000000000000000",
  findings: 0,
  resolved: 0,
  still_open: 0,
  operator: { id: "usr_1" as G.UserId, name: "Ada" },
  created_at: 1,
  busy_ms: 0,
  ...r,
});

describe("reviews", () => {
  it("say whether they wait, review or how they ended", () => {
    expect(reviewLook(review({ status: "queued" }))).toBe("waiting");
    expect(reviewOutcome(review({ status: "queued" }))).toBe("Waiting for a runner");
    expect(reviewLook(review({ status: "preparing" }))).toBe("reviewing");
    expect(reviewOutcome(review())).toBe("Reviewing");
    expect(reviewOutcome(review({ state: "passed", status: "stopped" }))).toBe("No issues");
    expect(reviewOutcome(review({ state: "commented", findings: 1 }))).toBe("1 issue");
    expect(reviewOutcome(review({ state: "commented", findings: 3 }))).toBe("3 issues");
    // Nothing new, with earlier findings still open.
    expect(reviewOutcome(review({ state: "commented", still_open: 2 }))).toBe("2 earlier issues open");
    expect(reviewOutcome(review({ state: "superseded" }))).toBe("Superseded");
    expect(reviewOutcome(review({ state: "stopped" }))).toBe("Stopped");
    expect(reviewOutcome(review({ state: "failed" }))).toBe("Failed");
  });

  it("count what they did with earlier findings", () => {
    expect(earlierFindings(review())).toEqual([]);
    expect(earlierFindings(review({ state: "passed", resolved: 1 }))).toEqual(["1 fixed"]);
    expect(earlierFindings(review({ state: "commented", findings: 2, still_open: 1, resolved: 2 }))).toEqual([
      "2 fixed",
      "1 still open",
    ]);
    // The outcome says it already.
    expect(earlierFindings(review({ state: "commented", still_open: 1 }))).toEqual([]);
  });

  it("name pull requests, commits and repositories briefly", () => {
    expect(pullRef(review())).toBe("web#12");
    expect(shortSha(review().head_sha)).toBe("3f2a1c9");
    const repo = (name: string): G.RepoRef => ({ owner: "acme", name });
    expect(repoList([repo("web")])).toBe("acme/web");
    expect(repoList([repo("web"), repo("api")])).toBe("acme/web and acme/api");
    expect(repoList([repo("web"), repo("api"), repo("docs")])).toBe("acme/web, acme/api and acme/docs");
    expect(repoList([repo("a"), repo("b"), repo("c"), repo("d")])).toBe("acme/a, acme/b and 2 more");
  });

  it("list the repositories they review or skip as the project does", () => {
    const repo = (name: string): G.RepoRef => ({ owner: "acme", name });
    // The primary repository first, then the others by name, whether they're reviewed or skipped.
    const agents = { repos: [repo("web"), repo("api"), repo("Docs")], skipped_repos: [repo("billing")] };
    expect(reviewRepos(agents, repo("web")).map((r) => r.name)).toEqual(["web", "api", "billing", "Docs"]);
    expect(
      reviewRepos({ repos: [repo("web")], skipped_repos: [repo("api")] }, { owner: "ACME", name: "API" }),
    ).toEqual([repo("api"), repo("web")]);
    expect(reviewRepos({ repos: [], skipped_repos: [] })).toEqual([]);
    expect(sameRepos(["acme/web", "acme/api"], ["acme/API", "acme/web"])).toBe(true);
    expect(sameRepos(["acme/web"], ["acme/web", "acme/api"])).toBe(false);
    expect(sameRepos([], [])).toBe(true);
  });

  it("are the owners' to turn on, and editors' to ask for", () => {
    expect(canManageReviews("owner")).toBe(true);
    expect(canManageReviews("maintainer")).toBe(false);
    expect(canAskForReview("editor")).toBe(true);
    expect(canAskForReview("viewer")).toBe(false);
    expect(canAskForReview(undefined)).toBe(false);
  });

  it("sum pull requests up as the note in their description reads", () => {
    const summary: G.PullRequestSummary = {
      risk: "high",
      risk_reason: "It changes who may delete projects.",
      overview: "Lets maintainers delete projects.\n\nOwners get an email when one goes.",
      highlight: { topic: "Data loss", text: "A deleted project's sessions go with it." },
    };
    expect(summaryMarkdown(summary)).toBe(
      "**High risk.** It changes who may delete projects.\n\n**What it does.** Lets maintainers delete projects.\n\nOwners get an email when one goes.\n\n**Data loss.** A deleted project's sessions go with it.",
    );
    expect(
      summaryMarkdown({
        ...summary,
        overview: "Lets maintainers delete projects.",
        highlight: { topic: "Why now?", text: "Owners asked." },
      }),
    ).toBe(
      "**High risk.** It changes who may delete projects.\n\n**What it does.** Lets maintainers delete projects.\n\n**Why now?** Owners asked.",
    );
    expect(Object.entries(RISKS).map(([risk, r]) => `${risk}: ${r.alert}`)).toEqual([
      "low: note",
      "medium: note",
      "high: warning",
      "critical: caution",
    ]);
  });
});

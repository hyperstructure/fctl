import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  checksMark,
  checksState,
  checksText,
  githubLink,
  prFullRef,
  prLabel,
  prRef,
  prReview,
  prReviewLook,
  prReviewText,
  prState,
  prStateText,
  prTitle,
  pullRequests,
  reviewIssues,
} from "./pulls";

const pr = (repo: string | undefined, number: number, title?: string): G.PullRequestRef => {
  const [owner = "", name = ""] = repo?.split("/") ?? [];
  return {
    number,
    url: `https://github.com/${repo ?? "acme/app"}/pull/${number}`,
    title,
    repo: repo ? { owner, name } : undefined,
  };
};

const NOW = Date.UTC(2026, 9, 21, 16, 0);
const HOUR = 3_600_000;
const checks = (passed: number, failed: number, pending: number): G.PullRequestChecks => ({
  passed,
  failed,
  pending,
  at: NOW,
});

describe("pullRequests", () => {
  it("lists every one, or the one a control plane before the list has", () => {
    const all = [pr("acme/app", 12), pr("acme/lib", 3)];
    expect(pullRequests({ pull_request: all[0], pull_requests: all })).toEqual(all);
    expect(pullRequests({ pull_request: all[0] })).toEqual([all[0]]);
    expect(pullRequests({})).toEqual([]);
  });

  it("lists only GitHub's own links", () => {
    const real = pr("acme/app", 12);
    const forged = [
      { ...real, url: "https://evil.example/acme/app/pull/12" },
      { ...real, url: "https://github.com.evil.example/acme/app/pull/12" },
      { ...real, url: "javascript:alert(1)//github.com/acme/app/pull/12" },
      { ...real, url: "https://github.com/acme/app/pull/12/../../../../evil" },
    ];
    expect(pullRequests({ pull_request: real, pull_requests: [real, ...forged] })).toEqual([real]);
    expect(pullRequests({ pull_request: forged[0] })).toEqual([]);
  });
});

describe("naming pull requests", () => {
  it("says the repository of each", () => {
    const app = pr("hyperstructure/fctl", 120, "Give freed space back");
    expect(prRef(app)).toBe("fctl#120");
    expect(prFullRef(app)).toBe("hyperstructure/fctl#120");
    expect(prTitle(app)).toBe("hyperstructure/fctl#120: Give freed space back");
    const unknown = pr(undefined, 7);
    expect(prRef(unknown)).toBe("#7");
    expect(prTitle(unknown)).toBe("#7");
  });
});

describe("a pull request's state", () => {
  it("is GitHub's, with the merge queue's names for queued, and unknown until someone looked", () => {
    const states = ["open", "draft", "queued", "in_merge_queue", "merged", "closed", undefined, "something"];
    expect(states.map((state) => prState({ ...pr("acme/app", 1), state }))).toEqual([
      "open",
      "draft",
      "queued",
      "queued",
      "merged",
      "closed",
      "unknown",
      "unknown",
    ]);
  });

  it("says when it was merged, closed or queued", () => {
    const at = (state: string, hours?: number) =>
      prStateText({ ...pr("acme/app", 1), state, state_changed_at: hours && NOW - hours * HOUR }, NOW);
    expect(at("merged", 2)).toBe("Merged 2 hours ago");
    expect(at("closed", 30)).toBe("Closed yesterday");
    expect(at("queued", 1)).toBe("Queued 1 hour ago");
    // Opened, or marked ready: GitHub shows when it was opened, which says less.
    expect(at("open", 50)).toBe("Open");
    expect(at("draft")).toBe("Draft");
    expect(at("merged")).toBe("Merged");
    expect(prStateText(pr("acme/app", 1))).toBe("Unknown");
  });
});

describe("a pull request's checks", () => {
  it("passed when every one did, failed when one did, and are pending meanwhile", () => {
    const of = (c: G.PullRequestChecks, state = "open") => {
      const p = { ...pr("acme/app", 1), state, checks: c };
      return [checksState(p), checksText(p)];
    };
    expect(of(checks(5, 0, 0))).toEqual(["passed", "All 5 checks passed"]);
    expect(of(checks(1, 0, 0))).toEqual(["passed", "1 check passed"]);
    expect(of(checks(2, 1, 1))).toEqual(["failed", "1 of 4 checks failed"]);
    expect(of(checks(0, 2, 0))).toEqual(["failed", "2 checks failed"]);
    expect(of(checks(3, 0, 2))).toEqual(["pending", "2 of 5 checks pending"]);
    expect(of(checks(0, 0, 1), "queued")).toEqual(["pending", "1 check pending"]);
    expect(of(checks(0, 0, 0))).toEqual([undefined, undefined]);
    // Once it's merged or closed, they no longer matter.
    expect(of(checks(2, 1, 0), "merged")).toEqual([undefined, undefined]);
    expect(of(checks(2, 1, 0), "closed")).toEqual([undefined, undefined]);
    expect(checksState(pr("acme/app", 1))).toBeUndefined();
  });

  it("show in its tooltip and in what screen readers say of its link", () => {
    const open: G.PullRequestRef = {
      ...pr("hyperstructure/fctl", 171, "Show pull requests' state"),
      state: "open",
      checks: checks(3, 1, 0),
    };
    expect(prTitle(open)).toBe(
      "hyperstructure/fctl#171: Show pull requests' state\nOpen · 1 of 4 checks failed",
    );
    expect(prLabel(open)).toBe("fctl#171, open, 1 of 4 checks failed");
    const merged = { ...open, state: "merged", state_changed_at: NOW - 2 * HOUR };
    expect(prTitle(merged)).toMatch(/^hyperstructure\/fctl#171: Show pull requests' state\nMerged .+2026/);
    expect(prLabel(merged, NOW)).toBe("fctl#171, merged 2 hours ago");
    expect(prLabel(pr("acme/app", 7))).toBe("app#7");
  });
});

describe("fctl's review of a pull request", () => {
  const review = (state: G.ReviewState, more: Partial<G.ReviewCheck> = {}): G.ReviewCheck => ({
    state,
    findings: 0,
    still_open: 0,
    session_id: "ses_review",
    ...more,
  });
  const reviewed = (r: G.ReviewCheck, c = checks(9, 0, 0), state = "open"): G.PullRequestRef => ({
    ...pr("hyperstructure/fctl", 302, "Choose a project's repositories from a list"),
    state,
    checks: { ...c, review: r },
  });

  it("says how it stands, as fctl show says it", () => {
    const said = (r: G.ReviewCheck) => [prReviewLook(r), prReviewText(r)];
    expect(said(review("running", { waiting: true }))).toEqual([
      "waiting",
      "fctl's review agent waits for a runner",
    ]);
    expect(said(review("running"))).toEqual(["reviewing", "fctl's review agent is reviewing it"]);
    expect(said(review("passed"))).toEqual(["passed", "fctl's review agent found no issues"]);
    expect(said(review("commented", { findings: 1, still_open: 2 }))).toEqual([
      "commented",
      "fctl's review agent found 1 issue",
    ]);
    expect(said(review("commented", { findings: 3 }))[1]).toBe("fctl's review agent found 3 issues");
    expect(said(review("commented", { still_open: 1 }))[1]).toBe(
      "1 issue from fctl's earlier reviews is still open",
    );
    expect(said(review("commented", { still_open: 2 }))[1]).toBe(
      "2 issues from fctl's earlier reviews are still open",
    );
    expect(said(review("commented"))[1]).toBe("fctl's review agent found issues");
    expect(said(review("stopped"))).toEqual(["stopped", "fctl's review stopped"]);
    expect(said(review("superseded"))[1]).toBe("fctl's review stopped");
    expect(said(review("failed"))).toEqual(["failed", "fctl's review failed"]);
  });

  it("counts the issues it found, else those still open", () => {
    expect(reviewIssues(review("commented", { findings: 3, still_open: 2 }))).toBe(3);
    expect(reviewIssues(review("commented", { still_open: 2 }))).toBe(2);
  });

  it("matters while its checks do, apart from them", () => {
    const found = reviewed(review("commented", { findings: 2 }));
    expect(prReview(found)?.findings).toBe(2);
    // The other checks: its own isn't counted among them.
    expect([checksState(found), checksText(found)]).toEqual(["passed", "All 9 checks passed"]);
    expect(prReview({ ...found, state: "merged" })).toBeUndefined();
    expect(prReview(pr("acme/app", 1))).toBeUndefined();
  });

  it("passes the checks' mark where nothing else checks the pull request", () => {
    const none = checks(0, 0, 0);
    expect(checksMark(reviewed(review("passed"), none))).toBe("passed");
    expect(checksMark(reviewed(review("commented", { findings: 1 }), none))).toBeUndefined();
    expect(checksMark(reviewed(review("running"), none))).toBeUndefined();
    // The other checks' state comes first.
    expect(checksMark(reviewed(review("passed"), checks(3, 0, 1)))).toBe("pending");
    expect(checksMark(reviewed(review("commented"), checks(3, 1, 0)))).toBe("failed");
  });

  it("shows in its tooltip and in what screen readers say of its link", () => {
    const found = reviewed(review("commented", { findings: 2 }));
    expect(prTitle(found)).toBe(
      "hyperstructure/fctl#302: Choose a project's repositories from a list\nOpen · All 9 checks passed · fctl's review agent found 2 issues",
    );
    expect(prLabel(found)).toBe("fctl#302, open, all 9 checks passed, fctl's review agent found 2 issues");
    const running = reviewed(review("running"), checks(5, 0, 2));
    expect(prLabel(running)).toBe(
      "fctl#302, open, 2 of 7 checks pending, fctl's review agent is reviewing it",
    );
    // Without other checks.
    expect(prLabel(reviewed(review("running", { waiting: true }), checks(0, 0, 0), "draft"))).toBe(
      "fctl#302, draft, fctl's review agent waits for a runner",
    );
  });
});

describe("githubLink", () => {
  it("keeps GitHub's pages and refuses links anywhere else", () => {
    for (const ok of [
      "https://github.com/acme/web/pull/12",
      "https://github.com/acme/web/pull/12#issuecomment-345",
      "https://github.com/acme/web/pull/12/commits/3f2a1c9",
    ])
      expect(githubLink(ok)).toBe(ok);
    for (const bad of [
      undefined,
      "",
      "https://evil.example/login",
      "https://github.com.evil.example/acme",
      "https://github.com",
      "https://github.com\\@evil.example/",
      "http://github.com/acme/web/pull/12",
      "javascript:alert(1)//https://github.com/",
      "vscode://file/etc/passwd",
      "https://github.com/acme web",
    ])
      expect(githubLink(bad)).toBeUndefined();
  });
});

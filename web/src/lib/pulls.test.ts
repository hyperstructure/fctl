import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  checksState,
  checksText,
  prFullRef,
  prLabel,
  prRef,
  prState,
  prStateText,
  prTitle,
  pullRequests,
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

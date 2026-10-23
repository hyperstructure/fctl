import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  action,
  checksRest,
  followUpHint,
  followUpsSummary,
  happenedText,
  lines,
  parts,
  place,
  pullRef,
  verdict,
} from "./follow-ups";

const comment = (c: Partial<G.FollowUpComment> = {}): G.FollowUpComment => ({
  id: 1,
  login: "ada",
  url: "https://github.com/acme/web/pull/12#issuecomment-1",
  body: "@fctl keep the query string",
  ...c,
});

const followUp = (events: G.FollowUpEvent[]): G.FollowUp => ({
  repo: { owner: "acme", name: "web" },
  number: 12,
  url: "https://github.com/acme/web/pull/12",
  title: "Fix the login redirect",
  state: "open",
  base: "main",
  events,
});

const said = (c: G.FollowUpComment): G.FollowUpEvent => ({ kind: "comment", ...c });

describe("follow-ups", () => {
  it("tell where a person wrote and what they did", () => {
    expect(place(comment())).toBe("conversation");
    expect(place(comment({ path: "src/login.rs" }))).toBe("lines");
    expect(place(comment({ review: "approved", path: "x" }))).toBe("review");
    expect(action(comment())).toBe("commented on");
    expect(action(comment({ path: "src/login.rs" }))).toBe("commented on");
    expect(action(comment({ path: "src/login.rs", in_reply_to: 9 }))).toBe("replied on");
    expect(action(comment({ review: "commented" }))).toBe("reviewed");
    expect(verdict(comment({ review: "approved" }))).toBe("Approved");
    expect(verdict(comment({ review: "changes_requested" }))).toBe("Changes requested");
    expect(verdict(comment({ review: "commented" }))).toBeUndefined();
    expect(lines(comment({ path: "src/login.rs", line: 42 }))).toBe("src/login.rs:42");
    expect(lines(comment({ path: "src/login.rs", line: 42, start_line: 40 }))).toBe("src/login.rs:40–42");
    expect(lines(comment({ path: "src/login.rs", line: 42, start_line: 42 }))).toBe("src/login.rs:42");
    expect(lines(comment({ path: "README.md" }))).toBe("README.md");
    expect(pullRef(followUp([]))).toBe("web#12");
  });

  it("put a review's comments on lines under it", () => {
    const review = comment({ id: 7, review: "changes_requested", body: "@fctl see below" });
    const mine = comment({ id: 8, path: "a.rs", line: 1, review_id: 7 });
    const other = comment({ id: 9, path: "b.rs", line: 2, review_id: 5 });
    const f = followUp([
      said(review),
      said(mine),
      said(other),
      { kind: "checks_failed", sha: "abc", names: ["Lint"], passed: 3, failed: 1, pending: 0 },
      { kind: "other" },
    ]);
    const { wrote, happened } = parts(f);
    expect(wrote.map((w) => [w.comment.id, w.lines.map((l) => l.id)])).toEqual([
      [7, [8]],
      [9, []],
    ]);
    expect(happened.map((e) => e.kind)).toEqual(["checks_failed"]);
  });

  it("say what else happened in a few words", () => {
    const f = followUp([]);
    const checks = (names: string[], passed: number, pending: number) =>
      ({ kind: "checks_failed", sha: "abc", names, passed, failed: names.length, pending }) as const;
    expect(happenedText(f, checks(["Lint"], 0, 0))).toBe("A check failed");
    expect(happenedText(f, checks(["Lint", "Rust tests"], 0, 0))).toBe("2 checks failed");
    expect(checksRest(checks(["Lint"], 7, 1))).toBe("1 still running · 7 passed");
    expect(checksRest(checks(["Lint"], 0, 0))).toBe("");
    expect(happenedText(f, { kind: "merged", by: "ada" })).toBe("Merged into main by ada");
    expect(happenedText({ ...f, base: undefined }, { kind: "merged" })).toBe("Merged");
    expect(happenedText(f, { kind: "closed", by: "bob" })).toBe("Closed without merging by bob");
  });

  it("say on a session's page what brings its agent back", () => {
    const on = (comments: boolean, checks: boolean, closed: boolean) =>
      followUpHint({ comments, checks, closed });
    expect(followUpHint(undefined)).toBeUndefined();
    expect(on(false, false, false)).toBeUndefined();
    expect(on(true, false, false)).toBe("Mention @fctl or @agent on GitHub to bring the agent back.");
    expect(on(true, true, true)).toBe(
      "Mention @fctl or @agent on GitHub to bring the agent back; failed checks and merges do too.",
    );
    expect(on(false, true, false)).toBe("Failed checks bring the agent back.");
    expect(on(false, true, true)).toBe("Failed checks and merges bring the agent back.");
    expect(on(false, false, true)).toBe("A merge brings the agent back.");
  });

  it("sum up what brings agents back", () => {
    expect(followUpsSummary({ comments: false, checks: false, closed: false })).toBe("Off");
    expect(followUpsSummary({ comments: true, checks: false, closed: false })).toBe("Comments");
    expect(followUpsSummary({ comments: false, checks: true, closed: true })).toBe(
      "Failed checks and merges",
    );
    expect(followUpsSummary({ comments: true, checks: true, closed: true })).toBe(
      "Comments, failed checks and merges",
    );
  });
});

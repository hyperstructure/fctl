import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { needSays, needsOf } from "./needs";

const NOW = Date.UTC(2026, 10, 7, 8, 10);
let n = 0;
function session(more: Partial<G.Session> = {}): G.Session {
  n++;
  return {
    id: `ses_${n}`,
    project_id: "prj_shop",
    project_slug: "shop",
    number: n,
    title: `Session ${n}`,
    status: "running",
    created_by: { id: "usr_noor", name: "Noor Haddad" },
    created_at: n,
    updated_at: n,
    stats: {} as G.SessionStats,
    archived: false,
    ...more,
  } as G.Session;
}
const pr = (number: number, more: Partial<G.PullRequestRef> = {}): G.PullRequestRef => ({
  number,
  url: `https://github.com/acme/shop/pull/${number}`,
  state: "open",
  ...more,
});

describe("what needs you", () => {
  it("puts questions first, then failures, failed checks, finished pull requests, replies and pauses", () => {
    const asking = session({
      status: "idle",
      status_reason: "Waiting for a message",
      question: { text: "Keep øre?", asked_at: NOW - 60_000 },
    });
    const failed = session({ status: "failed", ended_at: NOW - 3_600_000 });
    const checks = session({
      status: "idle",
      pull_requests: [pr(1, { checks: { passed: 1, failed: 1, pending: 0 } as G.PullRequestChecks })],
    });
    const ready = session({ status: "stopped", ended_at: NOW - 5 * 3_600_000, pull_requests: [pr(843)] });
    const waiting = session({ status: "idle", status_reason: "Waiting for a message" });
    const paused = session({ status: "paused" });
    const working = session({ status: "running", pull_requests: [pr(9)] });
    const needs = needsOf("usr_noor", [paused, waiting, ready, checks, failed, asking, working], NOW);
    expect(needs.map((x) => [x.s.id, x.kind])).toEqual([
      [asking.id, "asking"],
      [failed.id, "failed"],
      [checks.id, "checks"],
      [ready.id, "ready"],
      [waiting.id, "waiting"],
      [paused.id, "paused"],
    ]);
    expect(needs.map(needSays)).toEqual([
      "Keep øre?",
      "Failed",
      "Checks failed on #1",
      "#843 is ready for you",
      "Waiting for your reply",
      "Paused on a usage limit",
    ]);
  });

  it("waits for no reply once its pull requests are all merged or closed", () => {
    const idle = { status: "idle", status_reason: "Waiting for a message" } as const;
    const merged = session({ ...idle, pull_requests: [pr(452, { state: "merged" })] });
    const closed = session({
      ...idle,
      pull_requests: [pr(3, { state: "merged" }), pr(4, { state: "closed" })],
    });
    const draft = session({
      ...idle,
      pull_requests: [pr(5, { state: "merged" }), pr(6, { state: "draft" })],
    });
    const unknown = session({
      ...idle,
      pull_requests: [pr(7, { state: "merged" }), pr(8, { state: undefined })],
    });
    const open = session({ ...idle, pull_requests: [pr(9, { state: "merged" }), pr(10)] });
    const none = session(idle);
    expect(
      needsOf("usr_noor", [merged, closed, draft, unknown, open, none], NOW).map((x) => [x.s.id, x.kind]),
    ).toEqual([
      [open.id, "ready"],
      [draft.id, "waiting"],
      [unknown.id, "waiting"],
      [none.id, "waiting"],
    ]);
  });

  it("is about their sessions and those they follow, once each, as they are now", () => {
    const grace = { id: "usr_grace", name: "Grace" };
    const theirs = session({ status: "idle", status_reason: "Waiting for a message", created_by: grace });
    const followed = { ...theirs, id: "ses_followed", following: true };
    const before = session({ status: "idle", status_reason: "Waiting for a message" });
    const after = { ...before, status: "running" as const, updated_at: before.updated_at + 1 };
    expect(needsOf("usr_noor", [theirs, followed, before, after], NOW).map((x) => x.s.id)).toEqual([
      "ses_followed",
    ]);
  });

  it("is nothing until it is known who you are, since the lists hold other people's sessions", () => {
    const asking = session({ status: "idle", question: { text: "Keep øre?", asked_at: NOW } });
    expect(needsOf(undefined, [asking], NOW)).toEqual([]);
    expect(needsOf("usr_noor", [asking], NOW)).toHaveLength(1);
  });

  it("leaves out drafts, archived sessions, review agents' and failures from before yesterday", () => {
    const draft = session({ status: "stopped", pull_requests: [pr(2, { state: "draft" })] });
    const archived = session({ status: "idle", status_reason: "Waiting for a message", archived: true });
    const old = session({ status: "failed", ended_at: NOW - 2 * 86_400_000 });
    const review = session({
      status: "idle",
      status_reason: "Waiting for a message",
      review: {} as G.SessionReview,
    });
    expect(needsOf("usr_noor", [draft, archived, old, review], NOW)).toEqual([]);
  });
});

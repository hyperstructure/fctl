import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  askingOf,
  countStates,
  listPulls,
  needOf,
  peopleOf,
  pullMatches,
  sizeOf,
  standing,
  stateOf,
} from "./pull-list";

let n = 0;
function pr(number: number, more: Partial<G.PullRequestRef> = {}): G.PullRequestRef {
  return {
    number,
    url: `https://github.com/acme/shop/pull/${number}`,
    title: `Change ${number}`,
    state: "open",
    repo: { owner: "acme", name: "shop" },
    ...more,
  };
}

function session(prs: G.PullRequestRef[], more: Partial<G.Session> = {}): G.Session {
  n++;
  return {
    id: `ses_${n}`,
    project_id: "prj_shop",
    project_slug: "shop",
    number: n,
    title: `Session ${n}`,
    status: "stopped",
    created_by: { id: "usr_noor", name: "Noor Haddad" },
    created_at: n,
    updated_at: n,
    stats: {} as G.SessionStats,
    archived: false,
    pull_requests: prs,
    ...more,
  } as G.Session;
}

describe("the pull requests page's list", () => {
  it("lists each pull request once, as the session that heard of it last has it", () => {
    const later = session([pr(12, { state: "merged", updated_at: 30 })]);
    const earlier = session([pr(12, { updated_at: 10 }), pr(7, { updated_at: 20 })]);
    const list = listPulls([later, earlier]);
    expect(list.map((l) => l.pr.number)).toEqual([12, 7]);
    expect(list[0]?.pr.state).toBe("merged");
    expect(list[0]?.sessions.map((s) => s.id)).toEqual([later.id, earlier.id]);
  });

  it("says what each open one needs next", () => {
    const one = (p: G.PullRequestRef, s: Partial<G.Session> = {}) => {
      const [l] = listPulls([session([p], s)]);
      return l ? needOf(l) : undefined;
    };
    expect(one(pr(1))).toBe("ready");
    expect(one(pr(2), { status: "running" })).toBe("working");
    expect(one(pr(3, { state: "draft" }))).toBe("draft");
    expect(one(pr(4, { state: "queued" }))).toBe("queued");
    expect(
      one(pr(5, { checks: { passed: 3, failed: 1, pending: 0 } as G.PullRequestChecks }), {
        status: "running",
      }),
    ).toBe("failing");
  });

  it("puts one whose agent waits for an answer before the rest, whatever else it needs", () => {
    const question = { text: "Keep øre?", options: ["Keep øre", "Round"], asked_at: 5 };
    const failing = pr(6, { checks: { passed: 3, failed: 1, pending: 0 } as G.PullRequestChecks });
    const [asks] = listPulls([session([failing], { status: "idle", question })]);
    if (!asks) throw new Error("listed");
    expect(needOf(asks)).toBe("asking");
    expect(askingOf(asks)?.question?.text).toBe("Keep øre?");
    // Answered, it is what it was.
    const [answered] = listPulls([session([failing], { status: "running" })]);
    if (!answered) throw new Error("listed");
    expect(needOf(answered)).toBe("failing");
    expect(askingOf(answered)).toBeUndefined();
    // What its agent asks finds it too.
    expect(pullMatches(asks, "øre")).toBe(true);
  });

  it("knows its size where one session's branch is all of it", () => {
    const stats = { files_changed: 4, additions: 127, deletions: 4 } as G.SessionStats;
    const [alone] = listPulls([session([pr(20)], { stats })]);
    expect(alone && sizeOf(alone)).toEqual({ files: 4, additions: 127, deletions: 4 });
    // A session with pull requests in two repositories counts both's changes.
    const [shared] = listPulls([
      session([pr(21), pr(22, { url: "https://github.com/acme/web/pull/22" })], { stats }),
    ]);
    expect(shared && sizeOf(shared)).toBeUndefined();
    const [none] = listPulls([session([pr(23)])]);
    expect(none && sizeOf(none)).toBeUndefined();
  });

  it("names who started its sessions once each, newest first", () => {
    const asha = { id: "usr_asha", name: "Asha Patel" };
    const [l] = listPulls([
      session([pr(30, { updated_at: 3 })], { created_by: asha }),
      session([pr(30, { updated_at: 2 })]),
      session([pr(30, { updated_at: 1 })], { created_by: asha }),
    ]);
    expect(l && peopleOf(l).map((p) => p.name)).toEqual(["Asha Patel", "Noor Haddad"]);
  });

  it("tells when what the page shows of one changes", () => {
    const [before] = listPulls([
      session([pr(40, { checks: { passed: 1, failed: 0, pending: 2 } as G.PullRequestChecks })]),
    ]);
    const [after] = listPulls([
      session([pr(40, { checks: { passed: 3, failed: 0, pending: 0 } as G.PullRequestChecks })]),
    ]);
    const [merged] = listPulls([session([pr(40, { state: "merged" })])]);
    if (!before || !after || !merged) throw new Error("listed");
    expect(standing(before)).not.toBe(standing(after));
    expect(standing(after)).not.toBe(standing(merged));
    expect(standing(after)).toBe(standing({ ...after, sessions: [...after.sessions] }));
  });

  it("counts them by state, and nobody's look-up counts as open", () => {
    const list = listPulls([
      session([pr(1), pr(2, { state: "merged" }), pr(3, { state: "closed" }), pr(4, { state: undefined })]),
    ]);
    expect(countStates(list)).toEqual({ open: 2, merged: 1, closed: 1 });
    expect(list.filter((l) => stateOf(l) === "open").map((l) => l.pr.number)).toEqual([1, 4]);
  });

  it("filters by title, repository, number and session", () => {
    const [l] = listPulls([
      session([pr(843, { title: "Plan routes with time windows" })], { title: "Plan routes" }),
    ]);
    if (!l) throw new Error("listed");
    expect(pullMatches(l, "routes")).toBe(true);
    expect(pullMatches(l, "acme/shop")).toBe(true);
    expect(pullMatches(l, "#843")).toBe(true);
    // As its row shows it, and as its link says it.
    expect(pullMatches(l, "acme/shop#843")).toBe(true);
    expect(pullMatches(l, "shop#843")).toBe(true);
    expect(pullMatches(l, "https://github.com/acme/shop/pull/843")).toBe(true);
    expect(pullMatches(l, "acme/shop#844")).toBe(false);
    expect(pullMatches(l, "Noor")).toBe(true);
    expect(pullMatches(l, "routes payments")).toBe(false);
    expect(pullMatches(l, "  ")).toBe(true);
  });
});

import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { countStates, listPulls, needOf, pullMatches, stateOf } from "./pull-list";

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
    expect(pullMatches(l, "Noor")).toBe(true);
    expect(pullMatches(l, "routes payments")).toBe(false);
    expect(pullMatches(l, "  ")).toBe(true);
  });
});

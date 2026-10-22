import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { branchChanges, hasChanges, type RepoChanges, repoChanges, whereCommits } from "./git";

const file = (path: string, additions?: number, deletions?: number): G.FileChange => ({
  path,
  status: "modified",
  additions,
  deletions,
});

describe("repoChanges", () => {
  it("describes one repository as before", () => {
    const git: G.GitState = {
      ahead: 1,
      behind: 0,
      files: [file("a", 3, 1), file("b.png")],
      uncommitted: 0,
      pushed: true,
    };
    const [primary, ...rest] = repoChanges(git);
    expect(rest).toEqual([]);
    expect(primary).toMatchObject({ name: undefined, additions: 3, deletions: 1, ahead: 1, pushed: true });
  });

  it("keeps every commit in the sandbox when the agent doesn't push", () => {
    const git: G.GitState = {
      ahead: 2,
      behind: 0,
      files: [file("a", 1, 0)],
      uncommitted: 0,
      pushed: false,
      repos: [
        {
          owner: "acme",
          name: "lib",
          access: "write",
          ahead: 1,
          behind: 0,
          files: [],
          uncommitted: 0,
          pushed: false,
        },
      ],
    };
    const [primary, lib] = repoChanges(git, false);
    expect(primary && whereCommits(primary)).toEqual(["kept in the sandbox"]);
    expect(lib && whereCommits(lib)).toEqual(["kept in the sandbox"]);
    expect(repoChanges(git).map(whereCommits)).toEqual([["not pushed yet"], ["not pushed yet"]]);
  });

  it("lists the other repositories after the primary one", () => {
    const pr = { number: 7, url: "https://github.com/acme/lib/pull/7" };
    const git: G.GitState = {
      ahead: 0,
      behind: 0,
      files: [],
      uncommitted: 0,
      pushed: false,
      repos: [
        {
          owner: "acme",
          name: "lib",
          access: "write",
          ahead: 1,
          behind: 0,
          files: [file("x", 2, 0)],
          uncommitted: 0,
          pushed: true,
          pull_request: pr,
        },
        {
          owner: "acme",
          name: "docs",
          access: "read",
          ahead: 0,
          behind: 0,
          files: [],
          uncommitted: 0,
          pushed: false,
        },
      ],
    };
    const repos = repoChanges(git);
    expect(repos.map((r) => r.name)).toEqual([undefined, "acme/lib", "acme/docs"]);
    expect(repos.map(hasChanges)).toEqual([false, true, false]);
    // A pull request alone is no change: the session lists it on its own.
    const [, , docs] = repos;
    expect(docs && hasChanges({ ...docs, pr })).toBe(false);
    expect(repos[1]).toMatchObject({ readOnly: false, additions: 2, pr });
    expect(repos[2]?.readOnly).toBe(true);
  });
});

describe("whereCommits", () => {
  const repo = (r: Partial<RepoChanges>): RepoChanges => ({
    readOnly: false,
    files: [file("a", 1, 0)],
    additions: 1,
    deletions: 0,
    ahead: 1,
    pushed: false,
    ...r,
  });
  const pr = (state: string): G.PullRequestRef => ({
    number: 12,
    url: "https://github.com/acme/widgets/pull/12",
    state,
  });

  it("says whether the commits are pushed", () => {
    expect(whereCommits(repo({ pushed: true }))).toEqual(["pushed"]);
    expect(whereCommits(repo({}))).toEqual(["not pushed yet"]);
    expect(whereCommits(repo({ readOnly: true }))).toEqual(["kept in the sandbox"]);
    expect(whereCommits(repo({ ahead: 0 }))).toEqual([]);
    expect(whereCommits(repo({ pushed: true, pr: pr("open") }))).toEqual(["pushed"]);
  });

  it("says a merged branch is merged, and what came after", () => {
    // Its branch deleted, and the squashed commit still ahead of the base.
    expect(whereCommits(repo({ pr: pr("merged"), unmerged: 0 }))).toEqual(["#12 merged"]);
    // From an agent that couldn't count them.
    expect(whereCommits(repo({ pr: pr("merged") }))).toEqual(["#12 merged"]);
    expect(whereCommits(repo({ ahead: 2, pr: pr("merged"), unmerged: 1 }))).toEqual([
      "#12 merged",
      "1 commit since, not pushed",
    ]);
    expect(whereCommits(repo({ ahead: 3, pushed: true, pr: pr("merged"), unmerged: 2 }))).toEqual([
      "#12 merged",
      "2 commits since, pushed",
    ]);
  });
});

describe("branchChanges", () => {
  const state = (head: string, ahead: number, pushed: boolean, pr?: number): G.GitState => ({
    branch: "fctl/1",
    head,
    ahead,
    behind: 0,
    files: [],
    uncommitted: 0,
    pushed,
    pull_request: pr ? { number: pr, url: `https://github.com/acme/widgets/pull/${pr}` } : undefined,
  });

  it("finds commits, pushes and new pull requests, not edits", () => {
    const seen = new Set<string>();
    let before: G.GitState | undefined;
    const steps: [string, G.GitState][] = [
      ["a new session's branch", state("base", 0, false)],
      ["edits", { ...state("base", 0, false), files: [file("a.rs", 1, 0)], uncommitted: 1 }],
      ["a commit", state("c1", 1, false)],
      ["the same state again", state("c1", 1, false)],
      ["another commit, pushed", state("c2", 2, true)],
      ["a pull request", state("c2", 2, true, 12)],
      ["the same pull request", state("c2", 2, true, 12)],
      ["a lookup that found none", state("c2", 2, true)],
      ["the pull request again", state("c2", 2, true, 12)],
      ["a rebase", state("c3", 2, false)],
      ["a push", state("c3", 2, true)],
      ["a reset to the base", state("base", 0, false)],
    ];
    const changed = steps.map(([what, after]) => {
      const repos = branchChanges(before, after, seen);
      before = after;
      return [what, repos.length > 0];
    });
    expect(changed).toEqual([
      ["a new session's branch", false],
      ["edits", false],
      ["a commit", true],
      ["the same state again", false],
      ["another commit, pushed", true],
      ["a pull request", true],
      ["the same pull request", false],
      ["a lookup that found none", false],
      ["the pull request again", false],
      ["a rebase", true],
      ["a push", true],
      ["a reset to the base", false],
    ]);
    // The first state seen, of a branch with commits, shows them.
    expect(branchChanges(undefined, state("c3", 2, true), new Set())).toEqual([undefined]);
  });

  it("names the repositories that changed", () => {
    const lib = (head: string, ahead: number): G.RepoGitState => ({
      owner: "acme",
      name: "lib",
      access: "write",
      head,
      ahead,
      behind: 0,
      files: [],
      uncommitted: 0,
      pushed: false,
    });
    const docs: G.RepoGitState = {
      ...lib("d0", 0),
      name: "docs",
      access: "read",
      pull_request: { number: 3, url: "https://github.com/acme/docs/pull/3" },
    };
    const before = { ...state("c1", 1, false), repos: [lib("l0", 0)] };
    const after = { ...state("c1", 1, false), repos: [lib("l1", 1), docs] };
    expect(branchChanges(before, after, new Set())).toEqual(["acme/lib", "acme/docs"]);
  });
});

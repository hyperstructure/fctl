import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { hasChanges, repoChanges } from "./git";

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
    expect(repos[1]).toMatchObject({ readOnly: false, additions: 2, pr });
    expect(repos[2]?.readOnly).toBe(true);
  });
});

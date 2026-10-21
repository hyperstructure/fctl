import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { prFullRef, prRef, prTitle, pullRequests } from "./pulls";

const pr = (repo: string | undefined, number: number, title?: string): G.PullRequestRef => {
  const [owner = "", name = ""] = repo?.split("/") ?? [];
  return {
    number,
    url: `https://github.com/${repo ?? "acme/app"}/pull/${number}`,
    title,
    repo: repo ? { owner, name } : undefined,
  };
};

describe("pullRequests", () => {
  it("lists every one, or the one a control plane before the list has", () => {
    const all = [pr("acme/app", 12), pr("acme/lib", 3)];
    expect(pullRequests({ pull_request: all[0], pull_requests: all })).toEqual(all);
    expect(pullRequests({ pull_request: all[0] })).toEqual([all[0]]);
    expect(pullRequests({})).toEqual([]);
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

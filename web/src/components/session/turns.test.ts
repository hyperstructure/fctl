import { describe, expect, it } from "vitest";
import type { DiffGroup } from "./diff-files";
import { changeOf } from "./turns";

const file = (path: string) => ({
  path,
  full: `/home/agent/workspace/${path}`,
  lines: { added: 1, removed: 0 },
});
const groups: DiffGroup[] = [
  { files: [{ path: "src/a.ts", status: "modified" }] as DiffGroup["files"] },
  { repo: "acme/web", files: [{ path: "app/b.ts", status: "added" }] as DiffGroup["files"] },
];

describe("changeOf", () => {
  it("finds a file an edit changed among the session's changes, in its repository", () => {
    expect(changeOf(file("src/a.ts"), groups)).toEqual({ repo: undefined, path: "src/a.ts" });
    expect(changeOf({ ...file("x"), path: "~/repos/acme/web/app/b.ts" }, groups)).toEqual({
      repo: "acme/web",
      path: "app/b.ts",
    });
  });

  it("finds nothing for a file the changes don't have: reverted, or outside the repositories", () => {
    expect(changeOf(file("src/gone.ts"), groups)).toBeUndefined();
    expect(changeOf({ ...file("x"), path: "~/notes.md" }, groups)).toBeUndefined();
    expect(changeOf({ ...file("x"), path: "~/repos/acme/api/app/b.ts" }, groups)).toBeUndefined();
  });
});

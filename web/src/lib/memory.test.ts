import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { canEditMemory, headline, matches, ofSession, original, replacement } from "./memory";

const note = (over: Partial<G.Memory> = {}): G.Memory => ({
  id: "mem_1",
  project_id: "prj_1",
  text: "The e2e tests need Chromium.",
  created_at: 1,
  ...over,
});

describe("headline", () => {
  it("takes the first line with text, without Markdown's marks", () => {
    expect(headline("\n## Tests need npm ci\n\nRun it in web/ first.")).toBe("Tests need npm ci");
    expect(headline("- `cargo xtask check`   takes 4 minutes")).toBe("`cargo xtask check` takes 4 minutes");
    expect(headline("> quoted")).toBe("quoted");
  });
  it("cuts long lines with an ellipsis", () => {
    expect(headline("abcdefghij", 5)).toBe("abcd…");
    expect(headline("abcde", 5)).toBe("abcde");
    expect(headline("")).toBe("");
  });
});

describe("matches", () => {
  const m = note({
    text: "Deploys happen on Fridays.",
    session: { id: "ses_1", number: 12, title: "Fix the login redirect" },
    created_by: { id: "usr_1", name: "Ada" },
  });
  it("finds every word, in any order and case", () => {
    expect(matches(m, "")).toBe(true);
    expect(matches(m, "fridays deploys")).toBe(true);
    expect(matches(m, "#12")).toBe(true);
    expect(matches(m, "login ada")).toBe(true);
    expect(matches(m, "mondays")).toBe(false);
  });
});

describe("ofSession", () => {
  it("tells the notes a session wrote from those it marked stale", () => {
    const wrote = note({ id: "mem_a", session: { id: "ses_1", number: 1, title: "A" } });
    const staled = note({ id: "mem_b", stale: { at: 2, session: { id: "ses_1", number: 1, title: "A" } } });
    const other = note({ id: "mem_c", session: { id: "ses_2", number: 2, title: "B" } });
    const { added, staled: marked } = ofSession([wrote, staled, other], "ses_1");
    expect(added.map((m) => m.id)).toEqual(["mem_a"]);
    expect(marked.map((m) => m.id)).toEqual(["mem_b"]);
  });
});

describe("replacement", () => {
  it("finds the note that took a stale one's place", () => {
    const next = note({ id: "mem_new" });
    const old = note({ id: "mem_old", stale: { at: 2, replaced_by: "mem_new" } });
    expect(replacement([next, old], old)?.id).toBe("mem_new");
    expect(replacement([old], old)).toBeUndefined();
    expect(replacement([next], next)).toBeUndefined();
  });
});

describe("original", () => {
  it("finds the note a duplicate repeats", () => {
    const kept = note({ id: "mem_kept" });
    const copy = note({ id: "mem_copy", stale: { at: 2, duplicate_of: "mem_kept" } });
    expect(original([kept, copy], copy)?.id).toBe("mem_kept");
    expect(original([copy], copy)).toBeUndefined();
    expect(original([kept], kept)).toBeUndefined();
  });
});

describe("canEditMemory", () => {
  it("lets editors and up keep the memory", () => {
    expect(canEditMemory("editor")).toBe(true);
    expect(canEditMemory("owner")).toBe(true);
    expect(canEditMemory("viewer")).toBe(false);
    expect(canEditMemory(undefined)).toBe(false);
  });
});

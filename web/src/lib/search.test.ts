import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { around, landing, marked, said, searchable } from "./search";

const match = (m: Partial<G.SearchMatch>): G.SearchMatch => ({ kind: "reply", text: "", ranges: [], ...m });

describe("searching all sessions", () => {
  it("starts at three characters, or two of Chinese, Japanese or Korean", () => {
    expect(searchable("lo")).toBe(false);
    expect(searchable(" lo ")).toBe(false);
    expect(searchable("log")).toBe(true);
    expect(searchable("a b")).toBe(true);
    expect(searchable("検索")).toBe(true);
    expect(searchable("テ")).toBe(false);
    expect(searchable("데이")).toBe(true);
  });

  it("marks what it found in a snippet", () => {
    expect(marked("the login redirect", [[4, 9]])).toEqual([
      { text: "the ", hit: false },
      { text: "login", hit: true },
      { text: " redirect", hit: false },
    ]);
    // Out of order, overlapping or past the end: left as they are.
    expect(
      marked("abcabc", [
        [3, 6],
        [0, 3],
        [2, 4],
        [5, 9],
      ]),
    ).toEqual([
      { text: "abc", hit: true },
      { text: "abc", hit: true },
    ]);
    expect(marked("abc", [])).toEqual([{ text: "abc", hit: false }]);
  });

  it("shows a match early in a narrow line", () => {
    const text = "…things the agent said before it got to the login redirect, and after";
    const at = text.indexOf("login");
    const m = around(match({ text, ranges: [[at, at + 5]] }), 12);
    expect(m.text.startsWith("…")).toBe(true);
    const [[s, e] = [0, 0]] = m.ranges;
    expect(m.text.slice(s, e)).toBe("login");
    // From a word's start, at most 12 characters before the match.
    expect(s - 1).toBeLessThanOrEqual(12);
    expect(m.text[1]).toMatch(/\S/);
    expect(m.text[0]).toBe("…");
    // Early enough already: as it was.
    const early = match({ text: "the login", ranges: [[4, 9]] });
    expect(around(early, 12)).toBe(early);
  });

  it("says who said what it found", () => {
    const me = "usr_1" as G.UserId;
    const ada = { id: "usr_2" as G.UserId, name: "Ada" };
    expect(said(match({ kind: "message", author: { id: me, name: "Me" } }), me)).toBe("You");
    expect(said(match({ kind: "message", author: ada }), me)).toBe("Ada");
    expect(said(match({ kind: "message" }), me)).toBe("fctl");
    expect(said(match({ kind: "reply" }), me)).toBe("Agent");
    expect(said(match({ kind: "title" }), me)).toBe("Title");
  });

  it("opens a session at the newest message or reply it matched", () => {
    const hit = {
      session: {} as G.Session,
      count: 3,
      matches: [
        match({ kind: "title" }),
        match({ kind: "reply", seq: 40 }),
        match({ kind: "message", seq: 2 }),
      ],
    };
    expect(landing(hit, " redirect ")).toEqual({ find: "redirect", at: 40 });
    expect(landing({ ...hit, matches: [match({ kind: "title" })] }, "x")).toEqual({
      find: "x",
      at: undefined,
    });
  });
});

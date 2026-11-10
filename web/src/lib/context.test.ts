import { describe, expect, it } from "vitest";
import { COMPACT_CHOICES, compactChoice, compactGist, parseTokens, tokensText, validWindow } from "./context";

describe("the context size a project's sessions compact near", () => {
  it("reads sizes as people write them", () => {
    const read: [string, number][] = [
      ["400000", 400_000],
      ["400k", 400_000],
      ["400K", 400_000],
      [" 250k ", 250_000],
      ["1m", 1_000_000],
      ["1.5M", 1_500_000],
      ["0.25m", 250_000],
      [".5m", 500_000],
      ["400 000", 400_000],
      ["400,000", 400_000],
      ["2.5k", 2_500],
      ["1.23456m", 1_234_560],
    ];
    for (const [input, tokens] of read) expect(parseTokens(input), input).toBe(tokens);
    for (const input of ["", "k", "m", ".", "400.5", "abc", "4o0k", "-400k", "400kk", "1e6"])
      expect(parseTokens(input), input).toBeUndefined();
  });

  it("writes them back as they'd be typed", () => {
    expect(tokensText(300_000)).toBe("300K");
    expect(tokensText(1_000_000)).toBe("1M");
    expect(tokensText(123_456)).toBe("123456");
    for (const t of [300_000, 1_000_000, 123_456, 999_000]) expect(parseTokens(tokensText(t))).toBe(t);
  });

  it("offers a few sizes, and takes others between Claude's bounds", () => {
    expect(compactChoice(undefined)).toBe("");
    expect(compactChoice(null)).toBe("");
    expect(compactChoice(400_000)).toBe("400000");
    expect(compactChoice(300_000)).toBe("other");
    expect(COMPACT_CHOICES.every(validWindow)).toBe(true);
    expect([99_999, 1_000_001, 250_000.5].some(validWindow)).toBe(false);
  });

  it("says where sessions compact only where the project chose it", () => {
    expect(compactGist({})).toBeUndefined();
    expect(compactGist({ compact_window: 250_000 })).toBe("compacts near 250K");
  });
});

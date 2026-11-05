// @vitest-environment node
import { describe, expect, it } from "vitest";
import headers from "../../public/_headers?raw";
import { tokenize } from "./highlight";

/** The sources a directive of the app's Content Security Policy allows. */
function sources(directive: string): string[] {
  const policy = headers.match(/Content-Security-Policy:(.*)/)?.[1] ?? "";
  const found = policy
    .split(";")
    .map((d) => d.trim().split(/\s+/))
    .find((d) => d[0] === directive);
  return found?.slice(1) ?? [];
}

describe("syntax highlighting", () => {
  it("colors TypeScript, TSX and Rust", async () => {
    for (const [lang, code] of Object.entries({
      typescript: "const x: number = 1; // one",
      tsx: 'const a = <div className="x">{y}</div>;',
      rust: "fn main() { let x = 1; }",
    })) {
      const tokens = (await tokenize([code], lang))?.[0] ?? [];
      expect(tokens.map((t) => t.content).join(""), lang).toBe(code);
      expect(tokens.filter((t) => t.style?.["--shiki-dark"]).length, lang).toBeGreaterThan(1);
    }
  });

  // Shiki's engine is Oniguruma built to WebAssembly: browsers refuse to
  // compile it, and so to highlight anything, unless the policy allows it.
  it("may compile its WebAssembly under the app's Content Security Policy", () => {
    expect(sources("script-src")).toContain("'wasm-unsafe-eval'");
    expect(sources("script-src")).not.toContain("'unsafe-eval'");
  });
});

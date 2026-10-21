import { describe, expect, it } from "vitest";
import { appPath } from "./paths";

describe("a path to go on to", () => {
  it("stays in the app", () => {
    for (const p of ["/", "/settings", "/s/ses_1?x=1#y", "/invite", "/%5Cx"]) expect(appPath(p)).toBe(p);
  });

  it("goes home instead of leaving it", () => {
    for (const p of [
      undefined,
      "",
      "settings",
      "//evil.example",
      "/\\evil.example",
      "/\t/evil.example",
      "/\n/evil.example",
      "/ /evil.example",
      "/　/evil.example",
      "https://evil.example",
      "javascript:alert(1)",
    ])
      expect(appPath(p)).toBe("/");
  });
});

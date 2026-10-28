import { describe, expect, it } from "vitest";
import { areDefaults, DEFAULT_PULL_REQUEST_INSTRUCTIONS, shownInstructions } from "./pr-instructions";

describe("pull request instructions", () => {
  it("shows fctl's defaults until a project has its own", () => {
    expect(DEFAULT_PULL_REQUEST_INSTRUCTIONS).toContain("`gh pr checks 12 --watch --fail-fast`");
    expect(shownInstructions(null)).toBe(DEFAULT_PULL_REQUEST_INSTRUCTIONS);
    expect(shownInstructions(undefined)).toBe(DEFAULT_PULL_REQUEST_INSTRUCTIONS);
    expect(shownInstructions("Open pull requests as drafts.")).toBe("Open pull requests as drafts.");
    // Emptied, they tell agents nothing.
    expect(shownInstructions("")).toBe("");
  });

  it("knows the defaults as the control plane does", () => {
    expect(areDefaults(null)).toBe(true);
    expect(areDefaults(`\n${DEFAULT_PULL_REQUEST_INSTRUCTIONS.replace(/\n/g, "\r\n")}  `)).toBe(true);
    expect(areDefaults(`${DEFAULT_PULL_REQUEST_INSTRUCTIONS}\n- Open them as drafts.`)).toBe(false);
    expect(areDefaults("")).toBe(false);
  });
});

import { palette } from "@hyperstructure/design/brand";
import { describe, expect, it } from "vitest";
import { ownFill, WORKSPACE_COLORS, workspaceFill, workspaceInitials } from "./avatar";

describe("workspaceInitials", () => {
  it("takes the first letters of the first two words", () => {
    expect(workspaceInitials("hyperstructure")).toBe("H");
    expect(workspaceInitials("Side project")).toBe("SP");
    expect(workspaceInitials("Ada's lab and more")).toBe("AL");
    expect(workspaceInitials("ada-lovelace")).toBe("AL");
    expect(workspaceInitials("  ")).toBe("?");
    expect(workspaceInitials("Ölund verkstad")).toBe("ÖV");
  });
});

describe("a workspace's fill", () => {
  it("is a color of its own as chosen, with initials in the ink that reads on it", () => {
    // Blackish on the brand's blue and pink, as theirs.
    expect(ownFill("#007eff")).toEqual({ fill: "#007eff", ink: palette.blackish });
    expect(ownFill("#ff00d8").ink).toBe(palette.blackish);
    expect(ownFill("#ffffff").ink).toBe(palette.blackish);
    expect(ownFill("#1d3557").ink).toBe(palette.whiteish);
    expect(ownFill("#7b2cbf").ink).toBe(palette.whiteish);
  });

  it("is the brand color's without one, or with one that isn't a color", () => {
    expect(workspaceFill({ color: "pink" })).toBe(WORKSPACE_COLORS.pink);
    expect(workspaceFill({ color: "blue", hex: "#2A9D8F" }).fill).toBe("#2a9d8f");
    expect(workspaceFill({ color: "blue", hex: "teal" })).toBe(WORKSPACE_COLORS.blue);
  });
});

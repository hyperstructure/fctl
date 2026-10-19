import { describe, expect, it } from "vitest";
import { workspaceInitials } from "./avatar";

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

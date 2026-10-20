import { describe, expect, it } from "vitest";
import { slugOf } from "./new-workspace";

describe("slugOf", () => {
  it("makes an address of the name as the server does", () => {
    expect(slugOf("Side project")).toBe("side-project");
    expect(slugOf("Ada's lab")).toBe("adas-lab");
    expect(slugOf("Ada’s lab")).toBe("adas-lab");
    expect(slugOf("Café Crème")).toBe("cafe-creme");
  });

  it("gives short and reserved names a word more", () => {
    expect(slugOf("X")).toBe("workspace-x");
    expect(slugOf("!!")).toBe("workspace");
    expect(slugOf("New")).toBe("new-workspace");
    expect(slugOf("Docs")).toBe("docs-workspace");
  });
});

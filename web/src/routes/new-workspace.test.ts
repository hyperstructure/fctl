import { describe, expect, it } from "vitest";
import { beforeEnding, slugOf } from "./new-workspace";

describe("slugOf", () => {
  it("makes an address of the name as the server does", () => {
    expect(slugOf("Side project")).toEqual({ slug: "side-project" });
    expect(slugOf("Ada's lab")).toEqual({ slug: "adas-lab" });
    expect(slugOf("Ada’s lab")).toEqual({ slug: "adas-lab" });
    expect(slugOf("Café Crème")).toEqual({ slug: "cafe-creme" });
  });

  it("gives short names a word more", () => {
    expect(slugOf("X")).toEqual({ slug: "workspace-x" });
    expect(slugOf("!!")).toEqual({ slug: "workspace" });
  });

  it("knows fctl's own addresses and yours get a random ending", () => {
    expect(slugOf("New")).toEqual({ slug: "new", ending: "reserved" });
    expect(slugOf("Docs")).toEqual({ slug: "docs", ending: "reserved" });
    const mine = new Set(["side-project"]);
    expect(slugOf("Side project", mine)).toEqual({ slug: "side-project", ending: "taken" });
    expect(slugOf("Side quest", mine)).toEqual({ slug: "side-quest" });
  });

  it("leaves room for the ending as the server does", () => {
    expect(beforeEnding("new")).toBe("new");
    expect(beforeEnding("a".repeat(48))).toBe("a".repeat(41));
    expect(beforeEnding(`${"a".repeat(40)}-bbbbbbb`)).toBe("a".repeat(40));
  });
});

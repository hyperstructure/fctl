import { describe, expect, it } from "vitest";
import { ROW, ROW_COVER } from "./list-row";

describe("list rows", () => {
  // Forced colors (Windows' contrast themes) drop shadows and fills, and keep outlines and borders.
  it("draw their focus ring and the lines between them as forced colors keep them", () => {
    expect(ROW_COVER).toContain("focus-visible:after:outline-2");
    expect(ROW_COVER).not.toContain("shadow");
    expect(ROW).toContain("before:border-t");
    expect(ROW).not.toContain("before:bg-");
  });
});

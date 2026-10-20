import { describe, expect, it } from "vitest";
import { centerSquare } from "./avatar-image";

describe("centerSquare", () => {
  it("takes the middle of wide and tall pictures", () => {
    expect(centerSquare(400, 200)).toEqual([100, 0, 200]);
    expect(centerSquare(200, 400)).toEqual([0, 100, 200]);
    expect(centerSquare(300, 300)).toEqual([0, 0, 300]);
    expect(centerSquare(301, 300)).toEqual([1, 0, 300]);
  });
});

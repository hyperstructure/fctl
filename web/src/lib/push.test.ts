import { describe, expect, it } from "vitest";
import { base64urlBytes, madeWith } from "./push";

describe("push", () => {
  it("decodes server keys", () => {
    expect([...base64urlBytes("BAUG")]).toEqual([4, 5, 6]);
    expect([...base64urlBytes("-_8")]).toEqual([251, 255]);
    expect(base64urlBytes("A".repeat(87)).length).toBe(65);
  });

  it("tells whether a subscription was made with a key", () => {
    const sub = (key: number[] | null) =>
      ({ options: { applicationServerKey: key ? new Uint8Array(key).buffer : null } }) as PushSubscription;
    expect(madeWith(sub([4, 5, 6]), "BAUG")).toBe(true);
    expect(madeWith(sub([4, 5, 7]), "BAUG")).toBe(false);
    expect(madeWith(sub([4, 5]), "BAUG")).toBe(false);
    expect(madeWith(sub(null), "BAUG")).toBe(false);
  });
});

import { describe, expect, it } from "vitest";
import { unchanged } from "./diff-play";

describe("where a line written over changed", () => {
  it("is the start of the first word that differs", () => {
    expect(unchanged("/// What it comes to.", "/// What it came to.")).toBe(12);
    expect(
      unchanged(
        "    if db.charge_for(s, p).await?.is_some() {",
        "    if !db.claim_charge_key(s, p).await? {",
      ),
    ).toBe(7);
    expect(
      unchanged(
        "    let charge = stripe::charge(s, amount).await?;",
        "    let charge = stripe::charge_with_key(s, amount, &key).await?;",
      ),
    ).toBe(25);
    expect(unchanged("Ok(charge.id)", "Ok(Charged::Now(charge.id))")).toBe(3);
  });

  it("counts a tab as four columns", () => {
    expect(unchanged("\tlet a = 1;", "\tlet a = 2;")).toBe(12);
  });

  it("is the whole line when nothing is the same, or nothing changed", () => {
    expect(unchanged("alpha", "beta")).toBe(0);
    expect(unchanged("same", "same")).toBe(0);
  });
});

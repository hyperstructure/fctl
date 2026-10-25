import { describe, expect, it } from "vitest";
import { feedingTime, planBake } from "./plan";

const bake = { id: 1, recipe: "Country loaf", readyAt: new Date("2026-03-14T17:00:00") };

describe("planBake", () => {
  it("works back from when the bread is ready", () => {
    const steps = planBake(bake);
    expect(steps[0]).toEqual({ what: "Feed the starter", at: new Date("2026-03-13T22:00:00") });
    expect(steps.at(-1)).toEqual({ what: "Bake", at: new Date("2026-03-14T16:00:00") });
  });

  it("plans rye on its own times", () => {
    expect(planBake({ ...bake, recipe: "Rye" })).toHaveLength(4);
  });

  it("falls back to the country loaf", () => {
    expect(planBake({ ...bake, recipe: "Something new" })).toHaveLength(6);
  });

  it("knows when to feed the starter", () => {
    expect(feedingTime(bake)).toEqual(new Date("2026-03-13T22:00:00"));
  });
});

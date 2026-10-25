/** A bake, worked back from when the bread should be ready. */
export interface Bake {
  id: number;
  recipe: string;
  readyAt: Date;
}

export interface Step {
  what: string;
  at: Date;
}

const HOUR = 3_600_000;

/** How many hours before the bread is ready each step starts. */
const RECIPES: Record<string, [string, number][]> = {
  "Country loaf": [
    ["Feed the starter", 19],
    ["Mix the dough", 8],
    ["Stretch and fold", 7.5],
    ["Shape", 3.5],
    ["Proof", 3],
    ["Bake", 1],
  ],
  Rye: [
    ["Feed the starter", 16],
    ["Mix the dough", 6],
    ["Shape", 3],
    ["Bake", 1.5],
  ],
};

export function planBake(bake: Bake): Step[] {
  const steps = RECIPES[bake.recipe] ?? RECIPES["Country loaf"];
  return steps.map(([what, hours]) => ({ what, at: new Date(bake.readyAt.getTime() - hours * HOUR) }));
}

/** When the starter has to be fed for `bake`. */
export function feedingTime(bake: Bake): Date {
  return planBake(bake)[0].at;
}

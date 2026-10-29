// What the task boxes say while they are empty: a line picked at random each time one shows,
// some plain, some playful.

/** When in the day `hour` (0 to 23, the browser's clock) falls, to ship something in. */
export function shipWhen(hour: number): string {
  if (hour >= 5 && hour < 11) return "this morning";
  if (hour >= 11 && hour < 14) return "over lunch";
  if (hour >= 14 && hour < 17) return "today";
  if (hour >= 17 && hour < 21) return "this evening";
  return "tonight";
}

/** The Agent Floor's lines, `hour` being the browser's. */
export function floorLines(hour: number): string[] {
  return [
    "What should an agent build next?",
    "What needs doing?",
    "Describe a task, and an agent picks it up",
    "What's next on the list?",
    `What should we ship ${shipWhen(hour)}?`,
    "What should an agent build while you get coffee?",
    "Your backlog called. It misses you.",
    "Hand it off and go touch grass",
    "What would you build with a hundred interns?",
    "The agents are bored. Give them something.",
    "Delegate the boring part",
    "Describe it like you would to a very fast intern",
    "What's worth shipping today?",
    "Make no mistakes.",
    "ultrathink about what to build next",
    "You're absolutely right! What should I build?",
    "Build me a startup. Make no mistakes.",
    "How many r's in “refactor”? Ask an agent anything.",
    "Anything but paperclips",
    "Vibe code something",
    "Rewrite it in Rust?",
    "Let's make the world go foom",
    "Make it a challenge, let's ship faster than poteto",
    "Delegate now and watch me go brrrrrrrrrr",
    "It works on my machine. Now make it work on yours.",
    "The basilisk is watching, build something good",
  ];
}

/** A project's lines, given its name. */
export const PROJECT_LINES: ((name: string) => string)[] = [
  (p) => `What should an agent do in ${p}?`,
  (p) => `What does ${p} need next?`,
  (p) => `What's broken in ${p}?`,
  (p) => `What should change in ${p}?`,
  (p) => `Pick an issue in ${p} and hand it off`,
  (p) => `What's the one thing ${p} keeps putting off?`,
  (p) => `Find the TODO in ${p} that's been there longest`,
  (p) => `What would make ${p} 1% better today?`,
  (p) => `What should ${p} stop doing?`,
  (p) => `Somewhere in ${p} there's a bug. Go get it.`,
  (p) => `Fix ${p}. Make no mistakes.`,
  (p) => `ultrathink about ${p}`,
  (p) => `Rewrite ${p} in Rust (again)`,
  (p) => `Add tests to ${p}. Real ones, this time.`,
  (p) => `Make ${p} pop`,
  (p) => `LGTM-proof ${p}`,
  (p) => `Refactor ${p} and make it 10x`,
];

/** One of `lines`, at random (`random` returns a number in [0, 1), as `Math.random` does). */
export function pick<T>(lines: readonly T[], random: () => number = Math.random): T {
  return lines[Math.min(Math.floor(random() * lines.length), lines.length - 1)] as T;
}

/** A line for the Agent Floor's task box, at random. */
export function floorPlaceholder(now = new Date(), random: () => number = Math.random): string {
  return pick(floorLines(now.getHours()), random);
}

/** A line for a project's task box, at random, to give the project's name. */
export function projectPlaceholder(random: () => number = Math.random): (name: string) => string {
  return pick(PROJECT_LINES, random);
}

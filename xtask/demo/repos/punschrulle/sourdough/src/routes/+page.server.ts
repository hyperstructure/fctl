import { upcomingBakes } from "$lib/server/db";
import { planBake } from "$lib/plan";

export function load() {
  const [next] = upcomingBakes();
  return { next: next ? { ...next, steps: planBake(next) } : null };
}

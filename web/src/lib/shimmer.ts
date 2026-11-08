// The shimmer of "working" labels (.text-shimmer in styles/app.css) passes over
// all of them at once. A CSS animation starts when its label appears, so a
// thought that comes under a run's line would shimmer out of step with it;
// each one starts as if it had been there since the page opened instead. So do
// the new session page's signs of work: the waves through its dots
// (components/ui/dot-wave.tsx), the ring that turns around what the agent
// does now, and the comet along the thinking mark's trace (ui/icons.tsx).

const SYNCED = new Set(["text-shimmer", "dot-wave", "busy-ring", "thought-sweep"]);

/** Keeps the labels' shimmers, and the signs of work, in step. Returns what undoes it. */
export function syncShimmers(target: EventTarget = document) {
  const sync = (e: Event) => {
    const { animationName, pseudoElement } = e as AnimationEvent;
    if (!SYNCED.has(animationName) || !(e.target instanceof Element)) return;
    // A pseudo-element's animations are its element's only with those under it.
    for (const a of e.target.getAnimations(pseudoElement ? { subtree: true } : undefined)) {
      if (SYNCED.has((a as CSSAnimation).animationName)) a.startTime = 0;
    }
  };
  target.addEventListener("animationstart", sync);
  return () => target.removeEventListener("animationstart", sync);
}

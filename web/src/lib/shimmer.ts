// The shimmer of "working" labels (.text-shimmer in styles/app.css) passes over
// all of them at once. A CSS animation starts when its label appears, so a
// thought that comes under a run's line would shimmer out of step with it;
// each one starts as if it had been there since the page opened instead.

const SHIMMER = "text-shimmer";

/** Keeps the labels' shimmers in step. Returns what undoes it. */
export function syncShimmers(target: EventTarget = document) {
  const sync = (e: Event) => {
    if ((e as AnimationEvent).animationName !== SHIMMER || !(e.target instanceof Element)) return;
    for (const a of e.target.getAnimations()) {
      if ((a as CSSAnimation).animationName === SHIMMER) a.startTime = 0;
    }
  };
  target.addEventListener("animationstart", sync);
  return () => target.removeEventListener("animationstart", sync);
}

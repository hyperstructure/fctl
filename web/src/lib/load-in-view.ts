// Earlier activity loads by itself as the reader scrolls up to it, as in a
// chat: once the "Show earlier activity" button comes near the view, and again
// after each load that leaves it there, until the transcript fills the page.
// The button stays, to show the load under way and for a press. A load that
// brought nothing (the reader went offline) isn't tried again at once, which
// would repeat it as fast as it fails: the next waits for the reader to scroll
// away and back, or to press the button.

import { type RefObject, useEffect, useRef } from "react";

/**
 * How far (px) from the view the button starts a load, so that what it loads
 * is often in before the reader gets there.
 */
export const AHEAD = 400;

/**
 * Calls `load` as `button` comes within `AHEAD` px of the view, and again
 * once `loading` is over while it is still there. After a load that `failed`,
 * only once the button has left the view and come back. The button is there
 * as long as the component that calls this is.
 */
export function useLoadInView(
  button: RefObject<HTMLElement | null>,
  load: () => void,
  loading: boolean,
  failed: boolean,
) {
  const latest = useRef(load);
  latest.current = load;
  useEffect(() => {
    const el = button.current;
    if (!el || loading) return;
    // A new observer reports where the button is first: after a load, that's
    // whether it is still in view.
    let ready = !failed;
    const near = new IntersectionObserver(
      (entries) => {
        if (!entries.at(-1)?.isIntersecting) ready = true;
        else if (ready) latest.current();
      },
      { rootMargin: `${AHEAD}px 0px` },
    );
    near.observe(el);
    return () => near.disconnect();
  }, [button, loading, failed]);
}

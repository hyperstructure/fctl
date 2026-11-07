import { useEffect, useState } from "react";

/** The longest setTimeout waits, 2^31 - 1 ms (about 24 days): it fires at once for longer. */
const LONGEST_TIMEOUT = 2 ** 31 - 1;

/**
 * Renders again once `at` (ms since the epoch) has passed, so that what depends on the time shows
 * anew then. Waits longer than a timer can take several timers.
 */
export function useTickAt(at: number | undefined) {
  const [, tick] = useState(0);
  useEffect(() => {
    if (at === undefined || at <= Date.now()) return;
    let id: ReturnType<typeof setTimeout> | undefined;
    const wait = () => {
      const left = at - Date.now();
      if (left <= 0) tick((n) => n + 1);
      else id = setTimeout(wait, Math.min(left + 250, LONGEST_TIMEOUT));
    };
    wait();
    return () => clearTimeout(id);
  }, [at]);
}

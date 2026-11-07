// What needs the person, where they see it without looking at the app: a count before the page's
// title ("(2) fctl"), and on the app's icon where it is installed (the Badging API). Pages set
// their own title through `setPageTitle`, which keeps the count in front of it.

import { useEffect } from "react";

let base = "fctl";
let count = 0;

function apply() {
  document.title = count > 0 ? `(${count}) ${base}` : base;
}

/** The page's own title; none for fctl's. */
export function setPageTitle(title: string | undefined) {
  base = title || "fctl";
  apply();
}

type Badging = Navigator & {
  setAppBadge?: (n?: number) => Promise<void>;
  clearAppBadge?: () => Promise<void>;
};

/** Shows `n` things that need the person, in the title and on the app's icon (none: clears them). */
export function useAttention(n: number) {
  useEffect(() => {
    count = n;
    apply();
    const nav = navigator as Badging;
    const done = n > 0 ? nav.setAppBadge?.(n) : nav.clearAppBadge?.();
    // Browsers without the API, or that refuse it (not installed), say nothing.
    done?.catch(() => {});
  }, [n]);
}

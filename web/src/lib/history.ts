// iOS goes back when a finger swipes in from the left edge of the screen, and
// forward from the right, in the app on the Home Screen as in Safari, and
// whatever the page does with the touch: WebKit lets a page stop those swipes
// only by leaving them no page to go back or forward to (cancelling the touch
// as it starts, as fctl#492 did, doesn't). The app on an iPhone's or iPad's Home
// Screen has no back button, and people move about it with the sidebar and
// links, so there each page takes the place of the one before in the
// browser's history rather than adding to it: the swipes have nowhere to go,
// and a finger at the left edge drags the sidebar's drawer in instead
// (components/shell/drawer-drag.ts). Browsers' tabs keep their history, with
// their back buttons and swipes, and so does the app installed on Android,
// whose back gesture and button go back through it.

import { createBrowserHistory, type RouterHistory } from "@tanstack/react-router";
import { installed } from "./viewport";

/** Whether this is the app on an iPhone's or iPad's Home Screen, which keeps no history of its pages. */
export function keepsNoHistory(): boolean {
  // Only WebKit sends gesture events, and only a touch screen swipes (not a Mac's app in the Dock).
  return "GestureEvent" in window && navigator.maxTouchPoints > 0 && installed();
}

/** The router's history: where the app keeps none, each page it opens takes the place of the last. */
export function appHistory(): RouterHistory {
  const history = createBrowserHistory();
  if (keepsNoHistory()) history.push = history.replace;
  return history;
}

/** Loads `href` as a new page (signing in or out): in place of this one where the app keeps no history. */
export function goTo(href: string) {
  if (keepsNoHistory()) window.location.replace(href);
  else window.location.assign(href);
}

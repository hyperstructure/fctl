// iOS goes back when a finger swipes in from the left edge of the screen, and
// forward from the right, in the app on the Home Screen as in Safari, and
// whatever the page does with the touch: WebKit lets a page stop those swipes
// only by leaving them no page to go back or forward to (cancelling the touch
// as it starts, as fctl#492 did, doesn't). The app on an iPhone's or iPad's Home
// Screen has no back button, and people move about it with the sidebar and
// links, so there each page takes the place of the one before in the
// browser's history rather than adding to it: the swipes have nowhere to go,
// and a finger at the left edge drags the sidebar's drawer in instead
// (components/shell/drawer-drag.ts). Pages that load in full, the app's and
// other sites' (signing in with Google, installing the GitHub App), take its
// place too, though another site's own pages may still add to it. Browsers'
// tabs keep their history, with their back buttons and swipes, and so does
// the app installed on Android, whose back gesture and button go through it.

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

/**
 * Loads `href` as a new page (signing in or out, going to Linear to link it): in place of this
 * one where the app keeps no history.
 */
export function goTo(href: string) {
  if (keepsNoHistory()) window.location.replace(href);
  else window.location.assign(href);
}

/**
 * Where the app keeps no history, a link that loads a page in this window (signing in with
 * Google, installing the GitHub App) loads it in place of this one. The router's links, and
 * what else handles its own clicks, have cancelled them by then; one that opens a new tab or
 * window, with a modifier key or its target, goes on as the browser does. Returns what undoes it.
 */
export function linksInPlace(): () => void {
  if (!keepsNoHistory()) return () => {};
  const click = (e: MouseEvent) => {
    if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const a = e.target instanceof Element ? e.target.closest("a[href]") : null;
    if (!(a instanceof HTMLAnchorElement) || (a.target && a.target !== "_self") || a.hasAttribute("download"))
      return;
    if (a.protocol !== "http:" && a.protocol !== "https:") return;
    e.preventDefault();
    const here = window.location;
    if (
      a.origin === here.origin &&
      a.pathname === here.pathname &&
      a.search === here.search &&
      a.href.includes("#")
    ) {
      // A place on this page: there, in place of where it was, keeping the router's state, which
      // a page load would lose.
      window.history.replaceState(window.history.state, "", a.href);
      const id = decodeURIComponent(a.hash.slice(1));
      const to = id ? document.getElementById(id) : null;
      if (to) to.scrollIntoView();
      else if (!id || id.toLowerCase() === "top") window.scrollTo(0, 0);
      return;
    }
    window.location.replace(a.href);
  };
  document.addEventListener("click", click);
  return () => document.removeEventListener("click", click);
}

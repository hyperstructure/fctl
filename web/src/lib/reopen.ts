// The app on an iPhone's or iPad's Home Screen opens at its start page, the Agent Floor, whenever
// iOS starts it afresh. iOS does that by itself too: it ends an app that has been in the
// background a while (for its memory), and the next time it comes to the front, it starts again,
// with no trace of the page it was on. There the app keeps the page it is on, and an app that
// opens afresh at the start page goes back to it, as an app that iOS never ended would be.
//
// Closing the app from the app switcher and opening it again starts it afresh too, and that should
// start at the Agent Floor, as a fresh start does. The app can't tell the two apart (iOS tells a
// page nothing as it ends it), but when: a start within a minute of the app last being on screen
// is someone closing it as they used it and opening it again, and one later is iOS's. An app closed
// from the app switcher after a while in the background opens on its page again.
//
// A tap on a notification that starts the app may open it at the start page too (public/sw.js):
// then the notification's page opens (lib/push.ts onPushOpen), not the page left.

import type { RouterHistory } from "@tanstack/react-router";

/** localStorage: the page the app was on, and when it was last on it on the screen. */
const LEFT = "fctl:left";
/** sessionStorage: this run of the app has started already, so a page load now is the app's own. */
const RUNNING = "fctl:running";
/** A fresh start within this long of the app going to the background was someone closing it and opening it again (ms). */
export const RESTART_WITHIN = 60_000;

/** The page the app was on, and when it was last on the screen (ms). */
export interface Left {
  path: string;
  at: number;
}

/** Pages people go back to: those of the signed-in app. Signing in and one-off pages (approving a device) aren't. */
const PAGES = /^\/(?:$|s\/[^/]+$|p\/[^/]+$|w\/[^/]+$|pulls$|settings$|admin$|new$)/;
/** What of a page's address it opens with again: which tab or list it shows, but not a one-off outcome (`?github=installed`) or a place found. */
const KEPT = ["tab", "state", "scope", "q", "workspace"];

/** The address to go back to for a page at `pathname` with `search`, or nothing for a page not to go back to. */
export function keptAddress(pathname: string, search: string): string | undefined {
  if (!PAGES.test(pathname)) return undefined;
  const from = new URLSearchParams(search);
  const kept = new URLSearchParams();
  for (const k of KEPT) {
    const v = from.get(k);
    if (v !== null) kept.set(k, v);
  }
  const q = kept.toString();
  return q ? `${pathname}?${q}` : pathname;
}

/**
 * Where an app opening at `path` goes instead: the page it was `left` on, when this is a fresh
 * start at the start page more than `RESTART_WITHIN` after it was last on the screen.
 */
export function reopening(left: Left | undefined, opened: { path: string; fresh: boolean }, now: number) {
  if (!left || !opened.fresh || opened.path !== "/" || left.path === "/") return undefined;
  return now - left.at >= RESTART_WITHIN ? left.path : undefined;
}

function readLeft(): Left | undefined {
  try {
    const v = JSON.parse(localStorage.getItem(LEFT) ?? "null") as Partial<Left> | null;
    return v && typeof v.path === "string" && v.path.startsWith("/") && typeof v.at === "number"
      ? { path: v.path, at: v.at }
      : undefined;
  } catch {
    return undefined;
  }
}

function writeLeft(left: Left) {
  try {
    localStorage.setItem(LEFT, JSON.stringify(left));
  } catch {}
}

/** The page the app is on, if it is one to go back to (`keepWhereLeft`). */
let here: string | undefined;

/** Forgets the page the app was on, as someone signs out: the next one to sign in starts at the Agent Floor. */
export function forgetLeft() {
  here = undefined;
  try {
    localStorage.removeItem(LEFT);
  } catch {}
}

/**
 * Whether this page load starts the app afresh, rather than being one of its own (signing in, a
 * link loaded in place, WebKit loading the page again), and notes that it has started.
 */
function startsAfresh(): boolean {
  try {
    const fresh = sessionStorage.getItem(RUNNING) === null;
    sessionStorage.setItem(RUNNING, "1");
    return fresh;
  } catch {
    return false;
  }
}

/** Where the app goes back to as it starts (`reopen`), as `whereLeft` found. */
let pending: string | undefined;

/**
 * As the app starts, before the router reads the address: notes this run as started, and whether
 * it goes back to the page it was left on (`reopening`), which `reopen` does.
 */
export function whereLeft(now = Date.now()) {
  const fresh = startsAfresh();
  const { pathname, search, hash } = window.location;
  pending = reopening(readLeft(), { path: pathname + search + hash, fresh }, now);
}

/** How long the app waits at most, as it starts, to learn whether a notification opened it (ms). */
const ASK_FOR = 500;

/**
 * Before the first page shows: back to the page the app was left on, if `whereLeft` found one,
 * unless a tap on a notification opened the app (`opening`), which then opens its own page.
 */
export async function reopen(history: RouterHistory, opening: () => Promise<boolean>) {
  const to = pending;
  pending = undefined;
  if (!to) return;
  const waits = await Promise.race([
    opening().catch(() => false),
    new Promise<boolean>((done) => setTimeout(() => done(false), ASK_FOR)),
  ]);
  if (!waits) history.replace(to);
}

/**
 * Keeps the page the app is on, as `history` opens pages, and when it was last on the screen, as
 * the app goes to the background. Returns what stops it.
 */
export function keepWhereLeft(history: RouterHistory): () => void {
  const note = () => {
    const { pathname, search } = history.location;
    here = keptAddress(pathname, search);
    if (here) writeLeft({ path: here, at: Date.now() });
  };
  const left = () => {
    if (here) writeLeft({ path: here, at: Date.now() });
  };
  const hidden = () => {
    if (document.visibilityState === "hidden") left();
  };
  note();
  const unsubscribe = history.subscribe(note);
  document.addEventListener("visibilitychange", hidden);
  window.addEventListener("pagehide", left);
  return () => {
    here = undefined;
    unsubscribe();
    document.removeEventListener("visibilitychange", hidden);
    window.removeEventListener("pagehide", left);
  };
}

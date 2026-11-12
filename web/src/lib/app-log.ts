// The app's log in this browser: how each run of it started, when it went to the background and
// came back and how the page looked then, how the run before ended, the main thread stalling, and
// what went wrong (errors, a crash, WebGL losing its context). It stays in localStorage, so it
// outlasts the app being closed, and Settings → App log shows it to copy into a report.
//
// It is there for what nobody can watch happen: the app on an iPhone's Home Screen sometimes
// comes back from the background black, and stays so until it is closed (fctl#515 and before).
// When the page itself still runs, the log says what it saw. When it didn't, the next run says
// how the last one ended: on screen with the main thread stuck or the page gone, or in the
// background, where iOS ends apps.
//
// Where the page still runs but shows nothing (React took the app down after an error it didn't
// catch, or the screen came back with nothing on it), the app loads its page again
// (`reloadToRecover`), at most once a minute.

import { useSyncExternalStore } from "react";
import { duration } from "./format";
import { redact } from "./redact";

/** One thing the log says, and when (ms). */
export interface Entry {
  at: number;
  what: string;
}

/** localStorage: the log. */
const LOG = "fctl:app-log";
/** localStorage: the account the log is kept for (`logFor`). */
const OWNER = "fctl:app-log-owner";
/** localStorage: the run's heartbeat: when it was last heard from, and whether it was on screen then. */
const BEAT = "fctl:app-beat";
/** sessionStorage: this run started already (a page load now is one of its own), and when it last reloaded to recover. */
const RUNNING = "fctl:log-running";
const RECOVERED = "fctl:recovered-at";
/** Entries kept, the newest. */
export const KEEP = 300;
/** How often a run on screen says it is alive (ms). */
const BEAT_EVERY = 2_000;
/** A gap this much longer than a beat on screen is the main thread stuck (ms). */
const STALL = 3_000;
/** How soon after coming back the app looks for something on screen (ms). */
const LOOK_AFTER = 3_000;
/** Errors kept in a minute at most (`startAppLog`). */
const ERRORS = 10;
/** At most one reload to recover in this long (ms): one that doesn't help isn't tried again and again. */
const RECOVER_EVERY = 60_000;

/** The run's heartbeat. */
export interface Beat {
  at: number;
  shown: boolean;
}

/** The log as last read, which React shows until it changes. */
let entries: Entry[] | undefined;
const listeners = new Set<() => void>();

function stored(): Entry[] {
  try {
    const kept = JSON.parse(localStorage.getItem(LOG) ?? "[]") as unknown;
    return Array.isArray(kept)
      ? kept.filter(
          (e): e is Entry =>
            typeof e === "object" && e !== null && typeof e.at === "number" && typeof e.what === "string",
        )
      : [];
  } catch {
    return [];
  }
}

function changed(next: Entry[]) {
  entries = next;
  for (const l of listeners) l();
}

/** The log, oldest first. */
export function readLog(): Entry[] {
  entries ??= stored();
  return entries;
}

/**
 * Adds to the log what happened (at `at`), without anything that looks like a secret. It reads the
 * log afresh first: another tab of the app may have added to it.
 */
export function record(what: string, at = Date.now()) {
  const text = redact(what).replace(/\s+/g, " ").trim().slice(0, 400);
  const log = stored();
  // The same thing again, as an error thrown over and over is, counts on the entry it repeats
  // rather than pushing the rest of the log out.
  const last = log.at(-1);
  const repeat = last && /^(.*?)(?: \(×(\d+)\))?$/.exec(last.what);
  const next =
    repeat && repeat[1] === text
      ? [...log.slice(0, -1), { at, what: `${text} (×${Number(repeat[2] ?? 1) + 1})` }]
      : [...log, { at, what: text }].slice(-KEEP);
  try {
    localStorage.setItem(LOG, JSON.stringify(next));
  } catch {}
  changed(next);
}

/** Empties the log and forgets the run's heartbeat (Clear), keeping whose it is. */
export function clearLog() {
  try {
    localStorage.removeItem(LOG);
    localStorage.removeItem(BEAT);
  } catch {}
  changed([]);
}

/**
 * Signing out: empties the log and keeps it for no one, so that what this page and other tabs
 * still open add to it after, about the account that signed out, goes as anyone signs in next.
 */
export function forgetLog() {
  clearLog();
  try {
    localStorage.setItem(OWNER, "");
  } catch {}
}

/**
 * Keeps the log for `user`, who is signed in: one kept for someone else, whose sign-in ended here
 * without them signing out, or for no one since a sign-out, goes first. A sign-in that ends
 * leaves the log, as it may tell why.
 */
export function logFor(user: string) {
  try {
    const was = localStorage.getItem(OWNER);
    if (was !== null && was !== user) clearLog();
    localStorage.setItem(OWNER, user);
  } catch {}
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  // What another tab adds, or cleared, from now on and since this tab last read the log.
  const other = (e: StorageEvent) => {
    if (e.key === LOG || e.key === null) changed(stored());
  };
  window.addEventListener("storage", other);
  changed(stored());
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", other);
  };
}

/** The log, as it grows. */
export function useAppLog(): Entry[] {
  return useSyncExternalStore(subscribe, readLog, () => []);
}

const two = (n: number) => String(n).padStart(2, "0");

/** `2026-11-12 13:05:02`, in this browser's time zone. */
function at(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`;
}

/** The log as text to paste into a report, with the browser it is from. */
export function logText(log: Entry[], about: string): string {
  return [about, "", ...log.map((e) => `${at(e.at)}  ${e.what}`)].join("\n");
}

/** How the run before ended, as its last heartbeat tells, or nothing for a first run. */
export function previousEnd(beat: Beat | undefined): string | undefined {
  if (!beat) return undefined;
  return beat.shown
    ? `The run before stopped while on screen: last heard at ${at(beat.at)}. Its page stopped, or was ended, without going to the background.`
    : `The run before ended in the background: last on screen at ${at(beat.at)}.`;
}

/** The page's viewports and the screen, as `402×812 (visual 402×812 at 0), screen 402×874`. */
export function viewportText(v: {
  width: number;
  height: number;
  visualWidth?: number;
  visualHeight?: number;
  offsetTop?: number;
  screenWidth: number;
  screenHeight: number;
  scrollY: number;
}): string {
  const visual =
    v.visualHeight === undefined
      ? ""
      : ` (visual ${Math.round(v.visualWidth ?? 0)}×${Math.round(v.visualHeight)} at ${Math.round(v.offsetTop ?? 0)})`;
  return `viewport ${v.width}×${v.height}${visual}, screen ${v.screenWidth}×${v.screenHeight}, scrolled ${Math.round(v.scrollY)}`;
}

function viewportNow(): string {
  const vv = window.visualViewport;
  return viewportText({
    width: document.documentElement.clientWidth,
    height: document.documentElement.clientHeight,
    visualWidth: vv?.width,
    visualHeight: vv?.height,
    offsetTop: vv?.offsetTop,
    screenWidth: window.screen.width,
    screenHeight: window.screen.height,
    scrollY: window.scrollY,
  });
}

/** What is on screen: whether the app shows anything, and what is at the screen's middle. */
function onScreen(): string | undefined {
  const root = document.getElementById("root");
  if (!root?.childElementCount) return "nothing: the app shows no page";
  const middle = document.elementFromPoint(window.innerWidth / 2, window.innerHeight / 2);
  if (!middle || middle === document.documentElement || middle === document.body)
    return "no part of the page at the screen's middle";
  return undefined;
}

/** How the page loaded: `navigate`, `reload`, `back_forward`. */
function navigationType(): string {
  const nav = performance.getEntriesByType?.("navigation")[0] as PerformanceNavigationTiming | undefined;
  return nav?.type ?? "unknown";
}

/** Which build of the app runs: its main script's name (`index-tOVBOagz.js`), or `dev`. */
function build(): string {
  const src = document.querySelector<HTMLScriptElement>('script[type="module"][src*="/assets/"]')?.src;
  return src?.split("/").pop() ?? "dev";
}

/** Whether the app runs in a window of its own, from the Home Screen or the Dock: one run at a time. */
function installed(): boolean {
  return (
    (navigator as { standalone?: boolean }).standalone === true ||
    window.matchMedia?.("(display-mode: standalone)").matches === true
  );
}

function where(): string {
  if (!installed()) return "a browser tab";
  return "GestureEvent" in window && navigator.maxTouchPoints > 0
    ? "the Home Screen app"
    : "the installed app";
}

function message(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  if (typeof e === "object" && e !== null) {
    try {
      return JSON.stringify(e).slice(0, 200);
    } catch {}
  }
  return String(e);
}

/**
 * Loads the page again to bring back an app that shows nothing, unless it did so within the last
 * minute (the reload didn't help, and another wouldn't). Says which in the log.
 */
export function reloadToRecover(why: string, now = Date.now()): boolean {
  let last: number;
  try {
    last = Number(sessionStorage.getItem(RECOVERED) ?? 0);
  } catch {
    // Without a note of the last reload, one that doesn't help would go on and on.
    record(`${why}; not loading the page again, as this browser keeps no note of when it last did`, now);
    return false;
  }
  if (now - last < RECOVER_EVERY) {
    record(`${why}; not loading the page again, as it did ${duration(now - last)} ago`, now);
    return false;
  }
  try {
    sessionStorage.setItem(RECOVERED, String(now));
  } catch {
    record(`${why}; not loading the page again, as this browser keeps no note of when it last did`, now);
    return false;
  }
  record(`${why}; loading the page again`, now);
  window.location.reload();
  return true;
}

/** React took the app down after an error nothing caught: it shows nothing until the page loads again. */
export function onUncaughtError(error: unknown) {
  console.error(error);
  reloadToRecover(`Crashed: ${message(error)}`);
}

/** An error the router's error page caught and showed. */
export function onCaughtError(error: unknown) {
  // The router's not-found page catches its own, as errors: an address with no page isn't one.
  if (typeof error === "object" && error !== null && "isNotFound" in error) return;
  console.error(error);
  record(`Error page for: ${message(error)}`);
}

/**
 * Follows a WebGL scene's canvas for the log (it lost its context, got it back, or had none), and
 * as the canvas leaves the page, lets its context go at once, rather than when the browser
 * collects it, to free what it holds of the graphics memory. Call it once the scene is mounted;
 * it returns what to do as the scene is unmounted.
 */
export function watchWebgl(canvas: HTMLCanvasElement, name: string): () => void {
  const at = () => window.location.pathname;
  if (canvas.dataset.webgl === "unavailable") record(`WebGL gave the ${name} no context, on ${at()}`);
  else if (canvas.dataset.webgl === "failed") record(`WebGL couldn't start the ${name}, on ${at()}`);
  const lost = () => record(`WebGL lost the ${name}'s context, on ${at()}`);
  const restored = () => record(`WebGL restored the ${name}'s context, on ${at()}`);
  canvas.addEventListener("webglcontextlost", lost);
  canvas.addEventListener("webglcontextrestored", restored);
  return () => {
    canvas.removeEventListener("webglcontextlost", lost);
    canvas.removeEventListener("webglcontextrestored", restored);
    // Only a canvas that left the page: React's development mode unmounts and mounts a scene
    // again on the same canvas, which a lost context would leave blank.
    if (canvas.isConnected) return;
    const gl = canvas.getContext("webgl2");
    if (gl && !gl.isContextLost()) gl.getExtension("WEBGL_lose_context")?.loseContext();
  };
}

/**
 * Starts the log for this run: how it started and how the run before ended, then the run's trips
 * to the background and back, stalls and errors. Returns what stops it.
 */
export function startAppLog(): () => void {
  let fresh = true;
  try {
    fresh = sessionStorage.getItem(RUNNING) === null;
    sessionStorage.setItem(RUNNING, "1");
  } catch {}
  let before: Beat | undefined;
  try {
    const b = JSON.parse(localStorage.getItem(BEAT) ?? "null") as Partial<Beat> | null;
    if (b && typeof b.at === "number" && typeof b.shown === "boolean") before = { at: b.at, shown: b.shown };
  } catch {}
  // Tabs run side by side, each with its own heartbeat: only an installed app's runs follow one
  // another.
  const own = installed();
  if (fresh && own) {
    const end = previousEnd(before);
    if (end) record(end);
  }
  record(
    `Started (${fresh ? "a fresh run" : "a page load in the same run"}, ${navigationType()}) in ${where()} at ${window.location.pathname}, build ${build()}; ${viewportNow()}`,
  );

  const shown = () => document.visibilityState === "visible";
  const beat = (on = shown()) => {
    if (!own) return;
    try {
      localStorage.setItem(BEAT, JSON.stringify({ at: Date.now(), shown: on }));
    } catch {}
  };
  beat();

  let last = performance.now();
  let hiddenAt: number | undefined;
  let look: ReturnType<typeof setTimeout> | undefined;
  const tick = setInterval(() => {
    const now = performance.now();
    if (shown() && now - last > BEAT_EVERY + STALL)
      record(`The main thread was stuck for ${duration(now - last - BEAT_EVERY)} on screen`);
    last = now;
    if (shown()) beat();
  }, BEAT_EVERY);

  // A page that unloads goes hidden too: that isn't the app going to the background.
  let unloading = false;
  const visibility = () => {
    last = performance.now();
    beat();
    clearTimeout(look);
    if (unloading) return;
    if (!shown()) {
      hiddenAt = Date.now();
      record("Went to the background");
      return;
    }
    const away = hiddenAt === undefined ? "" : ` after ${duration(Date.now() - hiddenAt)} away`;
    hiddenAt = undefined;
    record(`Came back${away}; ${viewportNow()}`);
    look = setTimeout(() => {
      if (!shown()) return;
      const missing = onScreen();
      if (!missing) return;
      if (missing.startsWith("nothing")) reloadToRecover(`On screen again with ${missing}`);
      else record(`On screen again with ${missing}; ${viewportNow()}`);
    }, LOOK_AFTER);
  };
  const pageshow = (e: PageTransitionEvent) => {
    unloading = false;
    if (e.persisted) record("Came back from the browser's page cache");
  };
  const pagehide = (e: PageTransitionEvent) => {
    beat(false);
    unloading = !e.persisted;
    record(e.persisted ? "Left for the browser's page cache" : "Page unloading");
  };
  const frozen = () => record("Frozen by the browser");
  const resumed = () => record("Resumed by the browser");
  const offline = () => record("Went offline");
  const online = () => record("Back online");
  // At most ERRORS a minute, then one line that says more came: a handler that fails on every
  // frame would push out what the log is for, as different errors.
  let errors = 0;
  let minute = 0;
  const failed = (what: string) => {
    const now = Date.now();
    if (now - minute >= 60_000) {
      minute = now;
      errors = 0;
    }
    errors++;
    if (errors <= ERRORS) record(what, now);
    else if (errors === ERRORS + 1) record("More errors this minute, not kept", now);
  };
  const error = (e: ErrorEvent) => failed(`Error: ${e.message || message(e.error)}`);
  const rejection = (e: PromiseRejectionEvent) => failed(`Unhandled rejection: ${message(e.reason)}`);

  const off = new AbortController();
  const on = { signal: off.signal };
  document.addEventListener("visibilitychange", visibility, on);
  document.addEventListener("freeze", frozen, on);
  document.addEventListener("resume", resumed, on);
  window.addEventListener("pageshow", pageshow, on);
  window.addEventListener("pagehide", pagehide, on);
  window.addEventListener("offline", offline, on);
  window.addEventListener("online", online, on);
  window.addEventListener("error", error, on);
  window.addEventListener("unhandledrejection", rejection, on);
  return () => {
    off.abort();
    clearInterval(tick);
    clearTimeout(look);
  };
}

// Safari on iOS can lose track of the on-screen keyboard. The installed app
// can come back from the app switcher with WebKit taking a keyboard to cover
// the bottom of the screen though none shows, until the app is closed. (Most
// likely as WebKit ignores a keyboard going away while the app turns, and iOS
// turns it for the app switcher's snapshots.) The visual viewport is then
// shorter than the screen by the keyboard's height, or has no height at all,
// and the layout viewport, which sticky and fixed elements are placed against,
// moves only once the visual viewport passes one of its edges. As the page
// scrolls down, the screen leaves the layout viewport behind by up to that
// height, and what is pinned to its edges comes loose: the session's header
// scrolls off the top, and its composer rides up the screen.
//
// While that lasts, --viewport-shift on <html> says how far below the layout
// viewport the screen is, and what is pinned to the screen's edges moves down
// by as much (`top-(--viewport-shift)`, `-bottom-(--viewport-shift)`). It is 0
// otherwise. It comes a frame late while the screen leaves the layout viewport
// behind, and is right once the screen takes the layout viewport along.
//
// The installed app can also come back laid out under the status bar, until
// it is closed: the layout viewport starts at the screen's top rather than
// below the status bar, as tall as the screen, and the bar covers what is
// pinned to the top, with the session's header behind it (WebKit bug 301994
// has other web apps' reports of a status bar over their top). The status bar
// otherwise takes its own room above the page. What is pinned to the top
// makes room for the bar with --safe-top (app.css): Safari's safe area at the
// top, which counts the bar the page is under, or, should Safari leave it at
// 0, --status-bar on <html>, which says how tall the bar was when it last
// stood above the page, while the layout viewport is as tall as the screen.
//
// iOS then also draws a frosted band over the page's top, which dims and blurs
// what is under the bar and fades out below it, where the header's logo and
// the drawer's head blurred. On an iPhone's or iPad's Home Screen,
// --status-bar-fade on <html> says how far below the bar it fades out, and
// --safe-top makes room for that too, only while the bar covers the page. A
// band of the page's color covers it all (app.css), as the status bar's own
// band does while the page is below it.

/** A keyboard is at least this tall (px): a visual viewport this much shorter than the layout viewport has one over it. */
const KEYBOARD = 100;
/** A status bar is at most this tall (px): a layout viewport shorter than the screen by more has something else above it. */
const STATUS_BAR = 100;
/** Where the app keeps the status bar's height, for when it opens under the bar. */
const STATUS_BAR_KEY = "fctl:status-bar";
/**
 * How far below the status bar the frosted band iOS draws over a page under the bar fades out
 * (px): about 34 px on a 402 x 874 iPhone under its 62 px bar, and a little more to be clear of it.
 */
export const STATUS_BAR_FADE = 36;
/** How long a keyboard takes to go once its field loses the focus (ms): until then, it may still be up. */
const KEYBOARD_GOES = 1000;

/** The page's viewports, as tests can make them up. */
export interface Viewports {
  /** The layout viewport's height (px). */
  layout: number;
  /** The visual viewport's height (px), as Safari says. */
  visual: number;
  /** How far below the layout viewport's top the visual viewport's is (px). */
  offsetTop: number;
  /** How far the visual viewport is zoomed in. */
  scale: number;
  /** A field that brings up a keyboard has the focus, or just lost it. */
  typing: boolean;
  /** The app was opened from the home screen, where no toolbars come and go. */
  installed: boolean;
}

/**
 * Whether Safari takes a keyboard to cover the screen where none can be up:
 * in the installed app, with no field to type in, the visual viewport is
 * shorter than the layout viewport by a keyboard or more, and it isn't
 * zoomed in.
 */
export function phantomKeyboard(v: Viewports): boolean {
  return v.installed && !v.typing && Math.abs(v.scale - 1) < 0.01 && v.visual <= v.layout - KEYBOARD;
}

/** How far below the layout viewport the screen is (px): 0 unless Safari's phantom keyboard is there. */
export function viewportShift(v: Viewports): number {
  return phantomKeyboard(v) ? Math.max(0, v.offsetTop) : 0;
}

/** The screen and the layout viewport, as tests can make them up. */
export interface Screen {
  /** The screen's width (px), as it is held upright on an iPhone or iPad (`screen.width`). */
  width: number;
  /** The screen's height (px), held upright. */
  height: number;
  /** The layout viewport's width (px). */
  layoutWidth: number;
  /** The layout viewport's height (px). */
  layoutHeight: number;
}

/** Whether the app has the whole screen, held upright: an iPhone's in portrait, an iPad's in full screen. */
function upright(s: Screen): boolean {
  return Math.abs(s.layoutWidth - s.width) < 1;
}

/**
 * How tall the status bar is above the page (px): what the layout viewport leaves of the
 * screen's height where the app has the whole screen upright and a status bar could take it,
 * 0 otherwise.
 */
export function statusBarAbove(s: Screen): number {
  const gap = s.height - s.layoutHeight;
  return upright(s) && gap >= 1 && gap <= STATUS_BAR ? gap : 0;
}

/** Whether the page is laid out under the status bar: the layout viewport is as tall as the screen held upright. */
export function underStatusBar(s: Screen): boolean {
  return upright(s) && Math.abs(s.height - s.layoutHeight) < 1;
}

/** The screen and the layout viewport as they are. */
function screenNow(): Screen {
  const page = document.documentElement;
  return {
    width: window.screen.width,
    height: window.screen.height,
    layoutWidth: page.clientWidth,
    layoutHeight: page.clientHeight,
  };
}

/** The status bar's height this app saw above the page on this screen before (px), or 0. */
function rememberedStatusBar(s: Screen): number {
  try {
    const kept = JSON.parse(localStorage.getItem(STATUS_BAR_KEY) ?? "null");
    return kept?.screen === s.height && typeof kept.bar === "number" ? kept.bar : 0;
  } catch {
    return 0;
  }
}

function rememberStatusBar(s: Screen, bar: number) {
  try {
    localStorage.setItem(STATUS_BAR_KEY, JSON.stringify({ screen: s.height, bar }));
  } catch {}
}

/** A field that brings up a keyboard lost the focus a moment ago: its keyboard may still be going. */
let keyboardGoing = false;

/** Whether `el` brings up a keyboard when it has the focus. */
function takesKeyboard(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.isContentEditable || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement)
    return true;
  return (
    el instanceof HTMLInputElement &&
    !/^(button|checkbox|color|file|hidden|image|radio|range|reset|submit)$/.test(el.type)
  );
}

/** Whether the app was opened from the home screen rather than in a browser's tab. */
export function installed(): boolean {
  return (
    window.matchMedia("(display-mode: standalone)").matches ||
    (navigator as { standalone?: boolean }).standalone === true
  );
}

/** Whether this is the app on an iPhone's or iPad's Home Screen. */
export function onHomeScreen(): boolean {
  // Only WebKit sends gesture events, and only an iPhone or iPad has a touch screen (not a Mac,
  // whose app in the Dock is installed too).
  return "GestureEvent" in window && navigator.maxTouchPoints > 0 && installed();
}

/** The page's viewports as they are, or nothing in a browser without a visual viewport. */
export function viewports(): Viewports | undefined {
  const vv = window.visualViewport;
  if (!vv) return undefined;
  return {
    layout: document.documentElement.clientHeight,
    visual: vv.height,
    offsetTop: vv.offsetTop,
    scale: vv.scale,
    typing: keyboardGoing || takesKeyboard(document.activeElement),
    installed: installed(),
  };
}

/**
 * How tall the visual viewport is (px): as Safari says, shorter with the
 * keyboard up, but the layout viewport's height while Safari's phantom
 * keyboard shortens it. Nothing in a browser without a visual viewport.
 */
export function visualHeight(): number | undefined {
  const v = viewports();
  if (!v) return undefined;
  return phantomKeyboard(v) ? v.layout : v.visual;
}

/**
 * Keeps --viewport-shift on <html> up to date in the installed app, and --status-bar on an
 * iPhone's or iPad's Home Screen, where it also sets --status-bar-fade. Returns what stops it.
 */
export function watchViewport(): () => void {
  const vv = window.visualViewport;
  if (!vv || !installed()) return () => {};
  const html = document.documentElement;
  const ios = onHomeScreen();
  // Room for iOS's frosted band, which --safe-top adds only while the bar covers the page.
  if (ios) html.style.setProperty("--status-bar-fade", `${STATUS_BAR_FADE}px`);
  let shift = 0;
  let later: ReturnType<typeof setTimeout> | undefined;
  // The status bar's height as it last stood above the page, and what --status-bar says.
  let seen = ios ? rememberedStatusBar(screenNow()) : 0;
  let bar = 0;
  const statusBar = () => {
    const s = screenNow();
    const above = statusBarAbove(s);
    if (above && above !== seen) {
      seen = above;
      rememberStatusBar(s, above);
    }
    const next = underStatusBar(s) ? seen : 0;
    if (next === bar) return;
    bar = next;
    if (bar) html.style.setProperty("--status-bar", `${bar}px`);
    else html.style.removeProperty("--status-bar");
  };
  const update = () => {
    if (ios) statusBar();
    const v = viewports();
    const next = v ? viewportShift(v) : 0;
    if (next === shift) return;
    shift = next;
    if (shift) html.style.setProperty("--viewport-shift", `${shift}px`);
    else html.style.removeProperty("--viewport-shift");
  };
  const left = (e: FocusEvent) => {
    if (!takesKeyboard(e.target)) return;
    // Look again once its keyboard is gone.
    keyboardGoing = true;
    clearTimeout(later);
    later = setTimeout(() => {
      keyboardGoing = false;
      update();
    }, KEYBOARD_GOES);
  };
  // Safari lets the page scroll past its bottom by as much as the phantom
  // keyboard is tall, as if to bring what it covers into sight. Nothing shows
  // there, and what is pinned to the bottom can't follow past the page's end:
  // the page glides back to its end, as it does after scrolling past it.
  const back = () => {
    const v = viewports();
    const page = document.documentElement;
    const end = page.scrollHeight - page.clientHeight;
    if (v && phantomKeyboard(v) && window.scrollY > end + 1)
      window.scrollTo({ top: end, behavior: "smooth" });
  };
  const off = new AbortController();
  const on = { signal: off.signal };
  // Its scroll comes as the screen moves away from the layout viewport, not
  // as the page scrolls along with it.
  vv.addEventListener("resize", update, on);
  vv.addEventListener("scroll", update, { signal: off.signal, passive: true });
  // The layout viewport changes size, as it goes under the status bar or the app turns.
  window.addEventListener("resize", update, on);
  window.addEventListener("scrollend", back, on);
  window.addEventListener("pageshow", update, on);
  document.addEventListener("visibilitychange", update, on);
  document.addEventListener("focusin", update, on);
  document.addEventListener("focusout", left, on);
  update();
  return () => {
    off.abort();
    clearTimeout(later);
    keyboardGoing = false;
    html.style.removeProperty("--viewport-shift");
    html.style.removeProperty("--status-bar");
    html.style.removeProperty("--status-bar-fade");
  };
}

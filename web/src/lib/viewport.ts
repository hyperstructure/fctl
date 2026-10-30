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

/** A keyboard is at least this tall (px): a visual viewport this much shorter than the layout viewport has one over it. */
const KEYBOARD = 100;
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
function installed(): boolean {
  return (
    window.matchMedia("(display-mode: standalone)").matches ||
    (navigator as { standalone?: boolean }).standalone === true
  );
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

/** Keeps --viewport-shift on <html> up to date in the installed app. Returns what stops it. */
export function watchViewport(): () => void {
  const vv = window.visualViewport;
  if (!vv || !installed()) return () => {};
  const html = document.documentElement;
  let shift = 0;
  let later: ReturnType<typeof setTimeout> | undefined;
  const update = () => {
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
  };
}

// Plays a live diff's new version (components/diff.tsx) as it shows: where
// the reader sees it, the rows the agent's change brought open, one after
// the other from the top, and the code it wrote comes in from the left
// behind a caret, over what a line said before when it rewrote one; rows
// that went close, and a line it took out turns red from the left. The
// rows above and below what shows take their room at once, out of sight,
// and what changed there lights up once the reader comes to it
// (styles/app.css has the looks).

/** What a live diff asks of what shows it: to keep the reader's place, and to know what they see. */
export interface LiveView {
  /** Keeps the line the reader looks at where it shows, after rows above it came or went. */
  hold(): void;
  /** Keeps it there for this long, while rows open and close (ms). */
  moving(ms: number): void;
  /** What shows of the diffs, from under the header of the file at the top (client px). */
  band(): { top: number; bottom: number } | undefined;
  /** Changes the agent made out of the reader's sight, each its rows, for them to go to. */
  unseen(changes: HTMLElement[][]): void;
}

/** How long a row takes to open, and to close (ms). */
export const OPEN_MS = 300;
export const CLOSE_MS = 240;
/** Rows open one after another this far apart, or closer when many come at once: all within the cascade (ms). */
const STAGGER_MS = 26;
const CASCADE_MS = 380;
/** Code is written in after its row starts to open, at this pace, within these bounds (ms). */
const WRITE_AFTER_MS = 90;
const WRITE_MS_PER_CHAR = 7;
const WRITE_MS = [220, 560] as const;
/** How long what the agent changed stays lit (ms). */
export const LIT_MS = 2600;
/** When the rows that went are taken out and the version settles (ms): after the last has opened. */
export const SETTLE_MS = CASCADE_MS + OPEN_MS + 240;
const EASE_OUT = "cubic-bezier(0.16, 1, 0.3, 1)";
const EASE_IN_OUT = "cubic-bezier(0.65, 0, 0.35, 1)";

const reduced = () => window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;

/** How many columns a line's text takes, tabs as four. */
const columns = (text: string) => text.length + (text.split("\t").length - 1) * 3;

/** The diffs whose new versions play, gathered from one commit (each file's diff is its own). */
const queued = new Map<LiveView, Set<HTMLElement>>();

/**
 * Plays a new version's rows in `box`: those with `data-open` (`in` as they
 * come, `out` as they go) open or close, and those with `data-how` (how
 * their code came) are written in or lit. The diffs that change together,
 * in one commit, play together, once it is done: their rows are measured
 * where they end up, before any opens from nothing.
 */
export function play(box: HTMLElement, live: LiveView) {
  const boxes = queued.get(live);
  if (boxes) {
    boxes.add(box);
    return;
  }
  queued.set(live, new Set([box]));
  queueMicrotask(() => {
    const all = queued.get(live);
    queued.delete(live);
    if (all)
      playAll(
        [...all].filter((b) => b.isConnected),
        live,
      );
  });
}

function playAll(boxes: HTMLElement[], live: LiveView) {
  const pick = (selector: string) => boxes.flatMap((b) => [...b.querySelectorAll<HTMLElement>(selector)]);
  const opening = pick("[data-open]");
  const writing = pick("[data-how]");
  // What still played on these rows from the version before is done (and the layout read
  // below lets it play again).
  for (const el of writing) delete el.dataset.play;
  // The rows that came and went above what shows take their room out of sight.
  live.hold();
  const still = reduced();
  const band = live.band();

  // The rows the reader sees play: those in view, and those below that rows opening and
  // closing above them bring into view as they do (rows open from nothing). The others are
  // as they are now: those above take their room out of sight, and those that go, go once
  // the version settles.
  const rows = [...new Set([...opening, ...writing])];
  const rect = new Map(rows.map((el) => [el, el.getBoundingClientRect()]));
  const top = (el: HTMLElement) => rect.get(el)?.top ?? 0;
  rows.sort((a, b) => top(a) - top(b));
  const seen = new Set<HTMLElement>();
  let reach = 0;
  for (const el of rows) {
    const r = rect.get(el);
    if (!band || !r) break;
    if (r.bottom <= band.top) continue;
    if (r.top - reach >= band.bottom) break;
    seen.add(el);
    if (el.dataset.open) reach += r.height;
  }

  // One after another from the top, rows side by side together: a wave down what shows, as
  // quick for a screenful as for a few.
  const tops = [...new Set([...seen].map((el) => Math.round(top(el))))].sort((a, b) => a - b);
  const step = tops.length > 1 ? Math.min(STAGGER_MS, CASCADE_MS / (tops.length - 1)) : 0;
  const order = new Map(tops.map((t, i) => [t, i]));
  const wave = (el: HTMLElement) => Math.round((order.get(Math.round(top(el))) ?? 0) * step);

  let longest = 0;
  if (!still) {
    for (const el of opening) {
      if (!seen.has(el)) continue;
      const height = `${rect.get(el)?.height ?? el.offsetHeight}px`;
      const shut = { height: "0px", minHeight: "0px", opacity: 0, clipPath: "inset(0)" };
      const open = { height, minHeight: "0px", opacity: 1, clipPath: "inset(0)" };
      if (el.dataset.open === "out") {
        el.animate([open, shut], { duration: CLOSE_MS, easing: EASE_IN_OUT, fill: "forwards" });
        longest = Math.max(longest, CLOSE_MS);
      } else {
        const delay = wave(el);
        el.animate([shut, open], { duration: OPEN_MS, delay, easing: EASE_OUT, fill: "backwards" });
        longest = Math.max(longest, delay + OPEN_MS);
      }
    }
  }

  // The code the agent wrote: written in where it shows once the rows have played, lit once
  // the reader comes to it elsewhere.
  const shows = (el: HTMLElement) => {
    const r = rect.get(el);
    return !!band && !!r && seen.has(el) && r.bottom > band.top && r.top < band.bottom;
  };
  const unseen: HTMLElement[][] = [];
  const lit: HTMLElement[] = [];
  let run: HTMLElement[] | undefined;
  let lastLit = 0;
  for (const el of writing) {
    const how = el.dataset.how;
    const quiet = how === "unmarked";
    if (shows(el)) {
      run = undefined;
      if (still) {
        if (!quiet) lit.push(el);
        continue;
      }
      const delay = wave(el) + (el.closest("[data-open='in']") ? WRITE_AFTER_MS : 0);
      const code = el.querySelector<HTMLElement>(".diff-code");
      const chars = Math.max(columns(code?.textContent ?? ""), columns(code?.dataset.was ?? ""));
      // A line that wraps is written in line after line, too far for a caret to follow.
      if (wraps(code)) el.dataset.wraps = "";
      else delete el.dataset.wraps;
      const ms = Math.min(WRITE_MS[1], Math.max(WRITE_MS[0], chars * WRITE_MS_PER_CHAR));
      el.style.setProperty("--diff-chars", String(chars));
      el.style.setProperty("--diff-write-ms", `${ms}ms`);
      el.style.setProperty("--diff-delay", `${delay}ms`);
      el.dataset.play = how;
      el.dataset.playUntil = String(Math.round(performance.now() + delay + LIT_MS));
      lastLit = Math.max(lastLit, delay + LIT_MS);
      continue;
    }
    if (quiet) continue;
    // Rows one under the other are one change.
    const prev = run?.[run.length - 1];
    if (run && prev && adjacent(prev, el)) run.push(el);
    else {
      run = [el];
      unseen.push(run);
    }
  }
  if (lit.length) light(lit);
  if (lastLit) setTimeout(() => boxes.forEach(done), lastLit + 200);
  if (longest) live.moving(longest + 32);
  if (unseen.length) live.unseen(unseen);
}

/** Whether a line's code takes more than one line on the screen. */
function wraps(code: HTMLElement | null): boolean {
  const text = code?.querySelector(".diff-text");
  return !!text && text.getClientRects().length > 1;
}

/** Whether `b` is the row after `a` (or beside it, side by side). */
function adjacent(a: HTMLElement, b: HTMLElement): boolean {
  const row = (el: HTMLElement) => (el.parentElement?.classList.contains("grid") ? el.parentElement : el);
  const ra = row(a);
  const rb = row(b);
  return ra === rb || ra.nextElementSibling === rb;
}

/**
 * Lights rows the agent changed up, for a moment: as the reader comes to
 * them. A row that plays already (the agent changed it again) plays on.
 */
export function light(rows: HTMLElement[]) {
  const now = performance.now();
  const until = String(Math.round(now + LIT_MS));
  for (const el of rows) {
    if (!el.isConnected || Number(el.dataset.playUntil ?? 0) > now) continue;
    el.style.setProperty("--diff-delay", "0ms");
    el.dataset.play = "seen";
    el.dataset.playUntil = until;
  }
  const box = rows[0]?.closest<HTMLElement>(".diff");
  if (box) setTimeout(() => done(box), LIT_MS + 200);
}

/** Takes what played away once it has played, but what plays still (it played again since). */
function done(box: HTMLElement) {
  const now = performance.now();
  for (const el of box.querySelectorAll<HTMLElement>("[data-play]")) {
    if (Number(el.dataset.playUntil ?? 0) > now) continue;
    delete el.dataset.play;
    delete el.dataset.playUntil;
  }
}

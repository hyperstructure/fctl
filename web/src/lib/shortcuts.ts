// Every keyboard shortcut of the web app, as the keyboard shortcuts sheet (? or ⌘/) lists them,
// grouped by where they work, and how tooltips and the command palette say them. Those that work
// anywhere, or on a page, are taken with `useShortcut`; the others by what they act in (the
// diffs, the find bars, the text boxes), which does what this list says.

import { apple, type Chord, type HotkeyOptions, parseShortcut, useHotkeys } from "./hotkeys";

const LETTER = /^[a-z]$/;

/** Where shortcuts work, in the order the sheet shows them. */
export const GROUPS = [
  { id: "anywhere", title: "Anywhere" },
  { id: "go", title: "Go to" },
  { id: "session", title: "On a session's page" },
  { id: "writing", title: "Writing" },
  { id: "find", title: "Finding" },
  { id: "changes", title: "In the changes" },
  { id: "pulls", title: "On the pull requests page" },
] as const;

export type GroupId = (typeof GROUPS)[number]["id"];

/**
 * A way to press a shortcut, as `useHotkey` takes it ("mod+k"; "g f" is G, then F); or keys that
 * each do one of what its label lists, apart with slashes (["j", "k"] for "Next / previous").
 */
export type Keys = string | readonly string[];

export interface Shortcut {
  /** What it does. */
  label: string;
  group: GroupId;
  /** Its keys: any of them does it. The first is the one tooltips name. */
  keys: readonly Keys[];
  /** More keys that work, which the sheet doesn't show (⌘\ folded the sidebar before ⌘B). */
  also?: readonly string[];
  /** Where it works, when that isn't all of its group. */
  note?: string;
}

export const SHORTCUTS = {
  // Anywhere.
  palette: {
    label: "Search or jump to…",
    group: "anywhere",
    keys: ["mod+k"],
    // As quick open and the command palette in code editors.
    also: ["mod+p", "mod+shift+p"],
  },
  newSession: { label: "New session", group: "anywhere", keys: ["n", "mod+shift+o"] },
  help: { label: "Keyboard shortcuts", group: "anywhere", keys: ["?", "mod+/"] },
  fold: { label: "Fold or unfold the sidebar", group: "anywhere", keys: ["mod+b"], also: ["mod+\\"] },
  // Go to.
  floor: { label: "Agent Floor", group: "go", keys: ["g f"] },
  needs: { label: "Next in Needs you", group: "go", keys: ["g n"] },
  pulls: { label: "Pull requests", group: "go", keys: ["g p"] },
  settings: { label: "Settings", group: "go", keys: ["g s"] },
  // On a session's page.
  reply: { label: "Message the agent", group: "session", keys: ["r"] },
  changes: { label: "Open the changes", group: "session", keys: ["c"] },
  details: { label: "Show or hide the details", group: "session", keys: ["d"] },
  nextTurn: {
    label: "Next turn / previous",
    group: "session",
    keys: [["j", "k"]],
    note: "In the new view",
  },
  view: { label: "Switch between the classic and the new view", group: "session", keys: ["v"] },
  // Writing.
  start: { label: "Start the session", group: "writing", keys: ["mod+enter"], note: "In a task box" },
  send: {
    label: "Send / new line",
    group: "writing",
    keys: [["enter", "shift+enter"]],
    note: "In a message to an agent",
  },
  leave: { label: "Leave the box, for single keys", group: "writing", keys: ["escape"] },
  // Finding.
  find: { label: "Find", group: "find", keys: ["mod+f"], note: "On a session's page, and in its changes" },
  nextMatch: {
    label: "Next match / previous",
    group: "find",
    keys: [["enter", "shift+enter"]],
    note: "In the bar",
  },
  stepMatch: {
    label: "Next match / previous",
    group: "find",
    keys: [["mod+g", "mod+shift+g"]],
    note: "While the bar is open",
  },
  browserFind: {
    label: "Your browser's own find",
    group: "find",
    keys: ["mod+f"],
    note: "Again, in the bar",
  },
  // In the changes.
  nextFile: { label: "Next file / previous", group: "changes", keys: [["j", "k"]] },
  nextChange: { label: "Next change / previous", group: "changes", keys: [["n", "p"]] },
  sideBySide: { label: "Side by side, or one below the other", group: "changes", keys: ["s"] },
  wrap: { label: "Wrap long lines", group: "changes", keys: ["w"] },
  filterFiles: { label: "Filter the files", group: "changes", keys: ["/"] },
  // The find bar's toggles there, as in code editors.
  matchCase: { label: "Find: match case", group: "changes", keys: ["alt+c"] },
  wholeWords: { label: "Find: whole words", group: "changes", keys: ["alt+w"] },
  regex: { label: "Find: regular expressions", group: "changes", keys: ["alt+r"] },
  // On the pull requests page.
  filterPulls: { label: "Filter", group: "pulls", keys: ["/"] },
  nextPull: { label: "Next / previous", group: "pulls", keys: [["j", "k"]] },
  openPull: { label: "Open its session", group: "pulls", keys: ["enter"] },
  github: { label: "Open it on GitHub", group: "pulls", keys: ["o"] },
  copyLink: { label: "Copy its link", group: "pulls", keys: ["c"] },
  tabs: { label: "Next tab / previous", group: "pulls", keys: [["arrowright", "arrowleft"]] },
} satisfies Record<string, Shortcut>;

export type ShortcutId = keyof typeof SHORTCUTS;

/** Every way to press `s`, each on its own (one for each of a pair's keys), for `useHotkey`. */
export const combosOf = (s: Shortcut): string[] => [...s.keys.flat(), ...(s.also ?? [])];

/**
 * A key as a keycap shows it: ⌘, ⇧ and ⌥ on Apple's systems, Ctrl, Shift and Alt elsewhere, ↵ for
 * Enter, as the app's hints have it everywhere, and a letter in capitals.
 */
export function keyName(key: string): string {
  const mac = apple();
  switch (key) {
    case "mod":
      return mac ? "⌘" : "Ctrl";
    case "shift":
      return mac ? "⇧" : "Shift";
    case "alt":
      return mac ? "⌥" : "Alt";
    case "enter":
      return "↵";
    case "escape":
      return "Esc";
    case "arrowup":
      return "↑";
    case "arrowdown":
      return "↓";
    case "arrowleft":
      return "←";
    case "arrowright":
      return "→";
    default:
      return key.length === 1 ? key.toUpperCase() : key;
  }
}

/**
 * The keycaps of one step: its modifier keys, then its key. A letter alone is in lower case, as the
 * app's hints have it (j, k), and in capitals after a modifier key (⌘B), as menus have it.
 */
export function chordKeys(c: Chord): string[] {
  const mods = [c.mod && "mod", c.alt && "alt", c.shift && "shift"].filter((k) => !!k) as string[];
  const key = LETTER.test(c.key) && !mods.length ? c.key : keyName(c.key);
  return [...mods.map(keyName), key];
}

/** The keycaps of each step of `combo`. */
export const comboKeys = (combo: string): string[][] => parseShortcut(combo).map(chordKeys);

/**
 * `combo` as text says it: "⌘B" on Apple's systems and "Ctrl+B" elsewhere, a letter alone in lower
 * case ("d", as tooltips on GitHub have it), a sequence step by step ("g then f").
 */
export function comboText(combo: string): string {
  return parseShortcut(combo)
    .map((c) => chordKeys(c).join(apple() ? "" : "+"))
    .join(" then ");
}

/** The first way to press `s`, as text says it: what tooltips name. */
export function shortcutText(id: ShortcutId): string {
  const first: Keys | undefined = SHORTCUTS[id].keys[0];
  return comboText((typeof first === "string" ? first : first?.[0]) ?? "");
}

/** `text`, with the shortcut `id` after it in brackets, for a tooltip: "Details (d)". */
export const withShortcut = (text: string, id: ShortcutId) => `${text} (${shortcutText(id)})`;

const ARIA: Record<string, string> = {
  enter: "Enter",
  escape: "Escape",
  arrowup: "ArrowUp",
  arrowdown: "ArrowDown",
  arrowleft: "ArrowLeft",
  arrowright: "ArrowRight",
};

/**
 * The ways to press `id`, as `aria-keyshortcuts` says them to screen readers: ⌘ as Meta and Ctrl
 * as Control. It has no way to say a sequence (a space there separates the ways), so it leaves
 * those out, and says nothing for a shortcut that is only a sequence (the sheet says them).
 */
export function ariaShortcut(id: ShortcutId): string | undefined {
  const mod = apple() ? "Meta" : "Control";
  const ways = SHORTCUTS[id].keys
    .flat()
    .map(parseShortcut)
    .filter((steps) => steps.length === 1)
    .map(([c]) =>
      c
        ? [c.mod && mod, c.alt && "Alt", c.shift && "Shift", ARIA[c.key] ?? c.key.toUpperCase()]
            .filter(Boolean)
            .join("+")
        : "",
    );
  return ways.join(" ") || undefined;
}

/** Takes every way to press the shortcut `id` while the component is there. */
export function useShortcut(
  id: ShortcutId,
  fn: (e: KeyboardEvent) => void,
  opts: Omit<HotkeyOptions, "label"> = {},
) {
  const s: Shortcut = SHORTCUTS[id];
  useHotkeys(combosOf(s), fn, { ...opts, label: s.label });
}

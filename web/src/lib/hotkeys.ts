// Keyboard shortcuts: what a key press means, on any keyboard layout, and the hooks that take
// them. lib/shortcuts.ts lists the app's own, which the keyboard shortcuts sheet shows.

import { useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react";

// On Apple's systems shortcuts take ⌘ alone: there Ctrl moves the caret in text fields
// (Ctrl+P goes up a line, Ctrl+K deletes to its end). Elsewhere they take Ctrl too, so ⌘
// still works on a Mac whose browser says it is another system.
export const apple = () => /Macintosh|iPhone|iPad|iPod/.test(navigator.userAgent);

/** ⌘ (Ctrl elsewhere) and a key, as shortcuts say it. */
export const modKey = (key: string) => (apple() ? `⌘${key}` : `Ctrl+${key}`);

/** One step of a shortcut: a key, and the modifier keys held with it. */
export interface Chord {
  /** As `KeyboardEvent.key` names it, in lower case: "k", "/", "?", "enter", "arrowup". */
  key: string;
  /** ⌘, or Ctrl except on Apple's systems. */
  mod: boolean;
  shift: boolean;
  alt: boolean;
}

/**
 * A shortcut as the code writes it: its steps one after the other, apart ("g f" is G, then F),
 * each a key after the modifier keys held with it ("mod+shift+p").
 */
export function parseShortcut(combo: string): Chord[] {
  return combo
    .trim()
    .split(/\s+/)
    .map((step) => {
      const parts = step.toLowerCase().split("+");
      return {
        key: parts[parts.length - 1] ?? "",
        mod: parts.includes("mod"),
        shift: parts.includes("shift"),
        alt: parts.includes("alt"),
      };
    });
}

const LETTER = /^[a-z]$/;

/**
 * Whether a key types a character other than a letter, such as / or ?. Which modifier keys type
 * it depends on the layout (/ is Shift+7 on a Swedish or German keyboard, \ is ⌥⇧7 on a Swedish
 * Mac), so only the character counts, and ⌘ or Ctrl.
 */
const isSymbol = (key: string) => [...key].length === 1 && !LETTER.test(key);

/**
 * Whether `e` presses `key`: the key the layout gives, or, for a letter on a layout of another
 * script (Greek, Cyrillic), or with ⌥ on a Mac (⌥C types ç), the key in its place on a US
 * keyboard, as browsers' own shortcuts do.
 */
function presses(e: KeyboardEvent, key: string, option: boolean) {
  const typed = e.key.toLowerCase();
  if (typed === key) return true;
  if (!LETTER.test(key) || e.code !== `Key${key.toUpperCase()}`) return false;
  return option || /^(?!\p{Script=Latin})\p{L}$/u.test(typed);
}

/** Whether `e` is the step `c` of a shortcut. */
export function pressed(e: KeyboardEvent, c: Chord): boolean {
  const mac = apple();
  // AltGr types characters, as ⌥ does on a Mac: on Windows it holds Ctrl and Alt down too.
  const altGraph = e.getModifierState?.("AltGraph") ?? false;
  const ctrl = e.ctrlKey && !altGraph;
  const alt = e.altKey && !altGraph;
  if (c.mod) {
    if (!(e.metaKey || (!mac && ctrl))) return false;
    // ⌃⌘ is another shortcut on a Mac (⌃⌘F, full screen).
    if (mac && e.ctrlKey) return false;
  } else if (e.metaKey || ctrl) return false;
  const symbol = isSymbol(c.key);
  if ((!symbol || c.shift) && c.shift !== e.shiftKey) return false;
  if (!symbol && c.alt !== alt) return false;
  return presses(e, c.key, c.alt && mac);
}

export interface HotkeyOptions {
  /** It works in text fields too; a shortcut with ⌘ or Ctrl always does. */
  inInputs?: boolean;
  /**
   * It works in a dialog too, whose keys are otherwise its own: in any, or in those that match a
   * selector (the details sheet that `d` opens, for `d` to put it away).
   */
  inDialogs?: boolean | string;
  /** Holding the key down repeats it, as j and k in a list do; otherwise it happens once. */
  repeat?: boolean;
  /** What it does, for the hint that shows what can follow the first step of a sequence. */
  label?: string;
}

interface Binding extends HotkeyOptions {
  steps: Chord[];
  fn: (e: KeyboardEvent) => void;
}

/** In the order they came: the newest wins a key that two take. */
const bindings: Binding[] = [];

/** How long the next step of a sequence (G, then F) is awaited. */
export const SEQUENCE_MS = 2500;

/** The first step of a sequence, pressed: its next is awaited for SEQUENCE_MS. */
let pending: Chord | undefined;
let pendingTimer: ReturnType<typeof setTimeout> | undefined;
const watchers = new Set<() => void>();

function setPending(step: Chord | undefined) {
  clearTimeout(pendingTimer);
  if (step) pendingTimer = setTimeout(() => setPending(undefined), SEQUENCE_MS);
  if (pending === step) return;
  pending = step;
  for (const w of watchers) w();
}

const MODIFIERS = new Set(["Shift", "Control", "Alt", "AltGraph", "Meta", "CapsLock", "Fn", "OS"]);
const same = (a: Chord, b: Chord) =>
  a.key === b.key && a.mod === b.mod && a.shift === b.shift && a.alt === b.alt;

/**
 * Whether keys pressed in `t` type in it: a text field or an editable region, or a list that
 * takes letters to jump to an option (Select), as a native select does.
 */
function typingIn(t: EventTarget | null): boolean {
  if (!(t instanceof HTMLElement)) return false;
  return (
    t.isContentEditable ||
    ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName) ||
    t.getAttribute("role") === "combobox"
  );
}

function onKeyDown(e: KeyboardEvent) {
  // What the page handled already (a list's arrows, a dialog's own keys), and what an input
  // method composes, are no shortcuts.
  if (e.defaultPrevented || e.isComposing || MODIFIERS.has(e.key)) return;
  // The first key of a sequence, held down, still waits for the next.
  if (e.repeat && pending && pressed(e, pending)) {
    e.preventDefault();
    return;
  }
  const first = pending;
  setPending(undefined);
  const target = e.target instanceof Element ? e.target : null;
  const typing = typingIn(target);
  const dialog = target?.closest("dialog[open]");
  const inDialog = (b: Binding) =>
    !dialog || b.inDialogs === true || (typeof b.inDialogs === "string" && dialog.matches(b.inDialogs));
  const takes = (b: Binding, step: Chord) => (!typing || !!b.inInputs || step.mod) && inDialog(b);
  const newestFirst = [...bindings].reverse();
  if (first) {
    for (const b of newestFirst) {
      const [start, next] = b.steps;
      if (b.steps.length !== 2 || !start || !next || !same(start, first)) continue;
      if (!pressed(e, next) || !takes(b, next)) continue;
      e.preventDefault();
      b.fn(e);
      return;
    }
  }
  for (const b of newestFirst) {
    const step = b.steps[0];
    if (!step || !pressed(e, step)) continue;
    if (!takes(b, step)) {
      // In a dialog the app's own ⌘ shortcuts do nothing, and neither does the browser's (⌘K,
      // which Chrome searches with, and ⌘B, Firefox's bookmarks).
      if (step.mod) e.preventDefault();
      continue;
    }
    e.preventDefault();
    if (e.repeat && !b.repeat) return;
    if (b.steps.length > 1) setPending(step);
    else b.fn(e);
    return;
  }
}

let listening = false;
function listen() {
  if (listening) return;
  listening = true;
  window.addEventListener("keydown", onKeyDown);
  // A click ends a sequence.
  window.addEventListener("pointerdown", () => setPending(undefined), true);
}

/**
 * Registers a keyboard shortcut like "mod+k", "n" or "g f" (G, then F). Keys without ⌘ or Ctrl
 * are left to the text field you type in, unless `inInputs`; all are left to an open dialog,
 * unless `inDialogs`.
 */
export function useHotkey(combo: string, fn: (e: KeyboardEvent) => void, opts: HotkeyOptions = {}) {
  useHotkeys([combo], fn, opts);
}

/** Registers each of `combos` for `fn`, as `useHotkey` does one. */
export function useHotkeys(
  combos: readonly string[],
  fn: (e: KeyboardEvent) => void,
  opts: HotkeyOptions = {},
) {
  const latest = useRef(fn);
  useLayoutEffect(() => {
    latest.current = fn;
  });
  const { inInputs, inDialogs, repeat, label } = opts;
  const key = combos.join("\n");
  useEffect(() => {
    const added: Binding[] = key
      .split("\n")
      .filter((combo) => combo.trim())
      .map((combo) => ({
        steps: parseShortcut(combo),
        fn: (e) => latest.current(e),
        inInputs,
        inDialogs,
        repeat,
        label,
      }));
    bindings.push(...added);
    listen();
    return () => {
      for (const b of added) {
        const at = bindings.indexOf(b);
        if (at >= 0) bindings.splice(at, 1);
      }
    };
  }, [key, inInputs, inDialogs, repeat, label]);
}

/** What may follow the first step of a sequence: each next step, and what it does. */
export interface Sequence {
  first: Chord;
  next: { step: Chord; label?: string }[];
}

/** The first step of a sequence, while its next is awaited, and what may follow it. */
export function useSequence(): Sequence | undefined {
  const first = useSyncExternalStore(
    (fn) => {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    () => pending,
    () => undefined,
  );
  if (!first) return undefined;
  const next: Sequence["next"] = [];
  for (const b of bindings) {
    const [start, step] = b.steps;
    if (b.steps.length === 2 && start && step && same(start, first) && !next.some((n) => same(n.step, step)))
      next.push({ step, label: b.label });
  }
  return { first, next };
}

/** Ends the sequence awaited, as Escape does. */
export const endSequence = () => setPending(undefined);

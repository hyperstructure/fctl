// The home page's task box takes the focus as the page opens, so that a task
// can be typed right away, and whenever ⌘K's Home asks for it. Phones and
// tablets keep their keyboard down until the box is tapped.

import { type RefObject, useEffect } from "react";

/** Focuses the task box the home page shows, while it shows one. */
let focusShown: (() => void) | undefined;
/** ⌘K asked for the task box before the page showed it. */
let asked = false;

/** Focuses the task box: now if the home page shows it, else as soon as it does. */
export function focusTaskBox() {
  if (focusShown) focusShown();
  else asked = true;
}

/**
 * Makes `ref` the task box while `shown`: it takes the focus as it shows, unless the
 * keyboard is on the screen or you are typing in another field, and when ⌘K asks.
 */
export function useTaskBoxFocus(ref: RefObject<HTMLTextAreaElement | null>, shown: boolean) {
  useEffect(() => {
    if (!shown) return;
    const focus = () => ref.current?.focus({ preventScroll: true });
    if (asked || !(touchScreen() || typingElsewhere(ref.current))) focus();
    asked = false;
    focusShown = focus;
    return () => {
      if (focusShown === focus) focusShown = undefined;
    };
  }, [ref, shown]);
}

/** A phone or tablet, where a focused field raises the keyboard over the page. */
function touchScreen(): boolean {
  return typeof matchMedia === "function" && matchMedia("(pointer: coarse)").matches;
}

/** Whether a field other than `box` has the focus, like ⌘K's search. */
function typingElsewhere(box: HTMLElement | null): boolean {
  const el = document.activeElement;
  return (
    el instanceof HTMLElement && el !== box && (el.isContentEditable || el.matches("input, textarea, select"))
  );
}

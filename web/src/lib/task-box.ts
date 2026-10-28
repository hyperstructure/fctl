// The Agent Floor's task box takes the focus as the page opens, so that a task
// can be typed right away, and whenever ⌘K's Agent Floor asks for it. Phones and
// tablets keep their keyboard down until the box is tapped. On a Mac, ⌘← and ⌘→
// still go back and forward from the box while it is empty.

import { type RefObject, useEffect } from "react";
import { apple } from "./hotkeys";

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
 * keyboard is on the screen or you are typing in another field, and when ⌘K asks. While
 * it is empty, ⌘← and ⌘→ in it go back and forward (`historyKey`).
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
  useEffect(() => {
    const box = ref.current;
    if (!shown || !box) return;
    const onKey = (e: KeyboardEvent) => {
      const go = historyKey(e, box.value);
      if (!go) return;
      e.preventDefault();
      history[go]();
    };
    box.addEventListener("keydown", onKey);
    return () => box.removeEventListener("keydown", onKey);
  }, [ref, shown]);
}

/**
 * Where a key pressed in the task box, which holds `text`, takes the browser's history.
 * Browsers on a Mac go back with ⌘← and forward with ⌘→, but in a text field those move
 * the caret to the start or the end of its line, and the box has the focus as soon as the
 * Agent Floor opens. While it is empty, with no caret to move, they go back and forward
 * from it too; with text in it, they move its caret, as in any field.
 */
function historyKey(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "isComposing">,
  text: string,
): "back" | "forward" | undefined {
  if (text || e.isComposing || !e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || !apple()) return;
  return e.key === "ArrowLeft" ? "back" : e.key === "ArrowRight" ? "forward" : undefined;
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

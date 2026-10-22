import { useEffect } from "react";

// On Apple's systems shortcuts take ⌘ alone: there Ctrl moves the caret in text fields
// (Ctrl+P goes up a line, Ctrl+K deletes to its end). Elsewhere they take Ctrl too, so ⌘
// still works on a Mac whose browser says it is another system.
const apple = () => /Macintosh|iPhone|iPad|iPod/.test(navigator.userAgent);

/**
 * Registers a keyboard shortcut like "mod+k" or "n" (ignored while typing, unless `inInputs`).
 * "mod" is ⌘, or Ctrl except on Apple's systems.
 */
export function useHotkey(combo: string, fn: (e: KeyboardEvent) => void, opts: { inInputs?: boolean } = {}) {
  useEffect(() => {
    const parts = combo.toLowerCase().split("+");
    const key = parts[parts.length - 1];
    const mod = parts.includes("mod");
    const shift = parts.includes("shift");
    const ctrl = !apple();
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      // A combobox (Select) takes letters to jump to an option, as a native select does.
      const typing =
        !!t &&
        (t.isContentEditable ||
          ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName) ||
          t.getAttribute("role") === "combobox");
      if (typing && !opts.inInputs && !mod) return;
      if (mod ? !(e.metaKey || (ctrl && e.ctrlKey)) : e.metaKey || e.ctrlKey) return;
      if (shift !== e.shiftKey) return;
      if (e.key.toLowerCase() !== key) return;
      e.preventDefault();
      fn(e);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [combo, fn, opts.inInputs]);
}

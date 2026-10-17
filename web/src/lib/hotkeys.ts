import { useEffect } from "react";

/** Registers a keyboard shortcut like "mod+k" or "n" (ignored while typing, unless `inInputs`). */
export function useHotkey(combo: string, fn: (e: KeyboardEvent) => void, opts: { inInputs?: boolean } = {}) {
  useEffect(() => {
    const parts = combo.toLowerCase().split("+");
    const key = parts[parts.length - 1];
    const mod = parts.includes("mod");
    const shift = parts.includes("shift");
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = !!t && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));
      if (typing && !opts.inInputs && !mod) return;
      if (mod !== (e.metaKey || e.ctrlKey)) return;
      if (shift !== e.shiftKey) return;
      if (e.key.toLowerCase() !== key) return;
      e.preventDefault();
      fn(e);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [combo, fn, opts.inInputs]);
}

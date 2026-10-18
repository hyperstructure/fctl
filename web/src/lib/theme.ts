// Light and dark themes (the pre-paint script in public/theme.js applies the saved one).
import { palette } from "@hyperstructure/design/brand";
import { useSyncExternalStore } from "react";

type Theme = "light" | "dark";
const listeners = new Set<() => void>();

export function getTheme(): Theme {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

export function setTheme(t: Theme) {
  const apply = () => {
    document.documentElement.dataset.theme = t;
    // The browser's and the installed app's title bar take the canvas color.
    document
      .querySelector<HTMLMetaElement>('meta[name="theme-color"]')
      ?.setAttribute("content", t === "dark" ? palette.blackish : palette.whiteish);
    try {
      localStorage.setItem("fctl-theme", t);
    } catch {}
    for (const l of listeners) l();
  };
  const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
  if (doc.startViewTransition && !matchMedia("(prefers-reduced-motion: reduce)").matches)
    doc.startViewTransition(apply);
  else apply();
}

export function useTheme(): [Theme, (t: Theme) => void] {
  const t = useSyncExternalStore((fn) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  }, getTheme);
  return [t, setTheme];
}

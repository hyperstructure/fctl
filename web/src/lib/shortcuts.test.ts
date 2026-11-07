import { afterEach, describe, expect, it, vi } from "vitest";
import { parseShortcut } from "./hotkeys";
import {
  ariaShortcut,
  comboKeys,
  combosOf,
  comboText,
  GROUPS,
  type GroupId,
  SHORTCUTS,
  type Shortcut,
  shortcutText,
  withShortcut,
} from "./shortcuts";

const MAC = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0";
const on = (ua: string) => vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);

afterEach(() => {
  vi.restoreAllMocks();
});

const all = Object.entries(SHORTCUTS) as [string, Shortcut][];

describe("how shortcuts are said", () => {
  it("names ⌘, ⇧ and ⌥ on a Mac, and Ctrl, Shift and Alt elsewhere", () => {
    on(MAC);
    expect(comboKeys("mod+shift+o")).toEqual([["⌘", "⇧", "O"]]);
    expect(comboText("mod+b")).toBe("⌘B");
    expect(comboText("alt+c")).toBe("⌥C");
    on(LINUX);
    expect(comboKeys("mod+shift+o")).toEqual([["Ctrl", "Shift", "O"]]);
    expect(comboText("mod+b")).toBe("Ctrl+B");
  });

  it("says a letter alone in lower case, a sequence step by step, and named keys by their symbols", () => {
    on(LINUX);
    expect(comboText("d")).toBe("d");
    expect(comboText("g f")).toBe("g then f");
    expect(comboKeys("g f")).toEqual([["g"], ["f"]]);
    expect(comboText("?")).toBe("?");
    expect(comboKeys("shift+enter")).toEqual([["Shift", "↵"]]);
    expect(comboKeys("escape")).toEqual([["Esc"]]);
    expect(comboKeys("arrowleft")).toEqual([["←"]]);
  });

  it("names a shortcut's first keys in tooltips", () => {
    on(MAC);
    expect(shortcutText("fold")).toBe("⌘B");
    expect(shortcutText("newSession")).toBe("n");
    expect(withShortcut("Details", "details")).toBe("Details (d)");
    expect(shortcutText("nextFile")).toBe("j");
  });

  it("says every way to press one to screen readers", () => {
    on(MAC);
    expect(ariaShortcut("newSession")).toBe("N Meta+Shift+O");
    // It has no way to say G, then F: a space there separates the ways to press it.
    expect(ariaShortcut("floor")).toBeUndefined();
    on(LINUX);
    expect(ariaShortcut("help")).toBe("? Control+/");
    expect(ariaShortcut("send")).toBe("Enter Shift+Enter");
  });
});

describe("the shortcuts", () => {
  it("each have a group the sheet shows, and keys it can say", () => {
    const groups = new Set<string>(GROUPS.map((g) => g.id));
    for (const [id, s] of all) {
      expect(groups.has(s.group), id).toBe(true);
      for (const combo of combosOf(s)) {
        for (const step of parseShortcut(combo)) expect(step.key, `${id}: ${combo}`).not.toBe("");
      }
    }
  });

  it("take no keys another takes where both work", () => {
    // What works anywhere and the page's own work together; a dialog's keys are its own.
    const together: GroupId[][] = [["anywhere", "go", "session"], ["anywhere", "go", "pulls"], ["changes"]];
    for (const groups of together) {
      const seen = new Map<string, string>();
      for (const [id, s] of all) {
        if (!groups.includes(s.group)) continue;
        for (const combo of combosOf(s)) {
          expect(seen.get(combo), `${id} and ${seen.get(combo)} both take ${combo}`).toBeUndefined();
          seen.set(combo, id);
        }
      }
      // A key alone that a sequence starts with would never wait for the sequence's next.
      const firsts = [...seen.keys()].filter((c) => c.includes(" ")).map((c) => c.split(" ")[0]);
      for (const first of firsts) expect(seen.has(first ?? ""), first).toBe(false);
    }
  });

  it("need no Shift, ⌥ or AltGr for their letters on any layout", () => {
    // Letters are where every Latin layout has them; only ? and / take Shift on some layouts,
    // which the keys take as characters (lib/hotkeys.ts).
    for (const [id, s] of all) {
      if (!["anywhere", "go", "session"].includes(s.group)) continue;
      for (const combo of combosOf(s)) {
        for (const step of parseShortcut(combo)) {
          expect(/^[a-z]$|^[?/\\]$|^(enter|escape)$/.test(step.key), `${id}: ${combo}`).toBe(true);
          expect(step.alt, `${id}: ${combo}`).toBe(false);
        }
      }
    }
  });
});

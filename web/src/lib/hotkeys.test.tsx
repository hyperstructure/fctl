import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  endSequence,
  type HotkeyOptions,
  SEQUENCE_MS,
  type Sequence,
  useHotkey,
  useSequence,
} from "./hotkeys";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.restoreAllMocks();
});

function Shortcut({ combo, fn, opts }: { combo: string; fn: () => void; opts: HotkeyOptions }) {
  useHotkey(combo, fn, opts);
  return <textarea aria-label="Task for a new session" />;
}

/** `combo` registered on a page with a text field. */
function page(combo: string, opts: HotkeyOptions = {}) {
  const fn = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Shortcut combo={combo} fn={fn} opts={opts} />));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const field = container.querySelector("textarea");
  if (!field) throw new Error("no text field");
  return { fn, field, press };
}

/** A key pressed in `target`, with `mods`; with AltGr held, as Windows says it, when `altGraph`. */
function press(
  key: string,
  mods: KeyboardEventInit & { altGraph?: boolean } = {},
  target: Element = document.body,
) {
  const { altGraph, ...init } = mods;
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  // happy-dom says AltGraph whenever Alt is down; browsers, for AltGr only.
  const state = e.getModifierState.bind(e);
  Object.defineProperty(e, "getModifierState", {
    value: (k: string) => (k === "AltGraph" ? !!altGraph : state(k)),
  });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}

const LINUX =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const WINDOWS =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const MAC =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const IPHONE =
  "Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";
const on = (ua: string) => vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);

describe("useHotkey", () => {
  it.each([
    ["Linux", LINUX],
    ["Windows", WINDOWS],
  ])("takes ⌘ or Ctrl for mod on %s", (_, ua) => {
    on(ua);
    const { fn, press } = page("mod+k");
    expect(press("k", { metaKey: true }).defaultPrevented).toBe(true);
    expect(press("k", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(fn).toHaveBeenCalledTimes(2);
    expect(press("k").defaultPrevented).toBe(false);
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("takes only ⌘ for mod on a Mac, where Ctrl moves the caret in text fields", () => {
    on(MAC);
    const { fn, field, press } = page("mod+p", { inInputs: true });
    expect(press("p", { metaKey: true }, field).defaultPrevented).toBe(true);
    expect(fn).toHaveBeenCalledTimes(1);
    // Ctrl+P goes up a line.
    expect(press("p", { ctrlKey: true }, field).defaultPrevented).toBe(false);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("takes only ⌘ for mod on an iPhone or iPad", () => {
    on(IPHONE);
    const { fn, press } = page("mod+k");
    press("k", { ctrlKey: true });
    expect(fn).not.toHaveBeenCalled();
    press("k", { metaKey: true });
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("tells shift apart", () => {
    const plain = page("mod+p");
    const shifted = page("mod+shift+p");
    // With Shift, the key is the capital letter.
    plain.press("P", { metaKey: true, shiftKey: true });
    expect(plain.fn).not.toHaveBeenCalled();
    expect(shifted.fn).toHaveBeenCalledTimes(1);
    plain.press("p", { metaKey: true });
    expect(plain.fn).toHaveBeenCalledTimes(1);
    expect(shifted.fn).toHaveBeenCalledTimes(1);
  });

  it("takes a letter where a layout of another script has it", () => {
    const { fn, press } = page("mod+b");
    // A Russian layout gives и where a US one gives b; a Greek one, β.
    press("и", { ctrlKey: true, code: "KeyB" });
    press("β", { ctrlKey: true, code: "KeyB" });
    expect(fn).toHaveBeenCalledTimes(2);
    // Another key of that layout is another shortcut.
    press("ш", { ctrlKey: true, code: "KeyI" });
    expect(fn).toHaveBeenCalledTimes(2);
  });

  it("takes a letter where the layout has it, not where a US keyboard does", () => {
    const { fn, press } = page("mod+m");
    // On a French layout, the key in M's place gives a comma, and M is beside L.
    press(",", { ctrlKey: true, code: "KeyM" });
    expect(fn).not.toHaveBeenCalled();
    press("m", { ctrlKey: true, code: "Semicolon" });
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("leaves a key without mod to the field you are typing in, unless asked", () => {
    const { fn, field, press } = page("n");
    expect(press("n", {}, field).defaultPrevented).toBe(false);
    expect(fn).not.toHaveBeenCalled();
    press("n");
    expect(fn).toHaveBeenCalledTimes(1);
    const typing = page("n", { inInputs: true });
    typing.press("n", {}, typing.field);
    expect(typing.fn).toHaveBeenCalledTimes(1);
  });

  it("leaves a key without mod alone when ⌘ or Ctrl is held", () => {
    const { fn, press } = page("n");
    press("n", { metaKey: true });
    press("n", { ctrlKey: true });
    expect(fn).not.toHaveBeenCalled();
  });

  it("leaves a key without mod alone when Ctrl is held on a Mac", () => {
    on(MAC);
    const { fn, press } = page("n");
    press("n", { ctrlKey: true });
    expect(fn).not.toHaveBeenCalled();
  });
});

describe("useHotkey on other keyboard layouts", () => {
  it("takes a character however the layout types it", () => {
    const help = page("?");
    const filter = page("/");
    // ? is Shift and + on a Swedish keyboard, and / is Shift and 7, as on a German one.
    press("?", { shiftKey: true, code: "Minus" });
    press("/", { shiftKey: true, code: "Digit7" });
    // Where / is a key of its own, as on a US keyboard.
    press("/", { code: "Slash" });
    expect(help.fn).toHaveBeenCalledTimes(1);
    expect(filter.fn).toHaveBeenCalledTimes(2);
  });

  it("takes ⌘ or Ctrl with a character that takes Shift", () => {
    const { fn } = page("mod+/");
    press("/", { ctrlKey: true, shiftKey: true, code: "Digit7" });
    expect(fn).toHaveBeenCalledTimes(1);
    press("7", { ctrlKey: true, shiftKey: true, code: "Digit7" });
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("takes a character typed with ⌥ on a Mac, or AltGr elsewhere", () => {
    on(MAC);
    const fold = page("mod+\\");
    // \ is ⌥⇧7 on a Swedish Mac.
    press("\\", { metaKey: true, altKey: true, shiftKey: true, code: "Digit7" });
    expect(fold.fn).toHaveBeenCalledTimes(1);
    vi.restoreAllMocks();
    on(WINDOWS);
    const slash = page("\\");
    // AltGr and + on a Swedish keyboard: Windows says Ctrl and Alt are down.
    press("\\", { ctrlKey: true, altKey: true, altGraph: true, code: "Minus" });
    expect(slash.fn).toHaveBeenCalledTimes(1);
    // Ctrl and Alt themselves are no AltGr.
    press("\\", { ctrlKey: true, altKey: true, code: "Backslash" });
    expect(slash.fn).toHaveBeenCalledTimes(1);
  });

  it("leaves a letter typed with AltGr or Alt alone", () => {
    on(WINDOWS);
    const b = page("mod+b");
    const n = page("n");
    press("b", { ctrlKey: true, altKey: true, altGraph: true, code: "KeyB" });
    press("n", { altKey: true, code: "KeyN" });
    expect(b.fn).not.toHaveBeenCalled();
    expect(n.fn).not.toHaveBeenCalled();
  });
});

describe("sequences", () => {
  it("take their keys one after the other", () => {
    const floor = page("g f");
    const pulls = page("g p");
    expect(press("g").defaultPrevented).toBe(true);
    press("f");
    expect(floor.fn).toHaveBeenCalledTimes(1);
    press("f");
    expect(floor.fn).toHaveBeenCalledTimes(1);
    press("g");
    press("p");
    expect(pulls.fn).toHaveBeenCalledTimes(1);
  });

  it("wait for the next key while their first is held down", () => {
    const floor = page("g f");
    press("g");
    expect(press("g", { repeat: true }).defaultPrevented).toBe(true);
    press("g", { repeat: true });
    press("f");
    expect(floor.fn).toHaveBeenCalledTimes(1);
  });

  it("take a key that follows none as itself", () => {
    const floor = page("g f");
    const fresh = page("n");
    press("g");
    press("n");
    expect(fresh.fn).toHaveBeenCalledTimes(1);
    press("f");
    expect(floor.fn).not.toHaveBeenCalled();
  });

  it("wait for the next key a while, and not after a click", () => {
    vi.useFakeTimers();
    try {
      const floor = page("g f");
      press("g");
      act(() => vi.advanceTimersByTime(SEQUENCE_MS + 1));
      press("f");
      expect(floor.fn).not.toHaveBeenCalled();
      press("g");
      // Shift on its own, before a capital, isn't the next key.
      press("Shift", { shiftKey: true });
      act(() => {
        window.dispatchEvent(new Event("pointerdown"));
      });
      press("f");
      expect(floor.fn).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("say what can follow their first key", () => {
    const seen: (Sequence | undefined)[] = [];
    function Watch() {
      seen.push(useSequence());
      return null;
    }
    page("g f", { label: "Agent Floor" });
    page("g p", { label: "Pull requests" });
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() => root.render(<Watch />));
    cleanups.push(() => act(() => root.unmount()));
    press("g");
    expect(seen.at(-1)?.next.map((n) => [n.step.key, n.label])).toEqual([
      ["f", "Agent Floor"],
      ["p", "Pull requests"],
    ]);
    act(() => endSequence());
    expect(seen.at(-1)).toBeUndefined();
  });
});

describe("what useHotkey leaves alone", () => {
  it("takes a key in the dialogs it names", () => {
    const { fn } = page("d", { inDialogs: "dialog.sheet" });
    const open = (className: string) => {
      const dialog = document.createElement("dialog");
      dialog.className = className;
      dialog.setAttribute("open", "");
      document.body.append(dialog);
      cleanups.push(() => dialog.remove());
      return dialog;
    };
    press("d", {}, open("sheet"));
    expect(fn).toHaveBeenCalledTimes(1);
    expect(press("d", {}, open("other")).defaultPrevented).toBe(false);
    expect(fn).toHaveBeenCalledTimes(1);
  });

  it("takes a key on the page's body as the open dialog's, where a click on its text leaves the focus", () => {
    const fn = page("n");
    const sheet = page("d", { inDialogs: "dialog.sheet" });
    const dialog = document.createElement("dialog");
    dialog.className = "sheet";
    dialog.setAttribute("open", "");
    document.body.append(dialog);
    cleanups.push(() => dialog.remove());
    expect(press("n").defaultPrevented).toBe(false);
    expect(fn.fn).not.toHaveBeenCalled();
    press("d");
    expect(sheet.fn).toHaveBeenCalledTimes(1);
  });

  it("leaves a dialog's keys to it, unless asked", () => {
    const fn = page("n");
    const help = page("?", { inDialogs: true });
    const dialog = document.createElement("dialog");
    dialog.setAttribute("open", "");
    const inside = document.createElement("button");
    dialog.append(inside);
    document.body.append(dialog);
    cleanups.push(() => dialog.remove());
    expect(press("n", {}, inside).defaultPrevented).toBe(false);
    press("?", { shiftKey: true }, inside);
    expect(fn.fn).not.toHaveBeenCalled();
    expect(help.fn).toHaveBeenCalledTimes(1);
  });

  it("keeps the browser's own from a ⌘ shortcut that does nothing in a dialog", () => {
    const palette = page("mod+k");
    const dialog = document.createElement("dialog");
    dialog.setAttribute("open", "");
    const field = document.createElement("input");
    dialog.append(field);
    document.body.append(dialog);
    cleanups.push(() => dialog.remove());
    expect(press("k", { ctrlKey: true }, field).defaultPrevented).toBe(true);
    expect(palette.fn).not.toHaveBeenCalled();
  });

  it("leaves a key the page handled already", () => {
    const { fn } = page("j");
    const list = document.createElement("div");
    list.addEventListener("keydown", (e) => e.preventDefault());
    document.body.append(list);
    cleanups.push(() => list.remove());
    press("j", {}, list);
    expect(fn).not.toHaveBeenCalled();
  });

  it("takes a key held down once, unless it repeats", () => {
    const fold = page("mod+b");
    const next = page("j", { repeat: true });
    press("b", { ctrlKey: true });
    expect(press("b", { ctrlKey: true, repeat: true }).defaultPrevented).toBe(true);
    press("j");
    press("j", { repeat: true });
    expect(fold.fn).toHaveBeenCalledTimes(1);
    expect(next.fn).toHaveBeenCalledTimes(2);
  });

  it("gives a key two take to the newest", () => {
    const page1 = page("c");
    const page2 = page("c");
    press("c");
    expect(page1.fn).not.toHaveBeenCalled();
    expect(page2.fn).toHaveBeenCalledTimes(1);
  });
});

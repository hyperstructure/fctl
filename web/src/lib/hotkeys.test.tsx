import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useHotkey } from "./hotkeys";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.restoreAllMocks();
});

function Shortcut({ combo, fn, inInputs }: { combo: string; fn: () => void; inInputs?: boolean }) {
  useHotkey(combo, fn, { inInputs });
  return <textarea aria-label="Task for a new session" />;
}

/** `combo` registered on a page with a text field. */
function page(combo: string, opts: { inInputs?: boolean } = {}) {
  const fn = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Shortcut combo={combo} fn={fn} inInputs={opts.inInputs} />));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const field = container.querySelector("textarea");
  if (!field) throw new Error("no text field");
  const press = (key: string, mods: KeyboardEventInit = {}, target: Element = document.body) => {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods });
    target.dispatchEvent(e);
    return e;
  };
  return { fn, field, press };
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

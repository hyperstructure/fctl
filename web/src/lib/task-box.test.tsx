import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { focusTaskBox, useTaskBoxFocus } from "./task-box";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function TaskBox({ shown = true }: { shown?: boolean }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useTaskBoxFocus(ref, shown);
  return shown ? <textarea ref={ref} aria-label="Task for a new session" /> : null;
}

/** The home page's task box, in the document; `show` re-renders it shown or not. */
function homePage(shown = true) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = (s: boolean) => act(() => root.render(<TaskBox shown={s} />));
  show(shown);
  const box = () => container.querySelector("textarea");
  const leave = () => {
    act(() => root.unmount());
    container.remove();
  };
  cleanups.push(() => container.isConnected && leave());
  return { box, show, leave };
}

/** A phone or tablet: its primary pointer is a finger. */
function touchScreen() {
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: query === "(pointer: coarse)", media: query }));
}

const focused = () => document.activeElement;

describe("the home page's task box", () => {
  it("takes the focus as the page opens", () => {
    const { box } = homePage();
    expect(box()).not.toBeNull();
    expect(focused()).toBe(box());
  });

  it("takes it when it shows after the page opened", () => {
    const { box, show } = homePage(false);
    expect(box()).toBeNull();
    show(true);
    expect(focused()).toBe(box());
  });

  it("leaves the focus in a field you are typing in", () => {
    const search = document.createElement("input");
    document.body.append(search);
    cleanups.push(() => search.remove());
    search.focus();
    const { box } = homePage();
    expect(box()).not.toBeNull();
    expect(focused()).toBe(search);
  });

  it("waits for a tap on a phone, so that its keyboard stays down", () => {
    touchScreen();
    const { box } = homePage();
    expect(box()).not.toBeNull();
    expect(focused()).not.toBe(box());
  });

  it("takes the focus when ⌘K asks while the page shows it", () => {
    touchScreen();
    const { box } = homePage();
    expect(focused()).not.toBe(box());
    focusTaskBox();
    expect(focused()).toBe(box());
  });

  it("takes the focus when ⌘K asked on the way to the page, once", () => {
    touchScreen();
    focusTaskBox();
    const first = homePage();
    expect(focused()).toBe(first.box());
    first.leave();
    const again = homePage();
    expect(again.box()).not.toBeNull();
    expect(focused()).not.toBe(again.box());
  });
});

const MAC =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const WINDOWS =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/** The home page on a system whose browser says `ua`, with the browser's back and forward watched. */
function homePageOn(ua: string) {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(ua);
  const back = vi.spyOn(history, "back").mockImplementation(() => {});
  const forward = vi.spyOn(history, "forward").mockImplementation(() => {});
  const page = homePage();
  /** Presses `key` in the task box: whether the page took it from the box. */
  const press = (key: string, mods: KeyboardEventInit) => {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods });
    page.box()?.dispatchEvent(e);
    return e.defaultPrevented;
  };
  return { ...page, back, forward, press };
}

describe("⌘← and ⌘→ in the task box", () => {
  it("go back and forward on a Mac while it is empty, as they do outside it", () => {
    const { box, back, forward, press } = homePageOn(MAC);
    expect(focused()).toBe(box());
    expect(press("ArrowLeft", { metaKey: true })).toBe(true);
    expect(back).toHaveBeenCalledTimes(1);
    expect(press("ArrowRight", { metaKey: true })).toBe(true);
    expect(forward).toHaveBeenCalledTimes(1);
  });

  it("move the caret once something is written in it", () => {
    const { box, back, forward, press } = homePageOn(MAC);
    const field = box();
    if (!field) throw new Error("no task box");
    field.value = "Fix the login test";
    expect(press("ArrowLeft", { metaKey: true })).toBe(false);
    expect(press("ArrowRight", { metaKey: true })).toBe(false);
    expect(back).not.toHaveBeenCalled();
    expect(forward).not.toHaveBeenCalled();
  });

  it("leave the box its other keys: ⇧ to select, ⌥ to move by word, an input method's", () => {
    const { back, forward, press } = homePageOn(MAC);
    expect(press("ArrowLeft", { metaKey: true, shiftKey: true })).toBe(false);
    expect(press("ArrowLeft", { altKey: true })).toBe(false);
    expect(press("ArrowRight", { metaKey: true, isComposing: true })).toBe(false);
    expect(press("ArrowLeft", {})).toBe(false);
    expect(back).not.toHaveBeenCalled();
    expect(forward).not.toHaveBeenCalled();
  });

  it("are left alone on other systems, whose browsers go back with Alt+← even in a field", () => {
    const { back, press } = homePageOn(WINDOWS);
    expect(press("ArrowLeft", { metaKey: true })).toBe(false);
    expect(press("ArrowLeft", { altKey: true })).toBe(false);
    expect(back).not.toHaveBeenCalled();
  });
});

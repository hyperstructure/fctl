import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { focusTaskBox, useTaskBoxFocus } from "./task-box";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.unstubAllGlobals();
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

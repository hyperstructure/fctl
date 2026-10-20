import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { keyDirection, NEAR_BOTTOM, Stick, scrollsInside, useStickToBottom } from "./stick-to-bottom";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("sticking to the bottom", () => {
  /** A page stuck at its bottom, 1000 px down, as the hook leaves it after a step. */
  const atBottom = () => {
    const s = new Stick();
    s.pinned(1000);
    s.scrolled(1000, 0);
    return s;
  };

  it("starts stuck", () => {
    expect(new Stick().stuck).toBe(true);
  });

  it("lets go at the reader's first step up, however small", () => {
    const s = atBottom();
    s.scrolled(994, 6);
    expect(s.stuck).toBe(false);
    // It used to hold on for 160 px, and pulled the page back down at every word.
    s.scrolled(990, 10 + 26);
    expect(s.stuck).toBe(false);
  });

  it("lets go as the wheel turns up, before the page moves", () => {
    const s = atBottom();
    s.asked(-1, 1000, 0);
    expect(s.stuck).toBe(false);
    // The scroll event of the page's last step to the bottom comes after: it isn't the reader's.
    s.scrolled(1000, 0);
    expect(s.stuck).toBe(false);
  });

  it("stays stuck when the page can't scroll up", () => {
    const s = new Stick();
    s.asked(-1, 0, 0);
    expect(s.stuck).toBe(true);
  });

  it("sticks again when the reader scrolls down near the bottom", () => {
    const s = atBottom();
    s.scrolled(700, 300);
    s.scrolled(900, 100);
    expect(s.stuck).toBe(false);
    // A line came in as the page got there.
    s.scrolled(1000, 26);
    expect(s.stuck).toBe(true);
  });

  it("sticks again on a step down at the bottom, where the page can't move", () => {
    const s = atBottom();
    s.asked(-1, 1000, 0);
    s.asked(1, 1000, 0);
    expect(s.stuck).toBe(true);
    s.scrolled(700, 300);
    s.asked(1, 700, NEAR_BOTTOM + 1);
    expect(s.stuck).toBe(false);
  });

  it("sticks again on End, wherever the page is", () => {
    // The browser's scroll to the bottom may stop short when the page grows on the way.
    const s = atBottom();
    s.scrolled(500, 500);
    s.asked(Number.POSITIVE_INFINITY, 500, 500);
    expect(s.stuck).toBe(true);
  });

  it("doesn't count the browser keeping what is in view in place", () => {
    // Something above the view grew: the page moves down, its bottom with it.
    const s = atBottom();
    s.scrolled(700, 30);
    expect(s.stuck).toBe(false);
    s.scrolled(760, 30);
    expect(s.stuck).toBe(false);
    // Something above the view shrank, while the page was stuck.
    const t = atBottom();
    t.scrolled(950, 0);
    expect(t.stuck).toBe(true);
  });

  it("stays stuck when the page gets shorter at the bottom, or rounds", () => {
    const s = atBottom();
    s.scrolled(940, 0);
    expect(s.stuck).toBe(true);
    s.scrolled(939.5, 0.5);
    expect(s.stuck).toBe(true);
  });

  it("doesn't take its own step to the bottom for the reader's", () => {
    const s = atBottom();
    s.scrolled(980, 20);
    expect(s.stuck).toBe(false);
    s.asked(1, 980, 20);
    // The page grew 300 px, and the hook stepped to the new bottom.
    s.pinned(1300);
    s.scrolled(1300, 0);
    expect(s.stuck).toBe(true);
  });
});

describe("keys", () => {
  /** The keydown event `key` makes on `on` (the page by default), once dispatched. */
  function press(key: string, on: Element = document.body, init: KeyboardEventInit = {}) {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
    on.dispatchEvent(e);
    return e;
  }
  function el(html: string) {
    const host = document.createElement("div");
    host.innerHTML = html;
    document.body.append(host);
    cleanups.push(() => host.remove());
    return host.firstElementChild as HTMLElement;
  }

  it("scroll the page up or down", () => {
    for (const k of ["ArrowUp", "PageUp", "Home"]) expect(keyDirection(press(k))).toBe(-1);
    for (const k of ["ArrowDown", "PageDown"]) expect(keyDirection(press(k))).toBe(1);
    expect(keyDirection(press("End"))).toBe(Number.POSITIVE_INFINITY);
    expect(keyDirection(press("ArrowDown", document.body, { metaKey: true }))).toBe(Number.POSITIVE_INFINITY);
    expect(keyDirection(press(" "))).toBe(1);
    expect(keyDirection(press(" ", document.body, { shiftKey: true }))).toBe(-1);
    expect(keyDirection(press("a"))).toBe(0);
  });

  it("don't scroll it while typing, or when the page took them", () => {
    for (const html of ["<textarea></textarea>", "<input>", '<div contenteditable="true"></div>']) {
      expect(keyDirection(press("ArrowUp", el(html)))).toBe(0);
    }
    expect(keyDirection(press("ArrowUp", el('<button role="combobox"></button>')))).toBe(0);
    const taken = el("<div></div>");
    taken.addEventListener("keydown", (e) => e.preventDefault());
    expect(keyDirection(press("ArrowUp", taken))).toBe(0);
  });

  it("press a button with the space bar instead of scrolling", () => {
    expect(keyDirection(press(" ", el("<button></button>")))).toBe(0);
    expect(keyDirection(press("PageUp", el("<button></button>")))).toBe(-1);
  });
});

describe("a box that scrolls inside the page", () => {
  /** A box `room` px taller inside than out, scrolled `top` px down, with a line in it. */
  function box(overflow: string, room: number, top: number) {
    const b = document.createElement("div");
    b.style.overflowY = overflow;
    Object.defineProperties(b, {
      clientHeight: { value: 200 },
      scrollHeight: { value: 200 + room },
      scrollTop: { value: top },
    });
    const line = document.createElement("span");
    b.append(line);
    document.body.append(b);
    cleanups.push(() => b.remove());
    return line;
  }

  it("takes a scroll it has room for", () => {
    expect(scrollsInside(box("auto", 300, 100), -1)).toBe(true);
    expect(scrollsInside(box("scroll", 300, 100), 1)).toBe(true);
  });

  it("passes it on to the page at its end", () => {
    expect(scrollsInside(box("auto", 300, 0), -1)).toBe(false);
    expect(scrollsInside(box("auto", 300, 300), 1)).toBe(false);
  });

  it("isn't a box that scrolls unless it says so", () => {
    expect(scrollsInside(box("visible", 300, 100), -1)).toBe(false);
    expect(scrollsInside(document.body, -1)).toBe(false);
    expect(scrollsInside(null, -1)).toBe(false);
  });
});

describe("the session page", () => {
  /**
   * A page 800 px tall whose transcript the hook follows, with a scroll bar
   * the test moves: `grow` adds to the transcript as the agent's work does.
   */
  function sessionPage() {
    const view = 800;
    let top = 0;
    let height = 2000;
    Object.defineProperty(window, "innerHeight", { configurable: true, get: () => view });
    Object.defineProperty(window, "scrollY", { configurable: true, get: () => top });
    Object.defineProperty(document.documentElement, "scrollHeight", {
      configurable: true,
      get: () => height,
    });
    cleanups.push(() => {
      for (const k of ["innerHeight", "scrollY"]) Reflect.deleteProperty(window, k);
      Reflect.deleteProperty(document.documentElement, "scrollHeight");
    });
    const scroll = (to: number) => {
      top = Math.max(0, Math.min(to, height - view));
      window.dispatchEvent(new Event("scroll"));
    };
    vi.spyOn(window, "scrollTo").mockImplementation(((o: ScrollToOptions) => {
      top = Math.max(0, Math.min(o.top ?? 0, height - view));
    }) as typeof window.scrollTo);
    let resized: ResizeObserverCallback | undefined;
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: ResizeObserverCallback) {
          resized = cb;
        }
        observe() {}
        disconnect() {}
      },
    );

    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    function Page({ id }: { id: string }) {
      const transcript = useRef<HTMLDivElement>(null);
      useStickToBottom(transcript, id);
      return (
        <div ref={transcript}>
          <button type="button">Ran ls -la</button>
        </div>
      );
    }
    const show = (id: string) => act(() => root.render(<Page id={id} />));
    show("ses_1");
    const leave = () => {
      act(() => root.unmount());
      container.remove();
    };
    cleanups.push(() => container.isConnected && leave());
    const grow = (by: number) => {
      height += by;
      act(() => resized?.([{ contentRect: { height } } as ResizeObserverEntry], {} as ResizeObserver));
    };
    grow(0);
    const gap = () => height - top - view;
    const on = (type: string, init: object = {}) =>
      document.body.dispatchEvent(Object.assign(new Event(type, { bubbles: true }), init));
    const button = () => container.querySelector("button") as HTMLButtonElement;
    return { grow, scroll, gap, top: () => top, show, leave, on, button };
  }

  it("follows the transcript to its bottom as it grows", () => {
    const p = sessionPage();
    expect(p.gap()).toBe(0);
    p.grow(26);
    expect(p.gap()).toBe(0);
  });

  it("stays where the reader scrolls up to, and follows again at the bottom", () => {
    const p = sessionPage();
    p.scroll(p.top() - 40);
    p.grow(26);
    p.grow(26);
    expect(p.gap()).toBe(40 + 52);
    p.scroll(p.top() + 1000);
    p.grow(26);
    expect(p.gap()).toBe(0);
  });

  it("lets go as the wheel turns up", () => {
    const p = sessionPage();
    p.on("wheel", { deltaY: -4 });
    p.grow(26);
    expect(p.gap()).toBe(26);
    // A pinch zooms instead.
    p.on("wheel", { deltaY: 4 });
    p.grow(26);
    p.on("wheel", { deltaY: -4, ctrlKey: true });
    p.grow(26);
    expect(p.gap()).toBe(0);
  });

  it("holds still while a button is down, and follows once it is up", () => {
    const p = sessionPage();
    p.on("pointerdown", { button: 0 });
    p.grow(26);
    expect(p.gap()).toBe(26);
    p.on("pointerup");
    p.grow(26);
    expect(p.gap()).toBe(0);
  });

  it("leaves what the reader opens where it is", () => {
    const p = sessionPage();
    act(() => p.button().click());
    p.grow(300);
    expect(p.gap()).toBe(300);
    p.grow(26);
    expect(p.gap()).toBe(326);
  });

  it("leaves the browser's scroll anchoring to the reader once they scroll up", () => {
    const anchoring = () => document.documentElement.style.overflowAnchor;
    const p = sessionPage();
    expect(anchoring()).toBe("none");
    p.scroll(p.top() - 40);
    expect(anchoring()).toBe("");
    p.scroll(p.top() + 1000);
    expect(anchoring()).toBe("none");
    p.leave();
    expect(anchoring()).toBe("");
  });

  it("starts at the bottom for another session", () => {
    const p = sessionPage();
    p.scroll(p.top() - 300);
    p.show("ses_2");
    p.grow(26);
    expect(p.gap()).toBe(0);
  });
});

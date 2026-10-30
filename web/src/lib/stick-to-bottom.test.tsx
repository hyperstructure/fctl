import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  bottomOf,
  keyDirection,
  mayShrink,
  NEAR_BOTTOM,
  Stick,
  scrollsInside,
  useStickToBottom,
} from "./stick-to-bottom";

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

describe("the page's bottom", () => {
  it("is where the bottom of the page meets the bottom of the screen", () => {
    expect(bottomOf(2000, 800)).toBe(1200);
    // The on-screen keyboard covers the rest of the layout viewport.
    expect(bottomOf(2000, 800, 400)).toBe(1600);
    expect(bottomOf(500, 800)).toBe(0);
  });

  it("is never past the page's bottom, whatever the visual viewport is said to be", () => {
    // Safari on iOS after the app switcher.
    expect(bottomOf(2000, 800, -59)).toBe(1200);
    expect(bottomOf(2000, 800, 0)).toBe(1200);
    expect(bottomOf(2000, 800, Number.NaN)).toBe(1200);
    // A fraction taller than the layout viewport, rounded.
    expect(bottomOf(2000, 800, 800.5)).toBe(1200);
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

describe("changes to the page", () => {
  /** The records of what `change` does to a paragraph of text. */
  async function records(change: (p: HTMLElement) => void) {
    const p = document.createElement("p");
    p.lang = "en";
    p.append("Done. Everything looks good", document.createElement("span"));
    document.body.append(p);
    cleanups.push(() => p.remove());
    const seen: MutationRecord[] = [];
    const o = new MutationObserver((r) => seen.push(...r));
    // As the session page observes it.
    o.observe(p, {
      subtree: true,
      childList: true,
      attributes: true,
      characterData: true,
      attributeOldValue: true,
      characterDataOldValue: true,
    });
    change(p);
    await new Promise((r) => setTimeout(r));
    o.disconnect();
    return seen;
  }
  const text = (p: HTMLElement) => p.firstChild as Text;
  /** Whether what `change` does to a paragraph may make the page shorter. */
  const shrinks = async (change: (p: HTMLElement) => void) => mayShrink(await records(change));

  it("only grow the page when they add to it, as text that streams in does", async () => {
    expect(await shrinks((p) => p.append(document.createElement("em")))).toBe(false);
    expect(await shrinks((p) => text(p).appendData(": the change is committed"))).toBe(false);
  });

  it("change nothing when they leave an attribute as it was, as React does at every render", async () => {
    expect(await shrinks((p) => p.setAttribute("lang", "en"))).toBe(false);
    expect(
      await shrinks((p) => {
        p.setAttribute("title", "");
        p.removeAttribute("title");
      }),
    ).toBe(false);
  });

  it("may make it shorter when they take something out, shorten text or restyle", async () => {
    expect(await shrinks((p) => p.querySelector("span")?.remove())).toBe(true);
    expect(await shrinks((p) => text(p).deleteData(0, 6))).toBe(true);
    expect(await shrinks((p) => p.setAttribute("class", "hidden"))).toBe(true);
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
   * With `visual`, the browser says its visual viewport is that tall, and
   * scrolls the page until that much of it shows, as Safari on iOS does: less
   * than the screen with the keyboard up, and past the page's bottom with a
   * visual viewport of no height. With `installed`, it is the app opened from
   * the home screen.
   */
  function sessionPage({ visual, installed = false }: { visual?: number; installed?: boolean } = {}) {
    const view = 800;
    let top = 0;
    let height = 2000;
    const end = () => height - (visual ?? view);
    Object.defineProperty(window, "innerHeight", { configurable: true, get: () => view });
    Object.defineProperty(window, "scrollY", { configurable: true, get: () => top });
    if (visual !== undefined) {
      Object.defineProperty(window, "visualViewport", {
        configurable: true,
        value: { height: visual, offsetTop: 0, scale: 1 },
      });
    }
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: installed && query === "(display-mode: standalone)",
    }));
    Object.defineProperty(document.documentElement, "scrollHeight", {
      configurable: true,
      get: () => height,
    });
    Object.defineProperty(document.documentElement, "clientHeight", { configurable: true, get: () => view });
    cleanups.push(() => {
      for (const k of ["innerHeight", "scrollY", "visualViewport"]) Reflect.deleteProperty(window, k);
      for (const k of ["scrollHeight", "clientHeight"]) Reflect.deleteProperty(document.documentElement, k);
    });
    const scroll = (to: number) => {
      top = Math.max(0, Math.min(to, end()));
      window.dispatchEvent(new Event("scroll"));
    };
    vi.spyOn(window, "scrollTo").mockImplementation(((o: ScrollToOptions) => {
      top = Math.max(0, Math.min(o.top ?? 0, end()));
    }) as typeof window.scrollTo);
    let resized: ResizeObserverCallback | undefined;
    const observed = new Set<Element>();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: ResizeObserverCallback) {
          resized = cb;
        }
        observe(target: Element) {
          observed.add(target);
        }
        disconnect() {
          observed.clear();
        }
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
    /** The transcript grows by `by` px, or with `outside`, the rest of the page (the header, the composer). */
    const grow = (by: number, outside = false) => {
      height += by;
      const target = outside ? document.documentElement : container.firstElementChild;
      if (!target || !observed.has(target)) return;
      act(() =>
        resized?.(
          [{ target, contentRect: { height } } as unknown as ResizeObserverEntry],
          {} as ResizeObserver,
        ),
      );
    };
    grow(0);
    const gap = () => height - top - view;
    /** How much of the page is below the top of the screen (px): none once it is scrolled past its bottom. */
    const below = () => height - top;
    const on = (type: string, init: object = {}) =>
      document.body.dispatchEvent(Object.assign(new Event(type, { bubbles: true }), init));
    const button = () => container.querySelector("button") as HTMLButtonElement;
    /**
     * The page changes by `by` px and the browser lays it out, with its resize
     * observers and scroll event still to come at its next frame. Shorter at
     * its bottom, the page moves up to its new bottom.
     */
    const change = async (by: number) => {
      height += by;
      top = Math.min(top, height - view);
      // Shorter takes something out (a line of the header); longer, something in.
      if (by < 0) container.querySelector("span")?.remove();
      else container.append(document.createElement("span"));
      await act(async () => {});
    };
    const scrolled = () => window.dispatchEvent(new Event("scroll"));
    return { grow, scroll, gap, below, top: () => top, show, leave, on, button, change, scrolled };
  }

  it("follows the transcript to its bottom as it grows", () => {
    const p = sessionPage();
    expect(p.gap()).toBe(0);
    p.grow(26);
    expect(p.gap()).toBe(0);
  });

  it("keeps its bottom above the on-screen keyboard", () => {
    const p = sessionPage({ visual: 400 });
    p.grow(26);
    expect(p.below()).toBe(400);
  });

  it("keeps its bottom above the keyboard of the installed app's composer", () => {
    const field = document.createElement("textarea");
    document.body.append(field);
    cleanups.push(() => field.remove());
    field.focus();
    const p = sessionPage({ visual: 400, installed: true });
    p.grow(26);
    expect(p.below()).toBe(400);
  });

  it("keeps its bottom at the screen's when Safari on iOS takes a keyboard to be up that isn't", () => {
    // As it can in the installed app after the app switcher, with nothing to type in:
    // scrolled up to the keyboard, the page's bottom stopped over the middle of the screen.
    const p = sessionPage({ visual: 491, installed: true });
    p.grow(26);
    expect(p.below()).toBe(800);
  });

  it("stays on the screen when Safari on iOS says the visual viewport has no height", () => {
    // As it can after the app switcher. Asked to scroll to the page's height, it went
    // there, past the page's bottom, and the screen showed nothing of it.
    const p = sessionPage({ visual: -59 });
    expect(p.below()).toBe(800);
    p.grow(26);
    expect(p.below()).toBe(800);
  });

  it("follows the rest of the page to its bottom too, like a header that gets another line", () => {
    const p = sessionPage();
    p.grow(22, true);
    expect(p.gap()).toBe(0);
    p.scroll(p.top() - 40);
    p.grow(22, true);
    expect(p.gap()).toBe(40 + 22);
  });

  it("keeps following when the page gets shorter and grows again before it scrolls", async () => {
    const p = sessionPage();
    // The header loses a line, then the transcript gets one, before the next frame: its
    // scroll event says the page went up and away from its bottom, as the reader's would.
    await p.change(23);
    p.grow(0);
    await p.change(-23);
    await p.change(67);
    p.scrolled();
    p.grow(0);
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

  it("keeps following under a dialog the wheel and keys turn in, like the lightbox", () => {
    const p = sessionPage();
    const dialog = document.createElement("dialog");
    dialog.setAttribute("open", "");
    const picture = document.createElement("img");
    dialog.append(picture);
    document.body.append(dialog);
    cleanups.push(() => dialog.remove());
    picture.dispatchEvent(Object.assign(new Event("wheel", { bubbles: true }), { deltaY: -4 }));
    picture.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
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

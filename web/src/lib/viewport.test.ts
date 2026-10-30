import { afterEach, describe, expect, it, vi } from "vitest";
import { phantomKeyboard, type Viewports, viewportShift, visualHeight, watchViewport } from "./viewport";

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0).reverse()) clean();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

/**
 * The installed app on a 402 x 874 iPhone, with no field to type in, as
 * Safari left it after the app switcher in a screen recording: it takes a
 * keyboard 309 px tall to cover the screen, and none shows.
 */
const phantom: Viewports = {
  layout: 874,
  visual: 565,
  offsetTop: 0,
  scale: 1,
  typing: false,
  installed: true,
};

describe("Safari's phantom keyboard", () => {
  it("is there when the visual viewport is a keyboard short, with no field to type in", () => {
    expect(phantomKeyboard(phantom)).toBe(true);
    // Or has no height at all, as another installed app's logs show.
    expect(phantomKeyboard({ ...phantom, visual: -59 })).toBe(true);
  });

  it("isn't there when the visual viewport fills the layout viewport, or nearly", () => {
    expect(phantomKeyboard({ ...phantom, visual: 874 })).toBe(false);
    expect(phantomKeyboard({ ...phantom, visual: 812 })).toBe(false);
  });

  it("isn't the keyboard of a field being typed in", () => {
    expect(phantomKeyboard({ ...phantom, typing: true })).toBe(false);
  });

  it("isn't looked for in a browser's tab, whose toolbars come and go", () => {
    expect(phantomKeyboard({ ...phantom, installed: false })).toBe(false);
  });

  it("isn't a page zoomed in", () => {
    expect(phantomKeyboard({ ...phantom, visual: 437, scale: 2 })).toBe(false);
  });
});

describe("the viewport's shift", () => {
  it("is how far below the layout viewport the screen is while the phantom keyboard is there", () => {
    expect(viewportShift({ ...phantom, offsetTop: 120 })).toBe(120);
    expect(viewportShift({ ...phantom, offsetTop: 309 })).toBe(309);
  });

  it("is 0 without it, with a keyboard up or none", () => {
    expect(viewportShift({ ...phantom, offsetTop: 309, typing: true })).toBe(0);
    expect(viewportShift({ ...phantom, visual: 874 })).toBe(0);
  });

  it("never moves anything up, as the page bounces at its top", () => {
    expect(viewportShift({ ...phantom, offsetTop: -20 })).toBe(0);
  });
});

/**
 * The app on a 402 x 874 iPhone, installed or in a tab, with its visual
 * viewport, which `move` moves as Safari does: `height` tall as Safari says,
 * `offsetTop` px down the layout viewport. Its page is 3000 px tall, and
 * `scrolled` scrolls it as a finger does, to where it stops.
 */
function iPhone({ installed = true } = {}) {
  const vv = Object.assign(new EventTarget(), { height: 874, offsetTop: 0, scale: 1 });
  vi.stubGlobal("visualViewport", vv);
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: installed && query === "(display-mode: standalone)",
  }));
  const page = document.documentElement;
  Object.defineProperty(page, "clientHeight", { configurable: true, value: 874 });
  Object.defineProperty(page, "scrollHeight", { configurable: true, value: 3000 });
  let top = 0;
  Object.defineProperty(window, "scrollY", { configurable: true, get: () => top });
  const scrollTo = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
  cleanups.push(() => {
    for (const k of ["clientHeight", "scrollHeight"]) Reflect.deleteProperty(page, k);
    Reflect.deleteProperty(window, "scrollY");
    scrollTo.mockRestore();
  });
  const move = (height: number, offsetTop: number) => {
    Object.assign(vv, { height, offsetTop });
    vv.dispatchEvent(new Event("scroll"));
  };
  const scrolled = (to: number) => {
    top = to;
    window.dispatchEvent(new Event("scrollend"));
  };
  const field = document.createElement("textarea");
  document.body.append(field);
  cleanups.push(() => field.remove());
  const watch = () => {
    const stop = watchViewport();
    cleanups.push(stop);
    return stop;
  };
  const shift = () => document.documentElement.style.getPropertyValue("--viewport-shift");
  return { move, scrolled, scrollTo, field, watch, shift };
}

describe("watching the viewport", () => {
  it("moves what is pinned to the screen's edges along as the screen leaves the layout viewport", () => {
    const p = iPhone();
    p.watch();
    expect(p.shift()).toBe("");
    p.move(565, 120);
    expect(p.shift()).toBe("120px");
    p.move(565, 309);
    expect(p.shift()).toBe("309px");
    p.move(565, 0);
    expect(p.shift()).toBe("");
  });

  it("leaves the keyboard of a field being typed in to Safari", () => {
    const p = iPhone();
    p.watch();
    p.field.focus();
    p.move(565, 309);
    expect(p.shift()).toBe("");
  });

  it("waits for a field's keyboard to go before it takes Safari to be wrong", () => {
    vi.useFakeTimers();
    const p = iPhone();
    p.watch();
    p.field.focus();
    p.move(565, 309);
    p.field.blur();
    // The keyboard is going: the visual viewport is still short.
    p.move(565, 309);
    expect(p.shift()).toBe("");
    // Gone, but Safari still takes it to be up.
    vi.advanceTimersByTime(1000);
    expect(p.shift()).toBe("309px");
  });

  it("does nothing once the keyboard is gone for real", () => {
    vi.useFakeTimers();
    const p = iPhone();
    p.watch();
    p.field.focus();
    p.move(565, 309);
    p.field.blur();
    p.move(874, 0);
    vi.advanceTimersByTime(1000);
    expect(p.shift()).toBe("");
  });

  it("drops the shift as soon as a field takes the focus", () => {
    const p = iPhone();
    p.watch();
    p.move(565, 309);
    p.field.focus();
    expect(p.shift()).toBe("");
  });

  it("stays out of a browser's tab", () => {
    const p = iPhone({ installed: false });
    p.watch();
    p.move(565, 309);
    expect(p.shift()).toBe("");
    p.scrolled(3000 - 565);
    expect(p.scrollTo).not.toHaveBeenCalled();
  });

  it("takes the page back to its end from past it, where Safari lets a finger take it", () => {
    const p = iPhone();
    p.watch();
    p.move(565, 309);
    // The page's end is 309 px above the screen's.
    p.scrolled(3000 - 565);
    expect(p.scrollTo).toHaveBeenCalledWith({ top: 3000 - 874, behavior: "smooth" });
  });

  it("leaves the page where a finger takes it short of its end, or with a keyboard up", () => {
    const p = iPhone();
    p.watch();
    p.move(565, 309);
    p.scrolled(3000 - 874);
    p.field.focus();
    p.scrolled(3000 - 565);
    expect(p.scrollTo).not.toHaveBeenCalled();
  });

  it("takes the shift away when it stops", () => {
    const p = iPhone();
    const stop = p.watch();
    p.move(565, 309);
    stop();
    expect(p.shift()).toBe("");
  });
});

describe("the visual viewport's height", () => {
  it("is the layout viewport's while Safari's phantom keyboard shortens it", () => {
    const p = iPhone();
    p.move(565, 0);
    expect(visualHeight()).toBe(874);
  });

  it("is Safari's with a keyboard up, and in a browser's tab", () => {
    const p = iPhone();
    p.move(565, 0);
    p.field.focus();
    expect(visualHeight()).toBe(565);
    p.field.blur();
    iPhone({ installed: false }).move(565, 0);
    expect(visualHeight()).toBe(565);
  });

  it("is unknown without a visual viewport", () => {
    vi.stubGlobal("visualViewport", undefined);
    expect(visualHeight()).toBeUndefined();
  });
});

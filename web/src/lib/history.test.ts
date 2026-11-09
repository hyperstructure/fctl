import { afterEach, describe, expect, it, vi } from "vitest";
import { appHistory, goTo, keepsNoHistory } from "./history";

/** WebKit on a touch screen (an iPhone or iPad), and the app opened from the home screen or not. */
function apple({ webKit = true, touch = true, home = true } = {}) {
  if (webKit) vi.stubGlobal("GestureEvent", class {});
  Object.defineProperty(navigator, "maxTouchPoints", { value: touch ? 5 : 0, configurable: true });
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: home && query === "(display-mode: standalone)",
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (navigator as { maxTouchPoints?: number }).maxTouchPoints;
});

describe("keepsNoHistory", () => {
  it("is the app on an iPhone's or iPad's Home Screen", () => {
    apple();
    expect(keepsNoHistory()).toBe(true);
  });

  it("isn't a browser's tab, the app on Android or a Mac's app in the Dock", () => {
    apple({ home: false });
    expect(keepsNoHistory()).toBe(false);
    vi.unstubAllGlobals();
    apple({ webKit: false });
    expect(keepsNoHistory()).toBe(false);
    vi.unstubAllGlobals();
    apple({ touch: false });
    expect(keepsNoHistory()).toBe(false);
  });
});

describe("appHistory", () => {
  /** How many entries the browser's history has after the history opens a page. */
  async function opened(path: string) {
    const history = appHistory();
    const before = window.history.length;
    history.push(path);
    history.flush();
    await Promise.resolve();
    return { added: window.history.length - before, at: window.location.pathname, history };
  }

  it("opens each page in place of the last on an iPhone's or iPad's Home Screen", async () => {
    apple();
    const { added, at, history } = await opened("/s/one");
    expect(added).toBe(0);
    expect(at).toBe("/s/one");
    history.destroy();
  });

  it("adds each page to the browser's history in its tabs", async () => {
    apple({ home: false });
    const { added, at, history } = await opened("/s/two");
    expect(added).toBe(1);
    expect(at).toBe("/s/two");
    history.destroy();
  });
});

describe("goTo", () => {
  it("loads a page in place of this one on an iPhone's or iPad's Home Screen, and after it elsewhere", () => {
    const replace = vi.spyOn(window.location, "replace").mockImplementation(() => {});
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
    apple();
    goTo("/login");
    expect(replace).toHaveBeenCalledWith("/login");
    expect(assign).not.toHaveBeenCalled();
    apple({ home: false });
    goTo("/");
    expect(assign).toHaveBeenCalledWith("/");
  });
});

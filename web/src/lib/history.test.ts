import { afterEach, describe, expect, it, vi } from "vitest";
import { appHistory, goTo, keepsNoHistory, linksInPlace } from "./history";

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

describe("linksInPlace", () => {
  /** A click on a link to `href`, with `init`; whether it went on, and where it loaded in place. */
  function click(href: string, init: MouseEventInit = {}, link: Partial<HTMLAnchorElement> = {}) {
    const a = Object.assign(document.createElement("a"), { href }, link);
    const label = document.createElement("span");
    a.append(label);
    document.body.append(a);
    const replace = vi.spyOn(window.location, "replace").mockImplementation(() => {});
    const e = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init });
    label.dispatchEvent(e);
    a.remove();
    const loaded = replace.mock.calls[0]?.[0];
    replace.mockRestore();
    return { loaded, wentOn: !e.defaultPrevented };
  }

  it("loads a link's page in place of this one on an iPhone's or iPad's Home Screen", () => {
    apple();
    const undo = linksInPlace();
    expect(click("/api/v1/auth/google/start?intent=login")).toEqual({
      loaded: `${window.location.origin}/api/v1/auth/google/start?intent=login`,
      wentOn: false,
    });
    expect(click("https://github.com/apps/fctl/installations/new").loaded).toBe(
      "https://github.com/apps/fctl/installations/new",
    );
    undo();
  });

  it("leaves the router's links, new tabs and windows, and other kinds of link alone", () => {
    apple();
    const undo = linksInPlace();
    // The router's links cancel their clicks first.
    const routed = (e: Event) => e.preventDefault();
    document.addEventListener("click", routed, { capture: true });
    expect(click("/pulls").loaded).toBeUndefined();
    document.removeEventListener("click", routed, { capture: true });
    expect(click("/pulls", { metaKey: true })).toEqual({ loaded: undefined, wentOn: true });
    expect(click("https://github.com", {}, { target: "_blank" })).toEqual({
      loaded: undefined,
      wentOn: true,
    });
    expect(click("mailto:ada@example.com")).toEqual({ loaded: undefined, wentOn: true });
    undo();
  });

  it("leaves links to the browser in its tabs", () => {
    apple({ home: false });
    const undo = linksInPlace();
    expect(click("/api/v1/auth/google/start").loaded).toBeUndefined();
    undo();
  });
});

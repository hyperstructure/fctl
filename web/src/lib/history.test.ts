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
  // A link a test clicked may have taken happy-dom elsewhere.
  (window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(
    "http://localhost:3000/",
  );
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

  it("opens on the page it was left on when iOS starts the app afresh, and only there", () => {
    const left = JSON.stringify({ path: "/s/left", at: Date.now() - 3_600_000 });
    apple();
    localStorage.setItem("fctl:left", left);
    sessionStorage.clear();
    let history = appHistory();
    expect(history.location.pathname).toBe("/s/left");
    history.destroy();

    (window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(
      "http://localhost:3000/",
    );
    apple({ home: false });
    localStorage.setItem("fctl:left", left);
    sessionStorage.clear();
    history = appHistory();
    expect(history.location.pathname).toBe("/");
    history.destroy();
    localStorage.clear();
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
  /**
   * A click on a link to `href`, with `init`; whether it went on, and where it loaded in place.
   * On the link itself: happy-dom follows a link as a click on something in it bubbles through
   * it, before the document hears it, where a browser waits for the click to end.
   */
  function click(href: string, init: MouseEventInit = {}, link: Partial<HTMLAnchorElement> = {}) {
    const a = Object.assign(document.createElement("a"), { href }, link);
    document.body.append(a);
    const replace = vi.spyOn(window.location, "replace").mockImplementation(() => {});
    const e = new MouseEvent("click", { bubbles: true, cancelable: true, button: 0, ...init });
    a.dispatchEvent(e);
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

  it("moves to a place on this page in place of where it was, keeping the router's state", () => {
    apple();
    const undo = linksInPlace();
    window.history.replaceState({ __TSR_index: 3, __TSR_key: "k" }, "", "/w/acme");
    const place = Object.assign(document.createElement("h2"), { id: "enrollment-tokens" });
    document.body.append(place);
    const scrolled = vi.fn();
    place.scrollIntoView = scrolled;
    const before = window.history.length;
    try {
      // A page load would lose the router's state.
      expect(click("#enrollment-tokens")).toEqual({ loaded: undefined, wentOn: false });
      expect(window.location.pathname + window.location.hash).toBe("/w/acme#enrollment-tokens");
      expect(window.history.state).toEqual({ __TSR_index: 3, __TSR_key: "k" });
      expect(window.history.length).toBe(before);
      expect(scrolled).toHaveBeenCalled();
    } finally {
      place.remove();
      undo();
    }
  });

  it("finds a place by its name as written, or decoded, and takes a malformed one in its stride", () => {
    apple();
    const undo = linksInPlace();
    window.history.replaceState(null, "", "/w/acme");
    const spaced = Object.assign(document.createElement("h2"), { id: "two words" });
    const percent = Object.assign(document.createElement("h2"), { id: "100%" });
    document.body.append(spaced, percent);
    const scrolled: string[] = [];
    for (const el of [spaced, percent]) el.scrollIntoView = () => scrolled.push(el.id);
    try {
      expect(click("#two%20words").wentOn).toBe(false);
      expect(click("#100%").wentOn).toBe(false);
      expect(click("#%E0%A4%A").wentOn).toBe(false);
      expect(scrolled).toEqual(["two words", "100%"]);
      expect(window.location.hash).toBe("#%E0%A4%A");
    } finally {
      spaced.remove();
      percent.remove();
      undo();
    }
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

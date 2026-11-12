import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearLog,
  forgetLog,
  KEEP,
  logFor,
  logText,
  onCaughtError,
  previousEnd,
  readLog,
  record,
  reloadToRecover,
  startAppLog,
  useAppLog,
  viewportText,
  watchWebgl,
} from "./app-log";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const said = () => readLog().map((e) => e.what);

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { value: state, configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
}

/** The app opened from the Home Screen, or in a tab. */
function installed(yes: boolean) {
  const mm = window.matchMedia.bind(window);
  vi.stubGlobal("matchMedia", (q: string) =>
    q === "(display-mode: standalone)" ? ({ matches: yes } as MediaQueryList) : mm(q),
  );
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
  clearLog();
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete (document as { visibilityState?: string }).visibilityState;
  document.getElementById("root")?.remove();
});

describe("record", () => {
  it("keeps the newest entries, in localStorage, without secrets", () => {
    record("Error: token=fctl_cli_abcdef123 failed for dev@fctl.test", 1);
    expect(readLog()).toEqual([{ at: 1, what: "Error: token=[hidden] failed for [hidden]" }]);
    expect(JSON.parse(localStorage.getItem("fctl:app-log") ?? "[]")).toHaveLength(1);
    for (let i = 0; i < KEEP + 5; i++) record(`entry ${i}`, i);
    expect(readLog()).toHaveLength(KEEP);
    expect(readLog().at(-1)?.what).toBe(`entry ${KEEP + 4}`);
  });

  it("counts the same thing again on the entry it repeats", () => {
    record("Error: boom", 1);
    record("Error: boom", 2);
    record("Error: boom", 3);
    record("Went to the background", 4);
    record("Error: boom", 5);
    expect(readLog()).toEqual([
      { at: 3, what: "Error: boom (×3)" },
      { at: 4, what: "Went to the background" },
      { at: 5, what: "Error: boom" },
    ]);
  });

  it("is kept for whoever is signed in, and goes as someone else signs in", () => {
    logFor("usr_a");
    record("A's page", 1);
    logFor("usr_a");
    expect(said()).toEqual(["A's page"]);
    logFor("usr_b");
    expect(said()).toEqual([]);
    expect(localStorage.getItem("fctl:app-log-owner")).toBe("usr_b");
  });

  it("is kept for the next account to sign in only by the one it was kept for", () => {
    // A first run notes no one yet: what it logged before the sign-in stays.
    record("Started", 1);
    logFor("usr_a");
    expect(said()).toEqual(["Started"]);
    // Clear empties it, and what comes after is still A's.
    clearLog();
    record("After Clear", 2);
    expect(localStorage.getItem("fctl:app-log-owner")).toBe("usr_a");
    logFor("usr_a");
    expect(said()).toEqual(["After Clear"]);
    // Signing out keeps it for no one: what another tab adds after goes as anyone signs in.
    forgetLog();
    record("Another tab, still on A's page", 3);
    logFor("usr_b");
    expect(said()).toEqual([]);
    expect(localStorage.getItem("fctl:app-log-owner")).toBe("usr_b");
  });

  it("shows, as the page opens, what another tab wrote since this tab last read it", async () => {
    record("here", 1);
    expect(said()).toEqual(["here"]);
    localStorage.setItem(
      "fctl:app-log",
      JSON.stringify([
        { at: 1, what: "here" },
        { at: 2, what: "another tab" },
      ]),
    );
    const host = document.createElement("div");
    const root = createRoot(host);
    function Log() {
      const log = useAppLog();
      return createElement("p", null, log.map((e) => e.what).join(", "));
    }
    await act(async () => root.render(createElement(Log)));
    expect(host.textContent).toBe("here, another tab");
    await act(async () => root.unmount());
  });

  it("forgets the heartbeat with the log", () => {
    localStorage.setItem("fctl:app-beat", JSON.stringify({ at: 1, shown: true }));
    record("here", 1);
    clearLog();
    expect(readLog()).toEqual([]);
    expect(localStorage.getItem("fctl:app-beat")).toBeNull();
  });

  it("adds to what another tab wrote rather than writing over it", () => {
    record("here", 1);
    localStorage.setItem(
      "fctl:app-log",
      JSON.stringify([
        { at: 1, what: "here" },
        { at: 2, what: "another tab" },
      ]),
    );
    record("here again", 3);
    expect(said()).toEqual(["here", "another tab", "here again"]);
  });

  it("puts the log as text after the browser it is from", () => {
    const at = new Date(2026, 10, 12, 13, 5, 2).getTime();
    expect(logText([{ at, what: "Went to the background" }], "Safari")).toBe(
      "Safari\n\n2026-11-12 13:05:02  Went to the background",
    );
  });
});

describe("previousEnd", () => {
  it("tells a run that stopped on screen from one that ended in the background", () => {
    expect(previousEnd(undefined)).toBeUndefined();
    expect(previousEnd({ at: 0, shown: true })).toMatch(/^The run before stopped while on screen/);
    expect(previousEnd({ at: 0, shown: false })).toMatch(/^The run before ended in the background/);
  });
});

describe("viewportText", () => {
  it("says the viewports and the screen, as Safari's broken ones read", () => {
    expect(
      viewportText({
        width: 402,
        height: 812,
        visualWidth: 402,
        visualHeight: -59,
        offsetTop: 0,
        screenWidth: 402,
        screenHeight: 874,
        scrollY: 1200.4,
      }),
    ).toBe("viewport 402×812 (visual 402×-59 at 0), screen 402×874, scrolled 1200");
  });
});

describe("reloadToRecover", () => {
  it("loads the page again, but not twice within a minute", () => {
    const reload = vi.spyOn(window.location, "reload").mockImplementation(() => {});
    expect(reloadToRecover("Crashed", 100_000)).toBe(true);
    expect(reloadToRecover("Crashed", 130_000)).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1);
    expect(said()).toEqual([
      "Crashed; loading the page again",
      "Crashed; not loading the page again, as it did 30 s ago",
    ]);
    // Refusing doesn't move the minute on: a page that crashes every 40 s still gets its reload.
    expect(reloadToRecover("Crashed", 165_000)).toBe(true);
    expect(reload).toHaveBeenCalledTimes(2);
  });

  it("doesn't load the page again where it can't note when it last did", () => {
    const reload = vi.spyOn(window.location, "reload").mockImplementation(() => {});
    const refused = () => {
      throw new Error("refused");
    };
    vi.stubGlobal("sessionStorage", { getItem: refused, setItem: refused });
    expect(reloadToRecover("Crashed", 100_000)).toBe(false);
    expect(reload).not.toHaveBeenCalled();
  });
});

describe("onCaughtError", () => {
  it("keeps what the error page showed, but not an address with no page", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    onCaughtError(new TypeError("x is undefined"));
    onCaughtError({ isNotFound: true });
    expect(said()).toEqual(["Error page for: TypeError: x is undefined"]);
  });
});

describe("startAppLog", () => {
  it("says how the run before ended, in an installed app", () => {
    installed(true);
    localStorage.setItem("fctl:app-beat", JSON.stringify({ at: 1, shown: true }));
    const stop = startAppLog();
    expect(said()[0]).toMatch(/^The run before stopped while on screen/);
    expect(said()[1]).toMatch(/^Started \(a fresh run, .*\) in the installed app at \//);
    stop();
    // A page load of the same run's own says nothing of a run before.
    clearLog();
    const again = startAppLog();
    expect(said()).toHaveLength(1);
    expect(said()[0]).toMatch(/^Started \(a page load in the same run/);
    again();
  });

  it("leaves tabs, which run side by side, without a run before", () => {
    installed(false);
    localStorage.setItem("fctl:app-beat", JSON.stringify({ at: 1, shown: true }));
    const stop = startAppLog();
    expect(said()).toHaveLength(1);
    expect(said()[0]).toMatch(/in a browser tab/);
    stop();
  });

  it("follows the app to the background and back, and keeps the heartbeat", () => {
    vi.useFakeTimers();
    installed(true);
    const root = Object.assign(document.createElement("div"), { id: "root" });
    root.append(document.createElement("main"));
    document.body.append(root);
    const stop = startAppLog();
    setVisibility("hidden");
    expect(JSON.parse(localStorage.getItem("fctl:app-beat") ?? "{}")).toMatchObject({ shown: false });
    vi.advanceTimersByTime(90 * 60_000);
    setVisibility("visible");
    expect(said().slice(-2)[0]).toBe("Went to the background");
    expect(said().at(-1)).toMatch(/^Came back after 1h 30m away; viewport /);
    expect(JSON.parse(localStorage.getItem("fctl:app-beat") ?? "{}")).toMatchObject({ shown: true });
    stop();
  });

  it("loads the page again when the app comes back showing nothing", () => {
    vi.useFakeTimers();
    installed(true);
    const reload = vi.spyOn(window.location, "reload").mockImplementation(() => {});
    document.body.append(Object.assign(document.createElement("div"), { id: "root" }));
    const stop = startAppLog();
    setVisibility("hidden");
    setVisibility("visible");
    vi.advanceTimersByTime(3_000);
    expect(said().at(-1)).toBe("On screen again with nothing: the app shows no page; loading the page again");
    expect(reload).toHaveBeenCalledTimes(1);
    stop();
  });

  it("says when the main thread was stuck on screen", () => {
    vi.useFakeTimers();
    let now = 0;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const stop = startAppLog();
    now = 2_000;
    vi.advanceTimersByTime(2_000);
    expect(said().some((w) => w.includes("stuck"))).toBe(false);
    // The timer comes 9 s late: its task waited behind something.
    now = 13_000;
    vi.advanceTimersByTime(2_000);
    expect(said().at(-1)).toBe("The main thread was stuck for 9.0 s on screen");
    stop();
  });

  it("says a page unloads, without taking it for a trip to the background", () => {
    const stop = startAppLog();
    window.dispatchEvent(new PageTransitionEvent("pagehide", { persisted: false }));
    setVisibility("hidden");
    expect(said().at(-1)).toBe("Page unloading");
    stop();
  });

  it("keeps errors nothing caught, ten a minute at most", () => {
    const stop = startAppLog();
    window.dispatchEvent(new ErrorEvent("error", { message: "boom" }));
    expect(said().at(-1)).toBe("Error: boom");
    for (let i = 0; i < 20; i++) window.dispatchEvent(new ErrorEvent("error", { message: `boom ${i}` }));
    expect(said().filter((w) => w.startsWith("Error:"))).toHaveLength(10);
    expect(said().at(-1)).toBe("More errors this minute, not kept");
    stop();
  });
});

describe("watchWebgl", () => {
  /** A canvas whose WebGL context can be lost, and whether it was. */
  function canvas() {
    const lose = vi.fn();
    const el = document.createElement("canvas");
    const gl = { isContextLost: () => false, getExtension: () => ({ loseContext: lose }) };
    el.getContext = vi.fn(() => gl) as unknown as HTMLCanvasElement["getContext"];
    return { el, lose };
  }

  it("lets the context go once the canvas left the page", () => {
    const { el, lose } = canvas();
    document.body.append(el);
    const unwatch = watchWebgl(el, "agent orb");
    el.remove();
    unwatch();
    expect(lose).toHaveBeenCalledTimes(1);
  });

  it("keeps it while the canvas stays, as React's development mode mounts a scene again", () => {
    const { el, lose } = canvas();
    document.body.append(el);
    watchWebgl(el, "agent orb")();
    expect(lose).not.toHaveBeenCalled();
    el.remove();
  });

  it("logs a context lost, restored or never given", () => {
    const { el } = canvas();
    el.dataset.webgl = "unavailable";
    const unwatch = watchWebgl(el, "Agent Floor scene");
    el.dispatchEvent(new Event("webglcontextlost"));
    el.dispatchEvent(new Event("webglcontextrestored"));
    expect(said()).toEqual([
      "WebGL gave the Agent Floor scene no context, on /",
      "WebGL lost the Agent Floor scene's context, on /",
      "WebGL restored the Agent Floor scene's context, on /",
    ]);
    unwatch();
  });
});

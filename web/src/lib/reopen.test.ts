import { createBrowserHistory } from "@tanstack/react-router";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { forgetLeft, keepWhereLeft, keptAddress, RESTART_WITHIN, reopening, reopenWhereLeft } from "./reopen";

const HOUR = 3_600_000;

function setURL(url: string) {
  (window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(
    `http://localhost:3000${url}`,
  );
}

function left() {
  return JSON.parse(localStorage.getItem("fctl:left") ?? "null");
}

function setVisibility(state: "visible" | "hidden") {
  Object.defineProperty(document, "visibilityState", { value: state, configurable: true });
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

afterEach(() => {
  setURL("/");
  delete (document as { visibilityState?: string }).visibilityState;
});

describe("keptAddress", () => {
  it("keeps the signed-in app's pages, with the tab or list they show", () => {
    expect(keptAddress("/s/ses_1", "")).toBe("/s/ses_1");
    expect(keptAddress("/p/shop", "?tab=memory")).toBe("/p/shop?tab=memory");
    expect(keptAddress("/pulls", "?state=merged&scope=w%3Aacme&q=fix")).toBe(
      "/pulls?state=merged&scope=w%3Aacme&q=fix",
    );
    expect(keptAddress("/", "")).toBe("/");
    expect(keptAddress("/settings", "?tab=appearance")).toBe("/settings?tab=appearance");
  });

  it("leaves out what a one-off link or a search brought", () => {
    expect(keptAddress("/w/acme", "?tab=github&github=installed&account=acme")).toBe("/w/acme?tab=github");
    expect(keptAddress("/s/ses_1", "?find=deploy&at=12")).toBe("/s/ses_1");
    expect(keptAddress("/settings", "?linked=google")).toBe("/settings");
  });

  it("keeps no sign-in page, nor a device to approve", () => {
    for (const path of ["/login", "/setup", "/invite", "/reset", "/two-factor", "/early-access", "/device"])
      expect(keptAddress(path, "")).toBeUndefined();
    expect(keptAddress("/s/ses_1/more", "")).toBeUndefined();
  });
});

describe("reopening", () => {
  const now = 10 * HOUR;
  const fresh = { path: "/", fresh: true };

  it("goes back to the page left when iOS starts the app afresh a while after it was left", () => {
    expect(reopening({ path: "/s/ses_1", at: now - 3 * HOUR }, fresh, now)).toBe("/s/ses_1");
    expect(reopening({ path: "/p/shop?tab=memory", at: now - RESTART_WITHIN }, fresh, now)).toBe(
      "/p/shop?tab=memory",
    );
  });

  it("starts at the Agent Floor when someone closed the app and opened it again", () => {
    expect(reopening({ path: "/s/ses_1", at: now - 5_000 }, fresh, now)).toBeUndefined();
  });

  it("stays where a page load of the app's own goes, and where the app opens at another page", () => {
    const long = { path: "/s/ses_1", at: now - HOUR };
    // "Back home" from the error page, signing in: the app ran already.
    expect(reopening(long, { path: "/", fresh: false }, now)).toBeUndefined();
    // A notification's page.
    expect(reopening(long, { path: "/s/ses_2", fresh: true }, now)).toBeUndefined();
    expect(reopening(long, { path: "/?x=1", fresh: true }, now)).toBeUndefined();
  });

  it("has nowhere to go without a page left, or with the Agent Floor left", () => {
    expect(reopening(undefined, fresh, now)).toBeUndefined();
    expect(reopening({ path: "/", at: now - HOUR }, fresh, now)).toBeUndefined();
  });
});

describe("reopenWhereLeft", () => {
  it("puts the page left in the address once, as the app starts afresh", () => {
    const now = Date.now();
    localStorage.setItem("fctl:left", JSON.stringify({ path: "/s/ses_1", at: now - HOUR }));
    setURL("/");
    reopenWhereLeft(now);
    expect(window.location.pathname).toBe("/s/ses_1");
    // A page load later in the same run (going home from the error page) stays where it goes.
    setURL("/");
    reopenWhereLeft(now);
    expect(window.location.pathname).toBe("/");
  });

  it("takes nothing from storage it can't read", () => {
    localStorage.setItem("fctl:left", "{not json");
    setURL("/");
    reopenWhereLeft();
    expect(window.location.pathname).toBe("/");
  });
});

describe("keepWhereLeft", () => {
  it("keeps the page the app opens, and when it was last on the screen", async () => {
    setURL("/s/ses_1");
    const history = createBrowserHistory();
    const stop = keepWhereLeft(history);
    expect(left()).toMatchObject({ path: "/s/ses_1" });
    history.push("/p/shop?tab=memory&linear=installed");
    history.flush();
    await Promise.resolve();
    expect(left()).toMatchObject({ path: "/p/shop?tab=memory" });
    // A sign-in page isn't kept: the page before it is.
    history.push("/login");
    history.flush();
    await Promise.resolve();
    expect(left()).toMatchObject({ path: "/p/shop?tab=memory" });

    localStorage.setItem("fctl:left", JSON.stringify({ path: "/p/shop?tab=memory", at: 1 }));
    history.push("/p/shop?tab=memory");
    history.flush();
    await Promise.resolve();
    localStorage.setItem("fctl:left", JSON.stringify({ path: "/p/shop?tab=memory", at: 1 }));
    setVisibility("hidden");
    expect(left().at).toBeGreaterThan(1);
    stop();
    history.destroy();
  });

  it("forgets the page as someone signs out, and keeps nothing as the page goes", () => {
    setURL("/s/ses_1");
    const history = createBrowserHistory();
    const stop = keepWhereLeft(history);
    forgetLeft();
    window.dispatchEvent(new Event("pagehide"));
    setVisibility("hidden");
    expect(left()).toBeNull();
    stop();
    history.destroy();
  });
});

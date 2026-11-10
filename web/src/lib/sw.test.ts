// The service worker's notifications (public/sw.js), run in a stand-in for
// its global scope.
import { describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import source from "../../public/sw.js?raw";

type Listener = (event: Record<string, unknown>) => void;

interface FakeWindow {
  url: string;
  focus: ReturnType<typeof vi.fn>;
  postMessage: ReturnType<typeof vi.fn>;
}

/** Cache Storage, as far as the worker uses it, keyed by path. */
function fakeCaches() {
  const stores = new Map<string, Map<string, Response>>();
  const store = (name: string) => {
    const s = stores.get(name) ?? new Map<string, Response>();
    stores.set(name, s);
    return s;
  };
  return {
    stores,
    open: async (name: string) => ({
      put: async (key: string, res: Response) => void store(name).set(key, res),
      add: async () => {},
    }),
    keys: async () => [...stores.keys()],
    delete: async (name: string) => stores.delete(name),
    match: async () => undefined,
  };
}

function worker(windows: FakeWindow[] = []) {
  const listeners = new Map<string, Listener>();
  const shown = [
    { tag: "ses_1", close: vi.fn() },
    { tag: "ses_9", close: vi.fn() },
  ];
  const self = {
    location: { origin: "https://app.fctl.test" },
    addEventListener: (type: string, fn: Listener) => listeners.set(type, fn),
    skipWaiting: vi.fn(),
    registration: {
      showNotification: vi.fn(async () => {}),
      getNotifications: vi.fn(async () => shown),
    },
    clients: {
      claim: vi.fn(async () => {}),
      matchAll: vi.fn(async () => windows),
      openWindow: vi.fn(async () => null),
    },
  };
  const caches = fakeCaches();
  new Function("self", "caches", source)(self, caches);
  /** Dispatches an event and waits for what it asked to wait for. */
  const dispatch = async (type: string, event: Record<string, unknown>) => {
    let wait: Promise<unknown> = Promise.resolve();
    listeners.get(type)?.({
      ...event,
      waitUntil: (p: Promise<unknown>) => {
        wait = p;
      },
    });
    await wait;
  };
  const push = (data: unknown) =>
    dispatch("push", {
      data: {
        json: () => {
          if (typeof data === "string") return JSON.parse(data);
          return data;
        },
      },
    });
  const click = (url?: string) =>
    dispatch("notificationclick", { notification: { close: vi.fn(), data: url ? { url } : undefined } });
  /** What the worker kept for the app to open. */
  const opening = async () => {
    const res = caches.stores.get("fctl-opening")?.get("/fctl-opening");
    return res ? ((await res.clone().json()) as { url: string; at: number }) : undefined;
  };
  return { self, shown, push, click, opening, dispatch, caches };
}

const message: G.PushMessage = {
  title: "Fix the login test",
  body: "my-app#12 waits for your reply\nShall I add tests?",
  tag: "ses_12",
  url: "/s/ses_12",
  at: 1_792_272_742_000,
};

const win = (url: string): FakeWindow => ({ url, focus: vi.fn(async () => {}), postMessage: vi.fn() });

describe("the service worker", () => {
  it("shows a session's notification", async () => {
    const w = worker();
    await w.push(message);
    expect(w.self.registration.showNotification).toHaveBeenCalledWith("Fix the login test", {
      body: "my-app#12 waits for your reply\nShall I add tests?",
      tag: "ses_12",
      renotify: true,
      timestamp: message.at,
      icon: "/icon-192.png",
      data: { url: "/s/ses_12" },
    });
    expect(w.shown.every((n) => n.close.mock.calls.length === 0)).toBe(true);
  });

  it("closes the notifications a digest sums up", async () => {
    const w = worker();
    await w.push({
      ...message,
      title: "2 sessions failed",
      tag: "digest",
      url: "/",
      replaces: ["ses_1", "ses_2"],
    });
    expect(w.shown[0]?.close).toHaveBeenCalled();
    expect(w.shown[1]?.close).not.toHaveBeenCalled();
    expect(w.self.registration.showNotification).toHaveBeenCalledWith(
      "2 sessions failed",
      expect.objectContaining({ tag: "digest" }),
    );
  });

  it("still shows something for a push it cannot read", async () => {
    const w = worker();
    await w.push("not json");
    expect(w.self.registration.showNotification).toHaveBeenCalledWith(
      "fctl",
      expect.objectContaining({ data: { url: "/" } }),
    );
  });

  it("focuses the window that shows the session", async () => {
    const other = win("https://app.fctl.test/");
    const showing = win("https://app.fctl.test/s/ses_12");
    const w = worker([other, showing]);
    await w.click("/s/ses_12");
    expect(showing.focus).toHaveBeenCalled();
    // It takes what the click opens, so that no other window does later.
    expect(showing.postMessage).toHaveBeenCalledWith({ type: "open", url: "/s/ses_12" });
    expect(other.focus).not.toHaveBeenCalled();
    expect(other.postMessage).not.toHaveBeenCalled();
    expect(w.self.clients.openWindow).not.toHaveBeenCalled();
  });

  it("keeps what a click opens for the app to take as it starts or shows", async () => {
    const w = worker();
    const before = Date.now();
    await w.click("/s/ses_12?tab=files");
    const kept = await w.opening();
    expect(kept?.url).toBe("/s/ses_12?tab=files");
    expect(kept?.at).toBeGreaterThanOrEqual(before);
    await w.click("https://evil.example/");
    expect((await w.opening())?.url).toBe("/s/ses_12?tab=files");
  });

  it("keeps the click's page when the app was installed with an older worker", async () => {
    const w = worker();
    w.caches.stores.set("fctl-offline-v1", new Map());
    w.caches.stores.set("fctl-opening", new Map());
    await w.dispatch("activate", {});
    expect([...w.caches.stores.keys()]).toEqual(["fctl-opening"]);
  });

  it("still opens the page when the window can't be focused", async () => {
    const app = win("https://app.fctl.test/");
    app.focus.mockRejectedValue(new Error("Not allowed"));
    const w = worker([app]);
    await expect(w.click("/s/ses_12")).resolves.toBeUndefined();
    expect(app.postMessage).toHaveBeenCalledWith({ type: "open", url: "/s/ses_12" });
  });

  it("asks another app window to open the session", async () => {
    const other = win("https://app.fctl.test/p/my-app");
    const w = worker([other]);
    await w.click("/s/ses_12");
    expect(other.postMessage).toHaveBeenCalledWith({ type: "open", url: "/s/ses_12" });
    expect(other.focus).toHaveBeenCalled();
  });

  it("opens a window when there is none, and only for the app", async () => {
    const w = worker();
    await w.click("/s/ses_12");
    expect(w.self.clients.openWindow).toHaveBeenCalledWith("https://app.fctl.test/s/ses_12");
    await w.click("https://evil.example/");
    expect(w.self.clients.openWindow).toHaveBeenCalledTimes(1);
    await w.click();
    expect(w.self.clients.openWindow).toHaveBeenLastCalledWith("https://app.fctl.test/");
  });
});

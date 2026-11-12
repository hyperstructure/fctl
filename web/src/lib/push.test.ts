import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { base64urlBytes, madeWith, onPushOpen, openingWaits, takeOpening } from "./push";

describe("push", () => {
  it("decodes server keys", () => {
    expect([...base64urlBytes("BAUG")]).toEqual([4, 5, 6]);
    expect([...base64urlBytes("-_8")]).toEqual([251, 255]);
    expect(base64urlBytes("A".repeat(87)).length).toBe(65);
  });

  it("tells whether a subscription was made with a key", () => {
    const sub = (key: number[] | null) =>
      ({ options: { applicationServerKey: key ? new Uint8Array(key).buffer : null } }) as PushSubscription;
    expect(madeWith(sub([4, 5, 6]), "BAUG")).toBe(true);
    expect(madeWith(sub([4, 5, 7]), "BAUG")).toBe(false);
    expect(madeWith(sub([4, 5]), "BAUG")).toBe(false);
    expect(madeWith(sub(null), "BAUG")).toBe(false);
  });
});

/** Cache Storage holding what the service worker (public/sw.js) kept for a click on a notification. */
function keep(url: string, at = Date.now()) {
  const stores = new Map<string, Map<string, string>>([
    ["fctl-opening", new Map([["/fctl-opening", JSON.stringify({ url, at })]])],
  ]);
  const caches = {
    match: vi.fn(async (key: string, opts?: { cacheName?: string }) => {
      const body = stores.get(opts?.cacheName ?? "")?.get(key);
      return body === undefined ? undefined : new Response(body);
    }),
    delete: vi.fn(async (name: string) => stores.delete(name)),
  };
  vi.stubGlobal("caches", caches);
  return { stores, caches };
}

const setURL = (url: string) =>
  (window as unknown as { happyDOM: { setURL(url: string): void } }).happyDOM.setURL(url);

describe("clicks on notifications", () => {
  const worker = new EventTarget() as EventTarget & { startMessages: () => void };
  worker.startMessages = () => {};

  beforeEach(() => {
    vi.useFakeTimers();
    Object.defineProperty(navigator, "serviceWorker", { configurable: true, value: worker });
    setURL("https://app.fctl.test/");
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    Reflect.deleteProperty(navigator, "serviceWorker");
  });

  it("takes what a click kept once, while it is recent", async () => {
    const { stores } = keep("/s/ses_12");
    expect(await takeOpening()).toBe("/s/ses_12");
    expect(stores.has("fctl-opening")).toBe(false);
    expect(await takeOpening()).toBeNull();

    keep("/s/ses_12", Date.now() - 3 * 60_000);
    expect(await takeOpening()).toBeNull();
    keep("https://evil.example/");
    expect(await takeOpening()).toBeNull();
  });

  it("tells whether a recent click waits, without taking it", async () => {
    const { stores } = keep("/s/ses_12");
    expect(await openingWaits()).toBe(true);
    expect(stores.has("fctl-opening")).toBe(true);
    keep("/s/ses_12", Date.now() - 3 * 60_000);
    expect(await openingWaits()).toBe(false);
    vi.stubGlobal("caches", { match: async () => undefined });
    expect(await openingWaits()).toBe(false);
  });

  it("opens what a click kept as the app starts at its start page", async () => {
    keep("/s/ses_12");
    const navigate = vi.fn();
    const stop = onPushOpen(navigate);
    await vi.advanceTimersByTimeAsync(0);
    expect(navigate).toHaveBeenCalledWith("/s/ses_12");
    stop();
  });

  it("opens what a click kept after the app comes back to the front", async () => {
    vi.stubGlobal("caches", { match: async () => undefined, delete: async () => false });
    const navigate = vi.fn();
    const stop = onPushOpen(navigate);
    await vi.advanceTimersByTimeAsync(0);
    document.dispatchEvent(new Event("visibilitychange"));
    // The click reaches the service worker a moment after the app shows.
    await vi.advanceTimersByTimeAsync(200);
    keep("/s/ses_12");
    await vi.advanceTimersByTimeAsync(1_000);
    expect(navigate).toHaveBeenCalledTimes(1);
    expect(navigate).toHaveBeenCalledWith("/s/ses_12");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(navigate).toHaveBeenCalledTimes(1);
    stop();
  });

  it("follows the service worker's message, and forgets what it kept", async () => {
    vi.stubGlobal("caches", { match: async () => undefined, delete: async () => false });
    const navigate = vi.fn();
    const stop = onPushOpen(navigate);
    await vi.advanceTimersByTimeAsync(0);
    const { stores } = keep("/s/ses_12");
    worker.dispatchEvent(new MessageEvent("message", { data: { type: "open", url: "/s/ses_12" } }));
    await vi.advanceTimersByTimeAsync(0);
    expect(navigate).toHaveBeenCalledWith("/s/ses_12");
    expect(stores.has("fctl-opening")).toBe(false);
    stop();
  });

  it("stays on the page when it shows it already", async () => {
    setURL("https://app.fctl.test/s/ses_12");
    const { stores } = keep("/s/ses_12");
    const navigate = vi.fn();
    const stop = onPushOpen(navigate);
    worker.dispatchEvent(new MessageEvent("message", { data: { type: "open", url: "/s/ses_12" } }));
    await vi.advanceTimersByTimeAsync(5_000);
    expect(navigate).not.toHaveBeenCalled();
    expect(stores.has("fctl-opening")).toBe(false);
    stop();
  });

  it("ignores messages that aren't about opening the app's pages", async () => {
    vi.stubGlobal("caches", { match: async () => undefined, delete: async () => false });
    const navigate = vi.fn();
    const stop = onPushOpen(navigate);
    worker.dispatchEvent(
      new MessageEvent("message", { data: { type: "open", url: "https://evil.example/" } }),
    );
    worker.dispatchEvent(new MessageEvent("message", { data: { type: "other", url: "/s/ses_12" } }));
    await vi.advanceTimersByTimeAsync(0);
    expect(navigate).not.toHaveBeenCalled();
    stop();
  });
});

// The service worker. It shows the notifications the control plane sends
// (Web Push; crates/fctl-control/src/push.rs, payloads are PushMessage) and
// opens what a click is about. Registered in src/main.tsx, and by
// src/lib/push.ts when notifications are turned on. Browsers offer to install
// the app on its manifest alone.
//
// It has no fetch handler, and must keep none. With one, a page load waits for
// the worker to start and answer: in the app on an iPhone's Home Screen whose
// page iOS ended in the background, WebKit waits up to 70 s for a worker that
// doesn't answer before it loads the page from the network itself
// (ServiceWorkerFetchTask's timeout), and the screen stays black meanwhile.
// Every request a page makes would wait on it too, with no limit. Without one,
// browsers load pages and requests straight from the network, without
// starting the worker. A page that can't load offline shows the browser's own
// error page.

// What the last click on a notification opens, kept for the app to pick up
// (src/lib/push.ts takeOpening): an iPhone's Home Screen app may open at its
// start page whatever openWindow asks for, and may miss a message sent as it
// comes back to the front, so the app looks here too as it starts and shows.
const OPENING = "fctl-opening";
const OPENING_KEY = "/fctl-opening";

self.addEventListener("install", (event) => {
  event.waitUntil(self.skipWaiting());
});

// Workers before this one kept a copy of the offline page (fctl-offline-v1,
// v2): only the click's page stays.
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((key) => key !== OPENING).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("push", (event) => {
  event.waitUntil(show(event.data));
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  event.waitUntil(open(event.notification.data?.url ?? "/"));
});

async function show(data) {
  let message = null;
  try {
    message = data ? data.json() : null;
  } catch {
    message = null;
  }
  if (!message || typeof message.title !== "string") {
    // Browsers want a notification for every push.
    return self.registration.showNotification("fctl", {
      body: "A session you follow changed.",
      tag: "fctl",
      icon: "/icon-192.png",
      data: { url: "/" },
    });
  }
  // A digest sums up these sessions: their older notifications go.
  if (Array.isArray(message.replaces) && message.replaces.length) {
    for (const shown of await self.registration.getNotifications()) {
      if (message.replaces.includes(shown.tag)) shown.close();
    }
  }
  return self.registration.showNotification(message.title, {
    body: message.body,
    tag: message.tag,
    // A newer notification about the same session alerts again.
    renotify: Boolean(message.tag),
    timestamp: message.at,
    icon: "/icon-192.png",
    data: { url: message.url },
  });
}

/** Focuses the app on `path`: the window that shows it, another app window, or a new one. */
async function open(path) {
  const url = new URL(path, self.location.origin);
  if (url.origin !== self.location.origin) return;
  const to = url.pathname + url.search;
  await remember(to);
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const app = windows.find((w) => new URL(w.url).pathname === url.pathname) ?? windows[0];
  if (app) {
    // The app navigates itself (src/lib/push.ts), without reloading, unless it shows `to`
    // already; either way it forgets what remember() kept, so no other window takes it later.
    app.postMessage({ type: "open", url: to });
    // WebKit may refuse to focus; the system brings the app forward for the click anyway.
    return app.focus().catch(() => null);
  }
  return self.clients.openWindow(url.href).catch(() => null);
}

async function remember(to) {
  try {
    const cache = await caches.open(OPENING);
    await cache.put(
      OPENING_KEY,
      new Response(JSON.stringify({ url: to, at: Date.now() }), {
        headers: { "Content-Type": "application/json" },
      }),
    );
  } catch {
    // The message and openWindow still do on other browsers.
  }
}

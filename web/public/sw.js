// The service worker. With it, browsers offer to install fctl as an app, a
// page that cannot load offline shows offline.html instead of the browser's
// error, and notifications reach this browser: it shows the ones the control
// plane sends (Web Push; crates/fctl-control/src/push.rs, payloads are
// PushMessage) and opens what a click is about. It caches nothing else (the
// app needs the control plane anyway, and a deploy shows at once) and leaves
// /api/ to the network: sign-in redirects and downloads never pass through
// it. Registered in src/main.tsx, and by src/lib/push.ts when notifications
// are turned on.

// Bump it with every change to offline.html: browsers install the worker again,
// and so fetch the page anew, only when this file changes.
const CACHE = "fctl-offline-v2";
// public/offline.html, at the URL Workers static assets serve it from (they
// redirect /offline.html there, and a navigation cannot take a redirect).
const OFFLINE = "/offline";
// What the last click on a notification opens, kept for the app to pick up
// (src/lib/push.ts takeOpening): an iPhone's Home Screen app may open at its
// start page whatever openWindow asks for, and may miss a message sent as it
// comes back to the front, so the app looks here too as it starts and shows.
const OPENING = "fctl-opening";
const OPENING_KEY = "/fctl-opening";

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.add(new Request(OFFLINE, { cache: "reload" })))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((key) => key !== CACHE && key !== OPENING).map((key) => caches.delete(key))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  if (request.mode !== "navigate" || new URL(request.url).pathname.startsWith("/api/")) return;
  event.respondWith(
    fetch(request).catch(() => caches.match(OFFLINE).then((page) => page || Response.error())),
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

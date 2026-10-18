// The service worker: shows the notifications the control plane sends
// (Web Push; crates/fctl-control/src/push.rs, payloads are PushMessage) and
// opens what a click is about. It has no fetch handler, so pages load as if
// it were not there. Kept outside the bundle so its URL never changes.

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => event.waitUntil(self.clients.claim()));

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
      badge: "/badge-96.png",
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
    badge: "/badge-96.png",
    data: { url: message.url },
  });
}

/** Focuses the app on `path`: the window that shows it, another app window, or a new one. */
async function open(path) {
  const url = new URL(path, self.location.origin);
  if (url.origin !== self.location.origin) return;
  const windows = await self.clients.matchAll({ type: "window", includeUncontrolled: true });
  const showing = windows.find((w) => new URL(w.url).pathname === url.pathname);
  if (showing) return showing.focus();
  const app = windows[0];
  if (app) {
    // The app navigates itself (web/src/lib/push.ts), without reloading.
    app.postMessage({ type: "open", url: url.pathname + url.search });
    return app.focus();
  }
  return self.clients.openWindow(url.href);
}

// The service worker: with it, browsers offer to install fctl as an app, and a
// page that cannot load offline shows offline.html instead of the browser's
// error. It caches nothing else (the app needs the control plane anyway, and
// a deploy shows at once) and leaves /api/ to the network: sign-in redirects
// and downloads never pass through it. Registered in src/main.tsx.

const CACHE = "fctl-offline-v1";
// public/offline.html, at the URL Workers static assets serve it from (they
// redirect /offline.html there, and a navigation cannot take a redirect).
const OFFLINE = "/offline";

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
      .then((keys) => Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key))))
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

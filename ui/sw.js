importScripts("sw-notifications.js");
const CACHE = "pinned-shell-v1";
const BYPASS = ["/api/", "/ws/", "/install.sh", "/download/"];

self.addEventListener("install", (event) => {
  self.skipWaiting();
  event.waitUntil(precache());
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
  const url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== location.origin || BYPASS.some((path) => url.pathname.startsWith(path))) return;
  event.respondWith(networkFirst(event));
});

// The page loaded before this worker took control, so its files were never cached; fetch what
// index.html references now, so the first launch without a network still opens.
async function precache() {
  const cache = await caches.open(CACHE);
  const page = await fetch("/");
  const html = await page.clone().text();
  await cache.put("/", page);
  await cache.addAll([...html.matchAll(/(?:href|src)="([^"]+)"/g)].map((match) => match[1]));
}

async function networkFirst(event) {
  try {
    // The hub sends no Cache-Control, so the browser would guess its files stay fresh for hours
    // after a release; revalidating each time costs a 304.
    const response = await fetch(event.request, { cache: "no-cache" });
    if (response.status === 200) {
      const copy = response.clone();
      event.waitUntil(caches.open(CACHE).then((cache) => cache.put(event.request, copy)));
    }
    return response;
  } catch (error) {
    const cached = await caches.match(event.request, { ignoreSearch: true });
    if (cached) return cached;
    throw error;
  }
}

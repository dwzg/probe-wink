// Minimal offline cache for the Probe Wink page. Bump VERSION when any
// cached file changes so installed copies pick up the new files.
const VERSION = 'probewink-v1';
const FILES = [
  './',
  './index.html',
  './manifest.webmanifest',
  './icon.svg',
  './icon-192.png',
  './icon-512.png',
];

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(VERSION).then((c) => c.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== VERSION).map((k) => caches.delete(k))))
      .then(() => self.clients.claim()),
  );
});

// Stale-while-revalidate for same-origin GETs: answer from the cache at once
// (works offline in the car), refresh the cache in the background.
self.addEventListener('fetch', (event) => {
  const req = event.request;
  if (req.method !== 'GET' || new URL(req.url).origin !== self.location.origin) return;
  event.respondWith(
    caches.open(VERSION).then(async (cache) => {
      const cached = await cache.match(req, { ignoreSearch: true });
      const fresh = fetch(req)
        .then((res) => {
          if (res.ok && res.type === 'basic') {
            const key = new URL(req.url);
            key.search = ''; // ?demo=1 etc. share one entry
            cache.put(key.href, res.clone());
          }
          return res;
        })
        .catch(() => undefined);
      if (cached) {
        event.waitUntil(fresh);
        return cached;
      }
      const res = await fresh;
      return res || (req.mode === 'navigate' ? cache.match('./index.html') : Response.error());
    }),
  );
});

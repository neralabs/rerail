// Versioned, complete static cache. No application requests go to a backend.
const prefix = 'falling-facts-' + self.registration.scope + '-';
let current;
async function manifest() {
  if (!current) current = fetch(new URL('./manifest.json', self.location)).then(response => {
    if (!response.ok) throw new Error('Could not load the offline manifest');
    return response.json();
  });
  return current;
}
self.addEventListener('install', event => event.waitUntil((async () => {
  const { version, assets } = await manifest();
  const cache = await caches.open(prefix + version);
  await cache.addAll(['./manifest.json', './offline-worker.js', ...assets.map(path => './' + path)]);
  await self.skipWaiting();
})()));
self.addEventListener('activate', event => event.waitUntil((async () => {
  const { version } = await manifest();
  for (const name of await caches.keys()) if (name.startsWith(prefix) && name !== prefix + version) await caches.delete(name);
  await self.clients.claim();
})()));
self.addEventListener('fetch', event => {
  if (event.request.method !== 'GET' || new URL(event.request.url).origin !== self.location.origin) return;
  event.respondWith((async () => {
    const url = new URL(event.request.url);
    const base = new URL('./', self.location);
    if (!url.href.startsWith(base.href)) return fetch(event.request);
    if (url.pathname === base.pathname) url.pathname += 'index.html';
    const cached = await caches.match(url.href);
    return cached ?? fetch(event.request);
  })());
});

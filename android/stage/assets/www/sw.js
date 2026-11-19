'use strict';

/* RustCade service worker: caches the app shell, the Rust WASM core and every
   bundled game, so the emulator starts and plays with no network at all. */

const VERSION = 'v1';
const CACHE = `rustcade-${VERSION}`;
const SHELL = [
  './',
  'index.html',
  'styles.css',
  'app.js',
  'emulator.wasm',
  'manifest.webmanifest',
  'icon.svg',
  'icons/icon-192.png',
  'icons/icon-512.png',
  'icons/icon-maskable-512.png',
  'roms/index.json',
];

self.addEventListener('install', (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      await cache
        .addAll(SHELL.map((url) => new Request(url, { cache: 'reload' })))
        .catch(() => {});
      // Bundle the ROMs listed by the game picker.
      try {
        const response = await fetch('roms/index.json', { cache: 'reload' });
        if (response.ok) {
          const games = await response.json();
          await Promise.all(
            games.map((game) => cache.add(`roms/${game.file}`).catch(() => {}))
          );
        }
      } catch {}
      await self.skipWaiting();
    })()
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)));
      await self.clients.claim();
    })()
  );
});

self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET') return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  if (request.mode === 'navigate') {
    event.respondWith(
      (async () => {
        try {
          const fresh = await fetch(request);
          const cache = await caches.open(CACHE);
          cache.put('index.html', fresh.clone());
          return fresh;
        } catch {
          const cache = await caches.open(CACHE);
          return (
            (await cache.match('index.html')) ||
            (await cache.match('./')) ||
            new Response('Offline and no cached copy.', {
              status: 503,
              headers: { 'content-type': 'text/plain; charset=utf-8' },
            })
          );
        }
      })()
    );
    return;
  }

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE);
      const hit = await cache.match(request);
      if (hit) return hit;
      try {
        const response = await fetch(request);
        if (response.ok) cache.put(request, response.clone());
        return response;
      } catch {
        return new Response('', { status: 504 });
      }
    })()
  );
});

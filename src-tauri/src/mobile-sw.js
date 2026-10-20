"use strict";
// EasyLauncher phone console shell. API responses and console output are
// never cached: only the static page, manifest and icons live here.
const VERSION = "easylauncher-shell-v1";
const SHELL = [
  "/",
  "/manifest.webmanifest",
  "/icon-192.png",
  "/icon-512.png",
  "/favicon.ico"
];

self.addEventListener("install", function (event) {
  event.waitUntil(
    caches
      .open(VERSION)
      .then(function (cache) { return cache.addAll(SHELL); })
      .then(function () { return self.skipWaiting(); })
  );
});

self.addEventListener("activate", function (event) {
  event.waitUntil(
    caches
      .keys()
      .then(function (keys) {
        return Promise.all(
          keys
            .filter(function (key) { return key !== VERSION; })
            .map(function (key) { return caches.delete(key); })
        );
      })
      .then(function () { return self.clients.claim(); })
  );
});

self.addEventListener("fetch", function (event) {
  const request = event.request;
  const url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== self.location.origin) return;
  // Live state, console lines and actions always go to the machine directly.
  if (url.pathname.indexOf("/api/") === 0) return;
  if (request.mode === "navigate") {
    event.respondWith(
      fetch(request)
        .then(function (response) {
          if (response.ok) {
            const copy = response.clone();
            caches.open(VERSION).then(function (cache) { return cache.put("/", copy); });
          }
          return response;
        })
        .catch(function () { return caches.match("/"); })
    );
    return;
  }
  if (SHELL.indexOf(url.pathname) >= 0) {
    event.respondWith(
      caches.match(request).then(function (hit) {
        return hit || fetch(request);
      })
    );
  }
});

self.addEventListener("notificationclick", function (event) {
  event.notification.close();
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then(function (list) {
        for (let index = 0; index < list.length; index += 1) {
          const client = list[index];
          if ("focus" in client) return client.focus();
        }
        return self.clients.openWindow("/");
      })
  );
});

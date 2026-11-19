/* Mocidad listen / site service worker — emitted by cforge.
   Cache-first for heavy immutable assets (?v=BUILD_ID); network-first for
   HTML / build-id / gate. Placeholders: __ASSET_VER__ / __BUILD_ID__. */

var CACHE = "mocidad-__BUILD_ID__";
var VER = "__BUILD_ID__";
var BUILT_AT = "__BUILT_AT__";

/* Warm the shell only — never race the page for the multi-MB wasm on install.
   Wasm/js/png land in CACHE via fetchAndCache / page fetch wrapper. */
var PRECACHE = [
  "./",
  "./index.html",
  "./build-gate.js",
  "./build-id.json",
  "./boot.js?v=" + VER,
  "./sw.js",
  "./manifest.json?v=" + VER,
  "./logo.png?v=" + VER
];

function putBoth(cache, req, res) {
  if (!res || !res.ok) return Promise.resolve();
  var copy = res.clone();
  var jobs = [cache.put(req, copy)];
  try {
    var u = typeof req === "string" ? req : req.url;
    if (u) jobs.push(cache.put(u, res.clone()));
  } catch (e) { /* ignore */ }
  return Promise.all(jobs).catch(function () { return null; });
}

function fetchAndCache(req) {
  return fetch(req).then(function (res) {
    if (res && res.ok) {
      var copy = res.clone();
      caches.open(CACHE).then(function (c) { putBoth(c, req, copy); });
    }
    return res;
  });
}

self.addEventListener("install", function (event) {
  event.waitUntil(
    caches.open(CACHE).then(function (cache) {
      return Promise.all(
        PRECACHE.map(function (url) {
          return fetch(url)
            .then(function (res) {
              if (res && res.ok) return cache.put(url, res);
            })
            .catch(function () { return null; });
        })
      );
    }).then(function () {
      return self.skipWaiting();
    })
  );
});

self.addEventListener("activate", function (event) {
  event.waitUntil(
    caches.keys().then(function (keys) {
      return Promise.all(
        keys.map(function (key) {
          if (key.indexOf("mocidad-") === 0 && key !== CACHE) {
            return caches.delete(key);
          }
          return null;
        })
      );
    }).then(function () {
      return self.clients.claim();
    })
  );
});

function isHeavy(url) {
  return /\.(wasm|js|ttf|otf|woff2?|png|jpe?g|webp|gif)(\?|$)/i.test(url.pathname) ||
    /mui_dev\.wasm|mui-dev\.js|boot\.js/i.test(url.pathname);
}

function isShell(url) {
  return url.pathname === "/" ||
    /\/index\.html$|\/build-gate\.js$|\/build-id\.json$|\/app\.mui$|\/manifest\.json$|\/sw\.js$/i.test(url.pathname) ||
    /\.mui$/i.test(url.pathname);
}

function matchCached(req, url) {
  return caches.match(req).then(function (hit) {
    if (hit) return hit;
    return caches.match(url.href).then(function (hit2) {
      if (hit2) return hit2;
      /* Same path, any ?v= — survives soft reloads / stale query strings. */
      return caches.match(req, { ignoreSearch: true }).then(function (hit3) {
        if (hit3) return hit3;
        try {
          return caches.match(url.pathname + url.search, { ignoreSearch: true });
        } catch (e) {
          return null;
        }
      });
    });
  });
}

self.addEventListener("fetch", function (event) {
  var req = event.request;
  if (req.method !== "GET") return;
  var url;
  try {
    url = new URL(req.url);
  } catch (e) {
    return;
  }
  if (url.origin !== self.location.origin) return;

  if (isShell(url)) {
    event.respondWith(
      fetch(req)
        .then(function (res) {
          if (res && res.ok) {
            var copy = res.clone();
            caches.open(CACHE).then(function (c) { putBoth(c, req, copy); });
          }
          return res;
        })
        .catch(function () {
          return matchCached(req, url).then(function (hit) {
            return hit || caches.match("./index.html");
          });
        })
    );
    return;
  }

  if (!isHeavy(url)) return;

  /* Cache-first for wasm/js/fonts/images — this is what makes reload free. */
  event.respondWith(
    matchCached(req, url).then(function (hit) {
      if (hit) {
        /* Revalidate in background so a new ASSET_VER still gets picked up
           eventually without blocking this navigation. */
        if (url.search.indexOf("v=" + VER) < 0) {
          fetchAndCache(req).catch(function () {});
        }
        return hit;
      }
      return fetchAndCache(req);
    })
  );
});

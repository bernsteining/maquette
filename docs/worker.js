// Web Worker: owns the three wasm plugins so their (synchronous, often slow)
// `.call()` executions happen off the main thread. Without this the browser
// UI freezes for the whole duration of a helmet.glb render (~1-4 s of PBR +
// IBL + shadow-maps + WBOIT). With this, only the render canvas stalls; the
// picker, sliders, panels, and even scroll all stay live.
//
// Message protocol (both directions carry a matching `id`):
//   in : { id, kind, plugin, fn?, args?, key? }
//     kind: "ensure"        — lazy fetch+compile+instantiate a plugin
//           "setModel"      — args[0] becomes activeModel; key stashes it
//           "cache"         — args[0] stashed under key, activeModel untouched
//           "useKey"        — flip activeModel to a previously-cached key
//           "call"          — invoke wasm fn with the given args
//           "callWithModel" — invoke wasm fn, prepending activeModel as arg 0
//   out: { id, ok: true, result? } | { id, ok: false, error }
//
// IndexedDB module cache lives here too — main thread doesn't touch wasm APIs
// at all. Repeat visits skip both the download and the compile, keyed by the
// file's ETag/Last-Modified. A CI redeploy of a fresh wasm invalidates the
// cache automatically.

// Per-plugin state. `_argParts` / `_result` are module-level for the
// wasm-minimal-protocol host callbacks (they read/write via mem.buffer).
function makePlugin(url) {
  // _argParts is an ARRAY of Uint8Arrays — the write_args_to_buffer callback
  // copies each one straight into wasm memory at ptr+offset, skipping the
  // full-size intermediate buffer the naive impl allocates. Saves a 4 MB
  // copy per gltf render (the model bytes get memcpy'd once instead of twice).
  let _argParts, _result, inst, mem, ensurePromise;
  // Two-tier model cache:
  //   activeModel  — current bytes fed to every callWithModel().
  //   namedCache   — bytes stashed under a key (built-in preset name usually),
  //                  so subsequent picks of the same model swap the active
  //                  pointer with zero bytes over postMessage. Populated
  //                  eagerly at boot by the demo's preload path.
  let activeModel = null;
  const namedCache = new Map();
  const imports = { typst_env: {
    wasm_minimal_protocol_write_args_to_buffer: (ptr) => {
      const dst = new Uint8Array(mem.buffer);
      let o = ptr;
      for (const a of _argParts) { dst.set(a, o); o += a.length; }
    },
    wasm_minimal_protocol_send_result_to_host: (ptr, len) =>
      { _result = new Uint8Array(mem.buffer, ptr, len).slice(); },
  }};
  const p = {
    ready: false,
    // Memoize the in-flight load. Without this, two overlapping ensure() calls
    // (e.g. syncGltfInfo firing at t=0 and render() at t=120ms) each start
    // their own fetch+compile+instantiate; the second one overwrites `inst`
    // AFTER the first has already run and cached scene/texture data in the
    // first instance's memory. Subsequent calls hit a fresh instance and
    // re-decode everything, ballooning helmet renders from ~2s to ~19s.
    async ensure() {
      if (inst) return;
      if (!ensurePromise) {
        ensurePromise = (async () => {
          const module = await loadModule(url);
          const i = await WebAssembly.instantiate(module, imports);
          inst = i; mem = i.exports.memory;
        })();
      }
      await ensurePromise;
    },
    setModel(bytes, key) {
      activeModel = bytes;
      if (key) namedCache.set(key, bytes);
    },
    // Stash bytes without touching activeModel — used by the background
    // preload path so warming the cache doesn't yank the active model out
    // from under a render that's currently in flight.
    cache(key, bytes) { namedCache.set(key, bytes); },
    useKey(key) {
      const c = namedCache.get(key);
      if (!c) throw new Error(`${url}: no cached model for key: ${key}`);
      activeModel = c;
    },
    call(fn, args) {
      _argParts = args;
      _result = new Uint8Array();
      const rc = inst.exports[fn](...args.map((a) => a.length));
      if (rc !== 0) throw new Error(new TextDecoder().decode(_result) || `${url} call failed`);
      return _result;
    },
    callWithModel(fn, extraArgs) {
      if (!activeModel) throw new Error(`${url}: no model bound (call setModel/useKey first)`);
      return p.call(fn, [activeModel, ...extraArgs]);
    },
  };
  return p;
}

const plugins = {
  maquette:        makePlugin("maquette.wasm"),
  "maquette-scad": makePlugin("maquette-scad.wasm"),
  "maquette-gltf": makePlugin("maquette-gltf.wasm"),
};

// ─────────────────────────── IndexedDB module cache ─────────────────────────
// Cache the compiled WebAssembly.Module (structured-cloneable) keyed by the
// wasm file's ETag/Last-Modified. A cheap HEAD request tells us whether the
// cached module is still current; a CI redeploy invalidates it automatically.
const IDB_NAME = "maquette-cache", IDB_STORE = "modules";
// Memoize the DB open — every get/put previously opened a fresh connection.
// On failure we clear the promise so a later call can retry (private mode,
// storage quota, etc. may resolve).
let _dbPromise = null;
function idbOpen() {
  return _dbPromise ??= new Promise((res, rej) => {
    const r = indexedDB.open(IDB_NAME, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(IDB_STORE);
    r.onsuccess = () => res(r.result);
    r.onerror = () => { _dbPromise = null; rej(r.error); };
  });
}
async function idbGet(key) {
  try {
    const db = await idbOpen();
    return await new Promise((res, rej) => {
      const q = db.transaction(IDB_STORE, "readonly").objectStore(IDB_STORE).get(key);
      q.onsuccess = () => res(q.result);
      q.onerror = () => rej(q.error);
    });
  } catch { return undefined; }
}
async function idbPut(key, val) {
  try {
    const db = await idbOpen();
    await new Promise((res, rej) => {
      const q = db.transaction(IDB_STORE, "readwrite").objectStore(IDB_STORE).put(val, key);
      q.onsuccess = () => res();
      q.onerror = () => rej(q.error);
    });
  } catch { /* private mode, or a browser that won't structured-clone Module: skip caching */ }
}

async function compileModule(url) {
  // Streaming compile overlaps download with compilation. Fall back to a plain
  // compile if the response isn't served as application/wasm (some hosts).
  try { return await WebAssembly.compileStreaming(fetch(url)); }
  catch { return await WebAssembly.compile(await (await fetch(url)).arrayBuffer()); }
}
async function loadModule(url) {
  let tag = null;
  try {
    const h = await fetch(url, { method: "HEAD" });
    tag = h.headers.get("etag") || h.headers.get("last-modified");
  } catch { /* no freshness signal → compile fresh, don't cache */ }

  if (tag) {
    const hit = await idbGet(url);
    if (hit && hit.tag === tag && hit.module instanceof WebAssembly.Module) return hit.module;
  }
  const module = await compileModule(url);
  if (tag) idbPut(url, { tag, module });
  return module;
}

// ─────────────────────────── message dispatch ───────────────────────────────
const ok  = (id, extra)   => self.postMessage({ id, ok: true, ...extra });
const err = (id, error)   => self.postMessage({ id, ok: false, error });

self.onmessage = async (e) => {
  const { id, kind, plugin, fn, args, key } = e.data;
  const p = plugins[plugin];
  if (!p) return err(id, `unknown plugin: ${plugin}`);
  try {
    switch (kind) {
      case "ensure":   await p.ensure(); return ok(id);
      case "setModel": p.setModel(args[0], key); return ok(id);
      case "cache":    p.cache(key, args[0]);    return ok(id);
      case "useKey":   p.useKey(key);            return ok(id);
      case "call":
      case "callWithModel": {
        await p.ensure();
        const result = kind === "callWithModel" ? p.callWithModel(fn, args) : p.call(fn, args);
        // Transfer the result's underlying buffer — main thread only reads it,
        // and we don't retain a reference here. Saves a copy for big renders
        // (helmet's ~500 KB RGBA blob at 512×512).
        return self.postMessage({ id, ok: true, result }, [result.buffer]);
      }
      default: return err(id, `unknown kind: ${kind}`);
    }
  } catch (e2) {
    err(id, (e2 && e2.message) || String(e2));
  }
};

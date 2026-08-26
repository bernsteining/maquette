// Web Worker: owns the three wasm plugins so their (synchronous, often slow)
// `.call()` executions happen off the main thread. Without this the browser
// UI freezes for the whole duration of a helmet.glb render (~1-4 s of PBR +
// IBL + shadow-maps + WBOIT). With this, only the render canvas stalls; the
// picker, sliders, panels, and even scroll all stay live.
//
// Message protocol (both directions carry a matching `id`):
//   in : { id, kind: "ensure" | "call", plugin, fn?, args? }
//   out: { id, ok: true, result? } | { id, ok: false, error }
//
// IndexedDB module cache lives here too — main thread doesn't touch wasm APIs
// at all. Repeat visits skip both the download and the compile, keyed by the
// file's ETag/Last-Modified. A CI redeploy of a fresh wasm invalidates the
// cache automatically.

// Per-plugin state. `_args` / `_result` are module-level for the
// wasm-minimal-protocol host callbacks (they read/write via mem.buffer).
function makePlugin(url) {
  let _args, _result, inst, mem;
  const imports = { typst_env: {
    wasm_minimal_protocol_write_args_to_buffer: (ptr) =>
      new Uint8Array(mem.buffer, ptr, _args.length).set(_args),
    wasm_minimal_protocol_send_result_to_host: (ptr, len) =>
      { _result = new Uint8Array(mem.buffer, ptr, len).slice(); },
  }};
  return {
    ready: false,
    async ensure() {
      if (inst) return;
      const module = await loadModule(url);
      const i = await WebAssembly.instantiate(module, imports);
      inst = i; mem = i.exports.memory;
      this.ready = true;
    },
    call(fn, args) {
      const total = args.reduce((n, a) => n + a.length, 0);
      _args = new Uint8Array(total); let o = 0;
      for (const a of args) { _args.set(a, o); o += a.length; }
      _result = new Uint8Array();
      const rc = inst.exports[fn](...args.map((a) => a.length));
      if (rc !== 0) throw new Error(new TextDecoder().decode(_result) || `${url} call failed`);
      return _result;
    },
  };
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
function idbOpen() {
  return new Promise((res, rej) => {
    const r = indexedDB.open(IDB_NAME, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(IDB_STORE);
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
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
self.onmessage = async (e) => {
  const { id, kind, plugin, fn, args } = e.data;
  const p = plugins[plugin];
  if (!p) return self.postMessage({ id, ok: false, error: `unknown plugin: ${plugin}` });
  try {
    if (kind === "ensure") {
      await p.ensure();
      self.postMessage({ id, ok: true, ready: p.ready });
    } else if (kind === "call") {
      await p.ensure();
      const result = p.call(fn, args);
      // Transfer the result's underlying buffer — main thread only reads it,
      // and we don't retain a reference here. Saves a copy for big renders
      // (helmet's ~500 KB RGBA blob at 512×512).
      self.postMessage({ id, ok: true, result }, [result.buffer]);
    } else {
      self.postMessage({ id, ok: false, error: `unknown kind: ${kind}` });
    }
  } catch (err) {
    self.postMessage({ id, ok: false, error: (err && err.message) || String(err) });
  }
};

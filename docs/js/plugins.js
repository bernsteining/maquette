

// ─────────────────────────────── WASM shim ────────────────────────────────
// Three wasm-minimal-protocol plugins — maquette (OBJ/STL/PLY), maquette-scad
// (OpenSCAD → PLY compile), maquette-gltf (glTF 2.0 PBR) — all live in
// docs/worker.js. The main thread holds only thin async proxies here.
//
// Why a worker: `.call()` is synchronous inside the wasm, and a heavy render
// (helmet.glb at 512² PBR + IBL + shadow-maps + WBOIT) can take seconds. On
// the main thread that stalls scroll, sliders, the picker, everything. In a
// worker only the render canvas waits for its bytes; the rest of the UI stays
// live. See worker.js for the ensure/call message protocol + IDB module cache.

const worker = new Worker(new URL("../worker.js", import.meta.url));   // worker.js lives in docs/, one level up from docs/js/
let _wReqId = 0;
const _wPending = new Map();
worker.onmessage = (e) => {
  const { id, ok, result, error } = e.data;
  const p = _wPending.get(id); if (!p) return; _wPending.delete(id);
  if (!ok) return p.reject(new Error(error));
  // Raster render calls resolve with the whole payload ({bitmap,w,h,svg} or
  // {result}); every other call resolves with just the result bytes.
  p.resolve(p.raster ? e.data : result);
};
// wReq(msg) posts an id-tagged copy of `msg` to the worker and returns a
// Promise that resolves with the worker's response. `msg` must include at
// least { kind, plugin }; { fn, args, key } are per-kind (see worker.js).
function wReq(msg, raster) {
  return new Promise((resolve, reject) => {
    const id = ++_wReqId;
    _wPending.set(id, { resolve, reject, raster });
    worker.postMessage({ id, ...msg });
  });
}
// Proxy shape mirrors the old sync plugin — swap `.call()` for `await
// .call()` at every callsite, everything else stays the same.
//
// `.setModel(bytes)` + `.callWithModel(fn, ...extra)` are the perf-critical
// pair: on model swap the bytes cross to the worker ONCE via setModel; every
// subsequent render/info call passes just the config, and the worker prepends
// the cached bytes as arg 0 wasm-side. Without this, a helmet.glb render
// posts the same 4 MB buffer 3–5× per swap (info + preview + full + tweaks).
function makeWorkerPlugin(plugin) {
  const p = {
    ready: false,
    // Track which keys we've already stashed on the worker side so a repeat
    // pick sends useKey() (no bytes) instead of setModel() (bytes over wire).
    cached: new Set(),
    async ensure() { if (p.ready) return; await wReq({ kind: "ensure", plugin }); p.ready = true; },
    // key is optional; when set, worker stashes the bytes under that name so
    // future useKey(key) activations skip the postMessage entirely.
    async setModel(bytes, key = null) {
      await wReq({ kind: "setModel", plugin, args: [bytes], key });
      if (key) p.cached.add(key);
    },
    // Preload path: stash bytes under `key` without touching activeModel.
    // Safe to fire while a render is in flight.
    async cache(key, bytes) {
      await wReq({ kind: "cache", plugin, args: [bytes], key });
      p.cached.add(key);
    },
    async useKey(key) { await wReq({ kind: "useKey", plugin, key }); },
    async call(fn, ...args) { return await wReq({ kind: "call", plugin, fn, args }); },
    async callWithModel(fn, ...extraArgs) {
      return await wReq({ kind: "callWithModel", plugin, fn, args: extraArgs });
    },
    // Render variant: the worker decodes raster output into a transferable
    // ImageBitmap (painted off the main thread), falling back to raw bytes.
    // Resolves the full payload — { bitmap, w, h, svg } or { result }.
    async renderWithModel(fn, ...extraArgs) {
      return await wReq({ kind: "callWithModel", plugin, fn, args: extraArgs, raster: true }, true);
    },
  };
  return p;
}
const maquettePlugin = makeWorkerPlugin("maquette");
const scadPlugin     = makeWorkerPlugin("maquette-scad");
const gltfPlugin     = makeWorkerPlugin("maquette-gltf");
const molfigPlugin   = makeWorkerPlugin("molfig");

// Bind a model into the worker. Fire-and-forget: worker's FIFO message queue
// guarantees any subsequent render/info sees the bound bytes. Uses useKey()
// when the plugin has already stashed this name (preload or prior pick) so
// we skip the postMessage bytes; falls back to setModel() with a key so the
// next time is free.
function bindModel(plugin, key, bytes) {
  const p = plugin.cached.has(key) ? plugin.useKey(key) : plugin.setModel(bytes, key);
  p.catch(e => console.error("bindModel failed:", e));
}

// Touchscreen phones / save-data mode / low-RAM laptops: skip preload
// entirely. iOS Safari has a ~200 MB per-tab wasm ceiling, and warming
// 3 wasm plugins + several MB of model bytes on boot pushes past it —
// symptom: worker gets OOM-killed and every subsequent render silently
// hangs. On these devices we only ever compile a plugin + hold a model
// when the user actually picks one. `hover: none + pointer: coarse` is
// the canonical "primarily touch" media query (catches iPhone/iPad/most


export { maquettePlugin, scadPlugin, gltfPlugin, molfigPlugin, bindModel };




const worker = new Worker(new URL("../worker.js", import.meta.url));
let _wReqId = 0;
const _wPending = new Map();
worker.onmessage = (e) => {
  const { id, ok, result, error } = e.data;
  const p = _wPending.get(id); if (!p) return; _wPending.delete(id);
  if (!ok) return p.reject(new Error(error));
  p.resolve(p.raster ? e.data : result);
};
function wReq(msg, raster) {
  return new Promise((resolve, reject) => {
    const id = ++_wReqId;
    _wPending.set(id, { resolve, reject, raster });
    worker.postMessage({ id, ...msg });
  });
}
function makeWorkerPlugin(plugin) {
  const p = {
    ready: false,
    cached: new Set(),
    async ensure() { if (p.ready) return; await wReq({ kind: "ensure", plugin }); p.ready = true; },
    async setModel(bytes, key = null) {
      await wReq({ kind: "setModel", plugin, args: [bytes], key });
      if (key) p.cached.add(key);
    },
    async cache(key, bytes) {
      await wReq({ kind: "cache", plugin, args: [bytes], key });
      p.cached.add(key);
    },
    async useKey(key) { await wReq({ kind: "useKey", plugin, key }); },
    async call(fn, ...args) { return await wReq({ kind: "call", plugin, fn, args }); },
    async callWithModel(fn, ...extraArgs) {
      return await wReq({ kind: "callWithModel", plugin, fn, args: extraArgs });
    },
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

function bindModel(plugin, key, bytes) {
  const p = plugin.cached.has(key) ? plugin.useKey(key) : plugin.setModel(bytes, key);
  p.catch(e => console.error("bindModel failed:", e));
}



export { maquettePlugin, scadPlugin, gltfPlugin, molfigPlugin, bindModel };

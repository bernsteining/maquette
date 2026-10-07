async function compileModule(url) {
  try { return await WebAssembly.compileStreaming(fetch(url)); }
  catch { return await WebAssembly.compile(await (await fetch(url)).arrayBuffer()); }
}

const HANDLE_FNS = new Set(["render_obj", "render_obj_png", "render_stl", "render_stl_png", "render_ply", "render_ply_png", "get_obj_info", "get_stl_info", "get_ply_info", "render_gltf"]);

const IS_HELPER = new URL(self.location.href).searchParams.has("helper");
const BAND_PLUGIN = "maquette-gltf.wasm";
const BAND_MIN_SAMPLES = 250000;
const MAX_HELPERS = 3;
const ENC = new TextEncoder();
const DEC = new TextDecoder();

function makeHelper(url) {
  const w = new Worker(url);
  const pending = new Map();
  let seq = 0, bound = null, dead = false;
  const fail = (err) => { dead = true; for (const p of pending.values()) p.reject(err); pending.clear(); };
  w.onmessage = (e) => {
    const p = pending.get(e.data.id);
    if (!p) return;
    pending.delete(e.data.id);
    if (e.data.ok) p.resolve(e.data); else p.reject(new Error(e.data.error));
  };
  w.onerror = (e) => { e.preventDefault?.(); fail(new Error("band helper crashed")); };
  const req = (msg) => new Promise((resolve, reject) => {
    if (dead) return reject(new Error("band helper unavailable"));
    const id = ++seq;
    pending.set(id, { resolve, reject });
    w.postMessage({ id, ...msg });
  });
  return {
    get dead() { return dead; },
    async bind(plugin, scope, bytes) {
      if (bound === scope) return;
      bound = null;
      await req({ kind: "model", plugin, key: scope, bytes, activate: true });
      bound = scope;
    },
    async call(plugin, fn, args) {
      return (await req({ kind: "call", plugin, fn, args, withModel: true })).result;
    },
    async invoke(plugin, fn, args) {
      return (await req({ kind: "call", plugin, fn, args, withModel: false })).result;
    },
  };
}

let helpers = null;
function helperPool() {
  if (IS_HELPER) return [];
  if (!helpers) {
    const cores = self.navigator?.hardwareConcurrency || 2;
    const mem = self.navigator?.deviceMemory ?? 8;
    const n = mem < 4 ? 0 : cores >= 8 && mem >= 8 ? MAX_HELPERS : Math.max(0, Math.min(MAX_HELPERS - 1, cores - 2));
    const url = new URL(self.location.href);
    url.searchParams.set("helper", "1");
    helpers = Array.from({ length: n }, () => makeHelper(url.href));
  }
  return helpers.filter((h) => !h.dead);
}

const FRAME_CACHE_BYTES = 96 * 1024 * 1024;
const FRAME_CACHE_MAX_ARGS = 64 * 1024;
const frames = new Map();
let frameBytes = 0;
let bindSeq = 0;
function frameKey(scope, fn, args) {
  let n = 0;
  for (const a of args) n += a.length;
  if (n > FRAME_CACHE_MAX_ARGS) return null;
  const dec = new TextDecoder();
  return `${scope}\u0000${fn}\u0000${args.map((a) => dec.decode(a)).join("\u0001")}`;
}
function frameGet(key) {
  const hit = frames.get(key);
  if (!hit) return null;
  frames.delete(key);
  frames.set(key, hit);
  return hit.slice();
}
function framePut(key, bytes) {
  if (bytes.length > FRAME_CACHE_BYTES / 8) return;
  frames.set(key, bytes.slice());
  frameBytes += bytes.length;
  for (const [k, v] of frames) {
    if (frameBytes <= FRAME_CACHE_BYTES) break;
    frames.delete(k);
    frameBytes -= v.length;
  }
}

function makePlugin(url) {
  let argParts = [], result = new Uint8Array(), inst = null, compiled = null, ensuring = null, active = null, handle = null, handleMisses = 0, scope = "";
  const models = new Map();
  const imports = { typst_env: {
    wasm_minimal_protocol_write_args_to_buffer: (ptr) => {
      const dst = new Uint8Array(inst.exports.memory.buffer);
      let o = ptr >>> 0;
      for (const a of argParts) { dst.set(a, o); o += a.length; }
    },
    wasm_minimal_protocol_send_result_to_host: (ptr, len) => {
      result = new Uint8Array(inst.exports.memory.buffer, ptr >>> 0, len >>> 0).slice();
    },
  }};
  const plugin = {
    async ensure() {
      compiled ??= compileModule(url);
      ensuring ??= compiled.then((m) => WebAssembly.instantiate(m, imports)).then((i) => { inst = i; });
      try { await ensuring; } catch (e) { compiled = ensuring = null; throw e; }
    },
    bind(key, bytes, activate) {
      if (bytes && key) models.set(key, bytes);
      if (!activate) return;
      active = bytes || models.get(key);
      if (!active) throw new Error(`${url}: no cached model for key: ${key}`);
      handle = null;
      handleMisses = 0;
      scope = `${url}#${++bindSeq}`;
      if (url === BAND_PLUGIN) warmHelpers();
    },
    async call(fn, args, withModel) {
      if (!withModel) return invoke(fn, args);
      if (!active) throw new Error(`${url}: no model bound`);
      const fk = HANDLE_FNS.has(fn) ? frameKey(scope, fn, args) : null;
      const cached = fk && frameGet(fk);
      if (cached) return cached;
      const out = (await callBanded(fn, args)) || callModel(fn, args);
      if (fk) framePut(fk, out);
      return out;
    },
  };
  function warmHelpers() {
    const pool = helperPool();
    if (!pool.length) return;
    const [s, bytes] = [scope, active];
    for (const h of pool) {
      h.bind(URL_TO_ID[url], s, bytes)
        .then(() => h.call(URL_TO_ID[url], "render_gltf", [ENC.encode(JSON.stringify({ width: 8, height: 8 }))]))
        .catch(() => {});
    }
  }
  async function callBanded(fn, args) {
    if (url !== BAND_PLUGIN || fn !== "render_gltf" || args.length !== 1) return null;
    const pool = helperPool();
    if (!pool.length) return null;
    let cfg;
    try { cfg = JSON.parse(DEC.decode(args[0])); } catch { return null; }
    if (!cfg || cfg.band) return null;
    const w = Math.max(1, cfg.width | 0), h = Math.max(1, cfg.height | 0);
    const aa = [1, 1, 2, 4, 2, 4][cfg.antialias ?? 4] || 1;
    if (w * h * aa * aa < BAND_MIN_SAMPLES) return null;
    const deferred = !!cfg.ssao;
    const nb = Math.min(h, pool.length + 1);
    const cuts = Array.from({ length: nb + 1 }, (_, k) => Math.round((h * k) / nb));
    const bandArgs = (k) => [ENC.encode(JSON.stringify({ ...cfg, band: [cuts[k], cuts[k + 1]] }))];
    const rowArg = (k) => new Uint8Array(new Uint32Array([cuts[k]]).buffer);
    const queue = [...Array(nb).keys()];
    const parts = new Array(nb);
    const owner = new Array(nb).fill(null);
    const [s, bytes, id] = [scope, active, URL_TO_ID[url]];
    const remote = pool.map(async (hp) => {
      try { await hp.bind(id, s, bytes); } catch { return; }
      while (queue.length) {
        const k = queue.shift();
        try { parts[k] = await hp.call(id, fn, bandArgs(k)); owner[k] = hp; } catch { queue.push(k); return; }
      }
    });
    while (queue.length) {
      const k = queue.shift();
      parts[k] = callModel(fn, bandArgs(k));
      await new Promise((r) => setTimeout(r, 0));
    }
    await Promise.all(remote);
    for (let k = 0; k < nb; k++) if (!parts[k]) parts[k] = callModel(fn, bandArgs(k));
    if (deferred) {
      const depth = new Uint8Array(w * h * 4);
      for (let k = 0; k < nb; k++) depth.set(parts[k], cuts[k] * w * 4);
      const finish = (k) => invoke("finish_gltf_band", [rowArg(k), depth]);
      await Promise.all([...Array(nb).keys()].map(async (k) => {
        if (owner[k]) {
          try { parts[k] = await owner[k].invoke(id, "finish_gltf_band", [rowArg(k), depth]); return; } catch { }
          parts[k] = callModel(fn, bandArgs(k));
        }
        parts[k] = finish(k);
      }));
    }
    let len = 9;
    for (const p of parts) len += p.length - 9;
    const outBuf = new Uint8Array(len);
    outBuf.set(parts[0].subarray(0, 9));
    new DataView(outBuf.buffer).setUint32(5, h, true);
    let o = 9;
    for (const p of parts) { outBuf.set(p.subarray(9), o); o += p.length - 9; }
    return outBuf;
  }
  function callModel(fn, args) {
    const canHandle = HANDLE_FNS.has(fn) && handle !== false && "model_key" in inst.exports;
    if (canHandle && handle) {
      try {
        const out = invoke(fn, [handle, ...args]);
        handleMisses = 0;
        return out;
      } catch (e) {
        if (e instanceof WebAssembly.RuntimeError) throw e;
        if (++handleMisses >= 2) handle = false;
      }
    }
    const out = invoke(fn, [active, ...args]);
    if (canHandle && handle === null) handle = invoke("model_key", [active]);
    return out;
  }

  function invoke(fn, args) {
    argParts = args;
    result = new Uint8Array();
    try {
      const rc = inst.exports[fn](...args.map((a) => a.length));
      if (rc !== 0) throw new Error(new TextDecoder().decode(result) || `${url}: ${fn} failed`);
      return result;
    } catch (e) {
      if (e instanceof WebAssembly.RuntimeError) {
        const detail = lastPanic();
        inst = null; ensuring = null; handle = null;
        if (detail) throw new WebAssembly.RuntimeError(`${e.message} — ${detail}`);
      }
      throw e;
    } finally { argParts = []; }
  }

  function lastPanic() {
    if (!inst?.exports.get_last_panic) return "";
    argParts = [];
    result = new Uint8Array();
    try { inst.exports.get_last_panic(); return new TextDecoder().decode(result); } catch { return ""; }
  }
  return plugin;
}

const PLUGIN_URLS = {
  maquette: "maquette.wasm",
  "maquette-scad": "maquette-scad.wasm",
  "maquette-gltf": "maquette-gltf.wasm",
  molfig: "molfig.wasm",
};
const URL_TO_ID = Object.fromEntries(Object.entries(PLUGIN_URLS).map(([k, v]) => [v, k]));
const plugins = {};
const pluginFor = (id) => PLUGIN_URLS[id] ? (plugins[id] ??= makePlugin(PLUGIN_URLS[id])) : null;

let canvas = null, ctx = null;

function paint(result) {
  if (!ctx || result.length < 9 || (result[0] !== 0x00 && result[0] !== 0x02)) return null;
  const dv = new DataView(result.buffer, result.byteOffset);
  const w = dv.getUint32(1, true), h = dv.getUint32(5, true), n = w * h * 4;
  if (!w || !h || result.length < 9 + n) return null;
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;
  ctx.putImageData(new ImageData(new Uint8ClampedArray(result.buffer, result.byteOffset + 9, n), w, h), 0, 0);
  return { w, h, svg: result[0] === 0x02 ? result.slice(9 + n) : null };
}

async function handle(msg) {
  const { kind, plugin: id, fn, args = [], key, bytes, activate, withModel, raster } = msg;
  if (kind === "canvas") { canvas = msg.canvas; ctx = canvas.getContext("2d"); return {}; }
  if (kind === "snapshot") return { blob: canvas ? await canvas.convertToBlob({ type: "image/png" }) : null };
  const p = pluginFor(id);
  if (!p) throw new Error(`unknown plugin: ${id}`);
  if (kind === "ensure") { await p.ensure(); return {}; }
  if (kind === "model") { p.bind(key, bytes, activate); return {}; }
  if (kind !== "call") throw new Error(`unknown kind: ${kind}`);
  await p.ensure();
  const result = await p.call(fn, args, withModel);
  const painted = raster ? paint(result) : null;
  if (painted) return { painted: true, ...painted, transfer: painted.svg ? [painted.svg.buffer] : [] };
  return { result, transfer: [result.buffer] };
}

self.onmessage = async (e) => {
  const { id } = e.data;
  try {
    const { transfer = [], ...reply } = await handle(e.data);
    self.postMessage({ id, ok: true, ...reply }, transfer);
  } catch (err) {
    self.postMessage({ id, ok: false, error: (err && err.message) || String(err) });
  }
};

async function compileModule(url) {
  try { return await WebAssembly.compileStreaming(fetch(url)); }
  catch { return await WebAssembly.compile(await (await fetch(url)).arrayBuffer()); }
}

const HANDLE_FNS = new Set(["render_obj", "render_obj_png", "render_stl", "render_stl_png", "render_ply", "render_ply_png", "get_obj_info", "get_stl_info", "get_ply_info", "render_gltf"]);

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
      let o = ptr;
      for (const a of argParts) { dst.set(a, o); o += a.length; }
    },
    wasm_minimal_protocol_send_result_to_host: (ptr, len) => {
      result = new Uint8Array(inst.exports.memory.buffer, ptr, len).slice();
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
    },
    call(fn, args, withModel) {
      if (!withModel) return invoke(fn, args);
      if (!active) throw new Error(`${url}: no model bound`);
      const fk = HANDLE_FNS.has(fn) ? frameKey(scope, fn, args) : null;
      const cached = fk && frameGet(fk);
      if (cached) return cached;
      const out = callModel(fn, args);
      if (fk) framePut(fk, out);
      return out;
    },
  };
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
      if (e instanceof WebAssembly.RuntimeError) { inst = null; ensuring = null; handle = null; }
      throw e;
    } finally { argParts = []; }
  }
  return plugin;
}

const PLUGIN_URLS = {
  maquette: "maquette.wasm",
  "maquette-scad": "maquette-scad.wasm",
  "maquette-gltf": "maquette-gltf.wasm",
  molfig: "molfig.wasm",
};
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
  const result = p.call(fn, args, withModel);
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

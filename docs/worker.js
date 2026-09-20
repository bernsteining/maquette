
function makePlugin(url) {
  let _argParts, _result, inst, mem, ensurePromise;
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
  molfig:          makePlugin("molfig.wasm"),
};

const IDB_NAME = "maquette-cache", IDB_STORE = "modules";
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
  } catch {  }
}

async function compileModule(url) {
  try { return await WebAssembly.compileStreaming(fetch(url)); }
  catch { return await WebAssembly.compile(await (await fetch(url)).arrayBuffer()); }
}
async function loadModule(url) {
  let tag = null;
  try {
    const h = await fetch(url, { method: "HEAD" });
    tag = h.headers.get("etag") || h.headers.get("last-modified");
  } catch {  }

  if (tag) {
    const hit = await idbGet(url);
    if (hit && hit.tag === tag && hit.module instanceof WebAssembly.Module) return hit.module;
  }
  const module = await compileModule(url);
  if (tag) idbPut(url, { tag, module });
  return module;
}

const ok  = (id, extra)   => self.postMessage({ id, ok: true, ...extra });
const err = (id, error)   => self.postMessage({ id, ok: false, error });

async function toBitmapMessage(id, result) {
  if (!self.createImageBitmap || result.length < 9 || (result[0] !== 0x00 && result[0] !== 0x02)) return null;
  const w = result[1] | result[2] << 8 | result[3] << 16 | result[4] << 24;
  const h = result[5] | result[6] << 8 | result[7] << 16 | result[8] << 24;
  const n = w * h * 4;
  if (!w || !h || result.length < 9 + n) return null;
  const px = new Uint8ClampedArray(result.buffer, result.byteOffset + 9, n);
  const bitmap = await createImageBitmap(new ImageData(px, w, h));
  const transfer = [bitmap];
  let svg = null;
  if (result[0] === 0x02) { svg = result.slice(9 + n); transfer.push(svg.buffer); }
  return { msg: { id, ok: true, bitmap, w, h, svg }, transfer };
}

self.onmessage = async (e) => {
  const { id, kind, plugin, fn, args, key, raster } = e.data;
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
        if (raster) {
          try {
            const b = await toBitmapMessage(id, result);
            if (b) return self.postMessage(b.msg, b.transfer);
          } catch {  }
        }
        return self.postMessage({ id, ok: true, result }, [result.buffer]);
      }
      default: return err(id, `unknown kind: ${kind}`);
    }
  } catch (e2) {
    err(id, (e2 && e2.message) || String(e2));
  }
};

// maquette — 3D rendering (STL/OBJ/PLY, glTF PBR, OpenSCAD) in JavaScript, via
// the same WebAssembly the Typst plugins ship. Runs in Node and the browser.
//
//   import { createMaquette } from "maquette";
//   const mq = createMaquette();
//   const { width, height, pixels } = await mq.renderObj(objBytes, { azimuth: 30 });
//   const svg = await mq.renderObj(objBytes, {}, { format: "svg" });

const PLUGIN_FILE = {
  maquette: "maquette.wasm",
  gltf: "maquette-gltf.wasm",
  scad: "maquette-scad.wasm",
};

const ENC = new TextEncoder();
const DEC = new TextDecoder();

const asBytes = (x) =>
  x == null ? new Uint8Array()
  : typeof x === "string" ? ENC.encode(x)
  : x instanceof Uint8Array ? x
  : new Uint8Array(x);

const cfgBytes = (c) =>
  c == null ? ENC.encode("{}")
  : typeof c === "string" ? ENC.encode(c)
  : ENC.encode(JSON.stringify(c));

async function defaultLoad(file) {
  if (typeof process !== "undefined" && process.versions && process.versions.node) {
    const { readFile } = await import("node:fs/promises");
    const { fileURLToPath } = await import("node:url");
    const { dirname, join } = await import("node:path");
    const dir = dirname(fileURLToPath(import.meta.url));
    return new Uint8Array(await readFile(join(dir, "..", "wasm", file)));
  }
  const url = new URL(`../wasm/${file}`, import.meta.url);
  return new Uint8Array(await (await fetch(url)).arrayBuffer());
}

// One wasm-minimal-protocol host per plugin instance.
function makePlugin(bytesPromise) {
  let argParts, result, inst, mem, ready;
  const imports = {
    typst_env: {
      wasm_minimal_protocol_write_args_to_buffer: (ptr) => {
        const dst = new Uint8Array(mem.buffer);
        let o = ptr;
        for (const a of argParts) { dst.set(a, o); o += a.length; }
      },
      wasm_minimal_protocol_send_result_to_host: (ptr, len) => {
        result = new Uint8Array(mem.buffer, ptr, len).slice();
      },
    },
  };
  async function ensure() {
    if (inst) return;
    if (!ready) {
      ready = (async () => {
        const { instance } = await WebAssembly.instantiate(await bytesPromise, imports);
        inst = instance;
        mem = instance.exports.memory;
      })();
    }
    await ready;
  }
  return async function call(fn, args) {
    await ensure();
    argParts = args;
    result = new Uint8Array();
    const rc = inst.exports[fn](...args.map((a) => a.length));
    if (rc !== 0) throw new Error(DEC.decode(result) || `${fn} failed`);
    return result;
  };
}

// Decode the renderer's raster framebuffer:
// [0x00|0x02][w u32 LE][h u32 LE][rgba8 …] → { width, height, pixels }.
export function decodeRaster(bytes) {
  const marker = bytes[0];
  if (marker !== 0x00 && marker !== 0x02) throw new Error("not a raster result");
  const dv = new DataView(bytes.buffer, bytes.byteOffset);
  const width = dv.getUint32(1, true);
  const height = dv.getUint32(5, true);
  return { width, height, pixels: bytes.subarray(9, 9 + width * height * 4) };
}

// Browser helper: raster → ImageData (draw with ctx.putImageData).
export function toImageData(raster) {
  return new ImageData(new Uint8ClampedArray(raster.pixels), raster.width, raster.height);
}

export function createMaquette(opts = {}) {
  const load = opts.load || defaultLoad;
  const calls = {};
  const call = (k, fn, args) => (calls[k] ||= makePlugin(load(PLUGIN_FILE[k])))(fn, args);

  const raster = async (k, fn, args) => decodeRaster(await call(k, fn, args));
  const text = async (k, fn, args) => DEC.decode(await call(k, fn, args));
  const meta = async (k, fn, args) => JSON.parse(await text(k, fn, args));

  const mesh = (kind) => async (data, config, o = {}) => {
    const d = asBytes(data), c = cfgBytes(config);
    return o.format === "svg"
      ? text("maquette", `render_${kind}`, [d, c])
      : raster("maquette", `render_${kind}_png`, [d, c]);
  };

  const scadPly = (src, facets) =>
    call("scad", "build_scad", [asBytes(src), ENC.encode("{}"), ENC.encode(JSON.stringify({ fn: facets })), new Uint8Array()]);

  return {
    renderStl: mesh("stl"),
    renderObj: mesh("obj"),
    renderPly: mesh("ply"),
    renderGltf: (data, config) => raster("gltf", "render_gltf", [asBytes(data), cfgBytes(config)]),
    async renderScad(src, config, o = {}) {
      const facets = o.facets ?? 32;
      const ply = await scadPly(src, facets);
      const c = cfgBytes(config);
      return o.format === "svg"
        ? text("maquette", "render_ply", [ply, c])
        : raster("maquette", "render_ply_png", [ply, c]);
    },
    compileScad: (src, o = {}) => scadPly(src, o.facets ?? 32),
    infoStl: (data) => meta("maquette", "get_stl_info", [asBytes(data), ENC.encode("{}")]),
    infoObj: (data) => meta("maquette", "get_obj_info", [asBytes(data), ENC.encode("{}")]),
    infoPly: (data) => meta("maquette", "get_ply_info", [asBytes(data), ENC.encode("{}")]),
    infoGltf: (data) => meta("gltf", "get_gltf_info", [asBytes(data), ENC.encode("{}")]),
    decodeRaster,
    toImageData,
  };
}

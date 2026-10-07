import { readFile, mkdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import sharp from "sharp";
import * as S from "./shared.js";
const R = (process.env.MAQUETTE_REPO || fileURLToPath(new URL("..", import.meta.url))).replace(/\/$/, "") + "/";
const { createMaquette } = await import(R + "packages/maquette-js/src/index.js");
const mq = createMaquette();
const [what = "all", part = "0", parts = "1"] = process.argv.slice(2);
const save = async (r, f) => {
  const img = sharp(Buffer.from(r.pixels), { raw: { width: r.width, height: r.height, channels: 4 } });
  const flat = r.overlay ? sharp(await img.composite([{ input: Buffer.from(r.overlay) }]).png().toBuffer()) : img;
  return flat.webp({ quality: 93, alphaQuality: 100, effort: 4 }).toFile(f);
};
const load = async (p) => new Uint8Array(await readFile(R + p));
const frames = (a, b) => { const o = []; for (let f = Math.ceil(a * S.FPS - 1e-6); f < Math.round(b * S.FPS); f++) o.push(f); return o; };
const mine = (f) => f % +parts === +part;
const subframes = (a, b, keep = mine) => frames(a, b).filter(keep).flatMap((f) => S.subOffsets.map((o) => f + o));
const HQ = { antialias: 5, ssao: true, sharpen: { strength: 0.3 } };
const SH = { shadows: { per_pixel: true, light_size: 6, resolution: 2048, softness: 2 } };
const dir = async (n) => { await mkdir(`seq/${n}`, { recursive: true }); return `seq/${n}/`; };

if (what === "all" || what === "s2") {
  const bunny = await load("examples/data/bunny.obj");
  await dir("plain"); await dir("sss");
  const base = { width: 720, height: 720, up: [0, 1, 0], distance: 0.25, background: "none", ...HQ };
  for (const ff of subframes(S.S2.compile - 0.1, S.T.s3)) {
    const t = ff / S.FPS, azimuth = S.azS2(t);
    if (t < S.S2.sss + S.S2.sssFade + 0.1) await save(await mq.renderObj(bunny, { ...base, ...SH, azimuth }), S.frameFile("plain", ff));
    if (t > S.S2.sss - 0.1) await save(await mq.renderObj(bunny, { ...base, azimuth,
      lights: [{ type: "positional", vector: [-0.1, 0.14, -0.04], color: "#ff0000", intensity: 3 }],
      sss: { intensity: 4, power: 3.5, distortion: 0.2 } }), S.frameFile("sss", ff));
  }
  console.log("s2 done");
}
if (what === "all" || what === "s3") {
  const bunny = await load("examples/data/bunny.obj"), skull = await load("examples/data/brain_skull.obj"), crank = await load("examples/data/crankshaft.obj");
  const dragon = await load("examples/data/dragon.ply");
  const V = [], F = [];
  for (const l of new TextDecoder().decode(bunny).split("\n")) {
    if (l.startsWith("v ")) V.push(l.slice(2).trim().split(/\s+/).map(Number));
    else if (l.startsWith("f ")) F.push(l.slice(2).trim().split(/\s+/).map((t) => +t.split("/")[0]));
  }
  let rowObj = "";
  for (let k = 0; k < 6; k++) {
    for (const [x, y, z] of V) rowObj += `v ${(x + (k % 2 ? 0.08 : -0.08)).toFixed(5)} ${y.toFixed(5)} ${(z - k * 0.22).toFixed(5)}\n`;
    for (const f of F) rowObj += `f ${f.map((i) => i + k * V.length).join(" ")}\n`;
  }
  const bunnyRow = new TextEncoder().encode(rowObj);
  const pistons = Object.fromEntries("ABCDEF".split("").map((k) => ["Model__Piston_" + k, { label: "Piston " + k }]));
  await dir("feat");
  const Z = 920, b = { width: Z, height: Z, background: "none", ...HQ };
  for (const ff of subframes(S.T.s3, S.T.s4c)) {
    const t = ff / S.FPS, { i, u } = S.cutAt(t), id = S.CUTS[i].id, cam = { ...b, ...S.camAt(id, u, t).p };
    const yUp = { ...cam, up: [0, 1, 0] }, yDown = { ...cam, up: [0, -1, 0] };
    let r;
    switch (id) {
      case "lights": r = await mq.renderObj(bunny, { ...yUp, color: "#d8d8de", specular: 0.7, shininess: 40, ambient: 0.08,
        lights: S.LIGHT_COLORS.map((color, k) => ({ type: "positional", vector: S.lightPos(u, k), color })) }); break;
      case "decimate": r = await mq.renderObj(bunny, { ...yUp, mode: "solid+wireframe", color: "#e0e7ff", wireframe: { color: "#1e1b4b", width: 1.2 }, decimate: S.decAt(u) }); break;
      case "ann": r = await mq.renderObj(crank, { ...yDown, ...SH, color: "#5b6070", specular: 0.5, highlight: pistons,
        annotations: { groups: Object.keys(pistons), color: "#dc2626", font_size: 30, offset: 110 } }); break;
      case "points": r = await mq.renderPly(dragon, { ...yUp, point_splat: true, point_size: S.pointSizeAt(u),
        color_map: "scalar", scalar_function: "x", color_map_palette: ["#7c3aed", "#ec4899", "#f97316", "#facc15"] }); break;
      case "fog": r = await mq.renderObj(bunnyRow, { ...yUp, color: "#e9e4ff", specular: 0.4, fog: { intensity: S.fogAt(u), color: "#0e0f13" }, glow: { color: "#a78bfa", intensity: 0.85, radius: 16 } }); break;
      case "clip": r = await mq.renderObj(skull, { ...yUp, ...SH, shadows: { ...SH.shadows, light_size: 10 }, highlight: { Skull: { color: "#f1f1f1" }, Brain: { color: "#ff69b4" } },
        clip: { plane: [2, -1, 0, Math.round(S.clipAt(u))], cap: false }, cull_backface: false }); break;
      case "explode": r = await mq.renderObj(crank, { ...yDown, ...SH, color: "#2a2d36", specular: 0.5, explode: +S.explodeAt(u).toFixed(2) }); break;
    }
    await save(r, S.frameFile("feat", ff));
  }
  console.log("s3 done");
}
if (what === "all" || what === "scad") {
  await dir("scad");
  for (const ff of subframes(S.T.s4a, S.T.s4b)) {
    const t = ff / S.FPS;
    const ply = await mq.compileScad(S.scadSrc(S.scadHole(t).toFixed(2)), { facets: 100 });
    await save(await mq.renderPly(ply, { width: 900, height: 900, azimuth: S.scadAz(t), elevation: 33, up: [0, 1, 0], fov: 40, zoom: 0.92, color: "#f9d72c", specular: 0, cull_backface: false, background: "none", ...HQ, ...SH }), S.frameFile("scad", ff));
  }
  console.log("scad done");
}
if (what === "all" || what === "proj") {
  await dir("proj");
  const city = await load("examples/data/city.obj");
  const C = { width: 1920, height: 1080, up: [0, 1, 0], auto_center: false, auto_fit: false, background: "#0b1020", antialias: 4, sharpen: { strength: 0.3 },
    color_map: "scalar", scalar_function: "y", color_map_palette: ["#475569", "#239dad", "#4fd0e0", "#f9d72c"], cull_backface: false, light_dir: [1, 2.2, 0.6], shading: "cel", cel_bands: 3 };
  for (const ff of subframes(S.T.s4c, S.T.s4a)) {
    const t = ff / S.FPS, { i } = S.projAt(t), yaw = (S.projYaw(t) * Math.PI) / 180, proj = S.PROJS[i];
    const view = proj === "tiny-planet"
      ? { camera: [0, 4, 0], center: [0, 100, 0], up: [Math.cos(yaw), 0, Math.sin(yaw)], fov: 210, zoom: 1.5 }
      : { camera: [0, 6, 0], center: [20 * Math.cos(yaw), 2, 20 * Math.sin(yaw)], up: [0, 1, 0], fov: { perspective: 78, fisheye: 200, curvilinear: 100 }[proj] };
    await save(await mq.renderObj(city, { ...C, ...view, projection: proj }), S.frameFile("proj", ff));
  }
  console.log("proj done");
}
if (what === "all" || what === "mol") {
  await dir("molc"); await dir("mols");
  const DEC = new TextDecoder(), ENC = new TextEncoder();
  let margs, mres, minst, mmem;
  const minit = (await WebAssembly.instantiate(await readFile(R + "docs/molfig.wasm"), { typst_env: {
    wasm_minimal_protocol_write_args_to_buffer: (p) => { const d = new Uint8Array(mmem.buffer); let o = p >>> 0; for (const a of margs) { d.set(a, o); o += a.length; } },
    wasm_minimal_protocol_send_result_to_host: (p, l) => { mres = new Uint8Array(mmem.buffer, p >>> 0, l >>> 0).slice(); } } })).instance;
  minst = minit; mmem = minst.exports.memory;
  const pdb = await load("examples/data/molecules/1AON.pdb");
  const mol = (representation, theme, quality) => {
    margs = [pdb, ENC.encode(JSON.stringify({ format: "pdb", "mesh-format": "obj", representation, "color-theme": theme, quality }))];
    mres = new Uint8Array();
    if (minst.exports.render_object_bundle(...margs.map((a) => a.length)) !== 0) throw new Error(DEC.decode(mres));
    const ml = +DEC.decode(mres.slice(0, 8)), il = +DEC.decode(mres.slice(8, 16));
    return { materials: JSON.parse(DEC.decode(mres.slice(16, 16 + ml))), mesh: mres.slice(16 + ml + il) };
  };
  const style = [S.molStyle(false), S.molStyle(true)];
  const [cart, fill] = style.map((m) => mol(m.representation, m["color-theme"], m.quality));
  const M = { width: 960, height: 960, elevation: 18, background: "none", ...HQ };
  for (const ff of subframes(S.T.s4d, S.T.s5)) {
    const t = ff / S.FPS, view = { ...M, azimuth: S.molAz(t), zoom: S.molZoom(t) };
    if (t < S.MOL_SWITCH + 0.35) await save(await mq.renderObj(cart.mesh, { ...view, materials: cart.materials, specular: style[0].specular, ssao: style[0].ssao }), S.frameFile("molc", ff));
    if (t > S.MOL_SWITCH - 0.05) await save(await mq.renderObj(fill.mesh, { ...view, materials: fill.materials, specular: style[1].specular, ssao: style[1].ssao }), S.frameFile("mols", ff));
  }
  console.log("mol done");
}
if (what === "tokyo") {
  const tokyo = await load("examples/data/gltf/tokyo.glb");
  await dir("tokyo");
  const only = process.env.ONLY ? process.env.ONLY.split(",").map(Number) : null;
  for (const ff of subframes(S.T.s4b, S.T.s4d, (f) => (only ? only.includes(f) : mine(f)))) {
    await save(await mq.renderGltf(tokyo, { width: 1920, height: 1080, antialias: 4, ...S.GLTF_STYLE, ...S.tokyoView(ff / S.FPS) }), S.frameFile("tokyo", ff));
  }
  console.log("tokyo part", part, "done");
}

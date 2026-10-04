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
const dir = async (n) => { await mkdir(`seq/${n}`, { recursive: true }); return `seq/${n}/`; };

if (what === "all" || what === "s2") {
  const bunny = await load("examples/data/bunny.obj");
  const dp = await dir("plain"), ds = await dir("sss");
  const base = { width: 720, height: 720, up: [0, 1, 0], distance: 0.25, antialias: 3, background: "none" };
  for (const f of frames(S.S2.compile - 0.1, S.T.s3)) {
    const t = f / S.FPS, azimuth = S.azS2(t);
    if (t < S.S2.sss + S.S2.sssFade + 0.1) await save(await mq.renderObj(bunny, { ...base, azimuth }), dp + S.pad(f) + ".webp");
    if (t > S.S2.sss - 0.1) await save(await mq.renderObj(bunny, { ...base, azimuth,
      lights: [{ type: "positional", vector: [-0.1, 0.14, -0.04], color: "#ff0000", intensity: 3 }],
      sss: { intensity: 4, power: 3.5, distortion: 0.2 } }), ds + S.pad(f) + ".webp");
  }
  console.log("s2 done");
}
if (what === "all" || what === "s3") {
  const bunny = await load("examples/data/bunny.obj"), skull = await load("examples/data/brain_skull.obj"), crank = await load("examples/data/crankshaft.obj");
  const { bbox_min, bbox_max } = await mq.infoObj(bunny);
  const bc = bbox_min.map((v, i) => (v + bbox_max[i]) / 2);
  const orbit = (u, k) => { const a = 2 * Math.PI * (0.9 * u + k / 3); return [bc[0] + 0.14 * Math.cos(a), bc[1] + 0.05 + 0.04 * Math.sin(2 * a), bc[2] + 0.14 * Math.sin(a)]; };
  const pistons = Object.fromEntries("ABCDEF".split("").map((k) => ["Model__Piston_" + k, { label: "Piston " + k }]));
  const d = await dir("feat");
  const Z = 920, b = { width: Z, height: Z, antialias: 3, background: "none" };
  const bb = { ...b, up: [0, 1, 0], distance: 0.25 };
  for (const f of frames(S.T.s3, S.T.s4a)) {
    const t = f / S.FPS, { i, u } = S.cutAt(t), az = S.azS3(t);
    let r;
    switch (S.CUTS[i].id) {
      case "lights": r = await mq.renderObj(bunny, { ...bb, azimuth: az, color: "#d8d8de", specular: 0.7, shininess: 40, ambient: 0.08,
        lights: [["#ff2d55", 0], ["#22d3ee", 1], ["#facc15", 2]].map(([color, k]) => ({ type: "positional", vector: orbit(u, k), color, intensity: 1.6 })) }); break;
      case "glow": r = await mq.renderObj(bunny, { ...bb, background: S.CUTS[i].bg, azimuth: az, color: "#0b0d14", specular: 0.8, fresnel: 0.6, glow: { color: "#22d3ee", intensity: 0.32, radius: 16 } }); break;
      case "ann": r = await mq.renderObj(crank, { ...b, up: [0, -1, 0], camera: [-100 - 30 * u, -100, 500], zoom: 1.15, color: "#5b6070", specular: 0.5, highlight: pistons,
        annotations: { groups: Object.keys(pistons), color: "#0e0f13", font_size: 30, offset: 110 } }); break;
      case "xray": r = await mq.renderObj(skull, { ...b, azimuth: 232 + 36 * u, up: [0, 1, 0], distance: 200, highlight: { Skull: { color: "#e8e8e8" }, Brain: { color: "#ee69b4" } }, xray_opacity: 0.3, mode: "x-ray" }); break;
      case "clip": r = await mq.renderObj(skull, { ...b, azimuth: 214 + 9 * u, up: [0, 1, 0], distance: 180, highlight: { Skull: { color: "#f1f1f1" }, Brain: { color: "#ff69b4" } },
        clip: { plane: [2, -1, 0, S.lerp(125, 1, S.easeInOut(S.prog(u, 0.1, 0.7)))], cap: false }, cull_backface: false }); break;
      case "explode": r = await mq.renderObj(crank, { ...b, up: [0, -1, 0], camera: [-200, -200, 500], color: "#2a2d36", specular: 0.5, explode: 0.8 * S.easeOutCubic(S.prog(u, 0.08, 0.8)) }); break;
    }
    await save(r, d + S.pad(f) + ".webp");
  }
  console.log("s3 done");
}
if (what === "all" || what === "scad") {
  const d = await dir("scad");
  for (const f of frames(S.T.s4a, S.T.s4b)) {
    const t = f / S.FPS;
    const ply = await mq.compileScad(S.scadSrc(S.scadHole(t).toFixed(2)), { facets: 100 });
    await save(await mq.renderPly(ply, { width: 900, height: 900, azimuth: S.scadAz(t), elevation: 33, up: [0, 1, 0], fov: 40, zoom: 0.92, color: "#f9d72c", specular: 0, cull_backface: false, antialias: 3, background: "none" }), d + S.pad(f) + ".webp");
  }
  console.log("scad done");
}
if (what === "tokyo") {
  const tokyo = await load("examples/data/gltf/tokyo.glb");
  const d = await dir("tokyo");
  const C0 = [-86, -50, -25], cam0 = [-71.915, 159.083, 1026.801], up0 = [0, 0.989, -0.145], k = 0.8, shift = 150;
  const rotY = (v, a) => { const c = Math.cos(a), s = Math.sin(a); return [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]; };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const norm = (v) => { const l = Math.hypot(...v); return v.map((x) => x / l); };
  const only = process.env.ONLY ? process.env.ONLY.split(",").map(Number) : null;
  for (const f of frames(S.T.s4b, S.T.s5)) {
    if (only ? !only.includes(f) : f % +parts !== +part) continue;
    const p = S.tokyoParams(f / S.FPS), a = (p.angle * Math.PI) / 180;
    const off = rotY(cam0.map((v, i) => (v - C0[i]) * k), a), up = rotY(up0, a);
    const right = norm(cross(off.map((v) => -v), up));
    const center = C0.map((v, i) => v - right[i] * shift), camera = center.map((v, i) => v + off[i]);
    await save(await mq.renderGltf(tokyo, { width: 1920, height: 1080, camera, center, up, auto_center: false, auto_fit: false, background: "#182028", shadows: { resolution: 4096, softness: 2, light_size: 6 }, antialias: 4, ssao: true, time: p.time }), d + S.pad(f) + ".webp");
  }
  console.log("tokyo part", part, "done");
}

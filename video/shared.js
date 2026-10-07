export const FPS = 30, DUR = 40, W = 1920, H = 1080;
export const BPM = 120, BEAT = 60 / BPM;
const b = (n) => n * BEAT;
export const T = { s1: 0, s2: b(6), s3: b(16), cut: b(3), s4c: b(37), s4a: b(45), s4b: b(51), s4d: b(58), s5: b(66), s6: b(72), end: DUR };
export const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
export const prog = (t, a, b) => clamp((t - a) / (b - a));
export const lerp = (a, b, p) => a + (b - a) * p;
export const easeOutCubic = (p) => 1 - Math.pow(1 - p, 3);
export const easeOutExpo = (p) => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p));
export const easeInOut = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
export const easeInCubic = (p) => p * p * p;
export const easeOutBack = (p) => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2); };
export const spring = (t, t0, stiffness = 170, damping = 16) => {
  const dt = t - t0;
  if (dt <= 0) return 0;
  const w0 = Math.sqrt(stiffness), z = damping / (2 * w0);
  if (z >= 1) return 1 - Math.exp(-w0 * dt) * (1 + w0 * dt);
  const wd = w0 * Math.sqrt(1 - z * z);
  return 1 - Math.exp(-z * w0 * dt) * (Math.cos(wd * dt) + (z * w0 / wd) * Math.sin(wd * dt));
};

export const SUB = 4, SHUTTER = 0.5;
export const subOffsets = Array.from({ length: SUB }, (_, k) => (k * SHUTTER) / SUB);

export const S2 = { type0: 3.3, type1: 3.9, compile: b(8), scrub0: b(9), scrubMid: 5.25, scrub1: 5.95, sss: b(12), sssFade: 0.3, chips: 6.75 };
export function azS2(t) {
  if (t < S2.scrub0) return 180;
  if (t < S2.scrubMid) return lerp(180, 228, easeInOut(prog(t, S2.scrub0, S2.scrubMid)));
  if (t < S2.scrub1) return lerp(228, 150, easeInOut(prog(t, S2.scrubMid, S2.scrub1)));
  return lerp(150, 185, easeOutCubic(prog(t, S2.scrub1, T.s3)));
}

const f2 = (v) => (Math.abs(v) < 0.005 ? 0 : v).toFixed(2);
const vec = (v) => `(${v.map(f2).join(", ")})`;
export const BUNNY_C = [-0.0168, 0.1101, -0.0015];
export const LIGHT_COLORS = ["#ff2d55", "#22d3ee", "#facc15"];
export const lightPos = (u, k) => { const a = 2 * Math.PI * (0.9 * u + k / 3); return [BUNNY_C[0] + 0.14 * Math.cos(a), BUNNY_C[1] + 0.05 + 0.04 * Math.sin(2 * a), BUNNY_C[2] + 0.14 * Math.sin(a)]; };
export const decAt = (u) => 0.85 * easeInOut(prog(u, 0.1, 0.85));
export const pointSizeAt = (u) => lerp(0.0004, 0.0035, easeInOut(prog(u, 0.3, 1.0)));
export const fogAt = (u) => Math.round(lerp(0, 100, easeInOut(prog(u, 0.1, 0.75))));
export const clipAt = (u) => lerp(125, 1, easeInOut(prog(u, 0.1, 0.7)));
export const explodeAt = (u) => 0.8 * easeOutCubic(prog(u, 0.08, 0.8));
const f1 = (v) => v.toFixed(1), f3 = (v) => v.toFixed(3);
export function camAt(id, u, t) {
  const az = azS3(t);
  switch (id) {
    case "lights": case "decimate": return { p: { distance: 0.25, azimuth: az }, lines: ["distance: 0.25,", `azimuth: ${f1(az)},`] };
    case "points": return { p: { azimuth: 18 + 30 * u, elevation: 10, zoom: 1.5 }, lines: [`azimuth: ${f1(18 + 30 * u)},`, "elevation: 10,", "zoom: 1.5,"] };
    case "fog": { const cam = [0.58 - 0.08 * u, 0.25, 0.4 + 0.04 * u];
      return { p: { camera: cam, center: [0.1, 0.09, -0.5], auto_center: false, auto_fit: false, fov: 42, zoom: 1.15, pan: [0.1, 0.02] },
        lines: [`camera: (${cam.map(f3).join(", ")}),`, "center: (0.1, 0.09, -0.5),", "auto_center: false,", "auto_fit: false,", "fov: 42, zoom: 1.15, pan: (0.1, 0.02),"] }; }
    case "ann": return { p: { camera: [-100 - 30 * u, -100, 500], zoom: 1.15 }, lines: [`camera: (${f1(-100 - 30 * u)}, -100, 500),`, "zoom: 1.15,"] };
    case "clip": return { p: { azimuth: 214 + 9 * u, distance: 180 }, lines: [`azimuth: ${f1(214 + 9 * u)},`, "distance: 180,"] };
    case "explode": return { p: { camera: [-200, -200, 500] }, lines: ["camera: (-200, -200, 500),"] };
  }
}
const snippet = (fn, file, args) => [`#import "@preview/maquette:0.2.0": ${fn}`, "", `#${fn}(`, `  read("${file}"${fn === "render-ply" ? ", encoding: none" : ""}),`, ...args.map((l) => "  " + l), ")"];
export const CUTS = [
  { id: "lights", label: "Multiple\nlights", bg: "#0e0f13", fg: "#f6f7f9",
    code: (u, t, id) => snippet("render-obj", "bunny.obj", ["up: (0, 1, 0),", ...camAt(id, u, t).lines, "lights: (", ...LIGHT_COLORS.map((c, k) => `  (type: "positional", vector: ${vec(lightPos(u, k))}, color: "${c}"),`), "),"]) },
  { id: "decimate", label: "Decimation", bg: "#4338ca", fg: "#ffffff",
    code: (u, t, id) => snippet("render-obj", "bunny.obj", ["up: (0, 1, 0),", ...camAt(id, u, t).lines, 'mode: "solid+wireframe",', 'wireframe: (color: "#1e1b4b", width: 1.2),', `decimate: ${f2(decAt(u))},`]) },
  { id: "points", label: "Point\nclouds", bg: "#1b1035", fg: "#f6f7f9",
    code: (u, t, id) => snippet("render-ply", "dragon.ply", ["up: (0, 1, 0),", ...camAt(id, u, t).lines, "point_splat: true,", `point_size: ${pointSizeAt(u).toFixed(4)},`, 'color_map: "scalar",', 'scalar_function: "x",', 'color_map_palette: ("#7c3aed", "#ec4899", "#f97316", "#facc15"),']) },
  { id: "fog", label: "Fog &\nglow", bg: "#0e0f13", fg: "#f6f7f9",
    code: (u, t, id) => snippet("render-obj", "bunnies.obj", ["up: (0, 1, 0),", ...camAt(id, u, t).lines, `fog: (intensity: ${fogAt(u)}, color: "#0e0f13"),`, 'glow: (color: "#a78bfa", intensity: 0.85, radius: 16),']) },
  { id: "ann", label: "Annotations", bg: "#f6f7f9", fg: "#0e0f13",
    code: (u, t, id) => snippet("render-obj", "crankshaft.obj", ["up: (0, -1, 0),", ...camAt(id, u, t).lines, 'highlight: "ABCDEF".clusters().map(k =>', '  ("Model__Piston_" + k, (label: "Piston " + k))).to-dict(),', 'annotations: (color: "#dc2626", font_size: 30, offset: 110,', '  groups: "ABCDEF".clusters().map(k => "Model__Piston_" + k)),']) },
  { id: "clip", label: "Section\ncuts", bg: "#239DAD", fg: "#0b1418",
    code: (u, t, id) => snippet("render-obj", "brain_skull.obj", ["up: (0, 1, 0),", ...camAt(id, u, t).lines, 'highlight: (Skull: (color: "#f1f1f1"), Brain: (color: "#ff69b4")),', `clip: (plane: (2, -1, 0, ${Math.round(clipAt(u))}), cap: false),`, "cull_backface: false,"]) },
  { id: "explode", label: "Exploded\nviews", bg: "#f9d72c", fg: "#14150f",
    code: (u, t, id) => snippet("render-obj", "crankshaft.obj", ["up: (0, -1, 0),", ...camAt(id, u, t).lines, `explode: ${f2(explodeAt(u))},`]) },
];
export function cutAt(t) {
  const i = clamp(Math.floor((t - T.s3) / T.cut + 1e-9), 0, CUTS.length - 1);
  return { i, u: t - T.s3 - i * T.cut };
}
export const azS3 = (t) => 185 + 40 * (t - T.s3);

export function scadHole(t) {
  const a = T.s4a;
  if (t < a + 0.6) return 25;
  if (t < a + 1.15) return lerp(25, 13, easeInOut(prog(t, a + 0.6, a + 1.15)));
  if (t < a + 1.8) return lerp(13, 31, easeInOut(prog(t, a + 1.15, a + 1.8)));
  return lerp(31, 25, easeInOut(prog(t, a + 1.8, a + 2.2)));
}
export const scadAz = (t) => 205 + 16 * (t - T.s4a);
export const scadSrc = (hole) => `$fn = 100;
module rod(d, h) cylinder(h, d = d, center = true);
hole = ${hole};
len = 62.5;
difference() {
  sphere(d = 50);
  rod(hole, len);
  rotate([90, 0, 0]) rod(hole, len);
}
color([0.5, 0.3, 0.1, 0.6])
rotate([0, 90, 0]) rod(hole, len);
`;

export const tokyoParams = (t) => ({ time: 5.4 + (t - T.s4b), angle: lerp(-13, 9, prog(t, T.s4b, T.s4d)) });
const rotY = (v, a) => { const c = Math.cos(a), s = Math.sin(a); return [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]; };
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (v) => { const l = Math.hypot(...v); return v.map((x) => x / l); };
export function tokyoView(t) {
  const C0 = [-86, -50, -25], cam0 = [-71.915, 159.083, 1026.801], up0 = [0, 0.989, -0.145], p = tokyoParams(t), a = (p.angle * Math.PI) / 180;
  const off = rotY(cam0.map((v, i) => (v - C0[i]) * 0.8), a), up = rotY(up0, a);
  const right = norm(cross(off.map((v) => -v), up));
  const center = C0.map((v, i) => v - right[i] * 240);
  return { camera: center.map((v, i) => v + off[i]), center, up, time: p.time };
}
export const GLTF_STYLE = { auto_center: false, auto_fit: false, shadows: { resolution: 4096, softness: 2, light_size: 6 }, ssao: true, background: "#182028" };
export function gltfCode(t) {
  const v = tokyoView(t), tup = (a, d) => `(${a.map((x) => x.toFixed(d)).join(", ")})`;
  return ['#import "@preview/maquette-gltf:0.1.0": render-gltf', "", "#render-gltf(", '  read("tokyo.glb", encoding: none),',
    `  camera: ${tup(v.camera, 1)},`, `  center: ${tup(v.center, 1)},`, `  up: ${tup(v.up, 3)},`, "  auto_center: false,", "  auto_fit: false,",
    "  shadows: (resolution: 4096, softness: 2, light_size: 6),", "  ssao: true,", `  time: ${v.time.toFixed(2)},`, '  background: "#182028",', ")"];
}

export const PROJS = ["perspective", "fisheye", "curvilinear", "tiny-planet"];
export const PROJ_SEG = b(2);
export const projAt = (t) => { const i = clamp(Math.floor((t - T.s4c) / PROJ_SEG + 1e-9), 0, PROJS.length - 1); return { i, u: t - T.s4c - i * PROJ_SEG }; };
export const projYaw = (t) => 20 + 9 * (t - T.s4c);

export const MOL_SWITCH = T.s4d + b(4);
export const molAz = (t) => 30 + 22 * (t - T.s4d);
export const molZoom = (t) => lerp(1.15, 1.5, easeInOut(prog(t, T.s4d, T.s5)));
export const molStyle = (spacefill) => spacefill
  ? { representation: "spacefill", "color-theme": "element-symbol", quality: "low", specular: 0.3, ssao: { strength: 1.6 } }
  : { representation: "cartoon", "color-theme": "chain-id", quality: "high", specular: 0.5, ssao: true };
export function molCode(t, spacefill) {
  const m = molStyle(spacefill);
  return ['#import "@preview/molfig:0.1.5"', "", "#molfig.render(", '  read("1AON.pdb", encoding: none),', '  format: "pdb",',
    `  representation: "${m.representation}",`, `  color-theme: "${m["color-theme"]}",`, `  quality: "${m.quality}",`, "  config: (",
    `    azimuth: ${molAz(t).toFixed(1)},`, "    elevation: 18,", `    zoom: ${molZoom(t).toFixed(2)},`, `    specular: ${m.specular},`,
    `    ssao: ${spacefill ? "(strength: 1.6)" : "true"},`, "  ),", ")"];
}

export const pad = (f) => String(f).padStart(4, "0");
export const frameFile = (seq, ff) => {
  let f = Math.floor(ff + 1e-6), k = Math.round(((ff - f) * SUB) / SHUTTER);
  if (k >= SUB) { if (ff - f > (1 + SHUTTER) / 2) { f += 1; k = 0; } else k = SUB - 1; }
  return k === 0 ? `seq/${seq}/${pad(f)}.webp` : `seq/${seq}/${pad(f)}_${k}.webp`;
};

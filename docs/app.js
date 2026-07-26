// maquette browser demo — drives the exact WASM the Typst plugin uses.
//
// The form is generated from SCHEMA (below), which mirrors maquette's full
// config surface. The same SCHEMA drives three things: the DOM controls, the
// JSON config sent to the WASM, and the minimal Typst snippet exported on the
// left. Add a field to SCHEMA and it appears in all three.

// ─────────────────────────────── WASM shim ────────────────────────────────
const WASM_URL = "maquette.wasm";
let instance, memory, _args, _result;
const importObject = { typst_env: {
  wasm_minimal_protocol_write_args_to_buffer: (ptr) =>
    new Uint8Array(memory.buffer, ptr, _args.length).set(_args),
  wasm_minimal_protocol_send_result_to_host: (ptr, len) =>
    { _result = new Uint8Array(memory.buffer, ptr, len).slice(); },
}};
function callPlugin(fn, ...args) {
  const total = args.reduce((n, a) => n + a.length, 0);
  _args = new Uint8Array(total); let o = 0;
  for (const a of args) { _args.set(a, o); o += a.length; }
  _result = new Uint8Array();
  const rc = instance.exports[fn](...args.map((a) => a.length));
  if (rc !== 0) throw new Error(new TextDecoder().decode(_result) || "render failed");
  return _result;
}

// ──────────────────────────────── SCHEMA ──────────────────────────────────
// Field: {k, label, t, def, ...}. t ∈ sel|num|rng|col|bool|txt|vec.
// Group: {k, label, t:"grp", toggle, bool, def:{}, fields:[]}  (toggle→enable box; bool→`key:true` shorthand)
// Special: t ∈ views|lights|palette|map|raw.  when:(state,local)=>bool for conditional display.
const PROJ = ["perspective","orthographic","isometric","dimetric","trimetric","military",
  "cabinet","cavalier","fisheye","stereographic","curvilinear","cylindrical","pannini","tiny-planet"];

const SCHEMA = [
  { s: "Camera & viewport", open: true, fields: [
    // `init` = starting value (a good view of the default bunny); `def` = maquette's
    // real default, used as the export baseline so the snippet stays faithful.
    { k: "_cam", label: "Camera mode", t: "sel", def: "cartesian", init: "spherical", opts: [["cartesian","Cartesian (x,y,z)"],["spherical","Spherical"]] },
    { k: "camera", label: "Position", t: "vec", def: [3,3,3], when: s => s._cam === "cartesian" },
    { k: "azimuth", label: "Azimuth °", t: "num", def: 0, init: 180, when: s => s._cam === "spherical" },
    { k: "elevation", label: "Elevation °", t: "num", def: 0, when: s => s._cam === "spherical" },
    { k: "distance", label: "Distance (0 = auto)", t: "num", def: 0, omitIf: v => v === 0, when: s => s._cam === "spherical" },
    { k: "center", label: "Look-at center", t: "vec", def: [0,0,0] },
    { k: "up", label: "Up vector", t: "vec", def: [0,0,1], init: [0,1,0] },
    { k: "projection", label: "Projection", t: "sel", def: "perspective", opts: PROJ.map(p => [p, p]) },
    { k: "fov", label: "Field of view °", t: "num", def: 45 },
    { k: "zoom", label: "Zoom", t: "rng", def: 1, init: 1.4, min: 0.3, max: 4, step: 0.05 },
    { k: "pan", label: "Pan [right, up]", t: "vec", def: [0,0] },
    { k: "auto_center", label: "Auto-center", t: "bool", def: true },
    { k: "auto_fit", label: "Auto-fit to viewport", t: "bool", def: true },
    { k: "background", label: "Background", t: "col", def: "#f0f0f0" },
    { k: "_bgNone", label: "Transparent background", t: "bool", def: false },
    { k: "width", label: "Render width px", t: "num", def: 700, noExport: true },
    { k: "height", label: "Render height px", t: "num", def: 700, noExport: true },
  ]},

  { s: "Material", fields: [
    { k: "color", label: "Model color", t: "col", def: "#4488cc" },
    { k: "opacity", label: "Opacity", t: "rng", def: 1, min: 0, max: 1, step: 0.01 },
    { k: "specular", label: "Specular", t: "rng", def: 0.2, min: 0, max: 1, step: 0.01 },
    { k: "shininess", label: "Shininess", t: "num", def: 32 },
    { k: "smooth", label: "Smooth shading", t: "bool", def: true },
    { k: "gamma_correction", label: "Gamma correction", t: "bool", def: true },
    { k: "cull_backface", label: "Back-face culling", t: "bool", def: true },
  ]},

  { s: "Shading model", fields: [
    { k: "shading", label: "Model", t: "sel", def: "", opts: [["","Blinn–Phong"],["gooch","Gooch"],["cel","Cel"],["flat","Flat"],["normal","Normal map"]] },
    { k: "gooch_warm", label: "Gooch warm", t: "col", def: "#ffcc44", when: s => s.shading === "gooch" },
    { k: "gooch_cool", label: "Gooch cool", t: "col", def: "#4466cc", when: s => s.shading === "gooch" },
    { k: "cel_bands", label: "Cel bands", t: "num", def: 4, when: s => s.shading === "cel" },
  ]},

  { s: "Render mode", fields: [
    { k: "mode", label: "Mode", t: "sel", def: "solid", opts: [["solid","Solid"],["wireframe","Wireframe"],["solid+wireframe","Solid + wireframe"],["x-ray","X-ray"]] },
    { k: "xray_opacity", label: "X-ray opacity", t: "rng", def: 0.1, min: 0, max: 1, step: 0.01, when: s => s.mode === "x-ray" },
    { k: "stroke", label: "Edge stroke", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
    { k: "wireframe", label: "Wireframe style", t: "grp", toggle: true, def: { color: "#000000", width: 1 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 1 },
    ]},
  ]},

  { s: "Lighting", fields: [
    { k: "light_dir", label: "Light direction", t: "vec", def: [1,2,3] },
    { k: "ambient", label: "Ambient", t: "rng", def: 0.15, min: 0, max: 1, step: 0.01, when: s => !s._hemi.__on },
    { k: "_hemi", label: "Hemisphere ambient", t: "grp", toggle: true, def: { intensity: 0.3, sky: "#ccd4e0", ground: "#d4ccc4" }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "sky", label: "Sky", t: "col", def: "#ccd4e0" },
      { k: "ground", label: "Ground", t: "col", def: "#d4ccc4" },
    ]},
    { k: "fresnel", label: "Fresnel rim", t: "grp", toggle: false, def: { intensity: 0.3, power: 5 }, fields: [
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "power", label: "Power", t: "num", def: 5 },
    ]},
    { k: "tone_mapping", label: "Tone mapping", t: "grp", toggle: false, def: { method: "", exposure: 1 }, fields: [
      { k: "method", label: "Method", t: "sel", def: "", opts: [["","None"],["aces","ACES"],["reinhard","Reinhard"]] },
      { k: "exposure", label: "Exposure", t: "rng", def: 1, min: 0, max: 4, step: 0.05 },
    ]},
    { k: "sss", label: "Subsurface scattering", t: "grp", toggle: true, bool: true, def: { intensity: 0.5, power: 3, distortion: 0.2 }, fields: [
      { k: "intensity", label: "Intensity", t: "num", def: 0.5 },
      { k: "power", label: "Power", t: "num", def: 3 },
      { k: "distortion", label: "Distortion", t: "num", def: 0.2 },
    ]},
    { k: "lights", label: "Extra lights", t: "lights", def: [] },
  ]},

  { s: "Color mapping", fields: [
    { k: "color_map", label: "Map", t: "sel", def: "", opts: [["","Off"],["overhang","Overhang"],["curvature","Curvature"],["scalar","Scalar"]] },
    { k: "overhang_angle", label: "Overhang angle °", t: "num", def: 45, when: s => s.color_map === "overhang" },
    { k: "scalar_function", label: "Scalar function", t: "txt", def: "", when: s => s.color_map === "scalar" },
    { k: "vertex_smoothing", label: "Vertex smoothing 0–4", t: "num", def: 4, when: s => s.color_map !== "" },
    { k: "color_map_palette", label: "Palette", t: "palette", def: [], when: s => s.color_map === "curvature" || s.color_map === "scalar" },
  ]},

  { s: "Outlines", fields: [
    { k: "outline", label: "Silhouette outline", t: "grp", toggle: true, bool: true, def: { color: "#000000", width: 2 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#000000" },
      { k: "width", label: "Width", t: "num", def: 2 },
    ]},
  ]},

  { s: "Shadows", fields: [
    { k: "ground_shadow", label: "Ground shadow", t: "grp", toggle: true, bool: true, def: { opacity: 0.3, color: "#000000" }, fields: [
      { k: "opacity", label: "Opacity", t: "rng", def: 0.3, min: 0, max: 1, step: 0.01 },
      { k: "color", label: "Color", t: "col", def: "#000000" },
    ]},
    { k: "shadows", label: "Cast shadows (self-shadowing)", t: "grp", toggle: true, bool: true, def: {
        per_pixel: false, light_size: 0, strength: 1, softness: 1, color: "", resolution: 512, omni: false, bias: 0.0008, normal_bias: 2, slope_bias: 1 },
      fields: [
        { k: "per_pixel", label: "Per-pixel", t: "bool", def: false },
        { k: "light_size", label: "Light size (soft)", t: "num", def: 0 },
        { k: "strength", label: "Strength", t: "rng", def: 1, min: 0, max: 1, step: 0.01 },
        { k: "softness", label: "Softness", t: "num", def: 1 },
        { k: "color", label: "Tint (blank = none)", t: "col", def: "", allowBlank: true },
        { k: "resolution", label: "Resolution", t: "num", def: 512 },
        { k: "omni", label: "Omnidirectional", t: "bool", def: false },
        { k: "bias", label: "Bias", t: "num", def: 0.0008 },
        { k: "normal_bias", label: "Normal bias", t: "num", def: 2 },
        { k: "slope_bias", label: "Slope bias", t: "num", def: 1 },
      ]},
  ]},

  { s: "Post-processing", fields: [
    { k: "antialias", label: "Antialiasing", t: "sel", def: 1, num: true, opts: [[0,"Off"],[1,"FXAA"],[2,"SSAA ×2"],[4,"SSAA ×4"]] },
    { k: "ssao", label: "Ambient occlusion", t: "grp", toggle: true, bool: true, def: { samples: 16, radius: 0.5, bias: 0.025, strength: 1 }, fields: [
      { k: "samples", label: "Samples", t: "num", def: 16 },
      { k: "radius", label: "Radius", t: "num", def: 0.5 },
      { k: "bias", label: "Bias", t: "num", def: 0.025 },
      { k: "strength", label: "Strength", t: "rng", def: 1, min: 0, max: 2, step: 0.05 },
    ]},
    { k: "bloom", label: "Bloom", t: "grp", toggle: true, bool: true, def: { threshold: 0.8, intensity: 0.3, radius: 10 }, fields: [
      { k: "threshold", label: "Threshold", t: "rng", def: 0.8, min: 0, max: 1, step: 0.01 },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.3, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 10 },
    ]},
    { k: "glow", label: "Glow", t: "grp", toggle: true, bool: true, def: { color: "#ffffff", intensity: 0.5, radius: 15 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#ffffff" },
      { k: "intensity", label: "Intensity", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
      { k: "radius", label: "Radius", t: "num", def: 15 },
    ]},
    { k: "sharpen", label: "Sharpen", t: "grp", toggle: true, bool: true, def: { strength: 0.5 }, fields: [
      { k: "strength", label: "Strength", t: "rng", def: 0.5, min: 0, max: 2, step: 0.05 },
    ]},
  ]},

  { s: "Geometry & clipping", fields: [
    { k: "clip", label: "Clip plane", t: "grp", toggle: true, def: {
        source: "camera", depth: 0.5, keep: "far", cap: true,
        hatch: false, hstyle: "lines", hangle: 45, hspacing: 6, hwidth: 0.6, hcolor: "#333333" },
      build: "clip", fields: [
        { k: "source", label: "From", t: "sel", def: "camera", opts: [["camera","Camera"],["x","X axis"],["y","Y axis"],["z","Z axis"]] },
        { k: "depth", label: "Depth", t: "rng", def: 0.5, min: 0, max: 1, step: 0.01 },
        { k: "keep", label: "Keep", t: "sel", def: "far", opts: [["far","Far half"],["near","Near half"]] },
        { k: "cap", label: "Cap cross-section", t: "bool", def: true },
        { k: "hatch", label: "Hatch cap", t: "bool", def: false },
        { k: "hstyle", label: "Hatch style", t: "sel", def: "lines", opts: [["lines","Lines"],["cross","Cross"],["crosses","Crosses"]], when: (s,l) => l.hatch },
        { k: "hangle", label: "Hatch angle °", t: "num", def: 45, when: (s,l) => l.hatch },
        { k: "hspacing", label: "Hatch spacing", t: "num", def: 6, when: (s,l) => l.hatch },
        { k: "hwidth", label: "Hatch width", t: "num", def: 0.6, when: (s,l) => l.hatch },
        { k: "hcolor", label: "Hatch color", t: "col", def: "#333333", when: (s,l) => l.hatch },
      ]},
    { k: "explode", label: "Explode", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
    { k: "decimate", label: "Decimate", t: "rng", def: 0, min: 0, max: 1, step: 0.02 },
    { k: "point_size", label: "Point size (PLY clouds)", t: "num", def: 0, omitIf: v => v === 0 },
  ]},

  { s: "Multi-view", fields: [
    { k: "views", label: "Grid views", t: "views", def: [], opts: ["front","back","left","right","top","bottom","isometric"] },
    { k: "grid_labels", label: "Grid labels", t: "bool", def: true, when: s => s.views.length > 0 },
    { k: "turntable", label: "Turntable", t: "grp", toggle: true, def: { iterations: 6, elevation: 40 }, build: "turntable", fields: [
      { k: "iterations", label: "Frames", t: "num", def: 6 },
      { k: "elevation", label: "Elevation °", t: "num", def: 40 },
    ]},
  ]},

  { s: "OBJ groups", fields: [
    { k: "materials", label: "Materials (name → color)", t: "map", def: [] },
    { k: "highlight", label: "Highlight (group → color)", t: "map", def: [] },
    { k: "annotations", label: "Annotations", t: "grp", toggle: true, bool: true, def: { color: "#333333", font_size: 12, offset: 40 }, fields: [
      { k: "color", label: "Color", t: "col", def: "#333333" },
      { k: "font_size", label: "Font size", t: "num", def: 12 },
      { k: "offset", label: "Offset", t: "num", def: 40 },
    ]},
  ]},

  { s: "Diagnostics", fields: [
    { k: "debug", label: "Debug overlay", t: "bool", def: false },
    { k: "debug_color", label: "Debug color", t: "col", def: "#cc2222", when: s => s.debug },
  ]},

  { s: "Advanced (raw JSON, merged last)", fields: [
    { k: "_raw", label: "", t: "raw", def: "" },
  ]},
];

// ──────────────────────────── state (nested) ──────────────────────────────
function initState() {
  const st = {};
  for (const sec of SCHEMA) for (const f of sec.fields) {
    if (f.t === "grp") st[f.k] = { __on: !!f.def.__on, ...structuredClone(f.def) };
    else { const d = f.init !== undefined ? f.init : f.def; st[f.k] = Array.isArray(d) ? d.slice() : d; }
  }
  return st;
}
const state = initState();

// ─────────────────────── Typst / config value helpers ─────────────────────
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const num = (v) => Number.isInteger(v) ? String(v) : String(+v.toFixed(4));
function fmtT(v) {                                  // JS value → Typst literal
  if (typeof v === "string") return `"${v}"`;
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return num(v);
  if (Array.isArray(v)) return `(${v.map(fmtT).join(", ")})`;
  return `(${Object.entries(v).map(([k,x]) => `${k}: ${fmtT(x)}`).join(", ")})`;
}

// Collect a group's subfields into a plain object; `mode` = "cfg" | "diff".
function group(f, mode) {
  const s = state[f.k];
  if (f.build === "clip") {                          // clip: assemble source/keep/hatch
    const o = { depth: s.depth };
    if (s.source === "camera") o.from = "camera"; else o.axis = s.source;
    if (s.keep === "near") o.keep = "near";
    if (mode === "cfg" || s.cap !== true) o.cap = s.cap;
    if (s.hatch) o.hatch = { style: s.hstyle, angle: s.hangle, spacing: s.hspacing, width: s.hwidth, color: s.hcolor };
    return o;
  }
  if (f.build === "turntable") return { iterations: s.iterations, elevation: s.elevation };
  const o = {};
  for (const sub of f.fields) {
    const v = s[sub.k];
    if (sub.allowBlank && v === "") continue;        // blank optional color → omit
    if (mode === "diff" && eq(v, sub.def)) continue; // export: only changed subfields
    o[sub.k] = v;
  }
  return o;
}

// ─────────────────────── build render config (→ WASM) ─────────────────────
function buildConfig() {
  const c = {};
  for (const sec of SCHEMA) for (const f of sec.fields) {
    if (f.when && !f.when(state, state)) continue;
    if (f.k[0] === "_") continue;                     // UI-only fields
    if (f.omitIf && f.omitIf(state[f.k])) continue;
    switch (f.t) {
      case "grp":
        if (f.toggle && !state[f.k].__on) break;
        c[f.k] = group(f, "cfg");
        break;
      case "views": if (state.views.length) c.views = state.views.slice(); break;
      case "palette": if (state[f.k].length) c[f.k] = state[f.k].slice(); break;
      case "lights": if (state.lights.length) c.lights = state.lights.map(l => ({ ...l })); break;
      case "map": if (state[f.k].length) c[f.k] = Object.fromEntries(state[f.k].filter(r => r[0])); break;
      case "raw": break;
      default: c[f.k] = state[f.k];
    }
  }
  // polymorphic: hemisphere ambient overrides the scalar
  if (state._hemi.__on) c.ambient = { intensity: state._hemi.intensity, sky: state._hemi.sky, ground: state._hemi.ground };
  // background transparent
  if (state._bgNone) c.background = "none";
  // raw JSON merge (last word)
  if (state._raw.trim()) { try { Object.assign(c, JSON.parse(state._raw)); } catch {} }
  return c;
}

// ─────────────────────── build Typst snippet (minimal) ────────────────────
function buildTypst() {
  const fn = { obj: "render-obj", stl: "render-stl", ply: "render-ply" }[ext(model.name)] || "render-obj";
  const P = [];
  const push = (k, v) => P.push(`${k}: ${v}`);
  for (const sec of SCHEMA) for (const f of sec.fields) {
    if (f.when && !f.when(state, state)) continue;
    if (f.noExport || f.k === "_cam" || f.k === "width" || f.k === "height") continue;
    if (f.omitIf && f.omitIf(state[f.k])) continue;
    if (f.k === "_bgNone") { if (state._bgNone) push("background", "none"); continue; }
    if (f.k === "background") { if (!state._bgNone && state.background !== f.def) push("background", fmtT(state.background)); continue; }
    if (f.k === "ambient") { if (!state._hemi.__on && state.ambient !== f.def) push("ambient", num(state.ambient)); continue; }
    if (f.k === "_hemi") { if (state._hemi.__on) push("ambient", fmtT(group(f, "cfg"))); continue; }
    if (f.k[0] === "_") continue;
    switch (f.t) {
      case "grp": {
        if (f.toggle && !state[f.k].__on) break;
        if (f.build === "clip") { push(f.k, fmtT(group(f, "cfg"))); break; } // clip always needs its dict
        if (f.build === "turntable") { const s = state[f.k]; push("turntable", s.elevation === f.def.elevation ? num(s.iterations) : fmtT({ iterations: s.iterations, elevation: s.elevation })); break; }
        const d = group(f, "diff");
        if (Object.keys(d).length === 0) { if (f.toggle && f.bool) push(f.k, "true"); break; } // enabled-at-defaults → `key: true`; always-on group unchanged → omit
        push(f.k, fmtT(d));
        break;
      }
      case "views": if (state.views.length) push("views", fmtT(state.views)); break;
      case "palette": if (state[f.k].length) push(f.k, fmtT(state[f.k])); break;
      case "lights": if (state.lights.length) push("lights", fmtT(state.lights)); break;
      case "map": { const rows = state[f.k].filter(r => r[0]); if (rows.length) push(f.k, `(${rows.map(([n,v]) => `"${n}": ${fmtT(v)}`).join(", ")})`); break; }
      case "raw": break;
      default: if (!eq(state[f.k], f.def)) push(f.k, fmtT(state[f.k]));
    }
  }
  const body = P.length ? `#${fn}(model,\n  ${P.join(",\n  ")},\n)` : `#${fn}(model)`;
  return `#import "@preview/maquette:0.1.1": ${fn}\n\n#let model = read("${model.name}", encoding: none)\n\n${body}`;
}

// Tiny Typst highlighter — the generated snippet has a small, known grammar, so a
// hand-rolled tokenizer beats pulling in a library and keeps the demo self-contained.
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const HL_RULES = [
  ["ws", /^\s+/], ["comment", /^\/\/[^\n]*/],
  ["string", /^"(?:[^"\\]|\\.)*"/], ["number", /^-?\d+(?:\.\d+)?/],
  ["directive", /^#[A-Za-z][\w-]*/], ["kw", /^(?:none|true|false|auto)\b/],
  ["ident", /^[A-Za-z_][\w-]*/], ["punct", /^[(){}\[\],:*+=]/],
];
function highlightLine(line) {
  let s = line, out = "";
  outer: while (s) {
    for (let [type, re] of HL_RULES) {
      const m = re.exec(s);
      if (!m) continue;
      const txt = m[0];
      if (type === "ident" && /^\s*:/.test(s.slice(txt.length))) type = "key"; // `key:` → property
      out += (type === "ws") ? esc(txt) : `<span class="t-${type}">${esc(txt)}</span>`;
      s = s.slice(txt.length);
      continue outer;
    }
    out += esc(s[0]); s = s.slice(1); // fallback: emit one char
  }
  return out || "&nbsp;";
}
function renderCode() {
  $("code").innerHTML = buildTypst().split("\n").map((l, i) =>
    `<div class="cline"><span class="gutter">${i + 1}</span><span class="src">${highlightLine(l)}</span></div>`
  ).join("");
}

// ─────────────────────────────── DOM engine ───────────────────────────────
const $ = (id) => document.getElementById(id);
const ext = (name) => name.split(".").pop().toLowerCase();
let model = { name: "bunny.obj", bytes: null };
const conds = []; // {node, when, local} for visibility refresh

function ctl(f, slot, local) {
  const wrap = document.createElement("div");
  wrap.className = "ctl";
  const set = (v) => { slot[f.k] = v; onChange(); };
  const cur = slot[f.k];

  if (f.t === "bool") {
    const lab = document.createElement("label"); lab.className = "chk";
    const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = !!cur;
    cb.onchange = () => set(cb.checked);
    lab.append(cb, document.createTextNode(f.label)); wrap.append(lab);
  } else {
    const lab = document.createElement("label");
    const span = document.createElement("span"); span.textContent = f.label; lab.append(span);
    let input, valEl;
    if (f.t === "sel") {
      input = document.createElement("select");
      for (const [v, t] of f.opts) { const o = document.createElement("option"); o.value = v; o.textContent = t; input.append(o); }
      input.value = cur;
      input.onchange = () => set(f.num ? +input.value : input.value);
    } else if (f.t === "rng") {
      valEl = document.createElement("span"); valEl.className = "val"; valEl.textContent = (+cur).toFixed(2); lab.append(valEl);
      input = document.createElement("input"); input.type = "range"; input.min = f.min; input.max = f.max; input.step = f.step; input.value = cur;
      input.oninput = () => { valEl.textContent = (+input.value).toFixed(2); set(+input.value); };
    } else if (f.t === "num") {
      input = document.createElement("input"); input.type = "number"; input.value = cur; input.step = "any";
      input.oninput = () => set(input.value === "" ? f.def : +input.value);
    } else if (f.t === "col") {
      input = document.createElement("input"); input.type = "color"; input.value = cur || "#000000";
      input.oninput = () => set(input.value);
    } else if (f.t === "txt") {
      input = document.createElement("input"); input.type = "text"; input.value = cur;
      input.oninput = () => set(input.value);
    } else if (f.t === "vec") {
      input = document.createElement("div"); input.className = "row";
      cur.forEach((n, i) => {
        const ni = document.createElement("input"); ni.type = "number"; ni.value = n; ni.step = "any";
        ni.oninput = () => { const a = slot[f.k].slice(); a[i] = +ni.value; set(a); };
        input.append(ni);
      });
    }
    lab.querySelector("span") && wrap.append(lab);
    wrap.append(input);
  }
  if (f.when) conds.push({ node: wrap, when: f.when, local });
  return wrap;
}

function groupNode(f) {
  const box = document.createElement("div"); box.className = "group" + (f.toggle && !state[f.k].__on ? " off" : "");
  const head = document.createElement("label"); head.className = "head";
  if (f.toggle) {
    const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = state[f.k].__on;
    cb.onchange = () => { state[f.k].__on = cb.checked; box.classList.toggle("off", !cb.checked); onChange(); };
    head.append(cb);
  }
  head.append(document.createTextNode(f.label));
  const sub = document.createElement("div"); sub.className = "sub";
  for (const s of f.fields) sub.append(ctl(s, state[f.k], state[f.k]));
  box.append(head, sub);
  return box;
}

function listNode(f) {         // extra lights: dynamic array
  const box = document.createElement("div");
  const render = () => {
    box.innerHTML = "";
    state.lights.forEach((L, i) => {
      const it = document.createElement("div"); it.className = "item";
      const mk = (label, el) => { const d = document.createElement("div"); d.className = "ctl"; const lb = document.createElement("label"); lb.innerHTML = `<span>${label}</span>`; d.append(lb, el); return d; };
      const sel = document.createElement("select");
      [["directional","Directional"],["positional","Positional"],["area","Area"]].forEach(([v,t]) => { const o = document.createElement("option"); o.value=v; o.textContent=t; sel.append(o); });
      sel.value = L.type; sel.onchange = () => { L.type = sel.value; onChange(); };
      const vec = document.createElement("div"); vec.className = "row";
      L.vector.forEach((n, j) => { const ni = document.createElement("input"); ni.type="number"; ni.step="any"; ni.value=n; ni.oninput=()=>{L.vector[j]=+ni.value;onChange();}; vec.append(ni); });
      const col = document.createElement("input"); col.type="color"; col.value=L.color; col.oninput=()=>{L.color=col.value;onChange();};
      const inten = document.createElement("input"); inten.type="number"; inten.step="any"; inten.value=L.intensity; inten.oninput=()=>{L.intensity=+inten.value;onChange();};
      const size = document.createElement("input"); size.type="number"; size.step="any"; size.value=L.size; size.oninput=()=>{L.size=+size.value;onChange();};
      const castL = document.createElement("label"); castL.className="chk"; const cast=document.createElement("input"); cast.type="checkbox"; cast.checked=L.cast_shadow; cast.onchange=()=>{L.cast_shadow=cast.checked;onChange();}; castL.append(cast, document.createTextNode("casts shadow"));
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.title="remove"; rm.onclick=()=>{state.lights.splice(i,1); render(); onChange();};
      const top = document.createElement("div"); top.style.cssText="display:flex;gap:6px;align-items:center"; top.append(sel); top.style.marginBottom="6px"; const sp=document.createElement("span"); sp.style.flex="1"; top.append(sp, rm);
      it.append(top, mk("Vector", vec), mk("Color", col), mk("Intensity", inten), mk("Size (area)", size), castL);
      box.append(it);
    });
    const add = document.createElement("button"); add.className="mini-btn"; add.textContent="+ add light";
    add.onclick = () => { state.lights.push({ type:"directional", vector:[1,2,3], color:"#ffffff", intensity:1, cast_shadow:true, size:0 }); render(); onChange(); };
    box.append(add);
  };
  render();
  return box;
}

function paletteNode(f) {
  const box = document.createElement("div");
  const render = () => {
    box.innerHTML = ""; const row = document.createElement("div"); row.className="row"; row.style.flexWrap="wrap";
    state[f.k].forEach((c, i) => {
      const cell = document.createElement("div"); cell.style.cssText="display:flex;align-items:center;gap:2px;flex:0 0 auto";
      const col = document.createElement("input"); col.type="color"; col.value=c; col.style.width="34px"; col.oninput=()=>{state[f.k][i]=col.value;onChange();};
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.onclick=()=>{state[f.k].splice(i,1);render();onChange();};
      cell.append(col, rm); row.append(cell);
    });
    const add = document.createElement("button"); add.className="mini-btn"; add.textContent="+ color"; add.onclick=()=>{state[f.k].push("#888888");render();onChange();};
    box.append(row, add);
  };
  render(); return box;
}

function mapNode(f) {
  const box = document.createElement("div");
  const render = () => {
    box.innerHTML = "";
    state[f.k].forEach((row, i) => {
      const r = document.createElement("div"); r.className="row"; r.style.marginBottom="5px";
      const name = document.createElement("input"); name.type="text"; name.placeholder="name"; name.value=row[0]; name.oninput=()=>{row[0]=name.value;onChange();};
      const col = document.createElement("input"); col.type="color"; col.value=row[1]; col.style.flex="0 0 40px"; col.oninput=()=>{row[1]=col.value;onChange();};
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.onclick=()=>{state[f.k].splice(i,1);render();onChange();};
      r.append(name, col, rm); box.append(r);
    });
    const add = document.createElement("button"); add.className="mini-btn"; add.textContent="+ entry"; add.onclick=()=>{state[f.k].push(["","#88ccff"]);render();onChange();};
    box.append(add);
  };
  render(); return box;
}

function viewsNode(f) {
  const box = document.createElement("div"); box.className="row"; box.style.flexWrap="wrap";
  f.opts.forEach(v => {
    const lab = document.createElement("label"); lab.className="chk"; lab.style.flex="0 0 auto";
    const cb = document.createElement("input"); cb.type="checkbox"; cb.checked=state.views.includes(v);
    cb.onchange = () => { if (cb.checked) state.views.push(v); else state.views = state.views.filter(x=>x!==v); onChange(); };
    lab.append(cb, document.createTextNode(v)); box.append(lab);
  });
  return box;
}

function rawNode(f) {
  const ta = document.createElement("textarea"); ta.placeholder = '{ "color": "#ff0000" }';
  ta.value = state._raw; ta.oninput = () => { state._raw = ta.value; onChange(); };
  return ta;
}

function buildForm() {
  const root = $("form");
  for (const sec of SCHEMA) {
    const d = document.createElement("details"); if (sec.open) d.open = true;
    const sum = document.createElement("summary"); sum.textContent = sec.s; d.append(sum);
    const body = document.createElement("div"); body.className = "body";
    for (const f of sec.fields) {
      if (f.t === "grp") body.append(groupNode(f));
      else if (f.t === "lights") body.append(labelWrap(f, listNode(f)));
      else if (f.t === "palette") body.append(labelWrap(f, paletteNode(f)));
      else if (f.t === "map") body.append(labelWrap(f, mapNode(f)));
      else if (f.t === "views") body.append(labelWrap(f, viewsNode(f)));
      else if (f.t === "raw") body.append(rawNode(f));
      else body.append(ctl(f, state, state));
    }
    d.append(body); root.append(d);
  }
}
function labelWrap(f, node) {
  const w = document.createElement("div"); w.className = "ctl";
  if (f.label) { const l = document.createElement("label"); l.innerHTML = `<span>${f.label}</span>`; w.append(l); }
  w.append(node);
  if (f.when) conds.push({ node: w, when: f.when, local: state });
  return w;
}

function refreshVisibility() {
  for (const c of conds) c.node.style.display = c.when(state, c.local) ? "" : "none";
}

// ─────────────────────────────── update loop ──────────────────────────────
let t = null;
function onChange() {
  refreshVisibility();
  renderCode();
  clearTimeout(t); t = setTimeout(render, 120);
}

let lastUrl = null;
const RFN = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
function render() {
  if (!instance || !model.bytes) return;
  const fn = RFN[ext(model.name)];
  if (!fn) return showErr(`unsupported file type: .${ext(model.name)}`);
  try {
    const out = callPlugin(fn, model.bytes, new TextEncoder().encode(JSON.stringify(buildConfig())));
    // grid / turntable / debug / annotations wrap the render as SVG (text overlays);
    // everything else is a PNG. Sniff the magic and set the blob type accordingly.
    const isPng = out[0] === 0x89 && out[1] === 0x50;
    const url = URL.createObjectURL(new Blob([out], { type: isPng ? "image/png" : "image/svg+xml" }));
    $("out").src = url; $("out").style.display = "";
    if (lastUrl) URL.revokeObjectURL(lastUrl); lastUrl = url;
    showErr(null);
  } catch (e) { showErr(e.message); }
}
function showErr(m) { const e = $("err"); e.style.display = m ? "" : "none"; e.textContent = m || ""; }

// ─────────────────────────────── model I/O ────────────────────────────────
async function loadFile(file) {
  model = { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
  $("fname").textContent = model.name; $("hint").style.display = "none";
  onChange();
}
$("browse").onclick = () => $("file").click();
$("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);
$("copy").onclick = async () => { await navigator.clipboard.writeText(buildTypst()); $("copy").textContent = "copied"; setTimeout(() => ($("copy").textContent = "copy"), 1200); };

["dragenter","dragover"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); $("stage").classList.add("drag"); }));
["dragleave","drop"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); if (ev==="dragleave" && e.relatedTarget) return; $("stage").classList.remove("drag"); }));
document.addEventListener("drop", e => { const f = e.dataTransfer?.files?.[0]; if (f) loadFile(f); });

// ─────────────────────────────────── boot ─────────────────────────────────
(async function boot() {
  buildForm(); refreshVisibility();
  try {
    const wasmBytes = await (await fetch(WASM_URL)).arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(wasmBytes, importObject));
    memory = instance.exports.memory;
    model = { name: "bunny.obj", bytes: new Uint8Array(await (await fetch("bunny.obj")).arrayBuffer()) };
    $("hint").style.display = "none";
    onChange();
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
})();

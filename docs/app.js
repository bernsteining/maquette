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
  { s: "Camera & viewport", fields: [
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
];

// Tooltips (hover a label) — keyed by field key. Covers top-level fields and
// group headers, whose keys are unique. Shown via the native title attribute.
const HELP = {
  _cam: "Cartesian gives an explicit (x,y,z) camera; Spherical orbits by azimuth/elevation/distance.",
  camera: "Camera position in world space.", azimuth: "Horizontal orbit angle, in degrees.",
  elevation: "Vertical orbit angle, in degrees.", distance: "Camera distance from the center; 0 = auto-fit.",
  center: "Look-at target point.", up: "Up direction. Bunny/most OBJ models are Y-up (0,1,0).",
  projection: "Camera projection — perspective, orthographic, or one of 12 others.",
  fov: "Vertical field of view in degrees (perspective only).",
  zoom: "Magnify the auto-fit framing (>1 zooms in). Scroll the render to change.",
  pan: "Shift the framing in screen space, [right, up] as a fraction of the viewport.",
  auto_center: "Center the camera on the model's bounding box.",
  auto_fit: "Scale the model to fill the viewport.", background: "Background color.",
  _bgNone: "Render on a transparent background instead of a color.",
  width: "Render resolution width in pixels.", height: "Render resolution height in pixels.",
  color: "Base model fill color.", opacity: "Whole-model opacity (0 = invisible, 1 = opaque).",
  specular: "Specular highlight intensity.", shininess: "Specular exponent — higher = tighter highlight.",
  smooth: "Gouraud smooth shading (best with PNG).", gamma_correction: "Light in linear sRGB for accurate midtones.",
  cull_backface: "Skip triangles facing away from the camera.",
  shading: "Shading model — Blinn-Phong, Gooch, Cel, Flat, or Normal-map.",
  gooch_warm: "Gooch warm-tone color.", gooch_cool: "Gooch cool-tone color.", cel_bands: "Number of cel-shading bands.",
  mode: "Render as solid, wireframe, both, or x-ray.", xray_opacity: "Front-face opacity in x-ray mode.",
  stroke: "Draw an outline stroke on every triangle edge.", wireframe: "Wireframe edge color/width (wireframe modes).",
  light_dir: "Direction the key directional light comes from.",
  ambient: "Uniform fill light reaching all surfaces (0–1).",
  _hemi: "Sky/ground gradient ambient instead of a flat value.",
  fresnel: "Rim highlight on grazing-angle edges.", tone_mapping: "HDR tone mapping (ACES/Reinhard) + exposure.",
  sss: "Fake subsurface scattering — glow through thin geometry.", lights: "Add extra directional/positional/area lights.",
  color_map: "Color the surface by overhang, curvature, or a scalar function.",
  overhang_angle: "Overhang threshold in degrees.", scalar_function: "Expression over x,y,z, e.g. sqrt(x*x+y*y+z*z).",
  vertex_smoothing: "Smooth color-map values across vertices (0–4).", color_map_palette: "Custom gradient stops.",
  outline: "Bold silhouette contour around the model.",
  ground_shadow: "Project a silhouette shadow onto a floor plane.",
  shadows: "True self-shadowing via depth maps (PNG only).",
  antialias: "0 off · 1 FXAA · 2/4 supersampling (PNG only).",
  ssao: "Screen-space ambient occlusion — contact shadows (PNG only).",
  bloom: "Bleed light from bright areas (PNG only).", glow: "Aura around the silhouette (PNG only).",
  sharpen: "Unsharp-mask edge sharpening (PNG only).",
  clip: "Cut the model with a plane; optionally cap and hatch the section.",
  explode: "Push components outward from the center (multi-part models).",
  decimate: "Simplify the mesh (higher = fewer triangles).", point_size: "Neighbor radius for PLY point clouds.",
  views: "Render a grid of named orthographic views.", grid_labels: "Show labels on the multi-view grid.",
  turntable: "Render a spun grid of frames around the model.",
  materials: "Map OBJ material names to colors.", highlight: "Recolor named OBJ groups.",
  annotations: "Label OBJ groups on the render.", debug: "Overlay model metadata and light gizmos.",
  debug_color: "Debug overlay text color.",
};

// State→DOM sync closures (per top-level control) for programmatic updates
// (orbit, zoom, reset, shared-link restore). Search index for filtering.
const controlRefs = {};
const searchItems = [];   // {node, section, text}
const searchSections = []; // {el, open}
let lastRender = null;    // {bytes, type, ext} of the most recent render
let rafPending = false;

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
    if (f.k === "ambient" || f.k === "background") continue; // polymorphic — set below
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
      default: c[f.k] = state[f.k];
    }
  }
  c.ambient = ambientCfg();          // number, or hemisphere {intensity,sky,ground}
  c.background = bgCfg();             // color, or "none" (transparent)
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
    if (f.k === "background") { const b = bgCfg(); if (b !== f.def) push("background", b === "none" ? "none" : fmtT(b)); continue; }
    if (f.k === "_bgNone") continue;   // folded into the background branch above
    if (f.k === "ambient") { if (!state._hemi.__on && state.ambient !== f.def) push("ambient", num(state.ambient)); continue; }
    if (f.k === "_hemi") { if (state._hemi.__on) push("ambient", fmtT(ambientCfg())); continue; }
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
      default: if (!eq(state[f.k], f.def)) push(f.k, fmtT(state[f.k]));
    }
  }
  if (outputFormat === "svg") P.push('format: "svg"');
  const body = P.length ? `#${fn}(model,\n  ${P.join(",\n  ")},\n)` : `#${fn}(model)`;
  return `#import "@preview/maquette:0.1.2": ${fn}\n\n#let model = read("${model.name}", encoding: none)\n\n${body}`;
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
  elCode.innerHTML = buildTypst().split("\n").map((l, i) =>
    `<div class="cline"><span class="gutter">${i + 1}</span><span class="src">${highlightLine(l)}</span></div>`
  ).join("");
}

// ─────────────────────────────── DOM engine ───────────────────────────────
const $ = (id) => document.getElementById(id);
const ext = (name) => name.split(".").pop().toLowerCase();
let model = { name: "bunny.obj", bytes: null };
const conds = []; // {node, when, local} for visibility refresh

// Reused singletons + cached hot DOM nodes (avoid per-call allocation / lookup).
const ENC = new TextEncoder(), DEC = new TextDecoder();
const elCode = $("code"), elErr = $("err"), elOut = $("out"), elRtime = $("rtime"), elMeasure = $("measure");

// The two polymorphic config values, shared by buildConfig and buildTypst.
const ambientCfg = () => state._hemi.__on
  ? { intensity: state._hemi.intensity, sky: state._hemi.sky, ground: state._hemi.ground }
  : state.ambient;
const bgCfg = () => state._bgNone ? "none" : state.background;

function ctl(f, slot, local) {
  const wrap = document.createElement("div");
  wrap.className = "ctl";
  const set = (v) => { slot[f.k] = v; onChange(); };
  const cur = slot[f.k];
  let labelEl, sync = null;

  if (f.t === "bool") {
    labelEl = document.createElement("label"); labelEl.className = "chk";
    const cb = document.createElement("input"); cb.type = "checkbox"; cb.checked = !!cur;
    cb.onchange = () => set(cb.checked);
    labelEl.append(cb, document.createTextNode(f.label)); wrap.append(labelEl);
    sync = (v) => { cb.checked = !!v; };
  } else {
    labelEl = document.createElement("label");
    const span = document.createElement("span"); span.textContent = f.label; labelEl.append(span);
    let input, valEl;
    if (f.t === "sel") {
      input = document.createElement("select");
      for (const [v, t] of f.opts) { const o = document.createElement("option"); o.value = v; o.textContent = t; input.append(o); }
      input.value = cur;
      input.onchange = () => set(f.num ? +input.value : input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "rng") {
      valEl = document.createElement("span"); valEl.className = "val"; valEl.textContent = (+cur).toFixed(2); labelEl.append(valEl);
      input = document.createElement("input"); input.type = "range"; input.min = f.min; input.max = f.max; input.step = f.step; input.value = cur;
      input.oninput = () => { valEl.textContent = (+input.value).toFixed(2); set(+input.value); };
      sync = (v) => { input.value = v; valEl.textContent = (+v).toFixed(2); };
    } else if (f.t === "num") {
      input = document.createElement("input"); input.type = "number"; input.value = cur; input.step = "any";
      input.oninput = () => set(input.value === "" ? f.def : +input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "col") {
      input = document.createElement("input"); input.type = "color"; input.value = cur || "#000000";
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v || "#000000"; };
    } else if (f.t === "txt") {
      input = document.createElement("input"); input.type = "text"; input.value = cur;
      input.oninput = () => set(input.value);
      sync = (v) => { input.value = v; };
    } else if (f.t === "vec") {
      input = document.createElement("div"); input.className = "row";
      cur.forEach((n, i) => {
        const ni = document.createElement("input"); ni.type = "number"; ni.value = n; ni.step = "any";
        ni.oninput = () => { const a = slot[f.k].slice(); a[i] = +ni.value; set(a); };
        input.append(ni);
      });
      sync = (v) => v.forEach((n, i) => { if (input.children[i]) input.children[i].value = n; });
    }
    wrap.append(labelEl, input);
  }
  const tip = f.help || HELP[f.k];
  if (tip) labelEl.title = tip;
  if (slot === state && sync) controlRefs[f.k] = sync;
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
  { const tip = f.help || HELP[f.k]; if (tip) head.title = tip; }
  const sub = document.createElement("div"); sub.className = "sub";
  for (const s of f.fields) sub.append(ctl(s, state[f.k], state[f.k]));
  box.append(head, sub);
  return box;
}

// Dynamic array editor shared by extra-lights / palette / materials-map. `arr` is
// mutated in place (push/splice); `renderItem(item, i, rerender)` builds one row.
function dynList(arr, addLabel, newItem, renderItem, wrapRow = false) {
  const box = document.createElement("div");
  const rerender = () => {
    box.innerHTML = "";
    const host = wrapRow ? Object.assign(document.createElement("div"), { className: "row" }) : box;
    if (wrapRow) { host.style.flexWrap = "wrap"; box.append(host); }
    arr.forEach((item, i) => host.append(renderItem(item, i, rerender)));
    const add = document.createElement("button"); add.className = "mini-btn"; add.textContent = addLabel;
    add.onclick = () => { arr.push(newItem()); rerender(); onChange(); };
    box.append(add);
  };
  rerender();
  return box;
}

function listNode(f) {   // extra lights
  const mk = (label, el) => { const d = document.createElement("div"); d.className = "ctl"; const lb = document.createElement("label"); lb.innerHTML = `<span>${label}</span>`; d.append(lb, el); return d; };
  return dynList(state.lights, "+ add light",
    () => ({ type: "directional", vector: [1,2,3], color: "#ffffff", intensity: 1, cast_shadow: true, size: 0 }),
    (L, i, rerender) => {
      const it = document.createElement("div"); it.className = "item";
      const sel = document.createElement("select");
      [["directional","Directional"],["positional","Positional"],["area","Area"]].forEach(([v,t]) => { const o = document.createElement("option"); o.value=v; o.textContent=t; sel.append(o); });
      sel.value = L.type; sel.onchange = () => { L.type = sel.value; onChange(); };
      const vec = document.createElement("div"); vec.className = "row";
      L.vector.forEach((n, j) => { const ni = document.createElement("input"); ni.type="number"; ni.step="any"; ni.value=n; ni.oninput=()=>{L.vector[j]=+ni.value;onChange();}; vec.append(ni); });
      const col = document.createElement("input"); col.type="color"; col.value=L.color; col.oninput=()=>{L.color=col.value;onChange();};
      const inten = document.createElement("input"); inten.type="number"; inten.step="any"; inten.value=L.intensity; inten.oninput=()=>{L.intensity=+inten.value;onChange();};
      const size = document.createElement("input"); size.type="number"; size.step="any"; size.value=L.size; size.oninput=()=>{L.size=+size.value;onChange();};
      const castL = document.createElement("label"); castL.className="chk"; const cast=document.createElement("input"); cast.type="checkbox"; cast.checked=L.cast_shadow; cast.onchange=()=>{L.cast_shadow=cast.checked;onChange();}; castL.append(cast, document.createTextNode("casts shadow"));
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.title="remove"; rm.onclick=()=>{ state.lights.splice(i,1); rerender(); onChange(); };
      const top = document.createElement("div"); top.style.cssText="display:flex;gap:6px;align-items:center;margin-bottom:6px"; const sp=document.createElement("span"); sp.style.flex="1"; top.append(sel, sp, rm);
      it.append(top, mk("Vector", vec), mk("Color", col), mk("Intensity", inten), mk("Size (area)", size), castL);
      return it;
    });
}

function paletteNode(f) {
  return dynList(state[f.k], "+ color", () => "#888888",
    (c, i, rerender) => {
      const cell = document.createElement("div"); cell.style.cssText="display:flex;align-items:center;gap:2px;flex:0 0 auto";
      const col = document.createElement("input"); col.type="color"; col.value=c; col.style.width="34px"; col.oninput=()=>{ state[f.k][i]=col.value; onChange(); };
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.onclick=()=>{ state[f.k].splice(i,1); rerender(); onChange(); };
      cell.append(col, rm); return cell;
    }, true);
}

function mapNode(f) {
  return dynList(state[f.k], "+ entry", () => ["", "#88ccff"],
    (row, i, rerender) => {
      const r = document.createElement("div"); r.className="row"; r.style.marginBottom="5px";
      const name = document.createElement("input"); name.type="text"; name.placeholder="name"; name.value=row[0]; name.oninput=()=>{ row[0]=name.value; onChange(); };
      const col = document.createElement("input"); col.type="color"; col.value=row[1]; col.style.flex="0 0 40px"; col.oninput=()=>{ row[1]=col.value; onChange(); };
      const rm = document.createElement("button"); rm.className="rm"; rm.textContent="✕"; rm.onclick=()=>{ state[f.k].splice(i,1); rerender(); onChange(); };
      r.append(name, col, rm); return r;
    });
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

function fieldText(f) {
  let t = (f.label || "") + " " + f.k;
  if (f.fields) for (const s of f.fields) t += " " + (s.label || "") + " " + s.k;
  return t.toLowerCase();
}
function buildForm() {
  const root = $("form");
  root.innerHTML = "";
  conds.length = 0; searchItems.length = 0; searchSections.length = 0;
  for (const sec of SCHEMA) {
    const d = document.createElement("details"); if (sec.open) d.open = true;
    const sum = document.createElement("summary"); sum.textContent = sec.s; d.append(sum);
    const body = document.createElement("div"); body.className = "body";
    for (const f of sec.fields) {
      let node;
      if (f.t === "grp") node = groupNode(f);
      else if (f.t === "lights") node = labelWrap(f, listNode(f));
      else if (f.t === "palette") node = labelWrap(f, paletteNode(f));
      else if (f.t === "map") node = labelWrap(f, mapNode(f));
      else if (f.t === "views") node = labelWrap(f, viewsNode(f));
      else node = ctl(f, state, state);
      body.append(node);
      searchItems.push({ node, section: d, text: fieldText(f) });
    }
    d.append(body); root.append(d);
    searchSections.push({ el: d, open: !!sec.open });
  }
}

// Filter the form by a query — hides non-matching fields/sections, expands
// sections that contain a match. Composes with when-visibility (CSS !important).
function filterForm(query) {
  const q = (query || "").trim().toLowerCase();
  for (const it of searchItems) it.node.classList.toggle("search-hidden", !!q && !it.text.includes(q));
  for (const sec of searchSections) {
    const hit = searchItems.some(it => it.section === sec.el && !it.node.classList.contains("search-hidden"));
    sec.el.classList.toggle("search-hidden", !!q && !hit);
    sec.el.open = q ? hit : sec.open;
  }
}

// Rebuild the whole form from current state (used by reset & shared-link restore).
function rebuildForm() {
  buildForm(); refreshVisibility(); filterForm($("search").value);
}
function labelWrap(f, node) {
  const w = document.createElement("div"); w.className = "ctl";
  if (f.label) { const l = document.createElement("label"); l.innerHTML = `<span>${f.label}</span>`; const tip = f.help || HELP[f.k]; if (tip) l.title = tip; w.append(l); }
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
let outputFormat = "png";   // "png" | "svg" — chosen via the stage toolbar toggle
const RFN_PNG = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
const RFN_SVG = { obj: "render_obj", stl: "render_stl", ply: "render_ply" };
function render() {
  if (!instance || !model.bytes) return;
  const fn = (outputFormat === "svg" ? RFN_SVG : RFN_PNG)[ext(model.name)];
  if (!fn) return showErr(`unsupported file type: .${ext(model.name)}`);
  try {
    const t0 = performance.now();
    const out = callPlugin(fn, model.bytes, ENC.encode(JSON.stringify(buildConfig())));
    const ms = performance.now() - t0;
    // grid / turntable / debug / annotations wrap the render as SVG (text overlays);
    // everything else is a PNG. Sniff the magic and set the blob type accordingly.
    const isPng = out[0] === 0x89 && out[1] === 0x50;
    lastRender = { bytes: out, type: isPng ? "image/png" : "image/svg+xml", ext: isPng ? "png" : "svg" };
    const url = URL.createObjectURL(new Blob([out], { type: lastRender.type }));
    elOut.src = url; elOut.style.display = "";
    if (lastUrl) URL.revokeObjectURL(lastUrl); lastUrl = url;
    elRtime.textContent = `rendered in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`; elRtime.classList.add("show");
    showErr(null);
  } catch (e) { showErr(e.message); }
}
function showErr(m) { elErr.style.display = m ? "" : "none"; elErr.textContent = m || ""; }

// Clipboard with a fallback for non-secure contexts (file://, LAN IP) where
// navigator.clipboard is unavailable.
async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* fall through */ }
  try {
    const ta = document.createElement("textarea");
    ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
    document.body.append(ta); ta.select();
    const ok = document.execCommand("copy"); ta.remove(); return ok;
  } catch { return false; }
}

// ─────────────────────────────── model I/O ────────────────────────────────
async function loadFile(file) {
  model = { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) };
  $("fname").textContent = model.name;
  measure(); onChange();
}
$("browse").onclick = () => $("file").click();
$("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);
$("copy").onclick = async () => { await copyText(buildTypst()); const o = $("copy").textContent; $("copy").textContent = "Copied!"; setTimeout(() => ($("copy").textContent = o), 1200); };

// ── measurements (model-intrinsic; get-*-info) ─────────────────────────────
const INFO_FN = { obj: "get_obj_info", stl: "get_stl_info", ply: "get_ply_info" };
function measure() {
  elMeasure.innerHTML = "";
  if (!instance || !model.bytes) return;
  const fn = INFO_FN[ext(model.name)];
  if (!fn) return;
  try {
    const info = JSON.parse(DEC.decode(callPlugin(fn, model.bytes, ENC.encode("{}"))));
    const n = (x) => Number.isInteger(x) ? x.toLocaleString() : (+x).toPrecision(3);
    const stats = [];
    if (info.triangles != null) stats.push(["tris", n(info.triangles)]);
    if (info.vertices != null) stats.push(["verts", n(info.vertices)]);
    if (info.size) stats.push(["size", info.size.map(x => (+x).toPrecision(3)).join(" × ")]);
    if (info.surface_area != null) stats.push(["area", (+info.surface_area).toPrecision(3)]);
    if (info.volume != null) stats.push(["volume", (+info.volume).toPrecision(3)]);
    if (info.bbox_radius != null) stats.push(["radius", (+info.bbox_radius).toPrecision(3)]);
    elMeasure.innerHTML = stats.map(([k, v]) => `<span class="stat">${k} <b>${v}</b></span>`).join("");
  } catch { /* info unavailable for this model — leave blank */ }
}

// ── drag-to-orbit + scroll-to-zoom ─────────────────────────────────────────
// rAF-throttled update for the interactive (orbit/zoom) hot path — coalesces the
// code re-highlight and the WASM render to one per frame instead of per event.
function scheduleRender() {
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(() => { rafPending = false; renderCode(); render(); });
}
function ensureSpherical() {
  if (state._cam !== "spherical") { state._cam = "spherical"; controlRefs._cam?.("spherical"); refreshVisibility(); }
}
(function setupOrbit() {
  const stage = $("stage");
  let dragging = false, lx = 0, ly = 0;
  stage.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || e.target.closest("#tools")) return;   // ignore toolbar clicks
    dragging = true; lx = e.clientX; ly = e.clientY;
    try { stage.setPointerCapture(e.pointerId); } catch {}
    stage.classList.add("grabbing"); ensureSpherical();
  });
  stage.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    const dx = e.clientX - lx, dy = e.clientY - ly; lx = e.clientX; ly = e.clientY;
    state.azimuth = Math.round((state.azimuth - dx * 0.5) * 10) / 10;
    state.elevation = Math.max(-89, Math.min(89, Math.round((state.elevation + dy * 0.5) * 10) / 10));
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation);
    scheduleRender();
  });
  const end = () => { if (dragging) { dragging = false; stage.classList.remove("grabbing"); renderCode(); render(); } };
  stage.addEventListener("pointerup", end);
  stage.addEventListener("pointercancel", end);
  stage.addEventListener("wheel", (e) => {
    e.preventDefault();
    const f = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    state.zoom = Math.max(0.3, Math.min(4, Math.round(state.zoom * f * 1000) / 1000));
    controlRefs.zoom?.(state.zoom); scheduleRender();
  }, { passive: false });
})();

// ── download / share / reset ───────────────────────────────────────────────
$("btn-download").onclick = () => {
  if (!lastRender) return;
  const a = document.createElement("a");
  a.href = URL.createObjectURL(new Blob([lastRender.bytes], { type: lastRender.type }));
  a.download = model.name.replace(/\.[^.]+$/, "") + "." + lastRender.ext;
  a.click(); URL.revokeObjectURL(a.href);
};

// Encode the config (diff vs defaults) into the URL hash — shareable & compact.
function encodeState() {
  const init = initState(), diff = {};
  for (const k in state) if (!eq(state[k], init[k])) diff[k] = state[k];
  return btoa(unescape(encodeURIComponent(JSON.stringify(diff)))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
function applyStateFromHash() {
  const m = location.hash.match(/cfg=([^&]+)/);
  if (!m) return;
  try {
    const b64 = m[1].replace(/-/g, "+").replace(/_/g, "/");
    Object.assign(state, structuredClone(JSON.parse(decodeURIComponent(escape(atob(b64))))));
  } catch (e) { console.warn("ignoring malformed config link", e); }
}
$("btn-share").onclick = async () => {
  location.hash = "cfg=" + encodeState();
  const ok = await copyText(location.href);
  const b = $("btn-share"), o = b.textContent; b.textContent = ok ? "Copied!" : "Link in URL"; setTimeout(() => (b.textContent = o), 1400);
};

$("btn-reset").onclick = () => {
  const init = initState();
  for (const k in state) delete state[k];
  Object.assign(state, init);
  history.replaceState(null, "", location.pathname + location.search);
  $("search").value = "";
  rebuildForm(); measure(); onChange();
};

$("search").addEventListener("input", () => filterForm($("search").value));

// PNG / SVG output toggle
document.querySelectorAll("#fmt button").forEach((b) => {
  b.onclick = () => {
    outputFormat = b.dataset.fmt;
    document.querySelectorAll("#fmt button").forEach((x) => x.classList.toggle("on", x === b));
    onChange();
  };
});

["dragenter","dragover"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); $("stage").classList.add("drag"); }));
["dragleave","drop"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); if (ev==="dragleave" && e.relatedTarget) return; $("stage").classList.remove("drag"); }));
document.addEventListener("drop", e => { const f = e.dataTransfer?.files?.[0]; if (f) loadFile(f); });

// ─────────────────────────── wasm module cache (IndexedDB) ──────────────────
// Repeat visits skip both the download and the compile: we persist the compiled
// WebAssembly.Module in IndexedDB, keyed by the file's ETag/Last-Modified. A
// cheap HEAD request tells us whether the cached module is still current, so a
// CI redeploy of a new wasm invalidates the cache automatically.
const IDB = { name: "maquette-cache", store: "modules", key: "maquette.wasm" };
function idbOpen() {
  return new Promise((res, rej) => {
    const r = indexedDB.open(IDB.name, 1);
    r.onupgradeneeded = () => r.result.createObjectStore(IDB.store);
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function idbGet(key) {
  try {
    const db = await idbOpen();
    return await new Promise((res, rej) => {
      const q = db.transaction(IDB.store, "readonly").objectStore(IDB.store).get(key);
      q.onsuccess = () => res(q.result);
      q.onerror = () => rej(q.error);
    });
  } catch { return undefined; }
}
async function idbPut(key, val) {
  try {
    const db = await idbOpen();
    await new Promise((res, rej) => {
      const q = db.transaction(IDB.store, "readwrite").objectStore(IDB.store).put(val, key);
      q.onsuccess = () => res();
      q.onerror = () => rej(q.error);
    });
  } catch { /* private mode, or a browser that won't structured-clone Module: skip caching */ }
}

async function compileModule() {
  // Streaming compile overlaps download with compilation. Fall back to a plain
  // compile if the response isn't served as application/wasm (some hosts).
  try { return await WebAssembly.compileStreaming(fetch(WASM_URL)); }
  catch { return await WebAssembly.compile(await (await fetch(WASM_URL)).arrayBuffer()); }
}

async function loadModule() {
  let tag = null;
  try {
    const h = await fetch(WASM_URL, { method: "HEAD" });
    tag = h.headers.get("etag") || h.headers.get("last-modified");
  } catch { /* no freshness signal → compile fresh, don't cache */ }

  if (tag) {
    const hit = await idbGet(IDB.key);
    if (hit && hit.tag === tag && hit.module instanceof WebAssembly.Module) return hit.module;
  }
  const module = await compileModule();
  if (tag) idbPut(IDB.key, { tag, module });
  return module;
}

// ─────────────────────────────────── boot ─────────────────────────────────
(async function boot() {
  applyStateFromHash();          // restore a shared config before building the form
  buildForm(); refreshVisibility();
  try {
    const module = await loadModule();
    // With a Module (not bytes), instantiate() resolves to the Instance directly.
    instance = await WebAssembly.instantiate(module, importObject);
    memory = instance.exports.memory;
    model = { name: "bunny.obj", bytes: new Uint8Array(await (await fetch("bunny.obj")).arrayBuffer()) };
    measure(); onChange();
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
})();

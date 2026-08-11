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
  { s: "Point cloud (PLY)", open: true, when: () => ext(model.name) === "ply", fields: [
    { k: "point_size", label: "Point size / radius (0 = auto)", t: "num", def: 0, omitIf: v => v === 0 },
    { k: "point_neighbors", label: "Neighbors k (higher = fewer holes)", t: "num", def: 12, omitIf: v => v === 12 },
    { k: "point_boundary", label: "Boundary cut angle ° (0 = off)", t: "num", def: 60, omitIf: v => v === 60 },
  ]},
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
        source: "camera", plane: [0, 0, 1, 0], depth: 0.5, keep: "far", cap: true,
        hatch: false, hstyle: "lines", hangle: 45, hspacing: 6, hwidth: 0.6, hcolor: "#333333" },
      build: "clip", fields: [
        { k: "source", label: "From", t: "sel", def: "camera", opts: [["camera","Camera"],["x","X axis"],["y","Y axis"],["z","Z axis"],["plane","Plane (a,b,c,d)"]] },
        { k: "plane", label: "Plane a, b, c, d", t: "vec", def: [0,0,1,0], when: (s,l) => l.source === "plane" },
        { k: "depth", label: "Depth", t: "rng", def: 0.5, min: 0, max: 1, step: 0.01, when: (s,l) => l.source !== "plane" },
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
    { k: "highlight", label: "Highlight (group → appearance)", t: "map", rich: true, def: [] },
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
  decimate: "Simplify the mesh (higher = fewer triangles).", point_size: "Neighbor radius for PLY point clouds.", point_neighbors: "PLY clouds: neighbors per point (higher = fewer holes, slower).", point_boundary: "PLY clouds: cut connections across a normal jump > this angle\u00b0 (0 = keep all).",
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
let lastRender = null;    // {kind:"raw"} or {kind:"svg", bytes} of the most recent render
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
    const o = {};
    if (s.source === "plane") o.plane = s.plane.slice();
    else { o.depth = s.depth; if (s.source === "camera") o.from = "camera"; else o.axis = s.source; }
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
// Exact config from a deep-link, kept as a render override so features the UI
// can't fully represent (clip plane equation, per-group highlight stroke/opacity)
// still render pixel-exact. Wins over the state-derived config per top-level key;
// cleared the moment the user edits anything (then it's a live, state-driven config).
let renderOverride = null;
function renderConfig() {
  const c = buildConfig();
  return renderOverride ? { ...c, ...renderOverride } : c;
}
// Rich highlight per-group appearance ↔ demo state. Normalize a loaded value
// (plain "#color" or {color, stroke, stroke_width, opacity}) to a full object for
// editing; collapse back to the minimal form (a bare color string when that's all).
const hlNormalize = (cv) => {
  const a = { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 };
  if (typeof cv === "string") a.color = cv;
  else if (cv && typeof cv === "object") {
    if (cv.color) a.color = cv.color;
    if (cv.stroke) a.stroke = cv.stroke;
    if (cv.stroke_width != null) a.stroke_width = cv.stroke_width;
    if (cv.opacity != null) a.opacity = cv.opacity;
  }
  return a;
};
const hlCollapse = (v) => {
  if (typeof v === "string") return v;
  const o = { color: v.color };
  if (v.stroke) o.stroke = v.stroke;
  if (v.stroke_width) o.stroke_width = v.stroke_width;
  if (v.opacity != null && v.opacity !== 1) o.opacity = v.opacity;
  return Object.keys(o).length === 1 ? o.color : o;   // only color → plain string
};
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
      case "map": if (state[f.k].length) c[f.k] = Object.fromEntries(state[f.k].filter(r => r[0]).map(([n, v]) => [n, f.rich ? hlCollapse(v) : v])); break;
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
      case "map": { const rows = state[f.k].filter(r => r[0]); if (rows.length) push(f.k, `(${rows.map(([n,v]) => `"${n}": ${fmtT(f.rich ? hlCollapse(v) : v)}`).join(", ")})`); break; }
      default: if (!eq(state[f.k], f.def)) push(f.k, fmtT(state[f.k]));
    }
  }
  if (outputFormat === "svg") P.push('format: "svg"');
  const body = P.length ? `#${fn}(model,\n  ${P.join(",\n  ")},\n)` : `#${fn}(model)`;
  return `#import "@preview/maquette:0.1.3": ${fn}\n\n#let model = read("${model.name}", encoding: none)\n\n${body}`;
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
const elCode = $("code"), elErr = $("err"), elOut = $("out"), elOutc = $("outc"), elRtime = $("rtime"), elMeasure = $("measure");

// Build stamp: CI replaces __BUILD_COMMIT__ in index.html with the deployed short
// SHA. Hover shows it (desktop); tapping the pill reveals it (mobile). Locally the
// placeholder is untouched, so it reads "dev".
(function () {
  const el = $("build"); if (!el) return;
  const c = el.dataset.commit;
  const v = c && !c.startsWith("__") ? c : "dev";
  el.title = "deployed build · " + v;
  el.style.cursor = "pointer";
  el.onclick = () => { const o = el.textContent; el.textContent = v; setTimeout(() => (el.textContent = o), 1600); };
})();

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
  attachTip(labelEl, f.help || HELP[f.k]);
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
  attachTip(head, f.help || HELP[f.k]);
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
      const vec = document.createElement("div"); // per-axis slider + precise number box
      ["X","Y","Z"].forEach((axis, j) => {
        const row = document.createElement("div"); row.style.cssText = "display:flex;gap:6px;align-items:center;margin:2px 0";
        const tag = document.createElement("span"); tag.textContent = axis; tag.style.cssText = "width:1em;color:var(--muted);font-size:12px";
        const rng = document.createElement("input"); rng.type="range"; rng.min="-3"; rng.max="3"; rng.step="0.05"; rng.value=L.vector[j]; rng.style.flex="1";
        const ni = document.createElement("input"); ni.type="number"; ni.step="any"; ni.value=L.vector[j]; ni.style.width="4.5em";
        rng.oninput=()=>{ L.vector[j]=+rng.value; ni.value=rng.value; onChange(); };
        ni.oninput =()=>{ L.vector[j]=+ni.value; if (Math.abs(+ni.value)<=3) rng.value=ni.value; onChange(); };
        row.append(tag, rng, ni); vec.append(row);
      });
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
  const rm = (i, rerender) => { const b = document.createElement("button"); b.className="rm"; b.textContent="✕"; b.title="remove"; b.onclick=()=>{ state[f.k].splice(i,1); rerender(); onChange(); }; return b; };
  if (f.rich) return dynList(state[f.k], "+ entry",       // group → {color, stroke, stroke_width, opacity}
    () => ["", { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 }],
    (row, i, rerender) => {
      const v = row[1];
      const mk = (label, el) => { const d = document.createElement("div"); d.className="ctl"; const l = document.createElement("label"); l.innerHTML = `<span>${label}</span>`; d.append(l, el); return d; };
      const inp = (type, val, on, extra) => { const e = document.createElement("input"); e.type = type; e.value = val; if (extra) for (const a in extra) e.setAttribute(a, extra[a]); e.oninput = () => on(e.value); return e; };
      const name = inp("text", row[0], x => { row[0] = x; onChange(); }, { placeholder: "group name" });
      const color = inp("color", v.color, x => { v.color = x; onChange(); }); color.style.flex = "0 0 40px";
      const top = document.createElement("div"); top.className = "row"; top.style.marginBottom = "5px"; top.append(name, color, rm(i, rerender));
      const it = document.createElement("div"); it.className = "item";
      it.append(top,
        mk("Stroke (blank = none)", inp("text", v.stroke, x => { v.stroke = x; onChange(); }, { placeholder: "#ffffff" })),
        mk("Stroke width", inp("number", v.stroke_width, x => { v.stroke_width = x === "" ? 0 : +x; onChange(); }, { step: "any" })),
        mk("Opacity", inp("number", v.opacity, x => { v.opacity = x === "" ? 1 : +x; onChange(); }, { step: "0.05", min: "0", max: "1" })));
      return it;
    });
  return dynList(state[f.k], "+ entry", () => ["", "#88ccff"],
    (row, i, rerender) => {
      const r = document.createElement("div"); r.className="row"; r.style.marginBottom="5px";
      const name = document.createElement("input"); name.type="text"; name.placeholder="name"; name.value=row[0]; name.oninput=()=>{ row[0]=name.value; onChange(); };
      const col = document.createElement("input"); col.type="color"; col.value=row[1]; col.style.flex="0 0 40px"; col.oninput=()=>{ row[1]=col.value; onChange(); };
      r.append(name, col, rm(i, rerender)); return r;
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
    if (sec.when) conds.push({ node: d, when: sec.when, local: state });
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
  if (f.label) { const l = document.createElement("label"); l.innerHTML = `<span>${f.label}</span>`; attachTip(l, f.help || HELP[f.k]); w.append(l); }
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
let renderToken = 0;   // invalidates a pending async overlay draw when a newer render starts
let outputFormat = "png";   // "png" | "svg" — chosen via the stage toolbar toggle
const RFN_PNG = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
const RFN_SVG = { obj: "render_obj", stl: "render_stl", ply: "render_ply" };
function render() {
  if (!instance || !model.bytes) return;
  const fn = (outputFormat === "svg" ? RFN_SVG : RFN_PNG)[ext(model.name)];
  if (!fn) return showErr(`unsupported file type: .${ext(model.name)}`);
  try {
    const token = ++renderToken;
    const t0 = performance.now();
    const out = callPlugin(fn, model.bytes, ENC.encode(JSON.stringify(renderConfig())));
    const ms = performance.now() - t0;
    // Raster output is raw RGBA ([0x00][w][h][rgba8…]); grid / turntable / debug /
    // annotations add a transparent vector overlay ([0x02][w][h][rgba8 w*h*4][svg…]).
    // Vector mode returns SVG (0x3C).
    if (out[0] === 0x00 || out[0] === 0x02) {
      // Blit the pixels straight to the canvas — no PNG encode (plugin) or decode.
      const w = out[1] | out[2] << 8 | out[3] << 16 | out[4] << 24;
      const h = out[5] | out[6] << 8 | out[7] << 16 | out[8] << 24;
      const n = w * h * 4;
      const px = new Uint8ClampedArray(out.buffer, out.byteOffset + 9, n);
      elOutc.width = w; elOutc.height = h;
      const ctx = elOutc.getContext("2d");
      ctx.putImageData(new ImageData(px, w, h), 0, 0);
      elOutc.style.display = ""; elOut.style.display = "none";
      lastRender = { kind: "raw" };
      if (out[0] === 0x02) {
        // Layer the transparent SVG overlay onto the same canvas (labels, grid
        // lines, annotations, debug text). Async; guarded against a newer render.
        const url = URL.createObjectURL(new Blob([out.subarray(9 + n)], { type: "image/svg+xml" }));
        const svgImg = new Image();
        svgImg.onload = () => { if (token === renderToken) ctx.drawImage(svgImg, 0, 0, w, h); URL.revokeObjectURL(url); };
        svgImg.src = url;
      }
    } else {
      const url = URL.createObjectURL(new Blob([out], { type: "image/svg+xml" }));
      elOut.src = url; elOut.style.display = ""; elOutc.style.display = "none";
      if (lastUrl) URL.revokeObjectURL(lastUrl); lastUrl = url;
      lastRender = { kind: "svg", bytes: out };
    }
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
// Built-in models — fetched lazily (only when picked), same origin as bunny.obj.
const MODELS = [
  ["bunny.obj", "Bunny (OBJ)"],
  ["teapot.obj", "Teapot (OBJ)"],
  ["crankshaft.obj", "Crankshaft (OBJ)"],
  ["brain_skull.obj", "Brain + skull (OBJ)"],
  ["rubi_scan.ply", "Rubik scan (PLY point cloud)"],
];

// Per-model "showcase" config, applied when a built-in model is picked so each
// loads looking like its documentation example. On every preset load these keys
// are first reset to their schema defaults, then the picked model's overrides
// applied — so switching models never leaves another model's styling behind.
const MODEL_DEFAULTS = {
  // White skull with a pink brain revealed through x-ray (see documentation.pdf).
  "brain_skull.obj": {
    highlight: [["Skull", "#e8e8e8"], ["Brain", "#ee69b4"]],
    mode: "x-ray", xray_opacity: 0.3,
  },
  // Brushed-steel grey; crankshaft is modelled -Y up (as the documentation uses).
  "crankshaft.obj": {
    color: "#7d8590", specular: 0.6, shininess: 48, up: [0, -1, 0],
  },
  // Point-cloud reconstruction framed like the documentation's Rubik scan.
  "rubi_scan.ply": {
    point_size: 0.024, point_boundary: 110,
    _cam: "spherical", azimuth: -119, elevation: 24.5, up: [0, 1, 0], zoom: 2.254,
  },
};
const DEFAULTS_KEYS = [...new Set(Object.values(MODEL_DEFAULTS).flatMap(Object.keys))];
const TOP_FIELDS = Object.fromEntries(SCHEMA.flatMap(s => s.fields.map(f => [f.k, f])));
function applyModelDefaults(name) {
  for (const k of DEFAULTS_KEYS) {   // reset to the field's starting value (init, else def)
    const f = TOP_FIELDS[k];
    state[k] = structuredClone(f && f.init !== undefined ? f.init : f && f.def);
  }
  const ov = MODEL_DEFAULTS[name];
  if (ov) for (const k in ov) state[k] = structuredClone(ov[k]);
}

// Shared ingestion for both dropped/browsed files and built-in presets.
function ingest(name, bytes) {
  model = { name, bytes };
  syncPreset(name);
  renderOverride = null;
  if (location.search || location.hash) history.replaceState(null, "", location.pathname);  // drop a stale share link
  refreshVisibility(); measure(); onChange();
}
async function loadFile(file) {
  ingest(file.name, new Uint8Array(await file.arrayBuffer()));
}
async function loadPreset(name) {
  try {
    const bytes = new Uint8Array(await (await fetch(name)).arrayBuffer());
    applyModelDefaults(name);
    buildForm(); refreshVisibility();   // reflect the new state in the form controls
    ingest(name, bytes);
  } catch (e) { showErr("failed to load " + name + ": " + e.message); }
}
// Reflect the active model in the dropdown; a dropped file gets a transient entry.
function syncPreset(name) {
  const sel = $("preset");
  if (MODELS.some(([v]) => v === name)) { sel.value = name; return; }
  let custom = sel.querySelector("option[data-custom]");
  if (!custom) { custom = document.createElement("option"); custom.dataset.custom = "1"; sel.append(custom); }
  custom.value = name; custom.textContent = name + " (loaded)"; sel.value = name;
}
(function initPresets() {
  const sel = $("preset");
  for (const [v, t] of MODELS) { const o = document.createElement("option"); o.value = v; o.textContent = t; sel.append(o); }
  sel.onchange = () => { if (sel.value) loadPreset(sel.value); };
})();
$("browse").onclick = () => $("file").click();
$("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);

// ─────────────────────── hover descriptions (tooltips) ─────────────────────
// Tag an element with its help text; a single delegated listener shows an
// immediate, styled tooltip on hover (native `title` is too slow/subtle).
function attachTip(el, tip) { if (!tip) return; el.dataset.tip = tip; el.classList.add("has-tip"); }
const tipEl = $("tip");
function positionTip(target) {
  const r = target.getBoundingClientRect(), m = 8;
  const tw = tipEl.offsetWidth, th = tipEl.offsetHeight;
  let x = Math.min(r.left, innerWidth - tw - m);
  let y = r.bottom + 6;
  if (y + th > innerHeight - m) y = r.top - th - 6;   // flip above when no room below
  tipEl.style.left = Math.max(m, x) + "px";
  tipEl.style.top = Math.max(m, y) + "px";
}
document.addEventListener("mouseover", (e) => {
  const el = e.target.closest && e.target.closest("[data-tip]");
  if (!el) return;
  tipEl.textContent = el.dataset.tip; tipEl.classList.add("show"); positionTip(el);
});
document.addEventListener("mouseout", (e) => {
  const el = e.target.closest && e.target.closest("[data-tip]");
  if (el && !el.contains(e.relatedTarget)) tipEl.classList.remove("show");
});
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
    if (Array.isArray(info.bbox_center)) bboxCenter = info.bbox_center;   // for cartesian→spherical orbit
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
let bboxCenter = [0, 0, 0];   // model bbox center (from get-info), for cartesian→spherical
// Invert the plugin's up-based spherical basis (projection.rs) so an explicit
// cartesian camera maps to the azimuth/elevation/distance that reproduce its view.
function cartesianToSpherical(cam, center, up) {
  const sub = (a, b) => [a[0]-b[0], a[1]-b[1], a[2]-b[2]];
  const dot = (a, b) => a[0]*b[0] + a[1]*b[1] + a[2]*b[2];
  const cross = (a, b) => [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
  const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0]/l, a[1]/l, a[2]/l]; };
  const off = sub(cam, center), dist = Math.hypot(off[0], off[1], off[2]);
  if (dist < 1e-6) return null;
  const v = norm(off), u = norm(up);
  const arbitrary = Math.abs(u[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const right = norm(cross(u, arbitrary));
  const forward = norm(cross(right, u));
  const el = Math.asin(Math.max(-1, Math.min(1, dot(v, u))));
  const az = Math.atan2(dot(v, forward), dot(v, right));
  return { azimuth: az * 180 / Math.PI, elevation: el * 180 / Math.PI, distance: dist };
}
// Switch to orbit (spherical) mode. Clears any deep-link render override so the
// view stops being frozen, and converts an explicit cartesian camera to
// azimuth/elevation/distance so orbiting continues from exactly where it was.
function ensureSpherical() {
  renderOverride = null;
  if (state._cam === "spherical") return;
  const center = eq(state.center, [0, 0, 0]) ? bboxCenter : state.center;   // auto_center → bbox center
  const sph = cartesianToSpherical(state.camera, center, state.up);
  if (sph) {
    state.azimuth = Math.round(sph.azimuth * 10) / 10;
    state.elevation = Math.max(-89, Math.min(89, Math.round(sph.elevation * 10) / 10));
    state.distance = Math.round(sph.distance * 1000) / 1000;
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.distance?.(state.distance);
  }
  state._cam = "spherical"; controlRefs._cam?.("spherical"); refreshVisibility();
}
(function setupOrbit() {
  const stage = $("stage");
  const pts = new Map();          // active pointers: id → {x, y}
  let pd = 0, pcx = 0, pcy = 0;   // last two-finger distance + centroid while pinching
  const two = () => [...pts.values()];
  const twoDist = () => { const [a, b] = two(); return Math.hypot(a.x - b.x, a.y - b.y); };
  const twoCent = () => { const [a, b] = two(); return [(a.x + b.x) / 2, (a.y + b.y) / 2]; };
  const seedPinch = () => { pd = twoDist(); [pcx, pcy] = twoCent(); };
  const setZoom = (f) => {
    renderOverride = null;                       // zooming unfreezes a deep-link render
    state.zoom = Math.max(0.3, Math.min(4, Math.round(state.zoom * f * 1000) / 1000));
    controlRefs.zoom?.(state.zoom);
  };
  const orbitBy = (dx, dy, ptype) => {
    // Touch reads as direct manipulation, so its horizontal orbit is inverted vs a
    // mouse orbit; a mouse keeps the -1 direction.
    const xdir = ptype === "mouse" ? -1 : 1;
    state.azimuth = Math.round((state.azimuth + xdir * dx * 0.5) * 10) / 10;
    state.elevation = Math.max(-89, Math.min(89, Math.round((state.elevation + dy * 0.5) * 10) / 10));
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation);
  };

  stage.addEventListener("pointerdown", (e) => {
    if (e.target.closest("#tools")) return;                 // ignore toolbar clicks
    if (e.pointerType === "mouse" && e.button !== 0) return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    try { stage.setPointerCapture(e.pointerId); } catch {}
    stage.classList.add("grabbing"); ensureSpherical();
    if (pts.size === 2) seedPinch();                        // enter pinch
  });

  stage.addEventListener("pointermove", (e) => {
    const p = pts.get(e.pointerId);
    if (!p) return;
    if (pts.size >= 2) {                                    // two fingers → orbit + zoom together
      p.x = e.clientX; p.y = e.clientY;
      const d = twoDist(), [cx, cy] = twoCent();
      if (pd > 0 && d > 0) setZoom(d / pd);                 // pinch → zoom
      orbitBy(cx - pcx, cy - pcy, e.pointerType);           // drag centroid → orbit
      pd = d; pcx = cx; pcy = cy;
    } else {                                                // one pointer → orbit
      orbitBy(e.clientX - p.x, e.clientY - p.y, e.pointerType);
      p.x = e.clientX; p.y = e.clientY;
    }
    scheduleRender();
  });

  const end = (e) => {
    if (!pts.delete(e.pointerId)) return;
    pd = 0;                                                 // re-seed below if a pinch continues
    if (pts.size === 0) { stage.classList.remove("grabbing"); renderCode(); render(); }
    else if (pts.size === 2) seedPinch();
  };
  stage.addEventListener("pointerup", end);
  stage.addEventListener("pointercancel", end);

  stage.addEventListener("wheel", (e) => {
    e.preventDefault();
    setZoom(e.deltaY < 0 ? 1.1 : 1 / 1.1);
    scheduleRender();
  }, { passive: false });
})();

// ── download / share / reset ───────────────────────────────────────────────
$("btn-download").onclick = () => {
  if (!lastRender) return;
  const base = model.name.replace(/\.[^.]+$/, "");
  const save = (blob, ext) => {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = base + "." + ext;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  };
  if (lastRender.kind === "raw") {
    // The plugin no longer emits PNG; encode it here (browser-native) on demand.
    elOutc.toBlob((blob) => save(blob, "png"), "image/png");
  } else {
    save(new Blob([lastRender.bytes], { type: "image/svg+xml" }), "svg");
  }
};

// ── Shareable links ────────────────────────────────────────────────────────
// A link carries the model + config diff. We emit whichever encoding is shorter:
// a readable ?code=value… form (field names aliased to short codes, minimal
// percent-encoding — same scheme the Typst docs emit) or a compact
// ?_=1<deflate+base64url> blob (best for large configs). Both are decoded on load,
// and after any link loads the address bar is shortened to the best form.
// Old full-name ?model=…&field=… links and legacy #cfg= blobs still decode.
const bytesToB64u = (b) => { let s = ""; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]); return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); };
const b64uToBytes = (s) => { const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/")); const a = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i); return a; };
const hasCompression = typeof CompressionStream !== "undefined" && typeof DecompressionStream !== "undefined";
// Top-level config keys → short letter codes before deflate, shaving the
// field-name bytes deflate can't fully squeeze. APPEND-ONLY (never reorder/remove)
// so old links keep decoding; unlisted keys pass through unshortened. Only applied
// to top-level keys — map values (highlight/materials group names) are left alone.
const FIELD_CODES = ["model", "camera", "azimuth", "elevation", "distance", "center", "up", "projection",
  "fov", "zoom", "pan", "auto_center", "auto_fit", "background", "width", "height", "color", "opacity",
  "specular", "shininess", "smooth", "gamma_correction", "cull_backface", "shading", "gooch_warm",
  "gooch_cool", "cel_bands", "mode", "xray_opacity", "stroke", "wireframe", "light_dir", "ambient",
  "fresnel", "tone_mapping", "sss", "lights", "color_map", "overhang_angle", "scalar_function",
  "vertex_smoothing", "color_map_palette", "outline", "ground_shadow", "shadows", "antialias", "ssao",
  "bloom", "glow", "sharpen", "clip", "explode", "decimate", "views", "grid_labels", "turntable",
  "materials", "highlight", "annotations", "debug", "debug_color", "point_size", "point_neighbors",
  "point_boundary", "_cam", "_hemi", "_bgNone"];
// Letters-only codes so no code is an integer-like key (which JS would reorder).
const CODE_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const codeFor = (i) => i < 52 ? CODE_ALPHABET[i] : CODE_ALPHABET[((i - 52) / 52) | 0] + CODE_ALPHABET[(i - 52) % 52];
const KEY_ALIAS = {}, KEY_UNALIAS = {};
FIELD_CODES.forEach((k, i) => { const c = codeFor(i); KEY_ALIAS[k] = c; KEY_UNALIAS[c] = k; });
const aliasKeys = (o) => { const r = {}; for (const k in o) r[KEY_ALIAS[k] ?? k] = o[k]; return r; };
const unaliasKeys = (o) => { const r = {}; for (const k in o) r[KEY_UNALIAS[k] ?? k] = o[k]; return r; };
// Query chars safe to leave literal (shorter than %XX). URLSearchParams still
// parses these; only & = + # % space and non-ASCII get encoded. Must match the
// Typst documentation-link generator so both produce identical links.
const B_SAFE = new Set([..."ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~,:!$*;@/?[]{}\"()"].map((c) => c.charCodeAt(0)));
const pctB = (s) => [...s].map((ch) => B_SAFE.has(ch.charCodeAt(0)) ? ch : encodeURIComponent(ch)).join("");
async function deflate(str) {
  const cs = new CompressionStream("deflate-raw"), w = cs.writable.getWriter();
  w.write(ENC.encode(str)); w.close();
  return new Uint8Array(await new Response(cs.readable).arrayBuffer());
}
async function inflate(bytes) {
  const ds = new DecompressionStream("deflate-raw"), w = ds.writable.getWriter();
  w.write(bytes); w.close();
  return DEC.decode(await new Response(ds.readable).arrayBuffer());
}

function shareConfig() {                       // model + config diff vs defaults
  const init = initState(), cfg = { model: model.name };
  for (const k in state) if (!eq(state[k], init[k])) cfg[k] = state[k];
  return cfg;
}
const baseUrl = () => location.origin + location.pathname;
function readableUrl(cfg) {           // ?code=value… — aliased keys, minimal encoding
  const parts = [];
  for (const [k, v] of Object.entries(aliasKeys(cfg))) parts.push(k + "=" + pctB(typeof v === "string" ? v : JSON.stringify(v)));
  return baseUrl() + "?" + parts.join("&");
}
async function compactUrl(cfg) {      // ?_=1<deflate+base64url> ("_" is never a field code)
  if (!hasCompression) return null;
  try { return baseUrl() + "?_=1" + bytesToB64u(await deflate(JSON.stringify(aliasKeys(cfg)))); }
  catch { return null; }
}
async function bestUrl(cfg) {                  // shortest of readable / compact
  const r = readableUrl(cfg), c = await compactUrl(cfg);
  return c && c.length < r.length ? c : r;
}

// Decode a link into state. Returns the model to load, whether any config was
// present, and the raw config (for the exact-render override + address shortening).
async function applyStateFromUrl() {
  const p = new URLSearchParams(location.search);
  let raw = null, name = null, hadConfig = false;
  const blob = p.get("_");
  if (blob) {
    try {
      const bytes = b64uToBytes(blob.slice(1));
      const obj = unaliasKeys(JSON.parse(blob[0] === "1" ? await inflate(bytes) : DEC.decode(bytes)));
      if (obj.model != null) name = obj.model;
      delete obj.model;
      raw = obj; hadConfig = Object.keys(obj).length > 0;
    } catch (e) { console.warn("ignoring malformed compact link", e); }
  }
  if (!raw) {
    raw = {};
    for (const [k, v] of p) {
      if (k === "_") continue;
      const key = KEY_UNALIAS[k] ?? k;    // aliased code → field; old full-name links pass through
      if (key === "model") { name = v; continue; }
      hadConfig = true;
      let val; try { val = JSON.parse(v); } catch { val = v; }   // "-119"→-119, "x-ray"→"x-ray"
      raw[key] = val;
    }
    const m = location.hash.match(/cfg=([^&]+)/);       // legacy base64 blob
    if (m) {
      try { Object.assign(raw, JSON.parse(DEC.decode(b64uToBytes(m[1])))); hadConfig = true; }
      catch (e) { console.warn("ignoring malformed config link", e); }
    }
  }
  applyConfig(raw);
  renderOverride = hadConfig ? structuredClone(raw) : null;   // exact render, until first edit
  return { name, hadConfig, raw };
}
// An enabled toggle-group filled from a field's defaults.
const groupBase = (field) => ({ __on: true, ...structuredClone(field.def) });
// Normalize a plugin-style OR demo-state config object into demo state (in place).
// Lets documentation deep-links use the clean plugin config (sss:{…}, fresnel:0.3,
// background:"none", highlight:{name:color}) and still populate the UI correctly.
function applyConfig(cfg) {
  for (const k in cfg) {
    let v = cfg[k];
    const f = TOP_FIELDS[k];
    if (v === "none" || v === null) {          // Typst `none` = transparent bg / unset default
      if (k === "background") state._bgNone = true;
      continue;
    }
    if (k === "background" && v === "") { state._bgNone = true; continue; }
    if (f && f.t === "grp" && k !== "clip") {                   // toggle groups: true | {…} | scalar shorthands (clip handled below)
      const base = groupBase(f);
      if (v === true) v = base;
      else if (k === "fresnel" && typeof v === "number") v = { ...base, intensity: v };
      else if (k === "tone_mapping" && typeof v === "string") v = { ...base, method: v };
      else if (v && typeof v === "object" && !Array.isArray(v)) v = { ...base, ...v };
      state[k] = v; continue;
    }
    if (f && f.t === "map" && v && typeof v === "object" && !Array.isArray(v)) {   // {name: color|{color,…}} → rows
      state[k] = Object.entries(v).map(([n, cv]) => [n, f.rich ? hlNormalize(cv) : (cv && typeof cv === "object" ? (cv.color || "#88ccff") : cv)]);
      continue;
    }
    if (k === "ambient" && v && typeof v === "object" && !Array.isArray(v)) {      // hemisphere ambient
      state._hemi = { ...groupBase(TOP_FIELDS._hemi), ...v }; continue;
    }
    if (k === "clip" && v && typeof v === "object" && !Array.isArray(v)) {          // plugin clip → demo state
      const cl = groupBase(TOP_FIELDS.clip);
      if ("depth" in v) cl.depth = v.depth;
      if (Array.isArray(v.plane)) { cl.source = "plane"; cl.plane = v.plane.slice(); }   // explicit plane a·x+b·y+c·z+d
      else if (v.from) cl.source = v.from; else if (v.axis) cl.source = v.axis;          // camera | x/y/z
      if (v.keep) cl.keep = v.keep;
      if ("cap" in v) cl.cap = v.cap;
      if (v.hatch && typeof v.hatch === "object") {
        cl.hatch = true;
        const h = v.hatch;
        if (h.style != null) cl.hstyle = h.style;
        if (h.angle != null) cl.hangle = h.angle;
        if (h.spacing != null) cl.hspacing = h.spacing;
        if (h.width != null) cl.hwidth = h.width;
        if (h.color != null) cl.hcolor = h.color;
      }
      state.clip = cl; continue;
    }
    if (k in state) state[k] = v;
  }
  if (!("_cam" in cfg)) {                                       // infer camera mode from which params are present
    if ("azimuth" in cfg || "elevation" in cfg || "distance" in cfg) state._cam = "spherical";
    else if ("camera" in cfg) state._cam = "cartesian";
  }
}
$("btn-share").onclick = async () => {
  const url = await bestUrl(shareConfig());
  history.replaceState(null, "", url);
  const ok = await copyText(url);
  const b = $("btn-share"), o = b.textContent; b.textContent = ok ? "Copied!" : "Link in URL"; setTimeout(() => (b.textContent = o), 1400);
};

$("btn-reset").onclick = () => {
  const init = initState();
  for (const k in state) delete state[k];
  Object.assign(state, init);
  renderOverride = null;
  history.replaceState(null, "", location.pathname);
  $("search").value = "";
  rebuildForm(); measure(); onChange();
};

// The first manual edit of any control drops the deep-link override, so from
// there the render follows the (editable) state instead of the frozen link config.
$("form").addEventListener("input", () => { renderOverride = null; }, true);

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
  const { name: urlModel, hadConfig, raw } = await applyStateFromUrl();  // shared model + config, if any
  buildForm(); refreshVisibility();
  try {
    const module = await loadModule();
    // With a Module (not bytes), instantiate() resolves to the Instance directly.
    instance = await WebAssembly.instantiate(module, importObject);
    memory = instance.exports.memory;
    // Load the model named in the URL (any file present in the demo dir — not
    // just picker built-ins, so documentation deep-links resolve), else bunny.
    const wanted = urlModel || "bunny.obj";
    // Bare ?model=… link (no config): show that model's showcase defaults.
    if (urlModel && !hadConfig) { applyModelDefaults(wanted); buildForm(); }
    let name = wanted, bytes;
    try { const r = await fetch(wanted); if (!r.ok) throw 0; bytes = new Uint8Array(await r.arrayBuffer()); }
    catch { name = "bunny.obj"; bytes = new Uint8Array(await (await fetch("bunny.obj")).arrayBuffer()); }
    model = { name, bytes };
    syncPreset(name);
    refreshVisibility(); measure(); onChange();
    // Shorten the address bar to the compact form, so even a long readable
    // documentation link becomes short (and copy-ready) once it has loaded.
    if (hadConfig) bestUrl({ model: name, ...raw }).then(u => history.replaceState(null, "", u));
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
})();

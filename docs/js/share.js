import { $, ENC, DEC, elOutc } from "./dom.js";
import { state, model, setModel, makeModel, renderOverride, setRenderOverride, initState, resetState, getSchema, topFields, DEFAULTS_KEYS, MODEL_DEFAULTS, num, fmtT, eq, hlNormalize } from "./state.js";
import { buildConfig, buildTypst, renderCode } from "./config.js";
import { rebuildForm, buildForm, refreshVisibility, filterForm, controlRefs } from "./form.js";
import { onChange, render, safeRender, lastRender, copyText } from "./render.js";
import { applyModelDefaults, loadPresetByName, kindOf, kindDiffers, triggerRecompile, ingest, loadFile, enterScadMode, enterMolMode, setPlugin, syncGltfInfo, measure, gltfCameraNames } from "./models.js";

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
  "point_boundary", "_cam", "_hemi", "_bgNone", "scad_smooth_normals"];
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

function shareConfig() {                       // model + config diff vs the preset's baseline
  let modelName = model.name, scadSrc = null;
  if (model.scad) {
    modelName = $("preset").value || "__scad__";
    const src = $("scad-src").value;
    const canon = modelName === "__scad__" ? _scadDefault : scadCanonicalSrc[modelName];
    if (canon == null || src !== canon) { scadSrc = src; modelName = "__scad__"; }
  }
  // Baseline = schema init + the preset's MODEL_DEFAULTS overlay (same shape
  // as applyModelDefaults applies at load). Diffing against this — instead of
  // raw schema defaults — keeps `?a=lsd` short: everything the preset
  // sets doesn't need to appear in the URL.
  const base = initState();
  const ov = MODEL_DEFAULTS[modelName];
  if (ov) for (const k in ov) {
    const v = structuredClone(ov[k]);
    const cur = base[k];
    base[k] = (v && typeof v === "object" && !Array.isArray(v)
             && cur && typeof cur === "object" && !Array.isArray(cur))
      ? { ...cur, ...v } : v;
  }
  const cfg = { model: modelName };
  for (const k in state) {
    // molfig regenerates `materials` on every load from the source — no point
    // shipping the (identical) dict in the URL.
    if (model._mol && k === "materials") continue;
    if (!eq(state[k], base[k])) cfg[k] = state[k];
  }
  if (scadSrc != null) cfg.scad_src = scadSrc;
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
  let raw = null, name = null, hadConfig = false, scadSrc = null;
  const blob = p.get("_");
  if (blob) {
    try {
      const bytes = b64uToBytes(blob.slice(1));
      const obj = unaliasKeys(JSON.parse(blob[0] === "1" ? await inflate(bytes) : DEC.decode(bytes)));
      if (obj.model != null) name = obj.model;
      if (obj.scad_src != null) scadSrc = obj.scad_src;
      delete obj.model; delete obj.scad_src;
      raw = obj; hadConfig = Object.keys(obj).length > 0;
    } catch (e) { console.warn("ignoring malformed compact link", e); }
  }
  if (!raw) {
    raw = {};
    for (const [k, v] of p) {
      if (k === "_") continue;
      const key = KEY_UNALIAS[k] ?? k;    // aliased code → field; old full-name links pass through
      if (key === "model") { name = v; continue; }
      if (key === "scad_src") { scadSrc = v; continue; }
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
  // If the link points at a glTF model but our initial `model` is a maquette
  // one (defaulted at declaration), state was built from SCHEMA and applyConfig
  // will keep treating the incoming glTF fields (ibl / shadows / ground / …)
  // as maquette fields — bogus form + broken render. Switch model.name AND
  // wipe/rebuild state so applyConfig reads GLTF_SCHEMA via topFields(). The
  // same rebuild fires in the reverse direction so old maquette links still
  // work after we've been in glTF mode.
  if (name && kindDiffers(name, model.name)) {
    setModel(makeModel(name, null));
    resetState();
  }
  applyConfig(raw);
  setRenderOverride(hadConfig ? structuredClone(raw) : null);   // exact render, until first edit
  return { name, hadConfig, raw, scadSrc };
}
// An enabled toggle-group filled from a field's defaults.
const groupBase = (field) => ({ __on: true, ...structuredClone(field.def) });
// Normalize a plugin-style OR demo-state config object into demo state (in place).
// Lets documentation deep-links use the clean plugin config (sss:{…}, fresnel:0.3,
// background:"none", highlight:{name:color}) and still populate the UI correctly.
function applyConfig(cfg) {
  for (const k in cfg) {
    let v = cfg[k];
    const f = topFields()[k];
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
      state._hemi = { ...groupBase(topFields()._hemi), ...v }; continue;
    }
    if (k === "clip" && v && typeof v === "object" && !Array.isArray(v)) {          // plugin clip → demo state
      const cl = groupBase(topFields().clip);
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
  resetState();
  applyModelDefaults(model.scad ? ($("preset").value || "__scad__") : model.name);
  setRenderOverride(null);
  history.replaceState(null, "", location.pathname);
  $("search").value = "";
  if (model._gltf) { syncGltfInfo(); measure(); }
  else { rebuildForm(); measure(); onChange(); }
  if (model.scad) triggerRecompile("scad");
  else if (model._mol) triggerRecompile("mol");
};

// The first manual edit of any control drops the deep-link override, so from
// there the render follows the (editable) state instead of the frozen link config.
$("form").addEventListener("input", () => { setRenderOverride(null); }, true);

// Power-user shortcuts. Bindings match common editor conventions
// (Cmd/Ctrl+S save / share, Esc = revert, Cmd/Ctrl+Shift+D download).
// Skipped when the target is an editable text field (typing 's' in the


export { applyStateFromUrl, shareConfig, bestUrl, applyConfig };

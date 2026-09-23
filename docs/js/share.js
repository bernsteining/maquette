import { $, ENC, DEC, elOutc } from "./dom.js";
import { state, model, setModel, makeModel, renderOverride, setRenderOverride, initState, resetState, getSchema, topFields, DEFAULTS_KEYS, MODEL_DEFAULTS, num, fmtT, eq, hlNormalize } from "./state.js";
import { buildConfig, buildTypst, renderCode } from "./config.js";
import { rebuildForm, buildForm, refreshVisibility, filterForm, controlRefs } from "./form.js";
import { onChange, render, safeRender, lastRender, copyText } from "./render.js";
import { applyModelDefaults, loadPresetByName, kindOf, kindDiffers, triggerRecompile, ingest, loadFile, enterScadMode, enterMolMode, setPlugin, syncGltfInfo, measure, gltfCameraNames, _scadDefault, scadCanonicalSrc } from "./models.js";

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
    elOutc.toBlob((blob) => save(blob, "png"), "image/png");
  } else {
    save(new Blob([lastRender.bytes], { type: "image/svg+xml" }), "svg");
  }
};

const bytesToB64u = (b) => { let s = ""; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]); return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, ""); };
const b64uToBytes = (s) => { const bin = atob(s.replace(/-/g, "+").replace(/_/g, "/")); const a = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) a[i] = bin.charCodeAt(i); return a; };
const hasCompression = typeof CompressionStream !== "undefined" && typeof DecompressionStream !== "undefined";
const FIELD_CODES = ["model", "camera", "azimuth", "elevation", "distance", "center", "up", "projection",
  "fov", "zoom", "pan", "auto_center", "auto_fit", "background", "width", "height", "color", "opacity",
  "specular", "shininess", "smooth", "gamma_correction", "cull_backface", "shading", "gooch_warm",
  "gooch_cool", "cel_bands", "mode", "xray_opacity", "stroke", "wireframe", "light_dir", "ambient",
  "fresnel", "tone_mapping", "sss", "lights", "color_map", "overhang_angle", "scalar_function",
  "vertex_smoothing", "color_map_palette", "outline", "ground_shadow", "shadows", "antialias", "ssao",
  "bloom", "glow", "sharpen", "clip", "explode", "decimate", "views", "grid_labels", "turntable",
  "materials", "highlight", "annotations", "debug", "debug_color", "point_size", "point_neighbors",
  "point_boundary", "point_denoise", "point_splat", "_cam", "_hemi", "_bgNone", "scad_smooth_normals"];
const CODE_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const codeFor = (i) => i < 52 ? CODE_ALPHABET[i] : CODE_ALPHABET[((i - 52) / 52) | 0] + CODE_ALPHABET[(i - 52) % 52];
const KEY_ALIAS = {}, KEY_UNALIAS = {};
FIELD_CODES.forEach((k, i) => { const c = codeFor(i); KEY_ALIAS[k] = c; KEY_UNALIAS[c] = k; });
const aliasKeys = (o) => { const r = {}; for (const k in o) r[KEY_ALIAS[k] ?? k] = o[k]; return r; };
const unaliasKeys = (o) => { const r = {}; for (const k in o) r[KEY_UNALIAS[k] ?? k] = o[k]; return r; };
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

function shareConfig() {
  let modelName = model.name, scadSrc = null;
  if (model.scad) {
    modelName = $("preset").value || "__scad__";
    const src = $("scad-src").value;
    const canon = modelName === "__scad__" ? _scadDefault : scadCanonicalSrc[modelName];
    if (canon == null || src !== canon) { scadSrc = src; modelName = "__scad__"; }
  }
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
    if (model._mol && k === "materials") continue;
    if (!eq(state[k], base[k])) cfg[k] = state[k];
  }
  if (scadSrc != null) cfg.scad_src = scadSrc;
  return cfg;
}
const baseUrl = () => location.origin + location.pathname;
function readableUrl(cfg) {
  const parts = [];
  for (const [k, v] of Object.entries(aliasKeys(cfg))) parts.push(k + "=" + pctB(typeof v === "string" ? v : JSON.stringify(v)));
  return baseUrl() + "?" + parts.join("&");
}
async function compactUrl(cfg) {
  if (!hasCompression) return null;
  try { return baseUrl() + "?_=1" + bytesToB64u(await deflate(JSON.stringify(aliasKeys(cfg)))); }
  catch { return null; }
}
async function bestUrl(cfg) {
  const r = readableUrl(cfg), c = await compactUrl(cfg);
  return c && c.length < r.length ? c : r;
}

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
      const key = KEY_UNALIAS[k] ?? k;
      if (key === "model") { name = v; continue; }
      if (key === "scad_src") { scadSrc = v; continue; }
      hadConfig = true;
      let val; try { val = JSON.parse(v); } catch { val = v; }
      raw[key] = val;
    }
    const m = location.hash.match(/cfg=([^&]+)/);
    if (m) {
      try { Object.assign(raw, JSON.parse(DEC.decode(b64uToBytes(m[1])))); hadConfig = true; }
      catch (e) { console.warn("ignoring malformed config link", e); }
    }
  }
  if (name && kindDiffers(name, model.name)) {
    setModel(makeModel(name, null));
    resetState();
  }
  applyConfig(raw);
  setRenderOverride(hadConfig ? structuredClone(raw) : null);
  return { name, hadConfig, raw, scadSrc };
}
const groupBase = (field) => ({ __on: true, ...structuredClone(field.def) });
function applyConfig(cfg) {
  for (const k in cfg) {
    let v = cfg[k];
    const f = topFields()[k];
    if (v === "none" || v === null) {
      if (k === "background") state._bgNone = true;
      continue;
    }
    if (k === "background" && v === "") { state._bgNone = true; continue; }
    if (f && f.t === "grp" && k !== "clip") {
      const base = groupBase(f);
      if (v === true) v = base;
      else if (k === "fresnel" && typeof v === "number") v = { ...base, intensity: v };
      else if (k === "turntable" && typeof v === "number") v = { ...base, iterations: v };
      else if (k === "tone_mapping" && typeof v === "string") v = { ...base, method: v };
      else if (v && typeof v === "object" && !Array.isArray(v)) v = { ...base, ...v };
      state[k] = v; continue;
    }
    if (f && f.t === "map" && v && typeof v === "object" && !Array.isArray(v)) {
      state[k] = Object.entries(v).map(([n, cv]) => [n, f.rich ? hlNormalize(cv) : (cv && typeof cv === "object" ? (cv.color || "#88ccff") : cv)]);
      continue;
    }
    if (k === "ambient" && v && typeof v === "object" && !Array.isArray(v)) {
      state._hemi = { ...groupBase(topFields()._hemi), ...v }; continue;
    }
    if (k === "clip" && v && typeof v === "object" && !Array.isArray(v)) {
      const cl = groupBase(topFields().clip);
      if ("depth" in v) cl.depth = v.depth;
      if (Array.isArray(v.plane)) { cl.source = "plane"; cl.plane = v.plane.slice(); }
      else if (v.from) cl.source = v.from; else if (v.axis) cl.source = v.axis;
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
  if (!("_cam" in cfg)) {
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

$("form").addEventListener("input", () => { setRenderOverride(null); }, true);



export { applyStateFromUrl, shareConfig, bestUrl, applyConfig };

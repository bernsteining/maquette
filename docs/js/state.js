import { SCHEMA, GLTF_SCHEMA } from "./schema.js";

const ext = (name) => (name || "").split(".").pop().toLowerCase();
const GLTF_EXTS = new Set(["glb", "gltf", "blg"]);
const isGltf = (name) => GLTF_EXTS.has(ext(name));
const MOL_FMTS = { pdb: "pdb", cif: "cif", mmcif: "mmcif", bcif: "bcif", xyz: "xyz" };
const isMolExt = (name) => ext(name) in MOL_FMTS;

const catalog = {
  plugins: [],
  models: [],
  modelsByPlugin: {},
  pluginOfModel: {},
  molecules: {},
  modelDefaults: {},
  defaultsKeys: [],
};

function makeModel(name, bytes, extra = null) {
  const mol = catalog.molecules[name];
  const base = mol
    ? { name, bytes, _ext: "obj", _gltf: false, _mol: true, _molSrc: null, _molSrcPath: mol.src, _molFmt: mol.fmt, _molMaterials: null }
    : { name, bytes, _ext: ext(name), _gltf: isGltf(name), _mol: false };
  return extra ? { ...base, ...extra } : base;
}
let model = makeModel("bunny.obj", null);
const setModel = (v) => { model = v; };

const getSchema = () => model._gltf ? GLTF_SCHEMA : SCHEMA;

function initState() {
  const st = {};
  for (const sec of getSchema()) for (const f of sec.fields) {
    if (f.t === "grp") st[f.k] = { __on: !!f.def.__on, ...structuredClone(f.def) };
    else { const d = f.init !== undefined ? f.init : f.def; st[f.k] = Array.isArray(d) ? d.slice() : d; }
  }
  return st;
}
const state = initState();
function resetState() {
  for (const k in state) delete state[k];
  Object.assign(state, initState());
}

const isPlainObject = (v) => !!v && typeof v === "object" && !Array.isArray(v);
function overlayDefaults(target, overrides) {
  for (const k in overrides) {
    const v = structuredClone(overrides[k]);
    target[k] = isPlainObject(v) && isPlainObject(target[k]) ? { ...target[k], ...v } : v;
  }
  return target;
}

const topFieldsCache = new WeakMap();
function topFields() {
  const s = getSchema();
  let tf = topFieldsCache.get(s);
  if (!tf) topFieldsCache.set(s, tf = Object.fromEntries(s.flatMap((sec) => sec.fields.map((f) => [f.k, f]))));
  return tf;
}

function* visibleFields() {
  for (const sec of getSchema()) {
    if (sec.when && !sec.when(state, state, model)) continue;
    for (const f of sec.fields) if (!f.when || f.when(state, state, model)) yield f;
  }
}

const eq = (a, b) => {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((x, i) => x === b[i]);
  return JSON.stringify(a) === JSON.stringify(b);
};
const num = (v) => Number.isInteger(v) ? String(v) : String(+v.toFixed(4));
const round3 = (x) => Math.round(x * 1000) / 1000;
function fmtT(v) {
  if (typeof v === "string") return `"${v}"`;
  if (typeof v === "boolean") return v ? "true" : "false";
  if (typeof v === "number") return num(v);
  if (Array.isArray(v)) return `(${v.map(fmtT).join(", ")})`;
  return `(${Object.entries(v).map(([k, x]) => `${k}: ${fmtT(x)}`).join(", ")})`;
}

function group(f, mode) {
  const s = state[f.k];
  if (f.build === "clip") {
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
    if (v === undefined) continue;
    if (sub.allowBlank && v === "") continue;
    if (sub.omitIf && sub.omitIf(v)) continue;
    if (mode === "diff" && eq(v, sub.def)) continue;
    o[sub.k] = v;
  }
  return o;
}

function hlNormalize(cv) {
  const a = { color: "#88ccff", stroke: "", stroke_width: 0, opacity: 1 };
  if (typeof cv === "string") a.color = cv;
  else if (cv && typeof cv === "object") {
    if (cv.color) a.color = cv.color;
    if (cv.stroke) a.stroke = cv.stroke;
    if (cv.stroke_width != null) a.stroke_width = cv.stroke_width;
    if (cv.opacity != null) a.opacity = cv.opacity;
  }
  return a;
}
function hlCollapse(v) {
  if (typeof v === "string") return v;
  const o = { color: v.color };
  if (v.stroke) o.stroke = v.stroke;
  if (v.stroke_width) o.stroke_width = v.stroke_width;
  if (v.opacity != null && v.opacity !== 1) o.opacity = v.opacity;
  return Object.keys(o).length === 1 ? o.color : o;
}

const ambientCfg = () => state._hemi.__on
  ? { intensity: state._hemi.intensity, sky: state._hemi.sky, ground: state._hemi.ground }
  : state.ambient;
const bgCfg = () => state._bgNone ? "none" : state.background;

let renderOverride = null;
const setRenderOverride = (v) => { renderOverride = v; };
let outputFormat = "png";
const setOutputFormat = (v) => { outputFormat = v; };

export {
  ext, isGltf, MOL_FMTS, isMolExt, catalog, makeModel, model, setModel, getSchema, state, initState, resetState,
  overlayDefaults, topFields, visibleFields, eq, num, round3, fmtT, group, hlNormalize, hlCollapse, ambientCfg, bgCfg,
  renderOverride, setRenderOverride, outputFormat, setOutputFormat,
};

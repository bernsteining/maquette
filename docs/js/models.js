import { $, ENC, DEC, elCode, elMeasure, elOut, announce } from "./dom.js";
import { state, model, setModel, makeModel, ext, isGltf, MOL_FMTS, isMolExt, catalog, setRenderOverride, setOutputFormat, resetState, topFields, overlayDefaults, outputFormat } from "./state.js";
import { GLTF_SCHEMA } from "./schema.js";
import { plugins } from "./plugins.js";
import { buildMolOpts, updateScadHighlight } from "./config.js";
import { buildForm, refreshVisibility } from "./form.js";
import { showErr } from "./render.js";
import { setBboxCenter } from "./camera.js";
import { hooks } from "./hooks.js";

const PRELOAD_MAX_BYTES = 2 * 1024 * 1024;
const SCAD_DEFAULT_URL = "openscad-logo.scad";
const KNOWN_EXTS = new Set(["obj", "stl", "ply", "scad", "glb", "gltf", "blg", "pdb", "cif", "mmcif", "bcif", "xyz"]);
const INFO_FN = { obj: "get_obj_info", stl: "get_stl_info", ply: "get_ply_info" };
const GET_MODELS_LINKS = {
  maquette: { label: "More models on Sketchfab →", url: "https://sketchfab.com/3d-models?features=downloadable" },
  scad: { label: "More SCAD on Thingiverse →", url: "https://www.thingiverse.com/tag:openscad" },
  gltf: { label: "More glTF on Sketchfab →", url: "https://sketchfab.com/3d-models?features=downloadable" },
  molfig: { label: "More structures on RCSB PDB →", url: "https://www.rcsb.org/search/advanced" },
};
const GLTF_FIELDS = GLTF_SCHEMA.flatMap((s) => s.fields);
const GLTF_TIME_FIELD = GLTF_FIELDS.find((f) => f.k === "time");
const GLTF_CAMERA_FIELD = GLTF_FIELDS.find((f) => f.k === "camera_name");

let currentPluginId = null;
let scadDefault = null;
const scadCanonicalSrc = {};

const changed = () => hooks.change();
const assignDefaults = (defaults) => { for (const k in defaults || {}) state[k] = structuredClone(defaults[k]); };
const clearUrlState = () => { if (location.search || location.hash) history.replaceState(null, "", location.pathname); };
const rebuild = () => { buildForm(); refreshVisibility(); };

const isConstrainedDevice = () =>
  matchMedia("(hover: none) and (pointer: coarse)").matches ||
  navigator.connection?.saveData === true ||
  (navigator.deviceMemory ?? 8) < 4;

function preloadDemoModels(skipName) {
  if (isConstrainedDevice()) return;
  const idle = window.requestIdleCallback || ((cb) => setTimeout(cb, 300));
  idle(async () => {
    for (const [name] of catalog.models) {
      if (name === skipName || kindOf(name) !== "maquette") continue;
      try {
        const ctrl = new AbortController();
        const r = await fetch(name, { signal: ctrl.signal });
        if (!r.ok) continue;
        if (+(r.headers.get("content-length") || 0) > PRELOAD_MAX_BYTES) { ctrl.abort(); continue; }
        const bytes = new Uint8Array(await r.arrayBuffer());
        await plugins.maquette.ensure();
        await plugins.maquette.prefetch(name, bytes);
      } catch { }
    }
  });
}

const modelsReady = fetch("models.json").then((r) => r.json()).then((j) => {
  catalog.plugins.push(...(j.plugins || []));
  Object.assign(catalog.molecules, j.molecules || {});
  Object.assign(catalog.modelDefaults, j.defaults || {});
  for (const pl of catalog.plugins) {
    catalog.modelsByPlugin[pl.id] = pl.models || [];
    for (const [name, label] of catalog.modelsByPlugin[pl.id]) {
      catalog.models.push([name, label || name]);
      catalog.pluginOfModel[name] = pl.id;
    }
  }
  catalog.defaultsKeys.push(...new Set(Object.values(catalog.modelDefaults).flatMap(Object.keys)));
});

async function loadScadDefault() {
  if (scadDefault !== null) return scadDefault;
  try {
    const r = await fetch(SCAD_DEFAULT_URL);
    scadDefault = r.ok ? await r.text() : "";
  } catch { scadDefault = ""; }
  return scadDefault;
}
const canonicalScad = (name) => name === "__scad__" ? scadDefault : scadCanonicalSrc[name];

function kindOf(name) {
  if (!name) return "maquette";
  if (catalog.molecules[name] || isMolExt(name)) return "molfig";
  if (name === "__scad__" || ext(name) === "scad") return "scad";
  return isGltf(name) ? "gltf" : "maquette";
}
const kindDiffers = (a, b) => kindOf(a) !== kindOf(b);

function syncFmtToggleForKind(name) {
  const gltf = isGltf(name);
  const seg = $("fmt");
  if (seg) seg.style.display = gltf ? "none" : "";
  if (gltf && outputFormat !== "png") {
    setOutputFormat("png");
    document.querySelectorAll("#fmt button").forEach((x) => x.classList.toggle("on", x.dataset.fmt === "png"));
  }
}

function gltfCameraNames(bytes) {
  if (!bytes || bytes.length < 20) return [];
  try {
    const glb = bytes[0] === 0x67 && bytes[1] === 0x6C && bytes[2] === 0x54 && bytes[3] === 0x46;
    const jsonText = glb
      ? DEC.decode(bytes.subarray(20, 20 + new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(12, true)))
      : DEC.decode(bytes);
    return (JSON.parse(jsonText).cameras || []).map((c) => (c && typeof c.name === "string") ? c.name : "").filter(Boolean);
  } catch { return []; }
}

function applyModelDefaults(name) {
  const TF = topFields();
  for (const k of catalog.defaultsKeys) {
    const f = TF[k];
    if (f) state[k] = structuredClone(f.init !== undefined ? f.init : f.def);
  }
  const ov = catalog.modelDefaults[name];
  if (ov) overlayDefaults(state, ov);
}

function refreshGetModelsLink() {
  const el = $("get-models"); if (!el) return;
  const spec = GET_MODELS_LINKS[kindOf($("preset").value)];
  if (!spec) { el.textContent = ""; el.removeAttribute("href"); return; }
  el.textContent = spec.label;
  el.href = spec.url;
}

function loadedModel(label) {
  $("outc").setAttribute("aria-label", "3D render of " + label);
  elOut.alt = "3D render of " + label;
  announce("Loaded " + label);
}

function bindModel(name, bytes) {
  (isGltf(name) ? plugins.gltf : plugins.maquette).bind(bytes, name).catch((e) => console.error("bind failed:", e));
}

function showModel(name, bytes) {
  setModel(makeModel(name, bytes));
  syncPreset(name);
  bindModel(name, bytes);
  syncFmtToggleForKind(name);
  if (isGltf(name)) syncGltfInfo();
  refreshGetModelsLink();
}

function ingest(name, bytes) {
  const kindChanged = kindDiffers(model && model.name, name);
  setRenderOverride(null);
  showModel(name, bytes);
  if (kindChanged) { resetState(); buildForm(); }
  loadedModel(name);
  clearUrlState();
  refreshVisibility(); measure(); changed();
}

async function syncGltfInfo() {
  try {
    await plugins.gltf.ensure();
    const info = JSON.parse(DEC.decode(await plugins.gltf.callWithModel("get_gltf_info", ENC.encode("{}"))));
    const maxT = +info.max_animation_time || 0;
    const f = GLTF_TIME_FIELD;
    if (maxT > 0) {
      f.t = "rng"; f.min = 0; f.max = Math.ceil(maxT * 10) / 10; f.step = Math.max(0.02, maxT / 200); f.play = true;
    } else {
      f.t = "num"; delete f.min; delete f.max; delete f.step; delete f.play;
    }
    if (typeof state.time === "number" && state.time > maxT) state.time = 0;

    if (!catalog.modelDefaults[model.name] && Array.isArray(info.center) && info.radius > 0) {
      const [cx, cy, cz] = info.center;
      const d = info.radius * 3;
      state.center = [cx, cy, cz];
      state.camera = [cx + d, cy + d * 0.75, cz + d];
      state.up = [0, 1, 0];
      state.fov = 40;
    }
    const camNames = gltfCameraNames(model.bytes);
    if (GLTF_CAMERA_FIELD) {
      if (camNames.length) {
        GLTF_CAMERA_FIELD.t = "sel";
        GLTF_CAMERA_FIELD.opts = [["", "(auto)"], ...camNames.map((n) => [n, n])];
        if (state.camera_name && !camNames.includes(state.camera_name)) state.camera_name = "";
      } else {
        GLTF_CAMERA_FIELD.t = "txt"; delete GLTF_CAMERA_FIELD.opts;
      }
    }
    rebuild();
    changed();
  } catch { }
}

async function loadFile(file) {
  const e = ext(file.name);
  if (e === "scad") return enterScadMode(await file.text());
  if (isMolExt(file.name)) return enterMolModeFromFile(file.name, new Uint8Array(await file.arrayBuffer()));
  if (!KNOWN_EXTS.has(e)) {
    return showErr(`${file.name}: unsupported format (.${e}). Supported: .obj .stl .ply · .scad · .glb .gltf · .pdb .cif .mmcif .bcif .xyz`);
  }
  $("tab-scad").hidden = true; setTab("typst");
  ingest(file.name, new Uint8Array(await file.arrayBuffer()));
}

async function loadPreset(name) {
  try {
    const bytes = new Uint8Array(await (await fetch(name)).arrayBuffer());
    ingest(name, bytes);
    applyModelDefaults(name);
    rebuild();
    changed();
  } catch (e) { showErr("failed to load " + name + ": " + e.message); }
}

function syncPreset(name) {
  const pluginId = catalog.pluginOfModel[name] || kindOf(name);
  if (pluginId !== currentPluginId) setPlugin(pluginId, { autoLoad: false });
  const sel = $("preset");
  if ((catalog.modelsByPlugin[pluginId] || []).some(([v]) => v === name)) { sel.value = name; return; }
  let custom = sel.querySelector("option[data-custom]");
  if (!custom) { custom = document.createElement("option"); custom.dataset.custom = "1"; sel.append(custom); }
  custom.value = name; custom.textContent = name + " (loaded)"; sel.value = name;
}

function fillPresetDropdown(pluginId) {
  $("preset").replaceChildren(...(catalog.modelsByPlugin[pluginId] || []).map(([v, t]) => {
    const o = document.createElement("option");
    o.value = v; o.textContent = t;
    return o;
  }));
}

async function loadScadPreset(name) {
  try {
    const src = await (await fetch(name)).text();
    scadCanonicalSrc[name] = src;
    await enterScadMode(src, name);
  } catch (e) { showErr("failed to load " + name + ": " + e.message); }
}

function loadPresetByName(name, opts = {}) {
  if (!name) return;
  if (name === "__scad__") return enterScadMode();
  if (ext(name) === "scad") return loadScadPreset(name);
  if (catalog.molecules[name]) return enterMolMode(name, opts);
  $("tab-scad").hidden = true; setTab("typst");
  return loadPreset(name);
}

function setPlugin(pluginId, { autoLoad = true } = {}) {
  currentPluginId = pluginId;
  document.querySelectorAll("#plugin button").forEach((b) => {
    const on = b.dataset.plugin === pluginId;
    b.classList.toggle("on", on);
    b.setAttribute("aria-pressed", on ? "true" : "false");
  });
  fillPresetDropdown(pluginId);
  const pl = catalog.plugins.find((p) => p.id === pluginId);
  if (pl) {
    const link = (id, url, label) => {
      const el = $(id); if (!el || !url) return;
      el.href = url; el.setAttribute("aria-label", `${pl.label} — ${label}`);
    };
    link("btn-docs", pl.docs, "documentation");
    link("link-typst", pl.typst, "on Typst Universe");
    link("link-github", pl.github, "on GitHub");
    const fmts = $("formats");
    if (fmts) fmts.textContent = pl.formats ? "Supports " + pl.formats.join(", ") : "";
  }
  if (autoLoad) {
    const first = (catalog.modelsByPlugin[pluginId] || [])[0];
    if (first) loadPresetByName(first[0]);
  }
}

function setTab(which) {
  if (which === "scad" && $("tab-scad").hidden) which = "typst";
  $("tab-scad").classList.toggle("on", which === "scad");
  $("tab-typst").classList.toggle("on", which === "typst");
  $("scad-editor").hidden = which !== "scad";
  const foot = $("scad-foot"); if (foot) foot.hidden = which !== "scad";
  if (which !== "scad") { const s = $("scad-status"); if (s) s.textContent = ""; }
  elCode.style.display = which === "typst" ? "" : "none";
  const cs = $("code-status"); if (cs && which !== "typst") cs.hidden = true;
}

async function renderScadResult(ply) {
  const kindChanged = kindDiffers(model && model.name, "model.ply");
  setModel(makeModel("model.ply", ply, { scad: true }));
  try { await plugins.maquette.bind(ply); }
  catch (e) { console.error("bind failed:", e); }
  if (kindChanged) {
    resetState();
    assignDefaults(catalog.modelDefaults.__scad__);
    buildForm();
    syncFmtToggleForKind("model.ply");
  }
  setRenderOverride(null);
  refreshVisibility(); measure(); changed();
}

async function compileScad() {
  const src = $("scad-src").value;
  const status = $("scad-status");
  if (!src.trim()) { status.textContent = ""; return; }
  try {
    status.textContent = "compiling…";
    await plugins.scad.ensure();
    const t = performance.now();
    const opts = { fn: 32 };
    if (state.scad_smooth_normals) opts.smooth_normals = 30;
    const ply = await plugins.scad.call("build_scad", ENC.encode(src), ENC.encode("{}"), ENC.encode(JSON.stringify(opts)), new Uint8Array());
    status.textContent = `compiled in ${Math.round(performance.now() - t)} ms`;
    showErr("");
    await renderScadResult(ply);
  } catch (e) {
    status.textContent = "error";
    showErr("OpenSCAD: " + e.message);
  }
}

async function enterScadMode(initial, presetName = "__scad__") {
  setPlugin("scad", { autoLoad: false });
  $("preset").value = presetName;
  $("tab-scad").hidden = false;
  $("snippet-sec").open = true;
  const ta = $("scad-src");
  if (initial !== undefined) ta.value = initial;
  else if (!ta.value.trim()) ta.value = await loadScadDefault();
  updateScadHighlight();
  setTab("scad");
  assignDefaults(catalog.modelDefaults[presetName] || catalog.modelDefaults.__scad__);
  rebuild();
  refreshGetModelsLink();
  loadedModel("OpenSCAD model");
  await compileScad();
}

function unpackMolBundle(buf) {
  const dec = new TextDecoder();
  const materialsLen = +dec.decode(buf.slice(0, 8));
  const infoLen = +dec.decode(buf.slice(8, 16));
  const materialsEnd = 16 + materialsLen;
  return {
    materials: JSON.parse(dec.decode(buf.slice(16, materialsEnd))),
    mesh: buf.slice(materialsEnd + infoLen),
  };
}

async function compileMol() {
  if (!model._mol || !model._molSrc) return;
  try {
    await plugins.molfig.ensure();
    const opts = buildMolOpts(state, model._molFmt);
    const bundle = await plugins.molfig.call("render_object_bundle", model._molSrc, ENC.encode(JSON.stringify(opts)));
    const { materials, mesh } = unpackMolBundle(bundle);
    model.bytes = mesh;
    model._molMaterials = materials || {};
    state.materials = Object.entries(model._molMaterials);
    await plugins.maquette.bind(mesh);
    showErr("");
    refreshVisibility(); measure(); changed();
  } catch (e) {
    showErr("molfig: " + e.message);
  }
}

async function enterMolModeFromFile(name, bytes) {
  try {
    setPlugin("molfig", { autoLoad: false });
    $("tab-scad").hidden = true; setTab("typst");
    const kindChanged = kindDiffers(model && model.name, name);
    setModel({ name, bytes: null, _ext: "obj", _gltf: false, _mol: true, _molSrc: bytes, _molSrcPath: name, _molFmt: MOL_FMTS[ext(name)] || "auto", _molMaterials: null });
    if (kindChanged) resetState();
    rebuild();
    syncPreset(name);
    syncFmtToggleForKind(name);
    refreshGetModelsLink();
    loadedModel(name);
    clearUrlState();
    await compileMol();
  } catch (e) { showErr("failed to load molecule " + name + ": " + e.message); }
}

async function enterMolMode(name, { applyDefaults = true } = {}) {
  try {
    setPlugin("molfig", { autoLoad: false });
    $("preset").value = name;
    $("tab-scad").hidden = true; setTab("typst");
    const mol = catalog.molecules[name];
    if (!mol) return showErr("unknown molecule: " + name);
    const kindChanged = kindDiffers(model && model.name, name);
    const srcBytes = new Uint8Array(await (await fetch(mol.src)).arrayBuffer());
    setModel(makeModel(name, null, { _molSrc: srcBytes }));
    if (kindChanged) resetState();
    if (applyDefaults) applyModelDefaults(name);
    rebuild();
    syncFmtToggleForKind(name);
    refreshGetModelsLink();
    loadedModel(name);
    if (applyDefaults) clearUrlState();
    await compileMol();
  } catch (e) { showErr("failed to load molecule " + name + ": " + e.message); }
}

async function measure() {
  elMeasure.replaceChildren();
  const fn = INFO_FN[model._ext];
  if (!model.bytes || !fn) return;
  try {
    await plugins.maquette.ensure();
    const info = JSON.parse(DEC.decode(await plugins.maquette.callWithModel(fn, ENC.encode("{}"))));
    if (Array.isArray(info.bbox_center)) setBboxCenter(info.bbox_center);
    const n = (x) => Number.isInteger(x) ? x.toLocaleString() : (+x).toPrecision(3);
    const p3 = (x) => (+x).toPrecision(3);
    const stats = [
      ["tris", info.triangles != null && n(info.triangles)],
      ["verts", info.vertices != null && n(info.vertices)],
      ["size", info.size && info.size.map(p3).join(" × ")],
      ["area", info.surface_area != null && p3(info.surface_area)],
      ["volume", info.volume != null && p3(info.volume)],
      ["radius", info.bbox_radius != null && p3(info.bbox_radius)],
    ].filter(([, v]) => v);
    elMeasure.innerHTML = stats.map(([k, v]) => `<span class="stat">${k} <b>${v}</b></span>`).join("");
  } catch { }
}

function triggerRecompile(kind) {
  if (kind === "scad" && currentPluginId === "scad") compileScad();
  else if (kind === "mol" && model._mol) compileMol();
}

async function initModelsUi() {
  $("browse").onclick = () => $("file").click();
  $("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);
  $("tab-scad").onclick = () => setTab("scad");
  $("tab-typst").onclick = () => setTab("typst");
  let scadTimer = null;
  $("scad-src").addEventListener("input", () => {
    updateScadHighlight();
    clearTimeout(scadTimer); scadTimer = setTimeout(compileScad, 350);
  });
  $("scad-src").addEventListener("scroll", () => {
    const hl = $("scad-hl"), src = $("scad-src");
    hl.scrollTop = src.scrollTop; hl.scrollLeft = src.scrollLeft;
  });
  await modelsReady;
  $("plugin").replaceChildren(...catalog.plugins.map((pl) => {
    const b = document.createElement("button");
    b.type = "button"; b.dataset.plugin = pl.id; b.textContent = pl.label;
    b.setAttribute("aria-pressed", "false");
    if (pl.hint) b.title = pl.hint;
    b.onclick = () => setPlugin(pl.id);
    return b;
  }));
  if (catalog.plugins.length) setPlugin(catalog.plugins[0].id, { autoLoad: false });
  $("preset").onchange = () => loadPresetByName($("preset").value);
}

export {
  initModelsUi, modelsReady, preloadDemoModels, loadFile, loadPresetByName, showModel, applyModelDefaults, syncGltfInfo,
  measure, triggerRecompile, enterScadMode, kindOf, kindDiffers, canonicalScad,
};

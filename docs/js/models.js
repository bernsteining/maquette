import { $, ENC, DEC, elCode, elMeasure, elOutc, elOut, announce } from "./dom.js";
import { state, model, setModel, makeModel, getSchema, ext, isGltf, MOL_FMTS, isMolExt, GLTF_SCHEMA, PLUGINS, MODELS, MODELS_BY_PLUGIN, MOLECULES, PLUGIN_OF_MODEL, MODEL_DEFAULTS, DEFAULTS_KEYS, setRenderOverride, setOutputFormat, resetState, topFields, num, fmtT, eq, outputFormat } from "./state.js";
import { maquettePlugin, scadPlugin, gltfPlugin, molfigPlugin, bindModel } from "./plugins.js";
import { buildConfig, buildMolOpts, updateScadHighlight, renderCode, buildTypst } from "./config.js";
import { buildForm, rebuildForm, refreshVisibility, controlRefs, filterForm } from "./form.js";
import { onChange, render, safeRender, setStageBusy, showErr, scheduleRender } from "./render.js";
import { ensureSpherical, setBboxCenter } from "./camera.js";

let currentPluginId = null;   // active plugin tab (moved here from the state cluster)

const isConstrainedDevice = () =>
  matchMedia("(hover: none) and (pointer: coarse)").matches ||
  navigator.connection?.saveData === true ||
  (navigator.deviceMemory ?? 8) < 4;

// Warm the picker's other built-in models into the worker's named cache
// during idle time. Uses `cache()` (not `setModel()`) so the active model
// isn't disturbed — every subsequent preset click then flips the active
// pointer with zero bytes over postMessage. Runs sequentially so we don't
// saturate slow connections; fires on requestIdleCallback so it never
// competes with user actions for main-thread cycles. Skipped on mobile /
// low-memory devices, and even on desktop we HEAD-probe first and skip
// any asset > 2 MB (tokyo, crankshaft) — those still work on click, they
// just don't ride in the JS heap the whole session.
function preloadDemoModels(skipName) {
  if (isConstrainedDevice()) return;
  const idle = window.requestIdleCallback || (cb => setTimeout(cb, 300));
  idle(async () => {
    for (const [presetName] of MODELS) {
      if (presetName === skipName) continue;
      if (kindOf(presetName) !== "maquette") continue;   // scad / gltf / molfig fetch lazily
      try {
        const head = await fetch(presetName, { method: "HEAD" });
        if (!head.ok) continue;
        const len = +(head.headers.get("content-length") || 0);
        if (len > 2 * 1024 * 1024) continue;
        const r = await fetch(presetName);
        if (!r.ok) continue;
        const bytes = new Uint8Array(await r.arrayBuffer());
        await maquettePlugin.ensure();
        await maquettePlugin.cache(presetName, bytes);
      } catch { /* one preload failure isn't fatal; try the next */ }
    }
  });
}

// glTF-format extensions the maquette-gltf plugin handles. `.blg` is our
// convention for a GLB renamed with a friendly extension (Damaged Helmet).

function triggerRecompile(kind) {
  if (kind === "scad" && currentPluginId === "scad") compileScad();
  else if (kind === "mol" && model._mol) compileMol();
}

// Tooltips (hover a label) — keyed by field key. Covers top-level fields and
// group headers, whose keys are unique. Shown via the native title attribute.

// ─────────────────────────────── model I/O ────────────────────────────────
// Two-tier picker: plugin (segmented control) → example (dropdown). models.json
// is the source of truth for both tiers; the loader flattens plugin→models
// into MODELS/PLUGIN_OF_MODEL for downstream single-name lookups.
const modelsReady = fetch("models.json").then(r => r.json()).then(j => {
  PLUGINS.push(...(j.plugins || []));
  Object.assign(MOLECULES, j.molecules || {});
  Object.assign(MODEL_DEFAULTS, j.defaults || {});
  MODELS.length = 0;
  for (const k in MODELS_BY_PLUGIN) delete MODELS_BY_PLUGIN[k];
  for (const k in PLUGIN_OF_MODEL) delete PLUGIN_OF_MODEL[k];
  for (const pl of PLUGINS) {
    MODELS_BY_PLUGIN[pl.id] = pl.models || [];
    for (const [name, label] of MODELS_BY_PLUGIN[pl.id]) {
      MODELS.push([name, label || name]);
      PLUGIN_OF_MODEL[name] = pl.id;
    }
  }
  DEFAULTS_KEYS.push(...new Set(Object.values(MODEL_DEFAULTS).flatMap(Object.keys)));
});
// Default source for the editor — the OpenSCAD project's own logo.scad,
// served next to this app.js. Fetched on first entry into scad mode and
// cached; the fetch is fire-and-forget so the demo boot doesn't pay for
// bytes the user may never look at.
const SCAD_DEFAULT_URL = "openscad-logo.scad";
let _scadDefault = null;
const scadCanonicalSrc = {};
async function loadScadDefault() {
  if (_scadDefault !== null) return _scadDefault;
  try {
    const r = await fetch(SCAD_DEFAULT_URL);
    _scadDefault = r.ok ? await r.text() : "";
  } catch { _scadDefault = ""; }
  return _scadDefault;
}

// MODELS + MODEL_DEFAULTS live in docs/models.json — see the modelsReady loader above.
// Field-by-key lookups. `topFields()` returns the flat {k → field} map for
// the currently-active schema. Cached per-schema in a WeakMap so hot callers
// (applyConfig walks it once per config key) don't repeat the flatMap. Both
// SCHEMA and GLTF_SCHEMA are stable const references, so no invalidation is
// needed — a schema swap just misses the cache once, then reuses.

const kindOf = (name) => {
  if (!name) return "maquette";
  if (MOLECULES[name] || isMolExt(name)) return "molfig";
  if (name === "__scad__" || ext(name) === "scad") return "scad";
  return isGltf(name) ? "gltf" : "maquette";
};
const kindDiffers = (a, b) => kindOf(a) !== kindOf(b);

// The PNG/SVG toggle is meaningless for glTF assets (PBR is raster-only).
// Hide the control + force PNG when in glTF mode; show + leave alone
// otherwise. Called from every site that changes `model.name` from outside
// ingest() (renderScadResult, shared-link boot). ingest() calls this too.
function syncFmtToggleForKind(name) {
  const gltf = isGltf(name);
  const seg = document.getElementById("fmt");
  if (seg) seg.style.display = gltf ? "none" : "";
  if (gltf && outputFormat !== "png") {
    setOutputFormat("png");
    document.querySelectorAll("#fmt button").forEach((x) => x.classList.toggle("on", x.dataset.fmt === "png"));
  }
}

// Cached reference to the Animation-time field descriptor. syncGltfInfo()
// rewrites its `t`/`min`/`max`/`step` in place when the loaded asset has
// animations. Caching skips an O(sections × fields) search on every ingest.
const GLTF_TIME_FIELD = GLTF_SCHEMA.flatMap(s => s.fields).find(f => f.k === "time");
const GLTF_CAMERA_FIELD = GLTF_SCHEMA.flatMap(s => s.fields).find(f => f.k === "camera_name");
function gltfCameraNames(bytes) {
  if (!bytes || bytes.length < 20) return [];
  try {
    let jsonText;
    if (bytes[0] === 0x67 && bytes[1] === 0x6C && bytes[2] === 0x54 && bytes[3] === 0x46) {
      const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
      jsonText = DEC.decode(bytes.subarray(20, 20 + dv.getUint32(12, true)));
    } else {
      jsonText = DEC.decode(bytes);
    }
    return (JSON.parse(jsonText).cameras || [])
      .map(c => (c && typeof c.name === "string") ? c.name : "")
      .filter(n => n);
  } catch { return []; }
}
function applyModelDefaults(name) {
  const TF = topFields();
  for (const k of DEFAULTS_KEYS) {   // reset to the field's starting value (init, else def)
    const f = TF[k];
    if (!f) continue;
    state[k] = structuredClone(f.init !== undefined ? f.init : f.def);
  }
  const ov = MODEL_DEFAULTS[name];
  if (!ov) return;
  for (const k in ov) {
    const v = structuredClone(ov[k]);
    const cur = state[k];
    // Merge object values on top of existing subfields so a partial preset
    // override (e.g. ground: { color: … } with no `y`) keeps the defaults
    // the field's own definition seeded via initState().
    state[k] = (v && typeof v === "object" && !Array.isArray(v)
              && cur && typeof cur === "object" && !Array.isArray(cur))
      ? { ...cur, ...v } : v;
  }
}

// Curated "get more models" targets, picked by file kind. Each is the
// smallest useful landing page for that format — canonical/curated first
// (Khronos, Stanford, official OpenSCAD examples), broader libraries
// second (Sketchfab, Thingiverse) via the docs' own "Where to find
// sample models" sections.
const GET_MODELS_LINKS = {
  maquette: { label: "More models on Sketchfab →",       url: "https://sketchfab.com/3d-models?features=downloadable" },
  scad:     { label: "More SCAD on Thingiverse →",       url: "https://www.thingiverse.com/tag:openscad" },
  gltf:     { label: "More glTF on Sketchfab →",         url: "https://sketchfab.com/3d-models?features=downloadable" },
  molfig:   { label: "More structures on RCSB PDB →",    url: "https://www.rcsb.org/search/advanced" },
};
function refreshGetModelsLink() {
  const el = $("get-models"); if (!el) return;
  // Key off the picker's current value rather than model.name — SCAD compiles
  // to "model.ply" internally, but the picker still shows the SCAD preset.
  const spec = GET_MODELS_LINKS[kindOf($("preset").value)];
  if (!spec) { el.textContent = ""; el.removeAttribute("href"); return; }
  el.textContent = spec.label;
  el.href = spec.url;
}

// Shared ingestion for both dropped/browsed files and built-in presets.
// a11y: label the render for screen readers + announce the new model.
function loadedModel(label) {
  elOutc.setAttribute("aria-label", "3D render of " + label);
  elOut.alt = "3D render of " + label;
  announce("Loaded " + label);
}
function ingest(name, bytes) {
  const kindChanged = kindDiffers(model && model.name, name);
  setModel(makeModel(name, bytes));
  syncPreset(name);
  setRenderOverride(null);
  // Bind the model into the worker. If we already stashed this name via
  // preload/prior pick, a name-only useKey() flips the active pointer with
  // ZERO bytes across postMessage. Otherwise setModel() ships the bytes and
  // stashes them under `name` so the next pick is free. Fire-and-forget:
  // worker processes messages FIFO, so any later render/info call is
  // guaranteed to see the bound bytes.
  bindModel(isGltf(name) ? gltfPlugin : maquettePlugin, name, bytes);
  // Schema swap (maquette ↔ glTF) has completely different state fields —
  // wipe + refill so the form isn't reading undefined slots.
  if (kindChanged) { resetState(); buildForm(); }
  syncFmtToggleForKind(name);
  refreshGetModelsLink();
  loadedModel(name);
  // Probe animation length + retype the Animation-time field to a slider
  // when the asset actually has animations. Async: fires alongside the
  // render, doesn't gate it.
  if (isGltf(name)) syncGltfInfo();
  if (location.search || location.hash) history.replaceState(null, "", location.pathname);  // drop a stale share link
  refreshVisibility(); measure(); onChange();
}

// Fetch triangle count + animation length from the glTF plugin and, when the
// asset is animated, retype the `time` field from a bare number input to a
// slider bounded by the actual animation duration. Rebuilds the form so the
// new control replaces the old one, then re-runs onChange to render at t=0
// (or wherever the user left the value).
async function syncGltfInfo() {
  try {
    await gltfPlugin.ensure();
    const raw = await gltfPlugin.callWithModel("get_gltf_info", ENC.encode("{}"));
    const info = JSON.parse(DEC.decode(raw));
    const maxT = +info.max_animation_time || 0;
    const f = GLTF_TIME_FIELD;   // cached at module load — no search per call
    if (maxT > 0) {
      f.t = "rng"; f.min = 0; f.max = Math.ceil(maxT * 10) / 10; f.step = Math.max(0.02, maxT / 200);
    } else {
      f.t = "num"; delete f.min; delete f.max; delete f.step;
    }
    // Clamp state.time so an old value doesn't push the slider off the end.
    if (typeof state.time === "number" && state.time > maxT) state.time = 0;

    // Auto-frame uploaded assets that have no MODEL_DEFAULTS entry.
    if (!MODEL_DEFAULTS[model.name] && Array.isArray(info.center) && info.radius > 0) {
      const [cx, cy, cz] = info.center;
      const d = info.radius * 3;
      state.center = [cx, cy, cz];
      state.camera = [cx + d, cy + d * 0.75, cz + d];
      state.up     = [0, 1, 0];
      state.fov    = 40;
    }
    const camNames = gltfCameraNames(model.bytes);
    if (GLTF_CAMERA_FIELD) {
      if (camNames.length) {
        GLTF_CAMERA_FIELD.t = "sel";
        GLTF_CAMERA_FIELD.opts = [["", "(auto)"], ...camNames.map(n => [n, n])];
        if (state.camera_name && !camNames.includes(state.camera_name)) state.camera_name = "";
      } else {
        GLTF_CAMERA_FIELD.t = "txt"; delete GLTF_CAMERA_FIELD.opts;
      }
    }
    buildForm(); refreshVisibility();
    onChange();
  } catch { /* info fetch failure isn't fatal — the number input stays */ }
}
const KNOWN_EXTS = new Set([
  "obj","stl","ply", "scad", "glb","gltf","blg", "pdb","cif","mmcif","bcif","xyz",
]);
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
    // Ingest first so the schema swap + resetState happen BEFORE we apply
    // the preset's MODEL_DEFAULTS overrides. Historically the order was
    // reversed (apply then ingest) which worked only because helmet's
    // overrides happened to equal the gltf schema defaults — tokyo has
    // world-space camera coords ~x1000 those defaults, so ingest's
    // resetState wiped them and the user saw the model rendered from
    // essentially inside its bounding box.
    ingest(name, bytes);
    applyModelDefaults(name);
    buildForm(); refreshVisibility();
    onChange();
  } catch (e) { showErr("failed to load " + name + ": " + e.message); }
}
// Reflect the active model in the dropdown; also flip the plugin picker to
// whichever plugin owns this model. A dropped file gets a transient entry
// under the plugin whose file kind matches (maquette or gltf — scad and mol
// only ship built-in presets, no drag-drop path).
function syncPreset(name) {
  const pluginId = PLUGIN_OF_MODEL[name] || kindOf(name);
  if (pluginId !== currentPluginId) setPlugin(pluginId, { autoLoad: false });
  const sel = $("preset");
  if (MODELS_BY_PLUGIN[pluginId] && MODELS_BY_PLUGIN[pluginId].some(([v]) => v === name)) {
    sel.value = name; return;
  }
  let custom = sel.querySelector("option[data-custom]");
  if (!custom) { custom = document.createElement("option"); custom.dataset.custom = "1"; sel.append(custom); }
  custom.value = name; custom.textContent = name + " (loaded)"; sel.value = name;
}
function fillPresetDropdown(pluginId) {
  const sel = $("preset");
  sel.innerHTML = "";
  const models = MODELS_BY_PLUGIN[pluginId] || [];
  for (const [v, t] of models) {
    const o = document.createElement("option");
    o.value = v; o.textContent = t;
    sel.append(o);
  }
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
  if (MOLECULES[name]) return enterMolMode(name, opts);
  $("tab-scad").hidden = true; setTab("typst");
  loadPreset(name);
}
function setPlugin(pluginId, { autoLoad = true } = {}) {
  currentPluginId = pluginId;
  document.querySelectorAll("#plugin button").forEach(b => {
    const on = b.dataset.plugin === pluginId;
    b.classList.toggle("on", on);
    b.setAttribute("aria-pressed", on ? "true" : "false");
  });
  fillPresetDropdown(pluginId);
  const pl = PLUGINS.find(p => p.id === pluginId);
  if (pl) {
    const set = (id, url, label) => {
      const el = $(id); if (!el) return;
      el.href = url; el.setAttribute("aria-label", `${pl.label} — ${label}`);
    };
    if (pl.docs)   set("btn-docs",    pl.docs,   "documentation");
    if (pl.typst)  set("link-typst",  pl.typst,  "on Typst Universe");
    if (pl.github) set("link-github", pl.github, "on GitHub");
    const fmts = $("formats");
    if (fmts) fmts.textContent = pl.formats ? "Supports " + pl.formats.join(", ") : "";
  }
  if (autoLoad) {
    const first = (MODELS_BY_PLUGIN[pluginId] || [])[0];
    if (first) loadPresetByName(first[0]);
  }
}
(async function initPresets() {
  await modelsReady;
  const seg = $("plugin");
  seg.innerHTML = "";
  for (const pl of PLUGINS) {
    const b = document.createElement("button");
    b.type = "button"; b.dataset.plugin = pl.id;
    b.textContent = pl.label;
    b.setAttribute("aria-pressed", "false");
    if (pl.hint) b.title = pl.hint;
    b.onclick = () => setPlugin(pl.id);
    seg.append(b);
  }
  if (PLUGINS.length) setPlugin(PLUGINS[0].id, { autoLoad: false });
  $("preset").onchange = () => loadPresetByName($("preset").value);
})();
$("browse").onclick = () => $("file").click();
$("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);

// ── OpenSCAD live source ───────────────────────────────────────────────────
let scadTimer, snippetTab = "typst";
// The snippet panel has two tabs: the editable OpenSCAD source and the read-only
// generated Typst (the render params to copy into a document). The OpenSCAD tab is
// only offered when the model is OpenSCAD-sourced.
function setTab(which) {
  if (which === "scad" && $("tab-scad").hidden) which = "typst";
  snippetTab = which;
  $("tab-scad").classList.toggle("on", which === "scad");
  $("tab-typst").classList.toggle("on", which === "typst");
  $("scad-editor").hidden = which !== "scad";
  // Footer (hint + status) is a SCAD-only strip — hide it whenever the
  // tab isn't scad. Clearing the status here also stops a stale
  // "compiled in N ms" from lingering after a switch back to Typst.
  const foot = $("scad-foot"); if (foot) foot.hidden = which !== "scad";
  if (which !== "scad") { const s = $("scad-status"); if (s) s.textContent = ""; }
  elCode.style.display = which === "typst" ? "" : "none";
}
$("tab-scad").onclick = () => setTab("scad");
$("tab-typst").onclick = () => setTab("typst");
// Keep the highlight layer scrolled in lockstep with the textarea.
$("scad-src").addEventListener("scroll", () => {
  const hl = $("scad-hl"), src = $("scad-src");
  hl.scrollTop = src.scrollTop; hl.scrollLeft = src.scrollLeft;
});

// Render a freshly-compiled mesh without touching the picker (keeps it on
// "OpenSCAD"), unlike ingest() which syncs the dropdown to the loaded file name.
async function renderScadResult(ply) {
  // These bytes are the LIVE output of maquette-scad.wasm compiling the editor's
  // .scad source — never a fetched file. `.ply` is the real output format (routes
  // to render-ply); `scad: true` marks the source so the Typst snippet shows the
  // real compile-scad(read("model.scad")) workflow instead of a phantom read().
  //
  // Coming from a glTF model (state built for GLTF_SCHEMA), we need to rebuild
  // state for SCHEMA before rendering — otherwise buildConfig throws on
  // `state._hemi.__on` (undefined).
  const kindChanged = kindDiffers(model && model.name, "model.ply");
  setModel(makeModel("model.ply", ply, { scad: true }));
  // Fresh PLY per compile — bind into maquette plugin's worker cache so
  // the next callWithModel picks these bytes, not a previous model's.
  // AWAIT the setModel: without it, onChange() below schedules safeRender
  // before the worker has swapped `activeModel`, so the render can pick up
  // the previous compile's bytes and paint stale geometry.
  // No key: scad output is transient (recompiled on every edit), caching
  // by name would just churn.
  try { await maquettePlugin.setModel(ply); }
  catch (e) { console.error("setModel failed:", e); }
  if (kindChanged) {
    resetState();
    // Re-apply SCAD-mode defaults after the wipe.
    const sd = MODEL_DEFAULTS.__scad__ || {};
    for (const k in sd) state[k] = structuredClone(sd[k]);
    buildForm();
    syncFmtToggleForKind("model.ply");   // show the PNG/SVG toggle again
  }
  setRenderOverride(null);
  refreshVisibility(); measure(); onChange();
}
async function compileScad() {
  const src = $("scad-src").value;
  const status = $("scad-status");
  if (!src.trim()) { status.textContent = ""; return; }
  try {
    status.textContent = "compiling…";
    await scadPlugin.ensure();
    const t = performance.now();
    const opts = { fn: 32 };
    if (state.scad_smooth_normals) opts.smooth_normals = 30;
    const ply = await scadPlugin.call("build_scad", ENC.encode(src), ENC.encode("{}"),
      ENC.encode(JSON.stringify(opts)), new Uint8Array());
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
  const sd = MODEL_DEFAULTS[presetName] || MODEL_DEFAULTS.__scad__ || {};
  for (const k in sd) state[k] = structuredClone(sd[k]);
  buildForm(); refreshVisibility();
  refreshGetModelsLink();
  loadedModel("OpenSCAD model");
  await compileScad();
}
$("scad-src").addEventListener("input", () => {
  updateScadHighlight();
  clearTimeout(scadTimer); scadTimer = setTimeout(compileScad, 350);
});

// ───────────────────── molfig — molecule → mesh (third-party plugin) ─────
// Every mol_* field becomes a molfig option: `mol_color_theme` → `color-theme`.
// Values matching the schema default are omitted (molfig fills them in) — except
// for the fields in ALWAYS_SEND, where our default disagrees with molfig's and
// stripping would silently revert (e.g. our carbon-color default is element-
// symbol for grey CPK carbons; molfig's is chain-id).

function unpackMolBundle(buf) {
  const dec = new TextDecoder();
  const materialsLen = +dec.decode(buf.slice(0, 8));
  const infoLen      = +dec.decode(buf.slice(8, 16));
  const materialsEnd = 16 + materialsLen;
  const infoEnd      = materialsEnd + infoLen;
  const materials    = JSON.parse(dec.decode(buf.slice(16, materialsEnd)));
  const info         = JSON.parse(dec.decode(buf.slice(materialsEnd, infoEnd)));
  const mesh         = buf.slice(infoEnd);
  return { materials, info, mesh };
}
async function compileMol() {
  if (!model._mol || !model._molSrc) return;
  try {
    await molfigPlugin.ensure();
    const opts = buildMolOpts(state, model._molFmt);
    const t = performance.now();
    const bundle = await molfigPlugin.call("render_object_bundle",
      model._molSrc, ENC.encode(JSON.stringify(opts)));
    const { materials, mesh } = unpackMolBundle(bundle);
    model.bytes = mesh;
    model._molMaterials = materials || {};
    state.materials = Object.entries(model._molMaterials);
    await maquettePlugin.setModel(mesh);
    showErr("");
    refreshVisibility(); measure(); onChange();
  } catch (e) {
    showErr("molfig: " + e.message);
  }
}
async function enterMolModeFromFile(name, bytes) {
  try {
    const fmt = MOL_FMTS[ext(name)] || "auto";
    setPlugin("molfig", { autoLoad: false });
    $("tab-scad").hidden = true; setTab("typst");
    const kindChanged = kindDiffers(model && model.name, name);
    setModel({
      name, bytes: null, _ext: "obj", _gltf: false, _mol: true,
      _molSrc: bytes, _molSrcPath: name, _molFmt: fmt, _molMaterials: null,
    });
    if (kindChanged) resetState();
    buildForm(); refreshVisibility();
    syncPreset(name);
    syncFmtToggleForKind(name);
    refreshGetModelsLink();
    loadedModel(name);
    if (location.search || location.hash) history.replaceState(null, "", location.pathname);
    await compileMol();
  } catch (e) { showErr("failed to load molecule " + name + ": " + e.message); }
}

async function enterMolMode(name, { applyDefaults = true } = {}) {
  try {
    setPlugin("molfig", { autoLoad: false });
    $("preset").value = name;
    $("tab-scad").hidden = true; setTab("typst");
    const mol = MOLECULES[name];
    if (!mol) return showErr("unknown molecule: " + name);
    const kindChanged = kindDiffers(model && model.name, name);
    const srcBytes = new Uint8Array(await (await fetch(mol.src)).arrayBuffer());
    setModel(makeModel(name, null, { _molSrc: srcBytes }));
    if (kindChanged) resetState();
    if (applyDefaults) applyModelDefaults(name);
    buildForm(); refreshVisibility();
    syncFmtToggleForKind(name);
    refreshGetModelsLink();
    loadedModel(name);
    if (applyDefaults && (location.search || location.hash)) {
      history.replaceState(null, "", location.pathname);
    }
    await compileMol();
  } catch (e) { showErr("failed to load molecule " + name + ": " + e.message); }
}

// ── measurements (model-intrinsic; get-*-info) ─────────────────────────────
const INFO_FN = { obj: "get_obj_info", stl: "get_stl_info", ply: "get_ply_info" };
async function measure() {
  elMeasure.innerHTML = "";
  if (!maquettePlugin.ready || !model.bytes) return;
  const fn = INFO_FN[model._ext];
  if (!fn) return;
  try {
    const info = JSON.parse(DEC.decode(await maquettePlugin.callWithModel(fn, ENC.encode("{}"))));
    if (Array.isArray(info.bbox_center)) setBboxCenter(info.bbox_center);   // for cartesian→spherical orbit
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


export { currentPluginId, _scadDefault, scadCanonicalSrc, triggerRecompile, isConstrainedDevice, preloadDemoModels, modelsReady, loadScadDefault, kindOf, kindDiffers, syncFmtToggleForKind, gltfCameraNames, applyModelDefaults, GET_MODELS_LINKS, refreshGetModelsLink, ingest, syncGltfInfo, loadFile, loadPreset, syncPreset, fillPresetDropdown, loadScadPreset, loadPresetByName, setPlugin, setTab, renderScadResult, compileScad, enterScadMode, unpackMolBundle, compileMol, enterMolModeFromFile, enterMolMode, measure, INFO_FN };

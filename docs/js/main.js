import { $, flashLabel } from "./dom.js";
import { state, model, catalog, resetState, setOutputFormat, setRenderOverride, topFields } from "./state.js";
import { buildTypst, renderCode, parseTypst, setCodeGuard, highlightCode, sizeCode } from "./config.js";
import { buildForm, rebuildForm, refreshVisibility, filterForm, initTooltips } from "./form.js";
import { initRenderer, safeRender, setStageBusy, showErr, downloadRender } from "./render.js";
import { initOrbit } from "./camera.js";
import { applyStateFromUrl, bestUrl, shareConfig, applyConfig } from "./share.js";
import { initModelsUi, modelsReady, preloadDemoModels, loadFile, loadPresetByName, showModel, applyModelDefaults, syncGltfInfo, measure, triggerRecompile, enterScadMode, kindOf } from "./models.js";
import { plugins } from "./plugins.js";
import { hooks } from "./hooks.js";

const RENDER_DEBOUNCE_MS = 120;
const URL_SYNC_MS = 400;
const CODE_PARSE_MS = 250;
const RESIZE_MS = 160;

let renderTimer = null;
let urlTimer = null;
function scheduleUrlSync() {
  clearTimeout(urlTimer);
  urlTimer = setTimeout(async () => {
    try {
      const cfg = shareConfig();
      if (model && model.scad && "scad_src" in cfg) return;
      history.replaceState(null, "", await bestUrl(cfg));
    } catch { }
  }, URL_SYNC_MS);
}
function onChange() {
  refreshVisibility();
  renderCode();
  clearTimeout(renderTimer); renderTimer = setTimeout(safeRender, RENDER_DEBOUNCE_MS);
  scheduleUrlSync();
}
hooks.change = onChange;
hooks.recompile = triggerRecompile;
hooks.viewChanged = scheduleUrlSync;

async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { }
  try {
    const ta = document.createElement("textarea");
    ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
    document.body.append(ta); ta.select();
    const ok = document.execCommand("copy"); ta.remove(); return ok;
  } catch { return false; }
}

function cfgValid(cfg) {
  const TF = topFields();
  const valid = {
    bool: (v) => typeof v === "boolean",
    num: (v) => typeof v === "number",
    rng: (v) => typeof v === "number",
    vec: (v) => Array.isArray(v) && v.every((x) => typeof x === "number"),
    sel: (v) => typeof v === "string", txt: (v) => typeof v === "string", col: (v) => typeof v === "string",
    grp: (v) => v !== null && ["object", "boolean", "number", "string"].includes(typeof v),
    map: (v) => !!v && typeof v === "object" && !Array.isArray(v),
    lights: Array.isArray, palette: Array.isArray, views: Array.isArray,
  };
  return Object.entries(cfg).every(([k, v]) => {
    if (k === "ambient") return typeof v === "number" || (!!v && typeof v === "object" && !Array.isArray(v));
    if (k === "background") return typeof v === "string";
    const f = TF[k];
    return !f || !valid[f.t] || valid[f.t](v);
  });
}

function initCodeEditor() {
  const codeSrc = $("code-src");
  if (!codeSrc) return;
  const codeStatus = $("code-status");
  let timer = null;
  codeSrc.addEventListener("input", () => {
    highlightCode(codeSrc.value);
    sizeCode();
    clearTimeout(timer);
    timer = setTimeout(() => {
      const cfg = parseTypst(codeSrc.value);
      const ok = !!cfg && cfgValid(cfg);
      if (codeStatus) codeStatus.hidden = ok;
      if (!ok) return;
      const keepW = state.width, keepH = state.height;
      resetState();
      state.width = keepW; state.height = keepH;
      applyConfig(cfg);
      setCodeGuard(true);
      buildForm();
      onChange();
      setCodeGuard(false);
    }, CODE_PARSE_MS);
  });
  codeSrc.addEventListener("scroll", () => {
    const hl = $("code-hl");
    if (hl) { hl.scrollTop = codeSrc.scrollTop; hl.scrollLeft = codeSrc.scrollLeft; }
  });
}

const nativeFs = !!(document.fullscreenEnabled || document.webkitFullscreenEnabled);
const inNativeFs = () => !!(document.fullscreenElement || document.webkitFullscreenElement);
function updateFsBtn() {
  const b = $("fs-toggle"); if (!b) return;
  b.title = inNativeFs() || $("stage").classList.contains("pseudo-fs") ? "Exit fullscreen (Esc)" : "Fullscreen (F)";
  b.setAttribute("aria-label", b.title);
}
function toggleFullscreen() {
  const stage = $("stage");
  if (nativeFs) {
    if (inNativeFs()) (document.exitFullscreen || document.webkitExitFullscreen).call(document);
    else (stage.requestFullscreen || stage.webkitRequestFullscreen).call(stage);
  } else {
    stage.classList.toggle("pseudo-fs");
    updateFsBtn();
  }
}

let helpReturnFocus = null;
function toggleHelp(force) {
  const help = $("help");
  if (!help) return;
  const open = force === undefined ? help.hidden : force;
  help.hidden = !open;
  if (open) { helpReturnFocus = document.activeElement; $("help-x").focus(); }
  else if (helpReturnFocus && helpReturnFocus.focus) { helpReturnFocus.focus(); helpReturnFocus = null; }
}

function resetAll() {
  resetState();
  applyModelDefaults(model.scad ? ($("preset").value || "__scad__") : model.name);
  setRenderOverride(null);
  history.replaceState(null, "", location.pathname);
  $("search").value = "";
  if (model._gltf) { syncGltfInfo(); measure(); }
  else { rebuildForm(); measure(); onChange(); }
  if (model.scad) triggerRecompile("scad");
  else if (model._mol) triggerRecompile("mol");
}

function initShortcuts() {
  document.addEventListener("keydown", (e) => {
    const t = e.target;
    const editable = t && t.matches?.("input, textarea, select, [contenteditable]");
    if (e.key === "Escape") {
      if (!$("help").hidden) { e.preventDefault(); toggleHelp(false); }
      else if ($("stage").classList.contains("pseudo-fs")) { e.preventDefault(); toggleFullscreen(); }
      else if (t === $("search")) { e.preventDefault(); t.value = ""; filterForm(""); t.blur(); }
      else if (!editable) { e.preventDefault(); $("btn-reset").click(); }
      return;
    }
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
      e.preventDefault(); $("btn-share").click(); return;
    }
    if (e.key === "/" && !editable) { e.preventDefault(); const s = $("search"); s.focus(); s.select(); return; }
    if (editable || e.metaKey || e.ctrlKey || e.altKey) return;
    const act = { s: "btn-share", d: "btn-download", c: "copy", r: "btn-reset" }[e.key.toLowerCase()];
    if (act) { e.preventDefault(); $(act).click(); }
    else if (e.key.toLowerCase() === "f") { e.preventDefault(); toggleFullscreen(); }
    else if (e.key === "?") { e.preventDefault(); toggleHelp(); }
  });
}

function initControls() {
  $("search").addEventListener("input", () => filterForm($("search").value));
  document.querySelectorAll("#fmt button").forEach((b) => {
    b.onclick = () => {
      setOutputFormat(b.dataset.fmt);
      document.querySelectorAll("#fmt button").forEach((x) => {
        x.classList.toggle("on", x === b);
        x.setAttribute("aria-pressed", x === b ? "true" : "false");
      });
      onChange();
    };
  });
  $("fs-toggle").onclick = toggleFullscreen;
  document.addEventListener("fullscreenchange", updateFsBtn);
  document.addEventListener("webkitfullscreenchange", updateFsBtn);
  $("help-x").onclick = () => toggleHelp(false);
  $("hint-help").onclick = () => toggleHelp(true);
  $("help").addEventListener("click", (e) => { if (e.target === $("help")) toggleHelp(false); });
  $("help").addEventListener("keydown", (e) => { if (e.key === "Tab") { e.preventDefault(); $("help-x").focus(); } });
  $("copy").onclick = async () => { await copyText(buildTypst()); flashLabel($("copy"), "Copied!", 1200); };
  $("btn-share").onclick = async () => {
    const url = await bestUrl(shareConfig());
    history.replaceState(null, "", url);
    flashLabel($("btn-share"), (await copyText(url)) ? "Copied!" : "Link in URL", 1400);
  };
  $("btn-reset").onclick = resetAll;
  $("btn-download").onclick = downloadRender;
  $("form").addEventListener("input", () => setRenderOverride(null), true);

  let resizeTimer = null;
  new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => { if (model && model.bytes) safeRender(); }, RESIZE_MS);
  }).observe($("stage"));

  for (const ev of ["dragenter", "dragover"]) document.addEventListener(ev, (e) => { e.preventDefault(); $("stage").classList.add("drag"); });
  for (const ev of ["dragleave", "drop"]) document.addEventListener(ev, (e) => { e.preventDefault(); if (ev === "dragleave" && e.relatedTarget) return; $("stage").classList.remove("drag"); });
  document.addEventListener("drop", (e) => { const f = e.dataTransfer?.files?.[0]; if (f) loadFile(f); });
}

function initBuildBadge() {
  const el = $("build"); if (!el) return;
  el.style.cursor = "pointer";
  const stamped = el.dataset.commit;
  let v = stamped && !stamped.startsWith("__") ? stamped : "";
  const wire = () => {
    el.title = v ? "build · " + v : "dev build";
    el.onclick = () => flashLabel(el, v || "dev", 1600);
  };
  wire();
  if (!v) fetch("build.txt").then((r) => r.ok ? r.text() : "").then((t) => {
    t = (t || "").trim();
    if (/^[0-9a-f]{7,40}$/.test(t)) { v = t.slice(0, 12); wire(); }
  }).catch(() => {});
}

async function fetchBytes(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
}

async function boot() {
  const { name: urlModel, hadConfig, raw, scadSrc } = await applyStateFromUrl();
  buildForm(); refreshVisibility();
  setStageBusy(true, "loading plugin…");
  try {
    await Promise.all([plugins.maquette.ensure(), modelsReady]);
    const wanted = urlModel || "bunny.obj";
    if (catalog.molecules[wanted] || kindOf(wanted) === "scad") {
      setStageBusy(false);
      const isScad = scadSrc != null || kindOf(wanted) === "scad";
      if (scadSrc != null) await enterScadMode(scadSrc, "__scad__");
      else await loadPresetByName(wanted, { applyDefaults: !hadConfig });
      if (isScad && hadConfig) { applyConfig(raw); buildForm(); refreshVisibility(); triggerRecompile("scad"); }
      if (hadConfig || scadSrc != null) {
        const cfg = scadSrc != null ? { model: "__scad__", ...raw, scad_src: scadSrc } : { model: wanted, ...raw };
        bestUrl(cfg).then((u) => history.replaceState(null, "", u));
      }
      preloadDemoModels(wanted);
      return;
    }
    if (urlModel && !hadConfig) { applyModelDefaults(wanted); buildForm(); }
    let name = wanted, bytes;
    try { bytes = await fetchBytes(wanted); }
    catch {
      name = "bunny.obj";
      bytes = await fetchBytes(name);
      resetState();
      history.replaceState(null, "", location.pathname);
    }
    showModel(name, bytes);
    refreshVisibility(); measure(); onChange();
    if (hadConfig) bestUrl({ model: name, ...raw }).then((u) => history.replaceState(null, "", u));
    preloadDemoModels(name);
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
}

initBuildBadge();
initTooltips();
initRenderer();
initOrbit();
initShortcuts();
initControls();
initCodeEditor();
initModelsUi();
boot();

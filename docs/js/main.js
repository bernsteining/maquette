import { $ } from "./dom.js";
import { state, model, setModel, makeModel, outputFormat, setOutputFormat, MOLECULES, getSchema, isGltf, resetState, setRebuildForm } from "./state.js";
import { maquettePlugin, scadPlugin, gltfPlugin, molfigPlugin, bindModel } from "./plugins.js";
import { buildConfig, renderCode, buildTypst } from "./config.js";
import { buildForm, refreshVisibility, filterForm } from "./form.js";
setRebuildForm(() => { buildForm(); refreshVisibility(); });
import { onChange, render, safeRender, scheduleRender, setStageBusy, showErr, sizeCanvasDisplay } from "./render.js";
import { ensureSpherical } from "./camera.js";
import { applyStateFromUrl, bestUrl, shareConfig, applyConfig } from "./share.js";
import { loadFile, loadPresetByName, kindOf, preloadDemoModels, modelsReady, applyModelDefaults, setPlugin, measure, syncGltfInfo, syncPreset, syncFmtToggleForKind, refreshGetModelsLink, triggerRecompile, ingest, enterScadMode, enterMolMode, loadScadDefault, currentPluginId } from "./models.js";



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

$("search").addEventListener("input", () => filterForm($("search").value));

document.querySelectorAll("#fmt button").forEach((b) => {
  b.onclick = () => {
    setOutputFormat(b.dataset.fmt);
    document.querySelectorAll("#fmt button").forEach((x) => {
      const on = x === b;
      x.classList.toggle("on", on);
      x.setAttribute("aria-pressed", on ? "true" : "false");
    });
    onChange();
  };
});

const nativeFs = !!(document.fullscreenEnabled || document.webkitFullscreenEnabled);
const inNativeFs = () => !!(document.fullscreenElement || document.webkitFullscreenElement);
const fsActive = () => inNativeFs() || $("stage").classList.contains("pseudo-fs");
function updateFsBtn() {
  const b = $("fs-toggle"); if (!b) return;
  b.title = fsActive() ? "Exit fullscreen (Esc)" : "Fullscreen (F)";
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
$("fs-toggle").onclick = toggleFullscreen;
document.addEventListener("fullscreenchange", updateFsBtn);
document.addEventListener("webkitfullscreenchange", updateFsBtn);

let helpReturnFocus = null;
function toggleHelp(force) {
  const help = $("help");
  if (!help) return;
  const open = force === undefined ? help.hidden : force;
  help.hidden = !open;
  if (open) { helpReturnFocus = document.activeElement; $("help-x").focus(); }
  else if (helpReturnFocus && helpReturnFocus.focus) { helpReturnFocus.focus(); helpReturnFocus = null; }
}
$("help-x").onclick = () => toggleHelp(false);
$("hint-help").onclick = () => toggleHelp(true);
$("help").addEventListener("click", (e) => { if (e.target === $("help")) toggleHelp(false); });
$("help").addEventListener("keydown", (e) => { if (e.key === "Tab") { e.preventDefault(); $("help-x").focus(); } });

let resizeT = null;
new ResizeObserver(() => {
  clearTimeout(resizeT);
  resizeT = setTimeout(() => { if (maquettePlugin.ready && model && model.bytes) safeRender(); }, 160);
}).observe($("stage"));

["dragenter","dragover"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); $("stage").classList.add("drag"); }));
["dragleave","drop"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); if (ev==="dragleave" && e.relatedTarget) return; $("stage").classList.remove("drag"); }));
document.addEventListener("drop", e => { const f = e.dataTransfer?.files?.[0]; if (f) loadFile(f); });

(async function boot() {
  const { name: urlModel, hadConfig, raw, scadSrc } = await applyStateFromUrl();
  buildForm(); refreshVisibility();
  setStageBusy(true, "loading plugin…");
  try {
    await Promise.all([maquettePlugin.ensure(), modelsReady]);
    const wanted = urlModel || "bunny.obj";
    if (MOLECULES[wanted] || kindOf(wanted) === "scad") {
      setStageBusy(false);
      const isScad = scadSrc != null || kindOf(wanted) === "scad";
      if (scadSrc != null) await enterScadMode(scadSrc, "__scad__");
      else await loadPresetByName(wanted, { applyDefaults: !hadConfig });
      if (isScad && hadConfig) { applyConfig(raw); buildForm(); refreshVisibility(); triggerRecompile("scad"); }
      if (hadConfig || scadSrc != null) {
        const cfg = scadSrc != null ? { model: "__scad__", ...raw, scad_src: scadSrc } : { model: wanted, ...raw };
        bestUrl(cfg).then(u => history.replaceState(null, "", u));
      }
      preloadDemoModels(wanted);
      return;
    }
    if (urlModel && !hadConfig) { applyModelDefaults(wanted); buildForm(); }
    let name = wanted, bytes;
    try { const r = await fetch(wanted); if (!r.ok) throw 0; bytes = new Uint8Array(await r.arrayBuffer()); }
    catch {
      name = "bunny.obj";
      bytes = new Uint8Array(await (await fetch("bunny.obj")).arrayBuffer());
      resetState();
      history.replaceState(null, "", location.pathname);
    }
    setModel(makeModel(name, bytes));
    syncPreset(name);
    bindModel(isGltf(name) ? gltfPlugin : maquettePlugin, name, bytes);
    syncFmtToggleForKind(name);
    if (isGltf(name)) syncGltfInfo();
    refreshGetModelsLink();
    refreshVisibility(); measure(); onChange();
    if (hadConfig) bestUrl({ model: name, ...raw }).then(u => history.replaceState(null, "", u));
    preloadDemoModels(name);
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
})();



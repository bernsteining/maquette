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

// maquette browser demo — drives the exact WASM the Typst plugin uses.
//
// The form is generated from SCHEMA (below), which mirrors maquette's full
// config surface. The same SCHEMA drives three things: the DOM controls, the
// JSON config sent to the WASM, and the minimal Typst snippet exported on the
// left. Add a field to SCHEMA and it appears in all three.


// Single-key shortcuts (outside text fields), plus Esc handling. See the help
// overlay for the list. `/` focuses search; letters trigger the toolbar buttons.
document.addEventListener("keydown", (e) => {
  const t = e.target;
  const editable = t && t.matches?.("input, textarea, select, [contenteditable]");
  if (e.key === "Escape") {                                  // close → exit fs → clear search → reset
    if (!$("help").hidden) { e.preventDefault(); toggleHelp(false); }
    else if ($("stage").classList.contains("pseudo-fs")) { e.preventDefault(); toggleFullscreen(); }
    else if (t === $("search")) { e.preventDefault(); t.value = ""; filterForm(""); t.blur(); }
    else if (!editable) { e.preventDefault(); $("btn-reset").click(); }
    return;
  }
  // Cmd/Ctrl+S also shares, so the browser's Save-page dialog never fires.
  if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
    e.preventDefault(); $("btn-share").click(); return;
  }
  if (e.key === "/" && !editable) { e.preventDefault(); const s = $("search"); s.focus(); s.select(); return; }
  if (editable || e.metaKey || e.ctrlKey || e.altKey) return;   // rest are bare single keys
  const act = { s: "btn-share", d: "btn-download", c: "copy", r: "btn-reset" }[e.key.toLowerCase()];
  if (act) { e.preventDefault(); $(act).click(); }
  else if (e.key.toLowerCase() === "f") { e.preventDefault(); toggleFullscreen(); }
  else if (e.key === "?") { e.preventDefault(); toggleHelp(); }
});

$("search").addEventListener("input", () => filterForm($("search").value));

// PNG / SVG output toggle
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

// ── fullscreen ─────────────────────────────────────────────────────────────
// Prefer the native Fullscreen API (hides browser chrome). iOS Safari only
// supports it on <video>, so fall back to a CSS overlay (.pseudo-fs fills the
// viewport) — that works everywhere, incl. iPad. The stage resize triggers the
// ResizeObserver below, which re-renders at the new size.
const nativeFs = !!(document.fullscreenEnabled || document.webkitFullscreenEnabled);
const inNativeFs = () => !!(document.fullscreenElement || document.webkitFullscreenElement);
const fsActive = () => inNativeFs() || $("stage").classList.contains("pseudo-fs");
function updateFsBtn() {   // the enter/exit glyph swap is CSS-driven; JS only updates the tooltip
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

// ── keyboard-shortcuts help overlay (a modal: focus in, trap, restore) ─────
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
// Trap Tab inside the dialog (its only control is the close button).
$("help").addEventListener("keydown", (e) => { if (e.key === "Tab") { e.preventDefault(); $("help-x").focus(); } });

// ── re-render when the stage resizes (window resize, orientation, fullscreen) ─
// renderConfig() recomputes the fit-to-view size each render, so a re-render is
// all that's needed. Debounced; only fires once a model is loaded.
let resizeT = null;
new ResizeObserver(() => {
  clearTimeout(resizeT);
  resizeT = setTimeout(() => { if (maquettePlugin.ready && model && model.bytes) safeRender(); }, 160);
}).observe($("stage"));

["dragenter","dragover"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); $("stage").classList.add("drag"); }));
["dragleave","drop"].forEach(ev => document.addEventListener(ev, e => { e.preventDefault(); if (ev==="dragleave" && e.relatedTarget) return; $("stage").classList.remove("drag"); }));
document.addEventListener("drop", e => { const f = e.dataTransfer?.files?.[0]; if (f) loadFile(f); });

// ─────────────────────────────────── boot ─────────────────────────────────
(async function boot() {
  const { name: urlModel, hadConfig, raw, scadSrc } = await applyStateFromUrl();  // shared model + config, if any
  buildForm(); refreshVisibility();
  // First-load UX: the wasm compile below can take a couple of seconds
  // (cold IDB, iOS Safari). Show the busy indicator immediately so the
  // blank canvas isn't mistaken for a broken page.
  setStageBusy(true, "loading plugin…");
  try {
    // Worker handles fetch → compile → IDB cache → instantiate for the
    // maquette plugin. Return here means it's ready to `.call()`. In
    // parallel, wait for MODEL_DEFAULTS to arrive (needed by applyModel-
    // Defaults() below and by preloadDemoModels()'s MODELS iteration).
    await Promise.all([maquettePlugin.ensure(), modelsReady]);
    // Load the model named in the URL (any file present in the demo dir — not
    // just picker built-ins, so documentation deep-links resolve), else bunny.
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
    // Bare ?model=… link (no config): show that model's showcase defaults.
    if (urlModel && !hadConfig) { applyModelDefaults(wanted); buildForm(); }
    let name = wanted, bytes;
    try { const r = await fetch(wanted); if (!r.ok) throw 0; bytes = new Uint8Array(await r.arrayBuffer()); }
    catch {
      // Wanted model unreachable (typical: URL carries "model.ply" from a
      // prior SCAD session, which no fetch can restore). Fall back to
      // bunny AND wipe the URL-restored state — otherwise settings tied
      // to the missing model (e.g. SCAD's yellow flat-shading) contaminate
      // the fallback render.
      name = "bunny.obj";
      bytes = new Uint8Array(await (await fetch("bunny.obj")).arrayBuffer());
      resetState();
      history.replaceState(null, "", location.pathname);
    }
    setModel(makeModel(name, bytes));
    syncPreset(name);
    // Shared-link boot bypasses ingest() so we run its glTF-mode setup here.
    // Bind the model into the worker (uses useKey if preloaded).
    bindModel(isGltf(name) ? gltfPlugin : maquettePlugin, name, bytes);
    syncFmtToggleForKind(name);
    if (isGltf(name)) syncGltfInfo();   // retype Animation-time to a slider when animated
    refreshGetModelsLink();
    refreshVisibility(); measure(); onChange();
    // Shorten the address bar to the compact form, so even a long readable
    // documentation link becomes short (and copy-ready) once it has loaded.
    if (hadConfig) bestUrl({ model: name, ...raw }).then(u => history.replaceState(null, "", u));
    preloadDemoModels(name);
  } catch (e) { showErr("failed to load WASM/model: " + e.message); console.error(e); }
})();



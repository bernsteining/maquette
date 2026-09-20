import { $, ENC, elOut, elOutc, elRtime, elErr } from "./dom.js";
import { state, model, outputFormat, renderOverride } from "./state.js";
import { buildConfig, renderCode } from "./config.js";
import { refreshVisibility, rebuildForm } from "./form.js";
import { maquettePlugin, gltfPlugin } from "./plugins.js";
import { shareConfig, bestUrl } from "./share.js";

let lastRender = null;    // {kind:"raw"} or {kind:"svg", bytes} of the most recent render
let rafPending = false;

function renderConfig() {
  const c = buildConfig();
  const cfg = renderOverride ? { ...c, ...renderOverride } : c;
  // Render size. width/height are preview-only (never exported): 0 (the default)
  // means "fit to view" — fill the stage at the device pixel ratio (capped) so
  // the canvas is sharp without rendering more pixels than it shows. A non-zero
  // value is an explicit override (e.g. for a high-res download).
  let rw = cfg.width, rh = cfg.height;
  if (!rw || !rh) { const a = stageAvail(), dpr = renderDpr();
    ({ w: rw, h: rh } = clampPair(Math.round((a.w || 700) * dpr), Math.round((a.h || 700) * dpr))); }
  else ({ w: rw, h: rh } = clampPair(rw, rh, 8192));
  cfg.width = rw; cfg.height = rh;
  // Display box: the largest box of the render's aspect that fits the stage.
  // The paint step pins the canvas to this, so a reduced-resolution interactive
  // frame upscales to fill the stage (rather than shrinking).
  displayDims = fitBox(rw, rh);
  const bare = !!state._bgNone;   // transparent render → drop the canvas frame so it blends
  elOutc.classList.toggle("bare", bare);
  elOut.classList.toggle("bare", bare);
  const div = (interacting && outputFormat !== "svg") ? dragDiv : 1;
  lastRenderReduced = div > 1;
  if (lastRenderReduced) {
    let dw = rw / div, dh = rh / div;
    // Keep the aspect: if the shorter axis dips below the floor, scale both up.
    const lo = Math.min(dw, dh);
    if (lo < DRAG_MIN) { const k = DRAG_MIN / lo; dw *= k; dh *= k; }
    return {
      ...cfg,
      width: Math.round(dw),
      height: Math.round(dh),
      antialias: model._gltf ? 1 : 0,   // "off": maquette=0, gltf=1
      fxaa: false,
    };
  }
  return cfg;
}
// ── render-size helpers (fit to view) ──────────────────────────────────────
const DPR_CAP = 2;            // don't render past 2× — diminishing returns, big cost
const RENDER_MAXDIM = 2048;   // hard ceiling per axis for the auto-fit path
const renderDpr = () => Math.min(Math.max(window.devicePixelRatio || 1, 1), DPR_CAP);
// The stage's content box (client size minus padding) in CSS pixels.
function stageAvail() {
  const stage = $("stage");
  if (!stage) return { w: 700, h: 700 };
  const cs = getComputedStyle(stage);
  const w = stage.clientWidth  - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const h = stage.clientHeight - parseFloat(cs.paddingTop)  - parseFloat(cs.paddingBottom);
  return { w: Math.max(0, Math.floor(w)), h: Math.max(0, Math.floor(h)) };
}
// Largest box of aspect w:h that fits the stage content area (CSS px).
function fitBox(w, h) {
  const a = stageAvail();
  if (a.w < 1 || a.h < 1 || !w || !h) return { w: w || 700, h: h || 700 };
  const s = Math.min(a.w / w, a.h / h);
  return { w: Math.max(1, Math.round(w * s)), h: Math.max(1, Math.round(h * s)) };
}
// Scale a size down (preserving aspect) so neither axis exceeds `max`.
function clampPair(w, h, max = RENDER_MAXDIM) {
  w = Math.max(1, w); h = Math.max(1, h);
  const m = Math.max(w, h);
  if (m > max) { const k = max / m; return { w: Math.round(w * k), h: Math.round(h * k) }; }
  return { w, h };
}
// Rich highlight per-group appearance ↔ demo state. Normalize a loaded value
// (plain "#color" or {color, stroke, stroke_width, opacity}) to a full object for
// editing; collapse back to the minimal form (a bare color string when that's all).

let t = null;
// Debounced URL sync: 400 ms after the last state edit, rewrite the
// address bar to a shareable link for the current state. Makes copy/paste
// from the browser bar always work without the user having to click
// Share, and lets a reload restore what they were looking at. Kept
// separate from the render debounce so a fast drag doesn't spam
// history.replaceState (which is cheap but not free).
//
// SCAD mode carries a synthesized model name ("model.ply") that no fetch
// can restore, and its yellow flat-shading defaults would bleed into
// whatever the boot fallback loads (bunny). Skip the URL sync while
// the SCAD source is the model — sharing SCAD sessions is a separate
// story (source lives in the editor, not the URL) and the previous
// static URL persists until the user picks a real model again.
let urlT = null;
function scheduleUrlSync() {
  clearTimeout(urlT);
  urlT = setTimeout(async () => {
    try {
      const cfg = shareConfig();
      if (model && model.scad && "scad_src" in cfg) return;
      history.replaceState(null, "", await bestUrl(cfg));
    } catch { /* URL sync is best-effort */ }
  }, 400);
}
function onChange() {
  refreshVisibility();
  renderCode();
  clearTimeout(t); t = setTimeout(safeRender, 120);
  scheduleUrlSync();
}

// Single in-flight coalescer for every render trigger (scheduleRender for
// drag, safeRender for form input). Without this the worker queues one
// render per event and only paints the last — but has already done all the
// wasm work for the stale ones, so a rapid drag on helmet feels like the
// UI is stuck for N × 2 seconds. The dirty flag re-fires exactly once
// after the current render completes if any trigger came in mid-flight.
let renderRunning = false;
let renderDirty = false;
// While the camera moves, shrink the render (per axis, 1/2/4/8/16) for
// smoothness. The divisor ratchets up per drag (never down mid-drag) so a still-
// slow frame degrades further and holds; it's seeded from the last full frame.
let interacting = false;
let settleTimer = null;
let displayDims = null;   // { w, h } of the intended full output, set by renderConfig
let lastFullMs = 0;       // duration of the last full-resolution render
let lastRenderReduced = false;
let dragDiv = 1;          // current per-axis downscale for the ongoing drag (ratchets up)
const DRAG_MIN = 96;       // floor per axis (aspect-preserving) while dragging
const DRAG_MAX_DIV = 16;   // most we ever shrink each axis by
const SETTLE_MS = 200;
const REDUCE_MS = 150;     // seed threshold: full frames under this need no downscale
const SMOOTH_MS = 60;      // target drag-frame budget; a slower frame ratchets the divisor up
function tierFromMs(ms) {
  if (ms <= REDUCE_MS) return 1;
  return Math.min(DRAG_MAX_DIV, 1 << (Math.floor(Math.log2(ms / REDUCE_MS)) + 1));
}
function markInteracting() {
  if (!interacting) { interacting = true; dragDiv = tierFromMs(lastFullMs); }  // seed at drag start
  if (settleTimer) clearTimeout(settleTimer);
  settleTimer = setTimeout(settleRender, SETTLE_MS);
}
function ratchetDrag(ms) {
  if (interacting && ms > SMOOTH_MS && dragDiv < DRAG_MAX_DIV) dragDiv = Math.min(DRAG_MAX_DIV, dragDiv * 2);
}
function settleRender() {
  if (settleTimer) { clearTimeout(settleTimer); settleTimer = null; }
  interacting = false;
  safeRender();
}
// Cancel the settle timer + clear the interacting flag without re-rendering —
// the pointerup path (camera.js) does its own full render right after.
function endInteraction() {
  if (settleTimer) { clearTimeout(settleTimer); settleTimer = null; }
  interacting = false;
}
// Pin the canvas CSS box to the fitted display size so its backing store (which
// varies with dpr and the interactive reduced-res path) always scales to fill
// the same on-screen box instead of shrinking.
function sizeCanvasDisplay() {
  if (!displayDims) return;
  elOutc.style.width = displayDims.w + "px";
  elOutc.style.height = displayDims.h + "px";
}
// UX: flag the stage as busy while a render is in flight — flips a CSS
// class that shows a small spinner + "rendering…" caption, and sets
// aria-busy so screen readers announce it. Delayed 150 ms so cheap
// renders (bunny drag, small tweak) never flash the indicator; only
// visible on the second-plus renders where users actually wonder.
const BUSY_DELAY_MS = 150;
let busyTimer = null;
function setStageBusy(on, label) {
  const stage = $("stage");
  if (on) {
    if (busyTimer) return;
    busyTimer = setTimeout(() => {
      busyTimer = null;
      stage.classList.add("busy");
      stage.setAttribute("aria-busy", "true");
      const cap = $("busy-cap");
      if (cap) cap.textContent = label || "rendering…";
    }, BUSY_DELAY_MS);
  } else {
    if (busyTimer) { clearTimeout(busyTimer); busyTimer = null; }
    stage.classList.remove("busy");
    stage.removeAttribute("aria-busy");
  }
}
async function safeRender() {
  if (renderRunning) { renderDirty = true; return; }
  renderRunning = true;
  setStageBusy(true, "rendering…");
  try {
    do {
      renderDirty = false;
      await render();
    } while (renderDirty);
  } finally { renderRunning = false; setStageBusy(false); }
}

let lastUrl = null;
let renderToken = 0;   // invalidates a pending async overlay draw when a newer render starts

// Tracks the last glTF asset we rendered; used to detect "cold" renders (new
// model or reloaded bytes) so we can run a fast texture-less preview pass first.
let lastGltfBytes = null;
const RFN_PNG = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
const RFN_SVG = { obj: "render_obj", stl: "render_stl", ply: "render_ply" };
// Paint a raster render response to the canvas. `resp` is either the worker's
// ImageBitmap payload { bitmap, w, h, svg } (decoded off the main thread) or a
// raw-bytes payload { result } ([0x00|0x02][w][h][rgba8][svg?]). `token` guards
// the async SVG-overlay draw against a newer render superseding this one.
function paintRaster(resp, token) {
  let w, h, svgBytes = null;
  const ctx = elOutc.getContext("2d");
  if (resp.bitmap) {
    w = resp.w; h = resp.h;
    elOutc.width = w; elOutc.height = h; sizeCanvasDisplay();
    ctx.drawImage(resp.bitmap, 0, 0);
    if (resp.bitmap.close) resp.bitmap.close();
    svgBytes = resp.svg;
  } else {
    const out = resp.result;
    w = out[1] | out[2] << 8 | out[3] << 16 | out[4] << 24;
    h = out[5] | out[6] << 8 | out[7] << 16 | out[8] << 24;
    const n = w * h * 4;
    const px = new Uint8ClampedArray(out.buffer, out.byteOffset + 9, n);
    elOutc.width = w; elOutc.height = h; sizeCanvasDisplay();
    ctx.putImageData(new ImageData(px, w, h), 0, 0);
    if (out[0] === 0x02) svgBytes = out.subarray(9 + n);
  }
  elOutc.style.display = ""; elOut.style.display = "none";
  lastRender = { kind: "raw" };
  if (svgBytes && svgBytes.length) {
    // Layer the transparent SVG overlay (labels, grid, annotations) on top.
    const url = URL.createObjectURL(new Blob([svgBytes], { type: "image/svg+xml" }));
    const svgImg = new Image();
    svgImg.onload = () => { if (token === renderToken) ctx.drawImage(svgImg, 0, 0, w, h); URL.revokeObjectURL(url); };
    svgImg.src = url;
  }
}
async function render() {
  if (!maquettePlugin.ready || !model.bytes) return;
  // glTF assets take a separate code path — different plugin (lazily loaded),
  // different config schema, always raster output (SVG mode not supported yet
  // for glTF).
  if (model._gltf) {
    try {
      await gltfPlugin.ensure();
      // "Progressive" glTF: on a cold render (new model, or a model whose
      // textures the plugin's cache hasn't seen yet) do a fast preview
      // pass with `no_textures: true` and no SSAA. That skips ~4 s of
      // JPEG decode on the helmet and paints geometry + IBL + material
      // factors instantly. The second pass (full config) then replaces
      // it with the textured render. On warm subsequent renders (config
      // tweaks on the same model) we only do the full pass — the plugin's
      // scene cache means it's already quick.
      const cold = lastGltfBytes !== model.bytes;
      lastGltfBytes = model.bytes;
      const cfg = renderConfig();
      const token = ++renderToken;
      const t0 = performance.now();
      if (cold) {
        try {
          const preview = { ...cfg, no_textures: true, antialias: 1, fxaa: false, ssao: undefined };
          const previewOut = await gltfPlugin.renderWithModel("render_gltf", ENC.encode(JSON.stringify(preview)));
          if (token !== renderToken) return;   // superseded by a newer render while awaiting the worker
          paintRaster(previewOut, token);
          // Yield to the browser so the preview actually paints before we
          // start the (multi-second) full render.
          await new Promise(r => requestAnimationFrame(r));
          if (token !== renderToken) return;
        } catch (e) { /* preview failure isn't fatal — try full pass anyway */ }
      }
      const fullOut = await gltfPlugin.renderWithModel("render_gltf", ENC.encode(JSON.stringify(cfg)));
      if (token !== renderToken) return;
      paintRaster(fullOut, token);
      const ms = performance.now() - t0;
      if (!lastRenderReduced) lastFullMs = ms;
      ratchetDrag(ms);
      elRtime.textContent = `rendered in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
      elRtime.classList.add("show");
      showErr(null);
    } catch (e) { showErr(String(e && e.message || e)); }
    return;
  }
  const fn = (outputFormat === "svg" ? RFN_SVG : RFN_PNG)[model._ext];
  if (!fn) return showErr(`unsupported file type: .${model._ext} — supported: .obj, .stl, .ply, .glb, .gltf, .blg, .scad`);
  try {
    const token = ++renderToken;
    const t0 = performance.now();
    const resp = await maquettePlugin.renderWithModel(fn, ENC.encode(JSON.stringify(renderConfig())));
    if (token !== renderToken) return;   // superseded while awaiting the worker
    const ms = performance.now() - t0;
    if (!lastRenderReduced) lastFullMs = ms;
    ratchetDrag(ms);
    // Raster output is raw RGBA ([0x00][w][h][rgba8…]); grid / turntable / debug /
    // annotations add a transparent vector overlay ([0x02][…][svg…]) — both come
    // back as a bitmap or bytes and go through paintRaster. Vector mode returns
    // SVG (0x3C), always as bytes.
    if (resp.bitmap || resp.result[0] === 0x00 || resp.result[0] === 0x02) {
      paintRaster(resp, token);
    } else {
      const out = resp.result;
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

async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* fall through */ }
  try {
    const ta = document.createElement("textarea");
    ta.value = text; ta.style.position = "fixed"; ta.style.opacity = "0";
    document.body.append(ta); ta.select();
    const ok = document.execCommand("copy"); ta.remove(); return ok;
  } catch { return false; }
}

function scheduleRender() {
  markInteracting();
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(async () => {
    // Await through safeRender's coalescer before releasing rafPending —
    // otherwise a rapid drag stream queues one worker call per rAF and the
    // worker plows through every stale frame before painting the latest.
    // With this, drag events fired mid-render silently collapse into one
    // re-render after the current one completes (via renderDirty).
    renderCode();
    try { await safeRender(); } finally { rafPending = false; }
  });
}

export { onChange, scheduleUrlSync, safeRender, render, renderConfig, scheduleRender, showErr, setStageBusy, sizeCanvasDisplay, copyText, markInteracting, settleRender, endInteraction, lastRender, paintRaster };

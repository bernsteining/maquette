import { $, ENC, elOut, elOutc, elRtime, elErr } from "./dom.js";
import { state, model, outputFormat, renderOverride } from "./state.js";
import { buildConfig, renderCode } from "./config.js";
import { refreshVisibility, rebuildForm } from "./form.js";
import { maquettePlugin, gltfPlugin } from "./plugins.js";
import { shareConfig, bestUrl } from "./share.js";

let lastRender = null;
let rafPending = false;

function renderConfig() {
  const c = buildConfig();
  const cfg = renderOverride ? { ...c, ...renderOverride } : c;
  let rw = cfg.width, rh = cfg.height;
  if (!rw || !rh) { const a = stageAvail(), dpr = renderDpr();
    ({ w: rw, h: rh } = clampPair(Math.round((a.w || 700) * dpr), Math.round((a.h || 700) * dpr))); }
  else ({ w: rw, h: rh } = clampPair(rw, rh, 8192));
  cfg.width = rw; cfg.height = rh;
  displayDims = fitBox(rw, rh);
  const bare = !!state._bgNone;
  elOutc.classList.toggle("bare", bare);
  elOut.classList.toggle("bare", bare);
  const div = (interacting && outputFormat !== "svg") ? dragDiv : 1;
  lastRenderReduced = div > 1;
  if (lastRenderReduced) {
    let dw = rw / div, dh = rh / div;
    const lo = Math.min(dw, dh);
    if (lo < DRAG_MIN) { const k = DRAG_MIN / lo; dw *= k; dh *= k; }
    return {
      ...cfg,
      width: Math.round(dw),
      height: Math.round(dh),
      antialias: model._gltf ? 1 : 0,
      fxaa: false,
    };
  }
  return cfg;
}
const DPR_CAP = 2;
const RENDER_MAXDIM = 2048;
const renderDpr = () => Math.min(Math.max(window.devicePixelRatio || 1, 1), DPR_CAP);
function stageAvail() {
  const stage = $("stage");
  if (!stage) return { w: 700, h: 700 };
  const cs = getComputedStyle(stage);
  const w = stage.clientWidth  - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const h = stage.clientHeight - parseFloat(cs.paddingTop)  - parseFloat(cs.paddingBottom);
  return { w: Math.max(0, Math.floor(w)), h: Math.max(0, Math.floor(h)) };
}
function fitBox(w, h) {
  const a = stageAvail();
  if (a.w < 1 || a.h < 1 || !w || !h) return { w: w || 700, h: h || 700 };
  const s = Math.min(a.w / w, a.h / h);
  return { w: Math.max(1, Math.round(w * s)), h: Math.max(1, Math.round(h * s)) };
}
function clampPair(w, h, max = RENDER_MAXDIM) {
  w = Math.max(1, w); h = Math.max(1, h);
  const m = Math.max(w, h);
  if (m > max) { const k = max / m; return { w: Math.round(w * k), h: Math.round(h * k) }; }
  return { w, h };
}

let t = null;
let urlT = null;
function scheduleUrlSync() {
  clearTimeout(urlT);
  urlT = setTimeout(async () => {
    try {
      const cfg = shareConfig();
      if (model && model.scad && "scad_src" in cfg) return;
      history.replaceState(null, "", await bestUrl(cfg));
    } catch {  }
  }, 400);
}
function onChange() {
  refreshVisibility();
  renderCode();
  clearTimeout(t); t = setTimeout(safeRender, 120);
  scheduleUrlSync();
}

let renderRunning = false;
let renderDirty = false;
let interacting = false;
let settleTimer = null;
let displayDims = null;
let lastFullMs = 0;
let lastRenderReduced = false;
let dragDiv = 1;
const DRAG_MIN = 96;
const DRAG_MAX_DIV = 16;
const SETTLE_MS = 200;
const REDUCE_MS = 150;
const SMOOTH_MS = 60;
function tierFromMs(ms) {
  if (ms <= REDUCE_MS) return 1;
  return Math.min(DRAG_MAX_DIV, 1 << (Math.floor(Math.log2(ms / REDUCE_MS)) + 1));
}
function markInteracting() {
  if (!interacting) { interacting = true; dragDiv = tierFromMs(lastFullMs); }
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
function endInteraction() {
  if (settleTimer) { clearTimeout(settleTimer); settleTimer = null; }
  interacting = false;
}
function sizeCanvasDisplay() {
  if (!displayDims) return;
  elOutc.style.width = displayDims.w + "px";
  elOutc.style.height = displayDims.h + "px";
}
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
let renderToken = 0;

let lastGltfBytes = null;
const RFN_PNG = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
const RFN_SVG = { obj: "render_obj", stl: "render_stl", ply: "render_ply" };
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
  $("stage").classList.add("ready");
  lastRender = { kind: "raw" };
  if (svgBytes && svgBytes.length) {
    const url = URL.createObjectURL(new Blob([svgBytes], { type: "image/svg+xml" }));
    const svgImg = new Image();
    svgImg.onload = () => { if (token === renderToken) ctx.drawImage(svgImg, 0, 0, w, h); URL.revokeObjectURL(url); };
    svgImg.src = url;
  }
}
async function render() {
  if (!maquettePlugin.ready || !model.bytes) return;
  if (model._gltf) {
    try {
      await gltfPlugin.ensure();
      const cold = lastGltfBytes !== model.bytes;
      lastGltfBytes = model.bytes;
      const cfg = renderConfig();
      const token = ++renderToken;
      const t0 = performance.now();
      if (cold) {
        try {
          const preview = { ...cfg, no_textures: true, antialias: 1, fxaa: false, ssao: undefined };
          const previewOut = await gltfPlugin.renderWithModel("render_gltf", ENC.encode(JSON.stringify(preview)));
          if (token !== renderToken) return;
          paintRaster(previewOut, token);
          await new Promise(r => requestAnimationFrame(r));
          if (token !== renderToken) return;
        } catch (e) {  }
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
    if (token !== renderToken) return;
    const ms = performance.now() - t0;
    if (!lastRenderReduced) lastFullMs = ms;
    ratchetDrag(ms);
    if (resp.bitmap || resp.result[0] === 0x00 || resp.result[0] === 0x02) {
      paintRaster(resp, token);
    } else {
      const out = resp.result;
      const url = URL.createObjectURL(new Blob([out], { type: "image/svg+xml" }));
      elOut.src = url; elOut.style.display = ""; elOutc.style.display = "none";
      $("stage").classList.add("ready");
      if (lastUrl) URL.revokeObjectURL(lastUrl); lastUrl = url;
      lastRender = { kind: "svg", bytes: out };
    }
    elRtime.textContent = `rendered in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`; elRtime.classList.add("show");
    showErr(null);
  } catch (e) { showErr(e.message); }
}
function showErr(m) { elErr.style.display = m ? "" : "none"; elErr.textContent = m || ""; }

async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch {  }
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
    renderCode();
    try { await safeRender(); } finally { rafPending = false; }
  });
}

export { onChange, scheduleUrlSync, safeRender, render, renderConfig, scheduleRender, showErr, setStageBusy, sizeCanvasDisplay, copyText, markInteracting, settleRender, endInteraction, lastRender, paintRaster };

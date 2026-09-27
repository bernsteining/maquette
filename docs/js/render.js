import { $, ENC, elOut, elOverlay, elRtime, elErr } from "./dom.js";
import { state, model, outputFormat, renderOverride } from "./state.js";
import { buildConfig } from "./config.js";
import { plugins, paintInWorker, snapshotPng } from "./plugins.js";

const DPR_CAP = 2;
const RENDER_MAXDIM = 2048;
const DRAG_MIN = 96;
const DRAG_MAX_DIV = 16;
const SETTLE_MS = 200;
const REDUCE_MS = 150;
const SMOOTH_MS = 60;
const BUSY_DELAY_MS = 150;
const RFN_PNG = { obj: "render_obj_png", stl: "render_stl_png", ply: "render_ply_png" };
const RFN_SVG = { obj: "render_obj", stl: "render_stl", ply: "render_ply" };

let lastRender = null;
let displayDims = null;
let interacting = false;
let settleTimer = null;
let lastFullMs = 0;
let lastRenderReduced = false;
let dragDiv = 1;
let renderRunning = false;
let renderDirty = false;
let rafPending = false;
let busyTimer = null;
let svgUrl = null;
let overlayUrl = null;
let lastGltfBytes = null;

let stageSize = null;
const renderDpr = () => Math.min(Math.max(window.devicePixelRatio || 1, 1), DPR_CAP);
function measureStage() {
  const stage = $("stage");
  if (!stage) return { w: 700, h: 700 };
  const cs = getComputedStyle(stage);
  const w = stage.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const h = stage.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
  return { w: Math.max(0, Math.floor(w)), h: Math.max(0, Math.floor(h)) };
}
const stageAvail = () => stageSize ??= measureStage();
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

function renderConfig() {
  const c = buildConfig();
  const cfg = renderOverride ? { ...c, ...renderOverride } : c;
  let rw = cfg.width, rh = cfg.height;
  if (!rw || !rh) {
    const a = stageAvail(), dpr = renderDpr();
    ({ w: rw, h: rh } = clampPair(Math.round((a.w || 700) * dpr), Math.round((a.h || 700) * dpr)));
  } else ({ w: rw, h: rh } = clampPair(rw, rh, 8192));
  cfg.width = rw; cfg.height = rh;
  displayDims = fitBox(rw, rh);
  const bare = !!state._bgNone;
  $("outc").classList.toggle("bare", bare);
  elOut.classList.toggle("bare", bare);
  elOverlay?.classList.toggle("bare", bare);
  const div = (interacting && outputFormat !== "svg") ? dragDiv : 1;
  lastRenderReduced = div > 1;
  if (!lastRenderReduced) return cfg;
  let dw = rw / div, dh = rh / div;
  const lo = Math.min(dw, dh);
  if (lo < DRAG_MIN) { const k = DRAG_MIN / lo; dw *= k; dh *= k; }
  return { ...cfg, width: Math.round(dw), height: Math.round(dh), antialias: model._gltf ? 1 : 0, fxaa: false };
}

function tierFromMs(ms) {
  if (ms <= REDUCE_MS) return 1;
  return Math.min(DRAG_MAX_DIV, 1 << (Math.floor(Math.log2(ms / REDUCE_MS)) + 1));
}
function markInteracting() {
  if (!interacting) { interacting = true; dragDiv = tierFromMs(lastFullMs); }
  clearTimeout(settleTimer);
  settleTimer = setTimeout(settleRender, SETTLE_MS);
}
function ratchetDrag(ms) {
  if (interacting && ms > SMOOTH_MS && dragDiv < DRAG_MAX_DIV) dragDiv = Math.min(DRAG_MAX_DIV, dragDiv * 2);
}
function endInteraction() {
  clearTimeout(settleTimer); settleTimer = null;
  interacting = false;
}
function settleRender() {
  endInteraction();
  safeRender();
}

function setStageBusy(on, label) {
  const stage = $("stage");
  if (!on) {
    clearTimeout(busyTimer); busyTimer = null;
    stage.classList.remove("busy");
    stage.removeAttribute("aria-busy");
    return;
  }
  if (busyTimer) return;
  busyTimer = setTimeout(() => {
    busyTimer = null;
    stage.classList.add("busy");
    stage.setAttribute("aria-busy", "true");
    const cap = $("busy-cap");
    if (cap) cap.textContent = label || "rendering…";
  }, BUSY_DELAY_MS);
}
function showErr(m) { elErr.style.display = m ? "" : "none"; elErr.textContent = m || ""; }

function sized(el) {
  if (!displayDims) return;
  const w = displayDims.w + "px", h = displayDims.h + "px";
  if (el.style.width !== w) el.style.width = w;
  if (el.style.height !== h) el.style.height = h;
}

function showOverlay(svg) {
  if (overlayUrl) { URL.revokeObjectURL(overlayUrl); overlayUrl = null; }
  if (!elOverlay) return;
  if (!svg || !svg.length) { elOverlay.style.display = "none"; elOverlay.removeAttribute("src"); return; }
  overlayUrl = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  elOverlay.src = overlayUrl;
  sized(elOverlay);
  elOverlay.style.display = "";
}

function readRaster(out) {
  if (out.length < 9 || (out[0] !== 0x00 && out[0] !== 0x02)) return null;
  const dv = new DataView(out.buffer, out.byteOffset);
  const w = dv.getUint32(1, true), h = dv.getUint32(5, true), n = w * h * 4;
  return { w, h, pixels: new Uint8ClampedArray(out.buffer, out.byteOffset + 9, n), svg: out[0] === 0x02 ? out.subarray(9 + n) : null };
}

function showRaster(reply) {
  const canvas = $("outc");
  let { w, h, svg } = reply;
  if (!reply.painted) {
    const r = readRaster(reply.result);
    ({ w, h, svg } = r);
    canvas.width = w; canvas.height = h;
    canvas.getContext("2d").putImageData(new ImageData(r.pixels, w, h), 0, 0);
  }
  sized(canvas);
  canvas.style.display = ""; elOut.style.display = "none";
  showOverlay(svg);
  $("stage").classList.add("ready");
  lastRender = { kind: "raster", w, h, svg: svg && svg.length ? svg : null };
}

function showSvg(out) {
  const url = URL.createObjectURL(new Blob([out], { type: "image/svg+xml" }));
  elOut.src = url; elOut.style.display = ""; $("outc").style.display = "none";
  showOverlay(null);
  $("stage").classList.add("ready");
  if (svgUrl) URL.revokeObjectURL(svgUrl);
  svgUrl = url;
  lastRender = { kind: "svg", bytes: out };
}

function show(reply) {
  if (reply.painted || readRaster(reply.result)) showRaster(reply);
  else showSvg(reply.result);
}

function reportTime(ms) {
  if (!lastRenderReduced) lastFullMs = ms;
  ratchetDrag(ms);
  elRtime.textContent = `rendered in ${ms < 10 ? ms.toFixed(1) : Math.round(ms)} ms`;
  elRtime.classList.add("show");
  showErr(null);
}

async function renderGltf() {
  await plugins.gltf.ensure();
  const cold = lastGltfBytes !== model.bytes;
  lastGltfBytes = model.bytes;
  const cfg = renderConfig();
  const t0 = performance.now();
  if (cold) {
    try {
      const preview = { ...cfg, no_textures: true, antialias: 1, fxaa: false, ssao: undefined };
      show(await plugins.gltf.render("render_gltf", ENC.encode(JSON.stringify(preview))));
      await new Promise((r) => requestAnimationFrame(r));
    } catch { }
  }
  show(await plugins.gltf.render("render_gltf", ENC.encode(JSON.stringify(cfg))));
  reportTime(performance.now() - t0);
}

async function render() {
  if (!model.bytes) return;
  try {
    if (model._gltf) return await renderGltf();
    const fn = (outputFormat === "svg" ? RFN_SVG : RFN_PNG)[model._ext];
    if (!fn) return showErr(`unsupported file type: .${model._ext} — supported: .obj, .stl, .ply, .glb, .gltf, .blg, .scad`);
    await plugins.maquette.ensure();
    const t0 = performance.now();
    const reply = await plugins.maquette.render(fn, ENC.encode(JSON.stringify(renderConfig())));
    const ms = performance.now() - t0;
    show(reply);
    reportTime(ms);
  } catch (e) { showErr(String((e && e.message) || e)); }
}

async function safeRender() {
  if (renderRunning) { renderDirty = true; return; }
  renderRunning = true;
  setStageBusy(true, "rendering…");
  try {
    do { renderDirty = false; await render(); } while (renderDirty);
  } finally { renderRunning = false; setStageBusy(false); }
}

function scheduleRender(beforeRender) {
  markInteracting();
  if (rafPending) return;
  rafPending = true;
  requestAnimationFrame(async () => {
    if (beforeRender) beforeRender();
    try { await safeRender(); } finally { rafPending = false; }
  });
}

const transferred = new WeakSet();
function canvasForWorker() {
  let canvas = $("outc");
  if (transferred.has(canvas)) {
    const fresh = canvas.cloneNode(false);
    canvas.replaceWith(fresh);
    canvas = fresh;
  }
  transferred.add(canvas);
  return canvas;
}
function initRenderer() {
  paintInWorker(canvasForWorker);
  new ResizeObserver(() => { stageSize = null; }).observe($("stage"));
}

const loadImage = (src) => new Promise((res, rej) => { const i = new Image(); i.onload = () => res(i); i.onerror = rej; i.src = src; });
const canvasBlob = (c) => new Promise((res) => c.toBlob(res, "image/png"));
async function rasterPng() {
  const png = await snapshotPng().catch(() => null) || await canvasBlob($("outc"));
  if (!lastRender.svg) return png;
  const c = document.createElement("canvas");
  c.width = lastRender.w; c.height = lastRender.h;
  const ctx = c.getContext("2d");
  ctx.drawImage(await createImageBitmap(png), 0, 0);
  const url = URL.createObjectURL(new Blob([lastRender.svg], { type: "image/svg+xml" }));
  try { ctx.drawImage(await loadImage(url), 0, 0, c.width, c.height); } finally { URL.revokeObjectURL(url); }
  return canvasBlob(c);
}
async function downloadRender() {
  if (!lastRender) return;
  const raster = lastRender.kind === "raster";
  const blob = raster ? await rasterPng() : new Blob([lastRender.bytes], { type: "image/svg+xml" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = model.name.replace(/\.[^.]+$/, "") + (raster ? ".png" : ".svg");
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

export { initRenderer, safeRender, scheduleRender, endInteraction, setStageBusy, showErr, downloadRender };

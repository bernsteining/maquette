import { $ } from "./dom.js";
import { state, model, round3, eq, setRenderOverride } from "./state.js";
import { renderCode } from "./config.js";
import { controlRefs, refreshVisibility } from "./form.js";
import { scheduleRender, safeRender, endInteraction } from "./render.js";
import * as v3 from "./vec3.js";
import { hooks } from "./hooks.js";

const DEG_PER_PX = 0.5;
const MAX_ELEVATION = 89;
const clamp = (x, lo, hi) => Math.max(lo, Math.min(hi, x));
const round1 = (x) => Math.round(x * 10) / 10;
const wrap180 = (a) => { const w = ((a + 180) % 360 + 360) % 360 - 180; return w === -180 ? 180 : w; };

let bboxCenter = [0, 0, 0];
const setBboxCenter = (c) => { bboxCenter = c; };

function ensureSpherical() {
  setRenderOverride(null);
  if (state._cam === "spherical") return;
  const center = eq(state.center, [0, 0, 0]) ? bboxCenter : state.center;
  const off = v3.sub(state.camera, center), dist = v3.length(off);
  if (dist >= 1e-6) {
    const { azimuth, elevation } = v3.toSpherical(v3.normalize(off), v3.normalize(state.up));
    state.azimuth = round1(azimuth);
    state.elevation = clamp(round1(elevation), -MAX_ELEVATION, MAX_ELEVATION);
    state.distance = round3(dist);
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.distance?.(state.distance);
  }
  state._cam = "spherical"; controlRefs._cam?.("spherical"); refreshVisibility();
}

const spherical = { az: 0, el: 0, wroteAz: NaN, wroteEl: NaN };
function orbitSpherical(dx, dy) {
  if (state.azimuth !== spherical.wroteAz || state.elevation !== spherical.wroteEl) {
    spherical.az = state.azimuth; spherical.el = state.elevation;
  }
  spherical.az = wrap180(spherical.az + dx * DEG_PER_PX);
  spherical.el = clamp(spherical.el + dy * DEG_PER_PX, -MAX_ELEVATION, MAX_ELEVATION);
  state.azimuth = spherical.wroteAz = round1(spherical.az);
  state.elevation = spherical.wroteEl = round1(spherical.el);
  controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation);
}

const cartesian = { az: 0, el: 0, dist: 1, wrote: null, center: null, up: null };
function orbitCartesian(dx, dy) {
  const c = state.center || [0, 0, 0], upRaw = state.up || [0, 1, 0], up = v3.normalize(upRaw);
  if (!cartesian.wrote || !eq(state.camera, cartesian.wrote) || !eq(c, cartesian.center) || !eq(upRaw, cartesian.up)) {
    const off = v3.sub(state.camera, c);
    cartesian.dist = v3.length(off) || 1;
    const { azimuth, elevation } = v3.toSpherical(v3.normalize(off), up);
    cartesian.az = azimuth; cartesian.el = elevation;
    cartesian.center = c.slice(); cartesian.up = upRaw.slice();
  }
  cartesian.az = wrap180(cartesian.az + dx * DEG_PER_PX);
  cartesian.el = clamp(cartesian.el + dy * DEG_PER_PX, -MAX_ELEVATION, MAX_ELEVATION);
  state.camera = v3.add(c, v3.scale(v3.fromSpherical(up, cartesian.az, cartesian.el), cartesian.dist)).map(round3);
  cartesian.wrote = state.camera.slice();
  controlRefs.camera?.(state.camera);
}

function orbitBy(dx, dy) {
  setRenderOverride(null);
  if (model._gltf) orbitCartesian(dx, dy); else orbitSpherical(dx, dy);
}

function setZoom(f) {
  setRenderOverride(null);
  if (!model._gltf) {
    state.zoom = clamp(round3(state.zoom * f), 0.3, 4);
    controlRefs.zoom?.(state.zoom);
    return;
  }
  const c = state.center || [0, 0, 0];
  state.camera = v3.add(c, v3.scale(v3.sub(state.camera, c), 1 / f)).map(round3);
  controlRefs.camera?.(state.camera);
}

function screenUp(dir, up) {
  const s = v3.sub(up, v3.scale(dir, v3.dot(up, dir)));
  return v3.normalize(v3.length(s) < 1e-4 ? v3.basis(dir).right : s);
}

function rollBy(angle) {
  setRenderOverride(null);
  if (model._gltf) {
    const dir = v3.normalize(v3.sub(state.camera, state.center || [0, 0, 0]));
    state.up = v3.normalize(v3.rotate(screenUp(dir, v3.normalize(state.up || [0, 1, 0])), dir, angle)).map(round3);
    controlRefs.up?.(state.up);
    return;
  }
  const upW = v3.normalize(state.up || [0, 0, 1]);
  const dir = v3.fromSpherical(upW, state.azimuth, state.elevation);
  const up = v3.normalize(v3.rotate(screenUp(dir, upW), dir, angle));
  state.up = up.map(round3);
  state.elevation = 0;
  state.azimuth = round1(v3.toSpherical(dir, up).azimuth);
  controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.up?.(state.up);
}

function initOrbit() {
  const stage = $("stage");
  const pts = new Map();
  let pinchDist = 0, pinchCx = 0, pinchCy = 0, pinchAngle = 0;
  const pair = () => [...pts.values()];
  const pairDist = () => { const [a, b] = pair(); return Math.hypot(a.x - b.x, a.y - b.y); };
  const pairCenter = () => { const [a, b] = pair(); return [(a.x + b.x) / 2, (a.y + b.y) / 2]; };
  const pairAngle = () => { const [a, b] = pair(); return Math.atan2(b.y - a.y, b.x - a.x); };
  const seedPinch = () => { pinchDist = pairDist(); [pinchCx, pinchCy] = pairCenter(); pinchAngle = pairAngle(); };
  const rerender = () => { scheduleRender(renderCode); hooks.viewChanged(); };

  stage.addEventListener("pointerdown", (e) => {
    if (e.target.closest("#tools, #fs-toggle, #hint-help")) return;
    if (e.pointerType === "mouse" && e.button !== 0) return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    try { stage.setPointerCapture(e.pointerId); } catch { }
    stage.classList.add("grabbing");
    if (!model._gltf) ensureSpherical();
    if (pts.size === 2) seedPinch();
  });

  stage.addEventListener("pointermove", (e) => {
    const p = pts.get(e.pointerId);
    if (!p) return;
    if (pts.size >= 2) {
      p.x = e.clientX; p.y = e.clientY;
      const d = pairDist(), [cx, cy] = pairCenter(), ang = pairAngle();
      if (pinchDist > 0 && d > 0) setZoom(d / pinchDist);
      let dA = ang - pinchAngle;
      if (dA > Math.PI) dA -= 2 * Math.PI; else if (dA < -Math.PI) dA += 2 * Math.PI;
      if (Math.abs(dA) > 1e-4) rollBy(dA);
      orbitBy(cx - pinchCx, cy - pinchCy);
      pinchDist = d; pinchCx = cx; pinchCy = cy; pinchAngle = ang;
    } else {
      orbitBy(e.clientX - p.x, e.clientY - p.y);
      p.x = e.clientX; p.y = e.clientY;
    }
    rerender();
  });

  const end = (e) => {
    if (!pts.delete(e.pointerId)) return;
    pinchDist = 0;
    if (pts.size === 0) {
      stage.classList.remove("grabbing");
      endInteraction();
      renderCode();
      safeRender();
      hooks.viewChanged();
    } else if (pts.size === 2) seedPinch();
  };
  stage.addEventListener("pointerup", end);
  stage.addEventListener("pointercancel", end);

  stage.addEventListener("wheel", (e) => {
    e.preventDefault();
    setZoom(Math.exp(-clamp(e.deltaY, -200, 200) * 0.0011));
    rerender();
  }, { passive: false });

  stage.tabIndex = 0;
  document.addEventListener("keydown", (e) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target?.matches?.("input, textarea, select, [contenteditable]")) return;
    const step = e.shiftKey ? 30 : 10;
    const act = {
      ArrowLeft: () => orbitBy(-step, 0), ArrowRight: () => orbitBy(step, 0),
      ArrowUp: () => orbitBy(0, -step), ArrowDown: () => orbitBy(0, step),
      PageUp: () => setZoom(Math.exp(0.1)), "+": () => setZoom(Math.exp(0.1)), "=": () => setZoom(Math.exp(0.1)),
      PageDown: () => setZoom(Math.exp(-0.1)), "-": () => setZoom(Math.exp(-0.1)), _: () => setZoom(Math.exp(-0.1)),
    }[e.key];
    if (!act) return;
    e.preventDefault();
    act();
    rerender();
  });
}

export { initOrbit, setBboxCenter };

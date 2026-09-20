import { $ } from "./dom.js";
import { state, model, round3, eq, setRenderOverride } from "./state.js";
import { renderCode } from "./config.js";
import { controlRefs, refreshVisibility } from "./form.js";
import { scheduleRender, render, endInteraction } from "./render.js";

let bboxCenter = [0, 0, 0];   // model bbox center (from get-info), for cartesian→spherical
const setBboxCenter = (v) => { bboxCenter = v; };
// Invert the plugin's up-based spherical basis (projection.rs) so an explicit
// cartesian camera maps to the azimuth/elevation/distance that reproduce its view.
function cartesianToSpherical(cam, center, up) {
  const sub = (a, b) => [a[0]-b[0], a[1]-b[1], a[2]-b[2]];
  const dot = (a, b) => a[0]*b[0] + a[1]*b[1] + a[2]*b[2];
  const cross = (a, b) => [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
  const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0]/l, a[1]/l, a[2]/l]; };
  const off = sub(cam, center), dist = Math.hypot(off[0], off[1], off[2]);
  if (dist < 1e-6) return null;
  const v = norm(off), u = norm(up);
  const arbitrary = Math.abs(u[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
  const right = norm(cross(u, arbitrary));
  const forward = norm(cross(right, u));
  const el = Math.asin(Math.max(-1, Math.min(1, dot(v, u))));
  const az = Math.atan2(dot(v, forward), dot(v, right));
  return { azimuth: az * 180 / Math.PI, elevation: el * 180 / Math.PI, distance: dist };
}
// Switch to orbit (spherical) mode. Clears any deep-link render override so the
// view stops being frozen, and converts an explicit cartesian camera to
// azimuth/elevation/distance so orbiting continues from exactly where it was.
function ensureSpherical() {
  setRenderOverride(null);
  if (state._cam === "spherical") return;
  const center = eq(state.center, [0, 0, 0]) ? bboxCenter : state.center;   // auto_center → bbox center
  const sph = cartesianToSpherical(state.camera, center, state.up);
  if (sph) {
    state.azimuth = Math.round(sph.azimuth * 10) / 10;
    state.elevation = Math.max(-89, Math.min(89, Math.round(sph.elevation * 10) / 10));
    state.distance = round3(sph.distance);
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.distance?.(state.distance);
  }
  state._cam = "spherical"; controlRefs._cam?.("spherical"); refreshVisibility();
}
(function setupOrbit() {
  const stage = $("stage");
  const pts = new Map();          // active pointers: id → {x, y}
  let pd = 0, pcx = 0, pcy = 0, pa = 0;   // last two-finger distance / centroid / angle while pinching
  const two = () => [...pts.values()];
  const twoDist = () => { const [a, b] = two(); return Math.hypot(a.x - b.x, a.y - b.y); };
  const twoCent = () => { const [a, b] = two(); return [(a.x + b.x) / 2, (a.y + b.y) / 2]; };
  const twoAngle = () => { const [a, b] = two(); return Math.atan2(b.y - a.y, b.x - a.x); };
  const seedPinch = () => { pd = twoDist(); [pcx, pcy] = twoCent(); pa = twoAngle(); };
  // Trackball orbit. Azimuth/elevation cameras have a pole singularity: with a
  // fixed `up`, the screen orientation snaps 180° as the view crosses straight
  // up/down. To make VERTICAL orbit continuous *without* that snap, we carry the
  // camera's own up-vector and rotate it together with the view direction — so
  // `up` is always perpendicular to the view and the basis never degenerates.
  // Vertical drag rolls smoothly over the top (model goes gradually upside down,
  // like a rotisserie); horizontal drag spins about that up.
  const _sub = (a, b) => [a[0]-b[0], a[1]-b[1], a[2]-b[2]];
  const _add = (a, b) => [a[0]+b[0], a[1]+b[1], a[2]+b[2]];
  const _scl = (a, s) => [a[0]*s, a[1]*s, a[2]*s];
  const _dot = (a, b) => a[0]*b[0] + a[1]*b[1] + a[2]*b[2];
  const _crs = (a, b) => [a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0]];
  const _nrm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0]/l, a[1]/l, a[2]/l]; };
  // Rodrigues rotation of v about unit axis k by angle a.
  const _rot = (v, k, a) => {
    const c = Math.cos(a), s = Math.sin(a), d = _dot(k, v), x = _crs(k, v);
    return [v[0]*c + x[0]*s + k[0]*d*(1-c),
            v[1]*c + x[1]*s + k[1]*d*(1-c),
            v[2]*c + x[2]*s + k[2]*d*(1-c)];
  };
  // Orthonormal basis the plugin builds from `up` (must match projection.rs).
  const _basis = (up) => {
    const arb = Math.abs(up[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
    const r = _nrm(_crs(up, arb));
    return { r, f: _nrm(_crs(r, up)) };
  };
  // Apply a trackball drag to a view direction + up (both unit, perpendicular),
  // returning the new pair. Horizontal spins about `up`, vertical about the
  // screen-right axis.
  const _trackball = (dir, up, dx, dy) => {
    // Signs chosen to match the previous feel: drag right → same horizontal
    // spin as before; drag down → camera rises (see more of the top).
    const kH = -dx * 0.5 * Math.PI / 180, kV = dy * 0.5 * Math.PI / 180;
    dir = _rot(dir, up, kH);                    // up is invariant under its own rotation
    const rt = _nrm(_crs(dir, up));             // screen-right
    return { dir: _nrm(_rot(dir, rt, kV)), up: _nrm(_rot(up, rt, kV)) };
  };
  // glTF path — the glTF plugin's camera is a Cartesian (x,y,z) triple; the
  // maquette-side spherical (az/el/dist/zoom) model doesn't exist there. Rotate
  // and scale `state.camera` directly around `state.center` for the same feel.
  const orbitGltfBy = (dx, dy, ptype) => {
    // Any interaction unfreezes a shared-link render override — otherwise the
    // override keeps pushing the original camera into every render and the
    // drag has no visible effect.
    setRenderOverride(null);
    // glTF drives an explicit camera + up, so the trackball runs directly on the
    // offset and up-vector: continuous vertical roll, no pole snap.
    const c = state.center || [0, 0, 0];
    const off = _sub(state.camera, c);
    const dist = Math.hypot(off[0], off[1], off[2]) || 1;
    let dir = _nrm(off);
    let up = _nrm(state.up || [0, 1, 0]);
    let sUp = _sub(up, _scl(dir, _dot(up, dir)));                    // keep up ⊥ view
    if (Math.hypot(sUp[0], sUp[1], sUp[2]) < 1e-4) sUp = _basis(dir).r;
    sUp = _nrm(sUp);
    const t = _trackball(dir, sUp, dx, dy);
    state.up = t.up.map(round3);
    state.camera = _add(c, _scl(t.dir, dist)).map(round3);
    controlRefs.camera?.(state.camera); controlRefs.up?.(state.up);
  };
  const zoomGltfBy = (f) => {
    setRenderOverride(null);
    const c = state.center || [0, 0, 0];
    const off = [state.camera[0] - c[0], state.camera[1] - c[1], state.camera[2] - c[2]];
    // Inverted convention: wheel-up (f>1) shows more detail → move camera IN.
    const s = 1 / f;
    state.camera = [c[0] + off[0]*s, c[1] + off[1]*s, c[2] + off[2]*s].map(round3);
    controlRefs.camera?.(state.camera);
  };

  const setZoom = (f) => {
    if (model._gltf) { zoomGltfBy(f); return; }
    setRenderOverride(null);                       // zooming unfreezes a deep-link render
    state.zoom = Math.max(0.3, Math.min(4, round3(state.zoom * f)));
    controlRefs.zoom?.(state.zoom);
  };
  const orbitBy = (dx, dy, ptype) => {
    if (model._gltf) { orbitGltfBy(dx, dy, ptype); return; }
    // Reconstruct the current view direction from spherical az/el/up.
    const upW = _nrm(state.up || [0, 0, 1]);
    const { r, f } = _basis(upW);
    const az = state.azimuth * Math.PI / 180, el = state.elevation * Math.PI / 180;
    let dir = _nrm(_add(_add(_scl(r, Math.cos(el) * Math.cos(az)),
                             _scl(f, Math.cos(el) * Math.sin(az))),
                        _scl(upW, Math.sin(el))));
    // Screen-up = world-up projected perpendicular to the view (Gram-Schmidt).
    let sUp = _sub(upW, _scl(dir, _dot(upW, dir)));
    if (Math.hypot(sUp[0], sUp[1], sUp[2]) < 1e-4) sUp = _basis(dir).r;  // view ∥ up
    sUp = _nrm(sUp);
    const t = _trackball(dir, sUp, dx, dy);
    // Re-encode: `up` carries the orientation, elevation stays 0 (dir ⊥ up),
    // azimuth is dir's angle in the new up-basis. No pole singularity, no snap.
    const b2 = _basis(t.up);
    state.up = t.up.map(round3);
    state.elevation = 0;
    state.azimuth = Math.round(Math.atan2(_dot(t.dir, b2.f), _dot(t.dir, b2.r)) * 180 / Math.PI * 10) / 10;
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.up?.(state.up);
  };
  // Two-finger twist → roll: spin the view about the axis pointing at the camera
  // (dir), leaving the view direction fixed so the scene rotates in the screen
  // plane. `up` is invariant under the same trackball encoding used by orbit, so
  // we just rotate it about dir. `dTheta` is the finger-twist angle (radians).
  const rollBy = (dTheta) => {
    setRenderOverride(null);
    if (model._gltf) {
      const c = state.center || [0, 0, 0];
      let dir = _nrm(_sub(state.camera, c));
      let up = _nrm(state.up || [0, 1, 0]);
      let sUp = _sub(up, _scl(dir, _dot(up, dir)));
      if (Math.hypot(sUp[0], sUp[1], sUp[2]) < 1e-4) sUp = _basis(dir).r;
      state.up = _nrm(_rot(_nrm(sUp), dir, dTheta)).map(round3);
      controlRefs.up?.(state.up);
      return;
    }
    const upW = _nrm(state.up || [0, 0, 1]);
    const { r, f } = _basis(upW);
    const az = state.azimuth * Math.PI / 180, el = state.elevation * Math.PI / 180;
    const dir = _nrm(_add(_add(_scl(r, Math.cos(el) * Math.cos(az)),
                               _scl(f, Math.cos(el) * Math.sin(az))),
                          _scl(upW, Math.sin(el))));
    let sUp = _sub(upW, _scl(dir, _dot(upW, dir)));
    if (Math.hypot(sUp[0], sUp[1], sUp[2]) < 1e-4) sUp = _basis(dir).r;
    sUp = _nrm(_rot(_nrm(sUp), dir, dTheta));
    const b2 = _basis(sUp);
    state.up = sUp.map(round3);
    state.elevation = 0;
    state.azimuth = Math.round(Math.atan2(_dot(dir, b2.f), _dot(dir, b2.r)) * 180 / Math.PI * 10) / 10;
    controlRefs.azimuth?.(state.azimuth); controlRefs.elevation?.(state.elevation); controlRefs.up?.(state.up);
  };

  stage.addEventListener("pointerdown", (e) => {
    if (e.target.closest("#tools")) return;                 // ignore toolbar clicks
    if (e.pointerType === "mouse" && e.button !== 0) return;
    pts.set(e.pointerId, { x: e.clientX, y: e.clientY });
    try { stage.setPointerCapture(e.pointerId); } catch {}
    stage.classList.add("grabbing");
    // maquette side: convert current cartesian→spherical so orbit updates
    // az/el instead of dropping the user's view. glTF stays cartesian.
    if (!model._gltf) ensureSpherical();
    if (pts.size === 2) seedPinch();                        // enter pinch
  });

  stage.addEventListener("pointermove", (e) => {
    const p = pts.get(e.pointerId);
    if (!p) return;
    if (pts.size >= 2) {                                    // two fingers → orbit + zoom + roll
      p.x = e.clientX; p.y = e.clientY;
      const d = twoDist(), [cx, cy] = twoCent(), ang = twoAngle();
      if (pd > 0 && d > 0) setZoom(d / pd);                 // pinch → zoom
      let dA = ang - pa;                                    // twist → roll (about the view axis)
      if (dA > Math.PI) dA -= 2 * Math.PI; else if (dA < -Math.PI) dA += 2 * Math.PI;
      if (Math.abs(dA) > 1e-4) rollBy(dA);
      orbitBy(cx - pcx, cy - pcy, e.pointerType);           // drag centroid → orbit
      pd = d; pcx = cx; pcy = cy; pa = ang;
    } else {                                                // one pointer → orbit
      orbitBy(e.clientX - p.x, e.clientY - p.y, e.pointerType);
      p.x = e.clientX; p.y = e.clientY;
    }
    scheduleRender();
  });

  const end = (e) => {
    if (!pts.delete(e.pointerId)) return;
    pd = 0;                                                 // re-seed below if a pinch continues
    if (pts.size === 0) {
      stage.classList.remove("grabbing");
      endInteraction();   // clear the settle timer + interacting flag; full render below
      renderCode();
      render();
    }
    else if (pts.size === 2) seedPinch();
  };
  stage.addEventListener("pointerup", end);
  stage.addEventListener("pointercancel", end);

  stage.addEventListener("wheel", (e) => {
    e.preventDefault();
    // Scale the zoom factor by |deltaY| so one mouse-wheel detent (≈100)
    // and one trackpad tick (≈4) both feel proportional. Clamp deltaY —
    // some browsers emit spikes (>1000) on inertial flicks that would
    // otherwise send the camera into orbit. exp(-100 * 0.0011) ≈ 0.9 keeps
    // the historic "wheel = 10 %" feel while making trackpads sane.
    const dy = Math.max(-200, Math.min(200, e.deltaY));
    setZoom(Math.exp(-dy * 0.0011));
    scheduleRender();
  }, { passive: false });

  // Keyboard-only camera controls: arrow keys orbit ±5°, PageUp/Down zoom.
  // Stage is now `tabindex="0"` (set in index.html) so keyboard users can
  // focus it and drive the view without touching the form fields.
  stage.tabIndex = 0;
  stage.addEventListener("keydown", (e) => {
    // Only when the stage itself has focus (not a nested control).
    if (document.activeElement !== stage) return;
    const step = e.shiftKey ? 30 : 10;    // shift = coarse
    let handled = true;
    if (e.key === "ArrowLeft")  orbitBy(-step, 0, "mouse");
    else if (e.key === "ArrowRight") orbitBy( step, 0, "mouse");
    else if (e.key === "ArrowUp")    orbitBy(0, -step, "mouse");
    else if (e.key === "ArrowDown")  orbitBy(0,  step, "mouse");
    else if (e.key === "PageUp"   || e.key === "+" || e.key === "=") setZoom(Math.exp( 0.1));
    else if (e.key === "PageDown" || e.key === "-" || e.key === "_") setZoom(Math.exp(-0.1));
    else handled = false;
    if (handled) { e.preventDefault(); scheduleRender(); }
  });
})();


export { bboxCenter, setBboxCenter, ensureSpherical, cartesianToSpherical };

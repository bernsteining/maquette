// Regression test: `zoom` must actually magnify the projected image.
//
// The bug: `zoom` was folded into the bounding radius (`br / zoom`) before the
// fit computation. For the default *perspective* projection, auto-fit clamps
// the scale to the field of view (`required.max(current)` in src/projection.rs),
// so once a model already fits within the FOV — the common case — the clamp
// discarded the zoom entirely and `zoom` became a silent no-op (even zoom: 99
// changed nothing). The fix applies `zoom` as a direct post-fit multiplier on
// the projection magnification (`ProjectionSetup::magnified`).
//
// Run:  typst compile --root . tests/zoom_projection.typ /dev/null
// A failing `#assert` makes `typst compile` exit non-zero (CI-friendly).

#let mp = plugin("/maquette/maquette.wasm")
#let teapot = read("/examples/data/teapot.obj")

#let svg(cfg) = str(mp.render_obj(bytes(teapot), bytes(json.encode(cfg))))

// Default perspective projection, model under-fills the FOV → this is exactly
// the case the FOV clamp used to swallow. Zooming in MUST change the output.
#let base = (up: (0, 1, 0), ambient: 0.3)
#let z1 = svg(base)
#let z2 = svg((..base, zoom: 2.0))
#assert(
  z1 != z2,
  message: "REGRESSION: `zoom` has no effect under perspective — the auto-fit FOV clamp is discarding it. Apply zoom as a post-fit magnification multiplier (ProjectionSetup::magnified).",
)

// Zooming out must also change the output, and differ from zooming in.
#let zhalf = svg((..base, zoom: 0.5))
#assert(zhalf != z1 and zhalf != z2, message: "REGRESSION: `zoom < 1` has no effect.")

// Omitting `zoom` must equal `zoom: 1.0` (the documented default / no-op).
#assert(z1 == svg((..base, zoom: 1.0)), message: "`zoom: 1.0` must match the default (no zoom key).")

// Determinism: identical inputs must produce identical output.
#assert(z2 == svg((..base, zoom: 2.0)), message: "Non-deterministic render for identical inputs.")

#set page(width: auto, height: auto, margin: 2pt)
All zoom/projection assertions passed.

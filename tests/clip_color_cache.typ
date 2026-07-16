// Regression test: the clip path bakes the model's base `color` into the cut
// cap / clipped vertices, so the preprocessed mesh depends on `color` when clip
// is active. `prep_cache_key` (src/render.rs) MUST mix `color` into the cache
// key in that case — otherwise the first clipped render of a mesh is cached and
// every later render of the same mesh reuses its color, silently ignoring
// `color`. This guards against reintroducing that stale-cache bug.
//
// Run:  typst compile --root . tests/clip_color_cache.typ /dev/null
// A failing `#assert` makes `typst compile` exit non-zero (CI-friendly).

#let mp = plugin("/maquette/maquette.wasm")
#let cube = read("/examples/data/cube.stl", encoding: none)

#let svg(cfg) = str(mp.render_stl(cube, bytes(json.encode(cfg))))

#let clip = (from: "camera", depth: 0.45, hatch: true)
#let cam = (3, 2, 2)

// Same mesh, clip active, two different colors → outputs MUST differ.
// (This is the exact failure mode: a mesh-keyed prep cache would return the
// first color for both.)
#let red = svg((camera: cam, color: "#cc3333", clip: clip))
#let blue = svg((camera: cam, color: "#3333cc", clip: clip))
#assert(
  red != blue,
  message: "REGRESSION: clipped render ignores `color` — prep_cache_key must hash config.color when clip is active (stale preprocessed-mesh cache).",
)

// Sanity: color still varies on the plain (unclipped) path too.
#let red2 = svg((camera: cam, color: "#cc3333"))
#let blue2 = svg((camera: cam, color: "#3333cc"))
#assert(red2 != blue2, message: "REGRESSION: unclipped render ignores `color`.")

// Determinism: identical inputs must produce identical output.
#assert(red == svg((camera: cam, color: "#cc3333", clip: clip)), message: "Non-deterministic render for identical inputs.")

#set page(width: auto, height: auto, margin: 2pt)
All clip/color cache assertions passed.

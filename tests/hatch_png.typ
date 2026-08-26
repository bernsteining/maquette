// Regression test: clip-cap section hatching must render in PNG output, not
// only SVG. PNG has no `<pattern>` fill — the rasterizer overlays anti-aliased
// section lines on the visible cap fragments (PixelBuffer::hatch_triangle,
// wired in render_png). This guards against the hatch silently reverting to a
// plain solid cap in PNG.
//
// Run:  typst compile --root . tests/hatch_png.typ -f pdf /tmp/hatch_png.pdf
// A failing `#assert` makes `typst compile` exit non-zero (CI-friendly).

#let mp = plugin("/crates/maquette/maquette.wasm")
#let cube = read("/examples/data/cube.stl", encoding: none)

#let png(cfg) = array(mp.render_stl(cube, bytes(json.encode(cfg))))

#let cam = (3, 2, 2)
#let base = (camera: cam, color: "#9fb4cc", ambient: 0.35, width: 200, height: 200)
#let clip-plain = (from: "camera", depth: 0.45)
#let hatch(style) = (angle: 45, spacing: 7, width: 0.9, color: "#243040", style: style)
#let clip-hatch = (from: "camera", depth: 0.45, hatch: hatch("lines"))

// Same cut, same colour — the ONLY difference is the hatch. If PNG ignored it,
// these would be byte-identical.
#let plain = png((..base, clip: clip-plain))
#let hatched = png((..base, clip: clip-hatch))
#assert(
  plain != hatched,
  message: "REGRESSION: clip hatching has no effect in PNG output — render_png must overlay section lines on cap fragments (PixelBuffer::hatch_triangle).",
)

// Each hatch style must produce a distinct render.
#let lines = png((..base, clip: (from: "camera", depth: 0.45, hatch: hatch("lines"))))
#let cross = png((..base, clip: (from: "camera", depth: 0.45, hatch: hatch("cross"))))
#let crosses = png((..base, clip: (from: "camera", depth: 0.45, hatch: hatch("crosses"))))
#assert(lines != cross, message: "REGRESSION: hatch `style: cross` renders the same as `lines` in PNG.")
#assert(cross != crosses, message: "REGRESSION: hatch `style: crosses` renders the same as `cross` in PNG.")
#assert(lines != crosses, message: "REGRESSION: hatch `style: crosses` renders the same as `lines` in PNG.")

// Determinism: identical inputs must produce identical output.
#assert(hatched == png((..base, clip: clip-hatch)), message: "Non-deterministic hatched PNG for identical inputs.")

#set page(width: auto, height: auto, margin: 2pt)
PNG hatch assertions passed.

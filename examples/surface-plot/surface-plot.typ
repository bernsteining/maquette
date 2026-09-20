// Pure-Typst 3D surface plots on top of maquette — no Rust, no toolchain.
// Sample z = f(x, y) on a grid, mesh it, colour it by height, and hand the
// mesh to maquette's render-ply as an in-memory PLY. A worked example of the
// "generate a mesh string → render-*" path any Typst package can use.
#import "@local/maquette:0.1.3": render-ply

// viridis-ish 5-stop colormap: t in [0,1] -> (r, g, b) bytes.
#let _cols = ((68, 1, 84), (59, 82, 139), (33, 145, 140), (94, 201, 98), (253, 231, 37))
#let _cmap(t) = {
  let t = calc.max(0.0, calc.min(1.0, t))
  let seg = calc.min(3, int(t * 4))
  let u = t * 4 - seg
  let a = _cols.at(seg)
  let b = _cols.at(seg + 1)
  (0, 1, 2).map(k => calc.round(a.at(k) + (b.at(k) - a.at(k)) * u))
}

#let surface-plot(
  f,
  x-range: (-3, 3),
  y-range: (-3, 3),
  samples: 48,
  z-scale: 1.0,
  ..args,
) = {
  let n = samples
  let (x0, x1) = x-range
  let (y0, y1) = y-range
  let pts = ()
  let zmin = none
  let zmax = none
  for i in range(n) {
    for j in range(n) {
      let x = x0 + (x1 - x0) * i / (n - 1)
      let y = y0 + (y1 - y0) * j / (n - 1)
      let z = f(x, y) * z-scale
      pts.push((x, y, z))
      zmin = if zmin == none { z } else { calc.min(zmin, z) }
      zmax = if zmax == none { z } else { calc.max(zmax, z) }
    }
  }
  let dz = if zmax == zmin { 1.0 } else { zmax - zmin }
  let vlines = pts.map(p => {
    let (r, g, b) = _cmap((p.at(2) - zmin) / dz)
    str(p.at(0)) + " " + str(p.at(1)) + " " + str(p.at(2)) + " " + str(r) + " " + str(g) + " " + str(b)
  })
  let flines = ()
  for i in range(n - 1) {
    for j in range(n - 1) {
      let a = i * n + j
      let b = (i + 1) * n + j
      let c = (i + 1) * n + (j + 1)
      let d = i * n + (j + 1)
      flines.push("3 " + str(a) + " " + str(b) + " " + str(c))
      flines.push("3 " + str(a) + " " + str(c) + " " + str(d))
    }
  }
  let header = (
    "ply", "format ascii 1.0",
    "element vertex " + str(pts.len()),
    "property float x", "property float y", "property float z",
    "property uchar red", "property uchar green", "property uchar blue",
    "element face " + str(flines.len()),
    "property list uchar int vertex_indices",
    "end_header",
  )
  let ply = (header + vlines + flines).join("\n") + "\n"
  // Pass the render config as a POSITIONAL dict so width/height reach the
  // renderer as pixel resolution (named width/height are display lengths and
  // get popped by the wrapper). SSAA ×4 + a real resolution kill the jaggies.
  let cfg = (
    smooth: true, up: (0, 0, 1), azimuth: 40, elevation: 30, background: none,
    width: 1000, height: 1000, antialias: 4,
  ) + args.named()
  render-ply(bytes(ply), cfg)
}

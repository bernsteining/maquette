// maquette-scad — OpenSCAD-flavored procedural geometry for Typst.
//
// Build a solid with the helpers below, then compile it to a mesh with
// `scadypst(...)`, which returns PLY bytes you pass straight to maquette's
// `render-ply`:
//
//   #import "maquette-scad.typ": *
//   #import "../maquette/maquette.typ": render-ply
//   #let part = scadypst(difference(cube(20, center: true), sphere(12, fn: 48)))
//   #render-ply(part, ..)
//
// Like OpenSCAD there are two worlds: 2D shapes (square, circle, polygon, …) and
// 3D solids (cube, sphere, cylinder, …). `linear-extrude`/`rotate-extrude` turn a
// 2D shape into a 3D solid. Transforms and booleans work in either world; hull
// and minkowski are 3D. Use Typst's own `for`/`range`/`calc` to place things
// procedurally.
//
// The heavy CSG kernel lives in maquette-scad.wasm; the maquette renderer is a
// separate wasm. They only exchange the PLY blob — see scad/src/lib.rs.

#let _scad-plugin = plugin("maquette-scad.wasm")

// ---- 3D primitives ----
// `size` may be a single number (uniform) or a 3-array (x, y, z).
#let cube(size, center: false) = {
  let s = if type(size) == array { size } else { (size, size, size) }
  (op: "cube", size: s, center: center)
}
#let sphere(r, fn: none) = (op: "sphere", r: r, ..(if fn != none { (fn: fn) }))
// Straight cylinder (r) or cone/frustum (r1, r2).
#let cylinder(h, r: none, r1: none, r2: none, center: false, fn: none) = (
  op: "cylinder", h: h, center: center,
  ..(if r != none { (r: r) }),
  ..(if r1 != none { (r1: r1) }), ..(if r2 != none { (r2: r2) }),
  ..(if fn != none { (fn: fn) }),
)
// Raw mesh: points = ((x,y,z),..), faces = ((i,j,k,..),..).
#let polyhedron(points, faces) = (op: "polyhedron", points: points, faces: faces)

// ---- 2D primitives ----
#let square(size, center: false) = (op: "square", size: size, center: center)
#let circle(r, fn: none) = (op: "circle", r: r, ..(if fn != none { (fn: fn) }))
#let ellipse(w, h, fn: none) = (op: "ellipse", w: w, h: h, ..(if fn != none { (fn: fn) }))
// `paths` (optional) are index-rings into `points`: first = outer, rest = holes.
#let polygon(points, paths: none) = (
  op: "polygon", points: points, ..(if paths != none { (paths: paths) }),
)
// 3D text. Needs a font: pass `font: read("x.ttf", encoding: none)` to scadypst().
#let scad-text(str, size: 10) = (op: "text", text: str, size: size)
// Import an STL/OBJ mesh. Pass its bytes via scadypst(bin: (name: read(...))).
#let import-mesh(file) = (op: "import", file: file)
#let ngon(sides, r, fn: none) = (op: "ngon", sides: sides, r: r)
#let star(points, outer, inner) = (op: "star", points: points, outer: outer, inner: inner)
#let rounded-square(w, h, r, fn: none) = (
  op: "rounded_square", w: w, h: h, r: r, ..(if fn != none { (fn: fn) }),
)

// ---- 2D -> 3D ----
#let linear-extrude(h, child, center: false, twist: 0, scale: 1, slices: none) = (
  op: "linear_extrude", h: h, center: center, child: child,
  ..(if twist != 0 { (twist: twist) }),
  ..(if scale != 1 { (scale: scale) }),
  ..(if slices != none { (slices: slices) }),
)
// Revolve a 2D profile (in the +x half-plane) around the axis. `angle` degrees.
#let rotate-extrude(child, angle: 360, fn: none) = (
  op: "rotate_extrude", angle: angle, child: child, ..(if fn != none { (fn: fn) }),
)
// Flatten a 3D solid to its 2D shadow on the Z=0 plane.
#let projection(child) = (op: "projection", child: child)
// Horizontal cross-section of a 3D solid at Z = `z`. Returns a 2D shape;
// pipe into `scadypst-svg(..)` for a vector contour, or into further 2D
// ops. Distinct from `projection` (which unions every horizontal slice).
#let slice(child, z: 0) = (op: "slice", child: child, z: z)
// Cut a 3D solid with an arbitrary plane; keep the half where
// dot(pos, normal) >= offset. Cheaper + exact vs the "difference() with
// a giant cube" trick.
#let trim(child, normal, offset: 0) = (op: "trim", child: child, normal: normal, offset: offset)
// Convex hull of a raw 3D point set. Complements `hull()` (which takes
// geometry children); this takes a list of [x, y, z] points and returns
// the 3D hull directly.
#let hull-pts(points) = (op: "hull_pts", points: points)

// Vertex-reducing simplify: collapses edges shorter than `epsilon` (in the
// input's units). Dimension-agnostic — works on both 2D shapes and 3D
// solids. Cheap way to strip micro-detail from booleans or to slim a
// mesh before shipping it through PLY / SVG.
#let simplify(child, epsilon: 0.01) = (op: "simplify", child: child, epsilon: epsilon)

// Attach per-vertex normals to a 3D solid so maquette can smooth-shade
// curved surfaces while keeping crisp edges. Adjacent faces whose dihedral
// angle exceeds `sharp_angle` (degrees) stay sharp; the rest blend. Pipe
// AFTER final booleans/hulls — CSG ops discard the normals. Effect only
// shows under smooth shading (`shading: "smooth"`), not `openscad-view`
// (which is flat by design).
#let calculate-normals(child, sharp_angle: 60) = (
  op: "calculate_normals", child: child, sharp_angle: sharp_angle,
)

// ---- transforms (2D or 3D) ----
#let translate(v, child) = (op: "translate", v: v, child: child)
#let rotate(deg, child) = (op: "rotate", deg: deg, child: child)   // Euler degrees (x, y, z)
#let scale(v, child) = (op: "scale", v: v, child: child)
#let mirror(v, child) = (op: "mirror", v: v, child: child)         // reflect across plane with normal v
#let multmatrix(m, child) = (op: "multmatrix", m: m, child: child) // 4x4 / 4x3 affine matrix
#let resize(v, child) = (op: "resize", v: v, child: child)         // scale a 3D solid to a target bbox
#let offset(d, child) = (op: "offset", d: d, child: child)         // grow/shrink a 2D shape by d
// Color a subtree. `rgb` is a 3-array of 0..1 floats (or a 4-array [r,g,b,a]).
// `alpha` (0..1) makes the subtree translucent — interior features show through
// when rendered. Survives boolean ops.
#let color(rgb, child, alpha: none) = (
  op: "color", rgb: rgb, child: child, ..(if alpha != none { (alpha: alpha) }),
)

// ---- booleans (variadic) ----
#let union(..items) = (op: "union", children: items.pos())
#let difference(..items) = (op: "difference", children: items.pos())      // first minus the rest
#let intersection(..items) = (op: "intersection", children: items.pos())

// ---- hull / minkowski (3D, variadic) ----
#let hull(..items) = (op: "hull", children: items.pos())
#let minkowski(..items) = (op: "minkowski", children: items.pos())

// ---- clash-free aliases ----
// These OpenSCAD names shadow Typst built-ins under `import *`:
//   scale, rotate, circle, square, ellipse, polygon, color  (and text → scad-text).
// The `scad-` aliases below give a non-clashing name so you can keep Typst's too.
// Alternatively, import the whole module namespaced and skip aliases entirely:
//   #import "maquette-scad.typ" as scad   →   scad.scale(..), scad.rotate(..)
#let scad-scale = scale
#let scad-rotate = rotate
#let scad-circle = circle
#let scad-square = square
#let scad-ellipse = ellipse
#let scad-polygon = polygon
#let scad-color = color

// Frame a dict of name -> bytes into one blob: [u32 name_len][name][u32 len][data]…
#let _u32le(n) = bytes((
  calc.rem(n, 256),
  calc.rem(calc.quo(n, 256), 256),
  calc.rem(calc.quo(n, 65536), 256),
  calc.rem(calc.quo(n, 16777216), 256),
))
#let _pack-bin(items) = {
  let out = bytes(())
  for (name, data) in items {
    let nb = bytes(name)
    let db = bytes(data)
    out += _u32le(nb.len()) + nb + _u32le(db.len()) + db
  }
  out
}

// Compile a DSL tree to PLY bytes. `fn` sets the default facet count ($fn).
// `bin`/`font` supply bytes for `import-mesh`/`scad-text` (same as compile-scad).
// `smooth-normals: N` runs Manifold's `calculate_normals(0, N)` on the final
// mesh so it renders smooth-shaded under maquette's smooth shading modes;
// crease edges sharper than N degrees stay crisp. `none` = faceted.
// Feed the result to maquette's `render-ply`.
#let scadypst(node, bin: (:), font: none, fn: 32, smooth-normals: none) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  let opts = (fn: fn)
  if smooth-normals != none { opts = opts + (smooth_normals: smooth-normals) }
  _scad-plugin.build_ply(
    bytes(json.encode(node)), bytes(json.encode(opts)), _pack-bin(assets),
  )
}

// Direct 2D → SVG variant of `scadypst`. Errors if the tree resolves to a
// 3D solid — vector output only makes sense for 2D geometry (circles,
// polygons, extruded-then-projected shapes). The returned bytes are an
// SVG document, hand it straight to Typst's built-in `image()`:
//
//   #image(scadypst-svg(union(circle(5), square(4, center: true))))
//
// Skips maquette entirely — resolution-independent, editable in Inkscape,
// laser-cutter-ready.
#let scadypst-svg(node, bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  _scad-plugin.build_svg(
    bytes(json.encode(node)), bytes(json.encode((fn: fn))), _pack-bin(assets),
  )
}

// Compile REAL OpenSCAD source text (a string, or bytes from `read("x.scad")`)
// to PLY bytes. Supports a substantial subset: primitives, transforms, booleans,
// hull/minkowski, extrudes (incl. twist/scale), `for`/`if`, list comprehensions,
// variables, user modules & functions, closures, and an expression language.
//
// `files` supplies libraries for `use <path>` / `include <path>` — a dict of
// path -> source text. Since the wasm sandbox can't read files, YOU read them
// in Typst and pass them in, e.g.:
//   compile-scad(read("main.scad"), files: (
//     "BOSL2/std.scad": read("BOSL2/std.scad"),
//   ))
//
// `fn` is the default facet count when the source omits `$fn`. Feed the result
// to maquette's `render-ply`.

// `bin` supplies binary assets for `import("x.stl"|.obj|.dxf)` — a dict of
// filename -> bytes (from `read(..., encoding: none)`). `font` is TTF/OTF bytes
// used by `text(...)`. Example:
//   compile-scad(read("main.scad"),
//     bin: ("part.stl": read("part.stl", encoding: none)),
//     font: read("Roboto.ttf", encoding: none))
#let compile-scad(src, files: (:), bin: (:), font: none, fn: 32, trace: none, smooth-normals: none) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  let opts = (fn: fn)
  if trace != none { opts = opts + (trace: trace) }  // debug: stop before the Nth csgrs op
  if smooth-normals != none { opts = opts + (smooth_normals: smooth-normals) }
  _scad-plugin.build_scad(
    bytes(src),
    bytes(json.encode(files)),
    bytes(json.encode(opts)),
    _pack-bin(assets),
  )
}

// Direct 2D `.scad` → SVG variant of `compile-scad`. Errors if the source
// resolves to a 3D solid — SVG output only covers 2D geometry. Same
// files/bin/font/fn contract; returns SVG bytes instead of PLY. Hand the
// result to `image(..)` directly:
//
//   #image(compile-scad-svg(read("gasket.scad")))
#let compile-scad-svg(src, files: (:), bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  _scad-plugin.build_scad_svg(
    bytes(src),
    bytes(json.encode(files)),
    bytes(json.encode((fn: fn))),
    _pack-bin(assets),
  )
}

// Inspection API: returns a dict with the final geometry's stats without
// building the PLY. Fields: bbox_min / bbox_max / center / radius /
// volume / surface_area / num_tri / num_vert / genus. Use it to lay
// parts out by their real size, annotate a document with computed
// volumes, or fail early if a compile produced empty geometry.
//
//   #let info = scadypst-info(mypart)
//   Volume: #info.volume mm³, #info.num_tri triangles.
#let scadypst-info(node, bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  json(_scad-plugin.build_ply_info(
    bytes(json.encode(node)), bytes(json.encode((fn: fn))), _pack-bin(assets),
  ))
}
// Same, for `.scad` sources.
#let compile-scad-info(src, files: (:), bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  json(_scad-plugin.build_scad_info(
    bytes(src),
    bytes(json.encode(files)),
    bytes(json.encode((fn: fn))),
    _pack-bin(assets),
  ))
}

// Decompose the final geometry into its connected components and return
// each as its own PLY. Returns `array<bytes>`; each element is renderable
// via `render-ply` in isolation. Useful for laser-cut sheet layouts or
// per-part annotations. Framing on the wasm side: [u32 n][per-part:
// u32 len, bytes].
#let _unpack-parts(blob) = {
  let b = array(blob)
  let rd32 = i => b.at(i) + b.at(i+1) * 256 + b.at(i+2) * 65536 + b.at(i+3) * 16777216
  let n = rd32(0)
  let out = ()
  let pos = 4
  for _ in range(n) {
    let l = rd32(pos)
    pos += 4
    out.push(bytes(b.slice(pos, pos + l)))
    pos += l
  }
  out
}
#let scadypst-parts(node, bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  _unpack-parts(_scad-plugin.build_ply_parts(
    bytes(json.encode(node)), bytes(json.encode((fn: fn))), _pack-bin(assets),
  ))
}
#let compile-scad-parts(src, files: (:), bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  _unpack-parts(_scad-plugin.build_scad_parts(
    bytes(src),
    bytes(json.encode(files)),
    bytes(json.encode((fn: fn))),
    _pack-bin(assets),
  ))
}

// Ray-vs-mesh intersection: fires a segment from `origin` to `end` at the
// evaluated geometry and returns an array of hit dicts sorted by distance:
//   ( (face_id: <int>, distance: <float>,
//      position: (x, y, z), normal: (x, y, z)), .. )
// Use it to sample terrain height under an XY coordinate, pick the face at
// a screen click, or check line-of-sight between two points. Empty array
// means no intersection along the segment.
#let scadypst-raycast(node, origin, end, bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  json(_scad-plugin.build_ply_raycast(
    bytes(json.encode(node)), bytes(json.encode((fn: fn))), _pack-bin(assets),
    bytes(json.encode((origin: origin, end: end))),
  ))
}
// Same, for `.scad` sources.
#let compile-scad-raycast(src, origin, end, files: (:), bin: (:), font: none, fn: 32) = {
  let assets = bin
  if font != none { assets = assets + ("__font__": font) }
  json(_scad-plugin.build_scad_raycast(
    bytes(src),
    bytes(json.encode(files)),
    bytes(json.encode((fn: fn))),
    _pack-bin(assets),
    bytes(json.encode((origin: origin, end: end))),
  ))
}

// A render-config preset that approximates OpenSCAD's viewport look: the gold
// model (uncolored geometry already defaults to OpenSCAD's #f9d72c), a flat,
// matte, front-lit OpenGL style, and a light grey background. Spread it into
// maquette's render call:  render-ply(model, ..openscad-view, azimuth: 30, …)
#let openscad-view = (
  // Match OpenSCAD's viewport: FLAT (per-face) shading — each facet a uniform
  // tone from its own normal, with NO specular and NO smooth (Gouraud) gradient.
  // This is why coarse ($fn) cylinders show visible facet bands, exactly like
  // OpenSCAD's preview. A moderate ambient + one key + one fill light keep the
  // facets legible without a glossy falloff.
  shading: "flat",
  ambient: 0.5,
  specular: 0.0,
  background: "#e9e9ec",
  // OpenSCAD renders two-sided; disabling back-face culling avoids "holes" where
  // mirror()/negative-scale() flip a part's winding.
  cull_backface: false,
  lights: (
    (type: "directional", vector: (-0.3, -0.4, 1.0), color: "#ffffff", intensity: 0.55),
    (type: "directional", vector: (0.5, 0.6, 0.5), color: "#ffffff", intensity: 0.22),
  ),
)

// Enable OpenSCAD syntax highlighting for ```scad / ```openscad code blocks.
// Apply as a document show rule:
//   #import "maquette-scad.typ": scad-highlighting
//   #show: scad-highlighting
// The grammar path resolves relative to THIS file (openscad.sublime-syntax
// ships alongside it).
#let scad-highlighting(doc) = {
  set raw(syntaxes: "openscad.sublime-syntax")
  doc
}

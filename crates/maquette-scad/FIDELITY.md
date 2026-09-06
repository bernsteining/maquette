# maquette-scad — OpenSCAD language fidelity tracker

Goal: 100% OpenSCAD language coverage in `scad/src/scad.rs` (the evaluator over
`openscad-rs`'s AST). Legend: ✅ full · 🟡 partial · ❌ missing.

Organized after the official [OpenSCAD cheat sheet](https://openscad.org/cheatsheet/).

**CSG kernel: Manifold** (elalish/manifold, via `manifold-csg`). Guarantees
watertight, correctly-triangulated boolean output — replaced csgrs 0.20.1, whose
BSP union/difference left ~20% of faces open on any non-trivial cut. All geometry
this crate emits is now watertight (verified 0 open edges on the full Cyclone).

## Syntax / values
| Feature | Status | Notes |
|---|---|---|
| Comments, numbers, bool, string, `undef` | ✅ | |
| Vectors, ranges | ✅ | |
| Variable assignment (last-wins scope) | ✅ | |
| `let()` expression | ✅ | |
| Member access `.x/.y/.z` (`.r/.g/.b`) | ✅ | |
| Index `v[i]`, string index `s[i]` | ✅ | slicing ❌ |

## Special variables
| Feature | Status | Notes |
|---|---|---|
| `$fn`, `$t`, `$preview`, `$children` | ✅ | |
| `$fa`, `$fs` | 🟡 | defined with OpenSCAD defaults ($fa=12, $fs=2, $fn=0) so libraries that read them (e.g. dotSCAD's `__frags()`) work; our own primitive builder still tessellates from `$fn`/the caller default, not adaptively |
| `$vpr/$vpt/$vpd/$vpf` | ❌ | viewport vars — n/a |

## Operators
| Feature | Status | Notes |
|---|---|---|
| Arithmetic, exponent, comparison, logical, ternary | ✅ | |
| `+`/`-` elementwise (vectors AND matrices, recursive) | ✅ | recurses into nested vectors |
| scalar·vec/matrix, vec/matrix ÷ scalar (recursive) | ✅ | number × matrix, matrix ÷ number work |
| unary `-`/`+` on vectors/matrices | ✅ | negates/copies elementwise |
| vec·vec (dot), matrix·vector, matrix·matrix, row-vec·matrix | ✅ | full `*` linear algebra (dotSCAD's path_extrude/sweep rely on it) |
| `==`/`!=` on any types; `<`/`<=`/`>`/`>=` on strings (lexicographic) | ✅ | |
| `undef`/type-mismatch propagation | ✅ | undef or incompatible operands → undef (no error), matching OpenSCAD |

## 2D primitives
| Feature | Status | Notes |
|---|---|---|
| `circle`, `square`, `polygon` | ✅ | |
| `polygon` paths / holes | ✅ | first ring outer, rest subtracted |
| `text` | ✅ | TTF/OTF outlines via `ttf-parser`, tessellated to polygons. Default font: DejaVu Sans (Latin/Latin-Extended-A subset, ~30 KB) shipped in the wasm; override with `scadypst(font: read("MyFont.ttf", encoding: none))`. `halign`, `valign`, `size`, `spacing` respected. Per-call `font=` name in the .scad source is ignored — only the bytes passed via the wrapper are used. |
| `import` (svg/dxf) | ❌ | not implemented; imports skip gracefully (empty) |

## 3D primitives
| Feature | Status | Notes |
|---|---|---|
| `cube`, `sphere`, `cylinder`, `polyhedron` | ✅ | cone via r1/r2; `convexity` ignored |
| `import` (stl/obj) | ✅ | via `bin:` (bytes) from Typst; parsed by our own STL (bin+ascii) / OBJ reader, welded, fed to Manifold (skips if non-manifold) |
| `surface` (heightmap) | ❌ | |

## Transformations
| Feature | Status | Notes |
|---|---|---|
| `translate`, `rotate`, `scale`, `mirror`, `multmatrix` | ✅ | |
| `resize` | ✅ | bbox scaling; 0 axis = unscaled |
| `color` | ✅ | vector / name / `#hex` / alpha (per-part) |
| `offset` | ✅ | 2D, `r`/`delta` |
| `hull`, `minkowski` | ✅ | |

## Booleans
| `union` / `difference` / `intersection` | ✅ | dimension-aware |

## Extrusion
| Feature | Status | Notes |
|---|---|---|
| `linear_extrude` height/center | ✅ | |
| `linear_extrude` twist/slices/scale | ✅ | built manually (earcut caps + sliced walls) |
| `rotate_extrude` angle/`$fn` | ✅ | on-axis auto-nudge |
| `projection` | ✅ | shadow (flatten to Z=0); `cut` not distinguished |

## Flow control
| Feature | Status | Notes |
|---|---|---|
| `for` (single + cartesian), `intersection_for` | ✅ | |
| `if` / `else` (stmt + expr) | ✅ | |
| List comprehensions `for`/`if`/`let`/`each`, C-style | ✅ | nested + multi-binding |

## Modules & functions
| Feature | Status | Notes |
|---|---|---|
| `module` def + instantiation | ✅ | positional + named + defaults |
| User `function` (incl. recursion) | ✅ | |
| `children()` / `children(i)` / `children([…])` / `$children` | ✅ | |
| Function literals `function(x) …` | ✅ | closures |
| `echo` / `assert` / `assign` | ✅ | echo/assert no-op; assign binds |
| `include <>` / `use <>` | ✅ | via `files:` dict passed from Typst |

## Modifier characters
| Feature | Status | Notes |
|---|---|---|
| `*` disable, `%` background, `#` highlight | ✅ | `%`/`#` → per-part alpha |
| `!` root (show only this) | ❌ | |

## Built-in functions
| Feature | Status | Notes |
|---|---|---|
| Math: abs sin cos tan asin acos atan atan2 floor ceil round ln log exp pow sqrt sign | ✅ | trig in degrees |
| Vector: norm, cross, min/max (varargs or vector) | ✅ | |
| List: len (vec+string), concat, lookup, search | ✅ | search = basic first-match |
| Type: is_undef/is_list/is_num/is_bool/is_string/is_function | ✅ | |
| String: str, chr, ord | ✅ | |
| `rands` | ✅ | deterministic (seeded LCG) |

## Other
| Feature | Status | Notes |
|---|---|---|
| 2D top-level output | ✅ | auto-extruded to a thin plate |
| `render` | 🟡 | treated as plain group |

## Remaining for 100%
- **`surface`** (heightmap → mesh) — not implemented.
- **`import` dxf/svg** — not implemented (imports skip gracefully). stl/obj ✅.
- **`fill`** (2D) — not implemented; experimental **`object()`** — not implemented.
- **`polyhedron` strictness** — Manifold requires a valid 2-manifold; OpenSCAD renders
  some non-manifold "soups" loosely (we return an error/skip instead).
- **C++-abort hardening** — Manifold/Clipper2 throw (→ native abort / wasm trap) on
  extreme 2D coordinates; needs input validation to prevent the trap.
- **Minor:** `$fa`/`$fs` *adaptive* primitive tessellation (defined but our builder
  uses `$fn`), `!` root modifier, string slicing.

## Compliance benchmark (OpenSCAD's own `tests/data/scad` corpus)
- **Parse: 98.9%** (520/526); the 6 misses are intentionally-malformed error tests
  (unterminated comment/string/`use`). On well-formed OpenSCAD, effectively **100%**.
- **Eval: ~79%** pass (219/305 feature/fn/misc tests; geometry-less value tests count
  as pass, external-asset tests skipped). Remaining failures are the unimplemented
  features above, `polyhedron` strictness, C++ aborts, and a few negative tests.
- Recently closed via this benchmark: primitive default sizes (bare `sphere()`/
  `cube()`/… → 1), undefined-variable → `undef`, `let`/`assert`/`echo` as
  statement-modules and in expression position, and 2D `minkowski`.

## Parser note (`openscad-rs`)
`openscad-rs` 0.1.0 silently returns an EMPTY parse (no error) for any file whose
block comment ends in `**/` (a `*` right before `*/`) — extremely common as the
`/** … **/` doc-comment header in real libraries (all of dotSCAD). We work around
it by stripping block comments ourselves (comment/string-aware, newline-preserving)
before parsing — see `strip_block_comments` in `src/scad.rs`.

Everything else — the full language plus stl/obj `import`, `use`/`include`,
twist/scale extrude, full `*` matrix algebra, undef propagation, and WATERTIGHT
booleans — is ✅. Validated end-to-end on the Cyclone-PCB-Factory and dotSCAD's
torus-knot dragon (bezier + path_extrude + sweep).

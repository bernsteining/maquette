//! maquette-scad — a Typst wasm plugin that compiles an OpenSCAD-flavored DSL
//! tree (emitted by `maquette-scad.typ` via `json.encode`, or by the `.scad`
//! evaluator in `src/scad.rs`) into a triangle mesh, serialized as binary PLY.
//! Feed the result to maquette's `render-ply`.
//!
//! Geometry is done by the **Manifold** CSG kernel (elalish/manifold) via the
//! `manifold-csg` safe bindings over its C API. Manifold GUARANTEES watertight,
//! correctly-triangulated boolean output — the reason we moved off csgrs, whose
//! BSP union/difference leaked ~20% open faces on any non-trivial cut. This crate
//! is just the glue: JSON tree -> Manifold `Manifold`/`CrossSection` -> PLY bytes.
//! It is a SEPARATE wasm from the maquette renderer; they only exchange a PLY blob.
//!
//! Like OpenSCAD, the tree mixes a 2D subsystem (cross-sections) and a 3D
//! subsystem (manifolds); `linear_extrude`/`rotate_extrude` bridge 2D -> 3D.
//!
//! Per-region color survives booleans via Manifold's original-ID mechanism: every
//! colored leaf is stamped `as_original()`, its id recorded in [`PALETTE`], and the
//! output mesh's per-run `run_original_id` maps each triangle back to its color.

mod json;
mod scad;

use json::Json;
use manifold_csg::{CrossSection, FillRule, JoinType, Manifold};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_minimal_protocol::*;

initiate_protocol!();

/// Per-region RGBA color. Alpha 255 = opaque; lower = translucent (maquette's PLY
/// `alpha`, used by the `%`/`#` ghost modifiers and `color(..., alpha)`).
#[derive(Clone, Debug, PartialEq)]
struct Rgb([u8; 4]);

// Uncolored geometry defaults to OpenSCAD's signature "Cornfield" gold (#f9d72c),
// so renders read as OpenSCAD out of the box. `color(...)` overrides it.
const DEFAULT_RGB: Rgb = Rgb([249, 215, 44, 255]);

thread_local! {
    /// original-ID -> color, populated by [`register`], read by [`to_ply`].
    static PALETTE: RefCell<FxHashMap<u32, Rgb>> = RefCell::new(FxHashMap::default());
    /// Node address -> structural hash of its subtree (precomputed once per
    /// compile by [`prehash`], read by [`build`] to key the memo cache in O(1)).
    static NODE_HASH: RefCell<FxHashMap<usize, u64>> = RefCell::new(FxHashMap::default());
    /// (subtree hash, inherited color) -> built geometry. Lets a re-instantiated
    /// part (bolt/foot/bearing/…) be built once and reused. Reset per compile.
    static MEMO: RefCell<FxHashMap<(u64, [u8; 4]), Geo>> = RefCell::new(FxHashMap::default());
}

/// Structural (content) hash of a JSON subtree, computed bottom-up in a single
/// O(n) pass and recorded per node address in `map`. Two structurally-identical
/// subtrees (the same part instantiated twice) get the same hash regardless of
/// their address, which is what makes memoization hit across instances.
fn prehash(node: &Json, map: &mut FxHashMap<usize, u64>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match node {
        Json::Null => 0u8.hash(&mut h),
        Json::Bool(b) => {
            1u8.hash(&mut h);
            b.hash(&mut h);
        }
        Json::Num(n) => {
            2u8.hash(&mut h);
            n.to_bits().hash(&mut h);
        }
        Json::Str(s) => {
            3u8.hash(&mut h);
            s.hash(&mut h);
        }
        Json::Arr(a) => {
            4u8.hash(&mut h);
            for e in a {
                prehash(e, map).hash(&mut h);
            }
        }
        Json::Obj(entries) => {
            5u8.hash(&mut h);
            for (k, v) in entries {
                k.hash(&mut h);
                prehash(v, map).hash(&mut h);
            }
        }
    }
    let hv = h.finish();
    map.insert(node as *const Json as usize, hv);
    hv
}

/// Stamp a manifold as a color "original" and record its id -> color, so the
/// color survives all downstream transforms/booleans and can be recovered per
/// triangle in [`to_ply`]. Returns the stamped manifold (use it, not the input).
fn register(m: Manifold, color: &Rgb) -> Manifold {
    let m = m.as_original();
    let id = m.original_id();
    if id >= 0 {
        PALETTE.with(|p| p.borrow_mut().insert(id as u32, color.clone()));
    }
    m
}

/// Global defaults, overridable per-call via the `opts` argument.
struct Defaults {
    seg: usize, // default facet count ($fn)
    /// Binary assets from Typst (name -> bytes) for `import()`.
    bin: HashMap<String, Vec<u8>>,
}

/// Decode the framed binary blob: repeated [u32 name_len][name][u32 data_len][data].
fn parse_bin(blob: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut m = HashMap::new();
    let rd = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
    let mut i = 0;
    while i + 4 <= blob.len() {
        let nl = rd(blob, i);
        i += 4;
        if i + nl + 4 > blob.len() {
            break;
        }
        let name = String::from_utf8_lossy(&blob[i..i + nl]).into_owned();
        i += nl;
        let dl = rd(blob, i);
        i += 4;
        if i + dl > blob.len() {
            break;
        }
        m.insert(name, blob[i..i + dl].to_vec());
        i += dl;
    }
    m
}

fn f2u8(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// A node evaluates to either a 2D cross-section (carrying its color, since a
/// CrossSection has no per-region color of its own) or a 3D manifold (whose color
/// already lives in [`PALETTE`] via [`register`]). OpenSCAD's two worlds.
///
/// `Clone` is O(1): Manifold/CrossSection handles are Rc-backed lazy CSG trees, so
/// the memoization cache in [`build`] can hand out shared results cheaply.
#[derive(Clone)]
enum Geo {
    D2(CrossSection, Rgb),
    D3(Manifold),
}

impl Geo {
    fn into_manifold(self, ctx: &str) -> Result<Manifold, String> {
        match self {
            Geo::D3(m) => Ok(m),
            Geo::D2(..) => Err(format!("{ctx}: expected a 3D solid, got a 2D shape")),
        }
    }
    fn into_cross(self, ctx: &str) -> Result<(CrossSection, Rgb), String> {
        match self {
            Geo::D2(c, col) => Ok((c, col)),
            Geo::D3(_) => Err(format!("{ctx}: expected a 2D shape, got a 3D solid")),
        }
    }
    /// Force to 3D, extruding a 2D result to a thin plate (top-level 2D output).
    fn to_manifold_extruding(self) -> Manifold {
        match self {
            Geo::D3(m) => m,
            Geo::D2(c, col) => register(c.extrude(1.0), &col),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Bop {
    Union,
    Diff,
    Inter,
}

// --- small JSON extraction helpers ---

fn num(node: &Json, key: &str) -> Option<f64> {
    node.get(key).and_then(Json::as_f64)
}
fn vecn(node: &Json, key: &str) -> Option<Vec<f64>> {
    let a = node.get(key)?.as_arr()?;
    a.iter().map(Json::as_f64).collect()
}
/// A vec that may be given as 2 or 3 numbers; missing z defaults to 0.
fn v3(node: &Json, key: &str) -> Option<[f64; 3]> {
    let v = vecn(node, key)?;
    match v.len() {
        2 => Some([v[0], v[1], 0.0]),
        3 => Some([v[0], v[1], v[2]]),
        _ => None,
    }
}
fn seg_of(node: &Json, d: &Defaults) -> usize {
    (num(node, "fn").map(|f| f as usize).unwrap_or(d.seg)).max(3)
}
fn seg_i32(node: &Json, d: &Defaults) -> i32 {
    seg_of(node, d) as i32
}
fn children(node: &Json) -> Result<&[Json], String> {
    node.get("children")
        .and_then(Json::as_arr)
        .ok_or_else(|| "boolean/hull/minkowski: missing children[]".into())
}
fn child_of(node: &Json) -> Result<&Json, String> {
    node.get("child").ok_or_else(|| "node: missing \"child\"".into())
}

// --- validation helpers ---

/// Distance below which a revolve profile vertex counts as "on the axis".
const AXIS_EPS: f64 = 1e-4;

fn req(node: &Json, key: &str, ctx: &str) -> Result<f64, String> {
    let x = num(node, key).ok_or_else(|| format!("{ctx}: missing number \"{key}\""))?;
    if x.is_finite() {
        Ok(x)
    } else {
        Err(format!("{ctx}: \"{key}\" must be a finite number"))
    }
}
fn req_pos(node: &Json, key: &str, ctx: &str) -> Result<f64, String> {
    let x = req(node, key, ctx)?;
    if x > 0.0 {
        Ok(x)
    } else {
        Err(format!("{ctx}: \"{key}\" must be > 0 (got {x})"))
    }
}
fn finite_all(v: &[f64], ctx: &str) -> Result<(), String> {
    if v.iter().all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(format!("{ctx}: vector has a non-finite (NaN/inf) component"))
    }
}
/// Read a `points: [[x,y],…]` list, validating finiteness and a >=3 count.
fn polygon_points(node: &Json, ctx: &str) -> Result<Vec<[f64; 2]>, String> {
    let pts_j = node
        .get("points")
        .and_then(Json::as_arr)
        .ok_or_else(|| format!("{ctx}: missing points[]"))?;
    let mut pts: Vec<[f64; 2]> = Vec::with_capacity(pts_j.len());
    for p in pts_j {
        let a = p
            .as_arr()
            .and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]))
            .ok_or_else(|| format!("{ctx}: each point needs [x, y]"))?;
        finite_all(&a, ctx)?;
        pts.push(a);
    }
    if pts.len() < 3 {
        return Err(format!("{ctx}: needs at least 3 points (got {})", pts.len()));
    }
    Ok(pts)
}

/// Drop consecutive (and wrap-around) near-duplicate ring vertices.
fn sanitize_ring(pts: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let close = |a: &[f64; 2], b: &[f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) <= 1e-7;
    let mut out: Vec<[f64; 2]> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last().is_none_or(|q| !close(q, &p)) {
            out.push(p);
        }
    }
    if out.len() >= 2 && close(&out[0], out.last().unwrap()) {
        out.pop();
    }
    out
}

/// Build the 2D profile fed to `revolve`, nudged off the rotation axis (x=0):
/// a profile touching/crossing x<AXIS_EPS is invalid for a revolve, so shift the
/// whole cross-section into x>0 (a hole of radius AXIS_EPS — visually nothing).
fn build_revolve_profile(node: &Json, color: Rgb, d: &Defaults) -> Result<CrossSection, String> {
    let (cs, _col) = build(node, color, d)?.into_cross("rotate_extrude")?;
    let min_x = cs.bounds().min()[0];
    if min_x < AXIS_EPS {
        Ok(cs.translate(AXIS_EPS - min_x, 0.0))
    } else {
        Ok(cs)
    }
}

/// Memoizing entry to [`build_uncached`]: caches results by (subtree structural
/// hash, inherited color) so a part re-instantiated many times in an assembly is
/// built once and cheaply cloned thereafter (Manifold handles are Rc-backed).
/// Color is part of the key, so shared solids keep the correct per-region color.
/// Falls back to an uncached build if a node was somehow not pre-hashed.
fn build(node: &Json, color: Rgb, d: &Defaults) -> Result<Geo, String> {
    let hk = NODE_HASH.with(|m| m.borrow().get(&(node as *const Json as usize)).copied());
    let Some(hk) = hk else {
        return build_uncached(node, color, d);
    };
    let ck = (hk, color.0);
    if let Some(g) = MEMO.with(|m| m.borrow().get(&ck).cloned()) {
        return Ok(g);
    }
    let g = build_uncached(node, color, d)?;
    MEMO.with(|m| m.borrow_mut().insert(ck, g.clone()));
    Ok(g)
}

/// Recursively build geometry from a DSL node. `color` is inherited from the
/// nearest enclosing `color(...)`.
fn build_uncached(node: &Json, color: Rgb, d: &Defaults) -> Result<Geo, String> {
    let op = node.get("op").and_then(Json::as_str).ok_or("node: missing \"op\"")?;
    match op {
        // ---- 3D primitives ----
        "cube" => {
            let s = v3(node, "size").ok_or("cube: size")?;
            finite_all(&s, "cube")?;
            // OpenSCAD tolerates negative sizes (box spans into -axis) and treats
            // a zero size as empty. Build from |size| and offset accordingly.
            let ax = [s[0].abs(), s[1].abs(), s[2].abs()];
            if ax.iter().any(|&x| x < 1e-9) {
                return Ok(Geo::D3(Manifold::empty()));
            }
            let m = Manifold::cube(ax[0], ax[1], ax[2], false);
            let centered = node.get("center").and_then(Json::as_bool).unwrap_or(false);
            let m = if centered {
                m.translate(-ax[0] / 2.0, -ax[1] / 2.0, -ax[2] / 2.0)
            } else {
                m.translate(s[0].min(0.0), s[1].min(0.0), s[2].min(0.0))
            };
            Ok(Geo::D3(register(m, &color)))
        }
        "sphere" => {
            let r = req(node, "r", "sphere")?.abs();
            if r < 1e-9 {
                return Ok(Geo::D3(Manifold::empty()));
            }
            Ok(Geo::D3(register(Manifold::sphere(r, seg_i32(node, d)), &color)))
        }
        "cylinder" => {
            // OpenSCAD tolerates negative h (spans -z) and treats degenerate as
            // empty; radii are taken as |r|. r1=bottom, r2=top (cone/frustum).
            let h = req(node, "h", "cylinder")?;
            let ah = h.abs();
            let seg = seg_i32(node, d);
            if ah < 1e-9 {
                return Ok(Geo::D3(Manifold::empty()));
            }
            let (r1, r2) = if node.get("r1").is_some() || node.get("r2").is_some() {
                (req(node, "r1", "cylinder")?.abs(), req(node, "r2", "cylinder")?.abs())
            } else {
                let r = req(node, "r", "cylinder")?.abs();
                (r, r)
            };
            if r1 + r2 < 1e-9 {
                return Ok(Geo::D3(Manifold::empty()));
            }
            let m = Manifold::cylinder(ah, r1, r2, seg, false); // sits 0..ah on Z
            let centered = node.get("center").and_then(Json::as_bool).unwrap_or(false);
            let m = if centered {
                m.translate(0.0, 0.0, -ah / 2.0)
            } else if h < 0.0 {
                m.translate(0.0, 0.0, h) // negative height → span into -z
            } else {
                m
            };
            Ok(Geo::D3(register(m, &color)))
        }
        "polyhedron" => {
            let pts_j = node.get("points").and_then(Json::as_arr).ok_or("polyhedron: points")?;
            let mut verts: Vec<f64> = Vec::with_capacity(pts_j.len() * 3);
            for p in pts_j {
                let a = p.as_arr().and_then(|a| {
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
                });
                let a = a.ok_or("polyhedron: each point needs [x,y,z]")?;
                verts.extend_from_slice(&a);
            }
            let faces_j = node.get("faces").and_then(Json::as_arr).ok_or("polyhedron: faces")?;
            let mut tris: Vec<u64> = Vec::new();
            for f in faces_j {
                let idx = f.as_arr().ok_or("polyhedron: each face is a list of indices")?;
                let mut face = Vec::with_capacity(idx.len());
                for i in idx {
                    face.push(i.as_f64().ok_or("polyhedron: index must be a number")? as u64);
                }
                // Fan-triangulate the (assumed convex, planar) face.
                for k in 1..face.len().saturating_sub(1) {
                    tris.extend_from_slice(&[face[0], face[k], face[k + 1]]);
                }
            }
            let m = mesh_from_flat(&verts, &tris)
                .ok_or("polyhedron: not a valid manifold mesh (check face winding/closure)")?;
            Ok(Geo::D3(register(m, &color)))
        }

        // ---- 2D primitives ----
        "square" => {
            let (w, h) = square_dims(node);
            if w.abs() < 1e-9 || h.abs() < 1e-9 {
                return Ok(Geo::D2(CrossSection::empty(), color));
            }
            let centered = node.get("center").and_then(Json::as_bool).unwrap_or(false);
            Ok(Geo::D2(CrossSection::square(w.abs(), h.abs(), centered), color))
        }
        "circle" => {
            let r = req(node, "r", "circle")?.abs();
            if r < 1e-9 {
                return Ok(Geo::D2(CrossSection::empty(), color));
            }
            Ok(Geo::D2(CrossSection::circle(r, seg_i32(node, d)), color))
        }
        "ellipse" => {
            let w = req_pos(node, "w", "ellipse")?;
            let h = req_pos(node, "h", "ellipse")?;
            // Unit circle (diameter 2) scaled to the requested full width/height.
            let cs = CrossSection::circle(1.0, seg_i32(node, d)).scale(w * 0.5, h * 0.5);
            Ok(Geo::D2(cs, color))
        }
        "polygon" => {
            let pts = polygon_points(node, "polygon")?;
            if let Some(paths) = node.get("paths").and_then(Json::as_arr) {
                if paths.is_empty() {
                    return Err("polygon: empty paths".into());
                }
                let ring = |idxs: &Json| -> Result<Vec<[f64; 2]>, String> {
                    let ids = idxs.as_arr().ok_or("polygon: each path is a list of indices")?;
                    let mut r = Vec::with_capacity(ids.len());
                    for id in ids {
                        let i = id.as_f64().ok_or("polygon: path index must be a number")? as usize;
                        r.push(*pts.get(i).ok_or("polygon: path index out of range")?);
                    }
                    Ok(sanitize_ring(&r))
                };
                // All rings via EvenOdd: outer filled, nested rings become holes —
                // matching OpenSCAD's `paths` (first = outer, rest = holes) and
                // winding-agnostic.
                let mut rings: Vec<Vec<[f64; 2]>> = Vec::with_capacity(paths.len());
                for path in paths {
                    let r = ring(path)?;
                    if r.len() >= 3 {
                        rings.push(r);
                    }
                }
                if rings.is_empty() {
                    return Ok(Geo::D2(CrossSection::empty(), color));
                }
                Ok(Geo::D2(CrossSection::from_polygons_with_fill_rule(&rings, FillRule::EvenOdd), color))
            } else {
                let clean = sanitize_ring(&pts);
                if clean.len() < 3 {
                    return Ok(Geo::D2(CrossSection::empty(), color));
                }
                Ok(Geo::D2(CrossSection::from_polygons_with_fill_rule(&[clean], FillRule::EvenOdd), color))
            }
        }
        "ngon" => {
            let sides = req(node, "sides", "ngon")?;
            if sides < 3.0 {
                return Err(format!("ngon: sides must be >= 3 (got {sides})"));
            }
            let r = req_pos(node, "r", "ngon")?;
            // A circle with exactly `sides` segments IS a regular n-gon.
            Ok(Geo::D2(CrossSection::circle(r, sides as i32), color))
        }
        "star" => {
            let n = req(node, "points", "star")?;
            if n < 2.0 {
                return Err(format!("star: points must be >= 2 (got {n})"));
            }
            let n = n as usize;
            let ro = req_pos(node, "outer", "star")?;
            let ri = req_pos(node, "inner", "star")?;
            let mut ring = Vec::with_capacity(2 * n);
            for i in 0..2 * n {
                let ang = std::f64::consts::PI * (i as f64) / (n as f64);
                let rad = if i % 2 == 0 { ro } else { ri };
                ring.push([rad * ang.cos(), rad * ang.sin()]);
            }
            Ok(Geo::D2(CrossSection::from_polygons_with_fill_rule(&[ring], FillRule::EvenOdd), color))
        }
        "rounded_square" => {
            let w = req_pos(node, "w", "rounded_square")?;
            let h = req_pos(node, "h", "rounded_square")?;
            let r = req_pos(node, "r", "rounded_square")?;
            let seg = seg_i32(node, d);
            // Inset core rect grown back out by r with round joins → rounded rect
            // occupying 0..w × 0..h (matching un-centered square()).
            let iw = (w - 2.0 * r).max(1e-6);
            let ih = (h - 2.0 * r).max(1e-6);
            let core = CrossSection::square(iw, ih, false).translate(r, r);
            Ok(Geo::D2(core.offset(r, JoinType::Round, 2.0, seg), color))
        }
        "import" => {
            let file = node.get("file").and_then(Json::as_str).ok_or("import: file")?;
            let lower = file.to_ascii_lowercase();
            let is3d = [".stl", ".obj", ".off", ".3mf"].iter().any(|e| lower.ends_with(e));
            // Missing asset or unsupported format (svg/dxf/…) → skip (empty) rather
            // than abort a whole assembly for one import; empty of the right dim.
            let empty = || {
                if is3d {
                    Geo::D3(Manifold::empty())
                } else {
                    Geo::D2(CrossSection::empty(), color.clone())
                }
            };
            let data = match d.bin.get(file) {
                Some(d) if is3d => d,
                _ => return Ok(empty()),
            };
            let parsed = if lower.ends_with(".stl") {
                parse_stl(data)
            } else if lower.ends_with(".obj") {
                parse_obj(data)
            } else {
                None // .off/.3mf unsupported → skip
            };
            match parsed.and_then(|(v, t)| mesh_from_flat(&v, &t)) {
                Some(m) => Ok(Geo::D3(register(m, &color))),
                None => Ok(empty()), // unparseable / non-manifold → skip
            }
        }
        "text" => {
            // Manifold has no font engine (csgrs' truetype-text feature is gone).
            // Emit empty rather than abort; text() is cosmetic. Documented in FIDELITY.md.
            let _ = node.get("text");
            Ok(Geo::D2(CrossSection::empty(), color))
        }

        // ---- 2D -> 3D bridges ----
        "linear_extrude" => {
            let h = req_pos(node, "h", "linear_extrude")?;
            let twist = num(node, "twist").unwrap_or(0.0);
            let scale = num(node, "scale").unwrap_or(1.0);
            let (cs, ccol) = build(child_of(node)?, color.clone(), d)?.into_cross("linear_extrude")?;
            let m = if twist.abs() < 1e-9 && (scale - 1.0).abs() < 1e-9 {
                cs.extrude(h)
            } else {
                let slices = node
                    .get("slices")
                    .and_then(Json::as_f64)
                    .map(|s| s as i32)
                    .unwrap_or_else(|| {
                        (((twist.abs() / 15.0).ceil() as i32).max(seg_i32(node, d) / 2)).max(2)
                    });
                // OpenSCAD twists CCW-positive; Manifold twists CW-positive → negate.
                Manifold::extrude_with_options(&cs, h, slices, -twist, scale, scale)
            };
            let centered = node.get("center").and_then(Json::as_bool).unwrap_or(false);
            let m = if centered { m.translate(0.0, 0.0, -h / 2.0) } else { m };
            Ok(Geo::D3(register(m, &ccol)))
        }
        "rotate_extrude" => {
            let angle = num(node, "angle").unwrap_or(360.0);
            if !angle.is_finite() || angle == 0.0 {
                return Err("rotate_extrude: angle must be a non-zero finite number".into());
            }
            let seg = seg_i32(node, d);
            let cs = build_revolve_profile(child_of(node)?, color.clone(), d)?;
            // Manifold's revolve already matches OpenSCAD: the +x half of the XY
            // profile is spun around the Z axis (profile Y → height Z).
            let m = Manifold::revolve(&cs, seg, angle);
            Ok(Geo::D3(register(m, &color)))
        }
        "projection" => {
            // 3D → 2D shadow onto the Z=0 plane (union of all cross-sections).
            let mesh = build(child_of(node)?, color.clone(), d)?.into_manifold("projection")?;
            let polys = mesh.project();
            let cs = CrossSection::from_polygons_with_fill_rule(&polys, FillRule::NonZero);
            Ok(Geo::D2(cs, color))
        }

        // ---- transforms (dimension-agnostic) ----
        "translate" => {
            let v = v3(node, "v").ok_or("translate: v")?;
            finite_all(&v, "translate")?;
            Ok(match build(child_of(node)?, color, d)? {
                Geo::D3(m) => Geo::D3(m.translate(v[0], v[1], v[2])),
                Geo::D2(c, col) => Geo::D2(c.translate(v[0], v[1]), col),
            })
        }
        "rotate" => {
            let a = v3(node, "deg").ok_or("rotate: deg")?;
            finite_all(&a, "rotate")?;
            Ok(match build(child_of(node)?, color, d)? {
                Geo::D3(m) => Geo::D3(m.rotate(a[0], a[1], a[2])),
                Geo::D2(c, col) => Geo::D2(c.rotate(a[2]), col), // 2D rotates about Z
            })
        }
        "scale" => {
            let v = v3(node, "v").ok_or("scale: v")?;
            finite_all(&v, "scale")?;
            if v.contains(&0.0) {
                return Err(format!("scale: components must be non-zero (got {v:?})"));
            }
            Ok(match build(child_of(node)?, color, d)? {
                Geo::D3(m) => Geo::D3(m.scale(v[0], v[1], v[2])),
                Geo::D2(c, col) => Geo::D2(c.scale(v[0], v[1]), col),
            })
        }
        "mirror" => {
            let v = v3(node, "v").ok_or("mirror: v")?;
            finite_all(&v, "mirror")?;
            if v.iter().all(|&x| x == 0.0) {
                return Err("mirror: normal vector cannot be all zeros".into());
            }
            Ok(match build(child_of(node)?, color, d)? {
                Geo::D3(m) => Geo::D3(m.mirror([v[0], v[1], v[2]])),
                Geo::D2(c, col) => Geo::D2(c.mirror(v[0], v[1]), col),
            })
        }
        "multmatrix" => {
            // 4x4 (or 4x3) row-major affine matrix; missing entries = identity.
            let rows = node.get("m").and_then(Json::as_arr).ok_or("multmatrix: m")?;
            let mut m = [[0f64; 4]; 4];
            for (i, mi) in m.iter_mut().enumerate() {
                mi[i] = 1.0;
            }
            for (i, row) in rows.iter().take(4).enumerate() {
                if let Some(r) = row.as_arr() {
                    for (j, val) in r.iter().take(4).enumerate() {
                        m[i][j] = val.as_f64().ok_or("multmatrix: entries must be numbers")?;
                    }
                }
            }
            Ok(match build(child_of(node)?, color, d)? {
                // Manifold transform is column-major 3x4: [col0, col1, col2, translation].
                Geo::D3(me) => Geo::D3(me.transform(&[
                    m[0][0], m[1][0], m[2][0], //
                    m[0][1], m[1][1], m[2][1], //
                    m[0][2], m[1][2], m[2][2], //
                    m[0][3], m[1][3], m[2][3],
                ])),
                // CrossSection transform is column-major 2x3: [col0, col1, translation].
                Geo::D2(c, col) => Geo::D2(
                    c.transform(&[m[0][0], m[1][0], m[0][1], m[1][1], m[0][3], m[1][3]]),
                    col,
                ),
            })
        }
        "resize" => {
            // Scale a 3D solid so its bounding box matches the target; a 0
            // component leaves that axis unscaled.
            let target = v3(node, "v").ok_or("resize: v")?;
            let mesh = build(child_of(node)?, color.clone(), d)?.into_manifold("resize")?;
            let (mn, mx) = match mesh.bounding_box() {
                Some(bb) => (bb.min(), bb.max()),
                None => return Ok(Geo::D3(mesh)),
            };
            let mut f = [1.0; 3];
            for k in 0..3 {
                let size = mx[k] - mn[k];
                if target[k] > 0.0 && size > 1e-9 {
                    f[k] = target[k] / size;
                }
            }
            Ok(Geo::D3(mesh.scale(f[0], f[1], f[2])))
        }
        "offset" => {
            let dist = num(node, "d").ok_or("offset: d")?;
            let (cs, col) = build(child_of(node)?, color, d)?.into_cross("offset")?;
            // OpenSCAD offset(r=..) uses rounded joins; approximate with Round.
            Ok(Geo::D2(cs.offset(dist, JoinType::Round, 2.0, seg_i32(node, d)), col))
        }
        "color" => {
            let c = vecn(node, "rgb").ok_or("color: rgb")?;
            if c.len() < 3 {
                return Err("color: rgb needs 3 or 4 components".into());
            }
            finite_all(&c, "color")?;
            let a = if let Some(av) = num(node, "alpha") {
                f2u8(av)
            } else if c.len() >= 4 {
                f2u8(c[3])
            } else {
                255
            };
            // color() replaces the inherited color for its subtree (inner wins).
            build(child_of(node)?, Rgb([f2u8(c[0]), f2u8(c[1]), f2u8(c[2]), a]), d)
        }

        // ---- booleans ----
        "union" => boolean(node, color, d, Bop::Union),
        "difference" => boolean(node, color, d, Bop::Diff),
        "intersection" => boolean(node, color, d, Bop::Inter),

        // ---- hull / minkowski ----
        "hull" => {
            let cs = children(node)?;
            let first = cs.first().ok_or("hull: needs >=1 child")?;
            match build(first, color.clone(), d)? {
                Geo::D3(m0) => {
                    let mut ms = vec![m0];
                    for c in &cs[1..] {
                        ms.push(build(c, color.clone(), d)?.into_manifold("hull")?);
                    }
                    Ok(Geo::D3(register(Manifold::batch_hull(&ms), &color)))
                }
                Geo::D2(c0, col) => {
                    let mut ss = vec![c0];
                    for c in &cs[1..] {
                        ss.push(build(c, color.clone(), d)?.into_cross("hull")?.0);
                    }
                    Ok(Geo::D2(CrossSection::batch_hull(&ss), col))
                }
            }
        }
        "minkowski" => {
            let cs = children(node)?;
            let mut it = cs.iter();
            let first = it.next().ok_or("minkowski: needs >=1 child")?;
            match build(first, color.clone(), d)? {
                Geo::D3(mut acc) => {
                    for c in it {
                        let m = build(c, color.clone(), d)?.into_manifold("minkowski")?;
                        acc = acc.minkowski_sum(&m);
                    }
                    Ok(Geo::D3(register(acc, &color)))
                }
                // Manifold has no native 2D minkowski, so compute it via thin-slab
                // 3D minkowski then project back to the plane — correct for
                // arbitrary (incl. non-convex) 2D operands.
                Geo::D2(c0, col) => {
                    let mut acc = c0.extrude(1.0);
                    for c in it {
                        let (s, _) = build(c, color.clone(), d)?.into_cross("minkowski")?;
                        acc = acc.minkowski_sum(&s.extrude(1.0));
                    }
                    let polys = acc.project();
                    Ok(Geo::D2(CrossSection::from_polygons_with_fill_rule(&polys, FillRule::NonZero), col))
                }
            }
        }

        other => Err(format!("node: unknown op \"{other}\"")),
    }
}

fn square_dims(node: &Json) -> (f64, f64) {
    if let Some(v) = vecn(node, "size") {
        if v.len() == 2 {
            return (v[0], v[1]);
        }
    }
    let s = num(node, "size").unwrap_or(1.0);
    (s, s)
}

/// Fold a boolean op over children (OpenSCAD semantics: union of all;
/// difference = first minus each subsequent; intersection of all). Works in
/// whichever dimension the first child is; children must all match it.
///
/// Empty operands are handled algebraically rather than passed to the kernel: a
/// Manifold `union` with an empty operand returns EMPTY (poisoning a fold that
/// includes any `if`-skipped or degenerate child — very common in real assemblies
/// like the Cyclone), so we skip empties on union/difference and short-circuit
/// intersection.
fn boolean(node: &Json, color: Rgb, d: &Defaults, op: Bop) -> Result<Geo, String> {
    let cs = children(node)?;
    let mut it = cs.iter();
    let first = it.next().ok_or("boolean: needs >=1 child")?;
    match build(first, color.clone(), d)? {
        Geo::D3(first_m) => {
            // Build all 3D operands, then batch. Manifold's batch_union/
            // batch_difference build a balanced, lazily-evaluated CSG tree and
            // evaluate once — faster than a sequential fold for the many-child
            // unions in real assemblies. Empty operands are dropped first: a
            // union/difference with ∅ would otherwise poison the whole result.
            let mut ms = vec![first_m];
            for c in it {
                ms.push(build(c, color.clone(), d)?.into_manifold("boolean")?);
            }
            let acc = match op {
                Bop::Union => {
                    let ne: Vec<Manifold> = ms.into_iter().filter(|m| !m.is_empty()).collect();
                    match ne.len() {
                        0 => Manifold::empty(),
                        1 => ne.into_iter().next().unwrap(),
                        _ => Manifold::batch_union(&ne),
                    }
                }
                Bop::Diff => {
                    let mut it = ms.into_iter();
                    let head = it.next().unwrap();
                    if head.is_empty() {
                        head // ∅ - anything = ∅
                    } else {
                        let mut all = vec![head];
                        all.extend(it.filter(|m| !m.is_empty()));
                        if all.len() == 1 { all.pop().unwrap() } else { Manifold::batch_difference(&all) }
                    }
                }
                Bop::Inter => {
                    // No batch intersection in Manifold; fold with an ∅ short-circuit.
                    let mut acc: Option<Manifold> = None;
                    for m in ms {
                        if m.is_empty() {
                            acc = Some(Manifold::empty());
                            break;
                        }
                        acc = Some(match acc {
                            Some(a) => a.intersection(&m),
                            None => m,
                        });
                    }
                    acc.unwrap_or_else(Manifold::empty)
                }
            };
            Ok(Geo::D3(acc))
        }
        Geo::D2(mut acc, col) => {
            for c in it {
                let (s, _) = build(c, color.clone(), d)?.into_cross("boolean")?;
                acc = match op {
                    Bop::Union => {
                        if s.is_empty() {
                            acc
                        } else if acc.is_empty() {
                            s
                        } else {
                            acc.union(&s)
                        }
                    }
                    Bop::Diff => {
                        if s.is_empty() || acc.is_empty() {
                            acc
                        } else {
                            acc.difference(&s)
                        }
                    }
                    Bop::Inter => {
                        if s.is_empty() || acc.is_empty() {
                            CrossSection::empty()
                        } else {
                            acc.intersection(&s)
                        }
                    }
                };
            }
            Ok(Geo::D2(acc, col))
        }
    }
}

// --- mesh asset ingestion (import) ---

/// Weld a triangle soup (flat xyz verts, flat u64 tri indices into it) into a
/// Manifold, returning None if the result isn't a valid 2-manifold. Coincident
/// vertices are merged by quantized position so STL soups become watertight.
fn mesh_from_flat(verts: &[f64], tris: &[u64]) -> Option<Manifold> {
    if verts.len() < 9 || tris.len() < 3 {
        return None;
    }
    let m = Manifold::from_mesh_f64(verts, 3, tris).ok()?;
    m.status().ok().map(|_| m)
}

/// Weld a raw triangle soup (each 3 points = 1 triangle) into indexed form.
fn weld(soup: &[[f64; 3]]) -> (Vec<f64>, Vec<u64>) {
    let mut map: HashMap<[i64; 3], u64> = HashMap::new();
    let mut verts: Vec<f64> = Vec::new();
    let mut tris: Vec<u64> = Vec::with_capacity(soup.len());
    let q = |x: f64| (x * 1e6).round() as i64;
    for p in soup {
        let key = [q(p[0]), q(p[1]), q(p[2])];
        let idx = *map.entry(key).or_insert_with(|| {
            let i = (verts.len() / 3) as u64;
            verts.extend_from_slice(p);
            i
        });
        tris.push(idx);
    }
    (verts, tris)
}

/// Parse binary or ASCII STL into welded (verts, tri-indices).
fn parse_stl(data: &[u8]) -> Option<(Vec<f64>, Vec<u64>)> {
    // ASCII STL starts with "solid" and contains "facet"; binary is 84+ bytes.
    let looks_ascii = data.starts_with(b"solid")
        && data.windows(5).take(512).any(|w| w == b"facet");
    let mut soup: Vec<[f64; 3]> = Vec::new();
    if looks_ascii {
        let text = std::str::from_utf8(data).ok()?;
        for line in text.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("vertex ") {
                let mut it = rest.split_whitespace().filter_map(|t| t.parse::<f64>().ok());
                let p = [it.next()?, it.next()?, it.next()?];
                soup.push(p);
            }
        }
    } else {
        if data.len() < 84 {
            return None;
        }
        let n = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
        let mut off = 84;
        for _ in 0..n {
            if off + 50 > data.len() {
                break;
            }
            // skip 12-byte normal; read 3 vertices (9 f32)
            let rd = |o: usize| f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]) as f64;
            for k in 0..3 {
                let b = off + 12 + k * 12;
                soup.push([rd(b), rd(b + 4), rd(b + 8)]);
            }
            off += 50;
        }
    }
    if soup.len() < 3 {
        return None;
    }
    Some(weld(&soup))
}

/// Parse a minimal Wavefront OBJ (v / f, faces fan-triangulated, 1-based indices
/// with optional v/vt/vn) into (verts, tri-indices).
fn parse_obj(data: &[u8]) -> Option<(Vec<f64>, Vec<u64>)> {
    let text = std::str::from_utf8(data).ok()?;
    let mut verts: Vec<f64> = Vec::new();
    let mut nverts: usize = 0;
    let mut tris: Vec<u64> = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let x = it.next()?.parse::<f64>().ok()?;
                let y = it.next()?.parse::<f64>().ok()?;
                let z = it.next()?.parse::<f64>().ok()?;
                verts.extend_from_slice(&[x, y, z]);
                nverts += 1;
            }
            Some("f") => {
                let idx: Vec<u64> = it
                    .filter_map(|tok| tok.split('/').next())
                    .filter_map(|s| s.parse::<i64>().ok())
                    .map(|i| if i < 0 { (nverts as i64 + i) as u64 } else { (i - 1) as u64 })
                    .collect();
                for k in 1..idx.len().saturating_sub(1) {
                    tris.extend_from_slice(&[idx[0], idx[k], idx[k + 1]]);
                }
            }
            _ => {}
        }
    }
    if verts.len() < 9 || tris.len() < 3 {
        return None;
    }
    Some((verts, tris))
}

/// Serialize a Manifold to binary little-endian PLY with per-vertex color.
/// Triangles are non-indexed (3 fresh vertices each) so each face carries its own
/// flat color — recovered per triangle from Manifold's run/original-ID metadata
/// via [`PALETTE`]. This is the shape maquette's PLY reader consumes.
fn to_ply(mesh: &Manifold) -> Vec<u8> {
    let mg = mesh.to_meshgl64();
    let np = mg.num_prop().max(3);
    let vp = mg.vert_properties(); // flat f64, stride np, first 3 = xyz
    let idx = mg.tri_verts(); // flat u64, 3 per triangle
    let ntri = mg.num_tri();

    // Per-triangle color from the run/original-ID table.
    let mut tri_col: Vec<[u8; 4]> = vec![DEFAULT_RGB.0; ntri];
    let run_index = mg.run_index(); // Vec<u64>, boundaries into the flat idx array
    let run_oid = mg.run_original_id(); // Vec<u32>, one id per run
    PALETTE.with(|p| {
        let pal = p.borrow();
        for (r, &oid) in run_oid.iter().enumerate() {
            let start = (run_index.get(r).copied().unwrap_or(0) / 3) as usize;
            let end = (run_index.get(r + 1).copied().unwrap_or((ntri * 3) as u64) / 3) as usize;
            let col = pal.get(&oid).cloned().unwrap_or(DEFAULT_RGB).0;
            for t in start..end.min(ntri) {
                tri_col[t] = col;
            }
        }
    });

    let vcount = ntri * 3;
    let mut out = Vec::with_capacity(vcount * 16 + ntri * 13 + 256);
    let header = format!(
        "ply\n\
         format binary_little_endian 1.0\n\
         comment generated by maquette-scad (Manifold kernel)\n\
         element vertex {vcount}\n\
         property float x\nproperty float y\nproperty float z\n\
         property uchar red\nproperty uchar green\nproperty uchar blue\nproperty uchar alpha\n\
         element face {ntri}\n\
         property list uchar uint vertex_indices\n\
         end_header\n"
    );
    out.extend_from_slice(header.as_bytes());
    let pos = |vi: u64| {
        let b = (vi as usize) * np;
        [vp[b] as f32, vp[b + 1] as f32, vp[b + 2] as f32]
    };
    for t in 0..ntri {
        let col = tri_col[t];
        for k in 0..3 {
            let p = pos(idx[t * 3 + k]);
            out.extend_from_slice(&p[0].to_le_bytes());
            out.extend_from_slice(&p[1].to_le_bytes());
            out.extend_from_slice(&p[2].to_le_bytes());
            out.extend_from_slice(&col);
        }
    }
    for i in 0..ntri as u32 {
        out.push(3u8);
        out.extend_from_slice(&(i * 3).to_le_bytes());
        out.extend_from_slice(&(i * 3 + 1).to_le_bytes());
        out.extend_from_slice(&(i * 3 + 2).to_le_bytes());
    }
    out
}

/// Default facet count ($fn) from the options JSON blob (fallback 32).
fn opt_seg(opts: &[u8]) -> usize {
    json::parse(opts).ok().and_then(|o| num(&o, "fn")).map(|f| (f as usize).max(3)).unwrap_or(32)
}

/// Shared tail of both entry points: reset per-compile state, build the tree,
/// and serialize the resulting solid to PLY. A top-level 2D result is extruded to
/// a thin plate so it renders (OpenSCAD shows 2D output flat in the XY plane).
fn finish(tree: &Json, defaults: &Defaults) -> Result<Vec<u8>, String> {
    PALETTE.with(|p| p.borrow_mut().clear());
    // Precompute subtree hashes and reset the memo cache for this compile.
    NODE_HASH.with(|m| {
        let mut m = m.borrow_mut();
        m.clear();
        prehash(tree, &mut m);
    });
    MEMO.with(|m| m.borrow_mut().clear());
    let mesh = build(tree, DEFAULT_RGB, defaults)?.to_manifold_extruding();
    if mesh.is_empty() {
        return Err("scad: empty result (no geometry)".into());
    }
    Ok(to_ply(&mesh))
}

/// Entry point: DSL tree (JSON bytes) + options (JSON bytes) -> binary PLY.
#[wasm_func]
fn build_ply(dsl: &[u8], opts: &[u8], bin: &[u8]) -> Result<Vec<u8>, String> {
    let tree = json::parse(dsl)?;
    finish(&tree, &Defaults { seg: opt_seg(opts), bin: parse_bin(bin) })
}

/// Entry point: real OpenSCAD `.scad` source text + options (JSON) -> binary PLY.
#[wasm_func]
fn build_scad(src: &[u8], files: &[u8], opts: &[u8], bin: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(src).map_err(|_| "scad: source is not valid UTF-8")?;
    let mut file_map = HashMap::new();
    if let Ok(Json::Obj(entries)) = json::parse(files) {
        for (k, v) in entries {
            if let Some(s) = v.as_str() {
                file_map.insert(k, s.to_string());
            }
        }
    }
    compile_scad(text, file_map, opt_seg(opts), parse_bin(bin))
}

/// Compile `.scad` source (with resolved library `files` + binary `bin` assets)
/// to PLY bytes. Split out from the wasm entry so it can be driven natively
/// (the repro/probe/dragon harnesses on x86, where a Manifold trap prints a real
/// backtrace).
pub fn compile_scad(
    text: &str,
    files: HashMap<String, String>,
    seg: usize,
    bin: HashMap<String, Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let tree = scad::scad_to_dsl(text, files)?;
    finish(&tree, &Defaults { seg, bin })
}


//! OpenSCAD `text()` primitive — glyph outlines from a TTF/OTF font.
//!
//! Outputs a list of closed polygons (each an inner or outer contour) in
//! model units, ready for `CrossSection::from_polygons_with_fill_rule(...,
//! EvenOdd)`. The caller then treats the result like any other 2D shape
//! (extrude, offset, boolean, …).
//!
//! Font source resolution — user-supplied bytes win; otherwise we fall
//! back to the DejaVu Sans subset embedded at build time (assets/).
//! Manifold has no font engine of its own, so we do the shaping in Rust
//! with `ttf-parser` + a curve tessellator.

use ttf_parser::{Face, OutlineBuilder};

/// Latin-1 + Latin-Extended-A subset of DejaVu Sans — ~30 KB, covers ASCII
/// plus common European accents. See assets/DejaVuSans-LICENSE.txt.
pub const DEFAULT_FONT: &[u8] = include_bytes!("../assets/DejaVuSans-Subset.ttf");

/// Parameters mirroring OpenSCAD's `text(...)` module.
pub struct TextParams<'a> {
    pub text: &'a str,
    pub size: f64,        // capital-M height (OpenSCAD default 10)
    pub spacing: f64,     // advance-width multiplier (1.0 = font-native)
    pub halign: HAlign,
    pub valign: VAlign,
    /// Points on each quadratic / cubic Bezier segment. Higher = smoother
    /// curves, more polygon points. `$fn` on the node drives this.
    pub curve_steps: u32,
}

#[derive(Clone, Copy)]
pub enum HAlign {
    Left,
    Center,
    Right,
}
#[derive(Clone, Copy)]
pub enum VAlign {
    Baseline,
    Bottom,
    Center,
    Top,
}

/// Emit polygons (each a closed ring of `[x, y]`) that together form the
/// filled shape of `params.text`. Returns an empty vec if the font can't be
/// parsed or every char is missing — callers pipe that into `CrossSection`
/// unchanged (an empty section is a valid 2D geometry).
pub fn to_polygons(font_bytes: &[u8], params: &TextParams) -> Vec<Vec<[f64; 2]>> {
    let face = match Face::parse(font_bytes, 0) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    // OpenSCAD's `size` = cap height. We approximate by taking the font's
    // units-per-em as the "size 1" reference — a widely-accepted convention
    // for CAD text (matches OpenSCAD's own liboverflow behavior).
    let upem = face.units_per_em() as f64;
    let scale = params.size / upem;

    let mut all_polys: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cursor_x: f64 = 0.0;

    for ch in params.text.chars() {
        let Some(gid) = face.glyph_index(ch) else {
            // Missing glyph: skip, but still advance by the mean advance so
            // later characters don't collide back onto the missing one.
            cursor_x += (upem as f32 * 0.5) as f64 * scale * params.spacing;
            continue;
        };
        let mut builder = OutlineCollector::new(params.curve_steps);
        face.outline_glyph(gid, &mut builder);
        for ring in builder.finish() {
            all_polys.push(
                ring.into_iter()
                    .map(|[x, y]| [x as f64 * scale + cursor_x, y as f64 * scale])
                    .collect(),
            );
        }
        let adv = face.glyph_hor_advance(gid).unwrap_or(0) as f64;
        cursor_x += adv * scale * params.spacing;
    }

    // Alignment: shift the assembled ring set. Cheaper than re-emitting.
    let total_w = cursor_x;
    let dx = match params.halign {
        HAlign::Left => 0.0,
        HAlign::Center => -total_w / 2.0,
        HAlign::Right => -total_w,
    };
    let asc = face.ascender() as f64 * scale;
    let desc = face.descender() as f64 * scale;
    let dy = match params.valign {
        VAlign::Baseline => 0.0,
        VAlign::Bottom => -desc,             // desc is negative
        VAlign::Center => -(asc + desc) / 2.0,
        VAlign::Top => -asc,
    };
    if dx != 0.0 || dy != 0.0 {
        for poly in &mut all_polys {
            for p in poly {
                p[0] += dx;
                p[1] += dy;
            }
        }
    }
    all_polys
}

/// Accumulates line-segment approximations of a glyph's outline. Each
/// `close` finalises one contour; a glyph typically produces 1 (simple),
/// 2 (with a counter — e.g. `o`), or more (e.g. `%`).
struct OutlineCollector {
    steps: u32,
    cur: Vec<[f32; 2]>,
    done: Vec<Vec<[f32; 2]>>,
    pen: [f32; 2],
    start: [f32; 2],
}

impl OutlineCollector {
    fn new(steps: u32) -> Self {
        Self { steps: steps.max(2), cur: Vec::new(), done: Vec::new(), pen: [0.0; 2], start: [0.0; 2] }
    }
    fn finish(mut self) -> Vec<Vec<[f32; 2]>> {
        if !self.cur.is_empty() {
            self.done.push(std::mem::take(&mut self.cur));
        }
        self.done
    }
    fn push(&mut self, p: [f32; 2]) {
        // Drop duplicate consecutive points — Manifold rejects zero-length
        // edges when clipping.
        if self.cur.last().map_or(false, |&q| (q[0] - p[0]).abs() < 1e-6 && (q[1] - p[1]).abs() < 1e-6) {
            return;
        }
        self.cur.push(p);
        self.pen = p;
    }
}

impl OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        if !self.cur.is_empty() {
            self.done.push(std::mem::take(&mut self.cur));
        }
        self.pen = [x, y];
        self.start = [x, y];
        self.cur.push([x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push([x, y]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let [p0x, p0y] = self.pen;
        for i in 1..=self.steps {
            let t = i as f32 / self.steps as f32;
            let mt = 1.0 - t;
            let bx = mt * mt * p0x + 2.0 * mt * t * x1 + t * t * x;
            let by = mt * mt * p0y + 2.0 * mt * t * y1 + t * t * y;
            self.push([bx, by]);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let [p0x, p0y] = self.pen;
        for i in 1..=self.steps {
            let t = i as f32 / self.steps as f32;
            let mt = 1.0 - t;
            let bx = mt * mt * mt * p0x + 3.0 * mt * mt * t * x1 + 3.0 * mt * t * t * x2 + t * t * t * x;
            let by = mt * mt * mt * p0y + 3.0 * mt * mt * t * y1 + 3.0 * mt * t * t * y2 + t * t * t * y;
            self.push([bx, by]);
        }
    }
    fn close(&mut self) {
        // A ring that starts at the same point as the first move_to needs
        // no extra edge; Manifold's polygon builder closes automatically.
        if !self.cur.is_empty() {
            self.done.push(std::mem::take(&mut self.cur));
        }
    }
}

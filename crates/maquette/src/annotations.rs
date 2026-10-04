use crate::math::FloatExt;
use crate::config::{AnnotationConfig, GroupStyles};
use crate::math::FxHashMap;
use crate::svg::{push_f1, push_f2};

pub struct Annotation<'a> {
    pub anchor: (f64, f64),
    pub label_pos: (f64, f64),
    pub label: &'a str,
    /// The label text runs rightwards from `label_pos` (else leftwards).
    pub rightwards: bool,
}

/// Rough rendered width of `text` in a sans-serif font of `font_size`.
fn text_width(text: &str, font_size: f64) -> f64 {
    text.chars().count() as f64 * font_size * 0.6
}

pub fn compute_annotations<'a>(
    centroids: &FxHashMap<u32, (f64, f64)>,
    group_styles: &'a GroupStyles,
    ann: &AnnotationConfig,
    view_center: (f64, f64),
    w: f64,
    h: f64,
) -> Vec<Annotation<'a>> {
    let filter = &ann.groups;
    let mut anns: Vec<Annotation> = Vec::new();

    for (&gid, &(cx, cy)) in centroids {
        let Some(ga) = group_styles.get(&gid) else { continue };
        let Some(name) = ga.name.as_deref() else { continue };
        let label = ga.label.as_deref().unwrap_or(name);

        if !filter.is_empty() && !filter.iter().any(|f| f == name) {
            continue;
        }

        let dx = cx - view_center.0;
        let dy = cy - view_center.1;
        let len = (dx * dx + dy * dy).sqrt().fmax(1.0);
        let nx = dx / len;
        let ny = dy / len;

        let offset = ann.offset;
        let lx = cx + nx * offset;
        let ly = cy + ny * offset;

        anns.push(Annotation {
            anchor: (cx, cy),
            label_pos: (lx, ly),
            label,
            rightwards: lx >= cx,
        });
    }

    anns.sort_by(|a, b| a.label_pos.1.partial_cmp(&b.label_pos.1).unwrap_or(std::cmp::Ordering::Equal));

    let min_gap = ann.font_size * 1.4;
    for i in 1..anns.len() {
        let prev_y = anns[i - 1].label_pos.1;
        let cur_y = anns[i].label_pos.1;
        if cur_y - prev_y < min_gap {
            anns[i].label_pos.1 = prev_y + min_gap;
        }
    }

    let margin = ann.font_size;
    for a in &mut anns {
        let tw = text_width(a.label, ann.font_size);
        let (lo, hi) = if a.rightwards { (margin, w - margin - tw) } else { (margin + tw, w - margin) };
        a.label_pos.0 = if lo <= hi { a.label_pos.0.clamp(lo, hi) } else { a.label_pos.0.clamp(margin, w - margin) };
        a.label_pos.1 = a.label_pos.1.clamp(margin + ann.font_size, h - margin);
    }

    anns
}

pub fn write_annotations_svg(
    svg: &mut String,
    annotations: &[Annotation<'_>],
    ann: &AnnotationConfig,
) {
    let color = &ann.color;
    let font_size = ann.font_size;

    for a in annotations {
        let (ax, ay) = a.anchor;
        let (lx, ly) = a.label_pos;
        let anchor = if a.rightwards { "start" } else { "end" };

        svg.push_str("<circle cx=\""); push_f1(svg, ax);
        svg.push_str("\" cy=\""); push_f1(svg, ay);
        svg.push_str("\" r=\"3\" fill=\""); svg.push_str(color);
        svg.push_str("\"/>");

        svg.push_str("<line x1=\""); push_f1(svg, ax);
        svg.push_str("\" y1=\""); push_f1(svg, ay);
        svg.push_str("\" x2=\""); push_f1(svg, lx);
        svg.push_str("\" y2=\""); push_f1(svg, ly);
        svg.push_str("\" stroke=\""); svg.push_str(color);
        svg.push_str("\" stroke-width=\"1\"/>");

        svg.push_str("<text x=\""); push_f1(svg, lx);
        svg.push_str("\" y=\""); push_f1(svg, ly);
        svg.push_str("\" font-family=\"sans-serif\" font-size=\"");
        push_f2(svg, font_size);
        svg.push_str("\" fill=\""); svg.push_str(color);
        svg.push_str("\" text-anchor=\""); svg.push_str(anchor);
        svg.push_str("\">"); svg.push_str(&crate::svg::escape_xml(a.label));
        svg.push_str("</text>");
    }
}

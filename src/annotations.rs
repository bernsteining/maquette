use crate::config::{AnnotationConfig, GroupAppearance};
use crate::math::FxHashMap;
use crate::svg::{push_f1, push_f2};
use std::collections::HashMap;

pub struct Annotation<'a> {
    pub anchor: (f64, f64),
    pub label_pos: (f64, f64),
    pub name: &'a str,
}

pub fn compute_annotations<'a>(
    centroids: &FxHashMap<u32, (f64, f64)>,
    group_styles: &'a HashMap<u32, GroupAppearance>,
    ann: &AnnotationConfig,
    view_center: (f64, f64),
    w: f64,
    h: f64,
) -> Vec<Annotation<'a>> {
    let filter = &ann.groups;
    let mut anns: Vec<Annotation> = Vec::new();

    for (&gid, &(cx, cy)) in centroids {
        let name = match group_styles.get(&gid).and_then(|ga| ga.name.as_deref()) {
            Some(n) => n,
            None => continue,
        };

        // Filter by groups if specified
        if !filter.is_empty() && !filter.iter().any(|f| f == name) {
            continue;
        }

        // Direction from model center to group centroid
        let dx = cx - view_center.0;
        let dy = cy - view_center.1;
        let len = (dx * dx + dy * dy).sqrt().max(1.0);
        let nx = dx / len;
        let ny = dy / len;

        let offset = ann.offset;
        let lx = cx + nx * offset;
        let ly = cy + ny * offset;

        anns.push(Annotation {
            anchor: (cx, cy),
            label_pos: (lx, ly),
            name,
        });
    }

    // Sort by label Y position for overlap resolution
    anns.sort_by(|a, b| a.label_pos.1.partial_cmp(&b.label_pos.1).unwrap_or(std::cmp::Ordering::Equal));

    // Resolve vertical overlaps
    let min_gap = ann.font_size * 1.4;
    for i in 1..anns.len() {
        let prev_y = anns[i - 1].label_pos.1;
        let cur_y = anns[i].label_pos.1;
        if cur_y - prev_y < min_gap {
            anns[i].label_pos.1 = prev_y + min_gap;
        }
    }

    // Clamp to viewport
    let margin = ann.font_size;
    for a in &mut anns {
        a.label_pos.0 = a.label_pos.0.clamp(margin, w - margin);
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
        let anchor = if lx >= ax { "start" } else { "end" };

        // Dot at centroid
        svg.push_str("<circle cx=\""); push_f1(svg, ax);
        svg.push_str("\" cy=\""); push_f1(svg, ay);
        svg.push_str("\" r=\"3\" fill=\""); svg.push_str(color);
        svg.push_str("\"/>");

        // Leader line
        svg.push_str("<line x1=\""); push_f1(svg, ax);
        svg.push_str("\" y1=\""); push_f1(svg, ay);
        svg.push_str("\" x2=\""); push_f1(svg, lx);
        svg.push_str("\" y2=\""); push_f1(svg, ly);
        svg.push_str("\" stroke=\""); svg.push_str(color);
        svg.push_str("\" stroke-width=\"1\"/>");

        // Label
        svg.push_str("<text x=\""); push_f1(svg, lx);
        svg.push_str("\" y=\""); push_f1(svg, ly);
        svg.push_str("\" font-family=\"sans-serif\" font-size=\"");
        push_f2(svg, font_size);
        svg.push_str("\" fill=\""); svg.push_str(color);
        svg.push_str("\" text-anchor=\""); svg.push_str(anchor);
        svg.push_str("\">"); svg.push_str(a.name);
        svg.push_str("</text>");
    }
}

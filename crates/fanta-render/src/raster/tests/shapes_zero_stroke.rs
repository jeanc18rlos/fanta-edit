//! Zero-width stroke handling: a positive-width stroke paints its outline,
//! while a 0-width stroke must paint nothing (no Skia hairline artifact).
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

/// Build a doc with one 30×30 rect centred on the world origin, NO fill, and a
/// single solid stroke of the given `width` (logical px). At width 0 a faithful
/// renderer must paint nothing; a naive `set_stroke_width(0.0)` makes Skia draw a
/// one-device-pixel *hairline* along the outline — the bug this guards.
///
/// Geometry (64×64 surface, origin-centred viewport, zoom 1, display_scale 1):
/// world (x,y) → screen (x+32, y+32). The rect spans world (−15,−15)..(15,15) →
/// screen pixels (17,17)..(47,47), so its edges run along screen x∈{17,47} and
/// y∈{17,47}. A hairline (or any centered stroke) lights those edge pixels up.
fn zero_width_stroke_rect_doc(width: f64) -> Doc {
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), width);
    stroke.align = fanta_doc::StrokeAlign::Center;
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-15.0, -15.0, 30.0, 30.0),
        // No fill: only the stroke can paint, so any lit edge pixel is the stroke.
        fills: Default::default(),
        strokes,
        corner_radius: None,
        corner_radii: None,
    }));
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

#[test]
fn nonzero_width_stroke_still_paints_its_outline() {
    // Baseline / control: a normal (positive-width) stroke DOES paint along the
    // rect edge — the very pixel the zero-width case below must be empty. Without
    // this control a "skip zero-width" fix that accidentally skipped every stroke
    // would still pass the zero-width assertion.
    let doc = zero_width_stroke_rect_doc(4.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Right edge of the rect → screen (47, 32), under the centered 4px stroke.
    let edge = rgba_at(&buf, 64, 47, 32);
    assert!(
        edge[2] > 200 && edge[3] > 200,
        "a positive-width stroke must paint blue on the rect edge, got {edge:?}"
    );
}

#[test]
fn zero_width_stroke_paints_nothing() {
    // A 0-width stroke is invisible in Figma. Skia, left alone, treats width 0 as
    // a HAIRLINE (one device pixel) and would draw a 1px outline along the rect's
    // edges — a spurious artifact on imported `.fig` shapes carrying a 0-weight
    // stroke paint. `stroke_sk_path` must skip it, leaving the whole canvas
    // (which has no fill) transparent.
    let doc = zero_width_stroke_rect_doc(0.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Every edge pixel where a hairline would land must be empty.
    for (x, y) in [(47, 32), (17, 32), (32, 17), (32, 47)] {
        let p = rgba_at(&buf, 64, x, y);
        assert!(
            p[3] < 20,
            "zero-width stroke must paint no hairline at ({x},{y}), got {p:?}"
        );
    }
    // And nothing anywhere on the canvas (no fill, no stroke).
    assert_eq!(
        opaque_pixel_count(&buf),
        0,
        "a fill-less rect with a 0-width stroke must paint zero pixels"
    );
}

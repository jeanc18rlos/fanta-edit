//! Per-side borders on ROUNDED rects: outside/center bands must follow the
//! offset parallel curve at the corners, while a square box keeps hard corners.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

/// Build a 40×40 rounded rect (`radius`) centred on the world origin, filled
/// green, with a uniform 6px per-side border in the given `align`. Geometry on a
/// 64×64 origin-centred surface: world (-20,-20)..(20,20) → screen
/// (12,12)..(52,52); the box centre is screen (32,32). An Outside band reaches
/// screen (6..58); the straight outer edge at top sits at screen y ≈ 8.
fn rounded_per_side_doc(align: fanta_doc::StrokeAlign, radius: f64) -> Doc {
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 1.0);
    stroke.align = align;
    stroke.per_side = Some([6.0, 6.0, 6.0, 6.0]);
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: Some(radius),
        corner_radii: None,
    }));
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

#[test]
fn outside_per_side_border_rounds_its_outer_corner() {
    // An OUTSIDE-aligned per-side border on a ROUNDED rect must follow the
    // outline's parallel curve (corner radius `r + width`), not poke a hard
    // square corner past the rounded shape. With radius 20 the corners are full
    // quarter-circles, so the extreme outer corner pixel — which the old square
    // bands painted blue — must now be transparent, while the straight outer edge
    // mid-side still paints the blue band and the interior keeps the green fill.
    let doc = rounded_per_side_doc(fanta_doc::StrokeAlign::Outside, 20.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Extreme outer corner (screen (8,8)) is well outside the offset rounded
    // curve → the band is shaved away (transparent). Before the fix this was
    // a solid blue square corner.
    let corner = rgba_at(&buf, 64, 8, 8);
    assert!(
        corner[3] < 40,
        "outside border outer corner must be rounded away, got {corner:?}"
    );
    // Mid top edge, just outside the top (screen (32,8)): the straight outer band
    // is untouched by the corner rounding → still blue.
    let mid_top = rgba_at(&buf, 64, 32, 8);
    assert!(
        mid_top[2] > 200 && mid_top[3] > 200,
        "outside border straight outer edge must still paint blue, got {mid_top:?}"
    );
    // Interior centre stays the green fill (the outside band never covers it).
    let interior = rgba_at(&buf, 64, 32, 32);
    assert!(
        interior[1] > 180 && interior[2] < 40,
        "interior must stay the green fill, got {interior:?}"
    );
}

#[test]
fn outside_per_side_border_on_square_box_keeps_hard_outer_corner() {
    // Regression guard: an OUTSIDE per-side border on a SQUARE rect (no radius)
    // must keep its hard square outer corner — `outset_silhouette` returns `None`
    // for a plain rect, so the square-box path is byte-identical (the corner pixel
    // stays the border colour, exactly as before the rounded-corner fix).
    let doc = rounded_per_side_doc(fanta_doc::StrokeAlign::Outside, 0.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Extreme outer corner (screen (8,8)) is inside the 6px square outer band on
    // a square box → blue, opaque (no rounding clip applies).
    let corner = rgba_at(&buf, 64, 8, 8);
    assert!(
        corner[2] > 200 && corner[3] > 200,
        "square box outside border must keep its hard blue outer corner, got {corner:?}"
    );
}

#[test]
fn center_per_side_border_rounds_its_outer_corner() {
    // A CENTER-aligned per-side border on a rounded rect straddles the edge
    // (half in, half out), so its outer half must also follow the parallel curve.
    // With radius 20 the extreme outer corner (screen (10,10), past the centred
    // band's outer reach of 3px) is rounded away, while the straight outer band
    // mid-side still paints. (Center's outer reach is half of Outside's, so the
    // corner sample is closer in than the Outside test's.)
    let doc = rounded_per_side_doc(fanta_doc::StrokeAlign::Center, 20.0);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Extreme corner just past the box bbox (screen (10,10)) is outside the
    // offset rounded curve → transparent.
    let corner = rgba_at(&buf, 64, 10, 10);
    assert!(
        corner[3] < 40,
        "center border outer corner must be rounded away, got {corner:?}"
    );
    // Mid top edge, straddling the top edge just outside (screen (32,10)): blue.
    let mid_top = rgba_at(&buf, 64, 32, 10);
    assert!(
        mid_top[2] > 200 && mid_top[3] > 200,
        "center border straight outer edge must still paint blue, got {mid_top:?}"
    );
}

//! Stroke alignment (center / inside / outside) and per-side border weights,
//! including how per-side bands interact with corner radius.
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

/// Build a doc with one 30×30 rect centred on the world origin, a green fill,
/// and a single wide blue stroke whose alignment is `align`.
///
/// Geometry (64×64 surface, origin-centred viewport, zoom 1, display_scale
/// 1): world (x,y) → screen (x+32, y+32). The rect spans world
/// (−15,−15)..(15,15) → screen pixels (17,17)..(47,47); the right edge sits
/// at screen x = 47. With a stroke width of 8, a *centered* stroke reaches
/// world x ∈ [11, 19] → screen x ∈ [43, 51], so screen x = 50 lands just
/// outside the rect but under a centered stroke. An *inside* stroke reaches
/// only screen x ∈ [39, 47] (nothing past 47); an *outside* stroke reaches
/// screen x ∈ [47, 55] and leaves the interior fill untouched.
fn stroked_rect_doc(align: fanta_doc::StrokeAlign) -> Doc {
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 8.0);
    stroke.align = align;
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-15.0, -15.0, 30.0, 30.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

#[test]
fn centered_stroke_paints_just_outside_the_rect_edge() {
    // Baseline: the current (centered) behaviour DOES bleed past the rect
    // edge — this is the pixel the inside-aligned test below must clear.
    let doc = stroked_rect_doc(fanta_doc::StrokeAlign::Center);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Screen (50, 32): just outside the right edge (x=47), under the
    // centered stroke's outer half → blue, opaque.
    let p = rgba_at(&buf, 64, 50, 32);
    assert!(
        p[2] > 200 && p[3] > 200,
        "centered stroke should paint blue just outside the rect edge, got {p:?}"
    );
}

#[test]
fn inside_stroke_paints_nothing_outside_the_rect_bounds() {
    // An INSIDE-aligned stroke must stay entirely within the fill region:
    // the same pixel that the centered stroke painted blue is now empty.
    let doc = stroked_rect_doc(fanta_doc::StrokeAlign::Inside);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Just outside the rect edge → transparent (cleared canvas).
    let outside = rgba_at(&buf, 64, 50, 32);
    assert!(
        outside[3] < 40,
        "inside stroke must paint nothing outside the rect bounds, got {outside:?}"
    );
    // The stroke band still lands just inside the edge → blue.
    let band = rgba_at(&buf, 64, 45, 32);
    assert!(
        band[2] > 200 && band[3] > 200,
        "inside stroke should paint blue just inside the rect edge, got {band:?}"
    );
}

#[test]
fn outside_stroke_paints_outside_and_leaves_the_fill_interior_intact() {
    // An OUTSIDE-aligned stroke must paint beyond the rect edge while the
    // fill interior keeps its (green) fill — the stroke must not cover it.
    let doc = stroked_rect_doc(fanta_doc::StrokeAlign::Outside);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Just outside the edge → blue stroke.
    let outside = rgba_at(&buf, 64, 50, 32);
    assert!(
        outside[2] > 200 && outside[3] > 200,
        "outside stroke should paint blue beyond the rect edge, got {outside:?}"
    );
    // The interior (world origin → screen centre) stays the green fill,
    // untouched by the outside-clipped stroke.
    let interior = rgba_at(&buf, 64, 32, 32);
    assert!(
        interior[1] > 180 && interior[2] < 40,
        "outside stroke must leave the fill interior green, got {interior:?}"
    );
}

#[test]
fn per_side_border_draws_only_the_weighted_edges() {
    // F1 — a per-side stroke `[top=8, right=0, bottom=0, left=0]` (inside
    // align) must paint only the top edge band and leave the right/bottom/left
    // edges as the underlying green fill. Same 30×30 rect: screen
    // (17,17)..(47,47).
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 1.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    stroke.per_side = Some([8.0, 0.0, 0.0, 0.0]); // [top, right, bottom, left]
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-15.0, -15.0, 30.0, 30.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Top edge, just inside the top (screen y≈20, x=32): blue band.
    let top = rgba_at(&buf, 64, 32, 20);
    assert!(
        top[2] > 200 && top[3] > 200,
        "top edge with weight 8 must paint blue, got {top:?}"
    );
    // Right edge, just inside the right (screen x≈45, y=32): NO border → the
    // green fill shows through (right weight is 0).
    let right = rgba_at(&buf, 64, 45, 32);
    assert!(
        right[1] > 180 && right[2] < 40,
        "right edge with weight 0 must stay green, got {right:?}"
    );
    // Bottom edge, just inside the bottom (screen y≈44, x=32): also green.
    let bottom = rgba_at(&buf, 64, 32, 44);
    assert!(
        bottom[1] > 180 && bottom[2] < 40,
        "bottom edge with weight 0 must stay green, got {bottom:?}"
    );
}

#[test]
fn per_side_border_respects_corner_radius() {
    // A per-side (Inside-aligned) border on a ROUNDED rect must follow the
    // corner curve, not poke a hard square corner past the rounded fill. With a
    // radius == half the box the corners are fully carved away, so the extreme
    // corner pixel — which the old square-band code painted blue — must now be
    // transparent, while a mid-edge band pixel stays blue and the interior stays
    // the green fill.
    //
    // 40×40 rect at world (-20,-20)..(20,20) → screen (12,12)..(52,52); radius
    // 20 rounds every corner to a quarter-circle. A 6px inside border hugs each
    // edge.
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 1.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    // Unequal weights so this is unambiguously the per-side path.
    stroke.per_side = Some([6.0, 4.0, 6.0, 4.0]); // [top, right, bottom, left]
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: Some(20.0),
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Extreme top-left corner (screen (13,13)) is OUTSIDE the radius-20 curve —
    // the band must be clipped away, leaving the cleared (transparent) canvas.
    // Before the fix the square top/left bands painted this corner blue.
    let corner = rgba_at(&buf, 64, 13, 13);
    assert!(
        corner[3] < 40,
        "rounded corner must carve away the per-side band, got {corner:?}"
    );
    // Mid top edge, just inside the top (screen y≈15, x=32): still a blue band.
    let top = rgba_at(&buf, 64, 32, 15);
    assert!(
        top[2] > 200 && top[3] > 200,
        "mid top edge band must still paint blue, got {top:?}"
    );
    // Interior centre (screen (32,32)) stays the green fill, untouched.
    let interior = rgba_at(&buf, 64, 32, 32);
    assert!(
        interior[1] > 180 && interior[2] < 40,
        "interior must stay the green fill, got {interior:?}"
    );
}

#[test]
fn per_side_border_on_square_box_keeps_hard_corner() {
    // Regression guard: a per-side (Inside) border on a SQUARE rect (no corner
    // radius) must keep its hard square corner — the rounding clip is a no-op
    // when the shape is a plain rect, so the corner pixel stays the border
    // colour exactly as before the rounded-corner fix.
    //
    // 40×40 rect at world (-20,-20)..(20,20) → screen (12,12)..(52,52); a 6px
    // inside border on all sides covers the corner band.
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 1.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    stroke.per_side = Some([6.0, 4.0, 6.0, 4.0]);
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // The same top-left corner pixel (screen (14,14), safely inside the 6px/4px
    // top+left bands) is the blue border on a square box — proving the fix did
    // not regress hard-cornered per-side borders.
    let corner = rgba_at(&buf, 64, 14, 14);
    assert!(
        corner[2] > 200 && corner[3] > 200,
        "square box must keep its hard blue corner band, got {corner:?}"
    );
}

#[test]
fn thick_inside_stroke_keeps_the_rounded_outer_corner() {
    // An Inside stroke wider than twice the corner radius used to square off
    // the OUTER corner: the inset offset path clamped its radius to 0 and was
    // stroked at full width. Figma keeps the stroke's outer edge on the shape's
    // rounded outline (radius r); only the inner edge goes square.
    let mut stroke = fanta_doc::Stroke::solid(Color::rgb(0, 0, 255), 12.0);
    stroke.align = fanta_doc::StrokeAlign::Inside;
    let mut strokes = smallvec::SmallVec::new();
    strokes.push(stroke);
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path: fanta_doc::PathData::rect(-20.0, -20.0, 40.0, 40.0),
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes,
        corner_radius: Some(4.0),
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    doc.apply(Operation::create_node(n)).unwrap();
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Box is screen (12,12)..(52,52), radius 4 → corner pivot (16,16). Pixel
    // (12,12) (centre (12.5,12.5)) sits ~4.95 from the pivot, outside the
    // rounded outline: it must stay (nearly) unpainted. The old squared corner
    // painted it solid blue.
    let corner = rgba_at(&buf, 64, 12, 12);
    assert!(
        corner[3] < 100,
        "outer corner must stay rounded (unpainted), got {corner:?}"
    );
    // Mid-top edge inside the 12px band → blue.
    let mid_top = rgba_at(&buf, 64, 32, 15);
    assert!(
        mid_top[2] > 200 && mid_top[3] > 200,
        "inside stroke band must paint along the straight edge, got {mid_top:?}"
    );
    // Centre keeps the green fill (band reaches only 12px in).
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[1] > 180 && centre[2] < 40,
        "interior must stay the green fill, got {centre:?}"
    );
}

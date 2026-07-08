//! Path fill rules: disjoint subpaths fill as separate shapes (not one blob),
//! and even-odd vs non-zero winding on same-wound contours (donut hole).
use super::*;
use fanta_doc::{Doc, Operation, VectorNode};

/// Build a doc with one vector whose `PathData` has TWO disjoint, side-by-side
/// square subpaths (the "`{}`"/two-brace topology in miniature) and the given
/// `fill_rule`. World layout (centred on origin): a left square spanning world
/// x ∈ [−15, −5] and a right square x ∈ [5, 15], both y ∈ [−5, 5]. The middle
/// gap (x ∈ [−5, 5]) is empty in BOTH subpaths, so a correct renderer leaves it
/// transparent — the regression we guard against is the two contours rendering
/// as one connected blob that fills the gap.
fn two_square_subpaths_doc(fill_rule: fanta_doc::FillRule) -> Doc {
    let mut path = fanta_doc::PathData::new();
    // Left square.
    path.move_to(-15.0, -5.0)
        .line_to(-5.0, -5.0)
        .line_to(-5.0, 5.0)
        .line_to(-15.0, 5.0)
        .close();
    // Right square (a fresh move-to → a distinct subpath).
    path.move_to(5.0, -5.0)
        .line_to(15.0, -5.0)
        .line_to(15.0, 5.0)
        .line_to(5.0, 5.0)
        .close();
    path.fill_rule = fill_rule;
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path,
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

#[test]
fn two_disjoint_subpaths_fill_as_two_shapes_not_one_blob() {
    // The core "`{}`" regression: a single vector with two separate closed
    // subpaths must fill as TWO shapes with the empty middle gap staying
    // transparent — never one merged blob. Verified for BOTH fill rules, since
    // for disjoint (non-overlapping) contours non-zero and even-odd agree.
    //
    // Coordinate map (64×64, origin-centred, zoom 1, display_scale 1):
    // world (x,y) → screen (x+32, y+32). Left square → screen x ∈ [17, 27],
    // right square → screen x ∈ [37, 47], the gap → screen x ∈ [27, 37]. All at
    // screen y = 32 (world y = 0, the squares' vertical centre).
    for rule in [fanta_doc::FillRule::NonZero, fanta_doc::FillRule::EvenOdd] {
        let doc = two_square_subpaths_doc(rule);
        let mut r = RasterRenderer::new(64, 64).unwrap();
        r.render(&doc.scene, &doc.viewport);
        let buf = r.copy_rgba();
        // Left square centre (world (−10,0) → screen (22,32)) → green, opaque.
        let left = rgba_at(&buf, 64, 22, 32);
        assert!(
            left[1] > 180 && left[3] > 200,
            "{rule:?}: left brace must be filled green, got {left:?}"
        );
        // Right square centre (world (10,0) → screen (42,32)) → green, opaque.
        let right = rgba_at(&buf, 64, 42, 32);
        assert!(
            right[1] > 180 && right[3] > 200,
            "{rule:?}: right brace must be filled green, got {right:?}"
        );
        // The MIDDLE gap (world (0,0) → screen (32,32)) is in neither subpath →
        // it must stay transparent. A merged blob would fill it.
        let gap = rgba_at(&buf, 64, 32, 32);
        assert!(
            gap[3] < 40,
            "{rule:?}: the gap between the two braces must stay transparent (not a merged blob), got {gap:?}"
        );
    }
}

/// Build a doc with one vector whose `PathData` is a square with a smaller
/// square hole inside it (a donut), both contours wound the SAME direction, and
/// the given `fill_rule`. Outer world box x,y ∈ [−15, 15]; inner box ∈ [−5, 5].
/// Under EVEN-ODD the inner box carves a hole regardless of winding direction;
/// under NON-ZERO with both contours wound identically it fills solid (no hole).
/// This is the fill-rule that *must* be honoured so a designer's even-odd donut
/// reads as a ring, not a filled square.
fn donut_same_winding_doc(fill_rule: fanta_doc::FillRule) -> Doc {
    let mut path = fanta_doc::PathData::new();
    // Outer square, clockwise (in screen y-down): TL → TR → BR → BL.
    path.move_to(-15.0, -15.0)
        .line_to(15.0, -15.0)
        .line_to(15.0, 15.0)
        .line_to(-15.0, 15.0)
        .close();
    // Inner square, SAME winding order (also TL → TR → BR → BL).
    path.move_to(-5.0, -5.0)
        .line_to(5.0, -5.0)
        .line_to(5.0, 5.0)
        .line_to(-5.0, 5.0)
        .close();
    path.fill_rule = fill_rule;
    let n = CanvasNode::new(NodeData::Vector(VectorNode {
        path,
        fills: smallvec_of(Fill::solid(Color::rgb(0, 200, 0))),
        strokes: Default::default(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
    }));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(n)).unwrap();
    doc
}

#[test]
fn even_odd_carves_a_hole_that_non_zero_same_winding_fills() {
    // The fill-rule is load-bearing: with two same-wound nested contours,
    // EVEN-ODD must empty the inner region (a ring) while NON-ZERO fills it
    // solid. This proves `to_sk_path` actually carries `FillRule` into Skia's
    // `SkPathFillType` (the single owner of that mapping) — the renderer's half
    // of the "honour the winding rule" contract.
    //
    // Same 64×64 origin-centred map: world centre (0,0) → screen (32,32) is the
    // hole; world (10,0) → screen (42,32) is the solid ring band (between the
    // inner +5 and outer +15 edges).

    // EVEN-ODD: hole is empty, ring band is filled.
    let doc = donut_same_winding_doc(fanta_doc::FillRule::EvenOdd);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let hole = rgba_at(&buf, 64, 32, 32);
    assert!(
        hole[3] < 40,
        "even-odd must carve the inner hole transparent, got {hole:?}"
    );
    let band = rgba_at(&buf, 64, 42, 32);
    assert!(
        band[1] > 180 && band[3] > 200,
        "even-odd ring band must stay filled green, got {band:?}"
    );

    // NON-ZERO with the same (identical-winding) geometry: the inner square does
    // NOT carve — the whole outer square fills solid, so the centre is green.
    let doc = donut_same_winding_doc(fanta_doc::FillRule::NonZero);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let centre = rgba_at(&buf, 64, 32, 32);
    assert!(
        centre[1] > 180 && centre[3] > 200,
        "non-zero with same-wound contours fills the centre solid, got {centre:?}"
    );
}

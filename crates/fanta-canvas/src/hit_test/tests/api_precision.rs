//! Viewport-aware screen API resolution and Bounds-vs-Path hit precision.

use super::*;
// ---- viewport-aware screen API ------------------------------------------

#[test]
fn hit_test_screen_resolves_topmost_under_pointer() {
    let mut doc = Doc::new();
    let mut bottom = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    bottom.index = IndexKey::from_raw(1.0);
    doc.apply(Operation::create_node(bottom)).unwrap();
    let mut top = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::BLACK,
    )));
    top.index = IndexKey::from_raw(2.0);
    let top_id = top.id;
    doc.apply(Operation::create_node(top)).unwrap();

    let size = DVec2::new(800.0, 600.0);
    let hit = hit_test_screen(
        &doc.scene,
        &doc.viewport,
        size,
        size * 0.5,
        HitPrecision::Bounds,
        None,
    );
    assert_eq!(hit, Some(top_id));
}

#[test]
fn a_locked_node_is_not_hit_so_the_click_falls_through() {
    let mut doc = Doc::new();
    let mut bottom = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    bottom.index = IndexKey::from_raw(1.0);
    let bottom_id = bottom.id;
    doc.apply(Operation::create_node(bottom)).unwrap();
    let mut top = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::BLACK,
    )));
    top.index = IndexKey::from_raw(2.0);
    top.flags |= NodeFlags::LOCKED;
    let top_id = top.id;
    doc.apply(Operation::create_node(top)).unwrap();

    // A click over the locked top rect falls through to the bottom rect rather
    // than selecting the locked node.
    assert_eq!(
        hit_test(&doc.scene, DVec2::ZERO, HitPrecision::Bounds, None),
        Some(bottom_id),
        "a locked node must not be hit; the click falls through to what is behind"
    );
    // Unlocking restores the top rect as the topmost hit.
    doc.apply(Operation::SetFlags {
        id: top_id,
        old: NodeFlags::LOCKED,
        new: NodeFlags::empty(),
    })
    .unwrap();
    assert_eq!(
        hit_test(&doc.scene, DVec2::ZERO, HitPrecision::Bounds, None),
        Some(top_id),
    );
}

// ---- Bounds vs Path precision -------------------------------------------

#[test]
fn bounds_precision_catches_inside_aabb_only_holes() {
    // Build a "C" — outer rect minus an inner rect, fused into one path
    // via two subpaths. Even-odd rule treats the inner subpath as a hole.
    let mut path = PathData::new();
    // Outer (CCW)
    path.move_to(-20.0, -20.0)
        .line_to(20.0, -20.0)
        .line_to(20.0, 20.0)
        .line_to(-20.0, 20.0)
        .close();
    // Inner hole (CCW; even-odd makes this subtract)
    path.move_to(-10.0, -10.0)
        .line_to(10.0, -10.0)
        .line_to(10.0, 10.0)
        .line_to(-10.0, 10.0)
        .close();

    let v = VectorNode {
        path,
        fills: smallvec::smallvec![fanta_doc::Fill::solid(Color::BLACK)],
        strokes: smallvec::smallvec![],
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
    };
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::Vector(v));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();

    // Point inside the AABB (origin) but inside the hole.
    let p = DVec2::new(0.0, 0.0);
    assert_eq!(
        hit_test(&doc.scene, p, HitPrecision::Bounds, None),
        Some(id),
        "bounds mode catches the AABB"
    );
    assert_eq!(
        hit_test(&doc.scene, p, HitPrecision::Path, None),
        None,
        "path mode rejects the hole"
    );
    // Point inside the solid ring.
    let q = DVec2::new(15.0, 0.0);
    assert_eq!(hit_test(&doc.scene, q, HitPrecision::Path, None), Some(id));
}

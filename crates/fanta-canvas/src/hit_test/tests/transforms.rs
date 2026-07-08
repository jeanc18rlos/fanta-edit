//! `node_screen_bounds` zoom handling and transformed-node local-space hits.

use super::*;
// ---- node_screen_bounds -------------------------------------------------

#[test]
fn node_screen_bounds_accounts_for_zoom() {
    let (doc, id) = rect_doc(-10.0, -10.0, 20.0, 20.0);
    let size = DVec2::new(800.0, 600.0);
    let viewport = Viewport {
        center: [0.0, 0.0],
        zoom: 2.0,
    };
    let bb = node_screen_bounds(&doc.scene, id, &viewport, size).unwrap();
    // World rect 20x20 → 40x40 on screen at zoom 2.
    assert!((bb.width() - 40.0).abs() < 1e-9);
    assert!((bb.height() - 40.0).abs() < 1e-9);
}

// ---- transformed nodes --------------------------------------------------

#[test]
fn hit_test_path_under_translation_uses_local_space() {
    // A rect translated by +100 should still respond at world (100, 0),
    // not (0, 0). Confirms the world→local inverse is correct.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -5.0,
        -5.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    n.transform = Transform2D::translation(100.0, 0.0);
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    assert_eq!(
        hit_test(&doc.scene, DVec2::new(100.0, 0.0), HitPrecision::Path, None),
        Some(id)
    );
    assert_eq!(
        hit_test(&doc.scene, DVec2::new(0.0, 0.0), HitPrecision::Path, None),
        None
    );
}

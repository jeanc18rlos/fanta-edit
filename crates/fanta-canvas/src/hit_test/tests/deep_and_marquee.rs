//! Deep (all-hits) queries, group self-exclusion, and marquee selection
//! (contains/intersects, locked + hidden + group recursion).

use super::*;
// ---- deep hit + groups --------------------------------------------------

#[test]
fn hit_test_deep_returns_all_hits_top_first() {
    let mut doc = Doc::new();
    let mut a = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    a.index = IndexKey::from_raw(1.0);
    let a_id = a.id;
    doc.apply(Operation::create_node(a)).unwrap();
    let mut b = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::BLACK,
    )));
    b.index = IndexKey::from_raw(2.0);
    let b_id = b.id;
    doc.apply(Operation::create_node(b)).unwrap();

    let hits = hit_test_deep(&doc.scene, DVec2::ZERO, HitPrecision::Bounds, None);
    assert_eq!(hits.as_slice(), &[b_id, a_id]);
}

#[test]
fn groups_do_not_self_catch_hits() {
    let mut doc = Doc::new();
    let g = CanvasNode::new(NodeData::Group(GroupNode::default()));
    doc.apply(Operation::create_node(g)).unwrap();
    // Group is empty — nothing to hit, even though its derived bounds may
    // be ZERO at origin.
    assert_eq!(
        hit_test(&doc.scene, DVec2::ZERO, HitPrecision::Bounds, None),
        None
    );
}

#[test]
fn frame_surface_catches_clicks_on_its_body() {
    // A FRAME — a group with a clip box (or a background paint) — is a real
    // surface: clicking its body selects the frame so it can be grabbed and
    // dragged, unlike a plain (click-through) group above.
    let mut doc = Doc::new();
    let frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        ..Default::default()
    }));
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    assert_eq!(
        hit_test(&doc.scene, DVec2::new(50.0, 50.0), HitPrecision::Path, None),
        Some(frame_id),
    );
}

#[test]
fn child_wins_over_frame_body_but_frame_catches_empty_space() {
    let mut doc = Doc::new();
    let frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        ..Default::default()
    }));
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        10.0,
        10.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    child.parent = Some(frame_id);
    let c_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    // Over a child: the child (top-z) wins — drilling-in selection.
    assert_eq!(
        hit_test(&doc.scene, DVec2::new(20.0, 20.0), HitPrecision::Path, None),
        Some(c_id),
    );
    // Over the frame's empty body: the frame itself catches the click.
    assert_eq!(
        hit_test(&doc.scene, DVec2::new(80.0, 80.0), HitPrecision::Path, None),
        Some(frame_id),
    );
}

// ---- marquee ------------------------------------------------------------

#[test]
fn marquee_contains_requires_full_inclusion() {
    let (doc, id) = rect_doc(0.0, 0.0, 10.0, 10.0);
    // Marquee that fully contains
    let r = Bounds::from_xywh(-5.0, -5.0, 30.0, 30.0);
    assert_eq!(
        hit_test_within(&doc.scene, r, MarqueeMode::Contains, None).as_slice(),
        &[id]
    );
    // Marquee that only partially overlaps — not selected under Contains.
    let r2 = Bounds::from_xywh(5.0, 5.0, 30.0, 30.0);
    assert!(hit_test_within(&doc.scene, r2, MarqueeMode::Contains, None).is_empty());
    // Same partial marquee under Intersects — selected.
    assert_eq!(
        hit_test_within(&doc.scene, r2, MarqueeMode::Intersects, None).as_slice(),
        &[id]
    );
}

#[test]
fn marquee_skips_locked_nodes() {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    n.flags |= NodeFlags::LOCKED;
    doc.apply(Operation::create_node(n)).unwrap();
    let r = Bounds::from_xywh(-10.0, -10.0, 100.0, 100.0);
    assert!(hit_test_within(&doc.scene, r, MarqueeMode::Contains, None).is_empty());
}

#[test]
fn marquee_recurses_into_groups() {
    let mut doc = Doc::new();
    let g = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let g_id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    child.parent = Some(g_id);
    let c_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    let r = Bounds::from_xywh(-5.0, -5.0, 30.0, 30.0);
    // Group itself is omitted; its child is included.
    assert_eq!(
        hit_test_within(&doc.scene, r, MarqueeMode::Contains, None).as_slice(),
        &[c_id]
    );
}

#[test]
fn marquee_skips_hidden_nodes() {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    n.flags |= NodeFlags::HIDDEN;
    doc.apply(Operation::create_node(n)).unwrap();
    let r = Bounds::from_xywh(-10.0, -10.0, 100.0, 100.0);
    assert!(hit_test_within(&doc.scene, r, MarqueeMode::Contains, None).is_empty());
}

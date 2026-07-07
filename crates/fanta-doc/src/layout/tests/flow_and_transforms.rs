//! Nodes that keep their baked transforms: absolute children excluded from
//! flow, non-auto-layout frames, and a vector whose offset path origin must
//! still land its top-left at the padding target.

use super::*;

// ---------------------------------------------------------------------------
// Absolute child excluded from flow
// ---------------------------------------------------------------------------

#[test]
fn absolute_child_excluded_from_flow_and_untouched() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        spacing: 5.0,
        ..Default::default()
    };
    let f = t.push(frame(200.0, 50.0, al));
    let a = t.push(rect_child(f, 20.0, 10.0));
    let mut abs_node = rect_child(f, 20.0, 10.0);
    abs_node.transform = Transform2D::translation(150.0, 30.0);
    abs_node.layout_child = Some(LayoutChild {
        grow: 0.0,
        absolute: true,
        align_self: None,
    });
    let abs = t.push(abs_node);
    let b = t.push(rect_child(f, 20.0, 10.0));

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Flow = [a, b]: a@0, b@25 (absolute didn't consume a slot).
    approx(placed_origin(&t, a), [0.0, 0.0]);
    approx(placed_origin(&t, b), [25.0, 0.0]);
    // Absolute child keeps its own baked transform.
    approx(placed_origin(&t, abs), [150.0, 30.0]);
}

// ---------------------------------------------------------------------------
// Free (non-auto-layout) frames keep baked transforms
// ---------------------------------------------------------------------------

#[test]
fn non_auto_layout_frame_does_not_reposition_children() {
    let mut t = VecTree::new();
    // A plain frame: clip_size set, but NO auto_layout.
    let pid = t.push(CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([200.0, 100.0]),
        auto_layout: None,
        ..Default::default()
    })));

    let mut child = rect_child(pid, 20.0, 10.0);
    child.transform = Transform2D::translation(123.0, 45.0);
    let cid = t.push(child);

    solve_auto_layout(&mut t, pid, &mut no_measure);

    // Untouched — keeps its baked transform.
    approx(placed_origin(&t, cid), [123.0, 45.0]);
}

// ---------------------------------------------------------------------------
// Vector with offset path origin lands its top-left at the target
// ---------------------------------------------------------------------------

#[test]
fn child_with_offset_path_origin_lands_top_left_at_target() {
    let mut t = VecTree::new();
    let al = AutoLayout {
        mode: LayoutMode::Horizontal,
        padding: [5.0, 0.0, 0.0, 5.0],
        ..Default::default()
    };
    let f = t.push(frame(200.0, 100.0, al));
    // A vector whose path rect starts at (40, 70), not the origin.
    let mut v = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        40.0,
        70.0,
        30.0,
        20.0,
        Color::BLACK,
    )));
    v.parent = Some(f);
    let vid = t.push(v);

    solve_auto_layout(&mut t, f, &mut no_measure);

    // Its top-left (the path's min corner) must sit at padding (5,5) regardless
    // of the path's internal offset.
    approx(placed_origin(&t, vid), [5.0, 5.0]);
}

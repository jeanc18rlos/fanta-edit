use super::*;

// -------------------------------------------------------------------------
// Drag-to-reparent (ported from OpenPencil reparentOutsideNodes +
// findMoveDropTarget / doReorderChild)
// -------------------------------------------------------------------------

/// Build a Figma-style *clipped frame*: a group with a fixed `clip_size`
/// box (so its world bounds do NOT follow its children — the property that
/// lets a child be dragged "outside" the frame). The frame's local box spans
/// `[0,0,w,h]`; `origin` translates it in world space. Returns the frame id.
fn clipped_frame(doc: &mut Doc, origin: DVec2, w: f64, h: f64) -> NodeId {
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([w, h]),
        ..GroupNode::default()
    }));
    frame.transform = Transform2D::translation(origin.x, origin.y);
    frame.index = IndexKey::FIRST;
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    frame_id
}

/// Add a rect child of size `w`×`h` at local `local` inside `parent`.
fn child_rect(doc: &mut Doc, parent: NodeId, local: DVec2, w: f64, h: f64) -> NodeId {
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        w,
        h,
        Color::WHITE,
    )));
    child.parent = Some(parent);
    child.transform = Transform2D::translation(local.x, local.y);
    child.index = IndexKey::FIRST;
    let id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    id
}

/// Dragging a child entirely OUT of its (clipped) frame's world bounds pops
/// it up to the root (the frame is top-level, so its grandparent is `None`),
/// and the child's WORLD position is preserved across the reparent.
#[test]
fn drag_child_outside_frame_reparents_to_root_preserving_world() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Clipped frame spanning world [0,0,40,40]; a 20x20 child sits at local
    // (10,10) → world box [10,10,30,30], center (20,20) — inside the frame.
    let frame = clipped_frame(&mut doc, DVec2::ZERO, 40.0, 40.0);
    let child = child_rect(&mut doc, frame, DVec2::new(10.0, 10.0), 20.0, 20.0);
    doc.selection.select_only(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Press on the child center (world (20,20)), drag +200 in X — well clear
    // of the frame's 40-wide clip box, so the child lands entirely outside.
    let press = screen_for_world(DVec2::new(20.0, 20.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 200.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 200.0, press[1]], ModifierKeys::empty()),
    );

    // Reparented to root.
    assert_eq!(
        ctx.doc.scene.get(child).unwrap().parent,
        None,
        "child should have popped out to root"
    );
    // World position preserved: center now at world (220, 20).
    let b = ctx.doc.scene.world_bounds(child).unwrap();
    assert!(
        (b.center().x - 220.0).abs() < 1e-6 && (b.center().y - 20.0).abs() < 1e-6,
        "child world center should be (220,20), got {:?}",
        b.center()
    );
    // The frame is now empty of that child.
    assert!(!ctx.doc.scene.children_of(Some(frame)).contains(&child));
}

/// Dragging a root node so its center lands over a frame reparents it INTO
/// that frame, inserts it at the top of the frame's z-order, and preserves
/// the node's world position.
#[test]
fn drag_root_node_into_frame_reparents_and_preserves_world() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Clipped frame spanning world [-50,-50,50,50] (origin (-50,-50), 100x100).
    let frame = clipped_frame(&mut doc, DVec2::new(-50.0, -50.0), 100.0, 100.0);
    // A free root rect 20x20 placed far to the right (world center (300,0)).
    let rect = rect_at(&mut doc, 290.0, -10.0, 20.0, 20.0);
    doc.selection.select_only(rect);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Press on the rect center (world (300,0)), drag left by 300 so its center
    // lands at world (0,0) — squarely inside the frame box.
    let press = screen_for_world(DVec2::new(300.0, 0.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] - 300.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] - 300.0, press[1]], ModifierKeys::empty()),
    );

    // Reparented into the frame.
    assert_eq!(
        ctx.doc.scene.get(rect).unwrap().parent,
        Some(frame),
        "rect should have been adopted by the frame"
    );
    // Inserted as a sibling (top of z-order).
    assert!(ctx.doc.scene.children_of(Some(frame)).contains(&rect));
    // World position preserved: center at world (0,0).
    let b = ctx.doc.scene.world_bounds(rect).unwrap();
    assert!(
        b.center().length() < 1e-6,
        "rect world center should be origin, got {:?}",
        b.center()
    );
}

/// A drag that keeps the child inside its frame does NOT reparent it.
#[test]
fn small_drag_inside_frame_keeps_parent() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // (Unclipped) group with a 20x20 child centered at origin. An unclipped
    // group's bounds are the union of its children, so it can never have a
    // child fall "outside" — exactly the Figma group (vs frame) semantic.
    let (frame, child) = frame_with_child(&mut doc, DVec2::ZERO, DVec2::ZERO);
    // Add a second, larger sibling so the group's bounds span [-50,50] and a
    // small move keeps the child inside.
    let mut big = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -50.0,
        -50.0,
        100.0,
        100.0,
        Color::WHITE,
    )));
    big.parent = Some(frame);
    big.index = IndexKey::from_raw(0.5);
    let big_id = big.id;
    doc.apply(Operation::create_node(big)).unwrap();
    doc.selection.select_only(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    let press = screen_for_world(DVec2::ZERO, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    // Move only +5 in X — child center moves to (5,0), still well inside the
    // frame box [-50,50] and still inside its own old parent.
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 5.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 5.0, press[1]], ModifierKeys::empty()),
    );

    // Still parented to the same frame, no reparent.
    assert_eq!(ctx.doc.scene.get(child).unwrap().parent, Some(frame));
    let _ = big_id;
}

/// Move + reparent collapse into ONE undo step, and that single undo restores
/// both the original parent and the original world position.
#[test]
fn reparent_drag_is_one_undo_step_and_restores_parent() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Clipped frame [0,0,40,40]; child 20x20 at local (10,10) → world center
    // (20,20). Press there, drag +200 X so the child clears the clip box.
    let frame = clipped_frame(&mut doc, DVec2::ZERO, 40.0, 40.0);
    let child = child_rect(&mut doc, frame, DVec2::new(10.0, 10.0), 20.0, 20.0);
    let child_world_before = doc.scene.world_bounds(child).unwrap().center();
    doc.selection.select_only(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let undo_before = ctx.doc.history.undo_depth();

    let press = screen_for_world(DVec2::new(20.0, 20.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 200.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 200.0, press[1]], ModifierKeys::empty()),
    );
    // Exactly one transaction for move + reparent + rebase.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    assert_eq!(ctx.doc.scene.get(child).unwrap().parent, None);

    // One undo restores the original parent AND the original world center.
    assert!(ctx.doc.undo().unwrap());
    assert_eq!(
        ctx.doc.scene.get(child).unwrap().parent,
        Some(frame),
        "undo should restore the original parent"
    );
    let b = ctx.doc.scene.world_bounds(child).unwrap();
    assert!(
        (b.center() - child_world_before).length() < 1e-6,
        "undo should restore world position to {child_world_before:?}, got {:?}",
        b.center()
    );
}

/// Dragging a frame so its center lands over its OWN child must not reparent
/// the frame into its child (that would be a cycle). The set_parent cycle
/// guard backs this, but we also never propose the dragged subtree as a
/// target — verify the frame stays at root.
#[test]
fn dragging_frame_never_reparents_into_own_descendant() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Frame holds a group child (a container) so a naive search could pick
    // the inner group as a drop target.
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
    frame.index = IndexKey::FIRST;
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();
    let mut inner = CanvasNode::new(NodeData::Group(GroupNode::default()));
    inner.parent = Some(frame_id);
    inner.index = IndexKey::FIRST;
    let inner_id = inner.id;
    doc.apply(Operation::create_node(inner)).unwrap();
    // Give the inner group bounds via a leaf so it has world bounds.
    let mut leaf = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    leaf.parent = Some(inner_id);
    leaf.index = IndexKey::FIRST;
    let leaf_id = leaf.id;
    doc.apply(Operation::create_node(leaf)).unwrap();

    // Select the whole subtree (frame + inner + leaf). The move set prunes to
    // top-level selected nodes, so only the FRAME moves; the inner group and
    // leaf ride along. Pressing on the leaf (which is in the selection) keeps
    // the frame as the dragged root rather than replacing the selection.
    doc.selection.select_only(frame_id);
    doc.selection.add(inner_id);
    doc.selection.add(leaf_id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Press where the frame's content is (over the leaf at origin) and drag a
    // little; the frame moves with its subtree, so the drop center is still
    // over the (also-moved) inner group — which must be rejected as a target.
    let press = screen_for_world(DVec2::ZERO, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 8.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 8.0, press[1]], ModifierKeys::empty()),
    );

    // The frame stays at root — never adopted by its own descendant.
    assert_eq!(
        ctx.doc.scene.get(frame_id).unwrap().parent,
        None,
        "frame must not be reparented into its own subtree"
    );
}

//! Container-first selection + double-click drill-in (Figma parity).
//!
//! A single click selects the OUTERMOST top-level frame under the cursor (so a
//! frame whose body is fully covered by children is still selectable anywhere
//! inside it); a double-click drills in one level toward the cursor, tracked by
//! the select tool's `entered_scope`.

use super::*;

/// A frame *surface* (a clipping group, so it is a real hittable body) at world
/// `(x, y)` sized `w × h`, parented under `parent`.
fn frame_surface(doc: &mut Doc, parent: Option<NodeId>, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let mut g = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([w, h]),
        ..Default::default()
    }));
    g.parent = parent;
    g.transform = Transform2D::translation(x, y);
    let id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    id
}

/// A rect filling its parent frame's local `(0,0)-(w,h)` box — so it covers the
/// frame's whole body (the "no exposed frame body" case).
fn covering_rect(doc: &mut Doc, parent: NodeId, w: f64, h: f64) -> NodeId {
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        w,
        h,
        Color::WHITE,
    )));
    n.parent = Some(parent);
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    id
}

/// A page-root group (a non-surface backdrop, like the real document's pages),
/// registered as the active page.
fn active_page(doc: &mut Doc) -> NodeId {
    let g = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    doc.add_page(id);
    doc.set_active_page(Some(id));
    id
}

const SIZE: DVec2 = DVec2::new(800.0, 600.0);

#[test]
fn single_click_selects_top_level_frame_not_covered_child() {
    // page → frame (200×200 @ origin) → child covering it entirely.
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let _child = covering_rect(&mut doc, frame, 200.0, 200.0);

    let viewport0 = Viewport::default();
    let at = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();
    tool.handle_event(&mut ctx, pe_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    // The top-level frame is selected, NOT the covering child — the click had
    // no exposed frame body to land on, yet the frame is reachable.
    assert_eq!(ctx.doc.selection.as_slice(), &[frame]);
    assert_eq!(tool.entered_scope(), None);
}

#[test]
fn double_click_drills_into_frame_and_selects_child() {
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let child = covering_rect(&mut doc, frame, 200.0, 200.0);

    let viewport0 = Viewport::default();
    let at = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();
    // First click selects the frame.
    tool.handle_event(&mut ctx, pe_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[frame]);
    // Double-click drills one level: selects the child, entering the frame.
    tool.handle_event(&mut ctx, pe_double_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[child]);
    assert_eq!(tool.entered_scope(), Some(frame));
}

#[test]
fn successive_double_clicks_descend_one_level_each() {
    // page → frameA (200×200 @ 0,0) → frameB (100×100 @ 50,50) → leaf covering B.
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame_a = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let frame_b = frame_surface(&mut doc, Some(frame_a), 50.0, 50.0, 100.0, 100.0);
    let leaf = covering_rect(&mut doc, frame_b, 100.0, 100.0);

    let viewport0 = Viewport::default();
    let at = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();

    // Single click: outermost top-level frame.
    tool.handle_event(&mut ctx, pe_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_a]);

    // Drill 1 → frameB, scope = frameA.
    tool.handle_event(&mut ctx, pe_double_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_b]);
    assert_eq!(tool.entered_scope(), Some(frame_a));

    // Drill 2 → leaf, scope = frameB.
    tool.handle_event(&mut ctx, pe_double_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[leaf]);
    assert_eq!(tool.entered_scope(), Some(frame_b));

    // Drill 3 → already at the deepest leaf: a no-op (selection unchanged).
    tool.handle_event(&mut ctx, pe_double_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[leaf]);
}

#[test]
fn empty_click_resets_entered_scope() {
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let _child = covering_rect(&mut doc, frame, 200.0, 200.0);

    let viewport0 = Viewport::default();
    let at = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();
    // Drill into the frame.
    tool.handle_event(&mut ctx, pe_double_press(at, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::empty()));
    assert_eq!(tool.entered_scope(), Some(frame));
    // Click empty canvas (far corner, outside the frame): clears + resets scope.
    let far = [SIZE.x - 1.0, SIZE.y - 1.0];
    tool.handle_event(&mut ctx, pe_press(far, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(far, ModifierKeys::empty()));
    assert!(ctx.doc.selection.is_empty());
    assert_eq!(tool.entered_scope(), None);
}

#[test]
fn clicking_a_different_top_level_frame_exits_the_entered_scope() {
    // Two non-overlapping top-level frames A and B under the page.
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame_a = frame_surface(&mut doc, Some(page), 0.0, 0.0, 100.0, 100.0);
    let _child_a = covering_rect(&mut doc, frame_a, 100.0, 100.0);
    let frame_b = frame_surface(&mut doc, Some(page), 300.0, 0.0, 100.0, 100.0);
    let _child_b = covering_rect(&mut doc, frame_b, 100.0, 100.0);

    let viewport0 = Viewport::default();
    let in_a = screen_for_world(DVec2::new(50.0, 50.0), &viewport0, SIZE);
    let in_b = screen_for_world(DVec2::new(350.0, 50.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();
    // Drill into A.
    tool.handle_event(&mut ctx, pe_double_press(in_a, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(in_a, ModifierKeys::empty()));
    assert_eq!(tool.entered_scope(), Some(frame_a));
    // Single click inside B selects B (outermost) and exits A's scope.
    tool.handle_event(&mut ctx, pe_press(in_b, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(in_b, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_b]);
    assert_eq!(tool.entered_scope(), None);
}

#[test]
fn shift_click_toggles_the_resolved_top_level_frame() {
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let _child = covering_rect(&mut doc, frame, 200.0, 200.0);

    let viewport0 = Viewport::default();
    let at = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), SIZE);
    let mut tool = SelectTool::new();
    // Shift-click toggles the resolved frame (not the covered child) in.
    tool.handle_event(&mut ctx, pe_press(at, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::SHIFT));
    assert_eq!(ctx.doc.selection.as_slice(), &[frame]);
    // And shift-click again toggles it back out.
    tool.handle_event(&mut ctx, pe_press(at, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(at, ModifierKeys::SHIFT));
    assert!(ctx.doc.selection.is_empty());
}

#[test]
fn dragging_a_covered_frame_moves_the_whole_frame() {
    // The user's core complaint: a frame fully covered by a child must be
    // grabbable + draggable from anywhere inside it.
    let mut doc = Doc::new();
    let page = active_page(&mut doc);
    let frame = frame_surface(&mut doc, Some(page), 0.0, 0.0, 200.0, 200.0);
    let child = covering_rect(&mut doc, frame, 200.0, 200.0);

    let viewport0 = Viewport::default();
    let p0 = screen_for_world(DVec2::new(100.0, 100.0), &viewport0, SIZE);
    let p1 = screen_for_world(DVec2::new(150.0, 100.0), &viewport0, SIZE); // +50 in x
    let mut viewport = viewport0;
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), SIZE);
    let mut tool = SelectTool::new();
    let child_min_before = ctx.doc.scene.world_bounds(child).unwrap().min_x;

    tool.handle_event(&mut ctx, pe_press(p0, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(p1, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(p1, ModifierKeys::empty()));

    // The frame is the move set; it (and its child) shifted by exactly +50.
    assert_eq!(ctx.doc.selection.as_slice(), &[frame]);
    let frame_min = ctx.doc.scene.world_bounds(frame).unwrap().min_x;
    assert!(
        (frame_min - 50.0).abs() < 1e-6,
        "frame x = {frame_min}, expected 50"
    );
    let child_min = ctx.doc.scene.world_bounds(child).unwrap().min_x;
    assert!(
        (child_min - (child_min_before + 50.0)).abs() < 1e-6,
        "child x = {child_min}, expected {}",
        child_min_before + 50.0
    );
}

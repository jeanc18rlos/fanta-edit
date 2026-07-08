use super::*;

// -------------------------------------------------------------------------
// Move drag
// -------------------------------------------------------------------------

#[test]
fn drag_on_selected_node_moves_via_one_transaction() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let undo_depth_before = doc.history.undo_depth();
    // Disable snap so the move math is deterministic in this test.
    let snap = SnapEngine {
        zoom: 1.0,
        targets: fanta_canvas::SnapTargets::empty(),
        ..Default::default()
    };
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 50.0, center[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 50.0, center[1]], ModifierKeys::empty()),
    );
    // The node should now have a translated transform.
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO).x - 50.0).abs() < 1e-6);
    // Exactly one new transaction added to undo.
    assert_eq!(ctx.doc.history.undo_depth(), undo_depth_before + 1);
}

#[test]
fn drag_below_threshold_is_treated_as_click_not_move() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    // 2 px drag — under threshold (3 px).
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 2.0, center[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 2.0, center[1]], ModifierKeys::empty()),
    );
    // Click semantics — selection replaced, transform unchanged.
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO)).length() < 1e-9);
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

#[test]
fn move_excludes_self_from_snap_candidates() {
    // Confirm we don't snap the moving node to itself by having a single
    // node in the scene and verifying the move applies cleanly with the
    // snap engine on.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 100.0, 100.0);
    doc.selection.select_only(id);
    let snap = SnapEngine {
        zoom: 1.0,
        targets: fanta_canvas::SnapTargets::NODE_EDGES,
        ..Default::default()
    };
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 100.0, center[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 100.0, center[1]], ModifierKeys::empty()),
    );
    // Without self-snap, the rect moves the full requested 100 world units.
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO).x - 100.0).abs() < 1e-6);
}

// -------------------------------------------------------------------------
// Nested selection move (the parent+child double-move bug)
// -------------------------------------------------------------------------

/// The core regression: when a marquee grabs BOTH a frame and its child,
/// dragging by delta D must move the child's WORLD position by exactly D, not
/// 2×D. The old code translated every selected node, so the child shifted
/// once via its own transform and again via its moved parent — detaching it
/// from the frame. Pruning the move set to top-level nodes fixes it: only the
/// frame moves, the child rides along.
#[test]
fn moving_parent_and_child_moves_child_world_by_delta_not_double() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Frame offset +60x +20y; child sits at +15x +10y inside it → child
    // world center starts at (75, 30).
    let (frame, child) = frame_with_child(&mut doc, DVec2::new(60.0, 20.0), DVec2::new(15.0, 10.0));
    // Select BOTH — exactly what a marquee over a nested design produces.
    doc.selection.select_only(frame);
    doc.selection.add(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    let child_before = ctx.doc.scene.world_bounds(child).unwrap();
    // Press on the child's world center (the frame is a group, so the
    // hit-test resolves to the child), then drag +40 in screen X.
    let press = screen_for_world(DVec2::new(75.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 40.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 40.0, press[1]], ModifierKeys::empty()),
    );

    let child_after = ctx.doc.scene.world_bounds(child).unwrap();
    // World shift must equal the requested +40 (NOT +80 from double-applying
    // both the child's own and its parent's translation).
    assert!(
        (child_after.min_x - child_before.min_x - 40.0).abs() < 1e-6,
        "child world x shifted by {}, expected 40 (double-move would give 80)",
        child_after.min_x - child_before.min_x
    );
    // Y was not dragged, so it must be unchanged.
    assert!((child_after.min_y - child_before.min_y).abs() < 1e-6);
    // The child stays the same size — it was translated, not scaled/sheared.
    assert!((child_after.width() - child_before.width()).abs() < 1e-6);
    // And the child remains attached to its frame: the frame moved by the
    // same +40, so the child's offset within the frame is preserved.
    let frame_t = ctx.doc.scene.get(frame).unwrap().transform;
    assert!((frame_t.transform_point(DVec2::ZERO).x - (60.0 + 40.0)).abs() < 1e-6);
}

/// The mouse-drag analogue of arrow-nudging a child inside a frame: when ONLY
/// the child is selected (e.g. picked in the layers panel, with no drill scope),
/// pressing and dragging on it must move the CHILD — not re-pick its frame via
/// container-first selection and drag the whole frame. The frame must stay put.
#[test]
fn dragging_an_already_selected_child_moves_the_child_not_its_frame() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Frame at +60x +20y; child at +15x +10y inside it → child world (75, 30).
    let (frame, child) = frame_with_child(&mut doc, DVec2::new(60.0, 20.0), DVec2::new(15.0, 10.0));
    // Only the child is selected; the tool has NO drill scope (fresh).
    doc.selection.select_only(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    let child_before = ctx.doc.scene.world_bounds(child).unwrap();
    let frame_before = ctx.doc.scene.get(frame).unwrap().transform;

    let press = screen_for_world(DVec2::new(75.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 40.0, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + 40.0, press[1]], ModifierKeys::empty()),
    );

    // The child moved by the requested +40 in world X…
    let child_after = ctx.doc.scene.world_bounds(child).unwrap();
    assert!(
        (child_after.min_x - child_before.min_x - 40.0).abs() < 1e-6,
        "child world x shifted by {}, expected 40",
        child_after.min_x - child_before.min_x
    );
    // …and the FRAME did not move (the bug dragged the whole frame instead).
    let frame_after = ctx.doc.scene.get(frame).unwrap().transform;
    assert_eq!(
        frame_after, frame_before,
        "the frame moved — container-first re-pick dragged the frame, not the child"
    );
    // The selection is still just the child (not re-pointed at the frame).
    assert_eq!(ctx.doc.selection.as_slice(), &[child]);
}

/// A flat selection of N sibling roots all translate by exactly the drag
/// delta — the alignment-safe pairing keeps every id matched to its own
/// press-time transform, so none inherits a neighbor's base.
#[test]
fn flat_selection_of_n_siblings_all_translate_by_delta() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Five sibling rects spread across distinct columns so the press
    // unambiguously grabs one of them and none overlaps another.
    let mut ids = Vec::new();
    let mut origins = Vec::new();
    for i in 0..5 {
        let x = -150.0 + i as f64 * 60.0;
        let id = rect_at(&mut doc, x, -10.0, 20.0, 20.0);
        origins.push(DVec2::new(x, -10.0));
        ids.push(id);
    }
    for &id in &ids {
        doc.selection.add(id);
    }
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Press on the third rect's center (world x = -150 + 120 + 10 = -20).
    let press = screen_for_world(DVec2::new(-20.0, 0.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    // Drag +35x, +0y (Y kept out so screen→world Y orientation can't muddy
    // the assert; the X path is the proven convention in the other tests).
    let dx = 35.0;
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + dx, press[1]], ModifierKeys::empty()),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([press[0] + dx, press[1]], ModifierKeys::empty()),
    );

    // Every sibling moved by exactly +dx in world X and not at all in Y.
    for (&id, origin) in ids.iter().zip(origins.iter()) {
        let b = ctx.doc.scene.world_bounds(id).unwrap();
        assert!(
            (b.min_x - (origin.x + dx)).abs() < 1e-6,
            "sibling at {origin:?} moved to min_x {}, expected {}",
            b.min_x,
            origin.x + dx
        );
        assert!((b.min_y - origin.y).abs() < 1e-6, "sibling y drifted");
    }
}

/// Aborting an in-flight move (Esc) restores every node — frame and child
/// alike — to its exact press-time transform, with no history entry.
#[test]
fn abort_move_restores_nested_selection_to_press_time() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let (frame, child) = frame_with_child(&mut doc, DVec2::new(60.0, 20.0), DVec2::new(15.0, 10.0));
    doc.selection.select_only(frame);
    doc.selection.add(child);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Record exact press-time transforms for both nodes.
    let frame_before = ctx.doc.scene.get(frame).unwrap().transform;
    let child_before = ctx.doc.scene.get(child).unwrap().transform;
    let undo_before = ctx.doc.history.undo_depth();

    let press = screen_for_world(DVec2::new(75.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([press[0] + 40.0, press[1] + 25.0], ModifierKeys::empty()),
    );
    // Esc aborts the gesture.
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
    );

    // Both transforms are bit-for-bit the press-time values.
    assert_eq!(ctx.doc.scene.get(frame).unwrap().transform, frame_before);
    assert_eq!(ctx.doc.scene.get(child).unwrap().transform, child_before);
    // No transaction was recorded for the aborted drag.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before);
    assert!(tool.is_idle());
}

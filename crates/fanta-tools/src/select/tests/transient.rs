use super::*;

// -------------------------------------------------------------------------
// Transient drag + cached candidates (perf rework)
// -------------------------------------------------------------------------

#[test]
fn multi_frame_move_commits_exactly_one_transaction() {
    // Many Move frames during one gesture must collapse into a SINGLE undo
    // step — the drag is transient and only the release commits.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let undo_before = doc.history.undo_depth();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    // 30 incremental Move frames out to +120 in X.
    for i in 1..=30 {
        let dx = (i as f64) * 4.0;
        tool.handle_event(
            &mut ctx,
            pe_move([center[0] + dx, center[1]], ModifierKeys::empty()),
        );
    }
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 120.0, center[1]], ModifierKeys::empty()),
    );
    // Exactly one transaction regardless of frame count.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    // And it landed at the final cursor position (no compounding).
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO).x - 120.0).abs() < 1e-6);
}

#[test]
fn drag_is_transient_no_history_growth_mid_gesture() {
    // While dragging, the undo depth must NOT grow — frames write the scene
    // directly, not through history. (Old behavior pushed an op per frame
    // onto an open transaction; the new behavior touches history only on
    // release.)
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let undo_before = doc.history.undo_depth();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    for i in 1..=10 {
        let dx = (i as f64) * 5.0;
        tool.handle_event(
            &mut ctx,
            pe_move([center[0] + dx, center[1]], ModifierKeys::empty()),
        );
        // Depth unchanged on every in-flight frame; the scene preview moved.
        assert_eq!(ctx.doc.history.undo_depth(), undo_before);
        let live = ctx.doc.scene.get(id).unwrap().transform;
        assert!((live.transform_point(DVec2::ZERO).x - dx).abs() < 1e-6);
    }
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 50.0, center[1]], ModifierKeys::empty()),
    );
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
}

#[test]
fn undo_after_long_drag_restores_origin_in_one_step() {
    // A single undo after a multi-frame drag returns the node to its
    // press-time transform — the whole gesture is one history entry.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    for i in 1..=20 {
        let d = (i as f64) * 3.0;
        tool.handle_event(
            &mut ctx,
            pe_move([center[0] + d, center[1] + d], ModifierKeys::empty()),
        );
    }
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 60.0, center[1] + 60.0], ModifierKeys::empty()),
    );
    // Moved.
    let moved = ctx.doc.scene.get(id).unwrap().transform;
    assert!((moved.transform_point(DVec2::ZERO).x - 60.0).abs() < 1e-6);
    // One undo restores the origin.
    assert!(ctx.doc.undo().unwrap());
    let restored = ctx.doc.scene.get(id).unwrap().transform;
    assert!(
        restored.transform_point(DVec2::ZERO).length() < 1e-6,
        "expected origin after one undo, got {:?}",
        restored.transform_point(DVec2::ZERO)
    );
}

#[test]
fn snap_candidates_are_cached_for_the_gesture_not_recomputed_per_frame() {
    // A stationary neighbor with its LEFT edge at world x=80, placed clear
    // of the origin so the press unambiguously grabs the dragged rect. We
    // drag the origin rect rightward toward that edge. Candidates are
    // collected once at the first Move; we then teleport the neighbor far
    // away. A per-frame recompute would lose the candidate and NOT snap —
    // with caching the snapshot still holds the old edge, so a later frame
    // still snaps to x=80, proving candidates are not re-collected.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Neighbor world bounds [80,-25,130,25] → left edge x=80 (does NOT
    // overlap the origin, so it can't steal the press hit-test).
    let neighbor = rect_at(&mut doc, 80.0, -25.0, 50.0, 50.0);
    // Dragged rect centered at world origin, 50x50 → right edge starts at 25.
    let dragged = rect_at_origin(&mut doc, 50.0, 50.0);
    doc.selection.select_only(dragged);
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
    // First move past threshold: this is where candidates are collected
    // (neighbor edges at x=80 and x=130 enter the cached snapshot).
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 5.0, center[1]], ModifierKeys::empty()),
    );
    // Teleport the neighbor far away — a fresh collect would drop its edge.
    ctx.doc.scene.get_mut(neighbor).unwrap().transform = Transform2D::translation(9000.0, 0.0);
    // Drag the origin rect's right edge to ~78 (press-time right edge 25 +
    // 53), 2 world units shy of the cached neighbor left edge at 80 — inside
    // the 6px threshold, so it should snap the right edge to 80.
    let r = tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 53.0, center[1]], ModifierKeys::empty()),
    );
    // The cached candidate (neighbor's old left edge at 80) still catches.
    let snapped_guide = r
        .overlays
        .iter()
        .any(|o| matches!(o, ToolOverlay::SnapGuide(_)));
    assert!(
        snapped_guide,
        "expected a snap guide from the cached candidate after the neighbor moved away"
    );
    // The dragged rect's right edge snapped exactly to the cached x=80.
    let bb = ctx.doc.scene.world_bounds(dragged).unwrap();
    assert!(
        (bb.max_x - 80.0).abs() < 1e-6,
        "dragged right edge should snap to cached x=80, got {}",
        bb.max_x
    );
}

#[test]
fn resize_drag_is_transient_and_commits_one_step() {
    // Mirror of the move tests for the resize gesture: many frames, depth
    // stays flat mid-drag, exactly one transaction on release, one-step
    // undo restores the original size.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0); // world [-30,-30,30,30]
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let undo_before = ctx.doc.history.undo_depth();

    let press = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    for i in 1..=15 {
        let w = 30.0 + (i as f64) * 2.0;
        let drag = screen_for_world(DVec2::new(w, w), ctx.viewport, ctx.screen_size);
        tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
        // No history growth during the resize drag.
        assert_eq!(ctx.doc.history.undo_depth(), undo_before);
    }
    let final_drag = screen_for_world(DVec2::new(60.0, 60.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_release(final_drag, ModifierKeys::empty()));
    // One transaction; one-step undo restores 60x60.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    assert!(ctx.doc.undo().unwrap());
    let restored = ctx.doc.scene.world_bounds(id).unwrap();
    assert!((restored.width() - 60.0).abs() < 1e-6);
    assert!((restored.height() - 60.0).abs() < 1e-6);
}

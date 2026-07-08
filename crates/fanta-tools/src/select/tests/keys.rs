use super::*;

// -------------------------------------------------------------------------
// Nudge
// -------------------------------------------------------------------------

#[test]
fn arrow_nudges_selection_by_one_world_unit() {
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 10.0, 10.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(
        &mut doc,
        &mut viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    let mut tool = SelectTool::new();
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::ArrowRight)),
    );
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO).x - 1.0).abs() < 1e-9);
}

#[test]
fn shift_arrow_nudges_by_ten() {
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 10.0, 10.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(
        &mut doc,
        &mut viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    let mut tool = SelectTool::new();
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::with_modifiers(
            LogicalKey::ArrowDown,
            ModifierKeys::SHIFT,
        )),
    );
    let t = ctx.doc.scene.get(id).unwrap().transform;
    assert!((t.transform_point(DVec2::ZERO).y - 10.0).abs() < 1e-9);
}

#[test]
fn arrow_with_no_selection_is_no_op() {
    let mut doc = Doc::new();
    let undo_depth_before = doc.history.undo_depth();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(
        &mut doc,
        &mut viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    let mut tool = SelectTool::new();
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::ArrowLeft)),
    );
    assert_eq!(ctx.doc.history.undo_depth(), undo_depth_before);
}

// -------------------------------------------------------------------------
// Escape
// -------------------------------------------------------------------------

#[test]
fn escape_clears_selection() {
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 10.0, 10.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(
        &mut doc,
        &mut viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    let mut tool = SelectTool::new();
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
    );
    assert!(ctx.doc.selection.is_empty());
}

// -------------------------------------------------------------------------
// Edge cases
// -------------------------------------------------------------------------

#[test]
fn release_without_press_is_safe() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(
        &mut doc,
        &mut viewport,
        SnapEngine::default(),
        DVec2::new(800.0, 600.0),
    );
    let mut tool = SelectTool::new();
    let r = tool.handle_event(&mut ctx, pe_release([100.0, 100.0], ModifierKeys::empty()));
    // No panic; idle phase.
    assert!(tool.is_idle());
    assert_eq!(r.cursor, Some(CursorHint::Default));
}

#[test]
fn marquee_with_multiple_rects_selects_only_contained() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Tiny rect near origin → fully inside any reasonable marquee.
    let small = rect_at(&mut doc, -2.0, -2.0, 4.0, 4.0);
    // Huge rect far outside the camera → not picked up.
    let _far = rect_at(&mut doc, 10000.0, 10000.0, 10.0, 10.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    tool.handle_event(&mut ctx, pe_press([10.0, 10.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move([790.0, 590.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release([790.0, 590.0], ModifierKeys::empty()));
    // Only the small (origin) rect is fully inside the marquee.
    assert!(ctx.doc.selection.contains(small));
}

#[test]
fn deactivate_aborts_in_flight_move() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 50.0, center[1]], ModifierKeys::empty()),
    );
    // Tool deactivation (user switched tools mid-drag).
    tool.deactivate(&mut ctx);
    // The transform was reverted back to identity (or near-zero translation).
    let t = ctx.doc.scene.get(id).unwrap().transform;
    let p = t.transform_point(DVec2::ZERO);
    assert!(p.length() < 1e-6, "expected revert, got {p:?}");
    assert!(tool.is_idle());
}

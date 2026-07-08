use super::*;

// -------------------------------------------------------------------------
// Marquee
// -------------------------------------------------------------------------

#[test]
fn drag_on_empty_starts_marquee() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    tool.handle_event(&mut ctx, pe_press([700.0, 500.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move([720.0, 510.0], ModifierKeys::empty()));
    assert!(tool.is_marquee_active());
}

#[test]
fn marquee_emits_screen_rect_overlay() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    tool.handle_event(&mut ctx, pe_press([700.0, 500.0], ModifierKeys::empty()));
    let r = tool.handle_event(&mut ctx, pe_move([720.0, 510.0], ModifierKeys::empty()));
    let has_marquee = r
        .overlays
        .iter()
        .any(|o| matches!(o, ToolOverlay::Marquee { .. }));
    assert!(has_marquee);
}

#[test]
fn marquee_release_selects_contained_nodes() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 20.0, 20.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    // Marquee that covers the full canvas — should include our small rect.
    tool.handle_event(&mut ctx, pe_press([10.0, 10.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move([790.0, 590.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release([790.0, 590.0], ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
    assert!(tool.is_idle());
}

#[test]
fn alt_marquee_uses_intersects_mode() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Big rect — should be missed by a tiny Contains marquee but caught by Intersects.
    let id = rect_at_origin(&mut doc, 100.0, 100.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::ALT));
    tool.handle_event(
        &mut ctx,
        pe_move([center[0] + 10.0, center[1] + 10.0], ModifierKeys::ALT),
    );
    tool.handle_event(
        &mut ctx,
        pe_release([center[0] + 10.0, center[1] + 10.0], ModifierKeys::ALT),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

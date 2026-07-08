use super::*;

// -------------------------------------------------------------------------
// Selection clicks
// -------------------------------------------------------------------------

#[test]
fn click_on_node_replaces_selection() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

#[test]
fn shift_click_toggles_membership() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let center = screen_center(size);
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::SHIFT));
    assert!(ctx.doc.selection.is_empty());
    // And toggle again puts it back.
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::SHIFT));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

#[test]
fn click_on_empty_clears_selection() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let _ = rect_at_origin(&mut doc, 10.0, 10.0);
    let id = doc.scene.roots()[0];
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    // Click outside the small 10x10 rect (its world bounds are tiny near
    // the origin; click at far screen corner).
    let far = [size.x - 1.0, size.y - 1.0];
    tool.handle_event(&mut ctx, pe_press(far, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(far, ModifierKeys::empty()));
    assert!(ctx.doc.selection.is_empty());
}

#[test]
fn click_outside_with_shift_preserves_selection() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 10.0, 10.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let far = [size.x - 1.0, size.y - 1.0];
    tool.handle_event(&mut ctx, pe_press(far, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(far, ModifierKeys::SHIFT));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

// -------------------------------------------------------------------------
// Active-page scoping (BUG 1)
// -------------------------------------------------------------------------

/// Two pages (root groups), each holding one rect that overlaps the other's
/// rect at the world origin — the `.fig` shared-origin layout. Returns
/// `(page_a, rect_a, page_b, rect_b)`.
fn two_overlapping_pages(doc: &mut Doc) -> (NodeId, NodeId, NodeId, NodeId) {
    let mut ga = CanvasNode::new(NodeData::Group(GroupNode::default()));
    ga.index = IndexKey::FIRST;
    let page_a = ga.id;
    doc.apply(Operation::create_node(ga)).unwrap();
    let mut ra = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    ra.parent = Some(page_a);
    ra.index = IndexKey::FIRST;
    let rect_a = ra.id;
    doc.apply(Operation::create_node(ra)).unwrap();

    let mut gb = CanvasNode::new(NodeData::Group(GroupNode::default()));
    gb.index = IndexKey::after(IndexKey::FIRST);
    let page_b = gb.id;
    doc.apply(Operation::create_node(gb)).unwrap();
    let mut rb = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::BLACK,
    )));
    rb.parent = Some(page_b);
    rb.index = IndexKey::FIRST;
    let rect_b = rb.id;
    doc.apply(Operation::create_node(rb)).unwrap();

    doc.add_page(page_a);
    doc.add_page(page_b);
    (page_a, rect_a, page_b, rect_b)
}

#[test]
fn click_only_hits_active_page_node() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let (page_a, rect_a, page_b, rect_b) = two_overlapping_pages(&mut doc);
    let center = screen_center(size);

    // Active page A: clicking the overlap region selects A's rect, never B's
    // (even though B was created later / paints on top).
    doc.set_active_page(Some(page_a));
    {
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
        let mut tool = SelectTool::new();
        tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::empty()));
        assert_eq!(ctx.doc.selection.as_slice(), &[rect_a]);
    }

    // Switch to page B: now the same click selects B's rect.
    doc.set_active_page(Some(page_b));
    {
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
        let mut tool = SelectTool::new();
        tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::empty()));
        assert_eq!(ctx.doc.selection.as_slice(), &[rect_b]);
    }
}

#[test]
fn marquee_only_collects_active_page_nodes() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let (page_a, rect_a, _page_b, rect_b) = two_overlapping_pages(&mut doc);
    doc.set_active_page(Some(page_a));

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    // Drag a marquee across the whole canvas, fully containing both rects.
    let p0 = [10.0, 10.0];
    let p1 = [size.x - 10.0, size.y - 10.0];
    tool.handle_event(&mut ctx, pe_press(p0, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(p1, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(p1, ModifierKeys::empty()));

    // Only page A's rect is collected; B's overlapping rect is invisible to
    // selection on this page.
    assert_eq!(ctx.doc.selection.as_slice(), &[rect_a]);
    assert!(!ctx.doc.selection.contains(rect_b));
}

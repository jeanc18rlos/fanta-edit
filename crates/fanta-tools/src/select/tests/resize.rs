use super::*;

// ------------------------------------------------------------------------
// Resize handle tests
// ------------------------------------------------------------------------

#[test]
fn press_on_se_handle_of_single_selection_enters_resize() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0); // world bounds [-30,-30,30,30]
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing(), "expected to be resizing");
}

#[test]
fn drag_se_handle_grows_node_world_bounds() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0); // world bounds [-30,-30,30,30]
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();

    let press_world = DVec2::new(30.0, 30.0);
    let press = screen_for_world(press_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));

    // Drag SE corner from (30, 30) to (50, 50) in world space.
    let drag_world = DVec2::new(50.0, 50.0);
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));

    let new_bounds = ctx.doc.scene.world_bounds(id).unwrap();
    // The SE corner should now sit near (50, 50); NW corner stays at (-30, -30).
    assert!((new_bounds.max_x - 50.0).abs() < 1e-6);
    assert!((new_bounds.max_y - 50.0).abs() < 1e-6);
    assert!((new_bounds.min_x - (-30.0)).abs() < 1e-6);
    assert!((new_bounds.min_y - (-30.0)).abs() < 1e-6);
}

#[test]
fn resizing_a_text_node_changes_its_box_not_its_transform_scale() {
    use fanta_doc::{TextAutoResize, TextNode};
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // A 60×60 text box at the origin (world bounds [-? ..] — local origin is the
    // top-left, so place it via transform so world bounds are [0,0,60,60]).
    let mut n = CanvasNode::new(NodeData::Text(TextNode::new("Hello world", 60.0, 60.0)));
    n.transform = Transform2D::translation(0.0, 0.0);
    n.index = IndexKey::FIRST;
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // Grab the SE handle (world (60,60)) and drag it to (120, 100).
    let press = screen_for_world(DVec2::new(60.0, 60.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(120.0, 100.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing());
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let node = ctx.doc.scene.get(id).unwrap();
    // The transform's linear part stayed a pure (scale-1) translation — NO glyph
    // stretch baked in. A vector-style resize would have set sx≈2, sy≈1.67.
    let comps = node.transform.to_components(); // [a, b, c, d, tx, ty]
    assert!(
        (comps[0] - 1.0).abs() < 1e-6 && (comps[3] - 1.0).abs() < 1e-6,
        "text transform picked up a scale {comps:?}; box resize must not stretch glyphs"
    );
    // The box grew via local_size instead.
    if let NodeData::Text(t) = &node.data {
        assert!(
            (t.local_size[0] - 120.0).abs() < 1e-6,
            "width box → {:?}",
            t.local_size
        );
        assert!(
            (t.local_size[1] - 100.0).abs() < 1e-6,
            "height box → {:?}",
            t.local_size
        );
        // A corner drag (affects Y) fixes the box so the new height is honored.
        assert_eq!(t.auto_resize, TextAutoResize::None);
    } else {
        panic!("node is no longer a text node");
    }
    // World bounds reflect the new box.
    let b = ctx.doc.scene.world_bounds(id).unwrap();
    assert!((b.width() - 120.0).abs() < 1e-6 && (b.height() - 100.0).abs() < 1e-6);
}

#[test]
fn release_after_resize_commits_one_undo_step() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let undo_before = ctx.doc.history.undo_depth();

    let press = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(60.0, 60.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    // One transaction queued; subsequent undo collapses the entire drag
    // (not each move-frame's mini-op) back to the original size.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    let _ = ctx.doc.undo();
    let restored = ctx.doc.scene.world_bounds(id).unwrap();
    assert!((restored.width() - 60.0).abs() < 1e-6);
}

#[test]
fn escape_during_resize_reverts() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();

    let press = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(80.0, 80.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(crate::event::KeyEvent::press(LogicalKey::Escape)),
    );
    let restored = ctx.doc.scene.world_bounds(id).unwrap();
    // Original is 60x60 at origin.
    assert!((restored.width() - 60.0).abs() < 1e-6);
    assert!((restored.height() - 60.0).abs() < 1e-6);
}

#[test]
fn press_on_handle_with_multiple_selection_falls_through_to_move() {
    // v0 limitation: resize requires single selection. With two, the
    // press should be treated as a normal node hit (move) or marquee.
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let a = rect_at_origin(&mut doc, 60.0, 60.0);
    let mut other = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    other.transform = Transform2D::translation(200.0, 0.0);
    other.index = IndexKey::from_raw(2.0);
    let b = other.id;
    doc.apply(Operation::create_node(other)).unwrap();
    doc.selection.select_only(a);
    doc.selection.add(b);

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(!tool.is_resizing(), "multi-selection should not resize");
}

#[test]
fn shift_resize_locks_aspect_ratio() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Build a 10x20 rect (1:2 aspect).
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -5.0,
        -10.0,
        10.0,
        20.0,
        Color::WHITE,
    )));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();

    // Press on SE corner at world (5, 10).
    let press = screen_for_world(DVec2::new(5.0, 10.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    // Drag with shift to (15, 12); without aspect lock this would be 20x22
    // (close to square). With lock, the X axis is the "winner" so we
    // stretch to 20x40 (1:2 preserved).
    let drag = screen_for_world(DVec2::new(15.0, 12.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::SHIFT));
    let b = ctx.doc.scene.world_bounds(id).unwrap();
    assert!(
        (b.width() * 2.0 - b.height()).abs() < 1e-6,
        "{}x{}",
        b.width(),
        b.height()
    );
}

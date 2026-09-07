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
fn resizing_nested_text_reflows_without_changing_the_glyph_transform() {
    use fanta_doc::{TextAutoResize, TextNode};
    use fanta_text::{LayoutEngine, TextBuffer, TextStyle};

    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([400.0, 240.0]),
        ..Default::default()
    }));
    parent.transform = Transform2D::translation(180.0, 90.0);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent))
        .expect("create parent frame");

    let content = "one two three four five six seven eight";
    let mut text = CanvasNode::new(NodeData::Text(TextNode::new(content, 160.0, 90.0)));
    text.parent = Some(parent_id);
    text.transform = Transform2D::from_components([1.25, 0.0, 0.0, 0.8, 30.0, 20.0]);
    let original_transform = text.transform;
    let text_id = text.id;
    doc.apply(Operation::create_node(text))
        .expect("create nested text");
    doc.selection.select_only(text_id);

    let original_world = doc
        .scene
        .world_transform(text_id)
        .expect("nested text world transform");
    let original_north_west = original_world.transform_point(DVec2::ZERO);
    let press_world = original_world.transform_point(DVec2::new(160.0, 45.0));
    let drag_world = original_world.transform_point(DVec2::new(70.0, 45.0));
    let buffer = TextBuffer::from_str(content, TextStyle::default());
    let layout_engine = LayoutEngine::new();
    let wide_line_count = layout_engine.layout(&buffer, 160.0).line_count();

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(press_world, ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing());
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let node = ctx.doc.scene.get(text_id).expect("nested text remains");
    assert_eq!(
        &node.transform.to_components()[..4],
        &original_transform.to_components()[..4],
        "box resize must preserve the glyph transform's complete linear part"
    );
    let NodeData::Text(text) = &node.data else {
        panic!("expected text node");
    };
    assert!((text.local_size[0] - 70.0).abs() < 1e-6);
    assert!((text.local_size[1] - 90.0).abs() < 1e-6);
    assert_eq!(text.auto_resize, TextAutoResize::Height);
    assert_eq!(text.style.size_px, 16.0);

    let narrow_line_count = layout_engine
        .layout(&buffer, text.local_size[0])
        .line_count();
    assert!(
        narrow_line_count > wide_line_count,
        "shrinking the box should reflow {wide_line_count} lines into more lines, got {narrow_line_count}"
    );
    let resized_world = ctx
        .doc
        .scene
        .world_transform(text_id)
        .expect("resized nested text world transform");
    assert!(
        (resized_world.transform_point(DVec2::ZERO) - original_north_west).length() < 1e-6,
        "dragging the east handle must keep the opposite edge anchored"
    );
}

#[test]
fn resizing_frame_changes_its_box_without_scaling_child_text() {
    use fanta_doc::TextNode;

    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 80.0]),
        ..Default::default()
    }));
    frame.transform = Transform2D::translation(-50.0, -40.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Hello", 60.0, 24.0)));
    text.parent = Some(frame_id);
    text.transform = Transform2D::translation(12.0, 16.0);
    let text_id = text.id;
    doc.apply(Operation::create_node(text)).unwrap();
    doc.selection.select_only(frame_id);
    let child_world_before = doc.scene.world_transform(text_id).unwrap();
    let undo_before = doc.history.undo_depth();

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(50.0, 40.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(150.0, 100.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing());
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let frame = ctx.doc.scene.get(frame_id).unwrap();
    let NodeData::Group(group) = &frame.data else {
        panic!("frame changed variant");
    };
    assert_eq!(group.clip_size, Some([200.0, 140.0]));
    assert_eq!(&frame.transform.to_components()[..4], &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        ctx.doc.scene.world_transform(text_id).unwrap(),
        child_world_before,
        "resizing the container box must not scale or move unconstrained text"
    );
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    assert!(ctx.doc.undo().unwrap());
    let NodeData::Group(group) = &ctx.doc.scene.get(frame_id).unwrap().data else {
        panic!("frame changed variant");
    };
    assert_eq!(group.clip_size, Some([100.0, 80.0]));
}

#[test]
fn resizing_plain_group_keeps_it_non_clipping_and_preserves_children() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([100.0, 80.0]),
        ..Default::default()
    }));
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    child.parent = Some(group_id);
    child.transform = Transform2D::translation(10.0, 10.0);
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    doc.selection.select_only(group_id);
    let child_transform = doc.scene.get(child_id).unwrap().transform;

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(100.0, 80.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(140.0, 120.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let NodeData::Group(group) = &ctx.doc.scene.get(group_id).unwrap().data else {
        panic!("group changed variant");
    };
    assert_eq!(group.clip_size, None);
    assert_eq!(group.local_size, Some([140.0, 120.0]));
    assert_eq!(
        ctx.doc.scene.get(child_id).unwrap().transform,
        child_transform
    );
}

#[test]
fn plain_group_resize_handles_stay_on_box_when_child_overflows() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([100.0, 80.0]),
        ..Default::default()
    }));
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    child.parent = Some(group_id);
    child.transform = Transform2D::translation(150.0, 10.0);
    doc.apply(Operation::create_node(child)).unwrap();
    doc.selection.select_only(group_id);

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(100.0, 80.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));

    assert!(
        tool.is_resizing(),
        "overflow bounds are for culling/hits; resize handles belong to the explicit box"
    );
}

#[test]
fn resizing_legacy_plain_group_materializes_a_box_without_scaling_text() {
    use fanta_doc::TextNode;

    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();

    let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Legacy", 40.0, 20.0)));
    text.parent = Some(group_id);
    text.transform = Transform2D::translation(20.0, 30.0);
    let text_id = text.id;
    doc.apply(Operation::create_node(text)).unwrap();
    doc.history = Default::default();
    doc.selection.select_only(group_id);
    let text_world_before = doc.scene.world_transform(text_id).unwrap();

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(60.0, 50.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(100.0, 80.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing());
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let group = ctx.doc.scene.get(group_id).unwrap();
    let NodeData::Group(group_data) = &group.data else {
        panic!("group changed variant");
    };
    assert_eq!(group_data.clip_size, None);
    assert_eq!(group_data.local_size, Some([80.0, 50.0]));
    assert_eq!(&group.transform.to_components()[..4], &[1.0, 0.0, 0.0, 1.0]);
    assert_eq!(
        ctx.doc.scene.world_transform(text_id).unwrap(),
        text_world_before,
        "normalizing the legacy content origin must preserve the text in world space"
    );
    assert_eq!(ctx.doc.history.undo_depth(), 1);
    assert!(ctx.doc.undo().unwrap());
    let NodeData::Group(group_data) = &ctx.doc.scene.get(group_id).unwrap().data else {
        panic!("group changed variant");
    };
    assert_eq!(group_data.local_size, None);
    assert_eq!(
        ctx.doc.scene.get(text_id).unwrap().transform,
        Transform2D::translation(20.0, 30.0)
    );
}

#[test]
fn resizing_auto_layout_frame_does_not_apply_legacy_child_constraints() {
    use fanta_doc::{AutoLayout, ConstraintH, ConstraintV, Constraints};

    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 80.0]),
        auto_layout: Some(AutoLayout::default()),
        ..Default::default()
    }));
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    child.parent = Some(frame_id);
    child.transform = Transform2D::translation(10.0, 12.0);
    child.constraints = Some(Constraints {
        horizontal: ConstraintH::Scale,
        vertical: ConstraintV::Scale,
    });
    let child_id = child.id;
    let child_transform = child.transform;
    doc.apply(Operation::create_node(child)).unwrap();
    doc.selection.select_only(frame_id);

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let press = screen_for_world(DVec2::new(100.0, 80.0), ctx.viewport, ctx.screen_size);
    let drag = screen_for_world(DVec2::new(200.0, 160.0), ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    assert_eq!(
        ctx.doc.scene.get(child_id).unwrap().transform,
        child_transform,
        "auto-layout owns child placement; legacy constraints must not run too"
    );
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

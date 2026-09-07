use super::*;

// ------------------------------------------------------------------------
// Rotation handle tests
// ------------------------------------------------------------------------

/// World→screen helper for a rotation zone's center, so a test can press
/// exactly on the rotate affordance.
fn rotate_zone_screen(
    handle: fanta_canvas::RotateHandle,
    world_bounds: Bounds,
    viewport: &Viewport,
    size: DVec2,
) -> [f64; 2] {
    let p = fanta_canvas::rotate_handle_screen_position(handle, world_bounds, viewport, size);
    [p.x, p.y]
}

#[test]
fn press_in_rotation_zone_enters_rotating() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0); // world [-30,-30,30,30]
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = SelectTool::new();
    let world = ctx.doc.scene.world_bounds(id).unwrap();
    let press = rotate_zone_screen(
        fanta_canvas::RotateHandle::SouthEast,
        world,
        ctx.viewport,
        ctx.screen_size,
    );
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_rotating(), "expected rotation phase");
    assert!(!tool.is_resizing(), "rotate zone must not trigger resize");
}

#[test]
fn rotate_drag_produces_expected_angle_in_one_undo_step() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let undo_before = ctx.doc.history.undo_depth();

    let world = ctx.doc.scene.world_bounds(id).unwrap();
    let pivot = world.center(); // origin
    // Press on the SE rotation zone — guaranteed to enter the Rotating phase.
    let zone = rotate_zone_screen(
        fanta_canvas::RotateHandle::SouthEast,
        world,
        ctx.viewport,
        ctx.screen_size,
    );
    tool.handle_event(&mut ctx, pe_press(zone, ModifierKeys::empty()));
    assert!(tool.is_rotating());

    // Drag to a world point 90° (in world coords) from the press ray about
    // the pivot. We know the press world point, so rotate it about pivot.
    let press_world_actual = current_press_world(&mut tool, &ctx);
    let v = press_world_actual - pivot;
    let rotated = DVec2::new(-v.y, v.x); // +90° world rotation
    let drag_world = pivot + rotated;
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    // The node's transform should now carry a ~90° rotation.
    let t = ctx.doc.scene.get(id).unwrap().transform;
    let angle = fanta_canvas::transform_angle(&t);
    assert!(
        (angle.abs() - std::f64::consts::FRAC_PI_2).abs() < 1e-3,
        "expected ~90° rotation, got {} deg",
        angle.to_degrees()
    );
    // Exactly one undo step for the whole rotate gesture.
    assert_eq!(ctx.doc.history.undo_depth(), undo_before + 1);
    // One undo restores the un-rotated transform.
    assert!(ctx.doc.undo().unwrap());
    let restored = fanta_canvas::transform_angle(&ctx.doc.scene.get(id).unwrap().transform);
    assert!(
        restored.abs() < 1e-6,
        "undo should clear rotation, got {restored}"
    );
}

#[test]
fn shift_rotate_snaps_to_15_degrees() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    let world = ctx.doc.scene.world_bounds(id).unwrap();
    let pivot = world.center();
    let zone = rotate_zone_screen(
        fanta_canvas::RotateHandle::SouthEast,
        world,
        ctx.viewport,
        ctx.screen_size,
    );
    tool.handle_event(&mut ctx, pe_press(zone, ModifierKeys::empty()));
    assert!(tool.is_rotating());

    // Sweep the press ray by ~20° world; with Shift the result snaps to 15°.
    let press_world = current_press_world(&mut tool, &ctx);
    let v = press_world - pivot;
    let ang = 20.0_f64.to_radians();
    let (s, c) = ang.sin_cos();
    let rotated = DVec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
    let drag_world = pivot + rotated;
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::SHIFT));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::SHIFT));

    let angle = fanta_canvas::transform_angle(&ctx.doc.scene.get(id).unwrap().transform);
    // The world screen->world Y flip can give ±15°; assert magnitude is a
    // clean 15° increment.
    let deg = angle.to_degrees().abs();
    assert!(
        (deg - 15.0).abs() < 1e-3,
        "expected snap to 15°, got {deg} deg"
    );
}

#[test]
fn escape_during_rotate_reverts() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let before = ctx.doc.scene.get(id).unwrap().transform;
    let undo_before = ctx.doc.history.undo_depth();

    let world = ctx.doc.scene.world_bounds(id).unwrap();
    let pivot = world.center();
    let zone = rotate_zone_screen(
        fanta_canvas::RotateHandle::SouthEast,
        world,
        ctx.viewport,
        ctx.screen_size,
    );
    tool.handle_event(&mut ctx, pe_press(zone, ModifierKeys::empty()));
    let press_world = current_press_world(&mut tool, &ctx);
    let v = press_world - pivot;
    let drag_world = pivot + DVec2::new(-v.y, v.x); // +90°
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
    );

    // Transform restored bit-for-bit, no transaction recorded.
    assert_eq!(ctx.doc.scene.get(id).unwrap().transform, before);
    assert_eq!(ctx.doc.history.undo_depth(), undo_before);
    assert!(tool.is_idle());
}

#[test]
fn nested_rotation_is_rigid_in_world_space_under_a_scaled_parent() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([300.0, 200.0]),
        ..Default::default()
    }));
    parent.transform = Transform2D::from_components([2.0, 0.0, 0.0, 0.75, -100.0, -75.0]);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        60.0,
        40.0,
        Color::WHITE,
    )));
    child.parent = Some(parent_id);
    child.transform = Transform2D::translation(40.0, 50.0);
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    doc.selection.select_only(child_id);
    let world_before = doc.scene.world_transform(child_id).unwrap();

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();
    let local = ctx.doc.scene.local_bounds(child_id).unwrap();
    let corner_world = world_before.transform_point(DVec2::new(local.max_x, local.max_y));
    let pivot = world_before.transform_point(local.center());
    let world_bounds = ctx.doc.scene.world_bounds(child_id).unwrap();
    let press = rotate_zone_screen(
        fanta_canvas::RotateHandle::SouthEast,
        world_bounds,
        ctx.viewport,
        ctx.screen_size,
    );
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(
        tool.is_rotating(),
        "expected rotate zone beyond {corner_world:?}"
    );
    let captured = current_press_world(&mut tool, &ctx);
    let ray = captured - pivot;
    let drag_world = pivot + DVec2::new(-ray.y, ray.x);
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    let world_after = ctx.doc.scene.world_transform(child_id).unwrap();
    let expected = world_before.then(&fanta_canvas::rotate_about(
        pivot,
        std::f64::consts::FRAC_PI_2,
    ));
    for (actual, expected) in world_after
        .to_components()
        .iter()
        .zip(expected.to_components())
    {
        assert!(
            (actual - expected).abs() < 1e-6,
            "{world_after:?} != {expected:?}"
        );
    }
}

#[test]
fn resize_keeps_existing_rotation_through_the_tool() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    // Build a 40x40 rect and pre-rotate it 90° about its center.
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -20.0,
        -20.0,
        40.0,
        40.0,
        Color::WHITE,
    )));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    let theta = std::f64::consts::FRAC_PI_2; // 90°
    doc.scene.get_mut(id).unwrap().transform = Transform2D::rotation(theta);
    doc.scene.invalidate_world_cache();
    doc.selection.select_only(id);

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size);
    let mut tool = SelectTool::new();

    // The node's local SE corner (20,20) maps under 90° rotation to a world
    // point; we press there (the resize handle) and drag outward along the
    // node's rotated axes.
    let original = ctx.doc.scene.get(id).unwrap().transform;
    let se_world = original.transform_point(DVec2::new(20.0, 20.0));
    let press = screen_for_world(se_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_press(press, ModifierKeys::empty()));
    assert!(tool.is_resizing(), "expected resize on the corner handle");

    // Drag the corner further out along the rotated +diagonal.
    let drag_world = original.transform_point(DVec2::new(40.0, 40.0));
    let drag = screen_for_world(drag_world, ctx.viewport, ctx.screen_size);
    tool.handle_event(&mut ctx, pe_move(drag, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(drag, ModifierKeys::empty()));

    // Rotation is preserved — the resized transform still rotates 90°.
    let after = ctx.doc.scene.get(id).unwrap().transform;
    let a = fanta_canvas::transform_angle(&after);
    assert!(
        (a - theta).abs() < 1e-3,
        "resize dropped rotation: angle is now {} deg (expected 90)",
        a.to_degrees()
    );
}

/// Read the press-time world point the active rotation gesture captured.
/// Tests need this to compute a target drag that produces a known sweep.
fn current_press_world(tool: &mut SelectTool, _ctx: &ToolContext) -> DVec2 {
    match &tool.phase {
        Phase::Rotating(state) => state.press_world,
        _ => panic!("not rotating"),
    }
}

use super::*;
use fanta_doc::{EditingRules, Scope};

// ------------------------------------------------------------------------
// Fill mode: a template's disabled scopes are off limits
// ------------------------------------------------------------------------

fn disable(doc: &mut Doc, id: NodeId, scopes: &[Scope]) {
    let rules = EditingRules {
        disabled_scopes: scopes.iter().copied().collect(),
        placeholder: None,
    };
    doc.apply(Operation::SetEditingRules {
        id,
        old: None,
        new: Some(Box::new(rules)),
    })
    .expect("editing rules");
}

fn drag_right(tool: &mut SelectTool, ctx: &mut ToolContext, size: DVec2) {
    let center = screen_center(size);
    let moved = [center[0] + 50.0, center[1]];
    tool.handle_event(ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(ctx, pe_move(moved, ModifierKeys::empty()));
    tool.handle_event(ctx, pe_release(moved, ModifierKeys::empty()));
}

fn origin_x(ctx: &ToolContext, id: NodeId) -> f64 {
    ctx.doc
        .scene
        .get(id)
        .expect("node")
        .transform
        .transform_point(DVec2::ZERO)
        .x
}

#[test]
fn fill_mode_keeps_a_layer_whose_move_is_disabled_in_place() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0);
    disable(&mut doc, id, &[Scope::LayerMove]);
    let mut viewport = Viewport::default();
    let mut ctx =
        ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size).with_template_fill(true);
    let mut tool = SelectTool::new();

    drag_right(&mut tool, &mut ctx, size);
    assert!(origin_x(&ctx, id).abs() < 1e-6);
    assert!(ctx.doc.selection.contains(id), "the press still selects");

    ctx.template_fill = false;
    drag_right(&mut tool, &mut ctx, size);
    assert!(
        (origin_x(&ctx, id) - 50.0).abs() < 1e-6,
        "authoring moves it"
    );
}

#[test]
fn fill_mode_clicks_through_a_layer_whose_selection_is_disabled() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let below = rect_at_origin(&mut doc, 60.0, 60.0);
    // Siblings with equal index keys fall back to id order, which is random
    // for ids minted in the same millisecond, so give the top layer its own.
    let mut upper = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    upper.index = IndexKey::after(IndexKey::FIRST);
    let above = upper.id;
    doc.apply(Operation::create_node(upper))
        .expect("create the top layer");
    disable(&mut doc, above, &[Scope::EditorSelect]);
    let mut viewport = Viewport::default();
    let mut ctx =
        ToolContext::new(&mut doc, &mut viewport, no_snap_engine(), size).with_template_fill(true);
    let mut tool = SelectTool::new();
    let center = screen_center(size);

    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::empty()));
    assert!(ctx.doc.selection.contains(below));
    assert!(!ctx.doc.selection.contains(above));

    // A press keeps a selected layer under the cursor as its target, so
    // start the authoring click from an empty selection.
    ctx.template_fill = false;
    ctx.doc.selection.clear();
    tool.handle_event(&mut ctx, pe_press(center, ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release(center, ModifierKeys::empty()));
    assert!(
        ctx.doc.selection.contains(above),
        "authoring selects the top layer"
    );
}

#[test]
fn fill_mode_refuses_to_resize_a_layer_whose_resize_is_disabled() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at_origin(&mut doc, 60.0, 60.0); // world bounds [-30,-30,30,30]
    disable(&mut doc, id, &[Scope::LayerResize]);
    doc.selection.select_only(id);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size)
        .with_template_fill(true);
    let mut tool = SelectTool::new();
    let handle = screen_for_world(DVec2::new(30.0, 30.0), ctx.viewport, ctx.screen_size);

    tool.handle_event(&mut ctx, pe_press(handle, ModifierKeys::empty()));
    assert!(!tool.is_resizing());
    tool.handle_event(&mut ctx, pe_release(handle, ModifierKeys::empty()));

    ctx.template_fill = false;
    tool.handle_event(&mut ctx, pe_press(handle, ModifierKeys::empty()));
    assert!(tool.is_resizing(), "authoring resizes it");
}

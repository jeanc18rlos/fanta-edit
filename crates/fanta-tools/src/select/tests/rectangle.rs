use super::*;
use crate::select::{RectangleSelectTool, RectangleSelectionOperation};

fn select_rectangle(
    tool: &mut RectangleSelectTool,
    ctx: &mut ToolContext<'_>,
    first: [f64; 2],
    last: [f64; 2],
    modifiers: ModifierKeys,
) {
    tool.handle_event(ctx, pe_press(first, modifiers));
    tool.handle_event(ctx, pe_move(last, modifiers));
    tool.handle_event(ctx, pe_release(last, modifiers));
}

#[test]
fn rectangle_drag_selects_without_moving_a_node_under_the_press() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at(&mut doc, -20.0, -20.0, 40.0, 40.0);
    let original_transform = doc.scene.get(id).expect("rectangle").transform;
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();

    tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::ALT));
    let response = tool.handle_event(&mut ctx, pe_move([430.0, 330.0], ModifierKeys::ALT));
    assert!(
        response
            .overlays
            .iter()
            .any(|overlay| matches!(overlay, ToolOverlay::Marquee { .. }))
    );
    tool.handle_event(&mut ctx, pe_release([430.0, 330.0], ModifierKeys::ALT));

    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
    assert_eq!(
        ctx.doc.scene.get(id).expect("rectangle").transform,
        original_transform
    );
}

#[test]
fn rectangle_selection_operations_apply_to_current_vector_selection() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let left = rect_at(&mut doc, -110.0, -10.0, 20.0, 20.0);
    let right = rect_at(&mut doc, 90.0, -10.0, 20.0, 20.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();

    select_rectangle(
        &mut tool,
        &mut ctx,
        [285.0, 285.0],
        [315.0, 315.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[left]);

    ctx.rectangle_selection_operation = RectangleSelectionOperation::Add;
    select_rectangle(
        &mut tool,
        &mut ctx,
        [485.0, 285.0],
        [515.0, 315.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[left, right]);

    ctx.rectangle_selection_operation = RectangleSelectionOperation::Subtract;
    select_rectangle(
        &mut tool,
        &mut ctx,
        [285.0, 285.0],
        [315.0, 315.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[right]);

    ctx.rectangle_selection_operation = RectangleSelectionOperation::Add;
    select_rectangle(
        &mut tool,
        &mut ctx,
        [285.0, 285.0],
        [315.0, 315.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[right, left]);

    ctx.rectangle_selection_operation = RectangleSelectionOperation::Intersect;
    select_rectangle(
        &mut tool,
        &mut ctx,
        [285.0, 285.0],
        [315.0, 315.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[left]);
}

#[test]
fn rectangle_select_click_and_cancel_are_non_destructive() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let id = rect_at(&mut doc, -20.0, -20.0, 40.0, 40.0);
    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();

    tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);

    tool.handle_event(&mut ctx, pe_press([340.0, 240.0], ModifierKeys::empty()));
    tool.handle_event(&mut ctx, pe_move([460.0, 360.0], ModifierKeys::empty()));
    let response = tool.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent {
            key: LogicalKey::Escape,
            modifiers: ModifierKeys::empty(),
        }),
    );
    assert!(!response.wants_exit);
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

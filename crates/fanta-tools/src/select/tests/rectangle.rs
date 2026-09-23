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
fn rectangle_marquee_selects_the_visible_frame_containing_a_child() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    doc.apply(Operation::create_node(page))
        .expect("create page");
    doc.add_page(page_id);
    doc.set_active_page(Some(page_id));

    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([341.0, 153.0]),
        ..GroupNode::default()
    }));
    frame.parent = Some(page_id);
    frame.transform = Transform2D::translation(-168.0, 143.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame))
        .expect("create frame");

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        100.0,
        40.0,
        80.0,
        60.0,
        Color::rgba(255, 0, 0, 255),
    )));
    child.parent = Some(frame_id);
    let child_id = child.id;
    doc.apply(Operation::create_node(child))
        .expect("create frame child");

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();
    select_rectangle(
        &mut tool,
        &mut ctx,
        [360.0, 500.0],
        [360.0, 500.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_id]);

    select_rectangle(
        &mut tool,
        &mut ctx,
        [225.0, 430.0],
        [578.0, 610.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_id]);
    assert!(!ctx.doc.selection.contains(child_id));

    select_rectangle(
        &mut tool,
        &mut ctx,
        [325.0, 475.0],
        [420.0, 550.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[child_id]);
}

#[test]
fn rectangle_marquee_selects_an_empty_frame_surface() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    let page_id = page.id;
    doc.apply(Operation::create_node(page))
        .expect("create page");
    doc.add_page(page_id);
    doc.set_active_page(Some(page_id));

    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([341.0, 153.0]),
        ..GroupNode::default()
    }));
    frame.parent = Some(page_id);
    frame.transform = Transform2D::translation(-168.0, 143.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame))
        .expect("create empty frame");

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();
    select_rectangle(
        &mut tool,
        &mut ctx,
        [225.0, 430.0],
        [578.0, 610.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[frame_id]);
}

#[test]
fn rectangle_marquee_does_not_select_a_page_backdrop_without_active_scope() {
    let size = DVec2::new(800.0, 600.0);
    let mut doc = Doc::new();
    let page = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([300.0, 200.0]),
        ..GroupNode::default()
    }));
    let page_id = page.id;
    doc.apply(Operation::create_node(page))
        .expect("create page");
    doc.add_page(page_id);
    assert!(doc.set_active_page(None));

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        50.0,
        50.0,
        50.0,
        50.0,
        Color::WHITE,
    )));
    child.parent = Some(page_id);
    let child_id = child.id;
    doc.apply(Operation::create_node(child))
        .expect("create page child");

    let mut viewport = Viewport::default();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
    let mut tool = RectangleSelectTool::new();
    select_rectangle(
        &mut tool,
        &mut ctx,
        [390.0, 290.0],
        [710.0, 510.0],
        ModifierKeys::empty(),
    );
    assert_eq!(ctx.doc.selection.as_slice(), &[child_id]);
    assert!(!ctx.doc.selection.contains(page_id));
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

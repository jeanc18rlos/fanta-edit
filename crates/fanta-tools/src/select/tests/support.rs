use super::*;

pub(crate) fn screen_center(size: DVec2) -> [f64; 2] {
    [size.x * 0.5, size.y * 0.5]
}

pub(crate) fn pe_press(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Press {
        screen,
        button: Button::Primary,
        modifiers,
        count: 1,
    })
}

/// A primary press flagged as a double-click (`count = 2`) — drives the
/// select tool's "drill into a frame" without needing real press timing.
pub(crate) fn pe_double_press(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Press {
        screen,
        button: Button::Primary,
        modifiers,
        count: 2,
    })
}

pub(crate) fn pe_move(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Move { screen, modifiers })
}

pub(crate) fn pe_release(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Release {
        screen,
        button: Button::Primary,
        modifiers,
    })
}

pub(crate) fn rect_at_origin(doc: &mut Doc, w: f64, h: f64) -> NodeId {
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -w * 0.5,
        -h * 0.5,
        w,
        h,
        Color::WHITE,
    )));
    n.index = IndexKey::FIRST;
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    id
}

pub(crate) fn rect_at(doc: &mut Doc, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        x,
        y,
        w,
        h,
        Color::WHITE,
    )));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();
    id
}

/// Build a group ("frame") at the origin holding one child rect, returning
/// `(frame_id, child_id)`. The frame carries `frame_offset` as its local
/// translation and the child sits at `child_local` inside the frame, so the
/// child's world position is the sum — exactly the nested-design layout that
/// triggers the double-move bug when both are selected and dragged.
pub(crate) fn frame_with_child(
    doc: &mut Doc,
    frame_offset: DVec2,
    child_local: DVec2,
) -> (NodeId, NodeId) {
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
    frame.transform = Transform2D::translation(frame_offset.x, frame_offset.y);
    frame.index = IndexKey::FIRST;
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    child.parent = Some(frame_id);
    child.transform = Transform2D::translation(child_local.x, child_local.y);
    child.index = IndexKey::FIRST;
    let child_id = child.id;
    doc.apply(Operation::create_node(child)).unwrap();
    (frame_id, child_id)
}

/// Build the no-snap engine used by the transient-drag tests so the move
/// math is deterministic (pure cursor delta, no candidate pull).
pub(crate) fn no_snap_engine() -> SnapEngine {
    SnapEngine {
        zoom: 1.0,
        targets: fanta_canvas::SnapTargets::empty(),
        ..Default::default()
    }
}

pub(crate) fn screen_for_world(world: DVec2, viewport: &Viewport, size: DVec2) -> [f64; 2] {
    let s = fanta_canvas::world_to_screen(world, viewport, size);
    [s.x, s.y]
}

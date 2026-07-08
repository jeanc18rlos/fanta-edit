//! Cross-tool end-to-end flows.
//!
//! Each test stitches multiple tools together — create a shape, select it,
//! move it, pan the viewport while editing, etc. — to exercise the contract
//! between [`Tool`], [`ToolContext`], and the doc/canvas crates as a whole.
//!
//! These tests are intentionally less granular than the per-tool suites. If
//! one fails, the per-tool test failure should point at the actual bug; this
//! file catches integration-level regressions (e.g. "tools left history in a
//! bad state" or "viewport state leaked between gestures").
//!
//! [`Tool`]: fanta_tools::Tool
//! [`ToolContext`]: fanta_tools::ToolContext

use fanta_canvas::{SnapEngine, SnapTargets};
use fanta_doc::{Doc, Viewport};
use fanta_tools::{
    Button, EllipseTool, HandTool, KeyEvent, LineTool, LogicalKey, ModifierKeys, PointerEvent,
    RectTool, SelectTool, Tool, ToolContext, ToolEvent, ToolOverlay,
};
use glam::DVec2;

fn pe_press(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Press {
        screen,
        button: Button::Primary,
        modifiers,
        count: 1,
    })
}
fn pe_move(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Move { screen, modifiers })
}
fn pe_release(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
    ToolEvent::Pointer(PointerEvent::Release {
        screen,
        button: Button::Primary,
        modifiers,
    })
}

fn snap_disabled() -> SnapEngine {
    // Disable every snap target so geometry asserts measure raw deltas.
    SnapEngine {
        zoom: 1.0,
        targets: SnapTargets::empty(),
        ..Default::default()
    }
}

// =============================================================================
// Flow 1: Create a rectangle, then drag it to a new position.
// =============================================================================

#[test]
fn flow_rect_then_select_then_move() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap_disabled(), size);

    // 1) Rect tool creates a 100x60 rectangle starting at world (0,0).
    let mut rect = RectTool::new();
    rect.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
    rect.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
    let resp = rect.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
    assert!(resp.wants_exit);
    assert_eq!(ctx.doc.scene.len(), 1);
    let id = ctx.doc.scene.roots()[0];
    // The new node should already be selected.
    assert!(ctx.doc.selection.contains(id));

    // 2) Select tool drags the same node to the right by 50 px.
    let mut select = SelectTool::new();
    // Press inside the rect — its bounds are world (0,0) to (100,60), screen
    // (400,300) to (500,360). Center is around screen (450, 330).
    let pick = [450.0, 330.0];
    select.handle_event(&mut ctx, pe_press(pick, ModifierKeys::empty()));
    select.handle_event(
        &mut ctx,
        pe_move([pick[0] + 50.0, pick[1]], ModifierKeys::empty()),
    );
    select.handle_event(
        &mut ctx,
        pe_release([pick[0] + 50.0, pick[1]], ModifierKeys::empty()),
    );

    let bb = ctx.doc.scene.world_bounds(id).unwrap();
    // Original world rect min_x=0; after +50 world units min_x=50.
    assert!((bb.min_x - 50.0).abs() < 1e-6, "min_x = {}", bb.min_x);
}

// =============================================================================
// Flow 2: Pan the viewport, then click on a node — the hit-test must respect
// the new viewport state.
// =============================================================================

#[test]
fn flow_pan_viewport_then_select_node_at_new_screen_position() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);

    // Pre-existing rect at world (0,0) sized 60x60.
    use fanta_doc::{CanvasNode, Color, NodeData, Operation, VectorNode};
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::WHITE,
    )));
    let id = n.id;
    doc.apply(Operation::create_node(n)).unwrap();

    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap_disabled(), size);

    // Pan the viewport 100 px to the right.
    let mut hand = HandTool::new();
    hand.handle_event(&mut ctx, pe_press([100.0, 100.0], ModifierKeys::empty()));
    hand.handle_event(&mut ctx, pe_move([200.0, 100.0], ModifierKeys::empty()));
    hand.handle_event(&mut ctx, pe_release([200.0, 100.0], ModifierKeys::empty()));

    // The world origin used to be at screen center (400,300). After a 100 px
    // right pan, the world origin should now be at (500,300) on screen.
    let mut select = SelectTool::new();
    select.handle_event(&mut ctx, pe_press([500.0, 300.0], ModifierKeys::empty()));
    select.handle_event(&mut ctx, pe_release([500.0, 300.0], ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.as_slice(), &[id]);
}

// =============================================================================
// Flow 3: Marquee-select multiple shapes created with different tools, then
// nudge the group via arrow keys.
// =============================================================================

#[test]
fn flow_marquee_select_mixed_shapes_then_nudge() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap_disabled(), size);

    // Create a rectangle.
    let mut rect = RectTool::new();
    rect.handle_event(&mut ctx, pe_press([350.0, 280.0], ModifierKeys::empty()));
    rect.handle_event(&mut ctx, pe_move([400.0, 320.0], ModifierKeys::empty()));
    rect.handle_event(&mut ctx, pe_release([400.0, 320.0], ModifierKeys::empty()));

    // Create an ellipse next to it.
    let mut ellipse = EllipseTool::new();
    ellipse.handle_event(&mut ctx, pe_press([420.0, 280.0], ModifierKeys::empty()));
    ellipse.handle_event(&mut ctx, pe_move([470.0, 320.0], ModifierKeys::empty()));
    ellipse.handle_event(&mut ctx, pe_release([470.0, 320.0], ModifierKeys::empty()));

    assert_eq!(ctx.doc.scene.len(), 2);

    // Marquee-select everything.
    let mut select = SelectTool::new();
    select.handle_event(&mut ctx, pe_press([100.0, 100.0], ModifierKeys::empty()));
    select.handle_event(&mut ctx, pe_move([700.0, 500.0], ModifierKeys::empty()));
    select.handle_event(&mut ctx, pe_release([700.0, 500.0], ModifierKeys::empty()));
    assert_eq!(ctx.doc.selection.len(), 2);

    // Capture pre-nudge positions to compare against post-nudge.
    let ids: Vec<_> = ctx.doc.selection.iter().copied().collect();
    let centers_before: Vec<DVec2> = ids
        .iter()
        .map(|id| ctx.doc.scene.world_bounds(*id).unwrap().center())
        .collect();

    // Shift-ArrowDown nudges by 10.
    select.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::with_modifiers(
            LogicalKey::ArrowDown,
            ModifierKeys::SHIFT,
        )),
    );

    for (id, c_before) in ids.iter().zip(centers_before.iter()) {
        let c_after = ctx.doc.scene.world_bounds(*id).unwrap().center();
        assert!((c_after.y - c_before.y - 10.0).abs() < 1e-6);
        assert!((c_after.x - c_before.x).abs() < 1e-9);
    }
}

// =============================================================================
// Flow 4: Drag a line, then escape mid-drag — confirm the doc and history are
// untouched.
// =============================================================================

#[test]
fn flow_line_then_escape_leaves_doc_pristine() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);
    let nodes_before = doc.scene.len();
    let undo_before = doc.history.undo_depth();
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap_disabled(), size);

    let mut line = LineTool::new();
    line.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
    line.handle_event(&mut ctx, pe_move([500.0, 350.0], ModifierKeys::empty()));
    line.handle_event(
        &mut ctx,
        ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
    );

    assert_eq!(ctx.doc.scene.len(), nodes_before);
    assert_eq!(ctx.doc.history.undo_depth(), undo_before);
    assert!(!line.is_drafting());
}

// =============================================================================
// Flow 5: Create three shapes, then verify the move-and-undo cycle reverses
// exactly one user gesture.
// =============================================================================

#[test]
fn flow_create_and_undo_returns_to_prior_state() {
    let mut doc = Doc::new();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap_disabled(), size);

    // Create a rect.
    let mut rect = RectTool::new();
    rect.handle_event(&mut ctx, pe_press([350.0, 280.0], ModifierKeys::empty()));
    rect.handle_event(&mut ctx, pe_move([400.0, 320.0], ModifierKeys::empty()));
    rect.handle_event(&mut ctx, pe_release([400.0, 320.0], ModifierKeys::empty()));
    assert_eq!(ctx.doc.scene.len(), 1);

    // Select & move it.
    let mut select = SelectTool::new();
    let pick = [375.0, 300.0];
    select.handle_event(&mut ctx, pe_press(pick, ModifierKeys::empty()));
    select.handle_event(
        &mut ctx,
        pe_move([pick[0] + 50.0, pick[1]], ModifierKeys::empty()),
    );
    select.handle_event(
        &mut ctx,
        pe_release([pick[0] + 50.0, pick[1]], ModifierKeys::empty()),
    );

    let id = ctx.doc.scene.roots()[0];
    let bb_after_move = ctx.doc.scene.world_bounds(id).unwrap();
    // The original rect went from world (-50,-20) to (0,20). After a +50 x
    // shift the new min_x should be 0.
    assert!(
        (bb_after_move.min_x - 0.0).abs() < 1e-6,
        "min_x = {}",
        bb_after_move.min_x
    );

    // Undo: reverts the move. Node still exists.
    assert!(ctx.doc.undo().unwrap());
    let bb_after_undo = ctx.doc.scene.world_bounds(id).unwrap();
    // After undo the rect is back at min_x = -50.
    assert!(
        (bb_after_undo.min_x - (-50.0)).abs() < 1e-6,
        "after undo min_x = {}",
        bb_after_undo.min_x
    );

    // Undo again: reverts the create. Scene empty.
    assert!(ctx.doc.undo().unwrap());
    assert_eq!(ctx.doc.scene.len(), 0);
}

// =============================================================================
// Flow 6: Rect tool emits a preview overlay while dragging, then a snap
// guide once snap targets surface.
// =============================================================================

#[test]
fn flow_rect_overlay_and_snap_guide_emit_during_drag() {
    use fanta_doc::{CanvasNode, Color, NodeData, Operation, VectorNode};

    let mut doc = Doc::new();
    // Pre-existing neighbor at world (100, 0)..(150, 50). Drawing near 100
    // should produce an edge-snap candidate.
    let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        100.0,
        0.0,
        50.0,
        50.0,
        Color::WHITE,
    )));
    doc.apply(Operation::create_node(n)).unwrap();
    let mut viewport = Viewport::default();
    let size = DVec2::new(800.0, 600.0);
    // Use a snap engine with edge targets enabled; thresholds are screen-px.
    let snap = SnapEngine {
        zoom: 1.0,
        targets: SnapTargets::NODE_EDGES,
        ..Default::default()
    };
    let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);

    let mut rect = RectTool::new();
    rect.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
    // Drag the cursor to where the neighbor's left edge sits (world x = 100,
    // screen x = 400 + 100 = 500). Tiny offset to invite the snap engine.
    let response = rect.handle_event(&mut ctx, pe_move([501.0, 350.0], ModifierKeys::empty()));

    let has_preview = response
        .overlays
        .iter()
        .any(|o| matches!(o, ToolOverlay::PreviewRect { .. }));
    assert!(has_preview);
    let has_guide = response
        .overlays
        .iter()
        .any(|o| matches!(o, ToolOverlay::SnapGuide(_)));
    assert!(
        has_guide,
        "expected snap guide overlay near neighbor's edge"
    );
}

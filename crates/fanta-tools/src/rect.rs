//! Rectangle creation tool.
//!
//! ## Behavior
//!
//! - **Press** records the origin (snapped via the engine, no exclusions because
//!   the new node doesn't exist yet).
//! - **Move** updates the in-progress rect's other corner. Tool state holds the
//!   draft; the doc is untouched until release.
//! - **Release** commits the rect as a single [`Operation::CreateNode`].
//! - **Shift** during drag constrains to a square (matched to Figma).
//! - **Alt** during drag draws from the origin as the rect's *center* (the
//!   common "from center" convention; Figma toggles this with Option).
//! - **Escape** mid-drag clears the draft without mutating the doc.
//!
//! After commit, the tool sets `wants_exit = true` — Figma's "one shape per
//! tool selection" convention. The shell decides whether to honor that or
//! keep the tool active for repeated shapes.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, NodeData, Operation, VectorNode};
use glam::DVec2;

/// In-flight drag state for the rect tool.
#[derive(Debug, Clone, Copy)]
struct Draft {
    /// World-space origin (snapped at press time, never mutates during drag).
    origin: DVec2,
    /// World-space current cursor position (snapped at move time).
    current: DVec2,
}

impl Draft {
    /// Compute the world rect for the draft, respecting Shift (square) and
    /// Alt (from-center) modifiers.
    fn rect(&self, modifiers: ModifierKeys) -> Bounds {
        let mut dx = self.current.x - self.origin.x;
        let mut dy = self.current.y - self.origin.y;
        if modifiers.contains(ModifierKeys::SHIFT) {
            let m = dx.abs().max(dy.abs());
            dx = m.copysign(if dx == 0.0 { 1.0 } else { dx });
            dy = m.copysign(if dy == 0.0 { 1.0 } else { dy });
        }
        if modifiers.contains(ModifierKeys::ALT) {
            // Origin is the rect's *center*; the cursor sets one corner.
            let half = DVec2::new(dx, dy);
            let min = self.origin - half;
            let max = self.origin + half;
            Bounds::from_min_max(min.min(max), min.max(max))
        } else {
            let a = self.origin;
            let b = self.origin + DVec2::new(dx, dy);
            Bounds::from_min_max(a.min(b), a.max(b))
        }
    }
}

/// State machine for the rectangle tool.
#[derive(Debug, Default)]
pub struct RectTool {
    draft: Option<Draft>,
}

impl RectTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the tool is mid-drag.
    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for RectTool {
    fn name(&self) -> &'static str {
        "rect"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }
}

impl RectTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                let origin = snap.world;
                self.draft = Some(Draft {
                    origin,
                    current: origin,
                });
                let mut response = ToolResponse::cursor(CursorHint::Crosshair);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Move { screen, modifiers } => {
                let Some(mut draft) = self.draft else {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                self.draft = Some(draft);
                let rect = draft.rect(modifiers);
                let mut response = ToolResponse::cursor(CursorHint::Crosshair)
                    .with_overlay(ToolOverlay::PreviewRect { world_rect: rect });
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            } => {
                let Some(mut draft) = self.draft.take() else {
                    return ToolResponse::cursor(CursorHint::Default);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                let rect = draft.rect(modifiers);
                if rect.width() > 0.0 && rect.height() > 0.0 {
                    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                        rect.min_x,
                        rect.min_y,
                        rect.width(),
                        rect.height(),
                        ctx.new_shape_fill,
                    )));
                    // Sort the new shape above existing siblings (top of z-order)
                    // and bake the active fill into the CreateNode op so the
                    // color is part of the undoable creation, not a later patch.
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.rect", "create failed: {e}");
                    } else {
                        ctx.doc.selection.select_only(id);
                    }
                }
                let mut response = ToolResponse::exit().with_cursor(CursorHint::Default);
                // Carry the final snap guides on the release, useful for the
                // shell to render one last frame with the guide visible.
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, k: KeyEvent) -> ToolResponse {
        if matches!(k.key, LogicalKey::Escape) && self.draft.is_some() {
            self.draft = None;
            return ToolResponse::cursor(CursorHint::Default);
        }
        ToolResponse::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, Viewport};

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

    fn ctx_pieces() -> (Doc, Viewport, SnapEngine, DVec2) {
        (
            Doc::new(),
            Viewport::default(),
            // Disable all snap targets so tests measure raw geometry, not
            // grid-rounded values.
            SnapEngine {
                zoom: 1.0,
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        )
    }

    #[test]
    fn press_records_origin_no_doc_mutation() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let nodes_before = doc.scene.len();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), nodes_before);
    }

    #[test]
    fn release_commits_rect_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        // Press at screen (400, 300) is world (0, 0); release at (500, 360) is (100, 60).
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 60.0).abs() < 1e-6);
    }

    #[test]
    fn release_selects_new_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([420.0, 320.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        let id = doc.scene.roots()[0];
        assert_eq!(doc.selection.as_slice(), &[id]);
    }

    #[test]
    fn release_signals_wants_exit() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert!(r.wants_exit);
    }

    #[test]
    fn shift_constrains_to_square() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 320.0], ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, pe_release([500.0, 320.0], ModifierKeys::SHIFT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        // dx=100, dy=20 → square uses max(100, 20) = 100 on both axes.
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn alt_draws_from_center() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        // Press at world (0, 0), drag to world (50, 30) → without alt makes a
        // 50x30 rect at (0,0); with Alt the rect is centered on (0,0) and the
        // far corner is at (50, 30), so the rect spans (-50,-30) to (50,30).
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([450.0, 330.0], ModifierKeys::ALT));
        tool.handle_event(&mut ctx, pe_release([450.0, 330.0], ModifierKeys::ALT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 60.0).abs() < 1e-6);
        let c = bb.center();
        assert!(c.length() < 1e-6, "center should be near origin, got {c:?}");
    }

    #[test]
    fn escape_aborts_without_creating_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert_eq!(doc.scene.len(), 0);
        assert!(!tool.is_drafting());
    }

    #[test]
    fn zero_size_release_creates_no_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        // Release at same screen point → zero-size rect, should not commit.
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([100.0, 100.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn move_without_press_does_not_emit_preview() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        let r = tool.handle_event(&mut ctx, pe_move([100.0, 100.0], ModifierKeys::empty()));
        assert!(
            !r.overlays
                .iter()
                .any(|o| matches!(o, ToolOverlay::PreviewRect { .. }))
        );
    }

    #[test]
    fn preview_rect_overlay_grows_with_move() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        let preview = r
            .overlays
            .iter()
            .find_map(|o| match o {
                ToolOverlay::PreviewRect { world_rect } => Some(*world_rect),
                _ => None,
            })
            .expect("expected preview rect");
        assert!((preview.width() - 100.0).abs() < 1e-6);
        assert!((preview.height() - 60.0).abs() < 1e-6);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = RectTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }
}

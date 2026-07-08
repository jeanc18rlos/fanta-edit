//! Ellipse creation tool.
//!
//! Mirrors [`crate::rect::RectTool`] in shape: press records an origin, move
//! grows a preview, release commits a [`Operation::CreateNode`] carrying an
//! ellipse-shaped [`VectorNode`]. The geometric model is "the dragged
//! rectangle's bounding box is the ellipse's bounding box" — same as Figma.
//!
//! ## Modifiers (same as the rect tool by convention)
//!
//! - **Shift** constrains to a circle (square bbox).
//! - **Alt** draws from the press point as the ellipse's center.
//! - **Escape** mid-drag aborts.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, Fill, NodeData, Operation, PathData, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;

/// In-flight draft for the ellipse tool. Identical structure to the rect
/// tool's draft — kept private here rather than shared because keeping each
/// tool's state self-contained is more readable than sharing one struct.
#[derive(Debug, Clone, Copy)]
struct Draft {
    origin: DVec2,
    current: DVec2,
}

impl Draft {
    fn rect(&self, modifiers: ModifierKeys) -> Bounds {
        let mut dx = self.current.x - self.origin.x;
        let mut dy = self.current.y - self.origin.y;
        if modifiers.contains(ModifierKeys::SHIFT) {
            let m = dx.abs().max(dy.abs());
            dx = m.copysign(if dx == 0.0 { 1.0 } else { dx });
            dy = m.copysign(if dy == 0.0 { 1.0 } else { dy });
        }
        if modifiers.contains(ModifierKeys::ALT) {
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

/// State machine for the ellipse tool.
#[derive(Debug, Default)]
pub struct EllipseTool {
    draft: Option<Draft>,
}

impl EllipseTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for EllipseTool {
    fn name(&self) -> &'static str {
        "ellipse"
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

impl EllipseTool {
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
                    .with_overlay(ToolOverlay::PreviewEllipse { world_rect: rect });
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
                    let cx = rect.center().x;
                    let cy = rect.center().y;
                    let rx = rect.width() * 0.5;
                    let ry = rect.height() * 0.5;
                    let path = PathData::ellipse(cx, cy, rx, ry);
                    let mut fills: SmallVec<[Fill; 1]> = SmallVec::new();
                    fills.push(Fill::solid(ctx.new_shape_fill));
                    let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
                        path,
                        fills,
                        strokes: SmallVec::new(),
                        corner_radius: None,
                        corner_radii: None,
                        corner_smoothing: 0.0,
                        local_size: None,
                    }));
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.ellipse", "create failed: {e}");
                    } else {
                        ctx.doc.selection.select_only(id);
                    }
                }
                let mut response = ToolResponse::exit().with_cursor(CursorHint::Default);
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
            SnapEngine {
                zoom: 1.0,
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        )
    }

    #[test]
    fn press_does_not_mutate_doc() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_commits_ellipse_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        let node = doc.scene.get(id).unwrap();
        match &node.data {
            NodeData::Vector(v) => {
                // Ellipse path has 6 segments: M + 4 Cubic + Close.
                assert_eq!(v.path.segments.len(), 6);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn release_signals_wants_exit() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert!(r.wants_exit);
    }

    #[test]
    fn shift_constrains_to_circle() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 320.0], ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, pe_release([500.0, 320.0], ModifierKeys::SHIFT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        // Ellipse bounds should be ~100x100 (a circle).
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn alt_draws_centered_on_origin() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([450.0, 330.0], ModifierKeys::ALT));
        tool.handle_event(&mut ctx, pe_release([450.0, 330.0], ModifierKeys::ALT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        let c = bb.center();
        assert!(
            c.length() < 1e-6,
            "ellipse center should be at world origin, got {c:?}"
        );
    }

    #[test]
    fn escape_aborts_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
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
    fn zero_size_release_does_nothing() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn preview_overlay_present_during_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        let has_preview = r
            .overlays
            .iter()
            .any(|o| matches!(o, ToolOverlay::PreviewEllipse { .. }));
        assert!(has_preview);
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn new_node_becomes_sole_selection() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = EllipseTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([420.0, 320.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert_eq!(doc.selection.len(), 1);
        assert!(doc.selection.contains(doc.scene.roots()[0]));
    }
}

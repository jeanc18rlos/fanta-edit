//! Line creation tool.
//!
//! A line is two clicks (press → release). The committed node is a
//! [`VectorNode`] with an open path (no `Close`) and a single solid stroke.
//!
//! ## Modifiers
//!
//! - **Shift** snaps the angle to the nearest 45° step — Figma / Sketch / SVG
//!   editors all do this.
//! - **Alt** has no canonical meaning for a single line in Figma; we leave it
//!   reserved (no behavior) so future variants can pick it up without breaking
//!   existing reflexes.
//! - **Escape** mid-drag aborts.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{CanvasNode, Fill, NodeData, Operation, PathData, Stroke, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;
use std::f64::consts::TAU;

/// Snap a (dx, dy) vector to the nearest multiple of 45° (8 directions).
/// Preserves the magnitude — only the angle is quantized.
fn snap_to_45(dx: f64, dy: f64) -> (f64, f64) {
    let mag = (dx * dx + dy * dy).sqrt();
    if mag < f64::EPSILON {
        return (dx, dy);
    }
    let angle = dy.atan2(dx);
    let step = TAU / 8.0;
    let quantized = (angle / step).round() * step;
    (quantized.cos() * mag, quantized.sin() * mag)
}

/// In-flight draft state. World-space.
#[derive(Debug, Clone, Copy)]
struct Draft {
    start: DVec2,
    end: DVec2,
}

impl Draft {
    fn endpoints(&self, modifiers: ModifierKeys) -> (DVec2, DVec2) {
        if modifiers.contains(ModifierKeys::SHIFT) {
            let dx = self.end.x - self.start.x;
            let dy = self.end.y - self.start.y;
            let (qx, qy) = snap_to_45(dx, dy);
            (self.start, self.start + DVec2::new(qx, qy))
        } else {
            (self.start, self.end)
        }
    }
}

/// State machine for the line tool.
#[derive(Debug, Default)]
pub struct LineTool {
    draft: Option<Draft>,
}

impl LineTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        "line"
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

impl LineTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                let start = snap.world;
                self.draft = Some(Draft { start, end: start });
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
                draft.end = snap.world;
                self.draft = Some(draft);
                let (a, b) = draft.endpoints(modifiers);
                let mut response = ToolResponse::cursor(CursorHint::Crosshair).with_overlay(
                    ToolOverlay::PreviewLine {
                        world_start: [a.x, a.y],
                        world_end: [b.x, b.y],
                    },
                );
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
                draft.end = snap.world;
                let (a, b) = draft.endpoints(modifiers);
                if (b - a).length() > 0.0 {
                    let mut path = PathData::new();
                    path.move_to(a.x, a.y).line_to(b.x, b.y);
                    let mut strokes: SmallVec<[Stroke; 1]> = SmallVec::new();
                    // A line is a stroke, so the active "shape fill" color drives
                    // its stroke paint — cycling the palette recolors new lines.
                    strokes.push(Stroke {
                        paint: Fill::solid(ctx.new_shape_fill),
                        width: 2.0,
                        cap: Default::default(),
                        join: Default::default(),
                        miter_limit: 4.0,
                        dash: Vec::new(),
                        align: Default::default(),
                        per_side: None,
                    });
                    let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
                        path,
                        fills: SmallVec::new(),
                        strokes,
                        corner_radius: None,
                        corner_radii: None,
                        corner_smoothing: 0.0,
                    }));
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.line", "create failed: {e}");
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
    fn snap_to_45_quantizes_arbitrary_angle() {
        // Vector at ~30° should snap to the nearest multiple of 45° (either
        // 45° or 0°).
        let (x, _) = snap_to_45(10.0, 5.0);
        // The resulting x component must align with one of cos(45°), cos(0°),
        // etc. We just check the magnitude is preserved.
        let m_before = (10.0_f64 * 10.0 + 5.0 * 5.0).sqrt();
        let m_after = (x * x + snap_to_45(10.0, 5.0).1.powi(2)).sqrt();
        assert!((m_before - m_after).abs() < 1e-9);
    }

    #[test]
    fn snap_to_45_no_op_at_zero_length() {
        let (x, y) = snap_to_45(0.0, 0.0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
    }

    #[test]
    fn press_does_not_mutate_doc() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_commits_line_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // Open path: M + L. Two segments, no Close.
                assert_eq!(v.path.segments.len(), 2);
                assert_eq!(v.fills.len(), 0);
                assert_eq!(v.strokes.len(), 1);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn release_signals_wants_exit() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert!(r.wants_exit);
    }

    #[test]
    fn shift_snaps_to_axis_aligned_direction() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        // Drag almost horizontal: dx=100, dy=5 → with Shift the line should
        // snap to 0° (purely horizontal).
        tool.handle_event(&mut ctx, pe_move([500.0, 305.0], ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, pe_release([500.0, 305.0], ModifierKeys::SHIFT));
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // Second segment's end y should equal start y (0° snap).
                let line_end = v.path.segments.last().unwrap();
                match *line_end {
                    fanta_doc::PathSegment::Line { to } => {
                        // Start is at world (0, 0). End should be on the x-axis.
                        assert!(to[1].abs() < 1e-6, "y not snapped to 0: {}", to[1]);
                    }
                    _ => panic!("expected line segment"),
                }
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn escape_aborts_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
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
    fn zero_length_release_does_nothing() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn preview_overlay_present_during_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        let has_preview = r
            .overlays
            .iter()
            .any(|o| matches!(o, ToolOverlay::PreviewLine { .. }));
        assert!(has_preview);
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = LineTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }
}

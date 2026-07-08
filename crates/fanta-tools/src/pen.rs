//! Pen tool — bezier path authoring by clicking anchor points.
//!
//! ## Behavior
//!
//! - **Click** places an anchor. **Click-drag** sets that anchor's tangent
//!   handles (symmetric: dragging out one side mirrors the other), turning the
//!   adjoining segments into cubic curves — the standard pen gesture.
//! - **Click near the first anchor** closes the path and commits.
//! - **Enter** commits the current (open) path. **Escape** aborts the in-flight
//!   path (a second Escape, with no path, exits to Select).
//! - **Delete/Backspace** removes the last placed anchor.
//!
//! A committed path is a [`VectorNode`]: an open path becomes a **stroke** in
//! the active color (like the line tool); a closed path becomes a **fill**. The
//! pen is *sticky* — after a commit it stays active so you can draw the next
//! path, matching Figma.
//!
//! Live preview uses straight [`ToolOverlay::PreviewLine`] segments between
//! anchors plus a rubber-band to the cursor (there is no cubic-preview overlay
//! yet); the committed geometry is the exact cubic.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{CanvasNode, Fill, NodeData, Operation, PathData, Stroke, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;

/// Stroke width baked into a committed open path (matches the line tool).
const PEN_STROKE_WIDTH: f64 = 2.0;
/// World-distance below which a click on the first anchor counts as "close".
const CLOSE_DIST: f64 = 8.0;

/// One placed anchor: a position plus optional in/out tangent handles, stored as
/// world-space DELTAS from the anchor (so they transform with the node later).
#[derive(Debug, Clone, Copy)]
struct Anchor {
    pos: DVec2,
    tan_in: Option<DVec2>,
    tan_out: Option<DVec2>,
}

impl Anchor {
    fn ctrl_out(&self) -> Option<DVec2> {
        self.tan_out.map(|t| self.pos + t)
    }
    fn ctrl_in(&self) -> Option<DVec2> {
        self.tan_in.map(|t| self.pos + t)
    }
}

/// State machine for the pen tool.
#[derive(Debug, Default)]
pub struct PenTool {
    anchors: Vec<Anchor>,
    /// True between Press and Release on the most-recent anchor (adjusting its
    /// tangent by dragging).
    dragging: bool,
    /// Last-known cursor in world space (for the rubber-band preview).
    cursor: DVec2,
}

impl PenTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a path is in progress.
    pub fn is_drafting(&self) -> bool {
        !self.anchors.is_empty()
    }

    /// Number of placed anchors (for tests).
    pub fn anchor_count(&self) -> usize {
        self.anchors.len()
    }
}

impl Tool for PenTool {
    fn name(&self) -> &'static str {
        "pen"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(ctx, k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.anchors.clear();
        self.dragging = false;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.anchors.clear();
        self.dragging = false;
    }
}

impl PenTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                let pos = snap.world;
                self.cursor = pos;

                // Click on the first anchor (with at least a triangle of points)
                // closes the path and commits.
                if self.anchors.len() >= 2 {
                    if let Some(first) = self.anchors.first() {
                        if (first.pos - pos).length() <= CLOSE_DIST {
                            self.commit(ctx, true);
                            return ToolResponse::cursor(CursorHint::Crosshair);
                        }
                    }
                }

                // Starting a fresh path clears any prior selection so a stray
                // Backspace can't delete an unrelated node mid-draw.
                if self.anchors.is_empty() {
                    ctx.doc.selection.clear();
                }
                self.anchors.push(Anchor {
                    pos,
                    tan_in: None,
                    tan_out: None,
                });
                self.dragging = true;
                self.preview()
            }
            PointerEvent::Move { screen, .. } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                self.cursor = world;
                if self.dragging {
                    // Drag sets a symmetric tangent on the just-placed anchor.
                    if let Some(last) = self.anchors.last_mut() {
                        let t = world - last.pos;
                        last.tan_out = Some(t);
                        last.tan_in = Some(-t);
                    }
                }
                self.preview()
            }
            PointerEvent::Release {
                button: Button::Primary,
                ..
            } => {
                self.dragging = false;
                self.preview()
            }
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, ctx: &mut ToolContext, k: KeyEvent) -> ToolResponse {
        match k.key {
            LogicalKey::Enter => {
                self.commit(ctx, false);
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            LogicalKey::Escape => {
                if self.anchors.is_empty() {
                    // Nothing to abort — leave the tool.
                    ToolResponse::exit().with_cursor(CursorHint::Default)
                } else {
                    self.anchors.clear();
                    self.dragging = false;
                    ToolResponse::cursor(CursorHint::Crosshair)
                }
            }
            LogicalKey::Delete => {
                self.anchors.pop();
                self.dragging = false;
                self.preview()
            }
            _ => ToolResponse::empty(),
        }
    }

    /// Build straight-segment preview overlays between anchors plus a rubber-band
    /// to the cursor while drafting.
    fn preview(&self) -> ToolResponse {
        let mut response = ToolResponse::cursor(CursorHint::Crosshair);
        for pair in self.anchors.windows(2) {
            response.overlays.push(ToolOverlay::PreviewLine {
                world_start: [pair[0].pos.x, pair[0].pos.y],
                world_end: [pair[1].pos.x, pair[1].pos.y],
            });
        }
        if let Some(last) = self.anchors.last() {
            response.overlays.push(ToolOverlay::PreviewLine {
                world_start: [last.pos.x, last.pos.y],
                world_end: [self.cursor.x, self.cursor.y],
            });
        }
        response
    }

    /// Build the cubic path from the placed anchors and commit it as a
    /// `VectorNode`. `close` appends a closing segment + `Close`. Resets the
    /// draft (sticky tool). No-op for fewer than two anchors.
    fn commit(&mut self, ctx: &mut ToolContext, close: bool) {
        let anchors = std::mem::take(&mut self.anchors);
        self.dragging = false;
        if anchors.len() < 2 {
            return;
        }
        let mut path = PathData::new();
        path.move_to(anchors[0].pos.x, anchors[0].pos.y);
        for win in anchors.windows(2) {
            seg(&mut path, &win[0], &win[1]);
        }
        if close {
            // Closing segment from the last anchor back to the first.
            if let (Some(last), Some(first)) = (anchors.last(), anchors.first()) {
                seg(&mut path, last, first);
            }
            path.close();
        }

        let (fills, strokes) = if close {
            let mut fills: SmallVec<[Fill; 1]> = SmallVec::new();
            fills.push(Fill::solid(ctx.new_shape_fill));
            (fills, SmallVec::new())
        } else {
            let mut strokes: SmallVec<[Stroke; 1]> = SmallVec::new();
            strokes.push(Stroke {
                paint: Fill::solid(ctx.new_shape_fill),
                width: PEN_STROKE_WIDTH,
                cap: Default::default(),
                join: Default::default(),
                miter_limit: 4.0,
                dash: Vec::new(),
                align: Default::default(),
                per_side: None,
            });
            (SmallVec::new(), strokes)
        };

        let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills,
            strokes,
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
        }));
        ctx.place_new_node_on_active_page(&mut node);
        let id = node.id;
        if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
            tracing::warn!(target: "fanta-tools.pen", "create failed: {e}");
        } else {
            ctx.doc.selection.select_only(id);
        }
    }
}

/// Append the segment connecting `a`→`b`: a cubic when either side has a tangent
/// handle, else a straight line.
fn seg(path: &mut PathData, a: &Anchor, b: &Anchor) {
    match (a.ctrl_out(), b.ctrl_in()) {
        (None, None) => {
            path.line_to(b.pos.x, b.pos.y);
        }
        (c1, c2) => {
            let ctrl1 = c1.unwrap_or(a.pos);
            let ctrl2 = c2.unwrap_or(b.pos);
            path.cubic_to(ctrl1.x, ctrl1.y, ctrl2.x, ctrl2.y, b.pos.x, b.pos.y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModifierKeys;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, PathSegment, Viewport};

    fn press(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 1,
        })
    }
    fn mv(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Move {
            screen,
            modifiers: ModifierKeys::empty(),
        })
    }
    fn release(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Release {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
        })
    }
    fn key(k: LogicalKey) -> ToolEvent {
        ToolEvent::Key(KeyEvent::press(k))
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

    /// Click without dragging the tangent: press then release at the same point.
    fn click(tool: &mut PenTool, ctx: &mut ToolContext, screen: [f64; 2]) {
        tool.handle_event(ctx, press(screen));
        tool.handle_event(ctx, release(screen));
    }

    #[test]
    fn first_click_starts_draft_no_commit() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [100.0, 100.0]);
        assert!(tool.is_drafting());
        assert_eq!(tool.anchor_count(), 1);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn enter_commits_open_polyline() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 400.0]);
        tool.handle_event(&mut ctx, key(LogicalKey::Enter));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // Move + 2 lines, no Close (open).
                assert_eq!(v.path.segments.len(), 3);
                assert!(!matches!(v.path.segments.last(), Some(PathSegment::Close)));
                assert_eq!(v.strokes.len(), 1);
                assert_eq!(v.fills.len(), 0);
            }
            _ => panic!("expected vector"),
        }
        assert!(!tool.is_drafting(), "draft resets after commit");
    }

    #[test]
    fn click_first_anchor_closes_and_fills() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 400.0]);
        // Click back on the first anchor → close.
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                assert!(matches!(v.path.segments.last(), Some(PathSegment::Close)));
                assert_eq!(v.fills.len(), 1, "closed path is filled");
                assert_eq!(v.strokes.len(), 0);
            }
            _ => panic!("expected vector"),
        }
    }

    #[test]
    fn drag_sets_cubic_tangent() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        // Anchor 1: click-drag to set a tangent.
        tool.handle_event(&mut ctx, press([400.0, 300.0]));
        tool.handle_event(&mut ctx, mv([440.0, 300.0]));
        tool.handle_event(&mut ctx, release([440.0, 300.0]));
        // Anchor 2: plain click.
        click(&mut tool, &mut ctx, [500.0, 400.0]);
        tool.handle_event(&mut ctx, key(LogicalKey::Enter));
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                assert!(
                    v.path
                        .segments
                        .iter()
                        .any(|s| matches!(s, PathSegment::Cubic { .. })),
                    "a dragged tangent makes the segment a cubic"
                );
            }
            _ => panic!("expected vector"),
        }
    }

    #[test]
    fn backspace_removes_last_anchor() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 300.0]);
        assert_eq!(tool.anchor_count(), 2);
        tool.handle_event(&mut ctx, key(LogicalKey::Delete));
        assert_eq!(tool.anchor_count(), 1);
    }

    #[test]
    fn escape_aborts_then_exits() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        let r = tool.handle_event(&mut ctx, key(LogicalKey::Escape));
        assert!(!r.wants_exit, "first escape just aborts the draft");
        assert!(!tool.is_drafting());
        let r2 = tool.handle_event(&mut ctx, key(LogicalKey::Escape));
        assert!(r2.wants_exit, "escape with no draft exits to select");
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn single_anchor_enter_creates_nothing() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        tool.handle_event(&mut ctx, key(LogicalKey::Enter));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn rubber_band_preview_present_while_drafting() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        let r = tool.handle_event(&mut ctx, mv([460.0, 360.0]));
        assert!(
            r.overlays
                .iter()
                .any(|o| matches!(o, ToolOverlay::PreviewLine { .. }))
        );
    }
}

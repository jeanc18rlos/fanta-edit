//! Pen tool — bezier path authoring by clicking anchor points.
//!
//! ## Behavior
//!
//! - **Click** places an anchor. **Click-drag** sets that anchor's tangent
//!   handles (symmetric: dragging out one side mirrors the other), turning the
//!   adjoining segments into cubic curves — the standard pen gesture.
//! - **Click within 10 screen pixels of the first anchor** closes the path and
//!   commits. The draft's anchors are alignment snap targets.
//! - **Enter** commits the current (open) path. **Escape** aborts the in-flight
//!   path (a second Escape, with no path, exits to Select).
//!   Switching tools commits a draft of two or more anchors as an open path.
//! - **Delete/Backspace** removes the last placed anchor.
//!
//! A committed path is a [`VectorNode`]: an open path becomes a **stroke** in
//! the active color (like the line tool); a closed path becomes a **fill**. The
//! pen is *sticky* — after a commit it stays active so you can draw the next
//! path, matching Figma.
//!
//! The live preview is a [`ToolOverlay::PreviewPath`] of the draft's curves plus
//! the segment a press would add, ending at the snapped cursor. Inside the
//! close distance it previews the filled shape, rings the first anchor and
//! shows the pointer cursor.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{
    CanvasNode, Color, Fill, NodeData, Operation, PathData, Stroke, Transform2D, VectorNode,
};
use glam::DVec2;
use smallvec::SmallVec;

/// Stroke width baked into a committed open path (matches the line tool).
const PEN_STROKE_WIDTH: f64 = 2.0;
/// Screen distance within which a press on the first anchor closes the path.
const CLOSE_PX: f64 = 10.0;
/// A closed shape thinner than this on average, in screen pixels, draws as an
/// invisible sliver. Mean width (twice the area over the perimeter) rather
/// than area, because a sliver along a long segment has a large area: a
/// 1-pixel release jitter on a 100-pixel segment encloses about 60 square
/// pixels but is only 0.6 pixels wide.
const MIN_CLOSED_WIDTH_PX: f64 = 1.0;

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
    /// Where a press would place the next anchor, in world space (for the
    /// rubber-band preview).
    cursor: DVec2,
    /// The pointer is within the close distance of the first anchor.
    closing: bool,
    /// Viewport zoom at the last pointer event, to measure a close in pixels.
    zoom: f64,
    /// The color of the last press, kept for a commit on a tool switch whose
    /// context carries the next tool's color.
    color: Option<Color>,
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

    fn finish_draft(&mut self, ctx: &mut ToolContext) {
        let next_color = ctx.new_shape_fill;
        if let Some(color) = self.color {
            ctx.new_shape_fill = color;
        }
        self.commit(ctx, false);
        ctx.new_shape_fill = next_color;
    }

    fn has_draft(&self) -> bool {
        self.is_drafting()
    }

    fn undo_draft(&mut self) -> Option<ToolResponse> {
        self.anchors.pop()?;
        self.dragging = false;
        Some(self.preview())
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.anchors.clear();
        self.dragging = false;
    }
}

impl PenTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        self.zoom = ctx.viewport.zoom;
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let screen = DVec2::from(screen);
                // Raw pointer, so snapping cannot pull a press off the first anchor.
                if self.within_close_distance(ctx, screen) {
                    let close = self.close_has_area();
                    self.commit(ctx, close);
                    return ToolResponse::cursor(CursorHint::Crosshair);
                }
                let world = ctx.screen_to_world(screen);
                let pos = self.snap(ctx, world);
                self.cursor = pos;
                self.closing = false;
                self.color = Some(ctx.new_shape_fill);

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
                let screen = DVec2::from(screen);
                let world = ctx.screen_to_world(screen);
                if self.dragging {
                    self.cursor = world;
                    // Drag sets a symmetric tangent on the just-placed anchor.
                    if let Some(last) = self.anchors.last_mut() {
                        let t = world - last.pos;
                        last.tan_out = Some(t);
                        last.tan_in = Some(-t);
                    }
                } else {
                    self.closing = self.within_close_distance(ctx, screen);
                    self.cursor = self.snap(ctx, world);
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
            LogicalKey::Delete => self.undo_draft().unwrap_or_else(|| self.preview()),
            _ => ToolResponse::empty(),
        }
    }

    fn within_close_distance(&self, ctx: &ToolContext, screen: DVec2) -> bool {
        self.anchors.len() >= 2
            && self
                .anchors
                .first()
                .is_some_and(|first| ctx.world_to_screen(first.pos).distance(screen) <= CLOSE_PX)
    }

    /// Whether closing the draft encloses a shape wide enough to see.
    fn close_has_area(&self) -> bool {
        let outline = self.closed_outline();
        let mut twice_area = 0.0;
        let mut perimeter = 0.0;
        for (index, point) in outline.iter().enumerate() {
            let next = outline[(index + 1) % outline.len()];
            twice_area += point[0] * next[1] - next[0] * point[1];
            perimeter += DVec2::from(*point).distance(DVec2::from(next));
        }
        perimeter > 0.0 && twice_area.abs() / perimeter * self.zoom >= MIN_CLOSED_WIDTH_PX
    }

    /// The draft flattened, ending with the closing segment back to the first anchor.
    fn closed_outline(&self) -> Vec<[f64; 2]> {
        let (Some(first), Some(last)) = (self.anchors.first(), self.anchors.last()) else {
            return Vec::new();
        };
        let mut points = vec![[first.pos.x, first.pos.y]];
        for pair in self.anchors.windows(2) {
            flatten_segment(&mut points, &pair[0], &pair[1]);
        }
        flatten_segment(&mut points, last, first);
        points
    }

    /// Snap against the scene and the draft's own anchors.
    fn snap(&self, ctx: &ToolContext, world: DVec2) -> DVec2 {
        let points: SmallVec<[DVec2; 16]> = self.anchors.iter().map(|anchor| anchor.pos).collect();
        ctx.snap_point_with_points(world, &[], &points).world
    }

    /// The draft's curves, the segment a press would add, its anchors, the
    /// last anchor's handles and, inside the close distance, the close ring.
    fn preview(&self) -> ToolResponse {
        let closing = self.closing && !self.dragging && self.close_has_area();
        let mut response = ToolResponse::cursor(if closing {
            CursorHint::Pointer
        } else {
            CursorHint::Crosshair
        });
        let (Some(first), Some(last)) = (self.anchors.first(), self.anchors.last()) else {
            return response;
        };
        let mut world_points = vec![[first.pos.x, first.pos.y]];
        for pair in self.anchors.windows(2) {
            flatten_segment(&mut world_points, &pair[0], &pair[1]);
        }
        if closing {
            flatten_segment(&mut world_points, last, first);
        } else if !self.dragging {
            let next = Anchor {
                pos: self.cursor,
                tan_in: None,
                tan_out: None,
            };
            flatten_segment(&mut world_points, last, &next);
        }
        response.overlays.push(ToolOverlay::PreviewPath {
            world_points,
            closed: closing,
            filled: closing,
        });
        for anchor in &self.anchors {
            response.overlays.push(ToolOverlay::PathAnchor {
                world: [anchor.pos.x, anchor.pos.y],
                selected: false,
            });
        }
        for ctrl in [last.ctrl_in(), last.ctrl_out()].into_iter().flatten() {
            response.overlays.push(ToolOverlay::PathHandle {
                world_anchor: [last.pos.x, last.pos.y],
                world_ctrl: [ctrl.x, ctrl.y],
            });
        }
        if closing {
            response.overlays.push(ToolOverlay::PenCloseTarget {
                world: [first.pos.x, first.pos.y],
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
        let p0 = anchors[0].pos;
        // Build path relative to p0 as local origin; set transform for place.
        let mut path = PathData::new();
        path.move_to(0.0, 0.0);
        for win in anchors.windows(2) {
            let mut a = win[0];
            let mut b = win[1];
            a.pos = a.pos - p0;
            if let Some(t) = a.tan_out {
                a.tan_out = Some(t);
            } // tans are deltas, stay relative to their pos
            if let Some(t) = a.tan_in {
                a.tan_in = Some(t);
            }
            b.pos = b.pos - p0;
            if let Some(t) = b.tan_out {
                b.tan_out = Some(t);
            }
            if let Some(t) = b.tan_in {
                b.tan_in = Some(t);
            }
            seg(&mut path, &a, &b);
        }
        if close {
            if let (Some(last), Some(first)) = (anchors.last(), anchors.first()) {
                let mut a = *last;
                let mut b = *first;
                a.pos = a.pos - p0;
                b.pos = b.pos - p0;
                seg(&mut path, &a, &b);
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
            parametric: None,
        }));
        node.transform = Transform2D::translation(p0.x, p0.y);
        ctx.place_new_node_on_active_page(&mut node);
        let id = node.id;
        if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
            tracing::warn!(target: "fanta-tools.pen", "create failed: {e}");
        } else {
            ctx.doc.selection.select_only(id);
        }
    }
}

/// Append the points after `a` on the segment `a`→`b` that [`seg`] would build.
fn flatten_segment(points: &mut Vec<[f64; 2]>, a: &Anchor, b: &Anchor) {
    const STEPS: usize = 16;
    match (a.ctrl_out(), b.ctrl_in()) {
        (None, None) => points.push([b.pos.x, b.pos.y]),
        (c1, c2) => {
            let ctrl1 = c1.unwrap_or(a.pos);
            let ctrl2 = c2.unwrap_or(b.pos);
            for step in 1..=STEPS {
                let t = step as f64 / STEPS as f64;
                let u = 1.0 - t;
                let point = a.pos * (u * u * u)
                    + ctrl1 * (3.0 * u * u * t)
                    + ctrl2 * (3.0 * u * t * t)
                    + b.pos * (t * t * t);
                points.push([point.x, point.y]);
            }
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

    fn pieces_at_zoom(
        zoom: f64,
        targets: fanta_canvas::SnapTargets,
    ) -> (Doc, Viewport, SnapEngine, DVec2) {
        let (doc, _, mut snap, size) = ctx_pieces();
        snap.zoom = zoom;
        snap.targets = targets;
        (
            doc,
            Viewport {
                center: [0.0, 0.0],
                zoom,
            },
            snap,
            size,
        )
    }

    fn draft_triangle(tool: &mut PenTool, ctx: &mut ToolContext) {
        click(tool, ctx, [400.0, 300.0]);
        click(tool, ctx, [600.0, 300.0]);
        click(tool, ctx, [600.0, 500.0]);
    }

    /// A1: a press 9 screen pixels from the first anchor closes the path at
    /// every zoom, and a press 12 pixels away places another anchor.
    #[test]
    fn a1_close_distance_is_ten_screen_pixels_at_every_zoom() {
        for zoom in [0.25, 1.0, 8.0] {
            let (mut doc, mut viewport, snap, size) =
                pieces_at_zoom(zoom, fanta_canvas::SnapTargets::empty());
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap.clone(), size);
            let mut tool = PenTool::new();
            draft_triangle(&mut tool, &mut ctx);
            click(&mut tool, &mut ctx, [412.0, 300.0]);
            assert_eq!(
                tool.anchor_count(),
                4,
                "12 px does not close at zoom {zoom}"
            );
            assert_eq!(ctx.doc.scene.len(), 0);

            let (mut doc, mut viewport, snap, size) =
                pieces_at_zoom(zoom, fanta_canvas::SnapTargets::empty());
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
            let mut tool = PenTool::new();
            draft_triangle(&mut tool, &mut ctx);
            click(&mut tool, &mut ctx, [409.0, 300.0]);
            assert!(!tool.is_drafting(), "9 px closes at zoom {zoom}");
            let id = ctx.doc.scene.roots()[0];
            match &ctx.doc.scene.get(id).map(|node| &node.data) {
                Some(NodeData::Vector(v)) => {
                    assert!(matches!(v.path.segments.last(), Some(PathSegment::Close)));
                }
                _ => panic!("expected a closed vector at zoom {zoom}"),
            }
        }
    }

    /// A2: the first anchor wins over a grid line that is closer to the press.
    #[test]
    fn a2_first_anchor_wins_over_closer_grid_snap() {
        let (mut doc, mut viewport, mut snap, size) =
            pieces_at_zoom(1.0, fanta_canvas::SnapTargets::GRID);
        snap.grid = fanta_canvas::SnapGrid {
            spacing: 100.0,
            origin: [9.0, 9.0],
        };
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        draft_triangle(&mut tool, &mut ctx);
        // World (6, 0): the grid line x = 9 is 3 px away, the first anchor 6 px.
        click(&mut tool, &mut ctx, [406.0, 300.0]);
        assert!(!tool.is_drafting(), "the press closes the path");
        assert_eq!(ctx.doc.scene.len(), 1);
    }

    /// R2: the draft's other anchors align the next anchor without closing.
    #[test]
    fn draft_anchors_are_alignment_candidates() {
        let (mut doc, mut viewport, snap, size) =
            pieces_at_zoom(1.0, fanta_canvas::SnapTargets::empty());
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        draft_triangle(&mut tool, &mut ctx);
        // 4 px right of the first anchor's x, far below it: x aligns to it.
        click(&mut tool, &mut ctx, [404.0, 450.0]);
        tool.handle_event(&mut ctx, key(LogicalKey::Enter));
        let id = ctx.doc.scene.roots()[0];
        let node = ctx.doc.scene.get(id).expect("committed node");
        match &node.data {
            NodeData::Vector(v) => match v.path.segments.last() {
                Some(PathSegment::Line { to }) => assert!(to[0].abs() < 1e-9, "x = {}", to[0]),
                other => panic!("expected a line, got {other:?}"),
            },
            _ => panic!("expected vector"),
        }
    }

    /// A3: inside the close distance a hover shows the ring, the close cursor
    /// and a filled preview; away from it the preview is the curve a press
    /// would make, ending at the snapped point.
    #[test]
    fn a3_hover_previews_close_and_snapped_curve() {
        let (mut doc, mut viewport, snap, size) =
            pieces_at_zoom(1.0, fanta_canvas::SnapTargets::GRID);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [600.0, 300.0]);
        // The third anchor is dragged, so the next segment is a curve.
        tool.handle_event(&mut ctx, press([600.0, 500.0]));
        tool.handle_event(&mut ctx, mv([640.0, 500.0]));
        tool.handle_event(&mut ctx, release([640.0, 500.0]));

        let response = tool.handle_event(&mut ctx, mv([405.0, 303.0]));
        assert_eq!(response.cursor, Some(CursorHint::Pointer));
        assert!(
            response
                .overlays
                .contains(&ToolOverlay::PenCloseTarget { world: [0.0, 0.0] })
        );
        assert!(response.overlays.iter().any(|overlay| matches!(
            overlay,
            ToolOverlay::PreviewPath { closed: true, filled: true, world_points }
                if world_points.last() == Some(&[0.0, 0.0])
        )));

        // World (101, 151): the 8-unit grid snaps it to (104, 152).
        let response = tool.handle_event(&mut ctx, mv([501.0, 451.0]));
        assert_eq!(response.cursor, Some(CursorHint::Crosshair));
        assert!(
            !response
                .overlays
                .iter()
                .any(|overlay| matches!(overlay, ToolOverlay::PenCloseTarget { .. }))
        );
        let Some(ToolOverlay::PreviewPath {
            world_points,
            closed: false,
            filled: false,
        }) = response.overlays.first()
        else {
            panic!("expected an open path preview, got {:?}", response.overlays);
        };
        assert_eq!(world_points.last(), Some(&[104.0, 152.0]));
        let curve = world_points
            .iter()
            .skip_while(|point| **point != [200.0, 200.0])
            .collect::<Vec<_>>();
        assert!(curve.len() > 2, "the new segment is flattened as a curve");
        assert_eq!(
            response
                .overlays
                .iter()
                .filter(|overlay| matches!(overlay, ToolOverlay::PathAnchor { .. }))
                .count(),
            3
        );
        assert_eq!(
            response
                .overlays
                .iter()
                .filter(|overlay| matches!(overlay, ToolOverlay::PathHandle { .. }))
                .count(),
            2
        );
    }

    /// A8: Undo during a draft removes one anchor and adds no history entry;
    /// with no anchors left it falls through to document history.
    #[test]
    fn a8_undo_draft_pops_one_anchor_without_history() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        draft_triangle(&mut tool, &mut ctx);
        let depth = ctx.doc.history.undo_depth();
        let response = tool.undo_draft().expect("an anchor to undo");
        assert_eq!(tool.anchor_count(), 2);
        assert_eq!(
            response
                .overlays
                .iter()
                .filter(|overlay| matches!(overlay, ToolOverlay::PathAnchor { .. }))
                .count(),
            2
        );
        assert!(tool.undo_draft().is_some());
        assert!(tool.undo_draft().is_some());
        assert!(!tool.is_drafting());
        assert!(tool.undo_draft().is_none(), "Undo reaches document history");
        assert_eq!(ctx.doc.history.undo_depth(), depth);
    }

    /// draw-tools-2: closing two straight anchors would make an invisible
    /// zero-area fill, so the press commits the open line instead.
    #[test]
    fn closing_two_straight_anchors_commits_the_open_line() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 300.0]);
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        assert!(!tool.is_drafting());
        assert_eq!(ctx.doc.scene.len(), 1);
        let id = ctx.doc.scene.roots()[0];
        match ctx.doc.scene.get(id).map(|node| &node.data) {
            Some(NodeData::Vector(v)) => {
                assert_eq!(v.path.segments.len(), 2, "move and one line");
                assert_eq!(v.strokes.len(), 1);
                assert!(v.fills.is_empty());
            }
            _ => panic!("expected vector"),
        }
    }

    fn committed_vector(ctx: &ToolContext) -> VectorNode {
        let id = ctx.doc.scene.roots()[0];
        match ctx.doc.scene.get(id).map(|node| &node.data) {
            Some(NodeData::Vector(vector)) => vector.clone(),
            other => panic!("expected a vector, got {other:?}"),
        }
    }

    /// Three anchors on one line enclose nothing, so closing commits the open path.
    #[test]
    fn closing_collinear_anchors_commits_the_open_path() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        click(&mut tool, &mut ctx, [500.0, 300.0]);
        click(&mut tool, &mut ctx, [600.0, 300.0]);
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        assert!(!tool.is_drafting());
        let vector = committed_vector(&ctx);
        assert!(!matches!(
            vector.path.segments.last(),
            Some(PathSegment::Close)
        ));
        assert_eq!(vector.strokes.len(), 1);
        assert!(vector.fills.is_empty());
    }

    /// A click whose release jitters by a pixel leaves a tiny handle; closing
    /// two such anchors would make a sliver, so it commits the open line.
    #[test]
    fn closing_two_anchors_with_click_jitter_commits_the_open_line() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        tool.handle_event(&mut ctx, press([500.0, 300.0]));
        tool.handle_event(&mut ctx, mv([500.0, 301.0]));
        tool.handle_event(&mut ctx, release([500.0, 301.0]));
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        assert!(!tool.is_drafting());
        let vector = committed_vector(&ctx);
        assert!(!matches!(
            vector.path.segments.last(),
            Some(PathSegment::Close)
        ));
        assert!(vector.fills.is_empty());
    }

    /// Two anchors with a real handle enclose a lens, so they still close.
    #[test]
    fn closing_two_anchors_with_a_handle_fills_the_shape() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PenTool::new();
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        tool.handle_event(&mut ctx, press([500.0, 300.0]));
        tool.handle_event(&mut ctx, mv([500.0, 340.0]));
        tool.handle_event(&mut ctx, release([500.0, 340.0]));
        click(&mut tool, &mut ctx, [400.0, 300.0]);
        let vector = committed_vector(&ctx);
        assert!(matches!(
            vector.path.segments.last(),
            Some(PathSegment::Close)
        ));
        assert_eq!(vector.fills.len(), 1);
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
                .any(|o| matches!(o, ToolOverlay::PreviewPath { .. }))
        );
    }
}

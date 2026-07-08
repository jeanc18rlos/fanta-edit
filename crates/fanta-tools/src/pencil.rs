//! Pencil tool — freehand stroke capture.
//!
//! ## Behavior
//!
//! - **Press** begins a stroke at the cursor.
//! - **Move** (while pressed) samples points, throttled to roughly every
//!   [`SAMPLE_PX`] of screen travel so a slow drag doesn't bloat the path.
//! - **Release** simplifies the captured points (Ramer–Douglas–Peucker) and
//!   commits an open [`VectorNode`] path stroked in the active color.
//! - **Escape** mid-stroke aborts.
//!
//! The pencil is *sticky*: each press starts a new stroke, each release commits
//! one, and the tool stays active. Closing/curve-fitting is intentionally out of
//! scope (use the pen tool or a later edit-path tool).

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{CanvasNode, Fill, NodeData, Operation, PathData, Stroke, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;

/// Stroke width baked into a committed freehand path.
const PENCIL_STROKE_WIDTH: f64 = 2.0;
/// Minimum screen-space travel between captured samples.
const SAMPLE_PX: f64 = 3.0;
/// RDP simplification tolerance, in world units.
const SIMPLIFY_EPSILON: f64 = 1.2;

/// State machine for the pencil tool.
#[derive(Debug, Default)]
pub struct PencilTool {
    /// Captured world-space points for the in-flight stroke.
    points: Vec<DVec2>,
    /// True while the pointer is down capturing a stroke.
    active: bool,
    /// Screen position of the last accepted sample (sampling throttle).
    last_sample_screen: Option<DVec2>,
}

impl PencilTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a stroke is being captured.
    pub fn is_drafting(&self) -> bool {
        self.active
    }

    /// Captured point count (for tests).
    pub fn point_count(&self) -> usize {
        self.points.len()
    }
}

impl Tool for PencilTool {
    fn name(&self) -> &'static str {
        "pencil"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.reset();
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.reset();
    }
}

impl PencilTool {
    fn reset(&mut self) {
        self.points.clear();
        self.active = false;
        self.last_sample_screen = None;
    }

    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                self.points.clear();
                self.points.push(world);
                self.active = true;
                self.last_sample_screen = Some(DVec2::from(screen));
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            PointerEvent::Move { screen, .. } => {
                if !self.active {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                }
                let scr = DVec2::from(screen);
                let far_enough = self
                    .last_sample_screen
                    .map(|last| (scr - last).length() >= SAMPLE_PX)
                    .unwrap_or(true);
                if far_enough {
                    self.points.push(ctx.screen_to_world(scr));
                    self.last_sample_screen = Some(scr);
                }
                self.preview()
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                ..
            } => {
                if !self.active {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                }
                self.points.push(ctx.screen_to_world(DVec2::from(screen)));
                self.commit(ctx);
                self.reset();
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, k: KeyEvent) -> ToolResponse {
        if matches!(k.key, LogicalKey::Escape) {
            if self.active {
                self.reset();
                return ToolResponse::cursor(CursorHint::Crosshair);
            }
            return ToolResponse::exit().with_cursor(CursorHint::Default);
        }
        ToolResponse::empty()
    }

    /// Preview the captured stroke as straight segments between samples.
    fn preview(&self) -> ToolResponse {
        let mut response = ToolResponse::cursor(CursorHint::Crosshair);
        for pair in self.points.windows(2) {
            response.overlays.push(ToolOverlay::PreviewLine {
                world_start: [pair[0].x, pair[0].y],
                world_end: [pair[1].x, pair[1].y],
            });
        }
        response
    }

    /// Simplify the captured points and commit an open stroked path. No-op for
    /// fewer than two distinct points.
    fn commit(&mut self, ctx: &mut ToolContext) {
        let simplified = rdp(&self.points, SIMPLIFY_EPSILON);
        if simplified.len() < 2 {
            return;
        }
        // Reject a degenerate (zero-extent) stroke — e.g. a bare click whose
        // press and release land on the same point yields two identical samples.
        let p0 = simplified[0];
        if simplified.iter().all(|p| (*p - p0).length() < 0.5) {
            return;
        }
        let mut path = PathData::new();
        path.move_to(simplified[0].x, simplified[0].y);
        for pt in &simplified[1..] {
            path.line_to(pt.x, pt.y);
        }
        let mut strokes: SmallVec<[Stroke; 1]> = SmallVec::new();
        strokes.push(Stroke {
            paint: Fill::solid(ctx.new_shape_fill),
            width: PENCIL_STROKE_WIDTH,
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
            local_size: None,
        }));
        ctx.place_new_node_on_active_page(&mut node);
        let id = node.id;
        if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
            tracing::warn!(target: "fanta-tools.pencil", "create failed: {e}");
        } else {
            ctx.doc.selection.select_only(id);
        }
    }
}

/// Ramer–Douglas–Peucker polyline simplification. Keeps endpoints; drops points
/// that lie within `epsilon` of the segment spanning the current span.
fn rdp(points: &[DVec2], epsilon: f64) -> Vec<DVec2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    rdp_recurse(points, 0, points.len() - 1, epsilon, &mut keep);
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

fn rdp_recurse(points: &[DVec2], lo: usize, hi: usize, epsilon: f64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let a = points[lo];
    let b = points[hi];
    let mut max_d = 0.0;
    let mut idx = lo;
    for (i, p) in points.iter().enumerate().take(hi).skip(lo + 1) {
        let d = perpendicular_distance(*p, a, b);
        if d > max_d {
            max_d = d;
            idx = i;
        }
    }
    if max_d > epsilon {
        keep[idx] = true;
        rdp_recurse(points, lo, idx, epsilon, keep);
        rdp_recurse(points, idx, hi, epsilon, keep);
    }
}

/// Perpendicular distance from `p` to the segment `a`→`b` (degenerates to the
/// point distance when `a == b`).
fn perpendicular_distance(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len = ab.length();
    if len < f64::EPSILON {
        return (p - a).length();
    }
    // 2D cross product magnitude / base length = triangle height.
    ((p - a).x * ab.y - (p - a).y * ab.x).abs() / len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModifierKeys;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, Viewport};

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
    fn press_begins_capture() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PencilTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn move_throttles_samples() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PencilTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        // A sub-threshold move is ignored.
        tool.handle_event(&mut ctx, mv([101.0, 100.0]));
        assert_eq!(tool.point_count(), 1);
        // A move past the threshold is captured.
        tool.handle_event(&mut ctx, mv([120.0, 100.0]));
        assert_eq!(tool.point_count(), 2);
    }

    #[test]
    fn release_commits_simplified_open_path() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PencilTool::new();
        // A roughly-straight drag: many samples → simplify to ~2 points.
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        for x in (110..=300).step_by(10) {
            tool.handle_event(&mut ctx, mv([x as f64, 100.0]));
        }
        tool.handle_event(&mut ctx, release([300.0, 100.0]));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                assert_eq!(v.strokes.len(), 1);
                assert_eq!(v.fills.len(), 0);
                // A straight line simplifies to a Move + a single Line.
                assert!(
                    v.path.segments.len() <= 3,
                    "collinear points should collapse, got {}",
                    v.path.segments.len()
                );
            }
            _ => panic!("expected vector"),
        }
        assert!(!tool.is_drafting(), "sticky tool resets after a stroke");
    }

    #[test]
    fn single_point_release_creates_nothing() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PencilTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        tool.handle_event(&mut ctx, release([100.0, 100.0]));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn escape_aborts_stroke() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PencilTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        tool.handle_event(&mut ctx, mv([200.0, 200.0]));
        let r = tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert!(!r.wants_exit);
        assert!(!tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn rdp_drops_collinear_midpoints() {
        let pts = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(1.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(3.0, 0.0),
        ];
        let out = rdp(&pts, 1.0);
        assert_eq!(out.len(), 2, "a straight run collapses to its endpoints");
    }

    #[test]
    fn rdp_keeps_a_real_corner() {
        let pts = vec![
            DVec2::new(0.0, 0.0),
            DVec2::new(5.0, 0.0),
            DVec2::new(10.0, 10.0),
        ];
        let out = rdp(&pts, 1.0);
        assert_eq!(out.len(), 3, "a sharp corner is preserved");
    }
}

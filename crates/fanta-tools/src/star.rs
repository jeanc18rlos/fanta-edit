//! Star creation tool.
//!
//! Mirrors [`crate::polygon::PolygonTool`] in shape: press records an origin,
//! move grows a preview, release commits a [`Operation::CreateNode`] carrying a
//! star-shaped [`VectorNode`]. A star with `points` points has `2 * points`
//! vertices, alternating between an OUTER radius (the point tips, inscribed in
//! the drag box's inscribed ellipse) and an INNER radius (`outer * inner_ratio`,
//! the notches between tips). This is the same model Figma's star tool uses;
//! the point count and inner ratio are node properties the inspector edits
//! later (out of scope for this crate-level wave).
//!
//! ## Defaults & modifiers
//!
//! - Default point count is [`DEFAULT_POINTS`] (a 5-point star), inner ratio
//!   [`DEFAULT_INNER_RATIO`] — both match Figma's star defaults.
//! - **Shift** constrains the bounding box to a square (a radially symmetric
//!   star rather than a squashed one).
//! - **Alt** draws from the press point as the bounding box's center.
//! - **Escape** mid-drag aborts.
//!
//! The committed path uses the [`even-odd`](fanta_doc::FillRule::EvenOdd) fill
//! rule so the classic self-intersecting "pentagram" rendering is correct even
//! at small inner ratios.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::polygon::polyline_overlays;
use crate::tool::{CursorHint, SnapGuide, Tool, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, Fill, FillRule, NodeData, Operation, PathData, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;
use std::f64::consts::{FRAC_PI_2, TAU};

/// Default number of points (tips) for a freshly-drawn star, matching Figma's
/// star tool default.
pub const DEFAULT_POINTS: u32 = 5;

/// Lower clamp on the point count. Fewer than 3 points is not a star.
pub const MIN_POINTS: u32 = 3;

/// Default inner-radius ratio (notch radius / tip radius). Figma's star tool
/// defaults to ~0.382 (the golden-ratio value that makes a clean pentagram);
/// we use that.
pub const DEFAULT_INNER_RATIO: f64 = 0.382;

/// Compute the world-space vertices of a `points`-point star inscribed in
/// `rect`. Returns `2 * points` points: outer tips (on the box's inscribed
/// ellipse) interleaved with inner notch points (at `inner_ratio` of the radii).
/// The first vertex is an outer tip pointing straight up (screen y-down apex,
/// matching Figma), and the ring proceeds clockwise. First ≠ last; the contour
/// is closed by the consumer.
///
/// `points` is clamped to at least [`MIN_POINTS`] and `inner_ratio` to
/// `0.0..=1.0`, so a degenerate request can never produce a self-folding ring.
pub fn star_vertices(rect: Bounds, points: u32, inner_ratio: f64) -> Vec<[f64; 2]> {
    let n = points.max(MIN_POINTS);
    let ratio = inner_ratio.clamp(0.0, 1.0);
    let c = rect.center();
    let rx = rect.width() * 0.5;
    let ry = rect.height() * 0.5;
    let start = -FRAC_PI_2;
    // 2N vertices: even indices are outer tips, odd indices are inner notches.
    let step = TAU / (2.0 * n as f64);
    (0..(2 * n))
        .map(|i| {
            let theta = start + step * (i as f64);
            let scale = if i % 2 == 0 { 1.0 } else { ratio };
            [
                c.x + rx * scale * theta.cos(),
                c.y + ry * scale * theta.sin(),
            ]
        })
        .collect()
}

/// Build a closed even-odd [`PathData`] from a vertex ring.
fn path_from_vertices(verts: &[[f64; 2]]) -> PathData {
    let mut path = PathData::new();
    if let Some((first, rest)) = verts.split_first() {
        path.move_to(first[0], first[1]);
        for v in rest {
            path.line_to(v[0], v[1]);
        }
        path.close();
    }
    path.fill_rule = FillRule::EvenOdd;
    path
}

/// In-flight drag state. Same structure as the polygon tool's draft.
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

/// State machine for the star tool.
#[derive(Debug)]
pub struct StarTool {
    draft: Option<Draft>,
    /// Number of points (tips) new stars are created with.
    points: u32,
    /// Inner-radius ratio (notch radius / tip radius) for new stars.
    inner_ratio: f64,
}

impl Default for StarTool {
    fn default() -> Self {
        Self {
            draft: None,
            points: DEFAULT_POINTS,
            inner_ratio: DEFAULT_INNER_RATIO,
        }
    }
}

impl StarTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a star tool with a specific point count (clamped to
    /// [`MIN_POINTS`]) and inner ratio (clamped to `0.0..=1.0`).
    pub fn with_shape(points: u32, inner_ratio: f64) -> Self {
        Self {
            draft: None,
            points: points.max(MIN_POINTS),
            inner_ratio: inner_ratio.clamp(0.0, 1.0),
        }
    }

    /// The point count new stars will use.
    pub fn points(&self) -> u32 {
        self.points
    }

    /// The inner-radius ratio new stars will use.
    pub fn inner_ratio(&self) -> f64 {
        self.inner_ratio
    }

    /// Whether the tool is mid-drag.
    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for StarTool {
    fn name(&self) -> &'static str {
        "star"
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

impl StarTool {
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
                let verts = star_vertices(rect, self.points, self.inner_ratio);
                let mut response = ToolResponse::cursor(CursorHint::Crosshair);
                for o in polyline_overlays(&verts) {
                    response.overlays.push(o);
                }
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
                    let verts = star_vertices(rect, self.points, self.inner_ratio);
                    let path = path_from_vertices(&verts);
                    let mut fills: SmallVec<[Fill; 1]> = SmallVec::new();
                    fills.push(Fill::solid(ctx.new_shape_fill));
                    let mut node = CanvasNode::new(NodeData::Vector(VectorNode {
                        path,
                        fills,
                        strokes: SmallVec::new(),
                        corner_radius: None,
                        corner_radii: None,
                        corner_smoothing: 0.0,
                    }));
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.star", "create failed: {e}");
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
    use crate::tool::ToolOverlay;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, PathSegment, Viewport};

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
    fn vertices_count_is_two_n() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        for p in [3, 5, 6, 8] {
            assert_eq!(
                star_vertices(rect, p, DEFAULT_INNER_RATIO).len(),
                2 * p as usize
            );
        }
    }

    #[test]
    fn degenerate_point_count_is_clamped() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            star_vertices(rect, 0, DEFAULT_INNER_RATIO).len(),
            2 * MIN_POINTS as usize
        );
        assert_eq!(
            star_vertices(rect, 2, DEFAULT_INNER_RATIO).len(),
            2 * MIN_POINTS as usize
        );
    }

    #[test]
    fn radii_alternate_outer_then_inner() {
        // On a square box (circle), outer radius == rx, inner == rx * ratio.
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        let c = rect.center();
        let rx = rect.width() * 0.5;
        let ratio = 0.4;
        let verts = star_vertices(rect, 5, ratio);
        for (i, v) in verts.iter().enumerate() {
            let r = (DVec2::new(v[0], v[1]) - c).length();
            let expected = if i % 2 == 0 { rx } else { rx * ratio };
            assert!(
                (r - expected).abs() < 1e-9,
                "vertex {i} radius {r} != expected {expected}"
            );
        }
    }

    #[test]
    fn first_vertex_is_outer_top_tip() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 80.0);
        let c = rect.center();
        let v0 = star_vertices(rect, 5, DEFAULT_INNER_RATIO)[0];
        assert!((v0[0] - c.x).abs() < 1e-9, "tip not centered in x");
        assert!((v0[1] - rect.min_y).abs() < 1e-9, "tip not at top edge");
    }

    #[test]
    fn inner_ratio_is_clamped() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        let c = rect.center();
        let rx = rect.width() * 0.5;
        // ratio > 1 clamps to 1 → inner radius equals outer radius.
        let verts = star_vertices(rect, 5, 5.0);
        let inner = (DVec2::new(verts[1][0], verts[1][1]) - c).length();
        assert!((inner - rx).abs() < 1e-9);
        // ratio < 0 clamps to 0 → inner vertices at the center.
        let verts = star_vertices(rect, 5, -1.0);
        let inner = (DVec2::new(verts[1][0], verts[1][1]) - c).length();
        assert!(inner.abs() < 1e-9);
    }

    #[test]
    fn press_does_not_mutate_doc() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_commits_star_node_with_expected_segments() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new(); // 5 points → 10 vertices
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // 10 vertices: M + 9 L + Close = 11 segments.
                assert_eq!(v.path.segments.len(), 11);
                assert!(matches!(v.path.segments[0], PathSegment::Move { .. }));
                assert!(matches!(
                    v.path.segments.last().unwrap(),
                    PathSegment::Close
                ));
                assert_eq!(v.path.fill_rule, FillRule::EvenOdd);
                assert_eq!(v.fills.len(), 1);
                assert_eq!(v.strokes.len(), 0);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn six_point_star_commits_thirteen_segments() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::with_shape(6, 0.5);
        assert_eq!(tool.points(), 6);
        assert!((tool.inner_ratio() - 0.5).abs() < 1e-9);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // 12 vertices: M + 11 L + Close = 13 segments.
                assert_eq!(v.path.segments.len(), 13);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn committed_star_bbox_matches_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        // 4-point star: outer tips touch the 4 box edges, so bbox == drag box.
        let mut tool = StarTool::with_shape(4, 0.4);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 60.0).abs() < 1e-6);
    }

    #[test]
    fn alt_draws_centered_on_origin() {
        // A 4-point star is symmetric about both axes, so its tight bbox center
        // coincides with the drag-box center. With Alt the press point is the
        // drag-box center (world origin), so the star's bbox is centered there.
        // (An odd-point star is not vertically symmetric — top tip vs bottom
        // notches — so its tight bbox would be offset; that's geometry, not a
        // centering bug, hence the even-point choice here.)
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::with_shape(4, 0.4);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([450.0, 330.0], ModifierKeys::ALT));
        tool.handle_event(&mut ctx, pe_release([450.0, 330.0], ModifierKeys::ALT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        assert!(
            bb.center().length() < 1e-6,
            "star center should be at world origin, got {:?}",
            bb.center()
        );
    }

    #[test]
    fn escape_aborts_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
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
        let mut tool = StarTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn preview_overlay_present_during_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::with_shape(5, DEFAULT_INNER_RATIO);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        // A 5-point star outline previews as 10 edge segments (2N vertices).
        let edges = r
            .overlays
            .iter()
            .filter(|o| matches!(o, ToolOverlay::PreviewLine { .. }))
            .count();
        assert_eq!(edges, 10);
    }

    #[test]
    fn move_without_press_emits_no_preview() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
        let r = tool.handle_event(&mut ctx, pe_move([100.0, 100.0], ModifierKeys::empty()));
        assert!(
            !r.overlays
                .iter()
                .any(|o| matches!(o, ToolOverlay::PreviewLine { .. }))
        );
    }

    #[test]
    fn release_signals_wants_exit_and_selects() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_release([420.0, 320.0], ModifierKeys::empty()));
        assert!(r.wants_exit);
        let id = doc.scene.roots()[0];
        assert_eq!(doc.selection.as_slice(), &[id]);
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = StarTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }
}

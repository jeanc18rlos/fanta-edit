//! Regular polygon creation tool.
//!
//! Mirrors [`crate::ellipse::EllipseTool`] in shape: press records an origin,
//! move grows a preview, release commits a [`Operation::CreateNode`] carrying a
//! polygon-shaped [`VectorNode`]. The geometric model is "the dragged
//! rectangle's bounding box is the polygon's bounding box" — the N-gon is
//! inscribed in the box's inscribed ellipse, so a non-square drag yields a
//! squashed polygon (same convention Figma uses for its polygon tool).
//!
//! ## Defaults & modifiers
//!
//! - Default side count is [`DEFAULT_SIDES`] (a triangle). Figma's polygon tool
//!   defaults to 3 sides; the count is a node property the inspector edits later
//!   (out of scope for this crate-level wave).
//! - **Shift** constrains the bounding box to a square (so the polygon is
//!   regular, not squashed).
//! - **Alt** draws from the press point as the bounding box's center.
//! - **Escape** mid-drag aborts.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, Fill, NodeData, Operation, PathData, VectorNode};
use glam::DVec2;
use smallvec::SmallVec;
use std::f64::consts::{FRAC_PI_2, TAU};

/// Default number of sides for a freshly-drawn polygon (a triangle), matching
/// Figma's polygon tool default.
pub const DEFAULT_SIDES: u32 = 3;

/// Lower clamp on the side count. Fewer than 3 sides is not a polygon.
pub const MIN_SIDES: u32 = 3;

/// Compute the world-space vertices of a regular `sides`-gon inscribed in
/// `rect`. The polygon is inscribed in the box's inscribed ellipse: each vertex
/// sits on the ellipse `(cx + rx·cosθ, cy + ry·sinθ)`. The first vertex points
/// straight up (the "−y" apex, screen y-down), matching Figma's orientation,
/// and the ring proceeds clockwise. Returns `sides` points (first ≠ last; the
/// contour is closed by the consumer).
///
/// `sides` is clamped to at least [`MIN_SIDES`] so a degenerate request can
/// never produce a line or a point.
pub fn polygon_vertices(rect: Bounds, sides: u32) -> Vec<[f64; 2]> {
    let n = sides.max(MIN_SIDES);
    let c = rect.center();
    let rx = rect.width() * 0.5;
    let ry = rect.height() * 0.5;
    // Start at the top apex (angle −90°) and step clockwise around the ellipse.
    let start = -FRAC_PI_2;
    (0..n)
        .map(|i| {
            let theta = start + TAU * (i as f64) / (n as f64);
            [c.x + rx * theta.cos(), c.y + ry * theta.sin()]
        })
        .collect()
}

/// Turn a closed vertex ring into a fan of [`ToolOverlay::PreviewLine`] edges
/// the shell can stroke — one segment per edge, plus the closing edge back to
/// the first vertex. Shared by the polygon and star tools: both resolve their
/// outline to a point ring, so previewing is "draw the ring's edges." Using the
/// existing `PreviewLine` overlay (rather than a bespoke polygon/star variant)
/// keeps the host renderer unchanged while still showing the full outline live.
/// A ring of fewer than two points yields no edges.
pub fn polyline_overlays(verts: &[[f64; 2]]) -> SmallVec<[ToolOverlay; 4]> {
    let mut out: SmallVec<[ToolOverlay; 4]> = SmallVec::new();
    let n = verts.len();
    if n < 2 {
        return out;
    }
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        out.push(ToolOverlay::PreviewLine {
            world_start: a,
            world_end: b,
        });
    }
    out
}

/// Build a closed [`PathData`] from a vertex ring (move to the first point,
/// line through the rest, close).
fn path_from_vertices(verts: &[[f64; 2]]) -> PathData {
    let mut path = PathData::new();
    if let Some((first, rest)) = verts.split_first() {
        path.move_to(first[0], first[1]);
        for v in rest {
            path.line_to(v[0], v[1]);
        }
        path.close();
    }
    path
}

/// In-flight drag state. Identical structure to the ellipse tool's draft —
/// kept private here so each tool's state stays self-contained.
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

/// State machine for the polygon tool.
#[derive(Debug)]
pub struct PolygonTool {
    draft: Option<Draft>,
    /// Number of sides new polygons are created with.
    sides: u32,
}

impl Default for PolygonTool {
    fn default() -> Self {
        Self {
            draft: None,
            sides: DEFAULT_SIDES,
        }
    }
}

impl PolygonTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a polygon tool with a specific side count (clamped to
    /// [`MIN_SIDES`]). The shell may use this to honor an inspector setting.
    pub fn with_sides(sides: u32) -> Self {
        Self {
            draft: None,
            sides: sides.max(MIN_SIDES),
        }
    }

    /// The side count new polygons will use.
    pub fn sides(&self) -> u32 {
        self.sides
    }

    /// Whether the tool is mid-drag.
    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for PolygonTool {
    fn name(&self) -> &'static str {
        "polygon"
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

impl PolygonTool {
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
                let verts = polygon_vertices(rect, self.sides);
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
                    let verts = polygon_vertices(rect, self.sides);
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
                        local_size: None,
                    }));
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.polygon", "create failed: {e}");
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
    fn vertices_count_matches_sides() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        for n in [3, 4, 5, 6, 8] {
            assert_eq!(polygon_vertices(rect, n).len(), n as usize);
        }
    }

    #[test]
    fn degenerate_side_count_is_clamped_to_min() {
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        assert_eq!(polygon_vertices(rect, 0).len(), MIN_SIDES as usize);
        assert_eq!(polygon_vertices(rect, 1).len(), MIN_SIDES as usize);
        assert_eq!(polygon_vertices(rect, 2).len(), MIN_SIDES as usize);
    }

    #[test]
    fn vertices_lie_on_inscribed_ellipse() {
        // A 200x100 box: each vertex must satisfy ((x-cx)/rx)^2 + ((y-cy)/ry)^2 = 1.
        let rect = Bounds::from_xywh(10.0, 20.0, 200.0, 100.0);
        let c = rect.center();
        let rx = rect.width() * 0.5;
        let ry = rect.height() * 0.5;
        for v in polygon_vertices(rect, 5) {
            let nx = (v[0] - c.x) / rx;
            let ny = (v[1] - c.y) / ry;
            assert!(
                (nx * nx + ny * ny - 1.0).abs() < 1e-9,
                "vertex {v:?} off ellipse"
            );
        }
    }

    #[test]
    fn first_vertex_is_top_apex() {
        // First vertex points straight up (centered on x, at the top edge).
        let rect = Bounds::from_xywh(0.0, 0.0, 100.0, 80.0);
        let c = rect.center();
        let v0 = polygon_vertices(rect, 5)[0];
        assert!((v0[0] - c.x).abs() < 1e-9, "apex not centered in x");
        assert!((v0[1] - rect.min_y).abs() < 1e-9, "apex not at top edge");
    }

    #[test]
    fn vertices_centered_on_box_center() {
        // The centroid of a regular polygon's vertices is the box center.
        let rect = Bounds::from_xywh(-30.0, 40.0, 120.0, 120.0);
        let c = rect.center();
        let verts = polygon_vertices(rect, 6);
        let sum = verts
            .iter()
            .fold(DVec2::ZERO, |a, v| a + DVec2::new(v[0], v[1]));
        let centroid = sum / verts.len() as f64;
        assert!((centroid - c).length() < 1e-9);
    }

    #[test]
    fn press_does_not_mutate_doc() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn release_commits_polygon_node_with_expected_segments() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::new(); // 3 sides
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // 3-gon: M + 2 L + Close = 4 segments.
                assert_eq!(v.path.segments.len(), 4);
                assert!(matches!(v.path.segments[0], PathSegment::Move { .. }));
                assert!(matches!(
                    v.path.segments.last().unwrap(),
                    PathSegment::Close
                ));
                assert_eq!(v.fills.len(), 1);
                assert_eq!(v.strokes.len(), 0);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn hexagon_tool_commits_seven_segments() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::with_sides(6);
        assert_eq!(tool.sides(), 6);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        let id = doc.scene.roots()[0];
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                // 6-gon: M + 5 L + Close = 7 segments.
                assert_eq!(v.path.segments.len(), 7);
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn committed_polygon_bbox_matches_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::with_sides(4); // diamond touches all 4 edges
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0], ModifierKeys::empty()));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        // Drag spans world (0,0)..(100,60); a diamond's extrema hit the box edges.
        assert!((bb.width() - 100.0).abs() < 1e-6);
        assert!((bb.height() - 60.0).abs() < 1e-6);
    }

    #[test]
    fn shift_makes_regular_polygon() {
        // With Shift the drag box is forced square, so the inscribed ellipse is a
        // circle and every vertex is equidistant from the *drag-box center* — a
        // regular polygon. (The polygon's own tight bbox center differs from the
        // drag-box center for an odd-gon, so we measure radii from the known
        // drag-box center, which is the world origin: press at screen (400,300)
        // maps to world (0,0), and a square drag keeps the box centered when the
        // far corner is symmetric — here we instead derive the center from the
        // square box explicitly.)
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::with_sides(5);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([500.0, 320.0], ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, pe_release([500.0, 320.0], ModifierKeys::SHIFT));
        let id = doc.scene.roots()[0];
        // Square drag box: origin world (0,0), far corner squared to (100,100),
        // so the drag-box center is (50, 50).
        let center = DVec2::new(50.0, 50.0);
        match &doc.scene.get(id).unwrap().data {
            NodeData::Vector(v) => {
                let radii: Vec<f64> = v
                    .path
                    .segments
                    .iter()
                    .filter_map(|s| s.end_point())
                    .map(|p| (p - center).length())
                    .collect();
                let first = radii[0];
                for r in &radii {
                    assert!((r - first).abs() < 1e-6, "not equidistant: {r} vs {first}");
                }
            }
            _ => panic!("expected vector node"),
        }
    }

    #[test]
    fn alt_draws_centered_on_origin() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::with_sides(6);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_move([450.0, 330.0], ModifierKeys::ALT));
        tool.handle_event(&mut ctx, pe_release([450.0, 330.0], ModifierKeys::ALT));
        let id = doc.scene.roots()[0];
        let bb = doc.scene.world_bounds(id).unwrap();
        assert!(
            bb.center().length() < 1e-6,
            "polygon center should be at world origin, got {:?}",
            bb.center()
        );
    }

    #[test]
    fn escape_aborts_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::new();
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
        let mut tool = PolygonTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn polyline_overlays_closes_the_ring() {
        // 3 vertices → 3 edges (last edge wraps back to vertex 0).
        let verts = [[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]];
        let edges = polyline_overlays(&verts);
        assert_eq!(edges.len(), 3);
        // The final edge must close the ring (end == first vertex).
        match edges.last().unwrap() {
            ToolOverlay::PreviewLine { world_end, .. } => {
                assert_eq!(*world_end, verts[0]);
            }
            _ => panic!("expected preview line"),
        }
        // Degenerate rings produce no edges.
        assert!(polyline_overlays(&[]).is_empty());
        assert!(polyline_overlays(&[[1.0, 2.0]]).is_empty());
    }

    #[test]
    fn preview_overlay_present_during_drag() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::with_sides(5);
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        let r = tool.handle_event(&mut ctx, pe_move([500.0, 360.0], ModifierKeys::empty()));
        // A 5-gon outline previews as 5 edge segments.
        let edges = r
            .overlays
            .iter()
            .filter(|o| matches!(o, ToolOverlay::PreviewLine { .. }))
            .count();
        assert_eq!(edges, 5);
    }

    #[test]
    fn move_without_press_emits_no_preview() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::new();
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
        let mut tool = PolygonTool::new();
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
        let mut tool = PolygonTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0], ModifierKeys::empty()));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = PolygonTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0], ModifierKeys::empty()));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }
}

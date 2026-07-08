//! Snapping engine.
//!
//! When the user drags a node (or a vertex, or the marquee origin), each axis
//! gets a chance to lock onto a nearby snap candidate — the grid, another
//! node's edge, or another node's center. The result captures both the
//! adjusted position and *which* candidate caught — that secondary data is
//! what the UI uses to render "smart guides" (the dashed magenta lines you
//! see in Figma / Sketch / tldraw).
//!
//! ## Design choices
//!
//! - **Per-axis independence.** X and Y snap separately. A node can land on
//!   an X-edge of one neighbor and a Y-center of another simultaneously.
//! - **Closest-wins.** Within a single axis, the candidate with the smallest
//!   absolute screen-pixel delta wins. Thresholds are configured in screen
//!   pixels so behavior stays consistent across zoom levels.
//! - **Configurable targets.** A bit-flagged [`SnapTargets`] enables / disables
//!   grid, edges, centers, and pixel-grid. The wedge ships with all enabled
//!   by default.
//! - **Exclude list.** The dragged nodes don't snap to themselves.
//!
//! Inspired by Figma's smart-guide behavior, tldraw's snap engine, and
//! Sketch's pixel-grid quantization at high zoom.

use bitflags::bitflags;
use fanta_doc::{Bounds, NodeData, NodeFlags, NodeId, Scene};
use glam::DVec2;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

bitflags! {
    /// Which categories of snap target are active.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct SnapTargets: u32 {
        const GRID = 1 << 0;
        const NODE_EDGES = 1 << 1;
        const NODE_CENTERS = 1 << 2;
        /// Quantize to integer screen pixels at zoom ≥ 1.0. Mimics
        /// Sketch's "pixel preview" mode.
        const PIXEL_GRID = 1 << 3;
    }
}

impl SnapTargets {
    /// The default target set — everything except pixel-grid (which is opt-in
    /// because the constant quantization fights freeform drawing).
    pub const fn defaults() -> Self {
        Self::from_bits_truncate(
            Self::GRID.bits() | Self::NODE_EDGES.bits() | Self::NODE_CENTERS.bits(),
        )
    }
}

// =============================================================================
// Engine configuration
// =============================================================================

/// Tunable thresholds — all in **screen pixels** so behavior is zoom-stable.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapThresholds {
    /// Maximum screen-pixel distance to snap to a node edge.
    pub edge: f64,
    /// Maximum screen-pixel distance to snap to a node center.
    pub center: f64,
    /// Maximum screen-pixel distance to snap to a grid line.
    pub grid: f64,
}

impl Default for SnapThresholds {
    fn default() -> Self {
        Self {
            edge: 6.0,
            center: 6.0,
            grid: 4.0,
        }
    }
}

/// A grid definition. World-space spacing; world-space origin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapGrid {
    pub spacing: f64,
    pub origin: [f64; 2],
}

impl Default for SnapGrid {
    fn default() -> Self {
        Self {
            spacing: 8.0,
            origin: [0.0, 0.0],
        }
    }
}

/// Engine handle. Cheap to construct per-frame; no internal state.
#[derive(Debug, Clone)]
pub struct SnapEngine {
    pub targets: SnapTargets,
    pub thresholds: SnapThresholds,
    pub grid: SnapGrid,
    /// Current viewport zoom — needed to convert screen-pixel thresholds to
    /// world-space tolerances at snap time.
    pub zoom: f64,
}

impl Default for SnapEngine {
    fn default() -> Self {
        Self {
            targets: SnapTargets::defaults(),
            thresholds: SnapThresholds::default(),
            grid: SnapGrid::default(),
            zoom: 1.0,
        }
    }
}

// =============================================================================
// Snap result
// =============================================================================

/// Why an axis snapped — for "smart guide" overlay rendering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SnapKind {
    Grid,
    PixelGrid,
    NodeEdgeMin { source: NodeId },
    NodeEdgeMax { source: NodeId },
    NodeCenter { source: NodeId },
}

/// One axis's snap outcome.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisSnap {
    pub kind: SnapKind,
    /// World-space coordinate the axis was snapped to.
    pub at: f64,
    /// Signed delta applied to reach `at`, in world space.
    pub delta: f64,
}

/// Result of a snap query.
#[derive(Debug, Clone, Default)]
pub struct SnapResult {
    pub world: DVec2,
    pub x: Option<AxisSnap>,
    pub y: Option<AxisSnap>,
}

// =============================================================================
// Cached candidate snapshot
// =============================================================================

/// A precomputed snapshot of the per-axis snap candidates for one gesture.
///
/// ## Why this is gesture-stable
///
/// Collecting candidates means walking the entire scene and reading every
/// non-excluded node's world bounds — an O(total nodes) traversal. During a
/// drag the *only* nodes whose geometry changes are the ones being dragged,
/// and those are precisely the nodes in the `exclude` list, which the
/// collector skips. Every node that contributes a candidate (the stationary
/// neighbors) therefore keeps the exact same edges and centers for the whole
/// gesture. That makes the candidate set a loop-invariant: collect it once at
/// press, reuse it on every mouse-move frame, and the snap results are
/// bit-for-bit identical to recomputing it each frame — at O(neighbors) per
/// frame instead of O(total nodes).
///
/// Grid and pixel-grid targets are intentionally *not* cached here: they are
/// computed analytically per query (closest line to the value) rather than
/// from a candidate list, so they cost nothing to recompute and need no
/// snapshot.
#[derive(Debug, Clone, Default)]
pub struct SnapCandidates {
    x: SmallVec<[(f64, SnapKind); 16]>,
    y: SmallVec<[(f64, SnapKind); 16]>,
}

impl SnapCandidates {
    /// Number of X-axis candidates — exposed so tests/diagnostics can confirm
    /// the snapshot captured the expected neighbors.
    pub fn x_len(&self) -> usize {
        self.x.len()
    }

    /// Number of Y-axis candidates.
    pub fn y_len(&self) -> usize {
        self.y.len()
    }

    /// Whether the snapshot holds no node candidates (grid/pixel-grid still
    /// apply at snap time regardless).
    pub fn is_empty(&self) -> bool {
        self.x.is_empty() && self.y.is_empty()
    }
}

// =============================================================================
// Engine
// =============================================================================

impl SnapEngine {
    /// Walk the scene once and snapshot the per-axis snap candidates, skipping
    /// the `exclude` set (the dragged nodes — they must not snap to
    /// themselves). Call this **once** when a gesture begins and feed the
    /// result to [`SnapEngine::snap_bounds_with`] / [`SnapEngine::snap_point_with`]
    /// on each subsequent frame; see [`SnapCandidates`] for why the snapshot
    /// stays valid for the whole gesture.
    pub fn collect_candidates(&self, scene: &Scene, exclude: &[NodeId]) -> SnapCandidates {
        let axis = self.collect_axis_candidates(scene, exclude);
        SnapCandidates {
            x: axis.x,
            y: axis.y,
        }
    }

    /// Snap a single world-space point. Useful for free-floating tools (pen,
    /// rectangle origin) where the only thing being dragged is a point.
    ///
    /// One-shot convenience: collects candidates then delegates to
    /// [`SnapEngine::snap_point_with`]. Behavior is identical to the cached
    /// path — the only difference is that this walks the whole scene every
    /// call, so a drag loop should cache via [`SnapEngine::collect_candidates`]
    /// instead.
    pub fn snap_point(&self, world: DVec2, scene: &Scene, exclude: &[NodeId]) -> SnapResult {
        let candidates = self.collect_candidates(scene, exclude);
        self.snap_point_with(&candidates, world)
    }

    /// Snap a single world-space point against a precomputed candidate set.
    /// The hot-loop entry point during a gesture.
    pub fn snap_point_with(&self, candidates: &SnapCandidates, world: DVec2) -> SnapResult {
        let pixel_to_world = 1.0 / self.zoom.max(f64::EPSILON);

        let x = self.snap_axis_x(world.x, &candidates.x, pixel_to_world);
        let y = self.snap_axis_y(world.y, &candidates.y, pixel_to_world);

        let mut out = SnapResult { world, x, y };
        if let Some(s) = out.x {
            out.world.x = s.at;
        }
        if let Some(s) = out.y {
            out.world.y = s.at;
        }
        out
    }

    /// Snap a node's world bounds — the typical case during a drag. Each of
    /// the box's three meaningful X values (left, center, right) competes for
    /// snapping against each candidate; same for Y. The chosen pair minimizes
    /// the move distance.
    ///
    /// One-shot convenience: collects candidates then delegates to
    /// [`SnapEngine::snap_bounds_with`]. Identical behavior to the cached
    /// path; prefer the cached path inside a drag loop.
    pub fn snap_bounds(
        &self,
        world_bounds: Bounds,
        scene: &Scene,
        exclude: &[NodeId],
    ) -> SnapResult {
        let candidates = self.collect_candidates(scene, exclude);
        self.snap_bounds_with(&candidates, world_bounds)
    }

    /// Snap a node's world bounds against a precomputed candidate set. The
    /// hot-loop entry point during a move gesture — no scene walk, no
    /// allocation.
    pub fn snap_bounds_with(
        &self,
        candidates: &SnapCandidates,
        world_bounds: Bounds,
    ) -> SnapResult {
        let pixel_to_world = 1.0 / self.zoom.max(f64::EPSILON);

        let cx = world_bounds.center().x;
        let cy = world_bounds.center().y;

        // For each of (left, center, right), find the best snap and pick the
        // smallest delta among the three "anchors."
        let x_anchors = [world_bounds.min_x, cx, world_bounds.max_x];
        let y_anchors = [world_bounds.min_y, cy, world_bounds.max_y];

        let mut best_x: Option<(AxisSnap, f64)> = None; // (snap, |delta|)
        for &anchor in &x_anchors {
            if let Some(s) = self.snap_axis_x(anchor, &candidates.x, pixel_to_world) {
                let abs = s.delta.abs();
                if best_x.is_none_or(|(_, prev)| abs < prev) {
                    best_x = Some((s, abs));
                }
            }
        }
        let mut best_y: Option<(AxisSnap, f64)> = None;
        for &anchor in &y_anchors {
            if let Some(s) = self.snap_axis_y(anchor, &candidates.y, pixel_to_world) {
                let abs = s.delta.abs();
                if best_y.is_none_or(|(_, prev)| abs < prev) {
                    best_y = Some((s, abs));
                }
            }
        }

        SnapResult {
            world: DVec2::new(
                cx + best_x.map(|(s, _)| s.delta).unwrap_or(0.0),
                cy + best_y.map(|(s, _)| s.delta).unwrap_or(0.0),
            ),
            x: best_x.map(|(s, _)| s),
            y: best_y.map(|(s, _)| s),
        }
    }

    // ---- internals ----------------------------------------------------------

    fn snap_axis_x(
        &self,
        value: f64,
        candidates: &[(f64, SnapKind)],
        pixel_to_world: f64,
    ) -> Option<AxisSnap> {
        // Grid is special — closest grid line, not a precomputed candidate set
        // (otherwise the candidate list explodes).
        let mut best: Option<(f64, SnapKind, f64)> = None; // (target, kind, abs_world_delta)

        if self.targets.contains(SnapTargets::GRID) && self.grid.spacing > 0.0 {
            let snapped = grid_snap_1d(value, self.grid.origin[0], self.grid.spacing);
            let world_delta = (snapped - value).abs();
            if world_delta <= self.thresholds.grid * pixel_to_world {
                best = Some((snapped, SnapKind::Grid, world_delta));
            }
        }
        if self.targets.contains(SnapTargets::PIXEL_GRID) && self.zoom >= 1.0 {
            // Quantize to whole world units → whole pixels at zoom=1.
            let snapped = value.round();
            let world_delta = (snapped - value).abs();
            // Pixel-grid always wins inside its own zero-distance threshold.
            if world_delta < 1e-9 {
                return Some(AxisSnap {
                    kind: SnapKind::PixelGrid,
                    at: snapped,
                    delta: 0.0,
                });
            }
        }

        for (candidate_value, kind) in candidates {
            let world_delta = (*candidate_value - value).abs();
            let threshold_world = match kind {
                SnapKind::NodeEdgeMin { .. } | SnapKind::NodeEdgeMax { .. } => {
                    self.thresholds.edge * pixel_to_world
                }
                SnapKind::NodeCenter { .. } => self.thresholds.center * pixel_to_world,
                _ => continue,
            };
            if world_delta <= threshold_world {
                let entry = (*candidate_value, *kind, world_delta);
                if best.is_none_or(|(_, _, prev)| world_delta < prev) {
                    best = Some(entry);
                }
            }
        }

        best.map(|(at, kind, _)| AxisSnap {
            at,
            kind,
            delta: at - value,
        })
    }

    fn snap_axis_y(
        &self,
        value: f64,
        candidates: &[(f64, SnapKind)],
        pixel_to_world: f64,
    ) -> Option<AxisSnap> {
        let mut best: Option<(f64, SnapKind, f64)> = None;

        if self.targets.contains(SnapTargets::GRID) && self.grid.spacing > 0.0 {
            let snapped = grid_snap_1d(value, self.grid.origin[1], self.grid.spacing);
            let world_delta = (snapped - value).abs();
            if world_delta <= self.thresholds.grid * pixel_to_world {
                best = Some((snapped, SnapKind::Grid, world_delta));
            }
        }
        if self.targets.contains(SnapTargets::PIXEL_GRID) && self.zoom >= 1.0 {
            let snapped = value.round();
            let world_delta = (snapped - value).abs();
            if world_delta < 1e-9 {
                return Some(AxisSnap {
                    kind: SnapKind::PixelGrid,
                    at: snapped,
                    delta: 0.0,
                });
            }
        }

        for (candidate_value, kind) in candidates {
            let world_delta = (*candidate_value - value).abs();
            let threshold_world = match kind {
                SnapKind::NodeEdgeMin { .. } | SnapKind::NodeEdgeMax { .. } => {
                    self.thresholds.edge * pixel_to_world
                }
                SnapKind::NodeCenter { .. } => self.thresholds.center * pixel_to_world,
                _ => continue,
            };
            if world_delta <= threshold_world {
                let entry = (*candidate_value, *kind, world_delta);
                if best.is_none_or(|(_, _, prev)| world_delta < prev) {
                    best = Some(entry);
                }
            }
        }

        best.map(|(at, kind, _)| AxisSnap {
            at,
            kind,
            delta: at - value,
        })
    }

    fn collect_axis_candidates(&self, scene: &Scene, exclude: &[NodeId]) -> AxisCandidates {
        let mut out = AxisCandidates::default();
        let want_edges = self.targets.contains(SnapTargets::NODE_EDGES);
        let want_centers = self.targets.contains(SnapTargets::NODE_CENTERS);
        if !want_edges && !want_centers {
            return out;
        }
        for &root in scene.roots() {
            collect_node_candidates(scene, root, exclude, want_edges, want_centers, &mut out);
        }
        out
    }
}

fn collect_node_candidates(
    scene: &Scene,
    id: NodeId,
    exclude: &[NodeId],
    want_edges: bool,
    want_centers: bool,
    out: &mut AxisCandidates,
) {
    let Some(node) = scene.get(id) else { return };
    if node.flags.contains(NodeFlags::HIDDEN) {
        return;
    }
    if exclude.contains(&id) {
        // Skip the dragged node and its descendants — the engine isn't
        // supposed to snap a node to itself.
        return;
    }
    // Descend through groups; their bounds are derived from children and
    // would be duplicates.
    if matches!(node.data, NodeData::Group(_)) {
        for &c in scene.children_of(Some(id)) {
            collect_node_candidates(scene, c, exclude, want_edges, want_centers, out);
        }
        return;
    }
    let Some(bb) = scene.world_bounds(id) else {
        return;
    };
    if want_edges {
        out.x.push((bb.min_x, SnapKind::NodeEdgeMin { source: id }));
        out.x.push((bb.max_x, SnapKind::NodeEdgeMax { source: id }));
        out.y.push((bb.min_y, SnapKind::NodeEdgeMin { source: id }));
        out.y.push((bb.max_y, SnapKind::NodeEdgeMax { source: id }));
    }
    if want_centers {
        let c = bb.center();
        out.x.push((c.x, SnapKind::NodeCenter { source: id }));
        out.y.push((c.y, SnapKind::NodeCenter { source: id }));
    }
    for &c in scene.children_of(Some(id)) {
        collect_node_candidates(scene, c, exclude, want_edges, want_centers, out);
    }
}

#[derive(Debug, Default)]
struct AxisCandidates {
    x: SmallVec<[(f64, SnapKind); 16]>,
    y: SmallVec<[(f64, SnapKind); 16]>,
}

fn grid_snap_1d(value: f64, origin: f64, spacing: f64) -> f64 {
    debug_assert!(spacing > 0.0);
    let offset = value - origin;
    let snapped_offset = (offset / spacing).round() * spacing;
    origin + snapped_offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Color, Doc, NodeData, Operation, Transform2D, VectorNode};

    fn engine_with_zoom(zoom: f64) -> SnapEngine {
        SnapEngine {
            zoom,
            ..Default::default()
        }
    }

    // ---- grid math ----------------------------------------------------------

    #[test]
    fn grid_snap_rounds_to_nearest_line() {
        assert_eq!(grid_snap_1d(3.0, 0.0, 8.0), 0.0); // 3 < 4 → 0
        assert_eq!(grid_snap_1d(5.0, 0.0, 8.0), 8.0);
        assert_eq!(grid_snap_1d(-3.0, 0.0, 8.0), 0.0); // -3 > -4 → 0
        assert_eq!(grid_snap_1d(-5.0, 0.0, 8.0), -8.0);
    }

    #[test]
    fn grid_snap_respects_origin() {
        // Origin shifted by 4 — the lines fall at 4, 12, 20, ...
        assert_eq!(grid_snap_1d(7.0, 4.0, 8.0), 4.0);
        assert_eq!(grid_snap_1d(9.0, 4.0, 8.0), 12.0);
    }

    // ---- snap_point ---------------------------------------------------------

    #[test]
    fn snap_point_to_grid_within_threshold() {
        let engine = engine_with_zoom(1.0); // grid threshold = 4 world units
        let scene = Doc::new().scene;
        let r = engine.snap_point(DVec2::new(1.5, 9.0), &scene, &[]);
        // x=1.5 → grid 0; y=9 → grid 8 (since 9-8=1 ≤ 4)
        assert_eq!(r.world, DVec2::new(0.0, 8.0));
        assert!(matches!(r.x.unwrap().kind, SnapKind::Grid));
        assert!(matches!(r.y.unwrap().kind, SnapKind::Grid));
    }

    #[test]
    fn snap_point_outside_threshold_does_not_change_value() {
        let engine = SnapEngine {
            zoom: 1.0,
            grid: SnapGrid {
                spacing: 100.0,
                origin: [0.0, 0.0],
            },
            ..Default::default()
        };
        let scene = Doc::new().scene;
        // grid lines at 0 and 100. Threshold 4 world units. value=20 is too
        // far from either — should not snap.
        let r = engine.snap_point(DVec2::new(20.0, 20.0), &scene, &[]);
        assert!(r.x.is_none());
        assert!(r.y.is_none());
        assert_eq!(r.world, DVec2::new(20.0, 20.0));
    }

    // ---- snap to node edges/centers ----------------------------------------

    fn doc_with_rect(x: f64, y: f64, w: f64, h: f64) -> (Doc, NodeId) {
        let mut doc = Doc::new();
        let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x,
            y,
            w,
            h,
            Color::WHITE,
        )));
        let id = n.id;
        doc.apply(Operation::create_node(n)).unwrap();
        (doc, id)
    }

    #[test]
    fn snap_bounds_to_neighbor_left_edge() {
        let (doc, neighbor) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        // Disable grid so it doesn't compete with the edge candidate.
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES | SnapTargets::NODE_CENTERS,
            ..Default::default()
        };
        // A draft bounds where the right edge is at 98 — 2 world units away
        // from the neighbor's left edge (100). Should snap right→100.
        let draft = Bounds::from_xywh(48.0, 0.0, 50.0, 50.0);
        let r = engine.snap_bounds(draft, &doc.scene, &[]);
        let s = r.x.unwrap();
        // The snap landed on the neighbor's left edge.
        assert!(matches!(s.kind, SnapKind::NodeEdgeMin { source } if source == neighbor));
        assert_eq!(s.at, 100.0);
        assert!((s.delta - 2.0).abs() < 1e-9);
    }

    #[test]
    fn snap_bounds_excludes_dragged_node() {
        let (mut doc, neighbor) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        // Add a second node we're "dragging" near the neighbor.
        let dragged = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            48.0,
            0.0,
            50.0,
            50.0,
            Color::WHITE,
        )));
        let dragged_id = dragged.id;
        doc.apply(Operation::create_node(dragged)).unwrap();
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        let bb = doc.scene.world_bounds(dragged_id).unwrap();
        let r = engine.snap_bounds(bb, &doc.scene, &[dragged_id]);
        let s = r.x.unwrap();
        assert!(matches!(s.kind, SnapKind::NodeEdgeMin { source } if source == neighbor));
    }

    #[test]
    fn thresholds_scale_with_zoom() {
        // At zoom 2.0, a 6-px screen threshold equals 3 world units, so a
        // 4-world-unit gap should NOT snap. Same gap at zoom 0.5 (12-world-
        // unit threshold) should snap.
        let (doc, _id) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        let draft = Bounds::from_xywh(46.0, 0.0, 50.0, 50.0); // gap = 4 world units
        let engine_in = SnapEngine {
            zoom: 2.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        assert!(engine_in.snap_bounds(draft, &doc.scene, &[]).x.is_none());
        let engine_out = SnapEngine {
            zoom: 0.5,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        assert!(engine_out.snap_bounds(draft, &doc.scene, &[]).x.is_some());
    }

    #[test]
    fn snap_to_neighbor_center() {
        // Neighbor center at (50, 50). Draft bounds with cx near 50 should
        // snap to the center.
        let (doc, neighbor) = doc_with_rect(25.0, 25.0, 50.0, 50.0);
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_CENTERS,
            ..Default::default()
        };
        let draft = Bounds::from_xywh(48.0, 25.0, 4.0, 4.0); // cx = 50
        let r = engine.snap_bounds(draft, &doc.scene, &[]);
        // cx was already exactly 50 so delta is 0 — confirms the center
        // candidate was registered.
        assert!(matches!(
            r.x.unwrap().kind,
            SnapKind::NodeCenter { source } if source == neighbor
        ));
    }

    #[test]
    fn empty_scene_yields_no_snap_candidates() {
        let engine = engine_with_zoom(1.0);
        let scene = Doc::new().scene;
        let r = engine.snap_bounds(Bounds::from_xywh(123.0, 456.0, 10.0, 10.0), &scene, &[]);
        // Grid still applies; node candidates are absent. Since the value is
        // not near any 8-unit grid line, no snap.
        assert!(r.x.is_none() || matches!(r.x.unwrap().kind, SnapKind::Grid));
    }

    #[test]
    fn hidden_neighbors_do_not_produce_snap_candidates() {
        let (mut doc, _) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        // Toggle the only neighbor to hidden.
        let only_id = doc.scene.roots()[0];
        doc.scene.get_mut(only_id).unwrap().flags |= NodeFlags::HIDDEN;
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        let draft = Bounds::from_xywh(48.0, 0.0, 50.0, 50.0);
        let r = engine.snap_bounds(draft, &doc.scene, &[]);
        // Without the hidden neighbor as a candidate, no snap is produced.
        assert!(r.x.is_none() || matches!(r.x.unwrap().kind, SnapKind::Grid));
    }

    // ---- pixel grid ---------------------------------------------------------

    #[test]
    fn pixel_grid_quantizes_integer_world_units() {
        let engine = SnapEngine {
            zoom: 2.0,
            targets: SnapTargets::PIXEL_GRID,
            ..Default::default()
        };
        let scene = Doc::new().scene;
        let r = engine.snap_point(DVec2::new(3.0, 5.0), &scene, &[]);
        // The point is already integer — should report PixelGrid with delta 0.
        assert!(matches!(r.x.unwrap().kind, SnapKind::PixelGrid));
        assert!(matches!(r.y.unwrap().kind, SnapKind::PixelGrid));
    }

    // ---- composability ------------------------------------------------------

    #[test]
    fn snap_under_transform_uses_world_bounds() {
        // A node translated by +200 in X should produce candidates at 200 / 250.
        let mut doc = Doc::new();
        let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            50.0,
            50.0,
            Color::WHITE,
        )));
        n.transform = Transform2D::translation(200.0, 0.0);
        let neighbor = n.id;
        doc.apply(Operation::create_node(n)).unwrap();

        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        let draft = Bounds::from_xywh(198.0, 0.0, 50.0, 50.0);
        let r = engine.snap_bounds(draft, &doc.scene, &[]);
        let s = r.x.unwrap();
        assert!(matches!(s.kind, SnapKind::NodeEdgeMin { source } if source == neighbor));
        assert_eq!(s.at, 200.0);
    }

    // ---- cached candidates --------------------------------------------------

    #[test]
    fn collect_candidates_then_snap_bounds_with_matches_one_shot() {
        // The cached path must produce a bit-for-bit identical result to the
        // one-shot path — that equivalence is what lets the tool layer swap in
        // the cache without changing behavior.
        let (doc, neighbor) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES | SnapTargets::NODE_CENTERS,
            ..Default::default()
        };
        let draft = Bounds::from_xywh(48.0, 0.0, 50.0, 50.0);

        let one_shot = engine.snap_bounds(draft, &doc.scene, &[]);
        let cached = engine.collect_candidates(&doc.scene, &[]);
        let via_cache = engine.snap_bounds_with(&cached, draft);

        // Same chosen candidate, same coordinate, same delta.
        let a = one_shot.x.unwrap();
        let b = via_cache.x.unwrap();
        assert!(matches!(a.kind, SnapKind::NodeEdgeMin { source } if source == neighbor));
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.at, b.at);
        assert!((a.delta - b.delta).abs() < 1e-12);
        assert_eq!(one_shot.world, via_cache.world);
    }

    #[test]
    fn collect_candidates_then_snap_point_with_matches_one_shot() {
        let (doc, _neighbor) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES | SnapTargets::NODE_CENTERS,
            ..Default::default()
        };
        let probe = DVec2::new(98.0, 24.0);

        let one_shot = engine.snap_point(probe, &doc.scene, &[]);
        let cached = engine.collect_candidates(&doc.scene, &[]);
        let via_cache = engine.snap_point_with(&cached, probe);

        assert_eq!(one_shot.world, via_cache.world);
        assert_eq!(one_shot.x.map(|s| s.at), via_cache.x.map(|s| s.at));
        assert_eq!(one_shot.y.map(|s| s.at), via_cache.y.map(|s| s.at));
    }

    #[test]
    fn cached_candidates_are_stable_when_scene_mutates_mid_gesture() {
        // Collect once, then move the neighbor far away (simulating a stale
        // scene). Because the snapshot is by-value, the cached snap still locks
        // onto the *original* edge — proving the candidates are not recomputed
        // per frame. (In production the non-dragged nodes don't move, so this
        // staleness never manifests; the test exploits it to prove caching.)
        let (mut doc, neighbor) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        let cached = engine.collect_candidates(&doc.scene, &[]);

        // Yank the neighbor 10_000 units away. A fresh collect would now find
        // no candidate near x=100.
        doc.scene.get_mut(neighbor).unwrap().transform = Transform2D::translation(10_000.0, 0.0);

        let draft = Bounds::from_xywh(48.0, 0.0, 50.0, 50.0);
        let via_cache = engine.snap_bounds_with(&cached, draft);
        let s = via_cache.x.expect("cached candidate should still catch");
        assert!(matches!(s.kind, SnapKind::NodeEdgeMin { source } if source == neighbor));
        assert_eq!(s.at, 100.0);

        // Sanity: a fresh one-shot against the mutated scene no longer snaps
        // there, confirming the cache genuinely held the old geometry.
        let fresh = engine.snap_bounds(draft, &doc.scene, &[]);
        assert!(fresh.x.is_none());
    }

    #[test]
    fn collect_candidates_skips_excluded_nodes() {
        // Two rects; excluding one must drop its four edge candidates from the
        // snapshot (2 per axis).
        let (mut doc, _a) = doc_with_rect(100.0, 0.0, 50.0, 50.0);
        let dragged = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            50.0,
            50.0,
            Color::WHITE,
        )));
        let dragged_id = dragged.id;
        doc.apply(Operation::create_node(dragged)).unwrap();

        let engine = SnapEngine {
            zoom: 1.0,
            targets: SnapTargets::NODE_EDGES,
            ..Default::default()
        };
        let all = engine.collect_candidates(&doc.scene, &[]);
        let without = engine.collect_candidates(&doc.scene, &[dragged_id]);
        assert_eq!(all.x_len(), without.x_len() + 2);
        assert_eq!(all.y_len(), without.y_len() + 2);
        assert!(!without.is_empty());
    }
}

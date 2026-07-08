//! Refined hit-testing.
//!
//! `fanta-doc::Scene` already exposes an AABB-only `hit_test`; this module
//! adds path-precise vector hits (so clicking the *hole* inside a "C" shape
//! doesn't catch it), a deep variant that returns every node under a point,
//! and a marquee-style rect query for selection drags.
//!
//! ## Approach for point-in-path
//!
//! We flatten each [`PathSegment`] to a polyline (quadratic / cubic Béziers
//! subdivided with a fixed step), then apply the classic even-odd inclusion
//! test against the polyline. Pure Rust, no Skia dependency — `fanta-canvas`
//! stays renderer-agnostic.
//!
//! The flattening step is `O(segments × subdivisions)` with a small constant;
//! for design-tool path complexities it's well under a microsecond. When real
//! correctness matters (e.g. for export-time mesh ops) the renderer can
//! consult Skia's own `SkPath::contains` instead.
//!
//! [`PathSegment`]: fanta_doc::PathSegment

use crate::viewport::{screen_to_world, world_to_screen};
use fanta_doc::{
    Bounds, CanvasNode, NodeData, NodeFlags, NodeId, PathData, PathSegment, Scene, Viewport,
};
use glam::DVec2;
use smallvec::SmallVec;
/// How precise a vector hit-test should be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitPrecision {
    /// Fast — node's world AABB only (same as `Scene::hit_test`).
    Bounds,
    /// Precise — path-precise for [`NodeData::Vector`]; AABB fallback for
    /// every other variant.
    Path,
}

/// Marquee inclusion mode. Figma defaults to `Contains`; many tools offer
/// `Intersects` as the alt-key modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarqueeMode {
    /// Node's world bounds must be fully inside the marquee rectangle.
    Contains,
    /// Node's world bounds need only overlap the marquee.
    Intersects,
}

/// Number of straight segments used when flattening one Bézier curve.
/// 16 is the design-tool sweet spot — sub-pixel accuracy without measurable
/// cost.
pub const BEZIER_FLATTEN_STEPS: u32 = 16;

// =============================================================================
// Topmost hit
// =============================================================================

/// Whether `id` belongs to the active page's subtree. A node is "on" the page
/// iff its top-level root ancestor is `page`, or the node *is* the page itself.
///
/// `.fig` pages all share a coordinate origin, so without this scope a hit-test
/// would catch invisible nodes from pages that are not being rendered. Callers
/// pass the active page so clicks/marquee/move/reparent only consider nodes the
/// user can actually see. `page == None` means "no scoping" — every root is
/// eligible (back-compat for hand-authored single-page docs and tests).
pub fn on_active_page(scene: &Scene, id: NodeId, page: NodeId) -> bool {
    if id == page {
        return true;
    }
    // `page` scopes a hit when it is `id` itself or ANY ancestor of `id`. For a
    // real top-level page this is its root ancestor (the original semantics, so
    // cross-page rejection is unchanged). For a *nested* focus root — a component
    // master being edited in a component tab — this correctly scopes hits to that
    // master's subtree, so its inner layers are selectable (not just the master).
    scene.ancestors_of(id).any(|n| n.id == page)
}

/// Topmost node under `world_point` using the given precision, scoped to
/// `active_page` (see [`on_active_page`]). `active_page == None` searches all
/// roots.
///
/// Accelerated by `fanta-doc`'s spatial index: [`Scene::topmost_hit_where`]
/// enforces the structural predicate (top-z first, group exclusion,
/// ancestor-AABB containment, visibility) over only the indexed candidates
/// whose AABB contains the point; this closure layers on the active-page scope
/// and the path-precise refinement. The result matches the old full recursive
/// scan exactly when `active_page == None`.
pub fn hit_test(
    scene: &Scene,
    world_point: DVec2,
    precision: HitPrecision,
    active_page: Option<NodeId>,
) -> Option<NodeId> {
    scene.topmost_hit_where(world_point, |id| {
        page_scoped(scene, id, active_page)
            && !locked_by_flags(scene, id)
            && accept_precision(scene, id, world_point, precision)
    })
}

/// Whether `id` or any ancestor is locked. Unlike hidden nodes (whose subtrees
/// are dropped from the spatial index), locked nodes stay indexed, so point
/// hit-testing must reject them explicitly — otherwise a locked layer would
/// still be clickable and draggable in the canvas. Rejecting it lets the hit
/// fall through to whatever sits behind, matching Figma's lock behavior.
fn locked_by_flags(scene: &Scene, id: NodeId) -> bool {
    scene
        .get(id)
        .is_some_and(|node| node.flags.contains(NodeFlags::LOCKED))
        || scene
            .ancestors_of(id)
            .any(|node| node.flags.contains(NodeFlags::LOCKED))
}

/// Apply the active-page scope: accept every node when `page == None`,
/// otherwise only nodes whose top-level root ancestor is the active page.
fn page_scoped(scene: &Scene, id: NodeId, page: Option<NodeId>) -> bool {
    match page {
        Some(page) => {
            // A top-level page's own body is never a hit target, even though
            // it can carry the canvas background fill (which makes it a
            // "frame surface"): clicking the canvas backdrop deselects, like
            // Figma. Nested focus roots — a component master edited in its
            // own tab — remain selectable through their body.
            if id == page && scene.get(page).is_some_and(|node| node.parent.is_none()) {
                return false;
            }
            on_active_page(scene, id, page)
        }
        None => true,
    }
}

/// The path-precision refinement applied on top of the scene's structural
/// hit predicate: in [`HitPrecision::Path`] a [`NodeData::Vector`] hits only if
/// the point is inside its filled path; every other case accepts (the AABB
/// containment the index already verified is enough).
fn accept_precision(
    scene: &Scene,
    id: NodeId,
    world_point: DVec2,
    precision: HitPrecision,
) -> bool {
    let Some(node) = scene.get(id) else {
        return false;
    };
    match (precision, &node.data) {
        (HitPrecision::Path, NodeData::Vector(v)) => {
            let local = world_to_local(scene, node, world_point);
            point_in_path(&v.path, local)
        }
        _ => true,
    }
}

/// Topmost node under `screen_point`, with viewport translation handled here so
/// callers in the tool layer never re-derive the world coordinate.
pub fn hit_test_screen(
    scene: &Scene,
    viewport: &Viewport,
    screen_size: DVec2,
    screen_point: DVec2,
    precision: HitPrecision,
    active_page: Option<NodeId>,
) -> Option<NodeId> {
    let world = screen_to_world(screen_point, viewport, screen_size);
    hit_test(scene, world, precision, active_page)
}

// =============================================================================
// Deep hit (all nodes at a point, ordered)
// =============================================================================

/// Every node under `world_point`, sorted top-z first, scoped to `active_page`
/// (see [`on_active_page`]). `active_page == None` searches all roots. Useful
/// for alt-click "select behind" UX and for debugging.
///
/// Accelerated by the scene's spatial index via [`Scene::deep_hits_where`];
/// the per-node predicate (group exclusion, ancestor containment, visibility)
/// matches the old recursive collector, this closure adds the active-page scope
/// and path precision.
pub fn hit_test_deep(
    scene: &Scene,
    world_point: DVec2,
    precision: HitPrecision,
    active_page: Option<NodeId>,
) -> SmallVec<[NodeId; 8]> {
    scene
        .deep_hits_where(world_point, |id| {
            page_scoped(scene, id, active_page)
                && !locked_by_flags(scene, id)
                && accept_precision(scene, id, world_point, precision)
        })
        .into_iter()
        .collect()
}

// =============================================================================
// Marquee (rect-based selection)
// =============================================================================

/// All nodes whose world bounds satisfy the given inclusion mode relative to
/// `world_rect`, scoped to `active_page` (see [`on_active_page`]). `active_page
/// == None` collects from all roots. Excludes groups themselves (a marquee
/// selects leaves, not the containing group — Figma convention).
///
/// Accelerated by the scene's spatial index via [`Scene::rect_query_where`]:
/// only nodes whose world AABB overlaps `world_rect` are considered, then the
/// active-page scope, inclusion test, and locked/hidden-ancestor filtering
/// select the result. The returned set and its bottom-z-first order match the
/// old full recursion when `active_page == None`.
pub fn hit_test_within(
    scene: &Scene,
    world_rect: Bounds,
    mode: MarqueeMode,
    active_page: Option<NodeId>,
) -> SmallVec<[NodeId; 16]> {
    scene
        .rect_query_where(world_rect, |id, bb| {
            if !page_scoped(scene, id, active_page) {
                return false;
            }
            // Skip a node if it or any ancestor is hidden or locked — the
            // recursive marquee never descends past such a node. (Hidden
            // subtrees are already absent from the index; locked is not, so we
            // check the whole chain here.)
            if marquee_blocked_by_flags(scene, id) {
                return false;
            }
            match mode {
                MarqueeMode::Contains => {
                    bb.min_x >= world_rect.min_x
                        && bb.min_y >= world_rect.min_y
                        && bb.max_x <= world_rect.max_x
                        && bb.max_y <= world_rect.max_y
                }
                MarqueeMode::Intersects => bb.intersects(&world_rect),
            }
        })
        .into_iter()
        .collect()
}

/// Whether a marquee should skip `id` because it — or any ancestor — is hidden
/// or locked. Mirrors the early-return-without-recursing branch of the old
/// recursive `collect_within`.
fn marquee_blocked_by_flags(scene: &Scene, id: NodeId) -> bool {
    if let Some(node) = scene.get(id) {
        if node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED) {
            return true;
        }
    }
    scene
        .ancestors_of(id)
        .any(|a| a.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED))
}

/// Convenience: marquee in screen-space coords, scoped to `active_page`.
pub fn hit_test_within_screen(
    scene: &Scene,
    viewport: &Viewport,
    screen_size: DVec2,
    screen_rect: Bounds,
    mode: MarqueeMode,
    active_page: Option<NodeId>,
) -> SmallVec<[NodeId; 16]> {
    let p0 = screen_to_world(
        DVec2::new(screen_rect.min_x, screen_rect.min_y),
        viewport,
        screen_size,
    );
    let p1 = screen_to_world(
        DVec2::new(screen_rect.max_x, screen_rect.max_y),
        viewport,
        screen_size,
    );
    let world_rect = Bounds::from_min_max(p0.min(p1), p0.max(p1));
    hit_test_within(scene, world_rect, mode, active_page)
}

// =============================================================================
// Path containment
// =============================================================================

/// Test whether `point` (in node-local space) is inside the path's filled
/// area. Uses the even-odd rule on a flattened polyline approximation.
pub fn point_in_path(path: &PathData, point: DVec2) -> bool {
    let polylines = flatten_to_polylines(path);
    let mut inside = false;
    for poly in &polylines {
        if poly.len() < 3 {
            continue;
        }
        if even_odd_inside(poly, point) {
            inside = !inside;
        }
    }
    inside
}

/// Flatten a path into one or more closed polylines. Each `Move` starts a new
/// polyline. `Close` returns to the start of the current one.
fn flatten_to_polylines(path: &PathData) -> Vec<Vec<DVec2>> {
    let mut out: Vec<Vec<DVec2>> = Vec::new();
    let mut current: Vec<DVec2> = Vec::new();
    let mut subpath_start: Option<DVec2> = None;
    let mut last = DVec2::ZERO;

    let push = |out: &mut Vec<Vec<DVec2>>, cur: &mut Vec<DVec2>| {
        if cur.len() >= 2 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };

    for seg in &path.segments {
        match *seg {
            PathSegment::Move { to } => {
                push(&mut out, &mut current);
                let p = DVec2::new(to[0], to[1]);
                current.push(p);
                subpath_start = Some(p);
                last = p;
            }
            PathSegment::Line { to } => {
                let p = DVec2::new(to[0], to[1]);
                current.push(p);
                last = p;
            }
            PathSegment::Quad { ctrl, to } => {
                let c = DVec2::new(ctrl[0], ctrl[1]);
                let p = DVec2::new(to[0], to[1]);
                flatten_quad(last, c, p, &mut current);
                last = p;
            }
            PathSegment::Cubic { ctrl1, ctrl2, to } => {
                let c1 = DVec2::new(ctrl1[0], ctrl1[1]);
                let c2 = DVec2::new(ctrl2[0], ctrl2[1]);
                let p = DVec2::new(to[0], to[1]);
                flatten_cubic(last, c1, c2, p, &mut current);
                last = p;
            }
            PathSegment::Close => {
                if let Some(start) = subpath_start {
                    if current.last().copied() != Some(start) {
                        current.push(start);
                    }
                    last = start;
                }
                push(&mut out, &mut current);
                subpath_start = None;
            }
        }
    }
    push(&mut out, &mut current);
    out
}

fn flatten_quad(p0: DVec2, c: DVec2, p1: DVec2, out: &mut Vec<DVec2>) {
    let n = BEZIER_FLATTEN_STEPS as usize;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let one_minus_t = 1.0 - t;
        let p = one_minus_t * one_minus_t * p0 + 2.0 * one_minus_t * t * c + t * t * p1;
        out.push(p);
    }
}

fn flatten_cubic(p0: DVec2, c1: DVec2, c2: DVec2, p1: DVec2, out: &mut Vec<DVec2>) {
    let n = BEZIER_FLATTEN_STEPS as usize;
    for i in 1..=n {
        let t = i as f64 / n as f64;
        let one_minus_t = 1.0 - t;
        let p = one_minus_t.powi(3) * p0
            + 3.0 * one_minus_t.powi(2) * t * c1
            + 3.0 * one_minus_t * t.powi(2) * c2
            + t.powi(3) * p1;
        out.push(p);
    }
}

/// Even-odd inclusion test for a polyline interpreted as a closed polygon.
fn even_odd_inside(poly: &[DVec2], p: DVec2) -> bool {
    // Standard horizontal-ray casting.
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let a = poly[i];
        let b = poly[j];
        let crosses =
            (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x;
        if crosses {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// =============================================================================
// Helpers
// =============================================================================

fn world_to_local(scene: &Scene, node: &CanvasNode, world: DVec2) -> DVec2 {
    if let Some(t) = scene.world_transform(node.id) {
        t.inverse().transform_point(world)
    } else {
        world
    }
}

/// Convert a node's local bounds back to screen-space — handy for UI overlays
/// that need to draw a selection handle without re-deriving the transform.
pub fn node_screen_bounds(
    scene: &Scene,
    id: NodeId,
    viewport: &Viewport,
    screen_size: DVec2,
) -> Option<Bounds> {
    let world = scene.world_bounds(id)?;
    let tl = world_to_screen(DVec2::new(world.min_x, world.min_y), viewport, screen_size);
    let br = world_to_screen(DVec2::new(world.max_x, world.max_y), viewport, screen_size);
    Some(Bounds::from_min_max(tl.min(br), tl.max(br)))
}

#[cfg(test)]
mod tests;

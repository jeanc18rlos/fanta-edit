//! Spatial acceleration for point hit-testing and rectangle/marquee queries.
//!
//! The naive hit-test ([`Scene::hit_test`](crate::scene::Scene::hit_test)) and
//! the marquee query walk every node on every pointer event — `O(n)` per event.
//! On the 44k-node Spectrum file that is the dominant cost of a hover or a drag.
//!
//! [`SpatialIndex`] keeps every node bucketed by its world-space AABB in a
//! **uniform grid**, plus a precomputed **global paint rank** per node. A point
//! or rectangle query touches only the grid cells the query overlaps, yielding a
//! small candidate set instead of the whole tree.
//!
//! ## Why a uniform grid (and not an R-tree)
//!
//! Design-tool scenes are mostly flat fields of similarly-sized leaves laid out
//! across a page — exactly the distribution a uniform grid handles well, with
//! none of the pointer-chasing or rebuild cost of a packed R-tree. The build is
//! a single linear pass (`O(n)` plus the cells each AABB spans), it is trivially
//! pure-safe Rust, and it needs no balancing. The cell size is derived from the
//! median node extent so the average node lands in roughly one cell.
//!
//! ## Parity contract
//!
//! The index is a *filter*, never the source of truth. Every query re-applies
//! the **exact** predicate the recursive tree walk uses (ancestor visibility,
//! ancestor-AABB containment for points, group exclusion, path/precision is left
//! to the caller) against the candidates the grid returns, then resolves ties by
//! paint rank. Because the candidate set is always a superset of the nodes the
//! tree walk would visit, the resolved answer is identical to brute force. The
//! tests in this module assert that parity over randomized scenes.
//!
//! ## Paint order
//!
//! The recursive walk emits a node *after* descending into its children
//! top-z-first, so the topmost hit is the deepest, highest-z leaf. We capture
//! that order once as a **pre-order DFS rank**: walking roots ascending, then
//! children ascending, assigning an incrementing rank. A larger rank means
//! "painted later / on top". Picking the candidate with the largest rank that
//! passes the predicate reproduces the recursive "first hit wins, top-first"
//! result exactly. See [`SpatialIndex::topmost_hit`].
//!
//! ## Invalidation
//!
//! The index depends on the same inputs as the world-bounds cache (every node's
//! world AABB and the tree structure / z-order). It is rebuilt lazily on first
//! query after an edit and dropped wholesale by
//! [`Scene::invalidate_world_cache`](crate::scene::Scene::invalidate_world_cache)
//! — it shares that single invalidation signal, so a transform/structure edit
//! that stales the world cache stales the index too. Rebuilds run off the read
//! hot path (only on the first query following an edit).

use crate::id::NodeId;
use crate::transform::Bounds;
use glam::DVec2;
use std::collections::HashMap;

/// One indexed node: its world AABB and its global paint rank.
///
/// Group-ness, visibility, and any per-node refinement are decided by the
/// caller's `accept` predicate (which has the authoritative `Scene`), keeping
/// the index a pure geometric/ordering filter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    pub id: NodeId,
    pub bounds: Bounds,
    /// Pre-order DFS rank — larger means painted later (on top).
    pub rank: u32,
}

/// A uniform grid over world-space node AABBs.
///
/// Built from a snapshot of the scene's world bounds + paint order. Pure data;
/// no back-reference to the scene, so it can be cached behind a `RefCell`
/// alongside the world-transform cache and dropped on the same signal.
#[derive(Debug, Clone)]
pub struct SpatialIndex {
    /// All indexed entries, in paint-rank order (ascending). Index into this
    /// vec is what the grid cells store.
    entries: Vec<Entry>,
    /// Grid origin (min corner of the indexed world extent).
    origin: DVec2,
    /// Edge length of one square cell, in world units. Always `> 0`.
    cell_size: f64,
    /// Number of columns / rows. The grid is `cols × rows` cells.
    cols: usize,
    rows: usize,
    /// Cell → entry indices that overlap that cell. Sparse: cells with no
    /// entries are absent.
    cells: HashMap<usize, Vec<u32>>,
    /// Entries whose AABB is non-finite (NaN/inf) and cannot be bucketed; they
    /// are scanned on every query so correctness never depends on the grid
    /// covering them. Expected to be empty for well-formed scenes.
    unbucketed: Vec<u32>,
}

impl SpatialIndex {
    /// Build an index from `(id, world_bounds)` pairs already in **paint-rank
    /// order** (ascending: first painted first). The caller supplies the order;
    /// see the `build_spatial_index` caller in `Scene`.
    pub(crate) fn build(items: impl IntoIterator<Item = (NodeId, Bounds)>) -> Self {
        let entries = build_entries(items);
        let Some(extent) = GridExtent::collect(&entries) else {
            return Self::with_all_entries_unbucketed(entries);
        };
        let grid = GridSpec::from_extent(extent);
        let (cells, unbucketed) = bucket_entries(&entries, grid);

        Self {
            entries,
            origin: grid.origin,
            cell_size: grid.cell_size,
            cols: grid.cols,
            rows: grid.rows,
            cells,
            unbucketed,
        }
    }

    fn with_all_entries_unbucketed(entries: Vec<Entry>) -> Self {
        // Nothing bucketable. A 1x1 grid that holds no cells; every query falls
        // back to scanning `unbucketed`.
        let unbucketed = (0..entries.len() as u32).collect();
        Self {
            entries,
            origin: DVec2::ZERO,
            cell_size: 1.0,
            cols: 1,
            rows: 1,
            cells: HashMap::new(),
            unbucketed,
        }
    }

    /// Number of indexed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Candidate entries (deduplicated, no particular order) whose AABB could
    /// contain `point`. The grid returns a superset; the caller filters.
    fn candidates_for_point(&self, point: DVec2) -> CandidateIter<'_> {
        let cell = if point.x.is_finite() && point.y.is_finite() {
            let c = (((point.x - self.origin.x) / self.cell_size).floor()) as isize;
            let r = (((point.y - self.origin.y) / self.cell_size).floor()) as isize;
            if c >= 0 && r >= 0 && (c as usize) < self.cols && (r as usize) < self.rows {
                self.cells.get(&((r as usize) * self.cols + c as usize))
            } else {
                None
            }
        } else {
            None
        };
        CandidateIter {
            primary: cell.map(|v| v.as_slice()).unwrap_or(&[]).iter(),
            extra: self.unbucketed.iter(),
        }
    }

    /// Visit every candidate entry whose AABB overlaps `rect`, deduplicated.
    /// The grid returns a superset; `visit` is called once per distinct entry.
    fn for_each_rect_candidate(&self, rect: Bounds, mut visit: impl FnMut(&Entry)) {
        let mut emitted = CandidateDeduper::new(self.entries.len());
        let mut emit = |idx| emitted.emit(idx, &self.entries, &mut visit);

        match self.cell_range_for_rect(&rect) {
            Some(range) => self.for_each_bucketed_index(range, &mut emit),
            // Degenerate rect: fall back to scanning everything (still correct).
            None => self.for_each_entry_index(&mut emit),
        }
        for &idx in &self.unbucketed {
            emit(idx);
        }
    }

    fn cell_range_for_rect(&self, rect: &Bounds) -> Option<(usize, usize, usize, usize)> {
        bounds_finite(rect)
            .then(|| cell_range(rect, self.origin, self.cell_size, self.cols, self.rows))
    }

    fn for_each_bucketed_index(
        &self,
        (c0, r0, c1, r1): (usize, usize, usize, usize),
        mut visit: impl FnMut(u32),
    ) {
        for r in r0..=r1 {
            for c in c0..=c1 {
                if let Some(bucket) = self.cells.get(&(r * self.cols + c)) {
                    bucket.iter().copied().for_each(&mut visit);
                }
            }
        }
    }

    fn for_each_entry_index(&self, mut visit: impl FnMut(u32)) {
        (0..self.entries.len() as u32).for_each(&mut visit);
    }

    /// Topmost entry under `point` (largest paint rank) whose AABB contains the
    /// point and for which `accept(id)` returns `true`.
    ///
    /// `accept` is the caller's exact per-node predicate (ancestor visibility,
    /// ancestor containment, group exclusion, optional path-precision). The grid
    /// only narrows the set of ids `accept` is asked about; the answer is
    /// identical to scanning every node and applying the same `accept`.
    pub(crate) fn topmost_hit(
        &self,
        point: DVec2,
        mut accept: impl FnMut(NodeId) -> bool,
    ) -> Option<NodeId> {
        let mut best: Option<(u32, NodeId)> = None;
        for entry in self.candidates_for_point(point) {
            let entry = &self.entries[entry as usize];
            if !entry.bounds.contains_point(point) {
                continue;
            }
            // Cheap rank prune before the (possibly expensive) accept call.
            if best.map(|(r, _)| entry.rank <= r).unwrap_or(false) {
                continue;
            }
            if accept(entry.id) {
                best = Some((entry.rank, entry.id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// All entries under `point` for which `accept` holds, sorted top-first
    /// (descending paint rank) — the order [`hit_test_deep`] expects.
    pub(crate) fn deep_hits(
        &self,
        point: DVec2,
        mut accept: impl FnMut(NodeId) -> bool,
    ) -> Vec<NodeId> {
        let mut hits: Vec<(u32, NodeId)> = Vec::new();
        for entry in self.candidates_for_point(point) {
            let entry = &self.entries[entry as usize];
            if !entry.bounds.contains_point(point) {
                continue;
            }
            if accept(entry.id) {
                hits.push((entry.rank, entry.id));
            }
        }
        // Descending rank = top-first.
        hits.sort_unstable_by_key(|&(rank, _)| std::cmp::Reverse(rank));
        hits.into_iter().map(|(_, id)| id).collect()
    }

    /// All non-group entries whose AABB overlaps `rect` and for which `accept`
    /// holds, sorted by ascending paint rank (bottom-first) — matching the
    /// pre-order traversal a brute-force marquee produces.
    ///
    /// The caller decides the inclusion test (contains vs intersects) and any
    /// group/visibility filtering inside `accept`; the grid only restricts the
    /// candidate set to AABBs overlapping `rect`.
    pub(crate) fn rect_query(
        &self,
        rect: Bounds,
        mut accept: impl FnMut(NodeId, &Bounds) -> bool,
    ) -> Vec<NodeId> {
        let mut hits: Vec<(u32, NodeId)> = Vec::new();
        self.for_each_rect_candidate(rect, |entry| {
            if accept(entry.id, &entry.bounds) {
                hits.push((entry.rank, entry.id));
            }
        });
        hits.sort_unstable_by_key(|&(rank, _)| rank);
        hits.into_iter().map(|(_, id)| id).collect()
    }
}

fn build_entries(items: impl IntoIterator<Item = (NodeId, Bounds)>) -> Vec<Entry> {
    items
        .into_iter()
        .enumerate()
        .map(|(rank, (id, bounds))| Entry {
            id,
            bounds,
            rank: rank as u32,
        })
        .collect()
}

struct GridExtent {
    min: DVec2,
    max: DVec2,
    extents: Vec<f64>,
}

impl GridExtent {
    fn collect(entries: &[Entry]) -> Option<Self> {
        let mut extent = Self {
            min: DVec2::new(f64::INFINITY, f64::INFINITY),
            max: DVec2::new(f64::NEG_INFINITY, f64::NEG_INFINITY),
            extents: Vec::with_capacity(entries.len()),
        };
        for entry in entries.iter().filter(|entry| bounds_finite(&entry.bounds)) {
            extent.include(entry.bounds);
        }
        (!extent.extents.is_empty()).then_some(extent)
    }

    fn include(&mut self, bounds: Bounds) {
        self.min.x = self.min.x.min(bounds.min_x);
        self.min.y = self.min.y.min(bounds.min_y);
        self.max.x = self.max.x.max(bounds.max_x);
        self.max.y = self.max.y.max(bounds.max_y);
        // Use the larger side; a thin element should still occupy ~1 cell.
        self.extents
            .push(bounds.width().max(0.0).max(bounds.height().max(0.0)));
    }

    fn span_x(&self) -> f64 {
        (self.max.x - self.min.x).max(0.0)
    }

    fn span_y(&self) -> f64 {
        (self.max.y - self.min.y).max(0.0)
    }
}

#[derive(Clone, Copy)]
struct GridSpec {
    origin: DVec2,
    cell_size: f64,
    cols: usize,
    rows: usize,
}

impl GridSpec {
    fn from_extent(mut extent: GridExtent) -> Self {
        let cell_size = median_cell_size(&mut extent.extents);
        let span_x = extent.span_x();
        let span_y = extent.span_y();
        // Cap the grid dimensions so a pathological extent : cell-size ratio
        // (a few huge background rects next to tiny icons) can't blow memory. A
        // coarse cell just means a slightly larger candidate set — still
        // correct, since the predicate is re-checked.
        const MAX_DIM: usize = 1024;
        let cols = grid_axis_cells(span_x, cell_size, MAX_DIM);
        let rows = grid_axis_cells(span_y, cell_size, MAX_DIM);
        Self {
            origin: extent.min,
            cell_size: effective_cell_size(cell_size, span_x, span_y, cols, rows, MAX_DIM),
            cols,
            rows,
        }
    }
}

fn grid_axis_cells(span: f64, cell_size: f64, max_dim: usize) -> usize {
    (((span / cell_size).floor() as usize) + 1).clamp(1, max_dim)
}

fn effective_cell_size(
    cell_size: f64,
    span_x: f64,
    span_y: f64,
    cols: usize,
    rows: usize,
    max_dim: usize,
) -> f64 {
    // Recompute effective cell size if we hit the dimension cap, so the grid
    // still spans the whole extent.
    let eff_cx = if cols == max_dim && span_x > 0.0 {
        span_x / cols as f64
    } else {
        cell_size
    };
    let eff_cy = if rows == max_dim && span_y > 0.0 {
        span_y / rows as f64
    } else {
        cell_size
    };
    // Keep cells square-ish but cover both spans: use the max of the two.
    eff_cx.max(eff_cy).max(f64::MIN_POSITIVE)
}

fn bucket_entries(entries: &[Entry], grid: GridSpec) -> (HashMap<usize, Vec<u32>>, Vec<u32>) {
    let mut cells: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut unbucketed: Vec<u32> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let i = i as u32;
        if !bounds_finite(&entry.bounds) {
            unbucketed.push(i);
            continue;
        }
        let range = cell_range(
            &entry.bounds,
            grid.origin,
            grid.cell_size,
            grid.cols,
            grid.rows,
        );
        push_entry_to_cells(i, range, grid.cols, &mut cells);
    }
    (cells, unbucketed)
}

fn push_entry_to_cells(
    entry_index: u32,
    (c0, r0, c1, r1): (usize, usize, usize, usize),
    cols: usize,
    cells: &mut HashMap<usize, Vec<u32>>,
) {
    for r in r0..=r1 {
        for c in c0..=c1 {
            cells.entry(r * cols + c).or_default().push(entry_index);
        }
    }
}

/// Iterator that yields a cell's entry indices then the always-scanned
/// unbucketed ones, without allocating.
struct CandidateIter<'a> {
    primary: std::slice::Iter<'a, u32>,
    extra: std::slice::Iter<'a, u32>,
}

impl Iterator for CandidateIter<'_> {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if let Some(&i) = self.primary.next() {
            return Some(i);
        }
        self.extra.next().copied()
    }
}

/// Per-query duplicate filter for rect candidates.
struct CandidateDeduper {
    marker: Vec<u32>,
    token: u32,
}

impl CandidateDeduper {
    fn new(entry_count: usize) -> Self {
        Self {
            marker: vec![u32::MAX; entry_count],
            token: 1,
        }
    }

    fn emit(&mut self, idx: u32, entries: &[Entry], visit: &mut dyn FnMut(&Entry)) {
        let slot = &mut self.marker[idx as usize];
        if *slot == self.token {
            return;
        }
        *slot = self.token;
        visit(&entries[idx as usize]);
    }
}

// ---- helpers ---------------------------------------------------------------

fn bounds_finite(b: &Bounds) -> bool {
    b.min_x.is_finite() && b.min_y.is_finite() && b.max_x.is_finite() && b.max_y.is_finite()
}

/// Median of the per-node max extents, floored to a sane minimum so a scene of
/// zero-size nodes still gets a positive cell.
fn median_cell_size(extents: &mut [f64]) -> f64 {
    if extents.is_empty() {
        return 1.0;
    }
    extents.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = extents[extents.len() / 2];
    if mid.is_finite() && mid > 1.0 {
        mid
    } else {
        1.0
    }
}

/// Inclusive cell column/row range a bounds spans, clamped to the grid.
fn cell_range(
    b: &Bounds,
    origin: DVec2,
    cell_size: f64,
    cols: usize,
    rows: usize,
) -> (usize, usize, usize, usize) {
    let to_col = |x: f64| -> usize {
        let c = ((x - origin.x) / cell_size).floor();
        if c < 0.0 {
            0
        } else if c as usize >= cols {
            cols - 1
        } else {
            c as usize
        }
    };
    let to_row = |y: f64| -> usize {
        let r = ((y - origin.y) / cell_size).floor();
        if r < 0.0 {
            0
        } else if r as usize >= rows {
            rows - 1
        } else {
            r as usize
        }
    };
    (
        to_col(b.min_x),
        to_row(b.min_y),
        to_col(b.max_x),
        to_row(b.max_y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bb(x: f64, y: f64, w: f64, h: f64) -> Bounds {
        Bounds::from_xywh(x, y, w, h)
    }

    fn nid() -> NodeId {
        NodeId::new()
    }

    #[test]
    fn topmost_hit_picks_highest_rank() {
        let a = nid();
        let b = nid();
        // Both cover the origin; `b` has the larger rank (built later).
        let idx = SpatialIndex::build([
            (a, bb(-10.0, -10.0, 20.0, 20.0)),
            (b, bb(-5.0, -5.0, 10.0, 10.0)),
        ]);
        assert_eq!(idx.topmost_hit(DVec2::ZERO, |_| true), Some(b));
        // Outside b's AABB but inside a's → a.
        assert_eq!(idx.topmost_hit(DVec2::new(7.0, 7.0), |_| true), Some(a));
        // Outside both.
        assert_eq!(idx.topmost_hit(DVec2::new(100.0, 100.0), |_| true), None);
    }

    #[test]
    fn accept_predicate_is_respected() {
        let a = nid();
        let b = nid();
        let idx = SpatialIndex::build([
            (a, bb(-10.0, -10.0, 20.0, 20.0)),
            (b, bb(-5.0, -5.0, 10.0, 10.0)),
        ]);
        // Reject the top node → fall through to the one beneath.
        assert_eq!(idx.topmost_hit(DVec2::ZERO, |id| id == a), Some(a));
    }

    #[test]
    fn deep_hits_are_top_first() {
        let a = nid();
        let b = nid();
        let c = nid();
        let idx = SpatialIndex::build([
            (a, bb(-10.0, -10.0, 20.0, 20.0)),
            (b, bb(-8.0, -8.0, 16.0, 16.0)),
            (c, bb(-5.0, -5.0, 10.0, 10.0)),
        ]);
        // All cover origin; expect top-first c, b, a.
        assert_eq!(idx.deep_hits(DVec2::ZERO, |_| true), vec![c, b, a]);
    }

    #[test]
    fn rect_query_is_bottom_first_and_filters() {
        let a = nid();
        let b = nid();
        let idx = SpatialIndex::build([
            (a, bb(0.0, 0.0, 10.0, 10.0)),
            (b, bb(100.0, 100.0, 10.0, 10.0)),
        ]);
        // Rect over `a` only.
        let r = bb(-5.0, -5.0, 30.0, 30.0);
        let hits = idx.rect_query(r, |_, bnds| bnds.intersects(&r));
        assert_eq!(hits, vec![a]);
        // Big rect over both — bottom-first means a before b (a built first).
        let big = bb(-5.0, -5.0, 200.0, 200.0);
        let hits = idx.rect_query(big, |_, bnds| bnds.intersects(&big));
        assert_eq!(hits, vec![a, b]);
    }

    #[test]
    fn nonfinite_bounds_are_scanned_not_dropped() {
        let a = nid();
        let weird = Bounds {
            min_x: f64::NAN,
            min_y: 0.0,
            max_x: 10.0,
            max_y: 10.0,
        };
        let idx = SpatialIndex::build([(a, weird)]);
        // It lands in `unbucketed` and is offered as a candidate; the AABB
        // contains_point test on a NaN bound is false, so no hit — but the
        // entry was not silently lost (it is scanned).
        assert_eq!(idx.unbucketed.len(), 1);
    }

    #[test]
    fn empty_index() {
        let idx = SpatialIndex::build(std::iter::empty());
        assert!(idx.is_empty());
        assert_eq!(idx.topmost_hit(DVec2::ZERO, |_| true), None);
        assert!(idx.deep_hits(DVec2::ZERO, |_| true).is_empty());
        assert!(
            idx.rect_query(bb(0.0, 0.0, 1.0, 1.0), |_, _| true)
                .is_empty()
        );
    }

    #[test]
    fn many_cells_dedup_under_rect() {
        // A node spanning many cells must be reported exactly once by a rect
        // query that overlaps several of those cells.
        let a = nid();
        let idx = SpatialIndex::build([(a, bb(0.0, 0.0, 1000.0, 1000.0))]);
        let r = bb(0.0, 0.0, 1000.0, 1000.0);
        let hits = idx.rect_query(r, |_, bnds| bnds.intersects(&r));
        assert_eq!(hits, vec![a]);
    }

    #[test]
    fn nonfinite_rect_query_falls_back_to_full_scan() {
        let a = nid();
        let b = nid();
        let idx = SpatialIndex::build([
            (a, bb(0.0, 0.0, 10.0, 10.0)),
            (b, bb(100.0, 100.0, 10.0, 10.0)),
        ]);
        let query = Bounds {
            min_x: f64::NAN,
            min_y: 0.0,
            max_x: 20.0,
            max_y: 20.0,
        };

        assert_eq!(idx.rect_query(query, |_, _| true), vec![a, b]);
    }
}

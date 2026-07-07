//! World geometry, the lazily-built spatial index, and hit-testing for
//! [`Scene`] — a second `impl Scene` block over the `pub(crate)` fields defined
//! in [`crate::scene::graph`].

use crate::id::NodeId;
use crate::node::{NodeData, NodeFlags};
use crate::scene::graph::Scene;
use crate::spatial::SpatialIndex;
use crate::transform::{Bounds, Transform2D};
use glam::DVec2;

impl Scene {
    // ---- world geometry ------------------------------------------------------

    /// Composed transform from world space down to `id`'s local space. Returns
    /// `None` if `id` is not in the scene.
    ///
    /// ## Allocation-free hot path
    ///
    /// This is called per node by hit-testing, snapping, and selection drawing,
    /// so it must not allocate. Instead of collecting the ancestor chain into a
    /// `Vec` and folding it, we compute parent-first:
    ///
    /// ```text
    /// world_transform(id) = world_transform(parent) ∘ local(id)
    /// ```
    ///
    /// and memoize each node's result in [`Self::world_cache`]. The recursion is
    /// bounded by tree depth (handfuls, not the 44k node count) and every level
    /// is cached, so a warm cache answers in O(1) and a cold one fills each
    /// ancestor exactly once. See the field docs on `world_cache` for the
    /// invalidation contract. The recursion borrows the cache only briefly at
    /// each level (never across the recursive call) so re-entrant `RefCell`
    /// borrows cannot overlap.
    ///
    /// [`Self::world_cache`]: crate::scene::Scene
    pub fn world_transform(&self, id: NodeId) -> Option<Transform2D> {
        // Fast path: already memoized.
        if let Some(cached) = self.world_cache.borrow().get(&id) {
            return Some(*cached);
        }
        // Not cached — the node must exist for a transform to be defined.
        let node = self.nodes.get(&id)?;
        // Parent-first: a node's world transform is its parent's world
        // transform composed with its own local transform. Root nodes (no
        // parent) compose against identity. `then` applies `self` first, then
        // the argument, so `local.then(&parent_world)` yields parent ∘ local —
        // the same root→leaf order the old fold produced.
        let parent_world = match node.parent {
            Some(parent) => self.world_transform(parent)?,
            None => Transform2D::IDENTITY,
        };
        let world = node.transform.then(&parent_world);
        self.world_cache.borrow_mut().insert(id, world);
        Some(world)
    }

    /// Drop every memoized world transform *and* local-bounds entry. Called by
    /// every mutator that can change a node's transform, geometry, or place in
    /// the hierarchy — see the invalidation contracts on the cache fields. Both
    /// share one signal because a single transform/structure edit can stale
    /// entries in either. Cheap relative to the edit itself, and only ever runs
    /// off the read hot path.
    ///
    /// `pub` so callers that mutate node transforms *directly* through a
    /// `&mut Scene` (e.g. the auto-layout solver) can restore cache consistency
    /// after the edit.
    pub fn invalidate_world_cache(&self) {
        self.world_cache.borrow_mut().clear();
        self.local_bounds_cache.borrow_mut().clear();
        // The spatial index is derived from world AABBs + paint order, so any
        // edit that stales those caches stales the index too (see its field
        // docs). Drop it; the next spatial query rebuilds lazily.
        *self.spatial_index.borrow_mut() = None;
        self.revision.set(self.revision.get().wrapping_add(1));
    }

    /// The scene's content revision — see the field docs: equal values mean
    /// "nothing render-relevant changed", so callers can key memoized work
    /// (retained surfaces, chrome scans) on `(revision, viewport, …)`.
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    /// World-space axis-aligned bounds for `id`. `None` if the node has no
    /// intrinsic local bounds (an empty group with no clip, for instance).
    pub fn world_bounds(&self, id: NodeId) -> Option<Bounds> {
        let local = self.local_bounds(id)?;
        let world_t = self.world_transform(id)?;
        local.try_transformed(&world_t)
    }

    /// The node's ORIENTED size — the world-space lengths of its local-bounds
    /// edges (`(width, height)`). Unlike [`Self::world_bounds`] (the AABB, which
    /// inflates as the node rotates), these are the node's true rendered
    /// dimensions and stay CONSTANT under rotation. Used by the inspector W/H
    /// fields so rotating a node doesn't change its reported size. `None` for an
    /// unbounded node.
    pub fn world_obb_size(&self, id: NodeId) -> Option<(f64, f64)> {
        let local = self.local_bounds(id)?;
        let t = self.world_transform(id)?;
        let tl = t.transform_point(glam::DVec2::new(local.min_x, local.min_y));
        let tr = t.transform_point(glam::DVec2::new(local.max_x, local.min_y));
        let bl = t.transform_point(glam::DVec2::new(local.min_x, local.max_y));
        Some(((tr - tl).length(), (bl - tl).length()))
    }

    /// The local AABB of a node in its own coordinate space. For groups this
    /// is the union of (transformed) children's local bounds.
    ///
    /// Memoized in [`Self::local_bounds_cache`]: a clipped group short-circuits
    /// to its clip rect, but an *unclipped* group is the union of its whole
    /// subtree (O(subtree)). Because [`Self::world_bounds`] calls this once per
    /// visited node every frame (the viewport cull test, hit-test fast reject,
    /// alignment, fit-to-content), recomputing each time makes the render walk
    /// O(n·depth). With memoization the first `world_bounds(root)` of a frame
    /// fills the entire subtree and every later call is O(1) — so a full frame is
    /// O(n) even when a drag clears the cache every frame. See the field docs for
    /// the invalidation contract.
    ///
    /// [`Self::local_bounds_cache`]: crate::scene::Scene
    pub fn local_bounds(&self, id: NodeId) -> Option<Bounds> {
        // Fast path: memoized. The cached value is itself an `Option`, so a
        // genuinely unbounded node (an empty unclipped group) caches its `None`
        // and is not re-walked either.
        if let Some(cached) = self.local_bounds_cache.borrow().get(&id) {
            return *cached;
        }
        // Cold: compute, then memoize. `compute_local_bounds` recurses through
        // `self.local_bounds` (not itself), so each descendant is filled and
        // cached exactly once; we only borrow the cache briefly here, never
        // across the recursive call, so re-entrant `RefCell` borrows can't
        // overlap (same discipline as `world_transform`).
        let computed = self.compute_local_bounds(id);
        self.local_bounds_cache.borrow_mut().insert(id, computed);
        computed
    }

    /// Uncached local-bounds computation backing [`Self::local_bounds`]. The
    /// recursive child lookups go back through `local_bounds`, so they hit the
    /// memo; only `id`'s own result is (re)computed here.
    fn compute_local_bounds(&self, id: NodeId) -> Option<Bounds> {
        let node = self.nodes.get(&id)?;
        match &node.data {
            NodeData::Group(g) => {
                if let Some([w, h]) = g.clip_size {
                    return Some(Bounds::from_xywh(0.0, 0.0, w, h));
                }
                // Union of children's bounds, transformed into our local space.
                let mut acc: Option<Bounds> = None;
                for &child_id in self.children_of(Some(id)) {
                    if let Some(child_local) = self.local_bounds(child_id) {
                        let child_node = &self.nodes[&child_id];
                        if let Some(in_self_space) =
                            child_local.try_transformed(&child_node.transform)
                        {
                            acc = Some(match acc {
                                Some(a) => a.union(&in_self_space),
                                None => in_self_space,
                            });
                        }
                    }
                }
                acc
            }
            NodeData::Vector(v) => v.path.rough_bounds(),
            NodeData::Text(t) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                t.local_size[0],
                t.local_size[1],
            )),
            NodeData::Bitmap(b) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                b.local_size[0],
                b.local_size[1],
            )),
            NodeData::Video(v) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                v.local_size[0],
                v.local_size[1],
            )),
            NodeData::Audio(a) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                a.local_size[0],
                a.local_size[1],
            )),
            NodeData::NodeGraph(n) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                n.local_size[0],
                n.local_size[1],
            )),
            NodeData::Model3d(m) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                m.local_size[0],
                m.local_size[1],
            )),
            NodeData::AiArtifact(a) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                a.local_size[0],
                a.local_size[1],
            )),
            NodeData::Instance(i) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                i.local_size[0],
                i.local_size[1],
            )),
            NodeData::Embed(e) => Some(Bounds::from_xywh(
                0.0,
                0.0,
                e.local_size[0],
                e.local_size[1],
            )),
        }
    }

    // ---- spatial index -------------------------------------------------------

    /// Build (or refresh) and run `f` against the lazily-maintained spatial
    /// index. The index is rebuilt on the first spatial query after any edit
    /// (it is dropped by [`Self::invalidate_world_cache`]); afterwards every
    /// query reuses it until the next edit. Off the read hot path, the build is
    /// a single pre-order DFS that snapshots each *visible* node's world AABB
    /// and paint rank — the same `O(n)` work one brute-force scan would do, but
    /// amortized across every subsequent query between edits.
    ///
    /// HIDDEN subtrees are skipped during the build exactly as the recursive
    /// walk skips them, so a hidden node (and its descendants) never appears as
    /// a candidate.
    fn with_spatial_index<R>(&self, f: impl FnOnce(&SpatialIndex) -> R) -> R {
        if self.spatial_index.borrow().is_none() {
            let index = self.build_spatial_index();
            *self.spatial_index.borrow_mut() = Some(index);
        }
        let guard = self.spatial_index.borrow();
        f(guard.as_ref().expect("just built"))
    }

    /// Snapshot the scene into a [`SpatialIndex`]: a pre-order DFS over roots
    /// ascending then children ascending, assigning each visited node an
    /// incrementing paint rank, skipping HIDDEN subtrees. Nodes without world
    /// bounds (e.g. an empty unclipped group) are omitted — they can never be a
    /// hit or marquee candidate anyway.
    fn build_spatial_index(&self) -> SpatialIndex {
        let mut items: Vec<(NodeId, Bounds)> = Vec::with_capacity(self.nodes.len());
        // Iterative pre-order DFS. Push children in reverse so they pop in
        // ascending z-order — matching the brute-force traversal's rank order.
        let mut stack: Vec<NodeId> = self.roots().iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            if node.flags.contains(NodeFlags::HIDDEN) {
                // Skip the node and its whole subtree, exactly like the walk.
                continue;
            }
            if let Some(bounds) = self.world_bounds(id) {
                items.push((id, bounds));
            }
            for &child in self.children_of(Some(id)).iter().rev() {
                stack.push(child);
            }
        }
        SpatialIndex::build(items)
    }

    /// Whether every ancestor of `id`'s world AABB contains `world_point`. The
    /// recursive walk fast-rejects at each level, so a leaf only hits if every
    /// ancestor's AABB contains the point too. Group bounds are the union of
    /// their children, so this is satisfied automatically in practice, but we
    /// check it to guarantee exact parity with the recursive walk.
    fn ancestors_contain_point(&self, id: NodeId, world_point: DVec2) -> bool {
        let mut cursor = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(p) = cursor {
            match self.world_bounds(p) {
                Some(b) if b.contains_point(world_point) => {}
                // An ancestor with no bounds (empty group) would never have been
                // descended into by the walk; treat as a miss.
                _ => return false,
            }
            cursor = self.nodes.get(&p).and_then(|n| n.parent);
        }
        true
    }

    /// Find the topmost node under `world_point`. AABB-only test; the render
    /// layer can refine with path-precise hit-testing for vector nodes when
    /// the cursor is over a stroke or fill.
    ///
    /// Accelerated by the lazily-built [`crate::spatial::SpatialIndex`]: the
    /// grid narrows the work to candidates whose world AABB contains the point,
    /// then the same per-node predicate the recursive walk applies (not a
    /// group, every ancestor's AABB contains the point) resolves the top hit by
    /// paint rank. The result is identical to the old `O(n)` recursive scan —
    /// see the parity tests — but the per-event cost no longer scales with the
    /// whole node count.
    pub fn hit_test(&self, world_point: DVec2) -> Option<NodeId> {
        self.topmost_hit_where(world_point, |_| true)
    }

    /// Topmost node under `world_point` that additionally satisfies `accept`.
    ///
    /// `accept` is the caller's refinement (e.g. path-precise containment in
    /// `fanta-canvas`) layered on top of the structural predicate this method
    /// already enforces (not a group, ancestor-AABB containment, visibility).
    /// Used by `Scene::hit_test` (with a trivial `accept`) and by the canvas
    /// hit-test layer; the spatial index makes both sub-linear per query.
    pub fn topmost_hit_where(
        &self,
        world_point: DVec2,
        mut accept: impl FnMut(NodeId) -> bool,
    ) -> Option<NodeId> {
        self.with_spatial_index(|index| {
            index.topmost_hit(world_point, |id| {
                // Structural predicate identical to `hit_test_subtree`: a plain
                // group never self-catches (click passes through to a child),
                // but a FRAME surface (clip box / background) does, so frames
                // can be picked + dragged by their body. Every ancestor AABB
                // must contain the point. Visibility is already handled (hidden
                // subtrees are not in the index). Then defer to the caller's
                // refinement.
                let Some(node) = self.nodes.get(&id) else {
                    return false;
                };
                if let NodeData::Group(g) = &node.data {
                    if !g.is_frame_surface() {
                        return false;
                    }
                }
                if !self.ancestors_contain_point(id, world_point) {
                    return false;
                }
                accept(id)
            })
        })
    }

    /// Every node under `world_point` satisfying `accept`, sorted top-z first.
    /// The structural predicate (group exclusion, ancestor containment,
    /// visibility) matches the recursive deep walk; `accept` is the caller's
    /// refinement. Accelerated by the spatial index.
    pub fn deep_hits_where(
        &self,
        world_point: DVec2,
        mut accept: impl FnMut(NodeId) -> bool,
    ) -> Vec<NodeId> {
        self.with_spatial_index(|index| {
            index.deep_hits(world_point, |id| {
                let Some(node) = self.nodes.get(&id) else {
                    return false;
                };
                // Plain groups stay click-through; frame surfaces self-catch
                // (see `topmost_hit_where`).
                if let NodeData::Group(g) = &node.data {
                    if !g.is_frame_surface() {
                        return false;
                    }
                }
                if !self.ancestors_contain_point(id, world_point) {
                    return false;
                }
                accept(id)
            })
        })
    }

    /// Every non-group, visible node whose world AABB satisfies `accept`
    /// relative to `world_rect`, sorted bottom-z first (pre-order). `accept`
    /// receives the node id and its world AABB and decides the inclusion test
    /// (contains vs intersects) plus any extra filtering (e.g. LOCKED). The
    /// spatial index restricts candidates to AABBs overlapping `world_rect`, so
    /// a marquee no longer scans the whole tree.
    ///
    /// Group nodes are never reported (a marquee selects leaves, not the
    /// containing group — Figma convention), matching the brute-force walk.
    pub fn rect_query_where(
        &self,
        world_rect: Bounds,
        mut accept: impl FnMut(NodeId, &Bounds) -> bool,
    ) -> Vec<NodeId> {
        self.with_spatial_index(|index| {
            index.rect_query(world_rect, |id, bounds| {
                let Some(node) = self.nodes.get(&id) else {
                    return false;
                };
                if matches!(node.data, NodeData::Group(_)) {
                    return false;
                }
                accept(id, bounds)
            })
        })
    }

    /// Brute-force reference implementation of [`Self::hit_test`]: the old
    /// `O(n)` recursive top-first walk. Retained as the parity oracle the
    /// indexed path is tested against (and as a fallback that needs no index).
    #[doc(hidden)]
    pub fn hit_test_brute(&self, world_point: DVec2) -> Option<NodeId> {
        for &root in self.roots().iter().rev() {
            if let Some(hit) = self.hit_test_subtree(root, world_point) {
                return Some(hit);
            }
        }
        None
    }

    fn hit_test_subtree(&self, id: NodeId, world_point: DVec2) -> Option<NodeId> {
        let node = self.nodes.get(&id)?;
        if node.flags.contains(crate::node::NodeFlags::HIDDEN) {
            return None;
        }
        let bounds = self.world_bounds(id)?;
        if !bounds.contains_point(world_point) {
            return None;
        }
        // Children top-first; first hit wins. If no child hits but the point
        // is within our bounds and we are a visible content node, we hit
        // ourselves.
        let children = self.children_of(Some(id));
        for &child in children.iter().rev() {
            if let Some(hit) = self.hit_test_subtree(child, world_point) {
                return Some(hit);
            }
        }
        if let NodeData::Group(g) = &node.data {
            // A plain group doesn't "catch" hits — clicking empty space in it
            // passes through to siblings below. A FRAME surface (clip box /
            // background) does catch, so it can be selected + dragged by its
            // body. Kept in parity with `topmost_hit_where`/`deep_hits_where`.
            if g.is_frame_surface() { Some(id) } else { None }
        } else {
            Some(id)
        }
    }
}

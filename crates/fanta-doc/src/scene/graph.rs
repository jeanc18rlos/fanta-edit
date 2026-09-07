//! The [`Scene`] graph storage: node map + maintained child index, plus the
//! structural operations (get/insert/remove, hierarchy edits, traversal,
//! child-index maintenance, and the [`Scene::validate`] diagnostic) and the
//! [`Descendants`] / [`Ancestors`] iterators.
//!
//! World geometry, the spatial index, and hit-testing live in
//! [`crate::scene::geometry`] (a second `impl Scene` block over the same
//! `pub(crate)` fields).

use crate::id::{IdHashMap, NodeId};
use crate::index::IndexKey;
use crate::node::CanvasNode;
use crate::scene::error::SceneError;
use crate::spatial::SpatialIndex;
use crate::transform::{Bounds, Transform2D};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::HashMap;

// =============================================================================
// Scene
// =============================================================================

/// The scene graph. Owns nodes; offers traversal, ordering, and queries.
///
/// `Scene` is `Clone` for cheap snapshotting in `fanta-doc::history`. The clone
/// is O(n) in the node count — fine for design docs (single-digit ms at 10k
/// nodes), and the history layer keeps deltas, not snapshots, on the hot path.
/// `Clone` is implemented by hand (not derived) so a clone mints a fresh
/// [`Scene::instance_id`] — see that field's docs for why two scene instances
/// must never share one.
///
/// The fields are `pub(crate)` so the geometry / hit-test methods can live in a
/// sibling submodule ([`crate::scene::geometry`]) without widening the public
/// API — they were never part of it. External callers go through the methods.
#[derive(Debug, Serialize, Deserialize)]
pub struct Scene {
    pub(crate) nodes: IdHashMap<NodeId, CanvasNode>,
    /// Children of each parent, sorted by `IndexKey`. `None` key holds root
    /// children. Maintained on every insert / remove / reparent / reorder.
    #[serde(skip)]
    pub(crate) child_index: IdHashMap<Option<NodeId>, Vec<NodeId>>,
    /// Memoized world transforms, keyed by node id.
    ///
    /// ## Why interior mutability
    ///
    /// [`Scene::world_transform`] takes `&self` (many callers in `fanta-canvas`,
    /// `fanta-app`, and `fanta-export` only hold a shared borrow). To memoize
    /// without changing that signature we cache behind a [`RefCell`]. The doc is
    /// single-threaded — `Scene` is neither `Send` nor `Sync`-bound across the
    /// hot path — so a `RefCell` (not a `Mutex`) is the right primitive.
    ///
    /// ## Invalidation contract
    ///
    /// A cached world transform for `id` is the product of `id`'s local
    /// transform and every ancestor's local transform. It goes stale if any of
    /// those transforms change, or if `id`'s ancestor chain changes (reparent /
    /// reorder that moves `id` under a different subtree). Rather than track
    /// fine-grained dependencies, every method that *could* invalidate an entry
    /// clears the **whole** cache via [`Scene::invalidate_world_cache`]:
    ///
    /// - [`Scene::insert`], [`Scene::remove`] — add/drop nodes and subtrees.
    /// - [`Scene::set_parent`], [`Scene::set_index`] — change ancestor chains.
    /// - [`Scene::rebuild_child_index`] — wholesale index rebuild after load.
    /// - [`Scene::get_mut`] — opaque `&mut CanvasNode`; the caller may write
    ///   `transform`, so we must assume any transform changed.
    ///
    /// Clearing is O(n) in the cache size but happens only on mutation, not on
    /// the per-node read hot path. A cleared cache is always correct: the next
    /// [`Scene::world_transform`] recomputes lazily. Because clone /
    /// deserialize start with an empty cache (`#[serde(skip)]`), there is no way
    /// to resurrect a stale entry across those boundaries either.
    #[serde(skip)]
    pub(crate) world_cache: RefCell<IdHashMap<NodeId, Transform2D>>,
    /// Memoized *local* bounds, keyed by node id.
    ///
    /// A node's local bounds are independent of its place in the hierarchy
    /// (they're in the node's own coordinate space), but for an unclipped group
    /// they are the union of every descendant's transformed local bounds — an
    /// O(subtree) walk. [`Scene::world_bounds`] (the viewport cull test in
    /// `fanta-render`, the hit-test fast reject, alignment, fit-to-content) calls
    /// it once per visited node *every frame*, so without memoization the render
    /// cull walk is O(n·depth): the whole point of culling — cheap rejection — is
    /// swamped by the cost of computing the bounds it rejects on.
    ///
    /// Memoizing collapses that to O(n) per frame even under an
    /// invalidate-every-frame drag: the first `world_bounds(root)` fills the
    /// entire subtree, and every later cull test that frame is a cache hit.
    ///
    /// ## Invalidation contract
    ///
    /// Shares [`Scene::invalidate_world_cache`] with `world_cache`: any geometry
    /// or structure change (transform write via `get_mut`, insert/remove,
    /// reparent/reorder, index rebuild) clears it wholesale. A group's bounds
    /// depend on its descendants' local transforms and geometry, so a descendant
    /// edit must invalidate every ancestor's cached union — wholesale clearing
    /// covers that conservatively. `#[serde(skip)]` so clone/deserialize start
    /// empty and can never resurrect a stale entry.
    #[serde(skip)]
    pub(crate) local_bounds_cache: RefCell<IdHashMap<NodeId, Option<Bounds>>>,
    /// Lazily-built spatial acceleration structure over world AABBs, used to
    /// prune hit-test and marquee candidates instead of walking all `n` nodes.
    ///
    /// ## Why interior mutability
    ///
    /// Built on the first spatial query after an edit, from a `&self`. Like the
    /// other caches it lives behind a [`RefCell`]; the doc is single-threaded.
    ///
    /// ## Invalidation contract
    ///
    /// The index is a snapshot of every visible node's world AABB plus the
    /// global paint order. It goes stale on exactly the inputs that stale the
    /// world-transform / local-bounds caches: a transform write, a geometry
    /// edit, a structure change (insert / remove / reparent / reorder), or a
    /// visibility (HIDDEN) toggle — all of which already route through
    /// [`Scene::invalidate_world_cache`]. So it shares that single signal and is
    /// cleared there wholesale. `#[serde(skip)]` so clone / deserialize start
    /// empty and never resurrect a stale index. See [`crate::spatial`].
    #[serde(skip)]
    pub(crate) spatial_index: RefCell<Option<SpatialIndex>>,
    /// Monotonic content revision: bumped by [`Scene::invalidate_world_cache`]
    /// — the single funnel every structural/geometry mutation already goes
    /// through. Equal revisions GUARANTEE the render-relevant scene state is
    /// unchanged (the converse doesn't hold: a bump may be conservative).
    /// The memoization primitive for retained-surface blits and per-frame
    /// chrome scans (perf-findings-2026-06.md item 1). `Cell` because reads
    /// take `&self` on the single-threaded doc; skipped by serde like every
    /// other derived cache.
    #[serde(skip)]
    pub(crate) revision: std::cell::Cell<u64>,
    /// Per-node "data may have changed" stamps for renderer geometry caches.
    ///
    /// A node's stamp records the value [`Scene::revision`] had right after
    /// the mutation that touched it, so stamps drawn from the one strictly
    /// increasing counter are totally ordered: `max` over any subtree strictly
    /// increases whenever anything inside it is stamped. A renderer cache
    /// entry tagged with the stamp it was built at is valid exactly while the
    /// node's stamp is unchanged — moving node A no longer invalidates node
    /// B's cached geometry (the global `revision` still bumps for the
    /// session watermark; only cache granularity changed).
    #[serde(skip)]
    pub(crate) node_stamps: IdHashMap<NodeId, u64>,
    /// Wholesale stamp floor: [`Scene::node_stamp`] never reports below this.
    /// Raised by bulk operations (index rebuild, bulk detach) instead of
    /// stamping every node individually.
    #[serde(skip)]
    pub(crate) stamp_floor: std::cell::Cell<u64>,
    /// Bumped whenever nodes are removed; renderer caches purge dead ids only
    /// when this moved instead of scanning every frame.
    #[serde(skip)]
    pub(crate) removal_revision: std::cell::Cell<u64>,
    /// Process-unique identity of this in-memory `Scene` INSTANCE.
    ///
    /// [`Scene::revision`] is a per-instance counter: a freshly parsed or
    /// cloned scene restarts (or copies) it, so `(NodeId, revision)` pairs are
    /// NOT unique across scene instances — a reloaded document whose node ids
    /// survive the reparse can land on exactly the revision of the scene it
    /// replaced while carrying different content. Cross-scene memo consumers
    /// (the renderer's revision-keyed path/boolean caches) therefore also need
    /// to know *which scene instance* a revision belongs to; this id is that
    /// signal. Minted from a process-global counter on every construction
    /// path — [`Scene::new`] / [`Default`], `Clone` (two diverged clones must
    /// never alias a `(id, revision)` pair), and deserialize (re-minted via
    /// the serde `default`) — so equal ids GUARANTEE the same live instance.
    #[serde(skip, default = "mint_scene_instance_id")]
    pub(crate) instance_id: u64,
}

/// Next process-unique geometry stamp — see [`Scene::node_stamp`]. Starts at
/// 1 so 0 always means "never stamped". Relaxed suffices: only atomicity of
/// `fetch_add` matters.
fn next_geometry_stamp() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Next process-unique [`Scene::instance_id`]. Starts at 1 so 0 never denotes
/// a real scene (embedders can use it as an "unset" sentinel). Relaxed is
/// enough: uniqueness needs only the atomicity of `fetch_add`, no ordering.
fn mint_scene_instance_id() -> u64 {
    static NEXT_SCENE_INSTANCE_ID: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);
    NEXT_SCENE_INSTANCE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            nodes: IdHashMap::default(),
            child_index: IdHashMap::default(),
            node_stamps: IdHashMap::default(),
            stamp_floor: std::cell::Cell::new(0),
            removal_revision: std::cell::Cell::new(0),
            world_cache: RefCell::new(IdHashMap::default()),
            local_bounds_cache: RefCell::new(IdHashMap::default()),
            spatial_index: RefCell::new(None),
            revision: std::cell::Cell::new(0),
            instance_id: mint_scene_instance_id(),
        }
    }
}

impl Clone for Scene {
    /// Field-wise clone EXCEPT `instance_id`, which is minted fresh: the clone
    /// is a distinct instance whose revision counter diverges independently
    /// from the original's, so sharing the id would let the two alias
    /// `(NodeId, revision)` memo keys with different content.
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            child_index: self.child_index.clone(),
            node_stamps: self.node_stamps.clone(),
            stamp_floor: self.stamp_floor.clone(),
            removal_revision: self.removal_revision.clone(),
            world_cache: self.world_cache.clone(),
            local_bounds_cache: self.local_bounds_cache.clone(),
            spatial_index: self.spatial_index.clone(),
            revision: self.revision.clone(),
            instance_id: mint_scene_instance_id(),
        }
    }
}

impl Scene {
    /// Empty scene.
    pub fn new() -> Self {
        Self::default()
    }

    /// The process-unique id of this `Scene` instance — see the field docs.
    /// Two equal ids guarantee the same live instance, so a memo keyed by
    /// `(instance_id, revision, NodeId)` can never serve one scene's content
    /// for another's.
    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    /// Total node count, including all descendants.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    // ---- get -----------------------------------------------------------------

    pub fn get(&self, id: NodeId) -> Option<&CanvasNode> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut CanvasNode> {
        // Note: callers must not change `id`, `parent`, or `index` through the
        // mutable reference — those go through dedicated methods so the child
        // index stays consistent. Enforced socially for now; a `NodeMut`
        // newtype with restricted access would harden this in a later pass.
        //
        // The returned reference is opaque: the caller may write `transform`
        // (the common case — dragging mutates the local transform here). We
        // cannot know whether they will, so we conservatively clear the world
        // transform cache. A node's cached world transform is the product of
        // its and its ancestors' locals, so a mutated `transform` here would
        // also invalidate every descendant — clearing wholesale covers that.
        self.invalidate_world_cache();
        if self.nodes.contains_key(&id) {
            self.touch_stamp(id);
        }
        self.nodes.get_mut(&id)
    }

    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(&id)
    }

    // ---- insert / remove -----------------------------------------------------

    /// Insert a node. The node's `parent` and `index` are honored as-is and
    /// must be valid (parent exists and accepts children).
    pub fn insert(&mut self, node: CanvasNode) -> Result<NodeId, SceneError> {
        if self.nodes.contains_key(&node.id) {
            return Err(SceneError::Duplicate(node.id));
        }
        if let Some(parent) = node.parent {
            let parent_node = self
                .nodes
                .get(&parent)
                .ok_or(SceneError::ParentMissing(parent))?;
            if !parent_node.can_have_children() {
                return Err(SceneError::ParentNotContainer(parent));
            }
        }
        let id = node.id;
        let parent_key = node.parent;
        let index = node.index;
        self.nodes.insert(id, node);
        self.child_index_insert(parent_key, id, index);
        self.invalidate_world_cache();
        self.touch_stamp(id);
        if let Some(parent) = parent_key {
            self.touch_stamp(parent);
        }
        Ok(id)
    }

    /// Remove a node and all of its descendants. Returns the removed root.
    pub fn remove(&mut self, id: NodeId) -> Result<CanvasNode, SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::NotFound(id));
        }
        // `descendants_of` yields `id` itself first, then its subtree. We need
        // to return the root after the loop, so we stash it as we encounter it.
        let all_ids: Vec<NodeId> = self.descendants_of(id).collect();
        let mut root_removed: Option<CanvasNode> = None;
        for d in all_ids {
            if let Some(removed) = self.nodes.remove(&d) {
                self.child_index_remove(removed.parent, d);
                self.node_stamps.remove(&d);
                if d == id {
                    root_removed = Some(removed);
                }
            }
        }
        self.invalidate_world_cache();
        self.removal_revision.set(next_geometry_stamp());
        if let Some(parent) = root_removed.as_ref().and_then(|node| node.parent)
            && self.nodes.contains_key(&parent)
        {
            self.touch_stamp(parent);
        }
        root_removed.ok_or(SceneError::NotFound(id))
    }

    // ---- hierarchy edits -----------------------------------------------------

    /// Move a node to a new parent (or root if `None`). Refuses cycles.
    pub fn set_parent(
        &mut self,
        id: NodeId,
        new_parent: Option<NodeId>,
        new_index: IndexKey,
    ) -> Result<(), SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::NotFound(id));
        }
        if let Some(p) = new_parent {
            let pn = self.nodes.get(&p).ok_or(SceneError::ParentMissing(p))?;
            if !pn.can_have_children() {
                return Err(SceneError::ParentNotContainer(p));
            }
            // Cycle check: walk up from `p`; if we hit `id`, refuse.
            let mut cursor = Some(p);
            while let Some(c) = cursor {
                if c == id {
                    return Err(SceneError::Cycle {
                        descendant: id,
                        ancestor: p,
                    });
                }
                cursor = self.nodes.get(&c).and_then(|n| n.parent);
            }
        }

        // Remove under the OLD (parent, index) key first — the bucket is
        // sorted by the current index fields, so removal can binary-search
        // only while the node still carries the index the bucket was sorted
        // with.
        let old_parent = self.nodes.get(&id).expect("just checked").parent;
        self.child_index_remove(old_parent, id);
        let node = self.nodes.get_mut(&id).expect("just checked");
        node.parent = new_parent;
        node.index = new_index;
        self.child_index_insert(new_parent, id, new_index);
        // Reparenting changes `id`'s (and its subtree's) ancestor chain, so
        // every world transform under it could change.
        self.invalidate_world_cache();
        self.touch_stamp(id);
        for parent in [old_parent, new_parent].into_iter().flatten() {
            if self.nodes.contains_key(&parent) {
                self.touch_stamp(parent);
            }
        }
        Ok(())
    }

    /// Update the z-order index of an existing node.
    pub fn set_index(&mut self, id: NodeId, new_index: IndexKey) -> Result<(), SceneError> {
        let (parent, _) = {
            let node = self.nodes.get(&id).ok_or(SceneError::NotFound(id))?;
            (node.parent, node.index)
        };
        self.child_index_remove(parent, id);
        let node = self.nodes.get_mut(&id).expect("just checked");
        node.index = new_index;
        self.child_index_insert(parent, id, new_index);
        // Z-order does not affect world transforms, but `set_index` shares the
        // mutator contract; clearing keeps the invalidation surface uniform and
        // future-proof (e.g. if index ever feeds into layout).
        self.invalidate_world_cache();
        self.touch_stamp(id);
        if let Some(parent) = parent
            && self.nodes.contains_key(&parent)
        {
            self.touch_stamp(parent);
        }
        Ok(())
    }

    // ---- traversal -----------------------------------------------------------

    /// Children of `parent` (or root if `None`), sorted by z-index ascending
    /// (bottom-first). Empty if no children.
    pub fn children_of(&self, parent: Option<NodeId>) -> &[NodeId] {
        self.child_index
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Root children, sorted ascending.
    pub fn roots(&self) -> &[NodeId] {
        self.children_of(None)
    }

    /// The [`IndexKey`] a new child of `parent` should take to sort *above*
    /// every existing sibling (top of the z-order). Returns [`IndexKey::FIRST`]
    /// when there are no siblings yet.
    ///
    /// Creators should call this instead of leaving a node at the default
    /// [`IndexKey::FIRST`]: if every new node kept `FIRST`, sibling z-order
    /// would be ambiguous (equal keys sort by insertion happenstance) and
    /// "the newest node is on top / last in `children_of`" would not hold.
    pub fn next_child_index(&self, parent: Option<NodeId>) -> IndexKey {
        match self.children_of(parent).last() {
            Some(last) => IndexKey::after(self.nodes[last].index),
            None => IndexKey::FIRST,
        }
    }

    /// Convenience: [`next_child_index`] for a root-level insertion.
    ///
    /// [`next_child_index`]: Self::next_child_index
    pub fn next_root_index(&self) -> IndexKey {
        self.next_child_index(None)
    }

    /// Iterator over `id` and all of its descendants in depth-first order.
    /// The first yielded id is `id` itself; then the subtree below.
    pub fn descendants_of(&self, id: NodeId) -> Descendants<'_> {
        Descendants {
            scene: self,
            stack: vec![id],
        }
    }

    /// Walk up from `id` through ancestors, ending with the root child whose
    /// `parent` is `None`. Does **not** yield `id` itself.
    pub fn ancestors_of(&self, id: NodeId) -> Ancestors<'_> {
        let start = self.nodes.get(&id).and_then(|n| n.parent);
        Ancestors {
            scene: self,
            cursor: start,
        }
    }

    // ---- diagnostics ---------------------------------------------------------

    /// Re-check every invariant against the storage. Returns the first
    /// violation found, otherwise `Ok(())`. Linear in the node count.
    pub fn validate(&self) -> Result<(), SceneError> {
        // 1. Every parent ref exists and accepts children.
        for n in self.nodes.values() {
            if let Some(p) = n.parent {
                let parent = self.nodes.get(&p).ok_or_else(|| {
                    SceneError::InvariantViolated(format!("dangling parent ref: {p}"))
                })?;
                if !parent.can_have_children() {
                    return Err(SceneError::InvariantViolated(format!(
                        "non-container parent {p} for child {}",
                        n.id
                    )));
                }
            }
        }
        // 2. Acyclic — walk up from each node, bounded by node count.
        let max_depth = self.nodes.len() + 1;
        for n in self.nodes.values() {
            let mut depth = 0;
            let mut cursor = n.parent;
            while let Some(p) = cursor {
                if depth > max_depth {
                    return Err(SceneError::InvariantViolated(format!(
                        "cycle detected reaching {p} from {}",
                        n.id
                    )));
                }
                if p == n.id {
                    return Err(SceneError::InvariantViolated(format!("self-cycle on {p}")));
                }
                cursor = self.nodes.get(&p).and_then(|x| x.parent);
                depth += 1;
            }
        }
        // 3. Child index agrees with parent fields.
        let mut expected: HashMap<Option<NodeId>, Vec<NodeId>> = HashMap::new();
        for n in self.nodes.values() {
            expected.entry(n.parent).or_default().push(n.id);
        }
        for v in expected.values_mut() {
            // Total order: equal IndexKeys tie-break by the node's stable ULID,
            // so a post-deserialize rebuild matches the live incremental insert
            // order exactly (otherwise equal-index siblings get a process-random
            // HashMap order → non-deterministic replay/snapshot diffs).
            v.sort_by(|a, b| {
                self.nodes[a]
                    .index
                    .cmp(&self.nodes[b].index)
                    .then_with(|| a.cmp(b))
            });
        }
        if expected.len() != self.child_index.len() {
            return Err(SceneError::InvariantViolated(
                "child index parent-set differs from storage".into(),
            ));
        }
        for (parent, ids) in &expected {
            let stored = self.child_index.get(parent).ok_or_else(|| {
                SceneError::InvariantViolated(format!("missing child bucket for {parent:?}"))
            })?;
            if stored != ids {
                return Err(SceneError::InvariantViolated(format!(
                    "child index mismatch under {parent:?}"
                )));
            }
        }
        Ok(())
    }

    // ---- child index maintenance -------------------------------------------

    fn child_index_insert(&mut self, parent: Option<NodeId>, id: NodeId, index: IndexKey) {
        let bucket = self.child_index.entry(parent).or_default();
        // Sorted insert on the total order (index, NodeId) so equal-index
        // siblings land in stable-ULID order — identical to `rebuild_child_index`
        // after a deserialize. Without the NodeId tiebreak the live and rebuilt
        // orders diverge for equal keys and replay becomes non-deterministic.
        let pos = bucket
            .binary_search_by(|existing_id| {
                (self.nodes[existing_id].index, *existing_id).cmp(&(index, id))
            })
            .unwrap_or_else(|e| e);
        bucket.insert(pos, id);
    }

    fn child_index_remove(&mut self, parent: Option<NodeId>, id: NodeId) {
        let Some(target) = self.nodes.get(&id).map(|node| (node.index, id)) else {
            // Node already gone from the map: fall back to a scan.
            if let Some(bucket) = self.child_index.get_mut(&parent) {
                if let Some(pos) = bucket.iter().position(|&x| x == id) {
                    bucket.remove(pos);
                }
                if bucket.is_empty() {
                    self.child_index.remove(&parent);
                }
            }
            return;
        };
        if let Some(bucket) = self.child_index.get_mut(&parent) {
            // The bucket is sorted by (index, id); as long as the node still
            // carries the index it was inserted with, removal is O(log n).
            // A caller that changed the field through `get_mut` desyncs the
            // sort key, so a miss degrades to the old linear scan.
            let sorted_pos = bucket
                .binary_search_by(|existing| (self.nodes[existing].index, *existing).cmp(&target))
                .ok();
            let pos = sorted_pos.or_else(|| bucket.iter().position(|&x| x == id));
            if let Some(pos) = pos {
                bucket.remove(pos);
            }
            if bucket.is_empty() {
                self.child_index.remove(&parent);
            }
        }
    }

    /// Bulk-detach nodes from the child index in one pass per bucket — the
    /// importer-grade primitive for "insert everything, then reparent
    /// everything". Detached nodes keep existing in the node map with
    /// `parent = None` but are NOT listed under the root bucket until they
    /// are re-attached via [`set_parent`]/[`set_index`]; between the two
    /// steps `children_of(None)` intentionally omits them. Callers must
    /// re-attach (or [`remove`]) every detached node before handing the
    /// scene to anything else.
    ///
    /// [`set_parent`]: Self::set_parent
    /// [`set_index`]: Self::set_index
    /// [`remove`]: Self::remove
    pub fn detach_many(&mut self, ids: &std::collections::HashSet<NodeId>) {
        if ids.is_empty() {
            return;
        }
        for bucket in self.child_index.values_mut() {
            bucket.retain(|id| !ids.contains(id));
        }
        self.child_index.retain(|_, bucket| !bucket.is_empty());
        for id in ids {
            if let Some(node) = self.nodes.get_mut(id) {
                node.parent = None;
                node.index = IndexKey::FIRST;
            }
        }
        self.invalidate_world_cache();
        self.stamp_floor.set(next_geometry_stamp());
    }

    /// Rebuild the child index from scratch. Used after deserialization
    /// (the index is `#[serde(skip)]`) and as a recovery tool.
    pub fn rebuild_child_index(&mut self) {
        self.child_index.clear();
        let mut buckets: IdHashMap<Option<NodeId>, Vec<NodeId>> = IdHashMap::default();
        for n in self.nodes.values() {
            buckets.entry(n.parent).or_default().push(n.id);
        }
        for v in buckets.values_mut() {
            // Total order: equal IndexKeys tie-break by the node's stable ULID,
            // so a post-deserialize rebuild matches the live incremental insert
            // order exactly (otherwise equal-index siblings get a process-random
            // HashMap order → non-deterministic replay/snapshot diffs).
            v.sort_by(|a, b| {
                self.nodes[a]
                    .index
                    .cmp(&self.nodes[b].index)
                    .then_with(|| a.cmp(b))
            });
        }
        self.child_index = buckets;
        // The index rebuild follows a bulk mutation (typically a deserialize);
        // the world cache is already empty after `#[serde(skip)]`, but clear
        // defensively so this method is safe to call at any point.
        self.invalidate_world_cache();
        self.stamp_floor.set(next_geometry_stamp());
    }

    // ---- geometry stamps -----------------------------------------------------

    /// Record that `id`'s data may have changed. Stamp values come from a
    /// process-global monotonic counter, NOT the per-scene revision: clones
    /// share their ancestor's stamps, and any divergent mutation on either
    /// side mints a value no other scene can ever produce — so a renderer
    /// cache entry tagged with a stamp is valid for exactly the content that
    /// produced it, across projection rebuilds and scratch clones alike.
    fn touch_stamp(&mut self, id: NodeId) {
        self.node_stamps.insert(id, next_geometry_stamp());
    }

    /// The stamp renderer caches key one node's derived geometry on: unchanged
    /// stamp GUARANTEES the node's own data (and, for containers, its direct
    /// membership) is unchanged. Contrast [`Scene::revision`], which any
    /// mutation anywhere bumps.
    pub fn node_stamp(&self, id: NodeId) -> u64 {
        self.node_stamps
            .get(&id)
            .copied()
            .unwrap_or(0)
            .max(self.stamp_floor.get())
    }

    /// Max stamp over `id`'s subtree (including `id`). Because stamps come
    /// from one strictly increasing counter and structural changes stamp the
    /// parents they touch, this strictly increases whenever anything inside
    /// the subtree changes — the sound cache key for derived geometry that
    /// bakes descendants (boolean folds).
    pub fn subtree_stamp(&self, id: NodeId) -> u64 {
        self.descendants_of(id)
            .map(|node| self.node_stamp(node))
            .fold(self.stamp_floor.get(), u64::max)
    }

    /// Moved whenever nodes were removed — cache purges gate on this instead
    /// of scanning per frame.
    pub fn removal_revision(&self) -> u64 {
        self.removal_revision.get()
    }

    /// Transform-only write with cache-precision: bumps the revision and
    /// clears world caches like every mutator, but skips the geometry stamp
    /// unless a boolean ancestor bakes this node's transform into a cached
    /// fold. Local vector geometry does not depend on the transform, so a
    /// plain drag through this method leaves every renderer path cache entry
    /// valid.
    pub fn set_transform(&mut self, id: NodeId, transform: Transform2D) -> Result<(), SceneError> {
        if !self.nodes.contains_key(&id) {
            return Err(SceneError::NotFound(id));
        }
        let in_boolean = self
            .ancestors_of(id)
            .any(|ancestor| matches!(ancestor.data, crate::node::NodeData::Boolean(_)));
        let node = self.nodes.get_mut(&id).expect("just checked");
        node.transform = transform;
        self.invalidate_world_cache();
        if in_boolean {
            self.touch_stamp(id);
            // The fold lives on the boolean ancestor; stamping the moved
            // operand is enough (subtree max covers it), but stamp the direct
            // ancestors too so a parent-keyed consumer can't miss it.
            let ancestors: Vec<NodeId> = self
                .ancestors_of(id)
                .filter(|ancestor| matches!(ancestor.data, crate::node::NodeData::Boolean(_)))
                .map(|ancestor| ancestor.id)
                .collect();
            for ancestor in ancestors {
                self.touch_stamp(ancestor);
            }
        }
        Ok(())
    }
}

// =============================================================================
// Iterators
// =============================================================================

/// Depth-first iterator over a subtree, yielding the root first.
pub struct Descendants<'a> {
    scene: &'a Scene,
    stack: Vec<NodeId>,
}

impl Iterator for Descendants<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let id = self.stack.pop()?;
        // Push children in reverse so iteration emits them in ascending order.
        for &c in self.scene.children_of(Some(id)).iter().rev() {
            self.stack.push(c);
        }
        Some(id)
    }
}

/// Iterator walking up from a starting parent toward the root.
pub struct Ancestors<'a> {
    scene: &'a Scene,
    cursor: Option<NodeId>,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = &'a CanvasNode;
    fn next(&mut self) -> Option<&'a CanvasNode> {
        let id = self.cursor?;
        let node = self.scene.nodes.get(&id)?;
        self.cursor = node.parent;
        Some(node)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

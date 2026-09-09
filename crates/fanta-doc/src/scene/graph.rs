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
use std::collections::{HashMap, VecDeque};

// =============================================================================
// Change log
// =============================================================================

/// One recorded mutation, paired in [`Scene::change_log`] with the revision
/// the scene held right after it. See [`Scene::changes_since`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SceneChange {
    /// Only the node's local transform changed ([`Scene::set_transform`]).
    Transform(NodeId),
    /// The node's own data may have changed in any way except its parent and
    /// z-index ([`Scene::get_mut`], [`Scene::patch_node`]).
    Node(NodeId),
    /// Nodes were inserted, removed, reparented, reordered, or the child index
    /// was rebuilt — a copy cannot be brought up to date node by node.
    Structural,
    /// An edit through a `&mut Scene` the graph did not see, reported after the
    /// fact via [`Scene::invalidate_world_cache`].
    Unknown,
}

/// The deduplicated set of nodes a copy of the scene has to refresh to match
/// the original — the result of [`Scene::changes_since`]. Both lists are sorted
/// and disjoint: a node whose data may have changed is listed in `nodes` only,
/// since re-copying the node carries its transform along.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneDelta {
    /// Nodes whose only change since the queried revision is their local
    /// transform.
    pub transforms: Vec<NodeId>,
    /// Nodes whose data may have changed since the queried revision.
    pub nodes: Vec<NodeId>,
}

impl SceneDelta {
    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty() && self.nodes.is_empty()
    }
}

/// How many mutations [`Scene::change_log`] remembers. A consumer further
/// behind than this cannot be served a delta and re-copies the scene. Sized so
/// a drag of a few hundred nodes on a page whose render spans several input
/// frames still fits between two renders (entries are 24 bytes; the deque only
/// grows as far as it is used).
pub const SCENE_CHANGE_LOG_CAP: usize = 16_384;

/// The most nodes a [`SceneDelta`] may name. Beyond this a node-by-node patch
/// is no longer cheaper than a fresh copy, so [`Scene::changes_since`] gives
/// up instead.
pub const SCENE_DELTA_MAX_NODES: usize = 2048;

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
    /// reorder that moves `id` under a different subtree). Every method that
    /// *could* invalidate an entry clears the **whole** cache
    /// ([`Scene::clear_derived_caches`]):
    ///
    /// - [`Scene::insert`], [`Scene::insert_many`], [`Scene::remove`] — add/drop
    ///   nodes and subtrees.
    /// - [`Scene::set_parent`], [`Scene::set_index`] — change ancestor chains.
    /// - [`Scene::rebuild_child_index`] — wholesale index rebuild after load.
    /// - [`Scene::get_mut`] — opaque `&mut CanvasNode`; the caller may write
    ///   `transform`, so we must assume any transform changed.
    ///
    /// except the two edits whose reach is known exactly, which drop only the
    /// entries they can stale: [`Scene::set_transform`] (the node and its
    /// descendants) and [`Scene::patch_node`] (the same, with the hierarchy
    /// verified unchanged). A drag on a 30k-node page therefore recomputes the
    /// moved subtree, not the document.
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
    /// Shares [`Scene::clear_derived_caches`] with `world_cache`: any geometry
    /// or structure change (transform write via `get_mut`, insert/remove,
    /// reparent/reorder, index rebuild) clears it wholesale. A group's bounds
    /// depend on its descendants' local transforms and geometry, so a descendant
    /// edit must invalidate every ancestor's cached union — wholesale clearing
    /// covers that conservatively, and [`Scene::set_transform`] /
    /// [`Scene::patch_node`] drop exactly the edited node's ancestor chain (plus
    /// the node itself for a data edit). `#[serde(skip)]` so clone/deserialize
    /// start empty and can never resurrect a stale entry.
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
    /// Monotonic content revision: bumped by [`Scene::record_change`] — the
    /// single funnel every structural/geometry mutation goes through, which
    /// also logs what changed. Equal revisions GUARANTEE the render-relevant scene state is
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
    /// The last [`SCENE_CHANGE_LOG_CAP`] mutations, oldest first, each paired
    /// with the [`Scene::revision`] the scene held right after it. Every
    /// revision bump goes through [`Scene::record_change`] and appends exactly
    /// one entry, so consecutive entries carry consecutive revisions — which is
    /// what lets [`Scene::changes_since`] prove it saw every mutation between a
    /// copy's revision and now. `RefCell` for the same reason as the caches:
    /// [`Scene::invalidate_world_cache`] takes `&self`. Skipped by serde and
    /// started empty by `Clone`: a copy's history is not the original's.
    #[serde(skip)]
    pub(crate) change_log: RefCell<VecDeque<(u64, SceneChange)>>,
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
            change_log: RefCell::new(VecDeque::new()),
        }
    }
}

impl Clone for Scene {
    /// Field-wise clone EXCEPT `instance_id`, which is minted fresh: the clone
    /// is a distinct instance whose revision counter diverges independently
    /// from the original's, so sharing the id would let the two alias
    /// `(NodeId, revision)` memo keys with different content.
    ///
    /// The spatial index is not copied either: it is a derived structure the
    /// clone rebuilds lazily on its first hit-test, and copying it made every
    /// render-thread snapshot pay for an index the render never queries.
    fn clone(&self) -> Self {
        Self {
            nodes: self.nodes.clone(),
            child_index: self.child_index.clone(),
            node_stamps: self.node_stamps.clone(),
            stamp_floor: self.stamp_floor.clone(),
            removal_revision: self.removal_revision.clone(),
            world_cache: self.world_cache.clone(),
            local_bounds_cache: self.local_bounds_cache.clone(),
            spatial_index: RefCell::new(None),
            revision: self.revision.clone(),
            instance_id: mint_scene_instance_id(),
            change_log: RefCell::new(VecDeque::new()),
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
        //
        // A missing id hands out nothing, so nothing can change: bail before
        // touching the caches or the log, otherwise every probe for a stale
        // id would force a render-thread copy to refresh for no reason.
        if !self.nodes.contains_key(&id) {
            return None;
        }
        self.clear_derived_caches();
        self.record_change(SceneChange::Node(id));
        self.touch_stamp(id);
        self.nodes.get_mut(&id)
    }

    /// Replace `id`'s node with `node` — a copy's way of taking over an edit
    /// the original made through [`Scene::get_mut`] — and set its geometry
    /// stamp to `stamp` (the original's, so renderer caches keyed on stamps
    /// agree across the two scenes). The node's `parent` and `index` must match
    /// the stored node's: those are structural and go through
    /// [`Scene::set_parent`] / [`Scene::set_index`]; a mismatch is refused
    /// without touching the scene.
    ///
    /// Invalidation is targeted rather than wholesale: with the hierarchy
    /// unchanged, only `id` and its descendants can have a different world
    /// transform, and only `id` and its ancestors a different local-bounds
    /// union.
    pub fn patch_node(&mut self, node: CanvasNode, stamp: u64) -> Result<(), SceneError> {
        let id = node.id;
        let existing = self.nodes.get(&id).ok_or(SceneError::NotFound(id))?;
        if existing.parent != node.parent || existing.index != node.index {
            return Err(SceneError::InvariantViolated(format!(
                "patch_node {id}: parent or z-index differs from the stored node"
            )));
        }
        self.nodes.insert(id, node);
        self.invalidate_node_edit(id);
        self.record_change(SceneChange::Node(id));
        self.node_stamps.insert(id, stamp);
        Ok(())
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
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
        self.touch_stamp(id);
        if let Some(parent) = parent_key {
            self.touch_stamp(parent);
        }
        Ok(id)
    }

    /// Insert a batch of nodes as ONE structural edit: one derived-cache
    /// clear, one [`SceneChange::Structural`] log entry, one revision bump,
    /// and each touched child bucket sorted once instead of one binary-search
    /// insert per node. The result is byte-for-byte what inserting the same
    /// nodes one at a time with [`Scene::insert`] (in any valid order) would
    /// have produced: `children_of` is the `(index, id)` total order either
    /// way. Returns the inserted ids in batch order; an empty batch is a
    /// no-op that leaves the revision alone.
    ///
    /// Every node's `parent` and `index` are honored as-is. A parent may be
    /// an existing node or another node anywhere in the batch (before or
    /// after its child); either way it must be able to have children. The
    /// whole batch is validated first — duplicate ids within the batch or
    /// against the scene, a missing or non-container parent, a parent chain
    /// that cycles inside the batch — so an error leaves the scene untouched.
    pub fn insert_many(
        &mut self,
        nodes: impl IntoIterator<Item = CanvasNode>,
    ) -> Result<Vec<NodeId>, SceneError> {
        let batch: Vec<CanvasNode> = nodes.into_iter().collect();
        if batch.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch_position: IdHashMap<NodeId, usize> = IdHashMap::default();
        batch_position.reserve(batch.len());
        for (position, node) in batch.iter().enumerate() {
            if self.nodes.contains_key(&node.id)
                || batch_position.insert(node.id, position).is_some()
            {
                return Err(SceneError::Duplicate(node.id));
            }
        }
        for node in &batch {
            let Some(parent) = node.parent else {
                continue;
            };
            let parent_node = match self.nodes.get(&parent) {
                Some(existing) => existing,
                None => batch_position
                    .get(&parent)
                    .and_then(|position| batch.get(*position))
                    .ok_or(SceneError::ParentMissing(parent))?,
            };
            if !parent_node.can_have_children() {
                return Err(SceneError::ParentNotContainer(parent));
            }
        }
        Self::check_batch_acyclic(&batch, &batch_position)?;

        let mut new_children: IdHashMap<Option<NodeId>, Vec<NodeId>> = IdHashMap::default();
        let mut inserted = Vec::with_capacity(batch.len());
        for node in batch {
            let id = node.id;
            new_children.entry(node.parent).or_default().push(id);
            self.nodes.insert(id, node);
            inserted.push(id);
        }
        let mut touched_parents = Vec::with_capacity(new_children.len());
        for (parent, mut children) in new_children {
            let bucket = self.child_index.entry(parent).or_default();
            bucket.append(&mut children);
            sort_children(&self.nodes, bucket);
            if let Some(parent) = parent {
                touched_parents.push(parent);
            }
        }
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
        for id in inserted.iter().chain(touched_parents.iter()) {
            self.touch_stamp(*id);
        }
        Ok(inserted)
    }

    /// Refuse a batch whose parent links, followed through nodes of the batch
    /// itself, come back around. Parents already in the scene end a walk:
    /// the scene is acyclic and a batch node cannot become an ancestor of an
    /// existing one. Each node is walked once, so this is linear in the batch.
    fn check_batch_acyclic(
        batch: &[CanvasNode],
        batch_position: &IdHashMap<NodeId, usize>,
    ) -> Result<(), SceneError> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Walk {
            Unvisited,
            OnPath,
            Done,
        }
        let mut state = vec![Walk::Unvisited; batch.len()];
        let mut path: Vec<usize> = Vec::new();
        for start in 0..batch.len() {
            if state.get(start) != Some(&Walk::Unvisited) {
                continue;
            }
            path.clear();
            let mut cursor = Some(start);
            while let Some(position) = cursor {
                let Some(node) = batch.get(position) else {
                    break;
                };
                match state.get(position).copied() {
                    Some(Walk::Done) | None => break,
                    Some(Walk::OnPath) => {
                        let descendant = path
                            .last()
                            .and_then(|last| batch.get(*last))
                            .map(|last| last.id)
                            .unwrap_or(node.id);
                        return Err(SceneError::Cycle {
                            descendant,
                            ancestor: node.id,
                        });
                    }
                    Some(Walk::Unvisited) => {}
                }
                if let Some(slot) = state.get_mut(position) {
                    *slot = Walk::OnPath;
                }
                path.push(position);
                cursor = node
                    .parent
                    .and_then(|parent| batch_position.get(&parent))
                    .copied();
            }
            for position in path.drain(..) {
                if let Some(slot) = state.get_mut(position) {
                    *slot = Walk::Done;
                }
            }
        }
        Ok(())
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
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
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
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
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
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
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
            sort_children(&self.nodes, v);
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

    /// Rebuild the child index from scratch. Used after deserialization
    /// (the index is `#[serde(skip)]`) and as a recovery tool.
    pub fn rebuild_child_index(&mut self) {
        self.child_index.clear();
        let mut buckets: IdHashMap<Option<NodeId>, Vec<NodeId>> = IdHashMap::default();
        for n in self.nodes.values() {
            buckets.entry(n.parent).or_default().push(n.id);
        }
        for v in buckets.values_mut() {
            sort_children(&self.nodes, v);
        }
        self.child_index = buckets;
        // The index rebuild follows a bulk mutation (typically a deserialize);
        // the world cache is already empty after `#[serde(skip)]`, but clear
        // defensively so this method is safe to call at any point.
        self.clear_derived_caches();
        self.record_change(SceneChange::Structural);
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

    /// Transform-only write with cache-precision: bumps the revision like
    /// every mutator, but invalidates only what a local-transform change can
    /// stale — the world transforms of `id` and its descendants, the
    /// local-bounds unions of its ancestors, and the spatial index — so a drag
    /// on a huge page recomputes the moved subtree, not the whole document.
    /// It also skips the geometry stamp unless a boolean ancestor bakes this
    /// node's transform into a cached fold. Local vector geometry does not
    /// depend on the transform, so a plain drag through this method leaves
    /// every renderer path cache entry valid.
    pub fn set_transform(&mut self, id: NodeId, transform: Transform2D) -> Result<(), SceneError> {
        let in_boolean = self
            .ancestors_of(id)
            .any(|ancestor| matches!(ancestor.data, crate::node::NodeData::Boolean(_)));
        let node = self.nodes.get_mut(&id).ok_or(SceneError::NotFound(id))?;
        node.transform = transform;
        self.invalidate_transform_edit(id);
        self.record_change(SceneChange::Transform(id));
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

    // ---- change log ----------------------------------------------------------

    /// Bump the revision and log `change` against it. The ONLY place the
    /// revision moves, so the log's revision sequence has no gaps — see the
    /// field docs on [`Scene::change_log`].
    pub(crate) fn record_change(&self, change: SceneChange) {
        let revision = self.revision.get().wrapping_add(1);
        self.revision.set(revision);
        let mut log = self.change_log.borrow_mut();
        log.push_back((revision, change));
        while log.len() > SCENE_CHANGE_LOG_CAP {
            log.pop_front();
        }
    }

    /// What a copy of this scene taken at `revision` must refresh to match it
    /// now, or `None` when that cannot be answered node by node and the copy
    /// has to be re-taken: the copy is older than the log window, some
    /// mutation since was structural or untracked, more than
    /// [`SCENE_DELTA_MAX_NODES`] nodes were touched, or a touched node no
    /// longer exists. A copy at the current revision needs nothing
    /// (`Some(empty)`).
    ///
    /// Only meaningful for a copy of THIS instance (compare
    /// [`Scene::instance_id`] first): revisions of different instances are
    /// unrelated counters.
    pub fn changes_since(&self, revision: u64) -> Option<SceneDelta> {
        if revision == self.revision.get() {
            return Some(SceneDelta::default());
        }
        let log = self.change_log.borrow();
        let mut oldest_seen: Option<u64> = None;
        let mut transforms: Vec<NodeId> = Vec::new();
        let mut nodes: Vec<NodeId> = Vec::new();
        for &(revision_after, change) in log.iter().rev() {
            if revision_after <= revision {
                break;
            }
            oldest_seen = Some(revision_after);
            match change {
                SceneChange::Transform(id) => transforms.push(id),
                SceneChange::Node(id) => nodes.push(id),
                SceneChange::Structural | SceneChange::Unknown => return None,
            }
            if transforms.len() + nodes.len() > SCENE_DELTA_MAX_NODES {
                // Deduplicate before giving up: a drag logs the same ids frame
                // after frame, and only distinct nodes cost the consumer.
                transforms.sort_unstable();
                transforms.dedup();
                nodes.sort_unstable();
                nodes.dedup();
                if transforms.len() + nodes.len() > SCENE_DELTA_MAX_NODES {
                    return None;
                }
            }
        }
        // Contiguity: the oldest entry we consumed must be the very first
        // mutation after `revision`, otherwise the log window starts later
        // than the copy and some mutation went unseen.
        if oldest_seen != Some(revision.wrapping_add(1)) {
            return None;
        }
        nodes.sort_unstable();
        nodes.dedup();
        transforms.sort_unstable();
        transforms.dedup();
        transforms.retain(|id| nodes.binary_search(id).is_err());
        if transforms.len() + nodes.len() > SCENE_DELTA_MAX_NODES {
            return None;
        }
        if transforms
            .iter()
            .chain(nodes.iter())
            .any(|id| !self.nodes.contains_key(id))
        {
            return None;
        }
        Some(SceneDelta { transforms, nodes })
    }
}

/// Sort one child bucket into child-index order: ascending `IndexKey`,
/// equal keys tie-broken by the node's stable ULID. This is the total order
/// [`Scene::child_index_insert`] binary-searches on, so a bucket sorted here
/// — after a deserialize, a bulk insert, or the `validate` re-derivation —
/// is exactly what the same nodes inserted one at a time would have
/// produced; without the id tie-break, equal-index siblings would land in
/// process-random `HashMap` order and replay / snapshot diffs would be
/// non-deterministic. A node missing from the map cannot occur for a bucket
/// derived from it; it sorts first rather than panicking.
fn sort_children(nodes: &IdHashMap<NodeId, CanvasNode>, bucket: &mut [NodeId]) {
    bucket.sort_by(|a, b| {
        let index_of = |id: &NodeId| nodes.get(id).map(|node| node.index);
        index_of(a).cmp(&index_of(b)).then_with(|| a.cmp(b))
    });
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

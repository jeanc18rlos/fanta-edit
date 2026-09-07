//! [`Doc`] — the top-level Fanta document.
//!
//! Owns [`Scene`], [`Selection`], [`History`], a [`Viewport`], and [`DocMetadata`].
//! Serializes to the canonical `.fant.json` projection that AI agents read and
//! write directly (see ARCHITECTURE.md §6 and §8).

use crate::component::ComponentLibrary;
use crate::history::History;
use crate::id::{DocId, ModeId, NodeId, VariableCollectionId};
use crate::motion::MotionLibrary;
use crate::op::{OpCtx, Operation};
use crate::scene::{Scene, SceneError};
use crate::selection::Selection;
use crate::variables::VariableRegistry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current canonical JSON schema version. Bump when the on-disk shape changes;
/// `fanta-format` owns the migration table.
///
/// v2 (this version) made `PathData` an object `{ segments, fill_rule? }` (was a
/// bare array) and added components/variables/prototyping — all of which are
/// `#[serde(default)]`, so the only non-defaultable change driving the bump is
/// the `PathData` shape. Motion is likewise additive/defaultable and does not
/// require another bump. See `fanta-format::migrate` for the v1→v2 step.
pub const SCHEMA_VERSION: u32 = 2;

/// One Fantaisa document — the root of everything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Doc {
    /// Globally unique document identity. Stable for the document's lifetime.
    pub id: DocId,

    /// `.fant.json` schema version. Loaders use this to drive migrations.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,

    #[serde(default)]
    pub metadata: DocMetadata,

    pub scene: Scene,

    #[serde(default)]
    pub selection: Selection,

    /// The undo/redo stack. Persisted so a re-opened doc resumes its history
    /// (truncated to `History::max_depth` on save).
    #[serde(default)]
    pub history: History,

    /// Last-saved viewport state — saving this means "open the doc where the
    /// user left off" without an extra UI-state file.
    #[serde(default)]
    pub viewport: Viewport,

    /// Page roots, in document order. Each entry is a root-level container node
    /// id; a Figma `CANVAS` imports to one. Pages let a single document hold
    /// several independent canvases (Figma's page tabs) that share one undo
    /// history and asset pool but occupy separate coordinate spaces.
    ///
    /// Empty means "no explicit pages" — a hand-authored doc, or one saved
    /// before pages existed. Callers treat that as a single implicit page and
    /// render every root, so this is fully backward compatible (`#[serde(default)]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<NodeId>,

    /// Which page is currently shown in the editor, or `None` to show all roots
    /// (the implicit-single-page / legacy case). Persisted so a re-open restores
    /// the active page. Always either `None` or a member of [`Self::pages`];
    /// [`Self::set_active_page`] enforces that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_page: Option<NodeId>,

    /// Component & variant library: master definitions and component sets. The
    /// master *subtrees* live in [`Self::scene`] (under a hidden Components page);
    /// this registry holds only the metadata a plain subtree can't carry (prop
    /// schema, variant membership, revision). Empty in docs without components,
    /// so old files round-trip byte-identical.
    #[serde(default, skip_serializing_if = "ComponentLibrary::is_empty")]
    pub components: ComponentLibrary,

    /// Design-system variables (tokens): collections, modes, and per-mode values.
    #[serde(default, skip_serializing_if = "VariableRegistry::is_empty")]
    pub variables: VariableRegistry,

    /// The active mode per variable collection, document-wide. A frame can pin a
    /// different mode via `GroupNode.explicit_modes`; this is the doc-level
    /// fallback. Persisted alongside `viewport` so a theme choice survives a
    /// re-open.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub active_modes: BTreeMap<VariableCollectionId, ModeId>,

    /// Authored animation clips. Playback samples this library into transient
    /// overrides; the committed scene is never mutated by moving a playhead.
    #[serde(default, skip_serializing_if = "MotionLibrary::is_empty")]
    pub motion: MotionLibrary,

    /// The prototype flow's starting frame, if one is set. Present mode opens
    /// here; `None` falls back to the active page. When [`flows`](Self::flows)
    /// is non-empty this mirrors the first flow's start (kept as its own field
    /// for compatibility with pre-flows docs and ops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_start: Option<NodeId>,

    /// Named prototype flows (Figma `flowStartingPoints`): each is an entry
    /// point a presenter can offer in a flow picker. Import fills one entry
    /// per starting point; a single-flow doc may leave this empty and use only
    /// [`flow_start`](Self::flow_start). Additive — absent on old docs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<Flow>,

    /// Presentation device configuration (Figma `prototypeDevice`): the device
    /// box the prototype was designed for plus its surround color. Hosts and
    /// the present runtime letterbox frames into this. Additive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<PresentationConfig>,
}

/// One named prototype entry point (Figma flow starting point).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flow {
    /// Author-facing flow name ("Onboarding", "Checkout").
    pub name: String,
    /// The frame the flow opens on.
    pub start: NodeId,
}

/// Device/presentation settings a prototype was authored against (Figma
/// `prototypeDevice`). All fields optional — files without a device carry
/// `None` for the whole struct.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresentationConfig {
    /// Logical device size `[w, h]` the frames target; presenters letterbox
    /// frames into this box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_size: Option<[f64; 2]>,
    /// Figma device preset identifier (e.g. "IPHONE_16_PRO"), verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Device rotation: `true` when presented landscape.
    #[serde(default, skip_serializing_if = "crate::serde_util::is_false")]
    pub landscape: bool,
    /// Surround/chrome color painted around the letterboxed frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_color: Option<crate::color::Color>,
}

fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Doc {
    /// Empty document with default metadata, identity viewport, no nodes.
    pub fn new() -> Self {
        let now = unix_seconds_now();
        Self {
            id: DocId::new(),
            schema_version: SCHEMA_VERSION,
            metadata: DocMetadata {
                title: "Untitled".into(),
                created_at: now,
                modified_at: now,
                author: None,
            },
            scene: Scene::new(),
            selection: Selection::new(),
            history: History::new(),
            viewport: Viewport::default(),
            pages: Vec::new(),
            active_page: None,
            components: ComponentLibrary::new(),
            variables: VariableRegistry::new(),
            active_modes: BTreeMap::new(),
            motion: MotionLibrary::new(),
            flow_start: None,
            flows: Vec::new(),
            presentation: None,
        }
    }

    // ---- pages ---------------------------------------------------------------

    /// The document's page roots, in order. Empty for single-implicit-page docs.
    pub fn pages(&self) -> &[NodeId] {
        &self.pages
    }

    /// The active page root, or `None` when the document renders all roots
    /// (no explicit pages, or none selected yet).
    pub fn active_page(&self) -> Option<NodeId> {
        self.active_page
    }

    /// Register `root` as a page. Appends to [`Self::pages`] (ignoring a
    /// duplicate) and, if no page was active yet, makes this the active one — so
    /// the first page an importer adds becomes the initially-shown canvas.
    /// `root` is expected to be a root-level node id; ordering is insertion
    /// order, matching the source file's page order.
    pub fn add_page(&mut self, root: NodeId) {
        if !self.pages.contains(&root) {
            self.pages.push(root);
        }
        if self.active_page.is_none() {
            self.active_page = Some(root);
        }
    }

    /// Set the active page. `Some(id)` is honored only if `id` is a known page
    /// or a component master's root — a master root scopes the canvas to that
    /// component alone (component editing), and every subsystem keyed to the
    /// active page (render root, hit-testing, tool parenting) follows it for
    /// free. Any other id is a no-op, so a stale id can't blank the canvas;
    /// `None` resets to "render all roots". Returns whether the active page
    /// changed.
    pub fn set_active_page(&mut self, page: Option<NodeId>) -> bool {
        let next = match page {
            Some(id) if self.pages.contains(&id) || self.is_component_root(id) => Some(id),
            Some(_) => return false, // unknown page id — ignore
            None => None,
        };
        let changed = self.active_page != next;
        self.active_page = next;
        changed
    }

    /// Whether `id` is the root node of a registered component master.
    pub fn is_component_root(&self, id: NodeId) -> bool {
        self.components.defs.values().any(|def| def.root == id)
    }

    /// Remove `page` from the page registry, returning its position in
    /// [`Self::pages`] if it was present (so a caller can re-insert it there to
    /// undo). A no-op for an unknown id. Touches only the registry — the page's
    /// scene subtree is removed separately (e.g. via an undoable
    /// [`Operation::DeleteSubtree`]); this just stops listing it in the switcher.
    ///
    /// When the removed page was the active one, the active page falls back to
    /// the page now at its old slot (the next page, or the previous when it was
    /// last), or `None` when the registry is left empty — so the canvas never
    /// keeps pointing at a page that's gone.
    pub fn remove_page(&mut self, page: NodeId) -> Option<usize> {
        let idx = self.pages.iter().position(|&p| p == page)?;
        self.pages.remove(idx);
        if self.active_page == Some(page) {
            // Prefer the page that slid into this slot, else the new last page.
            self.active_page = self.pages.get(idx).or_else(|| self.pages.last()).copied();
        }
        Some(idx)
    }

    /// Insert `page` into the registry at `index` (clamped to the current
    /// length), the inverse of [`Self::remove_page`] for undo. Ignores a
    /// duplicate. Does not change the active page — the caller restores that.
    pub fn insert_page_at(&mut self, index: usize, page: NodeId) {
        if self.pages.contains(&page) {
            return;
        }
        let at = index.min(self.pages.len());
        self.pages.insert(at, page);
    }

    /// Display name of a page (its root node's `name`), for a page switcher.
    /// `None` if the id isn't in the scene.
    pub fn page_name(&self, page: NodeId) -> Option<&str> {
        self.scene.get(page).map(|n| n.name.as_str())
    }

    /// The prototype flow's start frame, if one is set. Present mode opens here.
    pub fn flow_start(&self) -> Option<NodeId> {
        self.flow_start
    }

    /// Apply an op via the history. Updates `metadata.modified_at`. This is
    /// the recommended chokepoint for all programmatic mutations — tools, AI,
    /// scripts. Direct `scene` access bypasses undo and is reserved for
    /// migrations and load paths.
    ///
    /// The op runs through an [`OpCtx`] borrowing the doc's scene + component
    /// library + variable registry + mode map + motion library + flow start —
    /// built here by disjoint field borrows so it doesn't conflict with the
    /// `&mut history`.
    pub fn apply(&mut self, op: Operation) -> Result<(), SceneError> {
        let mut ctx = OpCtx {
            scene: &mut self.scene,
            components: &mut self.components,
            variables: &mut self.variables,
            active_modes: &mut self.active_modes,
            motion: &mut self.motion,
            flow_start: &mut self.flow_start,
        };
        self.history.apply(op, &mut ctx)?;
        self.metadata.modified_at = unix_seconds_now();
        Ok(())
    }

    /// Apply a whole pre-built [`Transaction`] as one undo step. The replay
    /// engine uses this so a recorded multi-op transaction re-collapses to a
    /// single history entry (matching the original recording) rather than one
    /// entry per op. Threads the same [`OpCtx`] as [`Self::apply`].
    ///
    /// [`Transaction`]: crate::history::Transaction
    pub fn apply_transaction(&mut self, tx: crate::history::Transaction) -> Result<(), SceneError> {
        let mut ctx = OpCtx {
            scene: &mut self.scene,
            components: &mut self.components,
            variables: &mut self.variables,
            active_modes: &mut self.active_modes,
            motion: &mut self.motion,
            flow_start: &mut self.flow_start,
        };
        self.history.apply_transaction(tx, &mut ctx)?;
        self.metadata.modified_at = unix_seconds_now();
        Ok(())
    }

    /// Install (or clear) the session-journal sink on this doc's history. The
    /// app calls this once it owns the live doc (and after any doc swap) so
    /// every committed edit is recorded; a cloned doc never inherits it.
    pub fn set_journal_sink(&mut self, sink: Option<crate::journal::JournalSink>) {
        self.history.set_journal_sink(sink);
    }

    /// Whether this doc's history is emitting journal events.
    pub fn is_journaling(&self) -> bool {
        self.history.is_journaling()
    }

    /// Undo one transaction. Returns `false` if the undo stack was empty.
    pub fn undo(&mut self) -> Result<bool, SceneError> {
        let mut ctx = OpCtx {
            scene: &mut self.scene,
            components: &mut self.components,
            variables: &mut self.variables,
            active_modes: &mut self.active_modes,
            motion: &mut self.motion,
            flow_start: &mut self.flow_start,
        };
        let did = self.history.undo(&mut ctx)?;
        if did {
            self.metadata.modified_at = unix_seconds_now();
        }
        Ok(did)
    }

    /// Redo one transaction. Returns `false` if the redo stack was empty.
    pub fn redo(&mut self) -> Result<bool, SceneError> {
        let mut ctx = OpCtx {
            scene: &mut self.scene,
            components: &mut self.components,
            variables: &mut self.variables,
            active_modes: &mut self.active_modes,
            motion: &mut self.motion,
            flow_start: &mut self.flow_start,
        };
        let did = self.history.redo(&mut ctx)?;
        if did {
            self.metadata.modified_at = unix_seconds_now();
        }
        Ok(did)
    }

    /// Apply Figma-style constraints to direct children of `parent` when the
    /// parent's size changes (e.g. a frame/artboard resize). This wires the
    /// `constraints` data (imported by fanta-fig-interop and set on nodes) into
    /// live behavior: children with Left/Right/Scale etc. get their transforms
    /// adjusted via `CanvasNode::apply_constraints` + SetTransform ops.
    ///
    /// Returns the number of children whose transforms were updated.
    /// Tools (resize etc.) and importers that mutate container sizes should
    /// call this to keep responsive layout faithful.
    pub fn apply_constraints_for_parent_resize(
        &mut self,
        parent: NodeId,
        old_size: [f64; 2],
        new_size: [f64; 2],
    ) -> Result<usize, SceneError> {
        let children: Vec<NodeId> = self.scene.children_of(Some(parent)).to_vec();
        let mut adjusted = 0usize;
        for cid in children {
            if let Some(child) = self.scene.get(cid) {
                if child.constraints.is_some() {
                    if let Some(new_tx) = child.apply_constraints(old_size, new_size) {
                        let old_tx = child.transform;
                        self.apply(Operation::SetTransform {
                            id: cid,
                            old: old_tx,
                            new: new_tx,
                        })?;
                        adjusted += 1;
                    }
                }
            }
        }
        Ok(adjusted)
    }

    /// Serialize to the canonical `.fant.json` projection.
    pub fn to_json_string(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// Pretty-printed variant for diff-friendly version control and debugging.
    pub fn to_json_pretty(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    /// Parse from `.fant.json`. Runs any needed schema migration on the raw JSON
    /// FIRST (the bare-JSON load path the AI projection uses must migrate too,
    /// not only the `fanta-format` container path), then deserializes, rebuilds
    /// the scene's child index (`#[serde(skip)]` for compactness), and validates.
    pub fn from_json_str(s: &str) -> Result<Self, DocLoadError> {
        let mut value: serde_json::Value = serde_json::from_str(s)?;
        let from = value
            .get("schema_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        if from < SCHEMA_VERSION {
            migrate_doc_json(&mut value, from);
        }
        let mut doc: Doc = serde_json::from_value(value)?;
        doc.scene.rebuild_child_index();
        doc.scene.validate().map_err(DocLoadError::InvalidScene)?;
        Ok(doc)
    }
}

/// Migrate a raw `.fant.json` [`serde_json::Value`] in place from `from_version`
/// up to [`SCHEMA_VERSION`]. Shared by [`Doc::from_json_str`] and the
/// `fanta-format` container loader so both load paths apply the same steps.
///
/// Steps:
/// - **v1 → v2**: `PathData` changed from a bare segment array to an object
///   `{ "segments": [...] }`. Walk the whole doc generically and wrap every
///   vector node's array `path` — including paths embedded in persisted history
///   op snapshots (`DeleteSubtree`, `ReplaceData`), which a node-tree-only walk
///   would miss. Idempotent (an object `path` passes through).
pub fn migrate_doc_json(value: &mut serde_json::Value, from_version: u32) {
    if from_version < 2 {
        migrate_v1_to_v2(value);
    }
    // Stamp the current version so the deserialized doc reports v2.
    if let serde_json::Value::Object(map) = value {
        map.insert(
            "schema_version".into(),
            serde_json::Value::from(SCHEMA_VERSION),
        );
    }
}

/// Recursively wrap any vector node's bare-array `path` into `{ "segments": [...] }`.
fn migrate_v1_to_v2(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            let is_vector = map.get("type").and_then(|t| t.as_str()) == Some("vector");
            if is_vector {
                if let Some(path) = map.get_mut("path") {
                    if path.is_array() {
                        let segments = path.take();
                        *path = serde_json::json!({ "segments": segments });
                    }
                }
            }
            for child in map.values_mut() {
                migrate_v1_to_v2(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                migrate_v1_to_v2(item);
            }
        }
        _ => {}
    }
}

impl Default for Doc {
    fn default() -> Self {
        Self::new()
    }
}

/// Lightweight metadata shown in file pickers and recent-files lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocMetadata {
    pub title: String,
    /// Unix epoch seconds.
    pub created_at: i64,
    /// Unix epoch seconds.
    pub modified_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

impl Default for DocMetadata {
    fn default() -> Self {
        let now = unix_seconds_now();
        Self {
            title: "Untitled".into(),
            created_at: now,
            modified_at: now,
            author: None,
        }
    }
}

/// Last-saved viewport state. Persisted so re-opens land where the user left.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Viewport center in world space.
    pub center: [f64; 2],
    /// Zoom factor — world units per screen pixel inverted. 1.0 = 100%.
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            center: [0.0, 0.0],
            zoom: 1.0,
        }
    }
}

/// Errors that can happen when loading a `.fant.json`.
#[derive(Debug, thiserror::Error)]
pub enum DocLoadError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("scene failed validation: {0}")]
    InvalidScene(SceneError),
}

fn unix_seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{CanvasNode, GroupNode, NodeData, VectorNode};
    use crate::{NodeId, color::Color};

    #[test]
    fn empty_doc_round_trips_through_json() {
        let d = Doc::new();
        let s = d.to_json_string().unwrap();
        let back = Doc::from_json_str(&s).unwrap();
        assert_eq!(back.id, d.id);
        assert_eq!(back.schema_version, SCHEMA_VERSION);
    }

    /// Insert a root-level group named `name` and return its id.
    fn add_root_group(doc: &mut Doc, name: &str) -> NodeId {
        let mut g = CanvasNode::new(NodeData::Group(GroupNode::default()));
        g.name = name.to_owned();
        let id = g.id;
        doc.apply(Operation::create_node(g)).unwrap();
        id
    }

    #[test]
    fn pages_track_order_active_and_names() {
        let mut d = Doc::new();
        assert!(d.pages().is_empty());
        assert_eq!(d.active_page(), None);

        let p1 = add_root_group(&mut d, "Cover");
        let p2 = add_root_group(&mut d, "Components");
        d.add_page(p1);
        d.add_page(p2);

        // Insertion order preserved; first added page becomes active.
        assert_eq!(d.pages(), &[p1, p2]);
        assert_eq!(d.active_page(), Some(p1));
        assert_eq!(d.page_name(p1), Some("Cover"));
        assert_eq!(d.page_name(p2), Some("Components"));

        // Adding a duplicate is a no-op and doesn't disturb the active page.
        d.add_page(p1);
        assert_eq!(d.pages(), &[p1, p2]);

        // Switching to a known page changes the active page.
        assert!(d.set_active_page(Some(p2)));
        assert_eq!(d.active_page(), Some(p2));
        // Re-setting the same page reports "no change".
        assert!(!d.set_active_page(Some(p2)));

        // An unknown id is ignored — a stale page can't blank the canvas.
        assert!(!d.set_active_page(Some(NodeId::new())));
        assert_eq!(d.active_page(), Some(p2));

        // None resets to "render all roots".
        assert!(d.set_active_page(None));
        assert_eq!(d.active_page(), None);
    }

    #[test]
    fn a_component_master_root_can_be_the_active_page() {
        let mut d = Doc::new();
        let page = add_root_group(&mut d, "Page 1");
        d.add_page(page);
        let master = add_root_group(&mut d, "Button");
        d.apply(Operation::DefineComponent {
            def: Box::new(crate::component::ComponentDef {
                id: crate::ComponentId::new(),
                root: master,
                name: "Button".into(),
                variant_of: None,
                props: Vec::new(),
                rev: 0,
            }),
        })
        .unwrap();

        // Scoping the canvas to the master root is honored — component
        // editing renders, hit-tests, and parents into that subtree alone.
        assert!(d.is_component_root(master));
        assert!(d.set_active_page(Some(master)));
        assert_eq!(d.active_page(), Some(master));

        // A plain non-page, non-master node is still rejected.
        let loose = add_root_group(&mut d, "Loose");
        assert!(!d.set_active_page(Some(loose)));
        assert_eq!(d.active_page(), Some(master));
    }

    #[test]
    fn remove_page_drops_it_and_falls_back_active() {
        let mut d = Doc::new();
        let p1 = add_root_group(&mut d, "A");
        let p2 = add_root_group(&mut d, "B");
        let p3 = add_root_group(&mut d, "C");
        d.add_page(p1);
        d.add_page(p2);
        d.add_page(p3);
        assert_eq!(d.active_page(), Some(p1));

        // Removing a non-active middle page leaves active untouched, returns idx.
        assert_eq!(d.remove_page(p2), Some(1));
        assert_eq!(d.pages(), &[p1, p3]);
        assert_eq!(d.active_page(), Some(p1));

        // Removing the active page (slot 0) falls back to the page now at slot 0.
        assert_eq!(d.remove_page(p1), Some(0));
        assert_eq!(d.pages(), &[p3]);
        assert_eq!(d.active_page(), Some(p3));

        // Removing the last page leaves the registry empty and active None.
        assert_eq!(d.remove_page(p3), Some(0));
        assert!(d.pages().is_empty());
        assert_eq!(d.active_page(), None);

        // An unknown id is a no-op (None).
        assert_eq!(d.remove_page(NodeId::new()), None);
    }

    #[test]
    fn remove_active_last_page_falls_back_to_previous() {
        let mut d = Doc::new();
        let p1 = add_root_group(&mut d, "A");
        let p2 = add_root_group(&mut d, "B");
        d.add_page(p1);
        d.add_page(p2);
        d.set_active_page(Some(p2));
        // Removing the active *last* page falls back to the previous one.
        assert_eq!(d.remove_page(p2), Some(1));
        assert_eq!(d.pages(), &[p1]);
        assert_eq!(d.active_page(), Some(p1));
    }

    #[test]
    fn insert_page_at_inverts_remove() {
        let mut d = Doc::new();
        let p1 = add_root_group(&mut d, "A");
        let p2 = add_root_group(&mut d, "B");
        let p3 = add_root_group(&mut d, "C");
        d.add_page(p1);
        d.add_page(p2);
        d.add_page(p3);

        // Remove the middle page, then re-insert at its old slot to restore order.
        let idx = d.remove_page(p2).unwrap();
        assert_eq!(d.pages(), &[p1, p3]);
        d.insert_page_at(idx, p2);
        assert_eq!(d.pages(), &[p1, p2, p3]);

        // Inserting a duplicate is a no-op; out-of-range index clamps to the end.
        d.insert_page_at(0, p2);
        assert_eq!(d.pages(), &[p1, p2, p3]);
        let p4 = add_root_group(&mut d, "D");
        d.insert_page_at(99, p4);
        assert_eq!(d.pages(), &[p1, p2, p3, p4]);
    }

    #[test]
    fn pages_round_trip_through_json() {
        let mut d = Doc::new();
        let p1 = add_root_group(&mut d, "Page 1");
        let p2 = add_root_group(&mut d, "Page 2");
        d.add_page(p1);
        d.add_page(p2);
        d.set_active_page(Some(p2));

        let s = d.to_json_pretty().unwrap();
        let back = Doc::from_json_str(&s).unwrap();
        assert_eq!(back.pages(), &[p1, p2]);
        assert_eq!(back.active_page(), Some(p2));
    }

    #[test]
    fn populated_doc_round_trips_through_json() {
        let mut d = Doc::new();
        let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            5.0,
            5.0,
            20.0,
            20.0,
            Color::rgb(255, 100, 50),
        )));
        let id = n.id;
        d.apply(Operation::create_node(n)).unwrap();
        let s = d.to_json_pretty().unwrap();
        let back = Doc::from_json_str(&s).unwrap();
        assert!(back.scene.contains(id));
        // Rebuilt child index lets us see the root child.
        assert!(back.scene.roots().contains(&id));
    }

    #[test]
    fn modified_at_updates_on_apply() {
        let mut d = Doc::new();
        let original = d.metadata.modified_at;
        // Sleep a beat so the second timestamp is strictly later.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            1.0,
            1.0,
            Color::BLACK,
        )));
        d.apply(Operation::create_node(n)).unwrap();
        assert!(d.metadata.modified_at > original);
    }

    #[test]
    fn doc_load_rejects_scene_with_dangling_parent_ref() {
        // Craft a doc JSON whose node references a non-existent parent.
        let bad = serde_json::json!({
            "id": "01000000000000000000000000",
            "schema_version": 1,
            "metadata": {"title": "x", "created_at": 0, "modified_at": 0},
            "scene": {
                "nodes": {
                    "01000000000000000000000001": {
                        "id": "01000000000000000000000001",
                        "parent": "01000000000000000000000099",
                        "index": 1.0,
                        "name": "Orphan",
                        "type": "group"
                    }
                }
            },
        });
        let res = Doc::from_json_str(&bad.to_string());
        assert!(matches!(res, Err(DocLoadError::InvalidScene(_))));
    }
}

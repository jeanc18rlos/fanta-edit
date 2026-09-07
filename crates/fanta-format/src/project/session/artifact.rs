//! Per-artifact session: dirty FSM, apply, text edit, save, conflict.

use super::diagnostics::unknown_attribute_diagnostics;
use super::error::{SaveBlocked, SessionError};
use super::graphics::materialize_graphics;
use super::hash::{ContentHash, hash_file_set};
use super::materialize::{
    materialize_component, materialize_from_node_map, materialize_page, project_scene_to_node_map,
};
use super::review::{MergeReview, ProposalApplied, SourceProposalOutcome};
use super::types::{
    ArtifactDirty, ArtifactEdition, ArtifactId, ArtifactMeta, ArtifactOpImpact, ConflictOrigin,
    ConflictResolution, ConflictState, EditionSide, MergePreview, SaveResult, ScopedDoc,
    SourceDiagnostic, SourceRebuildReason, SourceSync,
};
use crate::project::layout::{
    DEF_JSON, MASTER_FNX, MASTER_IDS, PAGE_FNX, PAGE_IDS, PAGE_JSON, json_bytes,
};
use crate::project::merge::{NodeMapEdition, merge_artifact};
use fanta_doc::{
    CanvasNode, ComponentLibrary, Doc, History, NodeId, Operation, Selection, Transaction,
    VariableRegistry, Viewport,
};
use fanta_fnx::{
    ArtifactIr, ArtifactKind, FnxElement, FnxSidecar, FnxSourceMirror, RefTable,
    artifact_file_names, reconcile_sidecar,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// One open design tab.
#[derive(Debug, Clone)]
pub struct ArtifactSession {
    pub id: ArtifactId,
    pub kind: ArtifactKind,
    pub meta: ArtifactMeta,
    pub state: ArtifactDirty,
    pub(crate) ir: ArtifactIr,
    source: FnxSourceMirror,
    pub(crate) ir_stale: bool,
    pub(crate) scene_stale: bool,
    pub(crate) text: Option<String>,
    pub(crate) last_valid_ir: Option<ArtifactIr>,
    pub(crate) scoped: ScopedDoc,
    pub viewport: Viewport,
    pub vars_generation: u64,
    pub base_hash: ContentHash,
    pub disk_hash: ContentHash,
    pub base_nodes: Arc<NodeMapEdition>,
    pub fn_name: String,
    /// Name↔id context for every parse/print this session performs:
    /// `component="Button"` and `"$Collection/Name"` binding paths resolve
    /// through it on decode, and (for layout-v4 workspaces) ids re-sugar into
    /// those spellings on print. Rebuilt by the workspace whenever the shared
    /// variable registry changes (`vars_generation` bumps) and on every text
    /// commit / conflict resolution, which receive a fresh component library.
    /// The retained [`FnxSourceMirror`] holds the SAME `Arc`, so canvas
    /// patches and full prints always agree on reference spellings.
    pub(crate) ref_table: Arc<RefTable>,
    working_generation: u64,
    last_source_sync: SourceSync,
    /// Non-fatal authoring diagnostics from the LAST source ingestion — open,
    /// text commit, or source-proposal install (the paths that decode authored
    /// `.fnx`). Canvas edits never touch authored spellings, so they leave
    /// this untouched; a clean re-ingestion resets it to empty.
    last_diagnostics: Vec<SourceDiagnostic>,
    /// Scene state `(instance_id, revision)` at the last moment the retained
    /// FNX projection was known to equal the scene. `Scene::revision` bumps on
    /// every mutation path (conservatively — `get_mut` counts as a change) and
    /// `Scene::clone` mints a fresh `instance_id`, so an equal pair GUARANTEES
    /// the scene is byte-for-byte the one we last reconciled — which lets
    /// [`synchronize_retained_source_with_scene`](Self::synchronize_retained_source_with_scene)
    /// skip its two full-scene projections on the editor hot path. `None` (or
    /// any mismatch) degrades to the full reconciliation, never the reverse.
    last_synced_scene: Option<(u64, u64)>,
}

impl ArtifactSession {
    pub fn state(&self) -> &ArtifactDirty {
        &self.state
    }

    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    pub fn root(&self) -> fanta_doc::NodeId {
        self.scoped.root
    }

    pub fn doc(&self) -> &Doc {
        &self.scoped.doc
    }

    pub fn ir(&self) -> &ArtifactIr {
        &self.ir
    }

    pub fn text_buffer(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.viewport
    }

    /// Current in-memory FNX source. Canvas edits update this before returning;
    /// disk remains unchanged until [`save`](Self::save).
    pub fn source_text(&self) -> String {
        self.source.render()
    }

    pub fn working_generation(&self) -> u64 {
        self.working_generation
    }

    pub fn last_source_sync(&self) -> &SourceSync {
        &self.last_source_sync
    }

    /// Non-fatal diagnostics (e.g. `source.unknown_attribute` typo warnings)
    /// from the last authored-source ingestion — open, text commit, or
    /// source-proposal install. Empty when the last ingestion was clean;
    /// canvas edits leave the previous ingestion's diagnostics in place.
    pub fn source_diagnostics(&self) -> &[SourceDiagnostic] {
        &self.last_diagnostics
    }

    /// Whether the scene is provably unchanged since the retained source was
    /// last reconciled with it (see [`Self::last_synced_scene`]).
    fn scene_in_sync(&self) -> bool {
        self.last_synced_scene
            == Some((
                self.scoped.doc.scene.instance_id(),
                self.scoped.doc.scene.revision(),
            ))
    }

    /// Record that scene and retained source agree RIGHT NOW. Call only at a
    /// commit boundary where both sides were just installed or reconciled.
    fn mark_scene_synced(&mut self) {
        self.last_synced_scene = Some((
            self.scoped.doc.scene.instance_id(),
            self.scoped.doc.scene.revision(),
        ));
    }

    /// Apply an operation. Refuses variable-mutating ops (N13).
    pub fn apply(&mut self, op: Operation) -> Result<(), SessionError> {
        let mut transaction = Transaction::new(op.label());
        transaction.push(op);
        self.apply_transaction_atomic(transaction).map(|_| ())
    }

    /// Undo one canvas transaction while keeping the retained FNX projection
    /// in the same atomic session revision.
    pub fn undo_atomic(&mut self) -> Result<SourceSync, SessionError> {
        self.history_step_atomic(false)
    }

    /// Redo one canvas transaction while keeping the retained FNX projection
    /// in the same atomic session revision.
    pub fn redo_atomic(&mut self) -> Result<SourceSync, SessionError> {
        self.history_step_atomic(true)
    }

    /// Validate and apply a whole transaction against a scratch artifact, then
    /// install it in one swap. A failing later operation cannot leak earlier
    /// operations into the live scene, source mirror, generation, or history.
    pub fn apply_transaction_atomic(
        &mut self,
        transaction: Transaction,
    ) -> Result<SourceSync, SessionError> {
        if matches!(
            self.state,
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. }
        ) {
            return Err(SessionError::InvalidState(
                "cannot apply ops in Conflict/Invalid".into(),
            ));
        }
        if transaction.ops.iter().any(is_variable_op) {
            return Err(SessionError::UseWorkspaceArtifact);
        }
        if matches!(self.state, ArtifactDirty::DirtyText) {
            return Err(SessionError::InvalidState(
                "canvas ops forbidden while DirtyText — commit_text_to_scene first".into(),
            ));
        }

        if transaction.is_empty() {
            return Ok(SourceSync::Unchanged);
        }

        // Editor hot path: a non-structural transaction on a provably-synced
        // scene validates and installs in place with node-scoped rollback,
        // skipping the O(artifact) session clone entirely. Structural edits and
        // any drifted state keep the correct-first clone-then-swap below.
        let structural = transaction.ops.iter().any(is_structural_op);
        if !structural && self.scene_in_sync() {
            return self.apply_non_structural_in_place(transaction);
        }

        let mut scratch = self.clone();
        // `Scene::clone` mints a fresh `instance_id`, which would blind the
        // scratch's watermark even though its content is identical to ours.
        // Transfer the in-sync fact so the pre-apply reconciliation below can
        // take its skip path — this is the difference between O(edit) and
        // O(scene) for every pointer-move transaction.
        if self.scene_in_sync() {
            scratch.mark_scene_synced();
        }
        scratch.synchronize_retained_source_with_scene()?;
        let previous = if structural {
            BTreeMap::new()
        } else {
            scratch.capture_patch_targets(&transaction)
        };
        scratch
            .scoped
            .doc
            .apply_transaction(transaction.clone())
            .map_err(|e| SessionError::other(format!("apply: {e}")))?;
        if !structural && !previous.is_empty() {
            scratch
                .scoped
                .doc
                .undo()
                .map_err(|error| SessionError::other(format!("validate undo: {error}")))?;
            for (id, before) in &previous {
                let current = scratch.scoped.doc.scene.get(*id).ok_or_else(|| {
                    SessionError::OperationPrecondition {
                        node: id.to_string(),
                    }
                })?;
                let current = serde_json::to_value(current)
                    .map_err(|error| SessionError::other(error.to_string()))?;
                if current != before.value {
                    return Err(SessionError::OperationPrecondition {
                        node: id.to_string(),
                    });
                }
            }
            scratch
                .scoped
                .doc
                .redo()
                .map_err(|error| SessionError::other(format!("validate redo: {error}")))?;
        }
        let source_sync = if structural {
            scratch.rebuild_source_from_scene(SourceRebuildReason::StructuralOperation)?
        } else {
            scratch.patch_source_nodes(previous)?
        };

        // The scratch history intentionally has no journal sink. Preserve the
        // live history object, then record the already-validated transaction.
        let mut live_history = std::mem::replace(&mut self.scoped.doc.history, History::new());
        live_history.record_applied_transaction(transaction);
        scratch.scoped.doc.history = live_history;
        scratch.state = ArtifactDirty::DirtyCanvas;
        scratch.scene_stale = false;
        scratch.working_generation = self.working_generation.wrapping_add(1);
        scratch.last_source_sync = source_sync.clone();
        // The transaction's scene edits were just patched (or rebuilt) into the
        // retained source above — record the new commit boundary.
        scratch.mark_scene_synced();
        *self = scratch;
        Ok(source_sync)
    }

    /// The clone-free hot path for a non-structural transaction whose scene is
    /// provably in sync with the retained source (watermark match).
    ///
    /// Atomicity holds by staging: everything fallible runs before the first
    /// live mutation is allowed to survive, and each failure arm restores the
    /// captured node values, the live history object, and `modified_at`. The
    /// retained-source patch runs LAST; if it fails after the scene rollback,
    /// the session degrades to `ir_stale = true` with the watermark cleared, so
    /// the next synchronization re-derives (and self-heals) the projection
    /// instead of trusting a half-patched mirror.
    fn apply_non_structural_in_place(
        &mut self,
        transaction: Transaction,
    ) -> Result<SourceSync, SessionError> {
        let previous = self.capture_patch_targets(&transaction);

        // Swap in a sink-less scratch history: the journal must only observe
        // the transaction once it is fully validated (same contract as the
        // clone path), and undo/redo below must not replay through the sink.
        let live_history = std::mem::replace(&mut self.scoped.doc.history, History::new());
        let saved_modified = self.scoped.doc.metadata.modified_at;

        // Rollback helper: direct node writes are reserved for migrations and
        // THIS failure path — restoring validated pre-transaction snapshots of
        // exactly the nodes the transaction targeted.
        fn restore_nodes(scene: &mut fanta_doc::Scene, previous: &BTreeMap<NodeId, NodePatchBase>) {
            for (id, base) in previous {
                if let (Ok(node), Some(slot)) = (
                    serde_json::from_value::<CanvasNode>(base.value.clone()),
                    scene.get_mut(*id),
                ) {
                    *slot = node;
                }
            }
        }

        if let Err(error) = self.scoped.doc.apply_transaction(transaction.clone()) {
            restore_nodes(&mut self.scoped.doc.scene, &previous);
            self.scoped.doc.history = live_history;
            self.scoped.doc.metadata.modified_at = saved_modified;
            return Err(SessionError::other(format!("apply: {error}")));
        }

        // Stale-`old` detection, identical to the clone path: undo writes the
        // ops' stored `old` values back; a mismatch against the captured
        // pre-state means the caller's `old` no longer described the scene.
        if !previous.is_empty() {
            let validated = (|| -> Result<(), SessionError> {
                self.scoped
                    .doc
                    .undo()
                    .map_err(|error| SessionError::other(format!("validate undo: {error}")))?;
                for (id, before) in &previous {
                    let current = self.scoped.doc.scene.get(*id).ok_or_else(|| {
                        SessionError::OperationPrecondition {
                            node: id.to_string(),
                        }
                    })?;
                    let current = serde_json::to_value(current)
                        .map_err(|error| SessionError::other(error.to_string()))?;
                    if current != before.value {
                        return Err(SessionError::OperationPrecondition {
                            node: id.to_string(),
                        });
                    }
                }
                self.scoped
                    .doc
                    .redo()
                    .map(|_| ())
                    .map_err(|error| SessionError::other(format!("validate redo: {error}")))
            })();
            if let Err(error) = validated {
                restore_nodes(&mut self.scoped.doc.scene, &previous);
                self.scoped.doc.history = live_history;
                self.scoped.doc.metadata.modified_at = saved_modified;
                return Err(error);
            }
        }

        let source_sync = match self.patch_source_nodes(previous.clone()) {
            Ok(sync) => sync,
            Err(error) => {
                restore_nodes(&mut self.scoped.doc.scene, &previous);
                self.scoped.doc.history = live_history;
                self.scoped.doc.metadata.modified_at = saved_modified;
                // The mirror/IR may be part-patched; force the next sync to
                // re-derive from the (restored) scene rather than trust it.
                self.ir_stale = true;
                self.last_synced_scene = None;
                return Err(error);
            }
        };

        let mut live_history = live_history;
        live_history.record_applied_transaction(transaction);
        self.scoped.doc.history = live_history;
        self.state = ArtifactDirty::DirtyCanvas;
        self.scene_stale = false;
        self.working_generation = self.working_generation.wrapping_add(1);
        self.last_source_sync = source_sync.clone();
        self.mark_scene_synced();
        Ok(source_sync)
    }

    fn history_step_atomic(&mut self, redo: bool) -> Result<SourceSync, SessionError> {
        if matches!(
            self.state,
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. }
        ) {
            return Err(SessionError::InvalidState(
                "cannot change history in Conflict/Invalid".into(),
            ));
        }
        if matches!(self.state, ArtifactDirty::DirtyText) {
            return Err(SessionError::InvalidState(
                "canvas history is unavailable while DirtyText".into(),
            ));
        }

        // Validate the complete scene → retained-source transition before
        // touching the live document or its journal sink.
        let mut scratch = self.clone();
        let changed = if redo {
            scratch.scoped.doc.redo()
        } else {
            scratch.scoped.doc.undo()
        }
        .map_err(|error| SessionError::other(format!("history validation: {error}")))?;
        if !changed {
            return Ok(SourceSync::Unchanged);
        }
        scratch.ir_stale = true;
        let source_sync = scratch.synchronize_retained_source_with_scene()?;
        if matches!(source_sync, SourceSync::Unchanged) {
            scratch.state = ArtifactDirty::DirtyCanvas;
            scratch.ir_stale = false;
            scratch.working_generation = self.working_generation.wrapping_add(1);
            scratch.last_source_sync = SourceSync::Unchanged;
        }

        // Replay once on the live history so its journal sink observes the
        // undo/redo. The same transition already succeeded on the clone, so
        // installing the validated scene/source afterwards cannot fail.
        let committed = if redo {
            self.scoped.doc.redo()
        } else {
            self.scoped.doc.undo()
        }
        .map_err(|error| SessionError::other(format!("history commit: {error}")))?;
        if !committed {
            return Err(SessionError::other(
                "history changed between validation and commit",
            ));
        }
        scratch.scoped.doc.history =
            std::mem::replace(&mut self.scoped.doc.history, History::new());
        *self = scratch;
        Ok(source_sync)
    }

    /// Begin text editing: project canvas to IR if needed, return mutable buffer.
    pub fn begin_text_edit(&mut self) -> Result<&mut String, SessionError> {
        match self.state {
            ArtifactDirty::DirtyText => {}
            ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas => {
                self.commit_canvas_to_ir()?;
                let buffer = self.source.render();
                self.text = Some(buffer);
                self.state = ArtifactDirty::DirtyText;
                self.scene_stale = false;
            }
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. } => {
                return Err(SessionError::InvalidState(
                    "cannot begin text edit in this state".into(),
                ));
            }
        }
        Ok(self.text.as_mut().expect("DirtyText has buffer"))
    }

    pub fn set_text(&mut self, source: String) -> Result<(), SessionError> {
        if matches!(
            self.state,
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. }
        ) {
            return Err(SessionError::InvalidState(
                "cannot set_text in Conflict/Invalid".into(),
            ));
        }
        if matches!(
            self.state,
            ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas
        ) {
            self.commit_canvas_to_ir()?;
        }
        self.text = Some(source);
        self.state = ArtifactDirty::DirtyText;
        self.scene_stale = true;
        // First text mutation clears canvas history (mode-switch barrier).
        self.scoped.doc.history = History::new();
        Ok(())
    }

    /// Scene → IR (does not switch mode by itself).
    pub fn commit_canvas_to_ir(&mut self) -> Result<&ArtifactIr, SessionError> {
        self.synchronize_retained_source_with_scene()?;
        Ok(&self.ir)
    }

    /// Switch DirtyCanvas → DirtyText after projecting.
    pub fn switch_to_text(&mut self) -> Result<(), SessionError> {
        self.commit_canvas_to_ir()?;
        let buffer = self.source.render();
        self.text = Some(buffer);
        self.state = ArtifactDirty::DirtyText;
        Ok(())
    }

    /// Buffer → Scene; history cleared (N6).
    pub fn commit_text_to_scene(
        &mut self,
        project_id: fanta_doc::DocId,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    ) -> Result<(), SessionError> {
        if !matches!(self.state, ArtifactDirty::DirtyText) {
            return Err(SessionError::InvalidState(
                "commit_text_to_scene requires DirtyText".into(),
            ));
        }
        let source = self
            .text
            .as_ref()
            .ok_or_else(|| SessionError::other("DirtyText without buffer"))?
            .clone();
        match self.parse_and_materialize(&source, project_id, components, variables, active_modes) {
            Ok((ir, source_mirror, scoped, node_map, ref_table)) => {
                self.ir = ir;
                self.source = source_mirror;
                // The freshly built table (from the caller's component library
                // + variable registry) becomes the session context — the
                // mirror above already shares the same Arc.
                self.ref_table = ref_table;
                self.last_valid_ir = Some(self.ir.clone());
                self.scoped = scoped;
                self.scoped.doc.history = History::new();
                self.scene_stale = false;
                self.ir_stale = false;
                self.state = ArtifactDirty::DirtyCanvas;
                self.working_generation = self.working_generation.wrapping_add(1);
                self.last_source_sync = SourceSync::Rebuilt {
                    reason: SourceRebuildReason::StaleProjection,
                };
                // Scene was just materialized from the committed buffer.
                self.mark_scene_synced();
                // The committed buffer is authored source: surface typo'd
                // attributes as warnings (a clean commit resets to empty).
                self.last_diagnostics =
                    unknown_attribute_diagnostics(&node_map_values(&node_map.nodes));
                Ok(())
            }
            Err(e) => {
                let last_good = Some(Box::new(self.scoped.clone()));
                self.state = ArtifactDirty::Invalid {
                    error: e.to_string(),
                    last_good,
                };
                Err(e)
            }
        }
    }

    /// Pure Scene→IR preview without mutating session.ir.
    pub fn preview_ir(&self) -> Result<ArtifactIr, SessionError> {
        let header = self.base_nodes.header.clone();
        let (ir, _) = project_scene_to_node_map(&self.scoped, self.kind, &self.fn_name, header)?;
        Ok(ir)
    }

    fn parse_and_materialize(
        &self,
        source: &str,
        project_id: fanta_doc::DocId,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    ) -> Result<
        (
            ArtifactIr,
            FnxSourceMirror,
            ScopedDoc,
            NodeMapEdition,
            Arc<RefTable>,
        ),
        SessionError,
    > {
        // Build the resolution vocabulary from the caller's CURRENT component
        // library and variable registry (they are fresher than the session's
        // stored table — a component defined since open must resolve). The
        // emit-names policy is a workspace-layout fact and carries over.
        let ref_table = Arc::new(crate::project::refs_ctx::build_ref_table(
            &components,
            &variables,
            self.ref_table.emit_names(),
        ));
        let reconciled = deterministic_reconcile(source, self.ir.sidecar())?;
        let nodes = fanta_fnx::decode_subtree_with(source, &reconciled, &ref_table)?;
        let ir = ArtifactIr::from_source_with(
            self.kind,
            self.fn_name.clone(),
            source,
            reconciled.clone(),
            &ref_table,
        )?;
        let source_mirror =
            FnxSourceMirror::from_source_with(source, &reconciled, Arc::clone(&ref_table))?;
        let mut map = Map::new();
        for n in &nodes {
            if let Some(id) = n.get("id").and_then(Value::as_str) {
                map.insert(id.to_owned(), n.clone());
            }
        }
        let header = self.base_nodes.header.clone();
        let node_map = NodeMapEdition::new(header.clone(), map);
        let scoped = match self.kind {
            ArtifactKind::Page => {
                materialize_page(&ir, project_id, components, variables, active_modes)?
            }
            ArtifactKind::Component => {
                let def = serde_json::from_value(header).unwrap_or_else(|_| {
                    fanta_doc::ComponentDef::new(
                        match self.id {
                            ArtifactId::Component(c) => c,
                            _ => fanta_doc::ComponentId::new(),
                        },
                        ir.to_nodes()
                            .ok()
                            .and_then(|ns| {
                                ns.iter()
                                    .find(|n| n.get("parent").map_or(true, Value::is_null))
                                    .and_then(|n| n.get("id"))
                                    .and_then(Value::as_str)
                                    .and_then(|s| s.parse().ok())
                            })
                            .unwrap_or_else(fanta_doc::NodeId::new),
                        self.fn_name.clone(),
                    )
                });
                materialize_component(&ir, project_id, def, variables, active_modes)?
            }
            other => {
                return Err(SessionError::other(format!(
                    "kind {} not loadable",
                    other.label()
                )));
            }
        };
        Ok((ir, source_mirror, scoped, node_map, ref_table))
    }

    /// Project working state to file bytes for this artifact's file set.
    pub fn project_to_files(&self) -> Result<Vec<(String, Vec<u8>)>, SessionError> {
        match &self.state {
            ArtifactDirty::Clean => {
                // Re-read projection from scene for hash comparison.
                self.project_canvas_files()
            }
            ArtifactDirty::DirtyCanvas => self.project_canvas_files(),
            ArtifactDirty::DirtyText => {
                let source = self
                    .text
                    .as_ref()
                    .ok_or_else(|| SessionError::other("DirtyText missing buffer"))?;
                // Ensure parse works (name refs resolve against the retained table).
                let reconciled = deterministic_reconcile(source, self.ir.sidecar())?;
                let _nodes = fanta_fnx::decode_subtree_with(source, &reconciled, &self.ref_table)?;
                let mut files = Vec::new();
                let (fnx_name, ids_name, header_name) = file_names(self.kind);
                files.push((fnx_name.to_owned(), source.as_bytes().to_vec()));
                let mut sidecar_bytes = serde_json::to_vec_pretty(&reconciled)
                    .map_err(|e| SessionError::other(e.to_string()))?;
                sidecar_bytes.push(b'\n');
                files.push((ids_name.to_owned(), sidecar_bytes));
                let header = self.base_nodes.header.clone();
                files.push((
                    header_name.to_owned(),
                    json_bytes(&header).map_err(SessionError::from)?,
                ));
                Ok(files)
            }
            ArtifactDirty::Conflict(_) => Err(SessionError::SaveBlocked(SaveBlocked::InConflict)),
            ArtifactDirty::Invalid { .. } => Err(SessionError::SaveBlocked(SaveBlocked::Invalid)),
        }
    }

    fn project_canvas_files(&self) -> Result<Vec<(String, Vec<u8>)>, SessionError> {
        let header = self.projected_canvas_header()?;
        let (ir, text) = if self.ir_stale {
            let (ir, _map) =
                project_scene_to_node_map(&self.scoped, self.kind, &self.fn_name, header.clone())?;
            let text = ir.print_with(&self.ref_table);
            (ir, text)
        } else {
            (self.ir.clone(), self.source.render())
        };
        let (fnx_name, ids_name, header_name) = file_names(self.kind);
        let mut files = Vec::new();
        files.push((fnx_name.to_owned(), text.into_bytes()));
        let mut sidecar_bytes = serde_json::to_vec_pretty(ir.sidecar())
            .map_err(|e| SessionError::other(e.to_string()))?;
        sidecar_bytes.push(b'\n');
        files.push((ids_name.to_owned(), sidecar_bytes));
        files.push((
            header_name.to_owned(),
            json_bytes(&header).map_err(SessionError::from)?,
        ));
        Ok(files)
    }

    fn projected_canvas_header(&self) -> Result<Value, SessionError> {
        match self.kind {
            ArtifactKind::Page => {
                let mut h = Map::new();
                if let ArtifactId::Page(id) = self.id {
                    h.insert("id".into(), Value::from(id.to_string()));
                }
                if let Some(n) = self.scoped.doc.scene.get(self.scoped.root) {
                    h.insert("name".into(), Value::from(n.name.clone()));
                }
                // Preserve order if present.
                if let Some(o) = self.base_nodes.header.get("order") {
                    h.insert("order".into(), o.clone());
                } else {
                    h.insert("order".into(), Value::from(0u64));
                }
                Ok(Value::Object(h))
            }
            ArtifactKind::Component => {
                if let Some(def) = self.scoped.doc.components.defs.values().next() {
                    serde_json::to_value(def).map_err(|e| SessionError::other(e.to_string()))
                } else {
                    Ok(self.base_nodes.header.clone())
                }
            }
            _ => Ok(self.base_nodes.header.clone()),
        }
    }

    pub fn projected_node_map(&self) -> Result<NodeMapEdition, SessionError> {
        match &self.state {
            ArtifactDirty::DirtyText => {
                let source = self.text.as_ref().unwrap();
                let reconciled = deterministic_reconcile(source, self.ir.sidecar())?;
                let nodes = fanta_fnx::decode_subtree_with(source, &reconciled, &self.ref_table)?;
                let mut map = Map::new();
                for n in nodes {
                    if let Some(id) = n.get("id").and_then(Value::as_str) {
                        map.insert(id.to_owned(), n);
                    }
                }
                Ok(NodeMapEdition::new(self.base_nodes.header.clone(), map))
            }
            _ if self.ir_stale => self.scene_node_map(),
            _ => {
                let mut nodes = Map::new();
                for node in self.ir.to_nodes()? {
                    let id = node
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| SessionError::other("retained IR node is missing id"))?;
                    nodes.insert(id.to_owned(), node);
                }
                Ok(NodeMapEdition::new(self.projected_canvas_header()?, nodes))
            }
        }
    }

    /// Merge an agent-authored source candidate against the immutable disk
    /// base and the current unsaved canvas edition.
    ///
    /// A clean semantic merge installs atomically in memory. A real collision
    /// returns a detached review draft; the live canvas remains unchanged
    /// until [`apply_merge_review`](Self::apply_merge_review).
    pub fn propose_source(
        &mut self,
        source: &str,
        expected_base_hash: ContentHash,
    ) -> Result<SourceProposalOutcome, SessionError> {
        if expected_base_hash != self.base_hash {
            return Err(SessionError::BaseRevisionConflict {
                expected: expected_base_hash.to_hex(),
                actual: self.base_hash.to_hex(),
            });
        }
        if matches!(
            self.state,
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. } | ArtifactDirty::DirtyText
        ) {
            return Err(SessionError::InvalidState(
                "source proposals require a valid canvas edition".into(),
            ));
        }
        self.synchronize_retained_source_with_scene()?;

        let candidate_hash = hash_file_set(&[("candidate.fnx", source.as_bytes())]);
        let theirs = self.candidate_node_map(source, candidate_hash)?;
        let ours = self.projected_node_map()?;
        let merged = merge_artifact(&self.base_nodes, &ours, &theirs);
        let review = MergeReview::new(
            self.id.clone(),
            self.working_generation,
            self.base_hash,
            candidate_hash,
            Arc::clone(&self.base_nodes),
            ours,
            theirs,
            merged,
        );
        if review.conflicts.is_empty() {
            let applied = self.apply_merge_review(&review)?;
            Ok(SourceProposalOutcome::Applied(applied))
        } else {
            Ok(SourceProposalOutcome::Review(review))
        }
    }

    /// Atomically install a fully resolved in-memory proposal review.
    pub fn apply_merge_review(
        &mut self,
        review: &MergeReview,
    ) -> Result<ProposalApplied, SessionError> {
        if review.artifact != self.id {
            return Err(SessionError::InvalidState(
                "merge review belongs to another artifact".into(),
            ));
        }
        if matches!(
            self.state,
            ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. } | ArtifactDirty::DirtyText
        ) {
            return Err(SessionError::InvalidState(
                "merge review requires a valid canvas edition".into(),
            ));
        }
        self.synchronize_retained_source_with_scene()?;
        if review.expected_base_hash != self.base_hash {
            return Err(SessionError::BaseRevisionConflict {
                expected: review.expected_base_hash.to_hex(),
                actual: self.base_hash.to_hex(),
            });
        }
        if review.expected_generation != self.working_generation {
            return Err(SessionError::GenerationConflict {
                expected: review.expected_generation,
                actual: self.working_generation,
            });
        }
        if !review.is_resolved() {
            return Err(SessionError::ReviewIncomplete);
        }
        let source_sync = self.install_working_node_map(&review.draft)?;
        Ok(ProposalApplied {
            generation: self.working_generation,
            source_sync,
        })
    }

    /// Save with TOCTOU (N4/N15). `project_root` is the workspace root.
    pub fn save(&mut self, project_root: &Path) -> Result<SaveResult, SessionError> {
        if matches!(self.state, ArtifactDirty::Conflict(_)) {
            return Err(SessionError::SaveBlocked(SaveBlocked::InConflict));
        }
        if matches!(self.state, ArtifactDirty::Invalid { .. }) {
            return Err(SessionError::SaveBlocked(SaveBlocked::Invalid));
        }
        if matches!(
            self.state,
            ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas
        ) {
            self.synchronize_retained_source_with_scene()?;
        }
        if matches!(self.state, ArtifactDirty::Clean) {
            return Ok(SaveResult::NoOp);
        }

        let mut bytes = self.project_to_files()?;
        let mut projected_hash = hash_pairs(&bytes);
        let mut projected_nodes =
            node_map_from_files(self.kind, &bytes, &self.fn_name, &self.ref_table)?;

        let design_dir = project_root.join(&self.meta.design_dir);
        let mut current = read_file_set(&design_dir, self.kind)?;
        let mut current_disk_hash = hash_pairs(&current);

        if current_disk_hash != self.disk_hash {
            // Disk moved — never write pre-TOCTOU bytes.
            match self.handle_disk_changed_dirty(project_root, &current, current_disk_hash)? {
                MergePathResult::EnteredConflict => {
                    return Err(SessionError::SaveBlocked(SaveBlocked::EnteredConflict));
                }
                MergePathResult::CleanMerged { now_clean: true } => {
                    return Ok(SaveResult::NoOp);
                }
                MergePathResult::CleanMerged { now_clean: false } => {
                    bytes = self.project_to_files()?;
                    projected_hash = hash_pairs(&bytes);
                    projected_nodes =
                        node_map_from_files(self.kind, &bytes, &self.fn_name, &self.ref_table)?;
                    current = read_file_set(&design_dir, self.kind)?;
                    current_disk_hash = hash_pairs(&current);
                    if current_disk_hash != self.disk_hash {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                }
                MergePathResult::Invalid => {
                    return Err(SessionError::SaveBlocked(SaveBlocked::Invalid));
                }
            }
        }

        if projected_hash == self.disk_hash {
            self.state = ArtifactDirty::Clean;
            self.base_hash = self.disk_hash;
            self.base_nodes = Arc::new(projected_nodes);
            self.ir_stale = false;
            self.text = None;
            return Ok(SaveResult::NoOp);
        }

        let written = write_artifact_files(&design_dir, &bytes)?;
        self.disk_hash = projected_hash;
        self.base_hash = projected_hash;
        self.base_nodes = Arc::new(projected_nodes);
        self.state = ArtifactDirty::Clean;
        self.ir_stale = false;
        self.text = None;
        Ok(SaveResult::Wrote {
            paths: written
                .into_iter()
                .map(|n| self.meta.design_dir.join(n))
                .collect(),
        })
    }

    /// Handle external disk change while dirty.
    pub fn handle_disk_changed_dirty(
        &mut self,
        _project_root: &Path,
        observed_files: &[(String, Vec<u8>)],
        h_obs: ContentHash,
    ) -> Result<MergePathResult, SessionError> {
        let origin = match &self.state {
            ArtifactDirty::DirtyCanvas => ConflictOrigin::Canvas,
            ArtifactDirty::DirtyText => ConflictOrigin::Text,
            _ => {
                return Err(SessionError::InvalidState(
                    "handle_disk_changed_dirty only for Dirty*".into(),
                ));
            }
        };

        // Deleted covers the partial case too: `read_file_set` skips missing
        // files silently, so a crashed writer or a mid-checkout git state can
        // leave the header behind while `page.fnx` / the ids sidecar are gone.
        // Disk no longer holds a loadable edition either way — route it to the
        // Deleted conflict (KeepOurs rescues the working copy, TakeTheirs
        // accepts the removal) instead of surfacing a raw "missing page.fnx"
        // load error out of `save`.
        let (fnx_name, ids_name, _) = file_names(self.kind);
        let unloadable = !observed_files.iter().any(|(name, _)| name == fnx_name)
            || !observed_files.iter().any(|(name, _)| name == ids_name);
        if unloadable {
            let ours = self.snapshot_ours_edition()?;
            let base = self.snapshot_base_edition();
            self.disk_hash = h_obs;
            self.state = ArtifactDirty::Conflict(ConflictState {
                base: Arc::new(base),
                ours: Arc::new(ours),
                theirs: EditionSide::Deleted,
                auto: MergePreview {
                    merged_nodes: Map::new(),
                    merged_header: Value::Null,
                    merged_ir: self.ir.clone(),
                    conflicts: vec!["(deleted)".into()],
                },
                origin,
            });
            return Ok(MergePathResult::EnteredConflict);
        }

        let theirs_map =
            node_map_from_files(self.kind, observed_files, &self.fn_name, &self.ref_table)?;
        let ours_map = match origin {
            ConflictOrigin::Canvas => self.projected_node_map()?,
            ConflictOrigin::Text => {
                let source = self
                    .text
                    .as_ref()
                    .ok_or_else(|| SessionError::other("DirtyText missing buffer during merge"))?;
                match parse_source_to_node_map(
                    source,
                    &self.ir,
                    &self.base_nodes.header,
                    &self.ref_table,
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        self.state = ArtifactDirty::Invalid {
                            error: e.to_string(),
                            last_good: Some(Box::new(self.scoped.clone())),
                        };
                        return Ok(MergePathResult::Invalid);
                    }
                }
            }
        };

        let preview_merge = merge_artifact(&self.base_nodes, &ours_map, &theirs_map);
        let merged_list: Vec<Value> = preview_merge.nodes.values().cloned().collect();
        let merged_ir = ArtifactIr::from_nodes(self.kind, self.fn_name.clone(), &merged_list)
            .unwrap_or_else(|_| self.ir.clone());
        let preview = MergePreview {
            merged_nodes: preview_merge.nodes,
            merged_header: preview_merge.header,
            merged_ir,
            conflicts: preview_merge.conflicts,
        };

        if preview.conflicts.is_empty() {
            // Clean auto-merge is clone-then-swap: an invalid merged edition
            // cannot advance disk/base hashes or partially replace live state.
            let mut scratch = self.clone();
            scratch.base_hash = h_obs;
            scratch.base_nodes = Arc::new(theirs_map);
            scratch.disk_hash = h_obs;
            scratch.install_merged_working(origin, &preview)?;
            scratch.scoped.doc.history = History::new();
            let now_hash = hash_pairs(&scratch.project_to_files()?);
            if now_hash == h_obs {
                scratch.state = ArtifactDirty::Clean;
                scratch.text = None;
            } else {
                scratch.state = match origin {
                    ConflictOrigin::Canvas => ArtifactDirty::DirtyCanvas,
                    ConflictOrigin::Text => ArtifactDirty::DirtyText,
                };
            }
            let now_clean = matches!(scratch.state, ArtifactDirty::Clean);
            *self = scratch;
            Ok(MergePathResult::CleanMerged { now_clean })
        } else {
            let ours = self.snapshot_ours_edition()?;
            let base = self.snapshot_base_edition();
            let theirs_edition = ArtifactEdition {
                file_hash: h_obs,
                ir: ArtifactIr::from_nodes(
                    self.kind,
                    self.fn_name.clone(),
                    &theirs_map.nodes.values().cloned().collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| self.ir.clone()),
                text: None,
                node_map: theirs_map,
            };
            self.disk_hash = h_obs;
            self.state = ArtifactDirty::Conflict(ConflictState {
                base: Arc::new(base),
                ours: Arc::new(ours),
                theirs: EditionSide::Present(Arc::new(theirs_edition)),
                auto: preview,
                origin,
            });
            Ok(MergePathResult::EnteredConflict)
        }
    }

    fn install_merged_working(
        &mut self,
        origin: ConflictOrigin,
        preview: &MergePreview,
    ) -> Result<(), SessionError> {
        let (ir, mut scoped) = materialize_from_node_map(
            self.kind,
            &preview.merged_nodes,
            &preview.merged_header,
            self.scoped.doc.id,
            self.scoped.doc.components.clone(),
            self.scoped.doc.variables.clone(),
            self.scoped.doc.active_modes.clone(),
            &self.fn_name,
        )?;
        let old_sel = self.scoped.doc.selection.clone();
        scoped.doc.selection = filter_selection(old_sel, &scoped.doc);
        let text = ir.print_with(&self.ref_table);
        let source =
            FnxSourceMirror::from_source_with(&text, ir.sidecar(), Arc::clone(&self.ref_table))?;

        if origin == ConflictOrigin::Text {
            self.last_valid_ir = Some(ir.clone());
            self.text = Some(text);
        }
        self.ir = ir;
        self.source = source;
        self.scoped = scoped;
        self.ir_stale = false;
        self.scene_stale = false;
        self.last_source_sync = SourceSync::Rebuilt {
            reason: SourceRebuildReason::MergeInstall,
        };
        Ok(())
    }

    fn snapshot_ours_edition(&self) -> Result<ArtifactEdition, SessionError> {
        let node_map = self.projected_node_map()?;
        let ir = match &self.state {
            ArtifactDirty::DirtyText => self
                .last_valid_ir
                .clone()
                .unwrap_or_else(|| self.ir.clone()),
            _ => {
                let (ir, _) = project_scene_to_node_map(
                    &self.scoped,
                    self.kind,
                    &self.fn_name,
                    node_map.header.clone(),
                )?;
                ir
            }
        };
        Ok(ArtifactEdition {
            file_hash: ContentHash::default(),
            ir,
            text: self.text.clone(),
            node_map,
        })
    }

    fn snapshot_base_edition(&self) -> ArtifactEdition {
        let list: Vec<Value> = self.base_nodes.nodes.values().cloned().collect();
        let ir = ArtifactIr::from_nodes(self.kind, self.fn_name.clone(), &list)
            .unwrap_or_else(|_| self.ir.clone());
        ArtifactEdition {
            file_hash: self.base_hash,
            ir,
            text: None,
            node_map: (*self.base_nodes).clone(),
        }
    }

    pub fn resolve_conflict(
        &mut self,
        resolution: ConflictResolution,
        project_id: fanta_doc::DocId,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    ) -> Result<ResolveOutcome, SessionError> {
        let ArtifactDirty::Conflict(conflict) = self.state.clone() else {
            return Err(SessionError::InvalidState("not in Conflict state".into()));
        };
        // The caller hands us the current component library + variable
        // registry; refresh the name↔id vocabulary before any resolution path
        // parses or prints (a rename while conflicted must not resurrect a
        // stale spelling).
        self.ref_table = Arc::new(crate::project::refs_ctx::build_ref_table(
            &components,
            &variables,
            self.ref_table.emit_names(),
        ));
        self.source.set_ref_table(Arc::clone(&self.ref_table));
        match resolution {
            ConflictResolution::KeepOurs => {
                self.state = match conflict.origin {
                    ConflictOrigin::Canvas => ArtifactDirty::DirtyCanvas,
                    ConflictOrigin::Text => ArtifactDirty::DirtyText,
                };
                Ok(ResolveOutcome::Continue)
            }
            ConflictResolution::TakeTheirs => match &conflict.theirs {
                EditionSide::Deleted => Ok(ResolveOutcome::CloseDeleted),
                EditionSide::Present(theirs) => {
                    self.install_edition(theirs, project_id, components, variables, active_modes)?;
                    self.base_hash = self.disk_hash;
                    self.base_nodes = Arc::new(theirs.node_map.clone());
                    self.scoped.doc.history = History::new();
                    self.state = ArtifactDirty::Clean;
                    self.text = None;
                    Ok(ResolveOutcome::Continue)
                }
            },
            ConflictResolution::AcceptAuto => {
                let EditionSide::Present(theirs) = &conflict.theirs else {
                    return Err(SessionError::other("AcceptAuto requires Present theirs"));
                };
                let preview = &conflict.auto;
                self.install_merged_working(conflict.origin, preview)?;
                self.base_hash = self.disk_hash;
                self.base_nodes = Arc::new(theirs.node_map.clone());
                self.scoped.doc.history = History::new();
                self.state = match conflict.origin {
                    ConflictOrigin::Canvas => ArtifactDirty::DirtyCanvas,
                    ConflictOrigin::Text => ArtifactDirty::DirtyText,
                };
                let now_hash = hash_pairs(&self.project_to_files()?);
                if now_hash == self.disk_hash {
                    self.state = ArtifactDirty::Clean;
                    self.text = None;
                } else {
                    self.state = match conflict.origin {
                        ConflictOrigin::Canvas => ArtifactDirty::DirtyCanvas,
                        ConflictOrigin::Text => ArtifactDirty::DirtyText,
                    };
                }
                Ok(ResolveOutcome::Continue)
            }
            ConflictResolution::Manual(ir) => {
                let nodes = ir.to_nodes()?;
                let mut map = Map::new();
                for n in &nodes {
                    if let Some(id) = n.get("id").and_then(Value::as_str) {
                        map.insert(id.to_owned(), n.clone());
                    }
                }
                let header = self.base_nodes.header.clone();
                let (ir2, scoped) = materialize_from_node_map(
                    self.kind,
                    &map,
                    &header,
                    project_id,
                    components,
                    variables,
                    active_modes,
                    &self.fn_name,
                )?;
                let text = ir2.print_with(&self.ref_table);
                let source = FnxSourceMirror::from_source_with(
                    &text,
                    ir2.sidecar(),
                    Arc::clone(&self.ref_table),
                )?;
                self.ir = ir2;
                self.source = source;
                self.scoped = scoped;
                if let EditionSide::Present(theirs) = &conflict.theirs {
                    self.base_nodes = Arc::new(theirs.node_map.clone());
                }
                self.base_hash = self.disk_hash;
                self.scoped.doc.history = History::new();
                self.state = ArtifactDirty::DirtyCanvas;
                self.ir_stale = false;
                self.scene_stale = false;
                self.last_source_sync = SourceSync::Rebuilt {
                    reason: SourceRebuildReason::MergeInstall,
                };
                Ok(ResolveOutcome::Continue)
            }
        }
    }

    fn install_edition(
        &mut self,
        edition: &ArtifactEdition,
        project_id: fanta_doc::DocId,
        components: ComponentLibrary,
        variables: VariableRegistry,
        active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    ) -> Result<(), SessionError> {
        let (ir, scoped) = materialize_from_node_map(
            self.kind,
            &edition.node_map.nodes,
            &edition.node_map.header,
            project_id,
            components,
            variables,
            active_modes,
            &self.fn_name,
        )?;
        let text = ir.print_with(&self.ref_table);
        let source =
            FnxSourceMirror::from_source_with(&text, ir.sidecar(), Arc::clone(&self.ref_table))?;
        self.ir = ir;
        self.source = source;
        self.scoped = scoped;
        self.ir_stale = false;
        self.scene_stale = false;
        self.last_source_sync = SourceSync::Rebuilt {
            reason: SourceRebuildReason::MergeInstall,
        };
        Ok(())
    }

    /// Sync variables replica from workspace (N13). `ref_table` is the
    /// workspace's rebuilt name↔id context for the new registry (built once
    /// per generation, shared across every open session); the retained source
    /// mirror receives the same `Arc` so future canvas patches spell
    /// references with the fresh vocabulary.
    pub fn sync_variables(
        &mut self,
        variables: VariableRegistry,
        active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
        generation: u64,
        ref_table: Arc<RefTable>,
    ) {
        self.scoped.doc.variables = variables;
        self.scoped.doc.active_modes = active_modes;
        self.vars_generation = generation;
        self.source.set_ref_table(Arc::clone(&ref_table));
        self.ref_table = ref_table;
    }

    fn capture_patch_targets(&self, transaction: &Transaction) -> BTreeMap<NodeId, NodePatchBase> {
        transaction
            .ops
            .iter()
            .filter_map(patch_target)
            .filter_map(|id| {
                let element = self.ir.element(&id.0.to_string())?.clone();
                let node = self.scoped.doc.scene.get(id)?;
                let value = serde_json::to_value(node).ok()?;
                Some((id, NodePatchBase { element, value }))
            })
            .collect()
    }

    fn candidate_node_map(
        &self,
        source: &str,
        candidate_hash: ContentHash,
    ) -> Result<NodeMapEdition, SessionError> {
        let base_values: Vec<Value> = self.base_nodes.nodes.values().cloned().collect();
        let base_ir = ArtifactIr::from_nodes(self.kind, self.fn_name.clone(), &base_values)?;
        let reconciled =
            deterministic_reconcile_with_hash(source, base_ir.sidecar(), candidate_hash)?;
        let nodes = fanta_fnx::decode_subtree_with(source, &reconciled, &self.ref_table)?;
        let mut map = Map::new();
        for node in nodes {
            let id = node
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| SessionError::other("candidate node is missing id"))?;
            map.insert(id.to_owned(), node);
        }
        Ok(NodeMapEdition::new(self.base_nodes.header.clone(), map))
    }

    fn install_working_node_map(
        &mut self,
        edition: &NodeMapEdition,
    ) -> Result<SourceSync, SessionError> {
        let ours = self.projected_node_map()?;
        let structural = node_map_structure_changed(&ours, edition);
        let mut previous = BTreeMap::new();
        let mut after = BTreeMap::new();
        if !structural {
            for (key, old_value) in &ours.nodes {
                let Some(new_value) = edition.nodes.get(key) else {
                    continue;
                };
                if old_value == new_value {
                    continue;
                }
                let id: NodeId = serde_json::from_value(
                    old_value
                        .get("id")
                        .cloned()
                        .ok_or_else(|| SessionError::other("working node is missing id"))?,
                )
                .map_err(|error| SessionError::other(error.to_string()))?;
                let element = self
                    .ir
                    .element(key)
                    .cloned()
                    .ok_or_else(|| SessionError::other(format!("working IR lost node {id}")))?;
                previous.insert(
                    id,
                    NodePatchBase {
                        element,
                        value: old_value.clone(),
                    },
                );
                after.insert(id, new_value.clone());
            }
        }

        let mut scratch = self.clone();
        let (canonical_ir, mut scoped) = materialize_from_node_map(
            self.kind,
            &edition.nodes,
            &edition.header,
            self.scoped.doc.id,
            self.scoped.doc.components.clone(),
            self.scoped.doc.variables.clone(),
            self.scoped.doc.active_modes.clone(),
            &self.fn_name,
        )?;
        scoped.doc.selection = filter_selection(self.scoped.doc.selection.clone(), &scoped.doc);
        scratch.scoped = scoped;
        let source_sync = if structural {
            let text = canonical_ir.print_with(&scratch.ref_table);
            scratch.source = FnxSourceMirror::from_source_with(
                &text,
                canonical_ir.sidecar(),
                Arc::clone(&scratch.ref_table),
            )?;
            scratch.ir = canonical_ir;
            scratch.ir_stale = false;
            SourceSync::Rebuilt {
                reason: SourceRebuildReason::StructuralOperation,
            }
        } else {
            scratch.patch_source_values(previous, after)?
        };
        scratch.scoped.doc.history = History::new();
        scratch.state = ArtifactDirty::DirtyCanvas;
        scratch.scene_stale = false;
        scratch.working_generation = self.working_generation.wrapping_add(1);
        scratch.last_source_sync = source_sync.clone();
        // The installed edition carries the proposal's authored spellings
        // (merge preserves unknown keys): re-derive the typo warnings.
        scratch.last_diagnostics = unknown_attribute_diagnostics(&node_map_values(&edition.nodes));
        // Scene and source both derive from `edition` at this point.
        scratch.mark_scene_synced();
        *self = scratch;
        Ok(source_sync)
    }

    fn patch_source_nodes(
        &mut self,
        previous: BTreeMap<NodeId, NodePatchBase>,
    ) -> Result<SourceSync, SessionError> {
        let mut after = BTreeMap::new();
        for id in previous.keys() {
            let Some(node) = self.scoped.doc.scene.get(*id) else {
                return self.rebuild_source_from_scene(SourceRebuildReason::PatchUnavailable);
            };
            let value = serde_json::to_value(node)
                .map_err(|error| SessionError::other(error.to_string()))?;
            after.insert(*id, value);
        }
        self.patch_source_values(previous, after)
    }

    /// Reconcile legacy/direct scene mutations with retained FNX before any
    /// operation observes or persists the working edition.
    ///
    /// Normal canvas edits already patch source through
    /// [`apply_transaction_atomic`](Self::apply_transaction_atomic). This
    /// boundary also covers history/tool paths that still mutate the public
    /// scoped document directly. Deserializing retained nodes through
    /// `CanvasNode` strips unknown future FNX attributes from the comparison
    /// baseline; absent from both delta snapshots, those lexical attributes
    /// remain untouched.
    pub(super) fn synchronize_retained_source_with_scene(
        &mut self,
    ) -> Result<SourceSync, SessionError> {
        if matches!(
            self.state,
            ArtifactDirty::DirtyText | ArtifactDirty::Conflict(_) | ArtifactDirty::Invalid { .. }
        ) {
            return Ok(SourceSync::Unchanged);
        }

        // Hot-path skip: an unchanged `(instance_id, revision)` pair proves the
        // scene is exactly the one we last reconciled, so both full-scene
        // projections below would only rediscover an empty delta. This is what
        // keeps a pointer-drag apply O(edit) instead of O(scene).
        if self.scene_in_sync() {
            #[cfg(debug_assertions)]
            {
                // Debug builds re-derive the delta and assert the watermark
                // told the truth — CI runs every session test through this.
                let scene = self.scene_node_map()?;
                let retained = self.retained_scene_projection()?;
                debug_assert!(
                    retained.nodes == scene.nodes,
                    "scene watermark claimed sync but projections differ"
                );
            }
            self.ir_stale = false;
            return Ok(SourceSync::Unchanged);
        }

        let scene = self.scene_node_map()?;
        let retained = self.retained_scene_projection()?;
        if retained.nodes == scene.nodes {
            self.ir_stale = false;
            self.mark_scene_synced();
            return Ok(SourceSync::Unchanged);
        }

        let source_sync = if node_map_structure_changed(&retained, &scene) {
            self.rebuild_source_from_scene(SourceRebuildReason::StaleProjection)?
        } else {
            let mut previous = BTreeMap::new();
            let mut after = BTreeMap::new();
            for (key, before) in &retained.nodes {
                let Some(next) = scene.nodes.get(key) else {
                    continue;
                };
                if before == next {
                    continue;
                }
                let id: NodeId = serde_json::from_value(
                    before
                        .get("id")
                        .cloned()
                        .ok_or_else(|| SessionError::other("retained node is missing id"))?,
                )
                .map_err(|error| SessionError::other(error.to_string()))?;
                let element =
                    self.ir.element(key).cloned().ok_or_else(|| {
                        SessionError::other(format!("retained IR lost node {id}"))
                    })?;
                previous.insert(
                    id,
                    NodePatchBase {
                        element,
                        value: before.clone(),
                    },
                );
                after.insert(id, next.clone());
            }
            self.patch_source_values(previous, after)?
        };

        self.state = ArtifactDirty::DirtyCanvas;
        self.working_generation = self.working_generation.wrapping_add(1);
        self.last_source_sync = source_sync.clone();
        self.mark_scene_synced();
        Ok(source_sync)
    }

    fn scene_node_map(&self) -> Result<NodeMapEdition, SessionError> {
        let (_, map) = project_scene_to_node_map(
            &self.scoped,
            self.kind,
            &self.fn_name,
            self.projected_canvas_header()?,
        )?;
        Ok(map)
    }

    fn retained_scene_projection(&self) -> Result<NodeMapEdition, SessionError> {
        let mut nodes = Map::new();
        for value in self.ir.to_nodes()? {
            let node: CanvasNode = serde_json::from_value(value)
                .map_err(|error| SessionError::DocAssemble(error.to_string()))?;
            let id = node.id;
            let mut normalized = serde_json::to_value(node)
                .map_err(|error| SessionError::other(error.to_string()))?;
            if self.kind == ArtifactKind::Page
                && id == self.scoped.root
                && let Some(object) = normalized.as_object_mut()
            {
                // Page dimensions belong to source/import fidelity. The
                // editable page scene intentionally strips them.
                object.remove("clip_size");
                object.remove("local_size");
            }
            nodes.insert(id.0.to_string(), normalized);
        }
        Ok(NodeMapEdition::new(self.projected_canvas_header()?, nodes))
    }

    fn patch_source_values(
        &mut self,
        previous: BTreeMap<NodeId, NodePatchBase>,
        mut after: BTreeMap<NodeId, Value>,
    ) -> Result<SourceSync, SessionError> {
        let mut patched = Vec::new();
        for (id, before) in previous {
            let Some(value) = after.remove(&id) else {
                return self.rebuild_source_from_scene(SourceRebuildReason::PatchUnavailable);
            };
            let key = id.0.to_string();
            if self
                .ir
                .patch_node_delta(&key, &before.value, &value)
                .is_err()
            {
                return self.rebuild_source_from_scene(SourceRebuildReason::PatchUnavailable);
            }
            let new_element = self
                .ir
                .element(&key)
                .cloned()
                .ok_or_else(|| SessionError::other(format!("patched IR lost node {id}")))?;
            match self
                .source
                .patch_element_delta(&key, &before.element, &new_element)
            {
                Ok(true) => patched.push(id),
                Ok(false) => {}
                Err(_) => {
                    return self.rebuild_source_from_scene(SourceRebuildReason::PatchUnavailable);
                }
            }
        }
        self.ir_stale = false;
        if patched.is_empty() {
            Ok(SourceSync::Unchanged)
        } else {
            Ok(SourceSync::PatchedNodes { nodes: patched })
        }
    }

    fn rebuild_source_from_scene(
        &mut self,
        reason: SourceRebuildReason,
    ) -> Result<SourceSync, SessionError> {
        let header = self.base_nodes.header.clone();
        let (ir, _map) = project_scene_to_node_map(&self.scoped, self.kind, &self.fn_name, header)?;
        let text = ir.print_with(&self.ref_table);
        let source =
            FnxSourceMirror::from_source_with(&text, ir.sidecar(), Arc::clone(&self.ref_table))?;
        self.ir = ir;
        self.source = source;
        self.ir_stale = false;
        Ok(SourceSync::Rebuilt { reason })
    }
}

#[derive(Debug, Clone)]
struct NodePatchBase {
    element: FnxElement,
    value: Value,
}

#[derive(Debug)]
pub enum MergePathResult {
    EnteredConflict,
    CleanMerged { now_clean: bool },
    Invalid,
}

#[derive(Debug)]
pub enum ResolveOutcome {
    Continue,
    CloseDeleted,
}

/// Load an artifact from disk into a Clean session. `emit_names` is the
/// workspace's layout-version policy (v4+): whether prints re-sugar component
/// and variable ids into names; name RESOLUTION on load is unconditional.
#[expect(
    clippy::too_many_arguments,
    reason = "session bootstrap collects the workspace's whole shared context; a struct would rename, not reduce, it"
)]
pub fn load_artifact_session(
    project_root: &Path,
    meta: &ArtifactMeta,
    project_id: fanta_doc::DocId,
    components: ComponentLibrary,
    variables: VariableRegistry,
    active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    workspace_generation: u64,
    emit_names: bool,
) -> Result<ArtifactSession, SessionError> {
    let design_dir = project_root.join(&meta.design_dir);
    let files = read_file_set(&design_dir, meta.kind)?;
    if files.is_empty() {
        return Err(SessionError::other(format!(
            "no files in {}",
            design_dir.display()
        )));
    }
    let ref_table = Arc::new(crate::project::refs_ctx::build_ref_table(
        &components,
        &variables,
        emit_names,
    ));
    let disk_hash = hash_pairs(&files);
    let fn_name = meta.slug.replace('-', "_");
    let (ir, node_map) = load_ir_and_map(meta.kind, &files, &fn_name, &ref_table)?;
    let source_text = file_bytes(&files, file_names(meta.kind).0)
        .ok_or_else(|| SessionError::other("artifact source disappeared while opening"))?;
    let source_text = std::str::from_utf8(&source_text)
        .map_err(|error| SessionError::other(error.to_string()))?;
    let source =
        FnxSourceMirror::from_source_with(source_text, ir.sidecar(), Arc::clone(&ref_table))?;
    let scoped = match meta.kind {
        ArtifactKind::Page => {
            materialize_page(&ir, project_id, components, variables, active_modes)?
        }
        ArtifactKind::Component => {
            let def = serde_json::from_value(node_map.header.clone()).unwrap_or_else(|_| {
                let root = scoped_root_guess(&ir);
                fanta_doc::ComponentDef::new(
                    match meta.id {
                        ArtifactId::Component(c) => c,
                        _ => fanta_doc::ComponentId::new(),
                    },
                    root,
                    meta.slug.clone(),
                )
            });
            materialize_component(&ir, project_id, def, variables, active_modes)?
        }
        ArtifactKind::Graphics => materialize_graphics(&ir, project_id, variables, active_modes)?,
        other => {
            return Err(SessionError::other(format!(
                "load not implemented for {}",
                other.label()
            )));
        }
    };
    let scene_instance = scoped.doc.scene.instance_id();
    let scene_revision = scoped.doc.scene.revision();
    // The disk source is authored: an unknown attribute persisted by an
    // earlier tool (or a plain editor save) surfaces the same warning a fresh
    // text commit would.
    let last_diagnostics = unknown_attribute_diagnostics(&node_map_values(&node_map.nodes));
    Ok(ArtifactSession {
        id: meta.id.clone(),
        kind: meta.kind,
        meta: meta.clone(),
        state: ArtifactDirty::Clean,
        ir,
        source,
        ir_stale: false,
        scene_stale: false,
        text: None,
        last_valid_ir: None,
        scoped,
        viewport: Viewport::default(),
        vars_generation: workspace_generation,
        base_hash: disk_hash,
        disk_hash,
        base_nodes: Arc::new(node_map),
        fn_name,
        ref_table,
        working_generation: 0,
        last_source_sync: SourceSync::Unchanged,
        last_diagnostics,
        // Open materialized the scene FROM the retained source; they are in
        // sync by construction, so the first canvas apply skips reconciliation.
        last_synced_scene: Some((scene_instance, scene_revision)),
    })
}

fn scoped_root_guess(ir: &ArtifactIr) -> fanta_doc::NodeId {
    ir.sidecar()
        .ids
        .first()
        .and_then(|e| e.id.parse().ok())
        .unwrap_or_else(fanta_doc::NodeId::new)
}

pub(crate) fn load_ir_and_map_pub(
    kind: ArtifactKind,
    files: &[(String, Vec<u8>)],
    fn_name: &str,
    refs: &RefTable,
) -> Result<(ArtifactIr, NodeMapEdition), SessionError> {
    load_ir_and_map(kind, files, fn_name, refs)
}

fn load_ir_and_map(
    kind: ArtifactKind,
    files: &[(String, Vec<u8>)],
    fn_name: &str,
    refs: &RefTable,
) -> Result<(ArtifactIr, NodeMapEdition), SessionError> {
    let (fnx_name, ids_name, header_name) = file_names(kind);
    let source = file_bytes(files, fnx_name)
        .ok_or_else(|| SessionError::other(format!("missing {fnx_name}")))?;
    let source = std::str::from_utf8(&source).map_err(|e| SessionError::other(e.to_string()))?;
    let sidecar: FnxSidecar = file_bytes(files, ids_name)
        .ok_or_else(|| SessionError::other(format!("missing {ids_name}")))
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| SessionError::other(e.to_string())))?;
    let header: Value = file_bytes(files, header_name)
        .map(|b| serde_json::from_slice(&b).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    let nodes = fanta_fnx::decode_subtree_with(source, &sidecar, refs)?;
    let ir = ArtifactIr::from_source_with(kind, fn_name, source, sidecar, refs)?;
    let mut map = Map::new();
    for n in &nodes {
        if let Some(id) = n.get("id").and_then(Value::as_str) {
            map.insert(id.to_owned(), n.clone());
        }
    }
    Ok((ir, NodeMapEdition::new(header, map)))
}

fn node_map_from_files(
    kind: ArtifactKind,
    files: &[(String, Vec<u8>)],
    fn_name: &str,
    refs: &RefTable,
) -> Result<NodeMapEdition, SessionError> {
    let (_, map) = load_ir_and_map(kind, files, fn_name, refs)?;
    Ok(map)
}

fn parse_source_to_node_map(
    source: &str,
    previous: &ArtifactIr,
    header: &Value,
    refs: &RefTable,
) -> Result<NodeMapEdition, SessionError> {
    let reconciled = deterministic_reconcile(source, previous.sidecar())?;
    let nodes = fanta_fnx::decode_subtree_with(source, &reconciled, refs)?;
    let mut map = Map::new();
    for n in nodes {
        if let Some(id) = n.get("id").and_then(Value::as_str) {
            map.insert(id.to_owned(), n);
        }
    }
    Ok(NodeMapEdition::new(header.clone(), map))
}

/// Directory-scoped write (N1): only files inside the design dir.
pub fn write_artifact_files(
    design_dir: &Path,
    files: &[(String, Vec<u8>)],
) -> Result<Vec<String>, SessionError> {
    fs::create_dir_all(design_dir)?;
    let mut written = Vec::new();
    for (name, bytes) in files {
        // Reject path escape.
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return Err(SessionError::other(format!("illegal file name {name}")));
        }
        let path = design_dir.join(name);
        atomic_write(&path, bytes)?;
        written.push(name.clone());
    }
    Ok(written)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SessionError> {
    let parent = path
        .parent()
        .ok_or_else(|| SessionError::other("no parent"))?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temporary.write_all(bytes)?;
    temporary.flush()?;
    temporary
        .persist(path)
        .map_err(|e| SessionError::Io(e.error))?;
    Ok(())
}

pub fn read_file_set(
    design_dir: &Path,
    kind: ArtifactKind,
) -> Result<Vec<(String, Vec<u8>)>, SessionError> {
    let names = artifact_file_names(kind);
    let mut out = Vec::new();
    for name in names {
        let path = design_dir.join(name);
        if path.is_file() {
            out.push(((*name).to_owned(), fs::read(path)?));
        }
    }
    Ok(out)
}

fn file_names(kind: ArtifactKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        ArtifactKind::Page => (PAGE_FNX, PAGE_IDS, PAGE_JSON),
        ArtifactKind::Component => (MASTER_FNX, MASTER_IDS, DEF_JSON),
        ArtifactKind::Graphics => ("graphics.fnx", "graphics.ids.json", "graphics.json"),
        _ => ("source.fnx", "source.ids.json", "header.json"),
    }
}

fn file_bytes(files: &[(String, Vec<u8>)], name: &str) -> Option<Vec<u8>> {
    files
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| b.clone())
}

/// The values of a node map as the owned slice the diagnostics pass takes.
fn node_map_values(nodes: &Map<String, Value>) -> Vec<Value> {
    nodes.values().cloned().collect()
}

fn hash_pairs(files: &[(String, Vec<u8>)]) -> ContentHash {
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    hash_file_set(&refs)
}

fn is_variable_op(op: &Operation) -> bool {
    artifact_op_impact(op) == ArtifactOpImpact::Workspace
}

fn is_structural_op(op: &Operation) -> bool {
    artifact_op_impact(op) == ArtifactOpImpact::Structure
}

fn patch_target(op: &Operation) -> Option<NodeId> {
    match op {
        Operation::SetActiveMode {
            scope: fanta_doc::ModeScope::Frame { node },
            ..
        } => Some(*node),
        _ => op.primary_target(),
    }
}

/// Exhaustive persistence/shape classification for the operation surface.
pub fn artifact_op_impact(op: &Operation) -> ArtifactOpImpact {
    use ArtifactOpImpact as Impact;
    use Operation::*;
    match op {
        CreateNode { .. }
        | DeleteSubtree { .. }
        | Reparent { .. }
        | SetIndex { .. }
        | CreateInstance { .. }
        | DetachInstance { .. } => Impact::Structure,

        SetTransform { .. }
        | SetName { .. }
        | SetMeta { .. }
        | SetOpacity { .. }
        | SetBlendMode { .. }
        | SetEffects { .. }
        | SetBlurs { .. }
        | SetFlags { .. }
        | SetLayoutChild { .. }
        | ReplaceData { .. }
        | SetInstanceOverride { .. }
        | SwapInstance { .. }
        | SetInstanceProp { .. }
        | BindProperty { .. }
        | UnbindProperty { .. }
        | AddReaction { .. }
        | RemoveReaction { .. }
        | SetReaction { .. }
        | SetActiveMode {
            scope: fanta_doc::ModeScope::Frame { .. },
            ..
        } => Impact::NodeAttributes,

        DefineComponent { .. }
        | DeleteComponent { .. }
        | SetComponentProps { .. }
        | DefineComponentSet { .. }
        | DeleteComponentSet { .. }
        | SetVariantMembership { .. }
        | SetComponentSet { .. } => Impact::ComponentHeader,

        CreateVariableCollection { .. }
        | DeleteVariableCollection { .. }
        | AddMode { .. }
        | RemoveMode { .. }
        | CreateVariable { .. }
        | DeleteVariable { .. }
        | SetVariableValue { .. }
        | RenameVariable { .. }
        | RenameVariableCollection { .. }
        | RenameMode { .. }
        | SetActiveMode {
            scope: fanta_doc::ModeScope::Doc,
            ..
        } => Impact::Workspace,

        CreateAnimationClip { .. }
        | DeleteAnimationClip { .. }
        | SetAnimationClipName { .. }
        | SetAnimationClipDuration { .. }
        | SetAnimationTrack { .. }
        | SetKeyframe { .. } => Impact::Motion,

        SetFlowStart { .. } => Impact::Flow,
    }
}

fn node_map_structure_changed(before: &NodeMapEdition, after: &NodeMapEdition) -> bool {
    if before.nodes.keys().ne(after.nodes.keys()) {
        return true;
    }
    before.nodes.iter().any(|(id, old)| {
        let Some(new) = after.nodes.get(id) else {
            return true;
        };
        old.get("parent") != new.get("parent") || old.get("index") != new.get("index")
    })
}

fn deterministic_reconcile(
    source: &str,
    previous: &FnxSidecar,
) -> Result<FnxSidecar, SessionError> {
    let hash = hash_file_set(&[("candidate.fnx", source.as_bytes())]);
    deterministic_reconcile_with_hash(source, previous, hash)
}

fn deterministic_reconcile_with_hash(
    source: &str,
    previous: &FnxSidecar,
    hash: ContentHash,
) -> Result<FnxSidecar, SessionError> {
    let mut seed_bytes = [0u8; 16];
    seed_bytes.copy_from_slice(&hash.0[..16]);
    let seed = u128::from_be_bytes(seed_bytes);
    let existing: std::collections::BTreeSet<String> =
        previous.ids.iter().map(|entry| entry.id.clone()).collect();
    let mut ordinal = 0u128;
    reconcile_sidecar(source, previous, || {
        loop {
            ordinal = ordinal.wrapping_add(1);
            let id = NodeId::from_u128(seed.wrapping_add(ordinal).wrapping_add(FANTA_SEED));
            let encoded = id.0.to_string();
            if !existing.contains(&encoded) {
                break encoded;
            }
        }
    })
    .map_err(SessionError::from)
}

fn filter_selection(sel: Selection, doc: &Doc) -> Selection {
    // Selection API may vary; keep as-is if nodes still exist is best-effort.
    let _ = doc;
    sel
}

// Seed constant for local id minting in reconcile during text commit.
const FANTA_SEED: u128 = 0xFA_00_A0_0001;

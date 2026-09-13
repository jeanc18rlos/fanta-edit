//! [`WorkspaceSession`] — project shell, open map, shared vars, fs routing.

use super::artifact::{
    ArtifactSession, MergePathResult, ResolveOutcome, load_artifact_session, read_file_set,
};
use super::catalog::ComponentCatalog;
use super::error::{SaveBlocked, SessionError};
use super::hash::{ContentHash, hash_file_set};
use super::types::{
    ArtifactDirty, ArtifactId, ArtifactMeta, ClosePolicy, ConflictResolution, FsEvent, SaveResult,
    SessionEvent, WorkspaceDirty,
};
use crate::project::layout::{
    ACTIVE_MODES_JSON, AUDIO_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, FANTA_JSON, GRAPHICS_DIR,
    GRAPHICS_JSON, METADATA_JSON, MOTION_DIR, PAGES_DIR, PROTOTYPES_DIR, ProjectManifest,
    SETS_JSON, VARIABLES_JSON, is_project_dir, read_json_file, read_json_or, read_manifest,
    sorted_entries, write_json_file,
};
use crate::project::read::{component_id_of_dir, page_id_of_dir};
use fanta_doc::{ComponentLibrary, DocId, Operation, SCHEMA_VERSION, VariableRegistry};
use fanta_fnx::ArtifactKind;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Shared workspace state: variables/modes + dirty flag (N16/N19).
#[derive(Debug, Clone)]
pub struct WorkspaceSharedState {
    pub variables: VariableRegistry,
    pub active_modes: BTreeMap<fanta_doc::VariableCollectionId, fanta_doc::ModeId>,
    pub metadata: Value,
    pub dirty: WorkspaceDirty,
    pub base_hash: ContentHash,
    pub disk_hash: ContentHash,
    pub base_json: Value,
}

impl Default for WorkspaceSharedState {
    fn default() -> Self {
        Self {
            variables: VariableRegistry::new(),
            active_modes: BTreeMap::new(),
            metadata: json!({
                "title": "Untitled",
                "created_at": 0,
                "modified_at": 0,
            }),
            dirty: WorkspaceDirty::Clean,
            base_hash: ContentHash::default(),
            disk_hash: ContentHash::default(),
            base_json: Value::Null,
        }
    }
}

/// Live editor session for one project directory.
#[derive(Debug)]
pub struct WorkspaceSession {
    pub root: PathBuf,
    pub project_id: DocId,
    pub schema_version: u32,
    pub manifest: ProjectManifest,
    pub shared: WorkspaceSharedState,
    pub workspace_generation: u64,
    pub components: ComponentCatalog,
    pub artifacts: BTreeMap<ArtifactId, ArtifactMeta>,
    pub open: BTreeMap<ArtifactId, ArtifactSession>,
    pub file_index: BTreeMap<ArtifactId, ContentHash>,
    /// Optional workspace.fnx + dependency graph.
    pub workspace_ir: super::workspace_fnx::WorkspaceIr,
    /// Motion dual-read index (singleton or artifact stubs).
    pub motion: super::motion::MotionIndex,
}

impl WorkspaceSession {
    /// Open a project: index pages/components + load shared vars. No page scenes.
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        let root = project_root.as_ref().canonicalize()?;
        if !is_project_dir(&root) {
            return Err(SessionError::NotAProject { path: root });
        }
        let manifest = read_manifest(&root)?;
        if manifest.schema_version > SCHEMA_VERSION {
            return Err(SessionError::Format(
                crate::error::FormatError::UnsupportedSchema {
                    found: manifest.schema_version,
                    supported: SCHEMA_VERSION,
                },
            ));
        }
        let project_id: DocId = manifest
            .project_id
            .parse()
            .map_err(|e| SessionError::other(format!("bad project_id: {e}")))?;

        let mut shared = WorkspaceSharedState::default();
        let doc_dir = root.join(DOC_DIR);
        if doc_dir.join(METADATA_JSON).is_file() {
            shared.metadata = read_json_file(&doc_dir.join(METADATA_JSON))?;
        }
        let variables_val = read_json_or(&doc_dir.join(VARIABLES_JSON), json!({}))?;
        shared.variables = serde_json::from_value(variables_val.clone())
            .unwrap_or_else(|_| VariableRegistry::new());
        let modes_val = read_json_or(&doc_dir.join(ACTIVE_MODES_JSON), json!({}))?;
        shared.active_modes = serde_json::from_value(modes_val.clone()).unwrap_or_default();
        // Hash *on-disk* bytes (not re-serialized) so TOCTOU compares apples-to-apples.
        let h = hash_workspace_shared_on_disk(&root)?;
        shared.disk_hash = h;
        shared.base_hash = h;
        shared.base_json = json!({
            "variables": variables_val,
            "active_modes": modes_val,
        });

        let mut components = ComponentCatalog::new();
        let mut artifacts = BTreeMap::new();
        let mut file_index = BTreeMap::new();

        // Index pages
        let pages_dir = root.join(PAGES_DIR);
        if pages_dir.is_dir() {
            for entry in sorted_entries(&pages_dir)? {
                if !entry.is_dir() {
                    continue;
                }
                let slug = entry
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_owned();
                if slug.starts_with('_') {
                    continue;
                }
                let Some(page_id) = page_id_of_dir(&entry) else {
                    continue;
                };
                let id = ArtifactId::Page(page_id);
                let design_dir = PathBuf::from(PAGES_DIR).join(&slug);
                let files = read_file_set(&entry, ArtifactKind::Page)?;
                let disk_hash = hash_file_set(
                    &files
                        .iter()
                        .map(|(n, b)| (n.as_str(), b.as_slice()))
                        .collect::<Vec<_>>(),
                );
                let meta = ArtifactMeta {
                    id: id.clone(),
                    kind: ArtifactKind::Page,
                    slug,
                    design_dir,
                    disk_hash,
                };
                file_index.insert(id.clone(), disk_hash);
                artifacts.insert(id, meta);
            }
        }

        // Index components
        let comp_dir = root.join(COMPONENTS_DIR);
        if comp_dir.is_dir() {
            let sets_path = comp_dir.join(SETS_JSON);
            if sets_path.is_file() {
                components.defs.sets = serde_json::from_slice(&std::fs::read(&sets_path)?)
                    .map_err(|error| {
                        SessionError::other(format!(
                            "{}: bad component sets: {error}",
                            sets_path.display()
                        ))
                    })?;
            }
            for entry in sorted_entries(&comp_dir)? {
                if !entry.is_dir() {
                    continue;
                }
                let slug = entry
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_owned();
                let Some(cid) = component_id_of_dir(&entry) else {
                    continue;
                };
                let def_path = entry.join(DEF_JSON);
                if def_path.is_file() {
                    if let Ok(def) = serde_json::from_slice::<fanta_doc::ComponentDef>(
                        &std::fs::read(&def_path)?,
                    ) {
                        components.insert_def(def, PathBuf::from(COMPONENTS_DIR).join(&slug));
                    }
                }
                let id = ArtifactId::Component(cid);
                let design_dir = PathBuf::from(COMPONENTS_DIR).join(&slug);
                let files = read_file_set(&entry, ArtifactKind::Component)?;
                let disk_hash = hash_file_set(
                    &files
                        .iter()
                        .map(|(n, b)| (n.as_str(), b.as_slice()))
                        .collect::<Vec<_>>(),
                );
                let meta = ArtifactMeta {
                    id: id.clone(),
                    kind: ArtifactKind::Component,
                    slug,
                    design_dir,
                    disk_hash,
                };
                file_index.insert(id.clone(), disk_hash);
                artifacts.insert(id, meta);
            }
        }

        // Index graphics/<slug>/
        let graphics_dir = root.join(GRAPHICS_DIR);
        if graphics_dir.is_dir() {
            for entry in sorted_entries(&graphics_dir)? {
                if !entry.is_dir() {
                    continue;
                }
                let slug = entry
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_owned();
                let header_path = entry.join(GRAPHICS_JSON);
                let gid = if header_path.is_file() {
                    let header: Value = read_json_file(&header_path)?;
                    header
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or(&slug)
                        .to_owned()
                } else {
                    slug.clone()
                };
                let id = ArtifactId::Graphics(gid);
                let design_dir = PathBuf::from(GRAPHICS_DIR).join(&slug);
                let files = read_file_set(&entry, ArtifactKind::Graphics)?;
                let disk_hash = hash_file_set(
                    &files
                        .iter()
                        .map(|(n, b)| (n.as_str(), b.as_slice()))
                        .collect::<Vec<_>>(),
                );
                let meta = ArtifactMeta {
                    id: id.clone(),
                    kind: ArtifactKind::Graphics,
                    slug,
                    design_dir,
                    disk_hash,
                };
                file_index.insert(id.clone(), disk_hash);
                artifacts.insert(id, meta);
            }
        }

        // Stub index for prototypes / motion / audio (headers only when present)
        index_stub_kind(
            &root,
            PROTOTYPES_DIR,
            ArtifactKind::Prototype,
            |slug| ArtifactId::Prototype(slug.to_owned()),
            &mut artifacts,
            &mut file_index,
        )?;
        index_stub_kind(
            &root,
            MOTION_DIR,
            ArtifactKind::Motion,
            |slug| ArtifactId::Motion(slug.to_owned()),
            &mut artifacts,
            &mut file_index,
        )?;
        index_stub_kind(
            &root,
            AUDIO_DIR,
            ArtifactKind::Audio,
            |slug| ArtifactId::Audio(slug.to_owned()),
            &mut artifacts,
            &mut file_index,
        )?;

        let workspace_ir = super::workspace_fnx::load_workspace_ir(&root)?;
        let motion = super::motion::read_motion_dual(&root)?;

        Ok(Self {
            root,
            project_id,
            schema_version: manifest.schema_version,
            manifest,
            shared,
            workspace_generation: 1,
            components,
            artifacts,
            open: BTreeMap::new(),
            file_index,
            workspace_ir,
            motion,
        })
    }

    pub fn open_artifact(&mut self, id: ArtifactId) -> Result<ArtifactId, SessionError> {
        if self.open.contains_key(&id) {
            return Ok(id);
        }
        let meta = self
            .artifacts
            .get(&id)
            .cloned()
            .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
        let session = load_artifact_session(
            &self.root,
            &meta,
            self.project_id,
            self.components.defs.clone(),
            self.shared.variables.clone(),
            self.shared.active_modes.clone(),
            self.workspace_generation,
            self.emit_names(),
        )?;
        self.open.insert(id.clone(), session);
        Ok(id)
    }

    pub fn close_artifact(
        &mut self,
        id: ArtifactId,
        policy: ClosePolicy,
    ) -> Result<(), SessionError> {
        let Some(session) = self.open.get(&id) else {
            return Ok(());
        };
        match policy {
            ClosePolicy::CancelIfDirty if session.state.is_dirty() => {
                return Err(SessionError::InvalidState("artifact is dirty".into()));
            }
            ClosePolicy::Save if session.state.is_dirty() => {
                self.save_artifact(id.clone())?;
            }
            ClosePolicy::Discard | ClosePolicy::Save | ClosePolicy::CancelIfDirty => {}
        }
        self.open.remove(&id);
        Ok(())
    }

    pub fn artifact(&self, id: &ArtifactId) -> Option<&ArtifactSession> {
        self.open.get(id)
    }

    pub fn artifact_mut(&mut self, id: &ArtifactId) -> Option<&mut ArtifactSession> {
        self.open.get_mut(id)
    }

    pub fn save_artifact(&mut self, id: ArtifactId) -> Result<SaveResult, SessionError> {
        let session = self.open.get_mut(&id).ok_or(SessionError::NotOpen)?;
        let result = session.save(&self.root)?;
        self.file_index.insert(id.clone(), session.disk_hash);
        if let Some(meta) = self.artifacts.get_mut(&id) {
            meta.disk_hash = session.disk_hash;
        }
        if let ArtifactId::Component(cid) = id
            && let Some(def) = session.scoped.doc.components.defs.get(&cid)
        {
            self.components.defs.defs.insert(cid, def.clone());
        }
        Ok(result)
    }

    pub fn save_workspace_shared(&mut self) -> Result<SaveResult, SessionError> {
        if self.shared.dirty == WorkspaceDirty::Clean {
            return Ok(SaveResult::NoOp);
        }
        if self.shared.dirty == WorkspaceDirty::Conflict {
            return Err(SessionError::SaveBlocked(SaveBlocked::InConflict));
        }
        let doc_dir = self.root.join(DOC_DIR);
        std::fs::create_dir_all(&doc_dir)?;
        let variables = serde_json::to_value(&self.shared.variables)
            .map_err(|e| SessionError::other(e.to_string()))?;
        let modes = serde_json::to_value(&self.shared.active_modes)
            .map_err(|e| SessionError::other(e.to_string()))?;
        // TOCTOU: compare against real on-disk file-set hash.
        let current_hash = hash_workspace_shared_on_disk(&self.root)?;
        if current_hash != self.shared.disk_hash {
            return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
        }
        write_json_file(&doc_dir.join(VARIABLES_JSON), &variables)?;
        write_json_file(&doc_dir.join(ACTIVE_MODES_JSON), &modes)?;
        write_json_file(&doc_dir.join(METADATA_JSON), &self.shared.metadata)?;
        let h = hash_workspace_shared_on_disk(&self.root)?;
        self.shared.disk_hash = h;
        self.shared.base_hash = h;
        self.shared.base_json = json!({ "variables": variables, "active_modes": modes });
        self.shared.dirty = WorkspaceDirty::Clean;
        Ok(SaveResult::Wrote {
            paths: vec![
                PathBuf::from(DOC_DIR).join(VARIABLES_JSON),
                PathBuf::from(DOC_DIR).join(ACTIVE_MODES_JSON),
            ],
        })
    }

    /// N17: Components → Pages → Workspace shared.
    pub fn save_all(&mut self) -> Result<Vec<(ArtifactId, SaveResult)>, SessionError> {
        let mut ids: Vec<ArtifactId> = self.open.keys().cloned().collect();
        ids.sort_by_key(|id| match id {
            ArtifactId::Component(_) => 0u8,
            ArtifactId::Page(_) | ArtifactId::Graphics(_) => 1,
            _ => 2,
        });
        let mut out = Vec::new();
        for id in ids {
            if self
                .open
                .get(&id)
                .is_some_and(|s| s.state.is_dirty() && !s.state.is_conflict())
            {
                let r = self.save_artifact(id.clone())?;
                out.push((id, r));
            }
        }
        if self.shared.dirty == WorkspaceDirty::DirtyShared {
            out.push((ArtifactId::Workspace, self.save_workspace_shared()?));
        }
        Ok(out)
    }

    pub fn apply_workspace_op(&mut self, op: Operation) -> Result<(), SessionError> {
        // Build a throwaway doc that holds only variables for apply.
        let mut doc = fanta_doc::Doc::new();
        doc.id = self.project_id;
        doc.variables = self.shared.variables.clone();
        doc.active_modes = self.shared.active_modes.clone();
        doc.apply(op)
            .map_err(|e| SessionError::other(format!("workspace op: {e}")))?;
        self.shared.variables = doc.variables;
        self.shared.active_modes = doc.active_modes;
        self.shared.dirty = WorkspaceDirty::DirtyShared;
        self.workspace_generation = self.workspace_generation.wrapping_add(1);
        self.sync_vars_to_open();
        Ok(())
    }

    pub fn edit_variables(&mut self) -> Result<(), SessionError> {
        self.shared.dirty = WorkspaceDirty::DirtyShared;
        Ok(())
    }

    fn sync_vars_to_open(&mut self) {
        let vars = self.shared.variables.clone();
        let modes = self.shared.active_modes.clone();
        let generation = self.workspace_generation;
        // One table per generation: every open session (and its retained
        // source mirror) shares the same Arc, so a variable rename updates
        // the `$Collection/Name` vocabulary everywhere at once.
        let ref_table = std::sync::Arc::new(crate::project::refs_ctx::build_ref_table(
            &self.components.defs,
            &vars,
            self.emit_names(),
        ));
        for session in self.open.values_mut() {
            session.sync_variables(
                vars.clone(),
                modes.clone(),
                generation,
                std::sync::Arc::clone(&ref_table),
            );
        }
    }

    /// Layout-version policy for name-based reference EMISSION: v4 is the
    /// project layout where `component="Button"` / `"$Collection/Name"`
    /// spellings became part of the on-disk vocabulary. Resolution on parse
    /// is unconditional; only printing is gated.
    fn emit_names(&self) -> bool {
        self.manifest.version >= 4
    }

    pub fn workspace_generation(&self) -> u64 {
        self.workspace_generation
    }

    /// Apply op on an open artifact (refuses var ops).
    pub fn apply(&mut self, id: &ArtifactId, op: Operation) -> Result<(), SessionError> {
        let session = self.open.get_mut(id).ok_or(SessionError::NotOpen)?;
        session.apply(op)
    }

    pub fn resolve_conflict(
        &mut self,
        id: &ArtifactId,
        resolution: ConflictResolution,
    ) -> Result<(), SessionError> {
        let session = self.open.get_mut(id).ok_or(SessionError::NotOpen)?;
        let outcome = session.resolve_conflict(
            resolution,
            self.project_id,
            self.components.defs.clone(),
            self.shared.variables.clone(),
            self.shared.active_modes.clone(),
        )?;
        if matches!(outcome, ResolveOutcome::CloseDeleted) {
            self.open.remove(id);
            self.artifacts.remove(id);
            self.file_index.remove(id);
        }
        Ok(())
    }

    /// Route a filesystem event (hash-gated).
    pub fn notify_fs_event(&mut self, event: FsEvent) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        match event {
            FsEvent::Modified { path } | FsEvent::Created { path } | FsEvent::Removed { path } => {
                if let Some(id) = self.artifact_id_for_path(&path) {
                    if let Err(e) = self.handle_artifact_fs(&id, &path, &mut events) {
                        events.push(SessionEvent::Invalidated {
                            id,
                            error: e.to_string(),
                        });
                    }
                } else if is_workspace_shared_path(&path) {
                    if let Err(e) = self.reload_workspace_shared_if_clean() {
                        events.push(SessionEvent::Invalidated {
                            id: ArtifactId::Workspace,
                            error: e.to_string(),
                        });
                    } else {
                        events.push(SessionEvent::WorkspaceGeneration {
                            generation: self.workspace_generation,
                        });
                    }
                }
            }
        }
        events
    }

    fn artifact_id_for_path(&self, path: &Path) -> Option<ArtifactId> {
        let normalized = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let rel = normalized
            .strip_prefix(&self.root)
            .or_else(|_| path.strip_prefix(&self.root))
            .ok()?;
        let mut comps = rel.components();
        let first = comps.next()?.as_os_str().to_str()?;
        let slug = comps.next()?.as_os_str().to_str()?;
        match first {
            "pages" => self
                .artifacts
                .values()
                .find(|m| m.kind == ArtifactKind::Page && m.slug == slug)
                .map(|m| m.id.clone()),
            "components" => self
                .artifacts
                .values()
                .find(|m| m.kind == ArtifactKind::Component && m.slug == slug)
                .map(|m| m.id.clone()),
            _ => None,
        }
    }

    fn handle_artifact_fs(
        &mut self,
        id: &ArtifactId,
        _path: &Path,
        events: &mut Vec<SessionEvent>,
    ) -> Result<(), SessionError> {
        let meta = self
            .artifacts
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
        let design_dir = self.root.join(&meta.design_dir);
        if !design_dir.is_dir() {
            // Removed
            if let Some(session) = self.open.get_mut(id) {
                if matches!(session.state, ArtifactDirty::Clean) {
                    self.open.remove(id);
                    self.artifacts.remove(id);
                    self.file_index.remove(id);
                    events.push(SessionEvent::Removed { id: id.clone() });
                } else {
                    let empty: Vec<(String, Vec<u8>)> = Vec::new();
                    let h = ContentHash::default();
                    match session.handle_disk_changed_dirty(&self.root, &empty, h)? {
                        MergePathResult::EnteredConflict => {
                            events.push(SessionEvent::EnteredConflict {
                                id: id.clone(),
                                conflicts: vec!["(deleted)".into()],
                            });
                        }
                        _ => {}
                    }
                }
            } else {
                self.artifacts.remove(id);
                self.file_index.remove(id);
            }
            return Ok(());
        }

        let files = read_file_set(&design_dir, meta.kind)?;
        let h = hash_file_set(
            &files
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect::<Vec<_>>(),
        );
        if self.file_index.get(id) == Some(&h) {
            return Ok(()); // no-op
        }

        // Read the layout policy before the mutable borrow of the open map.
        let emit_names = self.emit_names();
        if let Some(session) = self.open.get_mut(id) {
            if matches!(session.state, ArtifactDirty::Clean) {
                // Silent reload
                let reloaded = load_artifact_session(
                    &self.root,
                    &meta,
                    self.project_id,
                    self.components.defs.clone(),
                    self.shared.variables.clone(),
                    self.shared.active_modes.clone(),
                    self.workspace_generation,
                    emit_names,
                )?;
                let viewport = session.viewport;
                *session = reloaded;
                session.viewport = viewport;
                self.file_index.insert(id.clone(), h);
                if let Some(m) = self.artifacts.get_mut(id) {
                    m.disk_hash = h;
                }
                events.push(SessionEvent::Reloaded { id: id.clone() });
            } else if session.state.is_dirty() {
                match session.handle_disk_changed_dirty(&self.root, &files, h)? {
                    MergePathResult::EnteredConflict => {
                        let conflicts = match &session.state {
                            ArtifactDirty::Conflict(c) => c.auto.conflicts.clone(),
                            _ => Vec::new(),
                        };
                        events.push(SessionEvent::EnteredConflict {
                            id: id.clone(),
                            conflicts,
                        });
                    }
                    MergePathResult::CleanMerged { .. } => {
                        self.file_index.insert(id.clone(), session.disk_hash);
                        events.push(SessionEvent::Reloaded { id: id.clone() });
                    }
                    MergePathResult::Invalid => {
                        events.push(SessionEvent::Invalidated {
                            id: id.clone(),
                            error: "merge parse failed".into(),
                        });
                    }
                }
            }
        } else {
            self.file_index.insert(id.clone(), h);
            if let Some(m) = self.artifacts.get_mut(id) {
                m.disk_hash = h;
            }
        }
        Ok(())
    }

    fn reload_workspace_shared_if_clean(&mut self) -> Result<(), SessionError> {
        if self.shared.dirty != WorkspaceDirty::Clean {
            // DirtyShared merge is a follow-on; mark generation only.
            return Ok(());
        }
        let doc_dir = self.root.join(DOC_DIR);
        let variables_val = read_json_or(&doc_dir.join(VARIABLES_JSON), json!({}))?;
        let modes_val = read_json_or(&doc_dir.join(ACTIVE_MODES_JSON), json!({}))?;
        self.shared.variables = serde_json::from_value(variables_val.clone())
            .unwrap_or_else(|_| VariableRegistry::new());
        self.shared.active_modes = serde_json::from_value(modes_val.clone()).unwrap_or_default();
        let h = hash_workspace_shared_on_disk(&self.root)?;
        self.shared.disk_hash = h;
        self.shared.base_hash = h;
        self.shared.base_json = json!({ "variables": variables_val, "active_modes": modes_val });
        self.workspace_generation = self.workspace_generation.wrapping_add(1);
        self.sync_vars_to_open();
        Ok(())
    }

    /// Component library for render inputs.
    pub fn component_library(&self) -> &ComponentLibrary {
        &self.components.defs
    }

    pub fn list_pages(&self) -> Vec<ArtifactId> {
        self.artifacts
            .keys()
            .filter(|id| matches!(id, ArtifactId::Page(_)))
            .cloned()
            .collect()
    }

    pub fn list_graphics(&self) -> Vec<ArtifactId> {
        self.artifacts
            .keys()
            .filter(|id| matches!(id, ArtifactId::Graphics(_)))
            .cloned()
            .collect()
    }

    /// Load a component master into the catalog side scene (N18).
    pub fn ensure_master_loaded(
        &mut self,
        id: fanta_doc::ComponentId,
    ) -> Result<super::catalog::MasterRef<'_>, SessionError> {
        let design_dir = self
            .components
            .paths
            .get(&id)
            .cloned()
            .ok_or(SessionError::MasterNotInScope)?;
        if !self.components.master_scenes.contains_key(&id) {
            let abs = self.root.join(&design_dir);
            let files = read_file_set(&abs, ArtifactKind::Component)?;
            let fn_name = design_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Component")
                .replace('-', "_");
            let refs = crate::project::refs_ctx::build_ref_table(
                &self.components.defs,
                &self.shared.variables,
                self.emit_names(),
            );
            let (ir, _) = super::artifact::load_ir_and_map_pub(
                ArtifactKind::Component,
                &files,
                &fn_name,
                &refs,
            )?;
            let nodes = ir.to_nodes()?;
            let scene = super::graphics::load_master_scene_from_nodes(&nodes)?;
            self.components.master_scenes.insert(id, scene);
        }
        let def = self
            .components
            .defs
            .defs
            .get(&id)
            .ok_or(SessionError::MasterNotInScope)?;
        let scene = self
            .components
            .master_scenes
            .get(&id)
            .ok_or(SessionError::MasterNotInScope)?;
        Ok(super::catalog::MasterRef { def, scene })
    }

    /// Bake a component into the open Graphics artifact (no live Instance).
    pub fn import_component_as_recreation(
        &mut self,
        graphics: &ArtifactId,
        component: fanta_doc::ComponentId,
        parent: fanta_doc::NodeId,
    ) -> Result<fanta_doc::NodeId, SessionError> {
        self.ensure_master_loaded(component)?;
        let session = self.open.get_mut(graphics).ok_or(SessionError::NotOpen)?;
        super::graphics::import_component_as_recreation(
            session,
            &mut self.components,
            component,
            parent,
        )
    }

    /// Create a minimal graphics artifact on disk and index it.
    pub fn create_graphics_artifact(
        &mut self,
        slug: &str,
        name: &str,
    ) -> Result<ArtifactId, SessionError> {
        use fanta_doc::{CanvasNode, GroupNode, NodeData};
        use fanta_fnx::ArtifactIr;
        let root_id = fanta_doc::NodeId::new();
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.id = root_id;
        node.name = name.to_owned();
        let node_val =
            serde_json::to_value(&node).map_err(|e| SessionError::other(e.to_string()))?;
        let ir =
            ArtifactIr::from_nodes(ArtifactKind::Graphics, slug.replace('-', "_"), &[node_val])?;
        let design_dir = PathBuf::from(GRAPHICS_DIR).join(slug);
        let abs = self.root.join(&design_dir);
        std::fs::create_dir_all(&abs)?;
        let header = json!({
            "id": slug,
            "name": name,
            "order": 0,
        });
        let files = [
            ("graphics.fnx", ir.print().into_bytes()),
            ("graphics.ids.json", {
                let sidecar_value = serde_json::to_value(ir.sidecar())
                    .map_err(|e| SessionError::other(e.to_string()))?;
                crate::project::layout::json_bytes(&sidecar_value)?
            }),
            (
                "graphics.json",
                crate::project::layout::json_bytes(&header)?,
            ),
        ];
        let pairs: Vec<(String, Vec<u8>)> =
            files.into_iter().map(|(n, b)| (n.to_owned(), b)).collect();
        super::artifact::write_artifact_files(&abs, &pairs)?;
        let disk_hash = hash_file_set(
            &pairs
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect::<Vec<_>>(),
        );
        let id = ArtifactId::Graphics(slug.to_owned());
        let meta = ArtifactMeta {
            id: id.clone(),
            kind: ArtifactKind::Graphics,
            slug: slug.to_owned(),
            design_dir,
            disk_hash,
        };
        self.file_index.insert(id.clone(), disk_hash);
        self.artifacts.insert(id.clone(), meta);
        Ok(id)
    }
}

fn index_stub_kind(
    root: &Path,
    dir_name: &str,
    kind: ArtifactKind,
    make_id: impl Fn(&str) -> ArtifactId,
    artifacts: &mut BTreeMap<ArtifactId, ArtifactMeta>,
    file_index: &mut BTreeMap<ArtifactId, ContentHash>,
) -> Result<(), SessionError> {
    let dir = root.join(dir_name);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in sorted_entries(&dir)? {
        if !entry.is_dir() {
            continue;
        }
        let slug = entry
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_owned();
        let id = make_id(&slug);
        let design_dir = PathBuf::from(dir_name).join(&slug);
        // Stubs may only have a header JSON; hash whatever files exist.
        let files = read_file_set(&entry, kind).unwrap_or_default();
        let disk_hash = hash_file_set(
            &files
                .iter()
                .map(|(n, b)| (n.as_str(), b.as_slice()))
                .collect::<Vec<_>>(),
        );
        let meta = ArtifactMeta {
            id: id.clone(),
            kind,
            slug,
            design_dir,
            disk_hash,
        };
        file_index.insert(id.clone(), disk_hash);
        artifacts.insert(id, meta);
    }
    Ok(())
}

fn is_workspace_shared_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(VARIABLES_JSON)
        || s.contains(ACTIVE_MODES_JSON)
        || s.contains("workspace.fnx")
        || s.ends_with(FANTA_JSON)
}

/// Hash workspace shared files from disk in N16 order.
fn hash_workspace_shared_on_disk(root: &Path) -> Result<ContentHash, SessionError> {
    let mut pairs: Vec<(String, Vec<u8>)> = Vec::new();
    for name in [
        format!("{DOC_DIR}/{VARIABLES_JSON}"),
        format!("{DOC_DIR}/{ACTIVE_MODES_JSON}"),
    ] {
        let path = root.join(&name);
        if path.is_file() {
            pairs.push((name, std::fs::read(path)?));
        }
    }
    let ws = root.join("workspace.fnx");
    if ws.is_file() {
        pairs.push(("workspace.fnx".into(), std::fs::read(ws)?));
    }
    Ok(hash_file_set(
        &pairs
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect::<Vec<_>>(),
    ))
}

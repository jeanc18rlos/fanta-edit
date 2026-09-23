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
    ACTIVE_MODES_JSON, AUDIO_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, FANTA_JSON, FLOW_START_JSON,
    GRAPHICS_DIR, GRAPHICS_JSON, METADATA_JSON, MOTION_DIR, MOTION_JSON, PAGE_JSON, PAGES_DIR,
    PROTOTYPES_DIR, ProjectManifest, SETS_JSON, VARIABLES_JSON, id_from_key, is_project_dir,
    json_bytes, read_json_file, read_json_or, read_manifest, sorted_entries, write_json_file,
};
use crate::project::read::{component_id_of_dir, page_id_of_dir, select_design_winners};
use fanta_doc::{
    ComponentId, ComponentLibrary, DocId, Operation, SCHEMA_VERSION, VariableRegistry,
};
use fanta_fnx::{ArtifactKind, artifact_file_names};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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
    pub(super) manifest_disk_hash: [u8; 32],
    pub(super) asset_index_disk_hash: Option<[u8; 32]>,
    pub(super) workspace_fnx_disk_hash: Option<[u8; 32]>,
    pub(super) other_singleton_disk_hashes: BTreeMap<PathBuf, Option<[u8; 32]>>,
    pub shared: WorkspaceSharedState,
    pub workspace_generation: u64,
    pub components: ComponentCatalog,
    pub artifacts: BTreeMap<ArtifactId, ArtifactMeta>,
    pub open: BTreeMap<ArtifactId, ArtifactSession>,
    pub file_index: BTreeMap<ArtifactId, ContentHash>,
    file_hash_index: BTreeMap<ArtifactId, BTreeMap<String, [u8; 32]>>,
    /// Optional workspace.fnx + dependency graph.
    pub workspace_ir: super::workspace_fnx::WorkspaceIr,
    /// Motion dual-read index (singleton or artifact stubs).
    pub motion: super::motion::MotionIndex,
}

impl WorkspaceSession {
    /// Open a project: index pages/components + load shared vars. No page scenes.
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        let root = project_root.as_ref().canonicalize()?;
        crate::project::layout::with_project_read_lock(&root, || Self::open_locked(root.clone()))
    }

    fn open_locked(root: PathBuf) -> Result<Self, SessionError> {
        if !is_project_dir(&root) {
            return Err(SessionError::NotAProject { path: root });
        }
        let manifest = read_manifest(&root)?;
        let manifest_disk_hash = sha256_bytes(&std::fs::read(root.join(FANTA_JSON))?);
        let asset_index_disk_hash = file_sha256_if_present(
            &root
                .join(crate::project::layout::ASSETS_DIR)
                .join(crate::project::media::ASSET_INDEX_FILE),
        )?;
        let workspace_fnx_disk_hash = file_sha256_if_present(&root.join("workspace.fnx"))?;
        let other_singleton_disk_hashes = [
            PathBuf::from(DOC_DIR).join(MOTION_JSON),
            PathBuf::from(DOC_DIR).join(FLOW_START_JSON),
            PathBuf::from(COMPONENTS_DIR).join(SETS_JSON),
        ]
        .into_iter()
        .map(|relative| file_sha256_if_present(&root.join(&relative)).map(|hash| (relative, hash)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
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
        let mut file_hash_index = BTreeMap::new();

        // Index pages
        let pages_dir = root.join(PAGES_DIR);
        if pages_dir.is_dir() {
            let mut candidates = Vec::new();
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
                if !entry.join(PAGE_JSON).is_file() {
                    continue;
                }
                let header: Value = read_json_file(&entry.join(PAGE_JSON))?;
                let page_id = page_id_from_header(&entry, &header)?;
                let id_from_header = header.get("id").and_then(Value::as_str).is_some();
                candidates.push((page_id, id_from_header, entry, slug));
            }
            let ranked: Vec<_> = candidates
                .iter()
                .map(|(id, from_header, entry, _)| (*id, *from_header, entry.as_path()))
                .collect();
            let winners = select_design_winners(&ranked);
            for (index, (page_id, _, entry, slug)) in candidates.into_iter().enumerate() {
                if !winners.contains(&index) {
                    continue;
                }
                let id = ArtifactId::Page(page_id);
                let design_dir = PathBuf::from(PAGES_DIR).join(&slug);
                let files = read_indexed_file_set(&entry, ArtifactKind::Page)?;
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
                file_hash_index.insert(id.clone(), hash_files(&files));
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
            let mut candidates = Vec::new();
            for entry in sorted_entries(&comp_dir)? {
                if !entry.is_dir() {
                    continue;
                }
                let slug = entry
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_owned();
                let def_path = entry.join(DEF_JSON);
                if !def_path.is_file() {
                    continue;
                }
                let mut def_value: Value = read_json_file(&def_path)?;
                let id_from_header = def_value.get("id").and_then(Value::as_str).is_some();
                let cid = if let Some(id) = def_value.get("id").and_then(Value::as_str) {
                    id_from_key::<ComponentId>(id).map_err(|error| {
                        SessionError::other(format!("{}: {error}", def_path.display()))
                    })?
                } else {
                    slug.parse::<ComponentId>().map_err(|error| {
                        SessionError::other(format!(
                            "{} has no valid component id: {error}",
                            def_path.display()
                        ))
                    })?
                };
                if !id_from_header {
                    def_value["id"] = json!(cid);
                }
                let def: fanta_doc::ComponentDef =
                    serde_json::from_value(def_value).map_err(|error| {
                        SessionError::other(format!("{}: {error}", def_path.display()))
                    })?;
                candidates.push((cid, id_from_header, entry, slug, def));
            }
            let ranked: Vec<_> = candidates
                .iter()
                .map(|(id, from_header, entry, _, _)| (*id, *from_header, entry.as_path()))
                .collect();
            let winners = select_design_winners(&ranked);
            for (index, (cid, _, entry, slug, def)) in candidates.into_iter().enumerate() {
                if !winners.contains(&index) {
                    continue;
                }
                components.insert_def(def, PathBuf::from(COMPONENTS_DIR).join(&slug));
                let id = ArtifactId::Component(cid);
                let design_dir = PathBuf::from(COMPONENTS_DIR).join(&slug);
                let files = read_indexed_file_set(&entry, ArtifactKind::Component)?;
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
                file_hash_index.insert(id.clone(), hash_files(&files));
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
                let files = read_indexed_file_set(&entry, ArtifactKind::Graphics)?;
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
            manifest_disk_hash,
            asset_index_disk_hash,
            workspace_fnx_disk_hash,
            other_singleton_disk_hashes,
            shared,
            workspace_generation: 1,
            components,
            artifacts,
            open: BTreeMap::new(),
            file_index,
            file_hash_index,
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

    pub fn has_indexed_source(&self, id: &ArtifactId) -> bool {
        let Some(files) = self.file_hash_index.get(id) else {
            return false;
        };
        let (source, ids) = match id {
            ArtifactId::Page(_) => (
                crate::project::layout::PAGE_FNX,
                crate::project::layout::PAGE_IDS,
            ),
            ArtifactId::Component(_) => (
                crate::project::layout::MASTER_FNX,
                crate::project::layout::MASTER_IDS,
            ),
            _ => return false,
        };
        files.contains_key(source) && files.contains_key(ids)
    }

    /// Source and identity bytes for the open page/component sessions. The
    /// caller should first adopt changed subtrees from the whole document;
    /// unchanged sessions retain their original source trivia automatically.
    pub fn validated_source_overrides(
        &mut self,
    ) -> Result<BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>, SessionError> {
        let mut overrides = BTreeMap::new();
        for session in self.open.values_mut() {
            let (source_name, ids_name) = match session.kind {
                ArtifactKind::Page => (
                    crate::project::layout::PAGE_FNX,
                    crate::project::layout::PAGE_IDS,
                ),
                ArtifactKind::Component => (
                    crate::project::layout::MASTER_FNX,
                    crate::project::layout::MASTER_IDS,
                ),
                _ => continue,
            };
            if !matches!(
                session.state,
                ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas
            ) {
                return Err(SessionError::InvalidState(format!(
                    "{} has uncommitted source or a conflict",
                    session.id.debug_label()
                )));
            }
            session.synchronize_retained_source_with_scene()?;
            let files = session.project_to_files()?;
            let source = files
                .iter()
                .find(|(name, _)| name == source_name)
                .map(|(_, bytes)| bytes.clone())
                .ok_or_else(|| SessionError::other("source projection is missing FNX"))?;
            let ids = files
                .iter()
                .find(|(name, _)| name == ids_name)
                .map(|(_, bytes)| bytes.clone())
                .ok_or_else(|| SessionError::other("source projection is missing IDs"))?;
            overrides.insert(session.meta.design_dir.clone(), (source, ids));
        }
        Ok(overrides)
    }

    /// The writer derives directory slugs from the current document, so an
    /// authored source follows a renamed page/component to its new directory.
    pub fn validated_source_overrides_for_document(
        &mut self,
        document: &fanta_doc::Doc,
    ) -> Result<BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>, SessionError> {
        for session in self.open.values() {
            if matches!(session.kind, ArtifactKind::Page | ArtifactKind::Component)
                && !matches!(
                    session.state,
                    ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas
                )
            {
                return Err(SessionError::InvalidState(format!(
                    "{} has uncommitted source or a conflict",
                    session.id.debug_label()
                )));
            }
        }
        let (pages, components) = crate::project::write::projected_design_dirs(document);
        let mut rekeyed = BTreeMap::new();
        for (id, meta) in &self.artifacts {
            let (projected_dir, source_name, ids_name) = match id {
                ArtifactId::Page(id) => (
                    pages.get(id),
                    crate::project::layout::PAGE_FNX,
                    crate::project::layout::PAGE_IDS,
                ),
                ArtifactId::Component(id) => (
                    components.get(id),
                    crate::project::layout::MASTER_FNX,
                    crate::project::layout::MASTER_IDS,
                ),
                _ => continue,
            };
            let Some(projected_dir) = projected_dir else {
                continue;
            };
            let projected = if let Some(session) = self.open.get_mut(id) {
                session.synchronize_retained_source_with_scene()?;
                if matches!(session.state, ArtifactDirty::DirtyCanvas) {
                    let files = session.project_to_files()?;
                    let source = files
                        .iter()
                        .find(|(name, _)| name == source_name)
                        .map(|(_, bytes)| bytes.clone())
                        .ok_or_else(|| SessionError::other("source projection is missing FNX"))?;
                    let ids = files
                        .iter()
                        .find(|(name, _)| name == ids_name)
                        .map(|(_, bytes)| bytes.clone())
                        .ok_or_else(|| SessionError::other("source projection is missing IDs"))?;
                    Some((source, ids))
                } else {
                    None
                }
            } else {
                None
            };
            let bytes = if let Some(projected) = projected {
                projected
            } else {
                let expected = self.file_hash_index.get(id).ok_or_else(|| {
                    SessionError::InvalidState(format!(
                        "{} has no reconciled file hashes",
                        id.debug_label()
                    ))
                })?;
                if !expected.contains_key(source_name) && !expected.contains_key(ids_name) {
                    continue;
                }
                let read_indexed = |name: &str| -> Result<Vec<u8>, SessionError> {
                    let bytes = std::fs::read(self.root.join(&meta.design_dir).join(name))?;
                    if expected.get(name).copied() != Some(sha256_bytes(&bytes)) {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                    Ok(bytes)
                };
                (read_indexed(source_name)?, read_indexed(ids_name)?)
            };
            if rekeyed.insert(projected_dir.clone(), bytes).is_some() {
                return Err(SessionError::other(
                    "two artifacts have the same projected path",
                ));
            }
        }
        Ok(rekeyed)
    }

    /// File-level old-content checks for a whole-document writer transaction.
    /// Uses indexed artifact hashes; the checked writer compares them with
    /// current disk bytes before a save can replace external edits.
    pub fn source_write_preconditions(
        &self,
        document: &fanta_doc::Doc,
    ) -> Result<BTreeMap<PathBuf, Option<[u8; 32]>>, SessionError> {
        let (pages, components) = crate::project::write::projected_design_dirs(document);
        let mut preconditions = BTreeMap::new();
        for (id, meta) in &self.artifacts {
            let projected_dir = match id {
                ArtifactId::Page(id) => pages.get(id),
                ArtifactId::Component(id) => components.get(id),
                _ => continue,
            };
            let expected = self.file_hash_index.get(id).ok_or_else(|| {
                SessionError::InvalidState(format!(
                    "{} has no reconciled file hashes",
                    id.debug_label()
                ))
            })?;
            for name in artifact_file_names(meta.kind) {
                let hash = expected.get(*name).copied();
                let path = meta.design_dir.join(name);
                if preconditions.insert(path, hash).is_some() {
                    return Err(SessionError::other(
                        "two artifacts occupy the same managed file path",
                    ));
                }
            }
            for (name, hash) in expected
                .iter()
                .filter(|(name, _)| name.starts_with("nodes/"))
            {
                preconditions.insert(meta.design_dir.join(name), Some(*hash));
            }
            if let Some(projected_dir) = projected_dir.filter(|dir| **dir != meta.design_dir) {
                let target = self.root.join(projected_dir);
                let occupant = if target.exists() {
                    let occupant = self.artifacts.values().find(|meta| {
                        meta.design_dir == *projected_dir
                            || matches!(
                                (
                                    self.root.join(&meta.design_dir).canonicalize(),
                                    target.canonicalize(),
                                ),
                                (Ok(previous), Ok(next)) if previous == next
                            )
                    });
                    if !occupant.is_some_and(|meta| self.file_hash_index.contains_key(&meta.id)) {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                    occupant
                } else {
                    None
                };
                for name in artifact_file_names(meta.kind) {
                    let path = projected_dir.join(name);
                    let hash = occupant
                        .and_then(|meta| self.file_hash_index.get(&meta.id))
                        .and_then(|files| files.get(*name))
                        .copied();
                    if let Some(previous) = preconditions.insert(path, hash)
                        && previous != hash
                    {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                }
            }
        }
        if hash_workspace_shared_on_disk(&self.root)? != self.shared.disk_hash {
            return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
        }
        let manifest_hash = file_sha256_if_present(&self.root.join(FANTA_JSON))?;
        let asset_index_path = PathBuf::from(crate::project::layout::ASSETS_DIR)
            .join(crate::project::media::ASSET_INDEX_FILE);
        let asset_index_hash = file_sha256_if_present(&self.root.join(&asset_index_path))?;
        if manifest_hash != Some(self.manifest_disk_hash)
            || asset_index_hash != self.asset_index_disk_hash
            || file_sha256_if_present(&self.root.join("workspace.fnx"))?
                != self.workspace_fnx_disk_hash
        {
            return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
        }
        for (relative, expected) in &self.other_singleton_disk_hashes {
            if file_sha256_if_present(&self.root.join(relative))? != *expected {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
        }
        for relative in [
            PathBuf::from(DOC_DIR).join(VARIABLES_JSON),
            PathBuf::from(DOC_DIR).join(ACTIVE_MODES_JSON),
            PathBuf::from(DOC_DIR).join(METADATA_JSON),
            PathBuf::from("workspace.fnx"),
            PathBuf::from(FANTA_JSON),
            asset_index_path,
        ] {
            let hash = file_sha256_if_present(&self.root.join(&relative))?;
            preconditions.insert(relative, hash);
        }
        preconditions.extend(self.other_singleton_disk_hashes.clone());
        Ok(preconditions)
    }

    pub fn asset_index_disk_hash(&self) -> Option<[u8; 32]> {
        self.asset_index_disk_hash
    }

    /// Accept one successful whole-project writer transaction after verifying
    /// that its on-disk files still equal the source and document being saved.
    /// A concurrent edit leaves every session dirty for normal reconciliation.
    pub fn accept_written_sources(
        &mut self,
        document: &fanta_doc::Doc,
        expected_asset_index_hash: [u8; 32],
        source_overrides: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
        written_hashes: &BTreeMap<PathBuf, [u8; 32]>,
    ) -> Result<(), SessionError> {
        let (pages, components) = crate::project::write::projected_design_dirs(document);
        let mut accepted = Vec::new();
        let mut removed = Vec::new();
        for (id, meta) in &self.artifacts {
            let (projected_dir, source_name, ids_name, header_name) = match id {
                ArtifactId::Page(page) => (
                    pages.get(page),
                    crate::project::layout::PAGE_FNX,
                    crate::project::layout::PAGE_IDS,
                    PAGE_JSON,
                ),
                ArtifactId::Component(component) => (
                    components.get(component),
                    crate::project::layout::MASTER_FNX,
                    crate::project::layout::MASTER_IDS,
                    DEF_JSON,
                ),
                _ => continue,
            };
            let Some(projected_dir) = projected_dir else {
                if !read_indexed_file_set(&self.root.join(&meta.design_dir), meta.kind)?.is_empty()
                {
                    return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                }
                removed.push(id.clone());
                continue;
            };
            let current = read_indexed_file_set(&self.root.join(projected_dir), meta.kind)?;
            let header = expected_artifact_header(document, id)?;
            let expected_header = json_bytes(&header)?;
            if current
                .iter()
                .find(|(file_name, _)| file_name == header_name)
                .map(|(_, bytes)| bytes.as_slice())
                != Some(expected_header.as_slice())
            {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
            if let Some((source, ids)) = source_overrides.get(projected_dir) {
                for (name, expected) in [(source_name, source), (ids_name, ids)] {
                    if current
                        .iter()
                        .find(|(file_name, _)| file_name == name)
                        .map(|(_, bytes)| bytes)
                        != Some(expected)
                    {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                }
            } else {
                for name in [source_name, ids_name] {
                    let path = projected_dir.join(name);
                    let expected = written_hashes.get(&path).copied().or_else(|| {
                        self.file_hash_index
                            .get(id)
                            .and_then(|files| files.get(name))
                            .copied()
                    });
                    let current_hash = current
                        .iter()
                        .find(|(file_name, _)| file_name == name)
                        .map(|(_, bytes)| sha256_bytes(bytes));
                    if current_hash != expected {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                }
            }
            for (name, bytes) in current
                .iter()
                .filter(|(name, _)| name.starts_with("nodes/"))
            {
                let expected = written_hashes
                    .get(&projected_dir.join(name))
                    .copied()
                    .or_else(|| {
                        self.file_hash_index
                            .get(id)
                            .and_then(|files| files.get(name))
                            .copied()
                    });
                if expected != Some(sha256_bytes(bytes)) {
                    return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                }
            }
            let disk_hash = hash_file_set(
                &current
                    .iter()
                    .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
                    .collect::<Vec<_>>(),
            );
            let mut meta = meta.clone();
            meta.design_dir = projected_dir.clone();
            meta.slug = projected_dir
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| SessionError::other("projected artifact slug is not UTF-8"))?
                .to_owned();
            meta.disk_hash = disk_hash;
            let node_map = if let Some(session) = self.open.get(id) {
                if matches!(session.state, ArtifactDirty::DirtyCanvas) {
                    let mut node_map = session.projected_node_map()?;
                    node_map.header = header;
                    Some(std::sync::Arc::new(node_map))
                } else if matches!(session.state, ArtifactDirty::Clean) {
                    if session.base_nodes.header == header {
                        Some(std::sync::Arc::clone(&session.base_nodes))
                    } else {
                        let mut node_map = (*session.base_nodes).clone();
                        node_map.header = header;
                        Some(std::sync::Arc::new(node_map))
                    }
                } else {
                    return Err(SessionError::InvalidState(format!(
                        "{} has uncommitted source or a conflict",
                        id.debug_label()
                    )));
                }
            } else {
                None
            };
            accepted.push((id.clone(), meta, disk_hash, node_map, hash_files(&current)));
        }

        let doc_dir = self.root.join(DOC_DIR);
        let variables = serde_json::to_value(&document.variables)
            .map_err(|error| SessionError::other(error.to_string()))?;
        let modes = serde_json::to_value(&document.active_modes)
            .map_err(|error| SessionError::other(error.to_string()))?;
        let metadata = serde_json::to_value(&document.metadata)
            .map_err(|error| SessionError::other(error.to_string()))?;
        for (name, value) in [
            (VARIABLES_JSON, &variables),
            (ACTIVE_MODES_JSON, &modes),
            (METADATA_JSON, &metadata),
        ] {
            if std::fs::read(doc_dir.join(name))? != json_bytes(value)? {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
        }
        let shared_hash = hash_workspace_shared_on_disk(&self.root)?;
        let expected_manifest = ProjectManifest::for_doc(document);
        let expected_manifest_bytes = json_bytes(
            &serde_json::to_value(&expected_manifest)
                .map_err(|error| SessionError::other(error.to_string()))?,
        )?;
        let manifest_bytes = std::fs::read(self.root.join(FANTA_JSON))?;
        let asset_index_bytes = std::fs::read(
            self.root
                .join(crate::project::layout::ASSETS_DIR)
                .join(crate::project::media::ASSET_INDEX_FILE),
        )?;
        if manifest_bytes != expected_manifest_bytes
            || sha256_bytes(&asset_index_bytes) != expected_asset_index_hash
            || file_sha256_if_present(&self.root.join("workspace.fnx"))?
                != self.workspace_fnx_disk_hash
        {
            return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
        }
        let motion = if document.motion.is_empty() {
            json!({})
        } else {
            serde_json::to_value(&document.motion)
                .map_err(|error| SessionError::other(error.to_string()))?
        };
        for (relative, value) in [
            (PathBuf::from(DOC_DIR).join(MOTION_JSON), motion),
            (
                PathBuf::from(DOC_DIR).join(FLOW_START_JSON),
                serde_json::to_value(document.flow_start)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
            (
                PathBuf::from(COMPONENTS_DIR).join(SETS_JSON),
                serde_json::to_value(&document.components.sets)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
        ] {
            if std::fs::read(self.root.join(&relative))? != json_bytes(&value)? {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
        }

        for id in removed {
            self.open.remove(&id);
            self.remove_artifact_index(&id);
        }
        for (id, meta, disk_hash, node_map, file_hashes) in accepted {
            if let Some(session) = self.open.get_mut(&id) {
                session.meta = meta.clone();
                session.disk_hash = disk_hash;
                session.base_hash = disk_hash;
                if let Some(node_map) = node_map {
                    session.base_nodes = node_map;
                }
                session.state = ArtifactDirty::Clean;
                session.ir_stale = false;
                session.scene_stale = false;
                session.text = None;
            }
            if let ArtifactId::Component(component_id) = id {
                self.components
                    .paths
                    .insert(component_id, meta.design_dir.clone());
            }
            self.file_index.insert(id.clone(), disk_hash);
            self.file_hash_index.insert(id.clone(), file_hashes);
            self.artifacts.insert(id, meta);
        }
        self.shared.variables = document.variables.clone();
        self.shared.active_modes = document.active_modes.clone();
        self.shared.metadata = metadata;
        self.shared.disk_hash = shared_hash;
        self.shared.base_hash = shared_hash;
        self.shared.base_json = json!({ "variables": variables, "active_modes": modes });
        self.shared.dirty = WorkspaceDirty::Clean;
        self.schema_version = expected_manifest.schema_version;
        self.manifest = expected_manifest;
        self.manifest_disk_hash = sha256_bytes(&manifest_bytes);
        self.asset_index_disk_hash = Some(expected_asset_index_hash);
        for (relative, hash) in &mut self.other_singleton_disk_hashes {
            *hash = file_sha256_if_present(&self.root.join(relative))?;
        }
        Ok(())
    }

    /// Update shared name-resolution context from a host-owned document before
    /// adopting changed artifacts and collecting their source buffers.
    pub fn adopt_document_shared(&mut self, document: &fanta_doc::Doc) -> bool {
        let variables_changed = self.shared.variables != document.variables
            || self.shared.active_modes != document.active_modes;
        let components_changed = self.components.defs != document.components;
        if !variables_changed && !components_changed {
            return false;
        }
        if variables_changed {
            self.shared.variables = document.variables.clone();
            self.shared.active_modes = document.active_modes.clone();
            self.shared.dirty = WorkspaceDirty::DirtyShared;
        }
        if components_changed {
            self.components.master_scenes.clear();
            self.components.defs = document.components.clone();
            let (_, component_paths) = crate::project::write::projected_design_dirs(document);
            self.components.paths = component_paths;
            for session in self.open.values_mut() {
                if matches!(session.kind, ArtifactKind::Page) {
                    session.scoped.doc.components = document.components.clone();
                }
            }
        }
        self.workspace_generation = self.workspace_generation.wrapping_add(1);
        self.sync_vars_to_open();
        true
    }

    pub fn save_artifact(&mut self, id: ArtifactId) -> Result<SaveResult, SessionError> {
        let session = self.open.get_mut(&id).ok_or(SessionError::NotOpen)?;
        let previous_disk_hash = session.disk_hash;
        let result = session.save(&self.root)?;
        if session.disk_hash != previous_disk_hash {
            self.file_hash_index
                .insert(id.clone(), hash_files(&session.project_to_files()?));
        }
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
            self.remove_artifact_index(id);
        } else if let Some(meta) = self.artifacts.get(id) {
            let files = read_indexed_file_set(&self.root.join(&meta.design_dir), meta.kind)?;
            self.file_hash_index.insert(id.clone(), hash_files(&files));
        }
        Ok(())
    }

    /// Route a filesystem event (hash-gated).
    pub fn notify_fs_event(&mut self, event: FsEvent) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        match event {
            FsEvent::Modified { path } | FsEvent::Created { path } | FsEvent::Removed { path } => {
                if artifact_location(&self.root, &path).is_some() {
                    if let Err(e) = self.refresh_artifact_path(&path, &mut events) {
                        events.push(SessionEvent::Invalidated {
                            id: self
                                .artifact_id_for_path(&path)
                                .unwrap_or(ArtifactId::Workspace),
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
        let (_, _, design_dir) = artifact_location(&self.root, path)?;
        self.artifacts
            .values()
            .find(|meta| meta.design_dir == design_dir)
            .map(|meta| meta.id.clone())
    }

    fn refresh_artifact_path(
        &mut self,
        path: &Path,
        events: &mut Vec<SessionEvent>,
    ) -> Result<(), SessionError> {
        let Some((kind, _, design_dir)) = artifact_location(&self.root, path) else {
            return Ok(());
        };
        let old_id = self.artifact_id_for_path(path);
        let disk_meta = artifact_meta_at(&self.root, &design_dir, kind)?;

        if let Some(meta) = disk_meta {
            if let Some(old_id) = old_id.filter(|old_id| *old_id != meta.id) {
                if let Some(session) = self.open.get_mut(&old_id) {
                    if session.state.is_dirty() {
                        session.state = ArtifactDirty::Invalid {
                            error: "artifact header now names a different identity".into(),
                            last_good: Some(Box::new(session.scoped.clone())),
                        };
                        events.push(SessionEvent::Invalidated {
                            id: old_id.clone(),
                            error: "artifact header now names a different identity".into(),
                        });
                    } else {
                        self.open.remove(&old_id);
                    }
                }
                self.remove_artifact_index(&old_id);
                events.push(SessionEvent::Removed { id: old_id });
            }
            let id = meta.id.clone();
            let known = self.artifacts.get(&id).cloned();
            let relocated = known
                .as_ref()
                .is_some_and(|known| known.design_dir != meta.design_dir);
            if known.is_none() {
                let files = read_indexed_file_set(&self.root.join(&meta.design_dir), meta.kind)?;
                self.file_index.insert(id.clone(), meta.disk_hash);
                self.file_hash_index.insert(id.clone(), hash_files(&files));
                self.artifacts.insert(id.clone(), meta.clone());
                self.refresh_component_def(&meta)?;
                events.push(SessionEvent::Created { id });
                return Ok(());
            }
            if relocated {
                if let Some(session) = self.open.get_mut(&id) {
                    session.meta = meta.clone();
                }
                self.artifacts.insert(id.clone(), meta.clone());
            }
            self.refresh_component_def(&meta)?;
            self.handle_artifact_fs(&id, path, events)?;
            if relocated && !events.iter().any(|event| matches!(event, SessionEvent::Reloaded { id: changed } if changed == &id)) {
                events.push(SessionEvent::Reloaded { id });
            }
            return Ok(());
        }

        if let Some(id) = old_id {
            if let Some(relocated) = find_artifact_by_id(&self.root, &id)? {
                if let Some(session) = self.open.get_mut(&id) {
                    session.meta = relocated.clone();
                }
                self.artifacts.insert(id.clone(), relocated.clone());
                self.refresh_component_def(&relocated)?;
                self.handle_artifact_fs(&id, path, events)?;
                if !events.iter().any(|event| matches!(event, SessionEvent::Reloaded { id: changed } if changed == &id)) {
                    events.push(SessionEvent::Reloaded { id });
                }
            } else {
                self.handle_artifact_fs(&id, path, events)?;
            }
        }
        Ok(())
    }

    fn remove_artifact_index(&mut self, id: &ArtifactId) {
        self.artifacts.remove(id);
        self.file_index.remove(id);
        self.file_hash_index.remove(id);
        if let ArtifactId::Component(component_id) = id {
            self.components.defs.defs.remove(component_id);
            self.components.paths.remove(component_id);
            self.components.master_scenes.remove(component_id);
            self.sync_vars_to_open();
        }
    }

    fn refresh_component_def(&mut self, meta: &ArtifactMeta) -> Result<(), SessionError> {
        let ArtifactId::Component(component_id) = meta.id else {
            return Ok(());
        };
        let path = self.root.join(&meta.design_dir).join(DEF_JSON);
        let bytes = std::fs::read(&path)?;
        let def: fanta_doc::ComponentDef = serde_json::from_slice(&bytes)
            .map_err(|error| SessionError::other(format!("{}: {error}", path.display())))?;
        if def.id != component_id {
            return Err(SessionError::other(format!(
                "{}: component id differs from directory identity",
                path.display()
            )));
        }
        let changed = self.components.defs.defs.get(&component_id) != Some(&def);
        self.components.insert_def(def, meta.design_dir.clone());
        if changed {
            self.components.master_scenes.remove(&component_id);
            self.sync_vars_to_open();
        }
        Ok(())
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
        if !design_dir.is_dir() || read_indexed_file_set(&design_dir, meta.kind)?.is_empty() {
            // Removed
            if let Some(session) = self.open.get_mut(id) {
                if matches!(session.state, ArtifactDirty::Clean) {
                    self.open.remove(id);
                    self.remove_artifact_index(id);
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
                self.remove_artifact_index(id);
            }
            return Ok(());
        }

        let files = read_indexed_file_set(&design_dir, meta.kind)?;
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
        self.file_hash_index.insert(id.clone(), hash_files(&files));
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
        self.shared.metadata =
            read_json_or(&doc_dir.join(METADATA_JSON), self.shared.metadata.clone())?;
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
            let files = read_indexed_file_set(&abs, ArtifactKind::Component)?;
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
        self.file_hash_index.insert(id.clone(), hash_files(&pairs));
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
        let files = read_indexed_file_set(&entry, kind).unwrap_or_default();
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

fn artifact_location(root: &Path, path: &Path) -> Option<(ArtifactKind, String, PathBuf)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let normalized = if absolute.starts_with(root) {
        absolute
    } else {
        let ancestor = absolute.ancestors().find_map(|ancestor| {
            ancestor
                .canonicalize()
                .ok()
                .map(|canonical| (ancestor, canonical))
        })?;
        ancestor.1.join(absolute.strip_prefix(ancestor.0).ok()?)
    };
    let mut parts = normalized.strip_prefix(root).ok()?.components();
    let directory = parts.next()?.as_os_str().to_str()?;
    let slug = parts.next()?.as_os_str().to_str()?.to_owned();
    let kind = match directory {
        PAGES_DIR => ArtifactKind::Page,
        COMPONENTS_DIR => ArtifactKind::Component,
        GRAPHICS_DIR => ArtifactKind::Graphics,
        _ => return None,
    };
    Some((kind, slug.clone(), PathBuf::from(directory).join(slug)))
}

fn page_id_from_header(
    design_dir: &Path,
    header: &Value,
) -> Result<fanta_doc::NodeId, SessionError> {
    let header_path = design_dir.join(PAGE_JSON);
    match header.get("id") {
        Some(Value::String(id)) => id
            .parse()
            .map_err(|error| SessionError::other(format!("{}: {error}", header_path.display()))),
        Some(_) => Err(SessionError::other(format!(
            "{} has a non-string page id",
            header_path.display()
        ))),
        None => page_id_of_dir(design_dir).ok_or_else(|| {
            SessionError::other(format!("{} has no valid page id", header_path.display()))
        }),
    }
}

fn artifact_meta_at(
    root: &Path,
    design_dir: &Path,
    kind: ArtifactKind,
) -> Result<Option<ArtifactMeta>, SessionError> {
    let absolute = root.join(design_dir);
    if !absolute.is_dir() {
        return Ok(None);
    }
    let slug = design_dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| SessionError::other("artifact directory has a non-UTF-8 name"))?
        .to_owned();
    let id = match kind {
        ArtifactKind::Page => {
            if slug.starts_with('_') || !absolute.join(PAGE_JSON).is_file() {
                return Ok(None);
            }
            let header = read_json_file(&absolute.join(PAGE_JSON))?;
            Some(ArtifactId::Page(page_id_from_header(&absolute, &header)?))
        }
        ArtifactKind::Component => {
            if !absolute.join(DEF_JSON).is_file() {
                return Ok(None);
            }
            component_id_of_dir(&absolute).map(ArtifactId::Component)
        }
        ArtifactKind::Graphics => {
            let header_path = absolute.join(GRAPHICS_JSON);
            if !header_path.is_file() {
                return Ok(None);
            }
            let header: Value = read_json_file(&header_path)?;
            Some(ArtifactId::Graphics(
                header
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or(&slug)
                    .to_owned(),
            ))
        }
        _ => return Ok(None),
    };
    let Some(id) = id else {
        return Err(SessionError::other(format!(
            "{} has no valid artifact id",
            absolute.display()
        )));
    };
    let files = read_indexed_file_set(&absolute, kind)?;
    let disk_hash = hash_file_set(
        &files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>(),
    );
    Ok(Some(ArtifactMeta {
        id,
        kind,
        slug,
        design_dir: design_dir.to_path_buf(),
        disk_hash,
    }))
}

fn find_artifact_by_id(root: &Path, id: &ArtifactId) -> Result<Option<ArtifactMeta>, SessionError> {
    let (directory, kind) = match id {
        ArtifactId::Page(_) => (PAGES_DIR, ArtifactKind::Page),
        ArtifactId::Component(_) => (COMPONENTS_DIR, ArtifactKind::Component),
        ArtifactId::Graphics(_) => (GRAPHICS_DIR, ArtifactKind::Graphics),
        _ => return Ok(None),
    };
    let parent = root.join(directory);
    if !parent.is_dir() {
        return Ok(None);
    }
    for entry in sorted_entries(&parent)? {
        if !entry.is_dir() {
            continue;
        }
        let Some(slug) = entry.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let relative = PathBuf::from(directory).join(slug);
        if let Some(meta) = artifact_meta_at(root, &relative, kind)?
            && &meta.id == id
        {
            return Ok(Some(meta));
        }
    }
    Ok(None)
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn read_indexed_file_set(
    design_dir: &Path,
    kind: ArtifactKind,
) -> Result<Vec<(String, Vec<u8>)>, SessionError> {
    let mut files = read_file_set(design_dir, kind)?;
    if !matches!(kind, ArtifactKind::Page | ArtifactKind::Component) {
        return Ok(files);
    }
    let (source_name, ids_name) = match kind {
        ArtifactKind::Page => (
            crate::project::layout::PAGE_FNX,
            crate::project::layout::PAGE_IDS,
        ),
        ArtifactKind::Component => (
            crate::project::layout::MASTER_FNX,
            crate::project::layout::MASTER_IDS,
        ),
        _ => return Ok(files),
    };
    if files.iter().any(|(name, _)| name == source_name)
        && files.iter().any(|(name, _)| name == ids_name)
    {
        return Ok(files);
    }
    let nodes_dir = design_dir.join("nodes");
    if nodes_dir.is_dir() {
        for path in sorted_entries(&nodes_dir)? {
            if !path.is_file() || path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_none_or(|stem| stem.parse::<fanta_doc::NodeId>().is_err())
            {
                continue;
            }
            files.push((format!("nodes/{name}"), std::fs::read(path)?));
        }
    }
    Ok(files)
}

fn hash_files(files: &[(String, Vec<u8>)]) -> BTreeMap<String, [u8; 32]> {
    files
        .iter()
        .map(|(name, bytes)| (name.clone(), sha256_bytes(bytes)))
        .collect()
}

fn file_sha256_if_present(path: &Path) -> Result<Option<[u8; 32]>, SessionError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(sha256_bytes(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn expected_artifact_header(
    document: &fanta_doc::Doc,
    id: &ArtifactId,
) -> Result<Value, SessionError> {
    match id {
        ArtifactId::Page(page) => {
            let order = document
                .pages
                .iter()
                .position(|candidate| candidate == page)
                .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
            let name = document
                .scene
                .get(*page)
                .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?
                .name
                .clone();
            Ok(json!({ "id": page.to_string(), "name": name, "order": order as u32 }))
        }
        ArtifactId::Component(component) => {
            let def = document
                .components
                .defs
                .get(component)
                .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
            serde_json::to_value(def).map_err(|error| SessionError::other(error.to_string()))
        }
        _ => Err(SessionError::InvalidState(
            "this artifact has no projected design header".into(),
        )),
    }
}

fn is_workspace_shared_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains(VARIABLES_JSON)
        || s.contains(ACTIVE_MODES_JSON)
        || s.contains(METADATA_JSON)
        || s.contains("workspace.fnx")
        || s.ends_with(FANTA_JSON)
}

/// Hash workspace shared files from disk in N16 order.
fn hash_workspace_shared_on_disk(root: &Path) -> Result<ContentHash, SessionError> {
    let mut pairs: Vec<(String, Vec<u8>)> = Vec::new();
    for name in [
        format!("{DOC_DIR}/{VARIABLES_JSON}"),
        format!("{DOC_DIR}/{ACTIVE_MODES_JSON}"),
        format!("{DOC_DIR}/{METADATA_JSON}"),
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

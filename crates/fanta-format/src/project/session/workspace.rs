//! [`WorkspaceSession`] — project shell, open map, shared vars, fs routing.

use super::artifact::{
    ArtifactSession, MergePathResult, ResolveOutcome, SourceIdentityWitness, load_artifact_session,
    read_file_set,
};
use super::catalog::ComponentCatalog;
use super::error::{SaveBlocked, SessionError};
use super::hash::{ContentHash, hash_file_set};
use super::types::{
    ArtifactDirty, ArtifactId, ArtifactMeta, ClosePolicy, ConflictResolution, FsEvent, SaveResult,
    SessionEvent, WorkspaceDirty,
};
use crate::project::component_dirs::{
    component_design_dir_of, component_dirs, merge_set_files, scan_component_dirs,
};
use crate::project::layout::SET_JSON;
use crate::project::layout::{
    ACTIVE_MODES_JSON, AUDIO_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, FANTA_JSON, FLOW_START_JSON,
    FLOWS_JSON, GRAPHICS_DIR, GRAPHICS_JSON, MASTER_IDS, METADATA_JSON, MOTION_DIR, MOTION_JSON,
    PAGE_JSON, PAGES_DIR, PRESENTATION_JSON, PROJECT_VERSION, PROTOTYPES_DIR, ProjectManifest,
    VARIABLES_JSON, id_from_key, is_project_dir, json_bytes, read_json_file, read_json_or,
    read_manifest, sorted_entries, write_json_file,
};
use crate::project::read::{component_id_of_dir, page_id_of_dir, select_design_winners};
use fanta_doc::{
    ComponentId, ComponentLibrary, DocId, Operation, SCHEMA_VERSION, VariableId, VariableRegistry,
};
use fanta_fnx::{ArtifactKind, RefTable, artifact_file_names};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

pub(super) fn decode_shared_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    value: &Value,
) -> Result<T, SessionError> {
    crate::project::read::validate_project_json_shape(path, value)?;
    serde_path_to_error::deserialize(value)
        .map_err(|error| SessionError::InvalidSource(format!("{}: {error}", path.display())))
}

#[derive(Debug)]
struct IndexedFileHashes {
    files: BTreeMap<String, [u8; 32]>,
    content_hash: ContentHash,
    order: Vec<String>,
    source_identity_verified: bool,
}

impl IndexedFileHashes {
    fn new(files: &[(String, Vec<u8>)], previous: Option<&Self>) -> Self {
        Self::with_source_identity(files, previous, None)
    }

    fn with_source_identity(
        files: &[(String, Vec<u8>)],
        previous: Option<&Self>,
        identity: Option<&SourceIdentityWitness>,
    ) -> Self {
        let file_hashes = hash_files(files);
        // The reconciliation base can lag behind disk during a conflict. Reuse
        // only an aggregate recorded alongside these exact per-file hashes.
        let unchanged = previous.filter(|previous| {
            previous.files == file_hashes
                && previous
                    .order
                    .iter()
                    .map(String::as_str)
                    .eq(files.iter().map(|(name, _)| name.as_str()))
        });
        let content_hash = unchanged
            .map(|previous| previous.content_hash)
            .unwrap_or_else(|| hash_file_set(files));
        let source_identity_verified = identity
            .is_some_and(|identity| identity.matches(&file_hashes))
            || unchanged
                .map(|previous| previous.source_identity_verified)
                .unwrap_or_else(|| design_source_identity_is_current(files));
        Self {
            files: file_hashes,
            content_hash,
            order: files.iter().map(|(name, _)| name.clone()).collect(),
            source_identity_verified,
        }
    }

    fn from_loaded_source(
        files: &[(String, Vec<u8>)],
        source_name: &str,
        sidecar_name: &str,
        identity: &crate::project::read::LoadedSourceIdentity,
    ) -> Result<Self, SessionError> {
        let hashes = hash_files(files);
        if hashes.get(source_name).copied() != identity.source_hash
            || hashes.get(sidecar_name).copied() != identity.sidecar_hash
        {
            return Err(SessionError::InvalidState(format!(
                "{source_name} or {sidecar_name} changed while the project was loading"
            )));
        }
        Ok(Self {
            files: hashes,
            content_hash: hash_file_set(files),
            order: files.iter().map(|(name, _)| name.clone()).collect(),
            source_identity_verified: identity.current,
        })
    }
}

struct PreparedArtifactSave {
    artifacts: BTreeMap<ArtifactId, IndexedFileHashes>,
    files: BTreeMap<PathBuf, Vec<u8>>,
    expected_disk: BTreeMap<PathBuf, Option<[u8; 32]>>,
}

#[derive(Debug)]
struct CachedReferenceTable {
    component_names: Vec<(ComponentId, String)>,
    variable_names: Vec<(VariableId, String, String)>,
    table: Arc<RefTable>,
}

impl CachedReferenceTable {
    fn matches(
        &self,
        components: &ComponentLibrary,
        variables: &VariableRegistry,
        emit_names: bool,
    ) -> bool {
        self.table.emit_names() == emit_names
            && self
                .component_names
                .iter()
                .map(|(id, name)| (*id, name.as_str()))
                .eq(components
                    .defs
                    .iter()
                    .map(|(id, def)| (*id, def.name.as_str())))
            && self
                .variable_names
                .iter()
                .map(|(id, collection, name)| (*id, collection.as_str(), name.as_str()))
                .eq(variables.variables.values().filter_map(|variable| {
                    let collection = variables.collections.get(&variable.collection)?;
                    Some((
                        variable.id,
                        collection.name.as_str(),
                        variable.name.as_str(),
                    ))
                }))
    }

    fn new(components: &ComponentLibrary, variables: &VariableRegistry, emit_names: bool) -> Self {
        Self {
            component_names: components
                .defs
                .iter()
                .map(|(id, def)| (*id, def.name.clone()))
                .collect(),
            variable_names: variables
                .variables
                .values()
                .filter_map(|variable| {
                    let collection = variables.collections.get(&variable.collection)?;
                    Some((variable.id, collection.name.clone(), variable.name.clone()))
                })
                .collect(),
            table: Arc::new(crate::project::refs_ctx::build_ref_table(
                components, variables, emit_names,
            )),
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
    reference_table_cache: Option<CachedReferenceTable>,
    source_reference_table: Arc<RefTable>,
    loaded_file_hashes: BTreeMap<PathBuf, [u8; 32]>,
    pub file_index: BTreeMap<ArtifactId, ContentHash>,
    file_hash_index: BTreeMap<ArtifactId, IndexedFileHashes>,
    /// Optional workspace.fnx + dependency graph.
    pub workspace_ir: super::workspace_fnx::WorkspaceIr,
    /// Motion dual-read index (singleton or artifact stubs).
    pub motion: super::motion::MotionIndex,
}

impl WorkspaceSession {
    /// Open a project: index pages/components + load shared vars. No page scenes.
    pub fn open(project_root: impl AsRef<Path>) -> Result<Self, SessionError> {
        let root = project_root.as_ref().canonicalize()?;
        crate::project::layout::with_project_read_lock(&root, || {
            Self::open_locked(root.clone(), None)
        })
    }

    /// Load the complete authoring document and its workspace index under one
    /// read lock, reusing validation of the exact source and sidecar bytes.
    pub fn open_project_document(
        project_root: impl AsRef<Path>,
    ) -> Result<(fanta_doc::Doc, BTreeMap<fanta_doc::AssetId, Vec<u8>>, Self), SessionError> {
        let root = project_root.as_ref().canonicalize()?;
        crate::project::layout::with_project_read_lock(&root, || {
            #[cfg(test)]
            let diagnostic_started = std::env::var_os("FANTA_PERF_PROJECT")
                .is_some()
                .then(std::time::Instant::now);
            let (document, assets, evidence) =
                crate::project::read::read_project_tree_with_source_identities(&root)?;
            #[cfg(test)]
            let reader_seconds = diagnostic_started.map(|started| started.elapsed().as_secs_f64());
            #[cfg(test)]
            let index_started = diagnostic_started.map(|_| std::time::Instant::now());
            let session = Self::open_locked(root.clone(), Some(&evidence))?;
            #[cfg(test)]
            let index_seconds = index_started.map(|started| started.elapsed().as_secs_f64());
            #[cfg(test)]
            let registry_started = diagnostic_started.map(|_| std::time::Instant::now());
            let metadata_changed = session.project_id != document.id
                || (session.schema_version == document.schema_version
                    && (session.components.defs != document.components
                        || session.shared.variables != document.variables
                        || session.shared.active_modes != document.active_modes));
            #[cfg(test)]
            if let Some(started) = diagnostic_started {
                eprintln!(
                    "{}",
                    json!({
                        "combined_load_stages": {
                            "reader_seconds": reader_seconds,
                            "index_seconds": index_seconds,
                            "registry_seconds": registry_started.map(|started| started.elapsed().as_secs_f64()),
                            "total_seconds": started.elapsed().as_secs_f64(),
                        }
                    })
                );
            }
            if metadata_changed {
                return Err(SessionError::InvalidState(
                    "project metadata changed while the project was loading".into(),
                ));
            }
            Ok((document, assets, session))
        })
    }

    fn open_locked(
        root: PathBuf,
        loaded: Option<&crate::project::read::LoadedProjectEvidence>,
    ) -> Result<Self, SessionError> {
        let source_identities = loaded.map(|evidence| &evidence.sources);
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
        let other_singleton_disk_hashes = singleton_disk_hashes(&root)?;
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
        let metadata = read_json_or(&doc_dir.join(METADATA_JSON), shared.metadata.clone())?;
        decode_shared_json::<fanta_doc::DocMetadata>(&doc_dir.join(METADATA_JSON), &metadata)?;
        let variables_val = read_json_or(&doc_dir.join(VARIABLES_JSON), json!({}))?;
        let variables = decode_shared_json(&doc_dir.join(VARIABLES_JSON), &variables_val)?;
        let modes_val = read_json_or(&doc_dir.join(ACTIVE_MODES_JSON), json!({}))?;
        let active_modes = decode_shared_json(&doc_dir.join(ACTIVE_MODES_JSON), &modes_val)?;
        // Hash *on-disk* bytes (not re-serialized) so TOCTOU compares apples-to-apples.
        let h = hash_workspace_shared_on_disk(&root)?;
        shared.metadata = metadata;
        shared.variables = variables;
        shared.active_modes = active_modes;
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
                if slug == crate::project::layout::LOOSE_DIR {
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
                let file_hashes = match source_identities {
                    Some(identities) => {
                        let identity = identities
                            .get(&entry.join(crate::project::layout::PAGE_FNX))
                            .ok_or_else(|| {
                                SessionError::InvalidState(
                                    "page selection changed while the project was loading".into(),
                                )
                            })?;
                        IndexedFileHashes::from_loaded_source(
                            &files,
                            crate::project::layout::PAGE_FNX,
                            crate::project::layout::PAGE_IDS,
                            identity,
                        )?
                    }
                    None => IndexedFileHashes::new(&files, None),
                };
                let disk_hash = file_hashes.content_hash;
                let meta = ArtifactMeta {
                    id: id.clone(),
                    kind: ArtifactKind::Page,
                    slug,
                    design_dir,
                    disk_hash,
                };
                file_hash_index.insert(id.clone(), file_hashes);
                file_index.insert(id.clone(), disk_hash);
                artifacts.insert(id, meta);
            }
        }

        // Index components: standalone ones, and v5 variants inside their
        // set's folder (v4 kept them flat, with every set in `sets.json`).
        let comp_dir = root.join(COMPONENTS_DIR);
        if comp_dir.is_dir() {
            let scanned = scan_component_dirs(&root)?;
            let set_files = scanned
                .set_files
                .iter()
                .map(|path| Ok((path.clone(), read_json_file(path)?)))
                .collect::<Result<Vec<_>, SessionError>>()?;
            components.defs.sets = serde_json::from_value(merge_set_files(&set_files)?)
                .map_err(|error| SessionError::other(format!("bad component sets: {error}")))?;
            let mut candidates = Vec::new();
            for entry in scanned.component_dirs {
                let slug = entry
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_owned();
                let design_dir = entry
                    .strip_prefix(&root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| PathBuf::from(COMPONENTS_DIR).join(&slug));
                let def_path = entry.join(DEF_JSON);
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
                candidates.push((cid, id_from_header, entry, slug, design_dir, def));
            }
            let ranked: Vec<_> = candidates
                .iter()
                .map(|(id, from_header, entry, _, _, _)| (*id, *from_header, entry.as_path()))
                .collect();
            let winners = select_design_winners(&ranked);
            let selected: Vec<_> = candidates
                .into_iter()
                .enumerate()
                .filter_map(|(index, candidate)| winners.contains(&index).then_some(candidate))
                .collect();
            let entries: Vec<_> = selected
                .iter()
                .map(|(_, _, entry, _, _, _)| entry.as_path())
                .collect();
            let indexed = index_initial_component_files(
                &entries,
                manifest.version == PROJECT_VERSION,
                source_identities,
            )?;
            for ((cid, _, _, slug, design_dir, def), file_hashes) in
                selected.into_iter().zip(indexed)
            {
                components.insert_def(def, design_dir.clone());
                let id = ArtifactId::Component(cid);
                let disk_hash = file_hashes.content_hash;
                let meta = ArtifactMeta {
                    id: id.clone(),
                    kind: ArtifactKind::Component,
                    slug,
                    design_dir,
                    disk_hash,
                };
                file_hash_index.insert(id.clone(), file_hashes);
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

        let reference_table_cache =
            CachedReferenceTable::new(&components.defs, &shared.variables, manifest.version >= 4);
        let source_reference_table = Arc::clone(&reference_table_cache.table);
        let loaded_file_hashes = loaded
            .map(|evidence| {
                evidence
                    .files
                    .iter()
                    .filter_map(|(path, hash)| hash.map(|hash| (path, hash)))
                    .map(|(path, hash)| {
                        path.strip_prefix(&root)
                            .map(|relative| (relative.to_path_buf(), hash))
                            .map_err(|error| SessionError::other(error.to_string()))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()
            })
            .transpose()?
            .unwrap_or_default();

        let session = Self {
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
            reference_table_cache: Some(reference_table_cache),
            source_reference_table,
            loaded_file_hashes,
            file_index,
            file_hash_index,
            workspace_ir,
            motion,
        };
        if let Some(evidence) = loaded {
            session.validate_loaded_project_evidence(evidence)?;
        }
        Ok(session)
    }

    fn validate_loaded_project_evidence(
        &self,
        loaded: &crate::project::read::LoadedProjectEvidence,
    ) -> Result<(), SessionError> {
        if self.manifest.version == PROJECT_VERSION {
            let pages: BTreeSet<_> = self
                .artifacts
                .keys()
                .filter_map(|id| match id {
                    ArtifactId::Page(page) => Some(*page),
                    _ => None,
                })
                .collect();
            if pages != loaded.pages.iter().copied().collect() {
                return Err(SessionError::InvalidState(
                    "the project page registry changed while the project was loading".into(),
                ));
            }
        }
        let headers: BTreeMap<_, _> = self
            .artifacts
            .iter()
            .filter_map(|(id, meta)| {
                matches!(id, ArtifactId::Page(_)).then(|| {
                    (
                        self.root.join(&meta.design_dir).join(PAGE_JSON),
                        self.file_hash_index
                            .get(id)
                            .and_then(|hashes| hashes.files.get(PAGE_JSON))
                            .copied(),
                    )
                })
            })
            .collect();
        for (path, expected) in &loaded.files {
            let actual = if path.file_name().is_some_and(|name| name == PAGE_JSON) {
                headers.get(path).copied().flatten()
            } else {
                file_sha256_if_present(path)?
            };
            if actual != *expected {
                return Err(SessionError::InvalidState(format!(
                    "{} changed while the project was loading",
                    path.display()
                )));
            }
        }
        Ok(())
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
        let ref_table = self.reference_table();
        let session = load_artifact_session(
            &self.root,
            &meta,
            self.project_id,
            &self.components.defs,
            self.shared.variables.clone(),
            self.shared.active_modes.clone(),
            self.workspace_generation,
            ref_table,
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
        let source = match id {
            ArtifactId::Page(_) => crate::project::layout::PAGE_FNX,
            ArtifactId::Component(_) => crate::project::layout::MASTER_FNX,
            _ => return false,
        };
        files.files.contains_key(source)
    }

    /// Consume exact file hashes from opening the project so initial watcher
    /// echoes can be checked without rebuilding the workspace index. Files
    /// absent from this snapshot still require normal disk reconciliation.
    pub fn take_loaded_file_hashes(&mut self) -> BTreeMap<PathBuf, [u8; 32]> {
        let mut hashes = std::mem::take(&mut self.loaded_file_hashes);
        if hashes.is_empty() {
            return hashes;
        }
        for (id, indexed) in &self.file_hash_index {
            if matches!(id, ArtifactId::Page(_) | ArtifactId::Component(_))
                && let Some(artifact) = self.artifacts.get(id)
            {
                hashes.extend(
                    indexed
                        .files
                        .iter()
                        .filter(|(name, _)| {
                            // Legacy node files have no full-reader byte
                            // witness. Migrated definitions likewise lack the
                            // current-schema semantic comparison at open.
                            matches!(
                                name.as_str(),
                                "page.fnx" | "page.ids.json" | "master.fnx" | "master.ids.json"
                            ) || (name.as_str() == DEF_JSON
                                && self.schema_version == SCHEMA_VERSION)
                        })
                        .map(|(name, hash)| (artifact.design_dir.join(name), *hash)),
                );
            }
        }
        for (path, hash) in &self.other_singleton_disk_hashes {
            if self.schema_version == SCHEMA_VERSION
                && path.starts_with(COMPONENTS_DIR)
                && let Some(hash) = hash
            {
                hashes.insert(path.clone(), *hash);
            }
        }
        hashes
    }

    /// Save ordinary edits without projecting or retaining untouched sources.
    /// Binary assets must be unchanged; their reconciled hashes are supplied by
    /// the host. Uncertain ownership, legacy sources, or project-wide changes
    /// return `None` so the host can use the whole-project writer.
    pub fn try_save_document_artifacts(
        &mut self,
        document: &fanta_doc::Doc,
        baseline: &fanta_doc::Doc,
        changed_artifacts: &BTreeSet<ArtifactId>,
        additional_disk_preconditions: &BTreeMap<PathBuf, Option<[u8; 32]>>,
    ) -> Result<Option<crate::project::write::WriteReport>, SessionError> {
        let Some(prepared) = self.prepare_document_artifact_save(
            document,
            baseline,
            changed_artifacts,
            additional_disk_preconditions,
        )?
        else {
            return Ok(None);
        };
        let (report, projected_hashes) = crate::project::write::write_project_artifacts_checked(
            &self.root,
            document,
            &prepared.files,
            &prepared.expected_disk,
        )?;
        self.accept_document_artifact_save(document, prepared, &projected_hashes)?;
        Ok(Some(report))
    }

    fn prepare_document_artifact_save(
        &mut self,
        document: &fanta_doc::Doc,
        baseline: &fanta_doc::Doc,
        changed_artifacts: &BTreeSet<ArtifactId>,
        additional_disk_preconditions: &BTreeMap<PathBuf, Option<[u8; 32]>>,
    ) -> Result<Option<PreparedArtifactSave>, SessionError> {
        if document.id != self.project_id
            || baseline.id != document.id
            || self.manifest != ProjectManifest::for_doc(document)
            || self.manifest != ProjectManifest::for_doc(baseline)
            || document.pages != baseline.pages
            || !components_match_for_artifact_save(
                &document.components,
                &baseline.components,
                &self.components.defs,
                changed_artifacts,
            )
            || document.variables != baseline.variables
            || document.variables != self.shared.variables
            || document.active_modes != baseline.active_modes
            || document.active_modes != self.shared.active_modes
            || document.asset_library != baseline.asset_library
            || document.motion != baseline.motion
            || document.flow_start != baseline.flow_start
            || document.flows != baseline.flows
            || document.presentation != baseline.presentation
            || self.asset_index_disk_hash.is_none()
            || changed_artifacts
                .iter()
                .any(|id| !self.artifacts.contains_key(id) || !self.file_index.contains_key(id))
        {
            return Ok(None);
        }
        let baseline_metadata = serde_json::to_value(&baseline.metadata)
            .map_err(|error| SessionError::other(error.to_string()))?;
        if baseline_metadata != self.shared.metadata {
            return Ok(None);
        }
        let (pages, components) = crate::project::write::projected_design_dirs(document);
        let design_count = self
            .artifacts
            .keys()
            .filter(|id| matches!(id, ArtifactId::Page(_) | ArtifactId::Component(_)))
            .count();
        if design_count != pages.len() + components.len() {
            return Ok(None);
        }
        for (id, meta) in &self.artifacts {
            let (directory, root, header_name) = match id {
                ArtifactId::Page(page) => (pages.get(page), Some(*page), PAGE_JSON),
                ArtifactId::Component(component) => (
                    components.get(component),
                    document
                        .components
                        .def(*component)
                        .map(|definition| definition.root),
                    DEF_JSON,
                ),
                _ => continue,
            };
            let Some(root) = root else {
                return Ok(None);
            };
            let Some(previous_root) = baseline.scene.get(root) else {
                return Ok(None);
            };
            let Some(current_root) = document.scene.get(root) else {
                return Ok(None);
            };
            if directory != Some(&meta.design_dir)
                || previous_root.parent != current_root.parent
                || previous_root.index != current_root.index
                || previous_root.name != current_root.name
            {
                return Ok(None);
            }
            let Some(indexed) = self.file_hash_index.get(id) else {
                return Ok(None);
            };
            if !indexed.source_identity_verified
                || indexed.files.len() != artifact_file_names(meta.kind).len()
                || indexed.files.get(header_name).copied()
                    != Some(sha256_bytes(&json_bytes(&expected_artifact_header(
                        baseline, id,
                    )?)?))
            {
                return Ok(None);
            }
        }
        if !artifact_save_scope_is_complete(document, baseline, changed_artifacts) {
            return Ok(None);
        }
        for (id, session) in &self.open {
            if !matches!(session.kind, ArtifactKind::Page | ArtifactKind::Component) {
                continue;
            }
            match session.state {
                ArtifactDirty::Clean => {}
                ArtifactDirty::DirtyCanvas if changed_artifacts.contains(id) => {}
                ArtifactDirty::DirtyCanvas => return Ok(None),
                _ => {
                    return Err(SessionError::InvalidState(format!(
                        "{} has uncommitted source or a conflict",
                        id.debug_label()
                    )));
                }
            }
        }
        let mut expected_disk = self.source_write_preconditions(document)?;
        for (path, hash) in additional_disk_preconditions {
            if let Some(previous) = expected_disk.insert(path.clone(), *hash)
                && previous != *hash
            {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
        }
        for id in changed_artifacts {
            let Some(indexed_hash) = self.file_index.get(id).copied() else {
                return Err(SessionError::ArtifactNotFound(id.debug_label()));
            };
            self.open_artifact(id.clone())?;
            if self.artifact(id).map(|artifact| artifact.disk_hash) != Some(indexed_hash) {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
        }
        self.adopt_document_artifacts(document, changed_artifacts)?;
        let mut artifacts = BTreeMap::new();
        let mut files = BTreeMap::new();
        for id in changed_artifacts {
            let session = self
                .open
                .get_mut(id)
                .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
            session.synchronize_retained_source_with_scene()?;
            let header = json_bytes(&expected_artifact_header(document, id)?)?;
            let (mut projected, identity) = session.project_files_with_identity()?;
            let names = artifact_file_names(session.kind);
            for (name, bytes) in &mut projected {
                if name == names.last().copied().unwrap_or("") {
                    *bytes = header.clone();
                } else if name.ends_with(".fnx") {
                    *bytes = crate::project::write::placed_source(&session.meta.design_dir, bytes);
                } else if name == MASTER_IDS
                    && let ArtifactId::Component(component) = id
                {
                    *bytes = component_sidecar_with_document_parent(
                        document,
                        *component,
                        std::mem::take(bytes),
                    )?;
                }
                files.insert(session.meta.design_dir.join(name), bytes.clone());
            }
            let indexed = IndexedFileHashes::with_source_identity(
                &projected,
                self.file_hash_index.get(id),
                identity.as_ref(),
            );
            if !indexed.source_identity_verified {
                return Err(SessionError::InvalidSource(format!(
                    "{} projected an incoherent source identity",
                    id.debug_label()
                )));
            }
            artifacts.insert(id.clone(), indexed);
        }
        Ok(Some(PreparedArtifactSave {
            artifacts,
            files,
            expected_disk,
        }))
    }

    fn accept_document_artifact_save(
        &mut self,
        document: &fanta_doc::Doc,
        prepared: PreparedArtifactSave,
        projected_hashes: &BTreeMap<PathBuf, [u8; 32]>,
    ) -> Result<(), SessionError> {
        let mut expected_disk = prepared.expected_disk;
        expected_disk.extend(
            projected_hashes
                .iter()
                .map(|(path, hash)| (path.clone(), Some(*hash))),
        );
        let mut accepted = Vec::new();
        for (id, file_hashes) in prepared.artifacts {
            let session = self
                .open
                .get(&id)
                .ok_or_else(|| SessionError::ArtifactNotFound(id.debug_label()))?;
            if file_hashes.files.iter().any(|(name, hash)| {
                projected_hashes.get(&session.meta.design_dir.join(name)) != Some(hash)
            }) {
                return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
            }
            let header = expected_artifact_header(document, &id)?;
            let node_map = match session.state {
                ArtifactDirty::DirtyCanvas => {
                    let mut node_map = session.projected_node_map()?;
                    node_map.header = header;
                    Arc::new(node_map)
                }
                ArtifactDirty::Clean if session.base_nodes.header == header => {
                    Arc::clone(&session.base_nodes)
                }
                ArtifactDirty::Clean => {
                    let mut node_map = (*session.base_nodes).clone();
                    node_map.header = header;
                    Arc::new(node_map)
                }
                _ => {
                    return Err(SessionError::InvalidState(format!(
                        "{} has uncommitted source or a conflict",
                        id.debug_label()
                    )));
                }
            };
            accepted.push((id, node_map, file_hashes));
        }
        let shared_hash = hash_workspace_shared_on_disk(&self.root)?;
        let metadata = serde_json::to_value(&document.metadata)
            .map_err(|error| SessionError::other(error.to_string()))?;
        crate::project::write::with_project_read_lock(&self.root, || {
            crate::project::write::validate_expected_disk_content(&self.root, &expected_disk)
        })
        .map_err(|error| match error {
            crate::error::FormatError::InvalidProjectTree(_) => {
                SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain)
            }
            error => SessionError::Format(error),
        })?;
        // A failed write or post-write validation must not advance component
        // revision caches or invalidate the master's accepted context.
        self.adopt_document_shared(document);
        for (id, node_map, file_hashes) in accepted {
            let disk_hash = file_hashes.content_hash;
            if let Some(session) = self.open.get_mut(&id) {
                session.meta.disk_hash = disk_hash;
                session.disk_hash = disk_hash;
                session.base_hash = disk_hash;
                session.base_nodes = node_map;
                session.state = ArtifactDirty::Clean;
                session.ir_stale = false;
                session.scene_stale = false;
                session.text = None;
            }
            if let Some(meta) = self.artifacts.get_mut(&id) {
                meta.disk_hash = disk_hash;
            }
            self.file_index.insert(id.clone(), disk_hash);
            self.file_hash_index.insert(id, file_hashes);
        }
        self.shared.metadata = metadata;
        self.shared.disk_hash = shared_hash;
        self.shared.base_hash = shared_hash;
        self.shared.base_json = json!({
            "variables": document.variables,
            "active_modes": document.active_modes,
        });
        self.shared.dirty = WorkspaceDirty::Clean;
        if let Some(hash) = projected_hashes.get(Path::new(FANTA_JSON)) {
            self.manifest_disk_hash = *hash;
        }
        for (path, hash) in &mut self.other_singleton_disk_hashes {
            if let Some(projected_hash) = projected_hashes.get(path) {
                *hash = Some(*projected_hash);
            }
        }
        Ok(())
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
        let reference_table = self.reference_table_for_document(document);
        let vocabulary_changed = !Arc::ptr_eq(&self.source_reference_table, &reference_table)
            && *self.source_reference_table != *reference_table;
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
                let expected = &self
                    .file_hash_index
                    .get(id)
                    .ok_or_else(|| {
                        SessionError::InvalidState(format!(
                            "{} has no reconciled file hashes",
                            id.debug_label()
                        ))
                    })?
                    .files;
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
                let source = read_indexed(source_name)?;
                let ids = if expected.contains_key(ids_name) {
                    Some(read_indexed(ids_name)?)
                } else {
                    if self.root.join(&meta.design_dir).join(ids_name).exists() {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                    None
                };
                let text = std::str::from_utf8(&source)
                    .map_err(|error| SessionError::other(error.to_string()))?;
                let coherent_sidecar = ids
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice::<fanta_fnx::FnxSidecar>(bytes).ok())
                    .filter(|sidecar| {
                        sidecar.ids.iter().all(|entry| {
                            id_from_key::<fanta_doc::NodeId>(&entry.id).is_ok()
                                && (entry.index.is_null() || entry.index.is_number())
                        }) && sidecar
                            .root_parent
                            .as_ref()
                            .is_none_or(|parent| id_from_key::<fanta_doc::NodeId>(parent).is_ok())
                    })
                    .filter(|sidecar| {
                        // Legacy identities, stale fingerprints and syntax the
                        // scanner cannot recognize need the existing repair path.
                        matches!(fanta_fnx::sidecar_matches_source(text, sidecar), Ok(true))
                    });
                let sidecar = match coherent_sidecar {
                    Some(sidecar) => sidecar,
                    None => crate::project::read::persisted_fnx_sidecar(text, ids.as_deref())?,
                };
                let ids = crate::project::layout::json_bytes(
                    &serde_json::to_value(sidecar)
                        .map_err(|error| SessionError::other(error.to_string()))?,
                )?;
                (source, ids)
            };
            if vocabulary_changed {
                validate_source_references(document, &bytes.0, &bytes.1, &reference_table)?;
            }
            let bytes = match id {
                ArtifactId::Component(component) => {
                    with_document_root_parent(document, *component, bytes)?
                }
                _ => bytes,
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
            let expected = &self
                .file_hash_index
                .get(id)
                .ok_or_else(|| {
                    SessionError::InvalidState(format!(
                        "{} has no reconciled file hashes",
                        id.debug_label()
                    ))
                })?
                .files;
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
                        .and_then(|files| files.files.get(*name))
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
        let reference_table = self.reference_table_for_document(document);
        let vocabulary_changed = !Arc::ptr_eq(&self.source_reference_table, &reference_table)
            && *self.source_reference_table != *reference_table;
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
            if vocabulary_changed {
                let source = current
                    .iter()
                    .find(|(name, _)| name == source_name)
                    .map(|(_, bytes)| bytes.as_slice())
                    .ok_or_else(|| {
                        SessionError::InvalidSource("written source is absent".into())
                    })?;
                let sidecar = current
                    .iter()
                    .find(|(name, _)| name == ids_name)
                    .map(|(_, bytes)| bytes.as_slice())
                    .ok_or_else(|| {
                        SessionError::InvalidSource("written sidecar is absent".into())
                    })?;
                validate_source_references(document, source, sidecar, &reference_table)?;
            }
            let file_hashes = IndexedFileHashes::new(&current, self.file_hash_index.get(id));
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
                let source = crate::project::write::placed_source(projected_dir, source);
                for (name, expected) in [(source_name, &source), (ids_name, ids)] {
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
                            .and_then(|files| files.files.get(name))
                            .copied()
                    });
                    let current_hash = file_hashes.files.get(name).copied();
                    if current_hash != expected {
                        return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                    }
                }
            }
            for (name, current_hash) in file_hashes
                .files
                .iter()
                .filter(|(name, _)| name.starts_with("nodes/"))
            {
                let expected = written_hashes
                    .get(&projected_dir.join(name))
                    .copied()
                    .or_else(|| {
                        self.file_hash_index
                            .get(id)
                            .and_then(|files| files.files.get(name))
                            .copied()
                    });
                if expected != Some(*current_hash) {
                    return Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain));
                }
            }
            let disk_hash = file_hashes.content_hash;
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
            accepted.push((id.clone(), meta, disk_hash, node_map, file_hashes));
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
            (
                PathBuf::from(DOC_DIR).join(crate::project::layout::ASSET_LIBRARY_JSON),
                serde_json::to_value(&document.asset_library)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
            (PathBuf::from(DOC_DIR).join(MOTION_JSON), motion),
            (
                PathBuf::from(DOC_DIR).join(FLOW_START_JSON),
                serde_json::to_value(document.flow_start)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
            (
                PathBuf::from(DOC_DIR).join(FLOWS_JSON),
                serde_json::to_value(&document.flows)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
            (
                PathBuf::from(DOC_DIR).join(PRESENTATION_JSON),
                serde_json::to_value(&document.presentation)
                    .map_err(|error| SessionError::other(error.to_string()))?,
            ),
        ]
        .into_iter()
        .chain(set_files_of(document)?)
        {
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
        self.other_singleton_disk_hashes = singleton_disk_hashes(&self.root)?;
        self.source_reference_table = self.reference_table_for_document(document);
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

    /// Adopt a captured document into already-open sources with one immutable
    /// ownership index; publicly mutable definitions cannot invalidate it mid-batch.
    pub fn adopt_document_artifacts<'a>(
        &mut self,
        document: &fanta_doc::Doc,
        artifacts: impl IntoIterator<Item = &'a ArtifactId>,
    ) -> Result<(), SessionError> {
        let scopes = super::materialize::DocumentScopes::new(document);
        for id in artifacts {
            if let Some(artifact) = self.artifact_mut(id) {
                artifact.adopt_document_scoped(&scopes)?;
            }
        }
        Ok(())
    }

    pub fn save_artifact(&mut self, id: ArtifactId) -> Result<SaveResult, SessionError> {
        let session = self.open.get_mut(&id).ok_or(SessionError::NotOpen)?;
        let previous_disk_hash = session.disk_hash;
        let result = session.save(&self.root)?;
        if session.disk_hash != previous_disk_hash {
            self.file_hash_index.insert(
                id.clone(),
                IndexedFileHashes::new(&session.project_to_files()?, None),
            );
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
        if matches!(op, Operation::SetPageRegistry { .. }) {
            return Err(SessionError::InvalidState(
                "page registry changes require the full project document and project-tree save"
                    .into(),
            ));
        }
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
        let ref_table = self.reference_table();
        for session in self.open.values_mut() {
            session.sync_variables(
                vars.clone(),
                modes.clone(),
                generation,
                std::sync::Arc::clone(&ref_table),
            );
        }
    }

    fn reference_table(&mut self) -> Arc<RefTable> {
        let emit_names = self.emit_names();
        // The registries are public and can change without a generation bump.
        // Compare the exact name/id inputs, including ambiguous names, before
        // sharing a table; revision-only and variable-value edits reuse it.
        if let Some(cached) = &self.reference_table_cache
            && cached.matches(&self.components.defs, &self.shared.variables, emit_names)
        {
            return Arc::clone(&cached.table);
        }
        let cached =
            CachedReferenceTable::new(&self.components.defs, &self.shared.variables, emit_names);
        let table = Arc::clone(&cached.table);
        self.reference_table_cache = Some(cached);
        table
    }

    fn reference_table_for_document(&self, document: &fanta_doc::Doc) -> Arc<RefTable> {
        self.reference_table_cache
            .as_ref()
            .filter(|cached| {
                cached.matches(&document.components, &document.variables, self.emit_names())
            })
            .map(|cached| Arc::clone(&cached.table))
            .unwrap_or_else(|| {
                Arc::new(crate::project::refs_ctx::build_ref_table(
                    &document.components,
                    &document.variables,
                    self.emit_names(),
                ))
            })
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
            self.file_hash_index
                .insert(id.clone(), IndexedFileHashes::new(&files, None));
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
                self.file_hash_index
                    .insert(id.clone(), IndexedFileHashes::new(&files, None));
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

        let reloaded = if self
            .open
            .get(id)
            .is_some_and(|session| matches!(session.state, ArtifactDirty::Clean))
        {
            let ref_table = self.reference_table();
            Some(load_artifact_session(
                &self.root,
                &meta,
                self.project_id,
                &self.components.defs,
                self.shared.variables.clone(),
                self.shared.active_modes.clone(),
                self.workspace_generation,
                ref_table,
            )?)
        } else {
            None
        };
        if let Some(session) = self.open.get_mut(id) {
            if let Some(reloaded) = reloaded {
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
        self.file_hash_index.insert(
            id.clone(),
            IndexedFileHashes {
                files: hash_files(&files),
                content_hash: h,
                order: files.iter().map(|(name, _)| name.clone()).collect(),
                source_identity_verified: design_source_identity_is_current(&files),
            },
        );
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
        let metadata = read_json_or(&doc_dir.join(METADATA_JSON), self.shared.metadata.clone())?;
        decode_shared_json::<fanta_doc::DocMetadata>(&doc_dir.join(METADATA_JSON), &metadata)?;
        let variables = decode_shared_json(&doc_dir.join(VARIABLES_JSON), &variables_val)?;
        let active_modes = decode_shared_json(&doc_dir.join(ACTIVE_MODES_JSON), &modes_val)?;
        let h = hash_workspace_shared_on_disk(&self.root)?;
        self.shared.metadata = metadata;
        self.shared.variables = variables;
        self.shared.active_modes = active_modes;
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
        self.file_hash_index.insert(
            id.clone(),
            IndexedFileHashes {
                files: hash_files(&pairs),
                content_hash: disk_hash,
                order: pairs.iter().map(|(name, _)| name.clone()).collect(),
                source_identity_verified: false,
            },
        );
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
    let relative = normalized.strip_prefix(root).ok()?;
    if let Some(design_dir) = component_design_dir_of(root, relative) {
        let slug = design_dir.file_name()?.to_str()?.to_owned();
        return Some((ArtifactKind::Component, slug, design_dir));
    }
    let mut parts = relative.components();
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
    if let ArtifactId::Component(_) = id {
        for dir in scan_component_dirs(root)?.component_dirs {
            let Ok(relative) = dir.strip_prefix(root) else {
                continue;
            };
            if let Some(meta) = artifact_meta_at(root, relative, ArtifactKind::Component)?
                && &meta.id == id
            {
                return Ok(Some(meta));
            }
        }
        return Ok(None);
    }
    let (directory, kind) = match id {
        ArtifactId::Page(_) => (PAGES_DIR, ArtifactKind::Page),
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

/// A component's source and sidecar with the sidecar's `root_parent` set to
/// where `document` has the master. That parent (a page, or a variant set's
/// frame) is outside the component's scope, so moving the master there marks
/// no artifact dirty: without this, a save writes back the old sidecar and the
/// master falls out of its frame on reopening.
fn with_document_root_parent(
    document: &fanta_doc::Doc,
    component: fanta_doc::ComponentId,
    (source, ids): (Vec<u8>, Vec<u8>),
) -> Result<(Vec<u8>, Vec<u8>), SessionError> {
    Ok((
        source,
        component_sidecar_with_document_parent(document, component, ids)?,
    ))
}

fn component_sidecar_with_document_parent(
    document: &fanta_doc::Doc,
    component: fanta_doc::ComponentId,
    ids: Vec<u8>,
) -> Result<Vec<u8>, SessionError> {
    let Some(root) = document.components.def(component).map(|def| def.root) else {
        return Ok(ids);
    };
    let Some(node) = document.scene.get(root) else {
        return Ok(ids);
    };
    let parent = node
        .parent
        .map(|parent| {
            serde_json::to_value(parent)
                .map_err(|error| SessionError::other(error.to_string()))
                .and_then(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| SessionError::other("a node id is not a string"))
                })
        })
        .transpose()?;
    let mut sidecar: fanta_fnx::FnxSidecar = serde_json::from_slice(&ids)
        .map_err(|error| SessionError::other(format!("component sidecar: {error}")))?;
    if sidecar.root_parent == parent {
        return Ok(ids);
    }
    sidecar.root_parent = parent;
    let ids = json_bytes(
        &serde_json::to_value(&sidecar).map_err(|error| SessionError::other(error.to_string()))?,
    )?;
    Ok(ids)
}

/// Workspace files outside any artifact that a save rewrites and that a
/// concurrent change must block: the doc singletons and every variant set's
/// definition on disk (v5 `components/<set>/set.json`, or v4
/// `components/sets.json`).
fn singleton_disk_hashes(root: &Path) -> Result<BTreeMap<PathBuf, Option<[u8; 32]>>, SessionError> {
    let mut paths = vec![
        PathBuf::from(DOC_DIR).join(crate::project::layout::ASSET_LIBRARY_JSON),
        PathBuf::from(DOC_DIR).join(MOTION_JSON),
        PathBuf::from(DOC_DIR).join(FLOW_START_JSON),
        PathBuf::from(DOC_DIR).join(FLOWS_JSON),
        PathBuf::from(DOC_DIR).join(PRESENTATION_JSON),
    ];
    paths.extend(
        scan_component_dirs(root)?
            .set_files
            .into_iter()
            .filter_map(|path| path.strip_prefix(root).ok().map(Path::to_path_buf)),
    );
    paths
        .into_iter()
        .map(|relative| file_sha256_if_present(&root.join(&relative)).map(|hash| (relative, hash)))
        .collect()
}

/// Each variant set's `set.json` path and contents, as the writer projects
/// them for `document`.
fn set_files_of(document: &fanta_doc::Doc) -> Result<Vec<(PathBuf, Value)>, SessionError> {
    let dirs = component_dirs(document);
    document
        .components
        .sets
        .iter()
        .map(|(id, set)| {
            let dir = dirs
                .sets
                .get(id)
                .ok_or_else(|| SessionError::other(format!("set {id} has no directory")))?;
            let value = serde_json::to_value(set)
                .map_err(|error| SessionError::other(error.to_string()))?;
            Ok((dir.join(SET_JSON), value))
        })
        .collect()
}

fn validate_source_references(
    document: &fanta_doc::Doc,
    source: &[u8],
    sidecar: &[u8],
    refs: &RefTable,
) -> Result<(), SessionError> {
    let source =
        std::str::from_utf8(source).map_err(|error| SessionError::other(error.to_string()))?;
    let sidecar = serde_json::from_slice::<fanta_fnx::FnxSidecar>(sidecar)
        .map_err(|error| SessionError::other(error.to_string()))?;
    for (node_id, references) in fanta_fnx::source_reference_nodes_with(source, &sidecar, refs)? {
        let node_id = id_from_key::<fanta_doc::NodeId>(&node_id)?;
        let node = document.scene.get(node_id).ok_or_else(|| {
            SessionError::InvalidSource(format!(
                "retained reference node {node_id} is absent from the document"
            ))
        })?;
        if let Some(component) = references.attrs.get("component") {
            let matches = match &node.data {
                fanta_doc::NodeData::Instance(instance) => {
                    *component
                        == serde_json::to_value(instance.component)
                            .map_err(|error| SessionError::other(error.to_string()))?
                }
                _ => false,
            };
            if !matches {
                return Err(SessionError::InvalidSource(format!(
                    "retained component name on node {node_id} resolves to a different id"
                )));
            }
        }
        if let Some(bindings) = references.attrs.get("bindings") {
            let pairs =
                serde_json::from_value::<Vec<(fanta_doc::BoundProp, VariableId)>>(bindings.clone())
                    .map_err(|error| SessionError::InvalidSource(error.to_string()))?;
            // CanvasNode's source adapter accepts arbitrary pair order and
            // retains the last duplicate. Compare that exact map semantics.
            let actual: BTreeMap<_, _> = pairs.into_iter().collect();
            if actual != node.bindings {
                return Err(SessionError::InvalidSource(format!(
                    "retained variable names on node {node_id} resolve to different ids"
                )));
            }
        }
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn design_source_identity_is_current(files: &[(String, Vec<u8>)]) -> bool {
    let source = files
        .iter()
        .find(|(name, _)| name == "page.fnx" || name == "master.fnx");
    let ids = files
        .iter()
        .find(|(name, _)| name == "page.ids.json" || name == "master.ids.json");
    let Some((_, source)) = source else {
        return false;
    };
    let Some((_, ids)) = ids else {
        return false;
    };
    let Ok(source) = std::str::from_utf8(source) else {
        return false;
    };
    let Ok(sidecar) = serde_json::from_slice::<fanta_fnx::FnxSidecar>(ids) else {
        return false;
    };
    sidecar.ids.iter().all(|entry| {
        id_from_key::<fanta_doc::NodeId>(&entry.id).is_ok()
            && (entry.index.is_null() || entry.index.is_number())
    }) && sidecar
        .root_parent
        .as_ref()
        .is_none_or(|parent| id_from_key::<fanta_doc::NodeId>(parent).is_ok())
        && matches!(
            fanta_fnx::sidecar_matches_source(source, &sidecar),
            Ok(true)
        )
}

fn components_match_for_artifact_save(
    current: &fanta_doc::ComponentLibrary,
    baseline: &fanta_doc::ComponentLibrary,
    indexed: &fanta_doc::ComponentLibrary,
    changed_artifacts: &BTreeSet<ArtifactId>,
) -> bool {
    if current.sets != baseline.sets
        || current.sets != indexed.sets
        || current.defs.len() != baseline.defs.len()
        || current.defs.len() != indexed.defs.len()
    {
        return false;
    }
    let structural_definition = |definition: &fanta_doc::ComponentDef| {
        let mut definition = definition.clone();
        definition.rev = 0;
        definition.preview_rev = 0;
        definition
    };
    current.defs.iter().all(|(id, current)| {
        let (Some(baseline), Some(indexed)) = (baseline.defs.get(id), indexed.defs.get(id)) else {
            return false;
        };
        if current == baseline && current == indexed {
            return true;
        }
        let structural = structural_definition(current);
        structural == structural_definition(baseline)
            && structural == structural_definition(indexed)
            && ((current.rev == baseline.rev && current.rev == indexed.rev)
                || changed_artifacts.contains(&ArtifactId::Component(*id)))
    })
}

fn artifact_save_scope_is_complete(
    document: &fanta_doc::Doc,
    baseline: &fanta_doc::Doc,
    changed_artifacts: &BTreeSet<ArtifactId>,
) -> bool {
    let Some(current_owners) = artifact_save_owners(document) else {
        return false;
    };
    let Some(previous_owners) = artifact_save_owners(baseline) else {
        return false;
    };
    changed_artifacts
        .iter()
        .all(|id| matches!(id, ArtifactId::Page(_) | ArtifactId::Component(_)))
        && current_owners.iter().all(|(id, owner)| {
            document.scene.shares_node(&baseline.scene, *id) || changed_artifacts.contains(owner)
        })
        && previous_owners.iter().all(|(id, owner)| {
            document.scene.shares_node(&baseline.scene, *id) || changed_artifacts.contains(owner)
        })
}

fn artifact_save_owners(
    document: &fanta_doc::Doc,
) -> Option<HashMap<fanta_doc::NodeId, ArtifactId>> {
    let mut roots = HashMap::new();
    for page in document.pages() {
        if document.scene.get(*page)?.parent.is_some()
            || roots.insert(*page, ArtifactId::Page(*page)).is_some()
        {
            return None;
        }
    }
    for (component, definition) in &document.components.defs {
        document.scene.get(definition.root)?;
        if roots
            .insert(definition.root, ArtifactId::Component(*component))
            .is_some()
        {
            return None;
        }
    }
    let mut owners = HashMap::with_capacity(document.scene.len());
    let mut pending = Vec::new();
    let mut visited = HashSet::new();
    for root in document.scene.roots() {
        pending.push((*root, None, roots.get(root)?.clone()));
    }
    while let Some((id, parent, owner)) = pending.pop() {
        let node = document.scene.get(id)?;
        if node.id != id || node.parent != parent || !visited.insert(id) {
            return None;
        }
        let children = document.scene.children_of(Some(id));
        if !children.is_empty() && !node.can_have_children() {
            return None;
        }
        let owner = roots.get(&id).cloned().unwrap_or(owner);
        owners.insert(id, owner.clone());
        pending.extend(
            children
                .iter()
                .rev()
                .map(|child| (*child, Some(id), owner.clone())),
        );
    }
    (owners.len() == document.scene.len()).then_some(owners)
}

fn index_initial_component_files(
    entries: &[&Path],
    parallel: bool,
    source_identities: Option<&crate::project::read::LoadedSourceIdentities>,
) -> Result<Vec<IndexedFileHashes>, SessionError> {
    const PARALLEL_MINIMUM_COMPONENTS: usize = 256;
    const INDEX_WORKERS: usize = 4;

    let index = |entries: &[&Path]| {
        entries
            .iter()
            .map(|entry| {
                let files = read_indexed_file_set(entry, ArtifactKind::Component)?;
                match source_identities {
                    Some(identities) => {
                        let identity = identities
                            .get(&entry.join(crate::project::layout::MASTER_FNX))
                            .ok_or_else(|| {
                                SessionError::InvalidState(
                                    "component selection changed while the project was loading"
                                        .into(),
                                )
                            })?;
                        IndexedFileHashes::from_loaded_source(
                            &files,
                            crate::project::layout::MASTER_FNX,
                            MASTER_IDS,
                            identity,
                        )
                    }
                    None => Ok(IndexedFileHashes::new(&files, None)),
                }
            })
            .collect::<Result<Vec<_>, SessionError>>()
    };
    if !parallel || entries.len() < PARALLEL_MINIMUM_COMPONENTS {
        return index(entries);
    }

    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(INDEX_WORKERS);
        for chunk in entries.chunks(entries.len().div_ceil(INDEX_WORKERS)) {
            let worker = std::thread::Builder::new().spawn_scoped(scope, move || index(chunk));
            workers.push((chunk, worker));
        }
        let mut indexed = Vec::with_capacity(entries.len());
        let mut first_error = None;
        // All joins must complete before any result is published or returned;
        // sorted chunk order preserves the serial path's first input error.
        for (chunk, worker) in workers {
            let result = match worker {
                Ok(worker) => worker.join().unwrap_or_else(|_| {
                    Err(SessionError::other(format!(
                        "component indexing worker panicked while checking {}",
                        chunk
                            .first()
                            .map_or_else(|| Path::new("components"), |entry| *entry)
                            .display()
                    )))
                }),
                Err(error) => Err(SessionError::Io(error)),
            };
            match result {
                Ok(mut hashes) if first_error.is_none() => indexed.append(&mut hashes),
                Ok(_) => {}
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(indexed),
        }
    })
}

fn read_indexed_file_set(
    design_dir: &Path,
    kind: ArtifactKind,
) -> Result<Vec<(String, Vec<u8>)>, SessionError> {
    let mut files = read_file_set(design_dir, kind)?;
    if !matches!(kind, ArtifactKind::Page | ArtifactKind::Component) {
        return Ok(files);
    }
    let source_name = match kind {
        ArtifactKind::Page => crate::project::layout::PAGE_FNX,
        ArtifactKind::Component => crate::project::layout::MASTER_FNX,
        _ => return Ok(files),
    };
    if files.iter().any(|(name, _)| name == source_name) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::layout::{PAGE_FNX, PAGE_IDS};

    #[test]
    fn reference_validation_uses_the_authored_binding_maps_semantics() {
        let mut document = fanta_doc::Doc::new();
        let first = VariableId::new();
        let second = VariableId::new();
        let mut node =
            fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(fanta_doc::GroupNode::default()));
        node.bindings.insert(fanta_doc::BoundProp::Opacity, first);
        node.bindings.insert(fanta_doc::BoundProp::Visible, second);
        let node_id = document.scene.insert(node.clone()).expect("node");
        document.add_page(node_id);
        let ir = fanta_fnx::ArtifactIr::from_nodes(
            ArtifactKind::Page,
            "Bindings",
            &[serde_json::to_value(&node).expect("node JSON")],
        )
        .expect("IR");
        let sidecar = json_bytes(&serde_json::to_value(ir.sidecar()).expect("sidecar"))
            .expect("sidecar bytes");
        let mut pairs: Vec<_> = node
            .bindings
            .iter()
            .map(|(property, variable)| (*property, *variable))
            .collect();
        pairs.reverse();
        for duplicate in [false, true] {
            let mut authored = pairs.clone();
            if duplicate {
                authored.insert(0, (fanta_doc::BoundProp::Visible, first));
            }
            let source = format!(
                "<Group id=\"{}\" bindings={{{}}} />",
                node_id.0,
                serde_json::to_string(&authored).expect("authored binding pairs")
            );
            validate_source_references(
                &document,
                source.as_bytes(),
                &sidecar,
                &RefTable::new(false),
            )
            .expect("same binding map despite ordering or overwritten duplicate");
        }
    }

    #[test]
    fn emitted_identity_witness_only_certifies_exact_source_and_sidecar_bytes() {
        let directory = tempfile::tempdir().expect("project");
        let mut document = fanta_doc::Doc::new();
        let page = document
            .scene
            .insert(fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                fanta_doc::GroupNode::default(),
            )))
            .expect("page");
        document.add_page(page);
        crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
            .expect("project files");
        let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
        let artifact = ArtifactId::Page(page);
        workspace.open_artifact(artifact.clone()).expect("session");
        let (files, witness) = workspace
            .artifact(&artifact)
            .expect("session")
            .project_files_with_identity()
            .expect("projected files");
        let witness = witness.expect("checked retained output");
        let checked = IndexedFileHashes::with_source_identity(&files, None, Some(&witness));
        assert!(checked.source_identity_verified);
        let parsed = IndexedFileHashes::new(&files, None);
        assert_eq!(checked.files, parsed.files);
        assert_eq!(checked.content_hash, parsed.content_hash);
        assert_eq!(checked.order, parsed.order);
        assert_eq!(
            checked.source_identity_verified,
            parsed.source_identity_verified
        );
        for name in [PAGE_FNX, PAGE_IDS] {
            let mut corrupted = files.clone();
            let bytes = &mut corrupted
                .iter_mut()
                .find(|(file, _)| file == name)
                .expect("file")
                .1;
            *bytes = if name == PAGE_FNX {
                b"<Group id=\"forged\" future={bad} />".to_vec()
            } else {
                b"{\"ids\":[]}".to_vec()
            };
            let rejected =
                IndexedFileHashes::with_source_identity(&corrupted, None, Some(&witness));
            assert!(!rejected.source_identity_verified, "{name}");
            let parsed = IndexedFileHashes::new(&corrupted, None);
            assert_eq!(rejected.files, parsed.files);
            assert_eq!(rejected.content_hash, parsed.content_hash);
            assert_eq!(rejected.order, parsed.order);
            assert_eq!(
                rejected.source_identity_verified,
                parsed.source_identity_verified
            );
        }
    }

    #[test]
    fn large_component_index_matches_serial_hashes_and_identity_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut document = fanta_doc::Doc::new();
        let page =
            document
                .scene
                .insert(fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                    fanta_doc::GroupNode::default(),
                )))?;
        document.add_page(page);
        for position in 0..300 {
            let mut master = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                fanta_doc::GroupNode::default(),
            ));
            master.name = format!("Master {position:04}");
            master.parent = Some(page);
            let root = document.scene.insert(master)?;
            let mut shape = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Vector(
                fanta_doc::VectorNode::rect_solid(0.0, 0.0, 20.0, 30.0, fanta_doc::Color::BLACK),
            ));
            shape.parent = Some(root);
            document.scene.insert(shape)?;
            let component = ComponentId::new();
            document.components.defs.insert(
                component,
                fanta_doc::ComponentDef::new(component, root, format!("Master {position:04}")),
            );
        }
        crate::write_project_tree(directory.path(), &document, &BTreeMap::new())?;
        let fresh = WorkspaceSession::open(directory.path())?;
        assert!(fresh.open.is_empty());
        assert_eq!(fresh.components.defs, document.components);
        assert_eq!(fresh.file_index.len(), 301);
        let paths: Vec<_> = fresh
            .artifacts
            .iter()
            .filter(|(id, _)| matches!(id, ArtifactId::Component(_)))
            .map(|(_, meta)| directory.path().join(&meta.design_dir))
            .collect();
        let entries: Vec<_> = paths.iter().map(PathBuf::as_path).collect();
        let assert_same = |current: &WorkspaceSession| -> Result<(), Box<dyn std::error::Error>> {
            let serial = index_initial_component_files(&entries, false, None)?;
            let parallel = index_initial_component_files(&entries, true, None)?;
            assert_eq!(serial.len(), entries.len());
            assert_eq!(parallel.len(), entries.len());
            for ((entry, serial), parallel) in entries.iter().zip(&serial).zip(&parallel) {
                assert_eq!(serial.content_hash, parallel.content_hash);
                assert_eq!(serial.files, parallel.files);
                assert_eq!(serial.order, parallel.order);
                assert_eq!(
                    serial.source_identity_verified,
                    parallel.source_identity_verified
                );
                let relative = entry.strip_prefix(directory.path())?;
                let metadata = current
                    .artifacts
                    .values()
                    .find(|meta| meta.design_dir == relative)
                    .expect("selected component metadata");
                let indexed = current
                    .file_hash_index
                    .get(&metadata.id)
                    .expect("indexed files");
                assert_eq!(
                    current.file_index.get(&metadata.id),
                    Some(&serial.content_hash)
                );
                assert_eq!(indexed.files, serial.files);
                assert_eq!(indexed.order, serial.order);
                assert_eq!(
                    indexed.source_identity_verified,
                    serial.source_identity_verified
                );
            }
            Ok(())
        };
        assert_same(&fresh)?;
        let assert_combined = || -> Result<(), Box<dyn std::error::Error>> {
            let (expected, expected_assets) = crate::read_project_tree(directory.path())?;
            let (loaded, assets, combined) =
                WorkspaceSession::open_project_document(directory.path())?;
            assert_eq!(
                serde_json::to_value(&loaded)?,
                serde_json::to_value(&expected)?
            );
            assert_eq!(loaded.pending_layout, expected.pending_layout);
            assert_eq!(assets, expected_assets);
            assert!(combined.open.is_empty());
            assert_eq!(combined.components.defs, loaded.components);
            assert_same(&combined)?;
            Ok(())
        };
        assert_combined()?;
        assert!(
            fresh
                .file_hash_index
                .values()
                .all(|hashes| hashes.source_identity_verified)
        );

        let damaged = entries.get(23).expect("first chunk component");
        let missing = entries.get(241).expect("last chunk component");
        std::fs::write(damaged.join(MASTER_IDS), b"{")?;
        std::fs::remove_file(missing.join(MASTER_IDS))?;
        let changed = WorkspaceSession::open(directory.path())?;
        assert_same(&changed)?;
        assert_combined()?;
        assert_eq!(changed.components.defs, fresh.components.defs);
        assert_eq!(changed.artifacts.len(), fresh.artifacts.len());
        assert!(changed.open.is_empty());
        for entry in [damaged, missing] {
            let relative = entry.strip_prefix(directory.path())?;
            let metadata = changed
                .artifacts
                .values()
                .find(|meta| meta.design_dir == relative)
                .expect("component with damaged identity");
            assert!(!changed.file_hash_index[&metadata.id].source_identity_verified);
        }

        let manifest_path = directory.path().join(FANTA_JSON);
        let mut manifest: Value = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
        manifest["version"] = json!(PROJECT_VERSION - 1);
        std::fs::write(manifest_path, json_bytes(&manifest)?)?;
        let legacy = WorkspaceSession::open(directory.path())?;
        assert_same(&legacy)?;
        assert_combined()?;
        assert_eq!(legacy.file_index, changed.file_index);
        assert_eq!(legacy.components.defs, changed.components.defs);
        Ok(())
    }

    #[test]
    fn combined_loading_tracks_exact_asset_bytes_for_startup_events()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let document = fanta_doc::Doc::new();
        let asset = fanta_doc::AssetId::new();
        let bytes = b"\x89PNG\r\n\x1a\nexact loaded asset".to_vec();
        crate::write_project_tree(
            directory.path(),
            &document,
            &BTreeMap::from([(asset, bytes.clone())]),
        )?;
        let root = directory.path().canonicalize()?;
        let asset_path = PathBuf::from(format!("assets/images/{asset}.png"));
        let index_path = PathBuf::from("assets/index.json");
        let (_, assets, evidence) =
            crate::project::read::read_project_tree_with_source_identities(&root)?;
        assert_eq!(assets.get(&asset), Some(&bytes));
        let mut session = WorkspaceSession::open_locked(root.clone(), Some(&evidence))?;
        let hashes = session.take_loaded_file_hashes();
        assert_eq!(hashes.get(&asset_path), Some(&sha256_bytes(&bytes)));
        assert_eq!(
            hashes.get(&index_path),
            Some(&sha256_bytes(&std::fs::read(root.join(&index_path))?))
        );
        for relative in [asset_path, index_path] {
            let path = root.join(relative);
            let original = std::fs::read(&path)?;
            let mut changed = original.clone();
            changed.push(b' ');
            std::fs::write(&path, changed)?;
            assert!(WorkspaceSession::open_locked(root.clone(), Some(&evidence)).is_err());
            std::fs::write(path, original)?;
        }
        let (_, _, mut reopened) = WorkspaceSession::open_project_document(&root)?;
        assert_eq!(reopened.take_loaded_file_hashes(), hashes);
        Ok(())
    }

    #[test]
    fn combined_loading_refuses_changed_bytes_and_invalid_unknown_attributes()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut document = fanta_doc::Doc::new();
        let page =
            document
                .scene
                .insert(fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                    Default::default(),
                )))?;
        document.add_page(page);
        let mut child = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(Default::default()));
        child.name = "Authored child".into();
        child.parent = Some(page);
        let child_id = document.scene.insert(child)?;
        crate::write_project_tree(directory.path(), &document, &BTreeMap::new())?;
        let root = directory.path().canonicalize()?;
        let first = WorkspaceSession::open(&root)?;
        let original_dir = root.join(
            &first
                .artifacts
                .get(&ArtifactId::Page(page))
                .ok_or("page metadata")?
                .design_dir,
        );
        let authored_dir = root.join(PAGES_DIR).join("_authored");
        std::fs::rename(original_dir, &authored_dir)?;
        let (_, _, identities) =
            crate::project::read::read_project_tree_with_source_identities(&root)?;
        let initial = WorkspaceSession::open_locked(root.clone(), Some(&identities))?;
        let meta = initial
            .artifacts
            .get(&ArtifactId::Page(page))
            .ok_or("page metadata")?;
        let source_path = root
            .join(&meta.design_dir)
            .join(crate::project::layout::PAGE_FNX);
        let sidecar_path = root
            .join(&meta.design_dir)
            .join(crate::project::layout::PAGE_IDS);
        let source = std::fs::read_to_string(&source_path)?;
        let sidecar = std::fs::read(&sidecar_path)?;
        let header_path = authored_dir.join(PAGE_JSON);
        let header = std::fs::read(&header_path)?;
        let mut changed_header = header.clone();
        changed_header.push(b' ');
        std::fs::write(&header_path, changed_header)?;
        assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
        std::fs::write(&header_path, &header)?;

        for relative in [
            FANTA_JSON.to_string(),
            format!("{DOC_DIR}/{METADATA_JSON}"),
            format!("{DOC_DIR}/{VARIABLES_JSON}"),
            format!("{DOC_DIR}/{ACTIVE_MODES_JSON}"),
        ] {
            let path = root.join(relative);
            let original = std::fs::read(&path)?;
            let mut changed = original.clone();
            changed.push(b' ');
            std::fs::write(&path, changed)?;
            assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
            std::fs::write(path, original)?;
        }
        let added_page = fanta_doc::NodeId::new();
        let added_child = fanta_doc::NodeId::new();
        let added_dir = root.join(PAGES_DIR).join("late-page");
        std::fs::create_dir(&added_dir)?;
        write_json_file(
            &added_dir.join(PAGE_JSON),
            &json!({ "id": added_page.to_string(), "name": "Late page", "order": 1 }),
        )?;
        std::fs::write(
            added_dir.join(crate::project::layout::PAGE_FNX),
            source
                .replace(&page.0.to_string(), &added_page.0.to_string())
                .replace(&child_id.0.to_string(), &added_child.0.to_string()),
        )?;
        assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
        let (two_pages, _, _) = WorkspaceSession::open_project_document(&root)?;
        assert_eq!(two_pages.pages().len(), 2);
        std::fs::remove_dir_all(added_dir)?;
        let removed_dir = root.join("removed-page");
        std::fs::rename(&authored_dir, &removed_dir)?;
        assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
        std::fs::rename(removed_dir, &authored_dir)?;
        for changed in [
            source.replace("Authored child", "Renamed child"),
            source.replace(
                &child_id.0.to_string(),
                &fanta_doc::NodeId::new().0.to_string(),
            ),
        ] {
            assert_ne!(changed, source);
            std::fs::write(&source_path, &changed)?;
            assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
            let (expected, _) = crate::read_project_tree(&root)?;
            let (loaded, _, refreshed) = WorkspaceSession::open_project_document(&root)?;
            assert_eq!(
                serde_json::to_value(&loaded)?,
                serde_json::to_value(expected)?
            );
            assert!(!refreshed.file_hash_index[&ArtifactId::Page(page)].source_identity_verified);
        }
        std::fs::write(&source_path, &source)?;
        let mut changed_sidecar = sidecar.clone();
        changed_sidecar.push(b' ');
        std::fs::write(&sidecar_path, changed_sidecar)?;
        assert!(WorkspaceSession::open_locked(root.clone(), Some(&identities)).is_err());
        std::fs::write(&sidecar_path, &sidecar)?;

        for attribute in [
            "future_hint={not_json}",
            "future_hint={1e400}",
            r#"future_hint="\q""#,
        ] {
            let changed = source.replacen("<Frame", &format!("<Frame {attribute}"), 1);
            assert_ne!(changed, source);
            std::fs::write(&source_path, changed)?;
            let expected = crate::read_project_tree(&root).expect_err("invalid attribute");
            let actual = WorkspaceSession::open_project_document(&root)
                .err()
                .ok_or("invalid attribute must fail combined load")?;
            assert_eq!(actual.to_string(), expected.to_string());
        }
        std::fs::write(&source_path, &source)?;
        let (loaded, _, final_session) = WorkspaceSession::open_project_document(&root)?;
        assert_eq!(loaded.scene.len(), document.scene.len());
        assert!(final_session.file_hash_index[&ArtifactId::Page(page)].source_identity_verified);
        assert_eq!(std::fs::read_to_string(source_path)?, source);
        assert_eq!(std::fs::read(sidecar_path)?, sidecar);
        Ok(())
    }

    #[test]
    fn indexed_file_hashes_require_the_same_order_names_and_exact_bytes() {
        let files = vec![
            ("page.fnx".to_owned(), b"source".to_vec()),
            ("page.ids.json".to_owned(), b"sidecar".to_vec()),
            ("page.json".to_owned(), b"header".to_vec()),
        ];
        let previous = IndexedFileHashes::new(&files, None);
        let mut changed_bytes = files.clone();
        changed_bytes.first_mut().expect("source slot").1 = b"Source".to_vec();
        let mut changed_order = files.clone();
        changed_order.reverse();
        let mut changed_name = files.clone();
        changed_name.first_mut().expect("source slot").0 = "master.fnx".into();
        let mut missing_file = files.clone();
        missing_file.pop();
        for current in [
            files.clone(),
            changed_bytes,
            changed_order,
            changed_name,
            missing_file,
        ] {
            let indexed = IndexedFileHashes::new(&current, Some(&previous));
            assert_eq!(indexed.content_hash, hash_file_set(&current));
            assert_eq!(indexed.files, hash_files(&current));
            if current != files {
                assert_ne!(indexed.content_hash, previous.content_hash);
            }
        }
    }

    #[test]
    fn incremental_artifact_acceptance_rejects_post_write_changes_atomically() {
        for name in [
            "page.fnx",
            "page.ids.json",
            "page.json",
            "doc/asset_library.json",
        ] {
            for remove in [false, true] {
                let directory = tempfile::tempdir().expect("project directory");
                let mut document = fanta_doc::Doc::new();
                let mut pages = Vec::new();
                for name in ["Edited", "Unopened"] {
                    let mut root = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                        fanta_doc::GroupNode::default(),
                    ));
                    root.name = name.into();
                    let page = document.scene.insert(root).expect("page root");
                    document.add_page(page);
                    pages.push(page);
                }
                let mut master = fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(
                    fanta_doc::GroupNode::default(),
                ));
                master.name = "Master".into();
                master.parent = pages.last().copied();
                let master = document.scene.insert(master).expect("master root");
                let component = fanta_doc::ComponentId::new();
                document.components.defs.insert(
                    component,
                    fanta_doc::ComponentDef::new(component, master, "Master"),
                );
                crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
                    .expect("initial project");
                let (mut document, _) =
                    crate::read_project_tree(directory.path()).expect("project");
                let baseline = document.clone_for_persist();
                let edited = *pages.first().expect("edited page");
                let unopened = *pages.last().expect("unopened page");
                document.scene.get_mut(edited).expect("page root").transform =
                    fanta_doc::Transform2D::translation(11.0, 17.0);
                document
                    .scene
                    .get_mut(master)
                    .expect("master root")
                    .transform = fanta_doc::Transform2D::translation(23.0, 31.0);
                assert_eq!(
                    document
                        .components
                        .bump_rev_for_node(&document.scene, master),
                    1
                );
                let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
                let prepared = workspace
                    .prepare_document_artifact_save(
                        &document,
                        &baseline,
                        &[ArtifactId::Page(edited), ArtifactId::Component(component)]
                            .into_iter()
                            .collect(),
                        &BTreeMap::new(),
                    )
                    .expect("prepare save")
                    .expect("ordinary edit");
                let (_, projected_hashes) = crate::project::write::write_project_artifacts_checked(
                    directory.path(),
                    &document,
                    &prepared.files,
                    &prepared.expected_disk,
                )
                .expect("checked write");
                let before_index = workspace.file_index.clone();
                let before_shared = workspace.shared.disk_hash;
                let before_components = workspace.components.defs.clone();
                let before_generation = workspace.workspace_generation;
                let before_base = Arc::clone(&workspace.open[&ArtifactId::Page(edited)].base_nodes);
                let relative = if name.starts_with("doc/") {
                    PathBuf::from(name)
                } else {
                    workspace.artifacts[&ArtifactId::Page(unopened)]
                        .design_dir
                        .join(name)
                };
                let path = directory.path().join(relative);
                let original = std::fs::read(&path).expect("unopened file");
                if remove {
                    std::fs::remove_file(&path).expect("external deletion");
                } else {
                    let mut external = original.clone();
                    *external.first_mut().expect("nonempty file") ^= 1;
                    std::fs::write(&path, external).expect("same-size external edit");
                }
                assert!(matches!(
                    workspace.accept_document_artifact_save(&document, prepared, &projected_hashes),
                    Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain)),
                ));
                assert_eq!(workspace.file_index, before_index);
                assert_eq!(workspace.shared.disk_hash, before_shared);
                assert_eq!(workspace.components.defs, before_components);
                assert_eq!(workspace.workspace_generation, before_generation);
                let edited_session = &workspace.open[&ArtifactId::Page(edited)];
                assert!(Arc::ptr_eq(&edited_session.base_nodes, &before_base));
                assert!(matches!(edited_session.state, ArtifactDirty::DirtyCanvas));
                assert!(!workspace.open.contains_key(&ArtifactId::Page(unopened)));
                std::fs::write(path, original).expect("restore external file");
            }
        }
    }
}

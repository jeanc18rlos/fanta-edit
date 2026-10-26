//! Project-tree → `Doc` projection (the inverse of [`super::write`]).
//!
//! The doc is reassembled in `serde_json::Value` space from the granular
//! files, run through the schema-migration ladder if the tree was written by
//! an older build, and only then deserialized into `Doc`, whose scene's child
//! index is rebuilt and validated. Presence state
//! (`active_page`, `selection`, `viewport`, `history`) is never on disk, so
//! the loaded doc carries serde defaults for all of it.

use crate::error::{FormatError, Result};
use crate::migrate::migrate;
use crate::sidecar::{persisted_fnx_sidecar_from_tree, reconcile_fnx_sidecar_from_tree};
use fanta_doc::{AssetId, ComponentId, Doc, DocId, NodeId, SCHEMA_VERSION};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::layout::{
    ACTIVE_MODES_JSON, ASSET_LIBRARY_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR,
    FLOW_START_JSON, FLOWS_JSON, LOOSE_DIR, MASTER_FNX, MASTER_IDS, METADATA_JSON, MOTION_JSON,
    NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON, PAGES_DIR, PRESENTATION_JSON, SET_JSON, STYLES_JSON,
    VARIABLES_JSON, id_from_key, is_project_dir, json_key, read_json_file, read_manifest,
    sorted_entries,
};

/// Load a project directory back into a [`Doc`] plus its asset blobs.
///
/// Errors with [`FormatError::NotAProject`] when `dir` has no tagged
/// `fanta.json`, and [`FormatError::UnsupportedSchema`] when the tree was
/// written by a newer doc schema than this build understands. Older schemas
/// are migrated in JSON space before deserialization.
pub fn read_project_tree(dir: &Path) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
    read_project_tree_with_source_override(dir, None)
}

pub(crate) struct LoadedSourceIdentity {
    pub(crate) source_hash: Option<[u8; 32]>,
    pub(crate) sidecar_hash: Option<[u8; 32]>,
    pub(crate) current: bool,
}

pub(crate) type LoadedSourceIdentities = BTreeMap<PathBuf, LoadedSourceIdentity>;

type LoadedFileHashes = BTreeMap<PathBuf, Option<[u8; 32]>>;

#[derive(Default)]
pub(crate) struct LoadedProjectEvidence {
    pub(crate) sources: LoadedSourceIdentities,
    pub(crate) files: LoadedFileHashes,
    pub(crate) pages: Vec<NodeId>,
}

pub(crate) fn read_project_tree_with_source_identities(
    dir: &Path,
) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>, LoadedProjectEvidence)> {
    super::layout::with_project_read_lock(dir, || {
        let mut evidence = LoadedProjectEvidence::default();
        let (doc, assets) = read_project_tree_with_source_override_locked(
            dir,
            None,
            None,
            Some(&mut evidence.sources),
            Some(&mut evidence.files),
        )?;
        evidence.pages = doc.pages().to_vec();
        Ok((doc, assets, evidence))
    })
}

pub(super) struct FnxSourceOverride<'a> {
    pub path: &'a Path,
    pub source: &'a str,
    pub sidecar: Option<&'a fanta_fnx::FnxSidecar>,
}

pub(super) fn read_project_tree_with_source_override(
    dir: &Path,
    source_override: Option<&FnxSourceOverride<'_>>,
) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
    super::layout::with_project_read_lock(dir, || {
        read_project_tree_with_source_override_locked(dir, source_override, None, None, None)
    })
}

pub(super) struct JsonSourceOverride<'a> {
    pub path: &'a Path,
    pub value: &'a Value,
}

pub(super) fn read_project_tree_with_json_override(
    dir: &Path,
    source_override: &JsonSourceOverride<'_>,
) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
    super::layout::with_project_read_lock(dir, || {
        read_project_tree_with_source_override_locked(dir, None, Some(source_override), None, None)
    })
}

fn read_project_json(
    path: &Path,
    source_override: Option<&JsonSourceOverride<'_>>,
) -> Result<Value> {
    let value = match source_override.filter(|source| source.path == path) {
        Some(source) => source.value.clone(),
        None => read_json_file(path)?,
    };
    validate_project_json_shape(path, &value)?;
    Ok(value)
}

fn read_project_json_with_hash(
    path: &Path,
    source_override: Option<&JsonSourceOverride<'_>>,
    capture_hash: bool,
) -> Result<(Value, Option<[u8; 32]>)> {
    if !capture_hash || source_override.is_some_and(|source| source.path == path) {
        return read_project_json(path, source_override).map(|value| (value, None));
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FormatError::MissingFile {
                name: path.display().to_string(),
            }
        } else {
            FormatError::Io(error)
        }
    })?;
    let value = serde_json::from_str(&text)
        .map_err(|error| FormatError::InvalidProjectTree(format!("{}: {error}", path.display())))?;
    validate_project_json_shape(path, &value)?;
    Ok((value, Some(Sha256::digest(text.as_bytes()).into())))
}

fn read_tracked_project_json(
    path: &Path,
    fallback: Option<Value>,
    source_override: Option<&JsonSourceOverride<'_>>,
    files: Option<&mut LoadedFileHashes>,
) -> Result<Value> {
    if !path.exists()
        && source_override.is_none_or(|source| source.path != path)
        && let Some(value) = fallback
    {
        validate_project_json_shape(path, &value)?;
        if let Some(files) = files {
            files.insert(path.to_path_buf(), None);
        }
        return Ok(value);
    }
    let (value, hash) = read_project_json_with_hash(path, source_override, files.is_some())?;
    if let Some(files) = files {
        files.insert(path.to_path_buf(), hash);
    }
    Ok(value)
}

pub(super) fn validate_project_json_shape(path: &Path, value: &Value) -> Result<()> {
    let (valid, expected) = match path.file_name().and_then(|name| name.to_str()) {
        Some(FLOW_START_JSON) => (value.is_null() || value.is_string(), "a node ID or null"),
        Some(FLOWS_JSON) => (value.is_array(), "an array"),
        Some(PRESENTATION_JSON) => (value.is_null() || value.is_object(), "an object or null"),
        _ => (value.is_object(), "an object"),
    };
    if !valid {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} must contain {expected}",
            path.display()
        )));
    }
    Ok(())
}

fn read_project_tree_with_source_override_locked(
    dir: &Path,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    mut source_identities: Option<&mut LoadedSourceIdentities>,
    mut loaded_files: Option<&mut LoadedFileHashes>,
) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
    let manifest =
        match json_override.filter(|source| source.path == dir.join(super::layout::FANTA_JSON)) {
            Some(source) => {
                validate_project_json_shape(source.path, source.value)?;
                let manifest: super::layout::ProjectManifest =
                    serde_json::from_value(source.value.clone())?;
                if manifest.format != super::layout::FORMAT_TAG {
                    return Err(FormatError::NotAProject {
                        path: dir.to_path_buf(),
                    });
                }
                if manifest.version > super::layout::PROJECT_VERSION {
                    return Err(FormatError::UnsupportedProjectVersion {
                        found: manifest.version,
                        supported: super::layout::PROJECT_VERSION,
                    });
                }
                manifest
            }
            None => {
                if let Some(files) = &mut loaded_files {
                    let (manifest, source) = super::layout::read_manifest_with_source(dir)?;
                    files.insert(
                        dir.join(super::layout::FANTA_JSON),
                        Some(Sha256::digest(source.as_bytes()).into()),
                    );
                    manifest
                } else {
                    read_manifest(dir)?
                }
            }
        };
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(FormatError::UnsupportedSchema {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION,
        });
    }

    let project_id: DocId = manifest.project_id.parse().map_err(|e| {
        FormatError::InvalidProjectTree(format!(
            "fanta.json project_id {:?} is not a DocId: {e}",
            manifest.project_id
        ))
    })?;

    let mut root = Map::new();
    root.insert("id".to_owned(), serde_json::to_value(project_id)?);
    root.insert(
        "schema_version".to_owned(),
        Value::from(manifest.schema_version),
    );

    let doc_dir = dir.join(DOC_DIR);
    root.insert(
        "metadata".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(METADATA_JSON),
            None,
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    root.insert(
        "asset_library".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(ASSET_LIBRARY_JSON),
            Some(json!({})),
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    let variables = read_tracked_project_json(
        &doc_dir.join(VARIABLES_JSON),
        Some(json!({})),
        json_override,
        loaded_files.as_deref_mut(),
    )?;
    root.insert("variables".to_owned(), variables.clone());
    root.insert(
        "styles".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(STYLES_JSON),
            Some(json!({})),
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    root.insert(
        "active_modes".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(ACTIVE_MODES_JSON),
            Some(json!({})),
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    root.insert(
        "motion".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(MOTION_JSON),
            Some(json!({})),
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    let flow_start = read_tracked_project_json(
        &doc_dir.join(FLOW_START_JSON),
        Some(Value::Null),
        json_override,
        loaded_files.as_deref_mut(),
    )?;
    if !flow_start.is_null() {
        root.insert("flow_start".to_owned(), flow_start);
    }
    root.insert(
        "flows".to_owned(),
        read_tracked_project_json(
            &doc_dir.join(FLOWS_JSON),
            Some(json!([])),
            json_override,
            loaded_files.as_deref_mut(),
        )?,
    );
    let presentation = read_tracked_project_json(
        &doc_dir.join(PRESENTATION_JSON),
        Some(Value::Null),
        json_override,
        loaded_files.as_deref_mut(),
    )?;
    if !presentation.is_null() {
        root.insert("presentation".to_owned(), presentation);
    }

    // Index component defs (and the variables read above) BEFORE decoding any
    // design source: name-based references — `component="Button"`,
    // `"$Collection/Name"` binding paths — may appear in ANY page or master,
    // so the resolution vocabulary must exist first. The scan collects
    // headers only; masters materialize after pages, below, from the same
    // deduped winner set. Names are emitted back only for layout v4+ trees;
    // resolution is accepted on read regardless (like the width/height
    // sugar), so a hand-named reference in an older tree still loads.
    let component_scan = scan_components(dir, json_override)?;
    let refs = crate::project::refs_ctx::build_ref_table_json(
        &component_scan.defs,
        &variables,
        manifest.version >= 4,
    );

    let mut nodes = Map::new();
    let mut pending_layout = BTreeSet::new();
    root.insert(
        "pages".to_owned(),
        read_pages(
            dir,
            manifest.version,
            source_override,
            json_override,
            &refs,
            &mut nodes,
            &mut pending_layout,
            source_identities.as_deref_mut(),
            loaded_files.as_deref_mut(),
        )?,
    );
    read_component_masters(
        &component_scan,
        manifest.version,
        source_override,
        json_override,
        &refs,
        &mut nodes,
        &mut pending_layout,
        source_identities,
    )?;
    root.insert(
        "components".to_owned(),
        Value::Object(Map::from_iter([
            ("defs".to_owned(), Value::Object(component_scan.defs)),
            ("sets".to_owned(), component_scan.sets),
        ])),
    );
    root.insert(
        "scene".to_owned(),
        Value::Object(Map::from_iter([("nodes".to_owned(), Value::Object(nodes))])),
    );

    let mut value = Value::Object(root);
    if manifest.schema_version < SCHEMA_VERSION {
        value = migrate(value, manifest.schema_version, SCHEMA_VERSION)?;
    }
    let assets = read_assets(dir, json_override, loaded_files)?;
    fanta_doc::prepare_doc_json(&mut value)
        .map_err(|error| FormatError::InvalidProjectTree(format!("{}: {error}", dir.display())))?;
    // Consume the assembled values so geometry JSON is released as typed
    // paths are built. The manifest gate and migration above already establish
    // the current schema; path-aware deserialization retains field diagnostics.
    let mut doc = serde_path_to_error::deserialize::<_, Doc>(value)
        .map_err(|error| FormatError::InvalidProjectTree(format!("{}: {error}", dir.display())))?;
    doc.scene.rebuild_child_index();
    doc.scene.validate().map_err(|error| {
        FormatError::InvalidProjectTree(format!(
            "{}: {}",
            dir.display(),
            fanta_doc::DocLoadError::InvalidScene(error)
        ))
    })?;
    doc.pending_layout = pending_layout;
    Ok((doc, assets))
}

/// Scan `pages/`: collect every page's nodes into `nodes` and rebuild the
/// `doc.pages` array by sorting the page headers on their `order` field
/// (dir name breaks ties, for stability). `_loose` contributes nodes only.
///
/// Identity comes from the header: v3 stores the page's id in `page.json`;
/// v2/v1 trees (slug-less, id-named directories) fall back to parsing the
/// directory name and upgrade on the next full-overwrite save.
///
/// A save that fails between its write and prune phases can leave TWO
/// directories for the same page (the pre-rename dir plus its replacement).
/// Directories are deduped by page id via [`select_design_winners`]; a losing
/// directory contributes nothing — no `doc.pages` entry and, crucially, no
/// nodes, so its stale batch can never overwrite the fresh dir's nodes in the
/// shared scene map.
#[expect(
    clippy::too_many_arguments,
    reason = "combined loading adds optional exact-byte evidence to the existing reader context"
)]
fn read_pages(
    dir: &Path,
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
    mut source_identities: Option<&mut LoadedSourceIdentities>,
    mut loaded_files: Option<&mut LoadedFileHashes>,
) -> Result<Value> {
    struct PageDir {
        id: NodeId,
        id_from_header: bool,
        entry: PathBuf,
        name: String,
        header: Value,
        header_hash: Option<[u8; 32]>,
    }
    let pages_dir = dir.join(PAGES_DIR);
    let mut pages: Vec<(u64, String, Value)> = Vec::new();
    if pages_dir.is_dir() {
        let mut candidates: Vec<PageDir> = Vec::new();
        for entry in sorted_entries(&pages_dir)? {
            if !entry.is_dir() {
                continue;
            }
            let name = dir_name(&entry)?;
            if name == LOOSE_DIR {
                // Orphans are kept as per-node JSON (they need not form a tree).
                read_nodes_into(&entry.join(NODES_DIR), json_override, nodes, pending_layout)?;
                continue;
            }
            if !entry.join(PAGE_JSON).is_file() {
                continue;
            }
            let (header, header_hash) = read_project_json_with_hash(
                &entry.join(PAGE_JSON),
                json_override,
                loaded_files.is_some(),
            )?;
            let (id, id_from_header) = match header.get("id").and_then(Value::as_str) {
                Some(id) => {
                    let id = id.parse().map_err(|e| {
                        FormatError::InvalidProjectTree(format!(
                            "page {name}: page.json id {id:?} is not a NodeId: {e}"
                        ))
                    })?;
                    (id, true)
                }
                // No header id: a v2 tree names the dir by the id — and a
                // hand-edited v3 page.json that dropped the id still recovers
                // it from the sidecar's root entry (pre-order first = the
                // page root). Headers are metadata, not identity of record.
                None => match name
                    .parse::<NodeId>()
                    .ok()
                    .or_else(|| sidecar_root_id(&entry))
                {
                    Some(id) => (id, false),
                    None => {
                        return Err(FormatError::InvalidProjectTree(format!(
                            "page dir {name:?}: no id in page.json, the dir name, or the sidecar"
                        )));
                    }
                },
            };
            candidates.push(PageDir {
                id,
                id_from_header,
                entry,
                name,
                header,
                header_hash,
            });
        }
        let ranked: Vec<(NodeId, bool, &Path)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.id,
                    candidate.id_from_header,
                    candidate.entry.as_path(),
                )
            })
            .collect();
        let winners = select_design_winners(&ranked);
        drop(ranked);
        for (index, candidate) in candidates.into_iter().enumerate() {
            if !winners.contains(&index) {
                continue;
            }
            if let Some(files) = &mut loaded_files {
                files.insert(candidate.entry.join(PAGE_JSON), candidate.header_hash);
            }
            // A page always has at least its root node, so a missing `.fnx`
            // (with no `nodes/` fallback) in a v2 tree is corruption, not empty.
            let decoded_root = read_design_nodes(
                &candidate.entry,
                PAGE_FNX,
                PAGE_IDS,
                version,
                true,
                source_override,
                json_override,
                refs,
                nodes,
                pending_layout,
                source_identities.as_deref_mut(),
            )?;
            if let Some(decoded_root) = decoded_root
                && decoded_root != candidate.id
            {
                return Err(FormatError::InvalidProjectTree(format!(
                    "page {}: page.json id {} differs from FNX root {decoded_root}",
                    candidate.name, candidate.id
                )));
            }
            // A page root is an unsized, unclipped group by invariant — its
            // background is the canvas color filling the whole viewport, and
            // sizing it would clip the page and demote that background to a
            // frame fill. Authored `width`/`height` sugar (or a stray
            // `clip_size`) on the root element must not break that. A root of
            // any other TAG is rejected here with a clear error: the page id
            // is referenced across the project, so decoding a type-flipped
            // node under it would misbind references (or fail the whole doc
            // with a cryptic missing-field error).
            if let Some(Value::Object(root)) = nodes.get_mut(&json_key(&candidate.id)?) {
                match root.get("type").and_then(Value::as_str) {
                    Some("group") => {
                        root.remove("clip_size");
                        root.remove("local_size");
                    }
                    ty => {
                        let found = ty
                            .and_then(fanta_fnx::tag_for_type)
                            .map_or_else(|| format!("type {ty:?}"), |tag| format!("<{tag}>"));
                        return Err(FormatError::InvalidProjectTree(format!(
                            "page {}: the root element of page.fnx must be a <Frame>, found {found}",
                            candidate.name
                        )));
                    }
                }
            }
            let name = candidate.name;
            let order = candidate
                .header
                .get("order")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    FormatError::InvalidProjectTree(format!("page {name}: page.json missing order"))
                })?;
            pages.push((order, name, serde_json::to_value(candidate.id)?));
        }
    }
    pages.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    Ok(Value::Array(
        pages.into_iter().map(|(_, _, id)| id).collect(),
    ))
}

/// Resolve duplicate design directories to one winner per design id.
///
/// A save that fails between writing a renamed design's new directory and
/// pruning the old one leaves both on disk claiming the same id. Preference,
/// in order:
/// 1. a directory whose id came from its JSON header (v3) beats one whose id
///    was parsed from the directory name (v2) — the header-carrying layout is
///    the newer write;
/// 2. the directory with the newest file mtime beats an older one — a crash
///    leftover predates the replacement that superseded it;
/// 3. the first directory in the (sorted) candidate order — arbitrary, but
///    deterministic.
/// Returns the winning indices; each losing directory is logged and must be
/// skipped entirely by the caller.
pub(super) fn select_design_winners<Id: Ord + Copy>(
    candidates: &[(Id, bool, &Path)],
) -> HashSet<usize> {
    let mut groups: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (index, (id, _, _)) in candidates.iter().enumerate() {
        groups.entry(*id).or_default().push(index);
    }
    let mut winners = HashSet::new();
    for group in groups.values() {
        if let [only] = group.as_slice() {
            winners.insert(*only);
            continue;
        }
        let best = group.iter().copied().max_by_key(|&index| {
            let (_, id_from_header, dir) = candidates[index];
            (id_from_header, newest_mtime(dir), std::cmp::Reverse(index))
        });
        let Some(best) = best else {
            continue;
        };
        for &index in group {
            if index != best {
                tracing::warn!(
                    target: "fanta::format",
                    "two design dirs claim one id (a crashed save's leftover): ignoring stale {} in favor of {}",
                    candidates[index].2.display(),
                    candidates[best].2.display(),
                );
            }
        }
        winners.insert(best);
    }
    winners
}

/// Newest modification time among the files directly inside `dir` — the
/// freshness signal for duplicate-dir resolution. Unreadable entries simply
/// don't contribute; freshness is best-effort, never an error.
fn newest_mtime(dir: &Path) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) {
                newest = newest.max(modified);
            }
        }
    }
    newest
}

/// The header half of a `components/` scan: every winning directory's def (the
/// `ComponentLibrary` JSON) plus the directories whose `master.fnx` still
/// needs materializing. Split from node reading so the defs can seed the
/// name-reference table BEFORE any design source is decoded.
struct ComponentScan {
    defs: Map<String, Value>,
    sets: Value,
    /// Winning component directories, in def order — the set
    /// [`read_component_masters`] materializes.
    winner_dirs: Vec<(PathBuf, Option<NodeId>)>,
}

/// Scan `components/`: `sets.json` plus one `def.json` per component
/// directory. Node piles are NOT read here — see [`ComponentScan`].
///
/// The `def.json` id is authoritative (dir names are v3 slugs); id-named v2
/// directories whose def somehow lacks an id fall back to the dir name.
///
/// Like [`read_pages`], directories are deduped by component id (a failed
/// slug-changing save can leave the old and the new dir side by side); losing
/// directories contribute neither a def nor nodes — see
/// [`select_design_winners`].
fn scan_components(
    dir: &Path,
    json_override: Option<&JsonSourceOverride<'_>>,
) -> Result<ComponentScan> {
    struct ComponentDir {
        id: ComponentId,
        id_from_header: bool,
        entry: PathBuf,
        def: Value,
    }
    let comp_dir = dir.join(COMPONENTS_DIR);
    let mut defs = Map::new();
    let mut sets = json!({});
    let mut winner_dirs = Vec::new();
    if comp_dir.is_dir() {
        // v5 keeps a variant set's variants inside its folder beside its
        // `set.json`; v4 kept them flat with every set in `sets.json`.
        let scanned = super::component_dirs::scan_component_dirs(dir)?;
        let set_files = scanned
            .set_files
            .iter()
            .map(|path| Ok((path.clone(), read_project_json(path, json_override)?)))
            .collect::<Result<Vec<_>>>()?;
        sets = super::component_dirs::merge_set_files(&set_files)?;
        let mut candidates: Vec<ComponentDir> = Vec::new();
        for entry in scanned.component_dirs {
            let name = dir_name(&entry)?;
            let def = read_project_json(&entry.join(DEF_JSON), json_override)?;
            let (id, id_from_header) = match def.get("id").and_then(Value::as_str) {
                Some(key) => {
                    let id = id_from_key(key).map_err(|e| {
                        FormatError::InvalidProjectTree(format!(
                            "component {name}: def.json id is not a ComponentId: {e}"
                        ))
                    })?;
                    (id, true)
                }
                None => {
                    let id = name.parse().map_err(|e| {
                        FormatError::InvalidProjectTree(format!(
                            "component dir {name:?} is not a ComponentId and def.json has no id: {e}"
                        ))
                    })?;
                    (id, false)
                }
            };
            candidates.push(ComponentDir {
                id,
                id_from_header,
                entry,
                def,
            });
        }
        let ranked: Vec<(ComponentId, bool, &Path)> = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.id,
                    candidate.id_from_header,
                    candidate.entry.as_path(),
                )
            })
            .collect();
        let winners = select_design_winners(&ranked);
        drop(ranked);
        for (index, candidate) in candidates.into_iter().enumerate() {
            if !winners.contains(&index) {
                continue;
            }
            let expected_root = candidate
                .def
                .get("root")
                .map(|value| {
                    serde_json::from_value::<NodeId>(value.clone()).map_err(|error| {
                        FormatError::InvalidProjectTree(format!(
                            "{}: def.json root is invalid: {error}",
                            candidate.entry.display()
                        ))
                    })
                })
                .transpose()?;
            defs.insert(json_key(&candidate.id)?, candidate.def);
            winner_dirs.push((candidate.entry, expected_root));
        }
    }
    Ok(ComponentScan {
        defs,
        sets,
        winner_dirs,
    })
}

/// Materialize the master subtrees of a completed [`scan_components`] pass
/// into the shared scene map. Runs after pages so both decode with the same
/// fully-populated reference table.
#[expect(
    clippy::too_many_arguments,
    reason = "combined loading adds optional exact-byte evidence to the existing reader context"
)]
fn read_component_masters(
    scan: &ComponentScan,
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
    source_identities: Option<&mut LoadedSourceIdentities>,
) -> Result<()> {
    if version < 5 || scan.winner_dirs.len() < 256 {
        return read_component_masters_serial(
            scan,
            version,
            source_override,
            json_override,
            refs,
            nodes,
            pending_layout,
            source_identities,
        );
    }
    type DecodedMaster = (Map<String, Value>, BTreeSet<NodeId>);
    let collect_identities = source_identities.is_some();
    let decoded = std::thread::scope(|scope| {
        let mut workers = Vec::new();
        let mut spawn_error = None;
        let chunk_size = scan.winner_dirs.len().div_ceil(4);
        for (worker, chunk) in scan.winner_dirs.chunks(chunk_size).enumerate() {
            let spawned = std::thread::Builder::new()
                .name(format!("fanta-master-read-{worker}"))
                .spawn_scoped(scope, move || {
                    let mut decoded = Vec::<DecodedMaster>::with_capacity(chunk.len());
                    let mut identities = LoadedSourceIdentities::new();
                    for design in chunk {
                        let mut nodes = Map::new();
                        let mut pending = BTreeSet::new();
                        read_component_master(
                            design,
                            version,
                            source_override,
                            json_override,
                            refs,
                            &mut nodes,
                            &mut pending,
                            collect_identities.then_some(&mut identities),
                        )?;
                        decoded.push((nodes, pending));
                    }
                    Ok::<_, FormatError>((decoded, identities))
                });
            match spawned {
                Ok(handle) => workers.push(handle),
                Err(error) => {
                    spawn_error = Some(FormatError::InvalidProjectTree(format!(
                        "cannot start component reader: {error}"
                    )));
                    break;
                }
            }
        }
        let mut decoded = Vec::with_capacity(workers.len());
        for worker in workers {
            decoded.push(match worker.join() {
                Ok(result) => result,
                Err(_) => Err(FormatError::InvalidProjectTree(
                    "component reader panicked".into(),
                )),
            });
        }
        if let Some(error) = spawn_error {
            decoded.push(Err(error));
        }
        // Join every worker before returning an error, and select errors by
        // source order rather than the nondeterministic completion order.
        decoded.into_iter().collect::<Result<Vec<_>>>()
    })?;
    let mut source_identities = source_identities;
    for (chunk, identities) in decoded {
        for (decoded_nodes, omissions) in chunk {
            for (key, node) in decoded_nodes {
                // A later source overwrites both the node and any earlier
                // omission marker, exactly as the serial design reader does.
                pending_layout.remove(&id_from_key::<NodeId>(&key)?);
                nodes.insert(key, node);
            }
            pending_layout.extend(omissions);
        }
        if let Some(source_identities) = &mut source_identities {
            source_identities.extend(identities);
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "combined loading adds optional exact-byte evidence to the existing reader context"
)]
fn read_component_masters_serial(
    scan: &ComponentScan,
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
    mut source_identities: Option<&mut LoadedSourceIdentities>,
) -> Result<()> {
    for design in &scan.winner_dirs {
        read_component_master(
            design,
            version,
            source_override,
            json_override,
            refs,
            nodes,
            pending_layout,
            source_identities.as_deref_mut(),
        )?;
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "combined loading adds optional exact-byte evidence to the existing reader context"
)]
fn read_component_master(
    (entry, expected_root): &(PathBuf, Option<NodeId>),
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
    source_identities: Option<&mut LoadedSourceIdentities>,
) -> Result<()> {
    // A dangling definition can legitimately have no remaining master source.
    let decoded_root = read_design_nodes(
        entry,
        MASTER_FNX,
        MASTER_IDS,
        version,
        false,
        source_override,
        json_override,
        refs,
        nodes,
        pending_layout,
        source_identities,
    )?;
    if let (Some(expected_root), Some(decoded_root)) = (expected_root, decoded_root)
        && *expected_root != decoded_root
    {
        return Err(FormatError::InvalidProjectTree(format!(
            "{}: def.json root {expected_root} differs from FNX root {decoded_root}",
            entry.display()
        )));
    }
    Ok(())
}

/// Read a design's nodes into the flat scene map. v2 decodes the `.fnx` source
/// and its `.ids` sidecar; the v1 fallback reads the per-node `nodes/` JSON pile
/// (so older project trees still load and upgrade to `.fnx` on the next save).
#[expect(
    clippy::too_many_arguments,
    reason = "internal plumbing fn; a params struct would only rename the seven call-site facts"
)]
fn read_design_nodes(
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    version: u32,
    required: bool,
    source_override: Option<&FnxSourceOverride<'_>>,
    json_override: Option<&JsonSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
    source_identities: Option<&mut LoadedSourceIdentities>,
) -> Result<Option<NodeId>> {
    let fnx_path = design_dir.join(fnx_name);
    let source_override = source_override.filter(|source| source.path == fnx_path);
    if source_override.is_none() && !fnx_path.is_file() {
        if let Some(source_identities) = source_identities {
            let sidecar_hash = match std::fs::read(design_dir.join(ids_name)) {
                Ok(bytes) => Some(Sha256::digest(&bytes).into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            source_identities.insert(
                fnx_path.clone(),
                LoadedSourceIdentity {
                    source_hash: None,
                    sidecar_hash,
                    current: false,
                },
            );
        }
        // No `.fnx`. A `nodes/` dir means either a v1 tree or a v2 design that
        // fell back to per-node JSON (an un-encodable bucket) — read it either
        // way. With neither file present, a `required` design (a page) in a v2
        // tree is corruption (a dropped `.fnx` from a bad merge/partial write),
        // NOT a silently-empty page that the next save would erase forever.
        let nodes_dir = design_dir.join(NODES_DIR);
        if nodes_dir.is_dir() {
            read_nodes_into(&nodes_dir, json_override, nodes, pending_layout)?;
            return Ok(None);
        }
        if required && version >= 2 {
            return Err(FormatError::MissingFile {
                name: fnx_path.display().to_string(),
            });
        }
        return Ok(None);
    }
    let text = match source_override {
        Some(source_override) => std::borrow::Cow::Borrowed(source_override.source),
        None => std::borrow::Cow::Owned(std::fs::read_to_string(&fnx_path)?),
    };
    let mut root = fanta_fnx::parse_doc(&text).map_err(|error| {
        FormatError::InvalidProjectTree(format!("{}: {error}", fnx_path.display()))
    })?;
    let mut identity_current = false;
    let mut sidecar_hash = None;
    let sidecar: fanta_fnx::FnxSidecar = match source_override
        .and_then(|source_override| source_override.sidecar)
    {
        Some(sidecar) => reconcile_fnx_sidecar_from_tree(&text, &root, sidecar),
        None => {
            let sidecar_path = design_dir.join(ids_name);
            let bytes =
                if let Some(source) = json_override.filter(|source| source.path == sidecar_path) {
                    Some(serde_json::to_vec(source.value)?)
                } else {
                    match std::fs::read(&sidecar_path) {
                        Ok(bytes) => Some(bytes),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => return Err(error.into()),
                    }
                };
            if source_identities.is_some() {
                sidecar_hash = bytes.as_deref().map(|bytes| Sha256::digest(bytes).into());
            }
            persisted_fnx_sidecar_from_tree(
                &text,
                &root,
                bytes.as_deref(),
                source_identities.as_ref().map(|_| &mut identity_current),
            )
        }
    }
    .map_err(|error| FormatError::InvalidProjectTree(format!("{}: {error}", fnx_path.display())))?;
    if let Some(source_identities) = source_identities {
        source_identities.insert(
            fnx_path.clone(),
            LoadedSourceIdentity {
                source_hash: Some(Sha256::digest(text.as_bytes()).into()),
                sidecar_hash,
                current: identity_current,
            },
        );
    }
    fanta_fnx::desugar_refs(&mut root, refs)
        .map_err(|e| FormatError::InvalidProjectTree(format!("{}: {e}", fnx_path.display())))?;
    let decoded =
        fanta_fnx::nodes_from_tree_owned(root, &sidecar.ids, sidecar.root_parent.as_deref())
            .map_err(|e| FormatError::InvalidProjectTree(format!("{}: {e}", fnx_path.display())))?;
    let decoded_root: NodeId = serde_json::from_value(
        decoded
            .first()
            .and_then(|node| node.get("id"))
            .cloned()
            .ok_or_else(|| {
                FormatError::InvalidProjectTree(format!(
                    "{}: decoded FNX has no root id",
                    fnx_path.display()
                ))
            })?,
    )
    .map_err(|error| {
        FormatError::InvalidProjectTree(format!(
            "{}: invalid FNX root id: {error}",
            fnx_path.display()
        ))
    })?;
    let omissions =
        pending_layout_from_nodes(&decoded, (fnx_name == PAGE_FNX).then_some(decoded_root))?;
    for mut node in decoded {
        backfill_required_geometry(&mut node);
        let key = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                FormatError::InvalidProjectTree(format!(
                    "{}: decoded node missing id",
                    fnx_path.display()
                ))
            })?
            .to_owned();
        pending_layout.remove(&serde_json::from_value::<NodeId>(Value::String(
            key.clone(),
        ))?);
        nodes.insert(key, node);
    }
    pending_layout.extend(omissions);
    Ok(Some(decoded_root))
}

pub(crate) fn pending_layout_from_nodes(
    nodes: &[Value],
    unsized_root: Option<NodeId>,
) -> Result<BTreeSet<NodeId>> {
    let by_id: BTreeMap<_, _> = nodes
        .iter()
        .filter_map(|node| Some((node.get("id")?.as_str()?, node)))
        .collect();
    let mut pending = BTreeSet::new();
    for node in nodes {
        let identifier: NodeId =
            serde_json::from_value(node.get("id").cloned().ok_or_else(|| {
                FormatError::InvalidProjectTree("source node missing id".into())
            })?)?;
        let missing = |key| node.get(key).is_none_or(Value::is_null);
        let missing_size = match node.get("type").and_then(Value::as_str) {
            Some("text") => missing("local_size"),
            Some("group") => {
                Some(identifier) != unsized_root
                    && node.get("auto_layout").is_some_and(Value::is_object)
                    && missing("clip_size")
                    && missing("local_size")
            }
            _ => false,
        };
        let missing_flow_position = missing("transform")
            && node
                .get("parent")
                .and_then(Value::as_str)
                .and_then(|parent| by_id.get(parent))
                .and_then(|parent| parent.get("auto_layout"))
                .filter(|layout| layout.is_object())
                .is_some_and(|layout| {
                    layout.get("child_layout").and_then(Value::as_bool) == Some(false)
                        || node
                            .get("layout_child")
                            .and_then(|child| child.get("absolute"))
                            .and_then(Value::as_bool)
                            != Some(true)
                });
        if missing_size || missing_flow_position {
            pending.insert(identifier);
        }
    }
    Ok(pending)
}

/// Backfill `local_size` on decoded nodes whose payload struct requires it.
/// `.fnx` source is hand- and agent-editable; an authored `<Text>` (or media /
/// `<Instance>`) element that omits both `local_size` and the `width`/`height`
/// sugar must still reassemble instead of failing the whole design with
/// "missing field `local_size`". Media nodes fall back to their intrinsic
/// `natural_size`; text estimates a box from its style and content; everything
/// else gets a visible placeholder box the user can resize.
/// Replace geometry that is not a finite number. JSON has no NaN/Infinity, so
/// `serde_json` writes a non-finite `f64` as `null`, and a `null` where the schema
/// wants a number fails the whole project on read. Projects written before the
/// importer's guards (`fanta-fig-interop` `read_transform`) carry such values on
/// degenerate Figma nodes. A broken transform becomes identity and a broken size
/// is dropped (an optional size reads as unset; a required one is backfilled).
/// Returns whether anything was repaired.
pub(crate) fn repair_non_finite_geometry(node: &mut Value) -> bool {
    let Some(obj) = node.as_object_mut() else {
        return false;
    };
    fn broken(obj: &Map<String, Value>, key: &str) -> bool {
        obj.get(key).and_then(Value::as_array).is_some_and(|items| {
            !items
                .iter()
                .all(|value| value.as_f64().is_some_and(f64::is_finite))
        })
    }
    let mut repaired = Vec::new();
    if broken(obj, "transform") {
        obj.insert(
            "transform".to_owned(),
            json!([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        );
        repaired.push("transform");
    }
    for key in ["clip_size", "local_size", "natural_size"] {
        if broken(obj, key) {
            obj.remove(key);
            repaired.push(key);
        }
    }
    // A Figma boolean whose bbox fallback had a non-finite size: its rectangle
    // path carries the same nulls. Zero them, leaving an empty shape.
    let mut path_repaired = false;
    if let Some(segments) = obj
        .get_mut("path")
        .and_then(|path| path.get_mut("segments"))
        .and_then(Value::as_array_mut)
    {
        for point in segments
            .iter_mut()
            .filter_map(Value::as_object_mut)
            .flat_map(|segment| segment.values_mut())
            .filter_map(Value::as_array_mut)
        {
            for coordinate in point.iter_mut() {
                if !coordinate.as_f64().is_some_and(f64::is_finite) {
                    *coordinate = json!(0.0);
                    path_repaired = true;
                }
            }
        }
    }
    if path_repaired {
        repaired.push("path");
    }
    if repaired.is_empty() {
        return false;
    }
    let id = obj.get("id").and_then(Value::as_str).unwrap_or("<no id>");
    tracing::warn!(
        target: "fanta::format",
        node = id,
        fields = ?repaired,
        "replaced non-finite geometry"
    );
    true
}

pub(crate) fn backfill_required_geometry(node: &mut Value) {
    repair_non_finite_geometry(node);
    let Some(obj) = node.as_object_mut() else {
        return;
    };
    let needs_local_size = matches!(
        obj.get("type").and_then(Value::as_str),
        Some(
            "text"
                | "bitmap"
                | "video"
                | "audio"
                | "node_graph"
                | "model3d"
                | "ai_artifact"
                | "embed"
                | "instance"
        )
    );
    if !needs_local_size || obj.get("local_size").is_some_and(|v| !v.is_null()) {
        return;
    }
    let natural = obj
        .get("natural_size")
        .and_then(Value::as_array)
        .and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?]))
        .filter(|[w, h]| *w >= 1.0 && *h >= 1.0);
    let size = natural.unwrap_or_else(|| {
        if obj.get("type").and_then(Value::as_str) == Some("text") {
            estimate_text_box(obj)
        } else {
            [200.0, 100.0]
        }
    });
    obj.insert("local_size".to_owned(), json!(size));
}

/// A crude but stable text-box estimate from the node's own style + content:
/// wide enough for the longest line at ~0.6em per character, tall enough for
/// every line at the style's line height. Only used when the source omitted
/// the size entirely; the canvas layout takes over once the node is edited.
fn estimate_text_box(obj: &Map<String, Value>) -> [f64; 2] {
    let size_px = obj
        .get("style")
        .and_then(|s| s.get("size_px"))
        .and_then(Value::as_f64)
        .filter(|s| s.is_finite() && *s > 0.0)
        .unwrap_or(16.0);
    let line_height = obj
        .get("style")
        .and_then(|s| s.get("line_height"))
        .and_then(Value::as_f64)
        .filter(|l| l.is_finite() && *l > 0.0)
        .unwrap_or(1.2);
    let content = obj.get("content").and_then(Value::as_str).unwrap_or("");
    let lines = content.lines().count().max(1) as f64;
    let longest = content
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    [
        (longest * size_px * 0.6).max(size_px),
        lines * size_px * line_height,
    ]
}

pub(super) fn reconcile_fnx_sidecar(
    _fnx_path: &Path,
    source: &str,
    sidecar: &fanta_fnx::FnxSidecar,
) -> Result<fanta_fnx::FnxSidecar> {
    let root = fanta_fnx::parse_doc(source)
        .map_err(|error| FormatError::InvalidProjectTree(error.to_string()))?;
    reconcile_fnx_sidecar_from_tree(source, &root, sidecar)
}

pub(super) fn read_fnx_sidecar(
    source_path: &Path,
    source: &str,
    sidecar_path: &Path,
) -> Result<fanta_fnx::FnxSidecar> {
    let bytes = match std::fs::read(sidecar_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    persisted_fnx_sidecar(source, bytes.as_deref()).map_err(|error| {
        FormatError::InvalidProjectTree(format!("{}: {error}", source_path.display()))
    })
}

pub(crate) fn persisted_fnx_sidecar(
    source: &str,
    bytes: Option<&[u8]>,
) -> Result<fanta_fnx::FnxSidecar> {
    let root = fanta_fnx::parse_doc(source)
        .map_err(|error| FormatError::InvalidProjectTree(error.to_string()))?;
    persisted_fnx_sidecar_from_tree(source, &root, bytes, None)
}

/// Read every `<node-id>.json` in `nodes_dir` into the flat scene map, keyed
/// by the id's serde form. The filename is the id of record.
fn read_nodes_into(
    nodes_dir: &Path,
    json_override: Option<&JsonSourceOverride<'_>>,
    nodes: &mut Map<String, Value>,
    pending_layout: &mut BTreeSet<NodeId>,
) -> Result<()> {
    if !nodes_dir.is_dir() {
        return Ok(());
    }
    for path in sorted_entries(nodes_dir)? {
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let id: NodeId = stem.parse().map_err(|e| {
            FormatError::InvalidProjectTree(format!("node file {stem:?} is not a NodeId: {e}"))
        })?;
        pending_layout.remove(&id);
        nodes.insert(json_key(&id)?, read_project_json(&path, json_override)?);
    }
    Ok(())
}

/// Scan `assets/**` and parse each filename stem back into an [`AssetId`].
/// Family folder and extension are projections only; files whose stem isn't an
/// id (OS noise like `.DS_Store`) are skipped with a warning.
fn read_assets(
    dir: &Path,
    json_override: Option<&JsonSourceOverride<'_>>,
    mut loaded_files: Option<&mut LoadedFileHashes>,
) -> Result<BTreeMap<AssetId, Vec<u8>>> {
    let mut assets = BTreeMap::new();
    let assets_dir = dir.join(ASSETS_DIR);
    if assets_dir.is_dir() {
        let index_path = assets_dir.join(super::media::ASSET_INDEX_FILE);
        let index = match json_override.filter(|source| source.path == index_path) {
            Some(source) => Some(super::media::read_asset_index(&serde_json::to_vec(
                source.value,
            )?)?),
            None => {
                let bytes = index_path
                    .is_file()
                    .then(|| std::fs::read(&index_path))
                    .transpose()?;
                if let Some(files) = &mut loaded_files {
                    files.insert(
                        index_path,
                        bytes.as_deref().map(|bytes| Sha256::digest(bytes).into()),
                    );
                }
                bytes
                    .map(|bytes| super::media::read_asset_index(&bytes))
                    .transpose()?
            }
        };
        collect_assets(&assets_dir, &mut assets, index.as_ref(), loaded_files)?;
        if let Some(index) = index {
            index.verify_complete(&assets)?;
        }
    }
    Ok(assets)
}

fn collect_assets(
    dir: &Path,
    assets: &mut BTreeMap<AssetId, Vec<u8>>,
    index: Option<&super::media::AssetIndex>,
    mut loaded_files: Option<&mut LoadedFileHashes>,
) -> Result<()> {
    for path in sorted_entries(dir)? {
        if path.is_dir() {
            collect_assets(&path, assets, index, loaded_files.as_deref_mut())?;
            continue;
        }
        if path.parent() == Some(dir)
            && path
                .file_name()
                .is_some_and(|name| name == super::media::ASSET_INDEX_FILE)
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(id) = stem.parse::<AssetId>() else {
            tracing::warn!(path = %path.display(), "skipping non-asset file in assets/");
            continue;
        };
        let bytes = std::fs::read(&path)?;
        if assets.contains_key(&id) {
            return Err(FormatError::InvalidProjectTree(format!(
                "duplicate asset id {id} at {}",
                path.display()
            )));
        }
        if let Some(index) = index {
            index.verify_entry(id, &bytes)?;
        }
        if let Some(files) = &mut loaded_files {
            files.insert(path, Some(Sha256::digest(&bytes).into()));
        }
        assets.insert(id, bytes);
    }
    Ok(())
}

/// Final path component as UTF-8, or an [`FormatError::InvalidProjectTree`].
fn dir_name(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            FormatError::InvalidProjectTree(format!("non-UTF-8 directory name: {}", path.display()))
        })
}

// ---- source resolvers -------------------------------------------------------
//
// v3 directory names are slugs, so the editor can no longer derive a design's
// source path from its id. These resolvers scan the design directories and
// take identity from the JSON headers (`page.json` "id" / `def.json`), falling
// back to the v2 dir-name-as-id convention, so callers never need to know the
// slug rules.

/// The design a project `.fnx` source belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopedDesign {
    Page(NodeId),
    Component(ComponentId),
}

/// The identity of the page directory at `design_dir`: `page.json`'s `"id"`
/// (v3), or the directory name parsed as a `NodeId` (v2). `None` when neither
/// yields an id.
pub fn page_id_of_dir(design_dir: &Path) -> Option<NodeId> {
    let header: Option<Value> = std::fs::read_to_string(design_dir.join(PAGE_JSON))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    if let Some(id) = header
        .as_ref()
        .and_then(|header| header.get("id"))
        .and_then(Value::as_str)
    {
        return id.parse().ok();
    }
    design_dir
        .file_name()?
        .to_str()?
        .parse()
        .ok()
        .or_else(|| sidecar_root_id(design_dir))
}

/// The page root id recovered from the `.ids` sidecar (its pre-order first
/// entry IS the design root) — the identity of last resort when a hand-edited
/// `page.json` dropped the `id` and the dir is a v3 slug.
fn sidecar_root_id(design_dir: &Path) -> Option<NodeId> {
    let sidecar: fanta_fnx::FnxSidecar = std::fs::read_to_string(design_dir.join(PAGE_IDS))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())?;
    let root = sidecar.ids.first()?;
    id_from_key(&root.id).ok()
}

/// The identity of the component directory at `design_dir`: `def.json`'s `id`
/// (authoritative), or the directory name parsed as a `ComponentId` (v2).
pub fn component_id_of_dir(design_dir: &Path) -> Option<ComponentId> {
    let def: Option<Value> = std::fs::read_to_string(design_dir.join(DEF_JSON))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    if let Some(key) = def
        .as_ref()
        .and_then(|def| def.get("id"))
        .and_then(Value::as_str)
    {
        return id_from_key(key).ok();
    }
    design_dir.file_name()?.to_str()?.parse().ok()
}

fn locate_design_source<Id: Copy + PartialEq>(
    designs_dir: &Path,
    fnx_name: &str,
    target: Id,
    id_of_dir: impl Fn(&Path) -> Option<Id>,
) -> Option<PathBuf> {
    let entries = std::fs::read_dir(designs_dir).ok()?;
    for entry in entries.flatten() {
        let design_dir = entry.path();
        if !design_dir.is_dir() {
            continue;
        }
        if id_of_dir(&design_dir) == Some(target) {
            let fnx_path = design_dir.join(fnx_name);
            if fnx_path.is_file() {
                return Some(fnx_path);
            }
        }
    }
    None
}

/// Find the `page.fnx` source of `page` under `project_root`, whatever the
/// page's directory is named (v3 slug or v2 id). `None` when the page has no
/// source file on disk.
pub fn locate_page_source(project_root: &Path, page: NodeId) -> Option<PathBuf> {
    locate_design_source(
        &project_root.join(PAGES_DIR),
        PAGE_FNX,
        page,
        page_id_of_dir,
    )
}

/// Find the `master.fnx` source of `component` under `project_root`, whatever
/// the component's directory is named (v3 slug or v2 id), standalone or a
/// variant inside its set's folder (v5).
pub fn locate_master_source(project_root: &Path, component: ComponentId) -> Option<PathBuf> {
    super::component_dirs::scan_component_dirs(project_root)
        .ok()?
        .component_dirs
        .into_iter()
        .find(|dir| component_id_of_dir(dir) == Some(component))
        .map(|dir| dir.join(MASTER_FNX))
        .filter(|path| path.is_file())
}

/// Resolve which project and design a `.fnx` source path belongs to.
///
/// Given a path ending in `pages/<dir>/page.fnx` or
/// `components/<dir>/master.fnx`, returns the project root and the design's
/// id, read from the adjacent header (`page.json` / `def.json`) with the v2
/// dir-name fallback. `None` when the path has the wrong shape, the root is
/// not a project directory, or no id can be resolved.
pub fn page_scope_of_source(path: &Path) -> Option<(PathBuf, ScopedDesign)> {
    let file_name = path.file_name()?.to_str()?;
    let design_dir = path.parent()?;
    let mut designs_dir = design_dir.parent()?;
    // A variant's master sits one folder deeper, in its set's folder (v5).
    if file_name == MASTER_FNX
        && designs_dir.join(SET_JSON).is_file()
        && let Some(components) = designs_dir.parent()
        && components
            .file_name()
            .is_some_and(|name| name == COMPONENTS_DIR)
    {
        designs_dir = components;
    }
    let designs_dir_name = designs_dir.file_name()?.to_str()?;
    let project_root = designs_dir.parent()?;
    let design = match (designs_dir_name, file_name) {
        (n, f) if n == PAGES_DIR && f == PAGE_FNX => {
            ScopedDesign::Page(page_id_of_dir(design_dir)?)
        }
        (n, f) if n == COMPONENTS_DIR && f == MASTER_FNX => {
            ScopedDesign::Component(component_id_of_dir(design_dir)?)
        }
        _ => return None,
    };
    if !is_project_dir(project_root) {
        return None;
    }
    Some((project_root.to_path_buf(), design))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::node::{CanvasNode, GroupNode, NodeData, VectorNode};
    use fanta_doc::{Color, IndexKey, UnitInterval};

    fn large_master_fixture() -> std::result::Result<
        (tempfile::TempDir, ComponentScan, Vec<NodeId>, NodeId),
        Box<dyn std::error::Error>,
    > {
        let directory = tempfile::tempdir()?;
        let duplicate = NodeId::new();
        let mut roots = Vec::new();
        let mut winner_dirs = Vec::new();
        for index in 0..260 {
            let design = directory.path().join(format!("master-{index:04}"));
            std::fs::create_dir(&design)?;
            let root = NodeId::new();
            roots.push(root);
            winner_dirs.push((design.clone(), Some(root)));
            if index == 259 {
                continue;
            }
            let text = if matches!(index, 64 | 130 | 195) {
                duplicate
            } else {
                NodeId::new()
            };
            let size = if index % 3 == 0 || matches!(index, 130 | 195) {
                " local_size={[20, 10]}"
            } else {
                ""
            };
            let vector = NodeId::new();
            let source = format!(
                "<Frame id=\"{}\" name=\"Master {index}\"><Text id=\"{}\" name=\"Label {index}\" content=\"雪🙂 {index}\"{size} /><Rect id=\"{}\" name=\"Box\" width={{20}} height={{10}} /></Frame>",
                root.0, text.0, vector.0
            );
            let sidecar = persisted_fnx_sidecar(&source, None)?;
            std::fs::write(design.join(MASTER_FNX), source)?;
            std::fs::write(design.join(MASTER_IDS), serde_json::to_vec(&sidecar)?)?;
        }
        Ok((
            directory,
            ComponentScan {
                defs: Map::new(),
                sets: json!({}),
                winner_dirs,
            },
            roots,
            duplicate,
        ))
    }

    #[test]
    fn parallel_master_loading_matches_serial_order_overrides_and_layout_omissions()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (_directory, scan, roots, duplicate) = large_master_fixture()?;
        let source_path = scan
            .winner_dirs
            .get(200)
            .ok_or("override design")?
            .0
            .join(MASTER_FNX);
        let mut root = fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path)?)?;
        root.attrs
            .insert("name".into(), Value::from("Source override 雪🙂"));
        let source = fanta_fnx::print_doc("Overridden", &root);
        let source_override = FnxSourceOverride {
            path: &source_path,
            source: &source,
            sidecar: None,
        };
        let ids_path = scan
            .winner_dirs
            .get(65)
            .ok_or("identity override")?
            .0
            .join(MASTER_IDS);
        let mut ids: Value = serde_json::from_slice(&std::fs::read(&ids_path)?)?;
        *ids.pointer_mut("/ids/1/index").ok_or("fractional order")? = json!(0.9999999999999999);
        let json_override = JsonSourceOverride {
            path: &ids_path,
            value: &ids,
        };
        let refs = fanta_fnx::RefTable::new(true);
        let initial = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let mut previous_duplicate = CanvasNode::new(NodeData::Group(GroupNode::default()));
        previous_duplicate.id = duplicate;
        let initial_nodes = Map::from_iter([
            (initial.id.0.to_string(), serde_json::to_value(&initial)?),
            (
                duplicate.0.to_string(),
                serde_json::to_value(previous_duplicate)?,
            ),
        ]);
        let initial_pending =
            BTreeSet::from([initial.id, duplicate, *roots.get(200).ok_or("root")?]);
        let mut serial_nodes = initial_nodes.clone();
        let mut serial_pending = initial_pending.clone();
        read_component_masters_serial(
            &scan,
            5,
            Some(&source_override),
            Some(&json_override),
            &refs,
            &mut serial_nodes,
            &mut serial_pending,
            None,
        )?;
        let mut parallel_nodes = initial_nodes;
        let mut parallel_pending = initial_pending;
        read_component_masters(
            &scan,
            5,
            Some(&source_override),
            Some(&json_override),
            &refs,
            &mut parallel_nodes,
            &mut parallel_pending,
            None,
        )?;
        assert_eq!(parallel_nodes, serial_nodes);
        assert_eq!(
            serde_json::to_vec(&parallel_nodes)?,
            serde_json::to_vec(&serial_nodes)?
        );
        assert_eq!(parallel_pending, serial_pending);
        assert!(parallel_pending.contains(&initial.id));
        assert!(
            !parallel_pending.contains(&duplicate),
            "the later sized source must clear an earlier omission"
        );
        assert!(!parallel_pending.contains(roots.get(200).ok_or("root")?));
        assert!(!parallel_nodes.contains_key(&roots.last().ok_or("dangling root")?.0.to_string()));
        assert_eq!(
            parallel_nodes
                .get(&duplicate.0.to_string())
                .and_then(|node| node.get("name")),
            Some(&Value::from("Label 195"))
        );
        let assemble = |nodes| -> std::result::Result<Doc, Box<dyn std::error::Error>> {
            let mut value = serde_json::to_value(Doc::new())?;
            *value.pointer_mut("/scene/nodes").ok_or("scene map")? = Value::Object(nodes);
            let mut document: Doc = serde_json::from_value(value)?;
            document.scene.rebuild_child_index();
            document.scene.validate()?;
            Ok(document)
        };
        let serial = assemble(serial_nodes)?;
        let parallel = assemble(parallel_nodes)?;
        assert_eq!(
            serde_json::to_value(&parallel.scene)?,
            serde_json::to_value(&serial.scene)?
        );
        assert_eq!(parallel.scene.roots(), serial.scene.roots());
        for root in roots {
            assert_eq!(
                parallel.scene.children_of(Some(root)),
                serial.scene.children_of(Some(root))
            );
        }
        Ok(())
    }

    #[test]
    fn parallel_master_loading_reports_the_earliest_invalid_source_across_chunks()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (_directory, scan, _, _) = large_master_fixture()?;
        for index in [7, 137] {
            let source = scan
                .winner_dirs
                .get(index)
                .ok_or("invalid design")?
                .0
                .join(MASTER_FNX);
            std::fs::write(source, "<Frame name=\"Broken\" opacity={invalid_json} />")?;
        }
        let refs = fanta_fnx::RefTable::new(true);
        let expected = read_component_masters_serial(
            &scan,
            5,
            None,
            None,
            &refs,
            &mut Map::new(),
            &mut BTreeSet::new(),
            None,
        )
        .expect_err("invalid serial source")
        .to_string();
        let mut nodes = Map::new();
        let mut pending = BTreeSet::new();
        let actual =
            read_component_masters(&scan, 5, None, None, &refs, &mut nodes, &mut pending, None)
                .expect_err("invalid parallel source")
                .to_string();
        assert_eq!(actual, expected);
        assert!(actual.contains("master-0007"), "{actual}");
        assert!(!actual.contains("master-0137"), "{actual}");
        assert!(
            nodes.is_empty() && pending.is_empty(),
            "failed workers cannot publish a partial scene"
        );
        Ok(())
    }

    #[test]
    fn exact_modern_sidecar_reuse_preserves_order_parent_and_duplicate_checks()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let root_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let child_id = "01ARZ3NDEKTSV4RRFFQ69G5FAW";
        let source = format!(
            "<Frame id=\"{root_id}\" name=\"Root\"><Text id=\"{child_id}\" name=\"Child\" content=\"雪\" future_hint={{[1,2,3]}} /></Frame>"
        );
        let parent = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
        let mut sidecar = fanta_fnx::reconcile_sidecar(
            &source,
            &fanta_fnx::FnxSidecar {
                root_parent: Some(parent.into()),
                ids: Vec::new(),
            },
            || NodeId::new().0.to_string(),
        )?;
        sidecar.ids.first_mut().ok_or("root identity")?.index = json!(0.125);
        sidecar.ids.get_mut(1).ok_or("child identity")?.index = json!(7.75);
        let parsed = fanta_fnx::parse_doc(&source)?;
        let expected = fanta_fnx::reconcile_sidecar_from_tree(&parsed, &sidecar, || {
            NodeId::new().0.to_string()
        })?;
        let bytes = serde_json::to_vec(&sidecar)?;
        let mut current = false;
        let actual =
            persisted_fnx_sidecar_from_tree(&source, &parsed, Some(&bytes), Some(&mut current))?;
        assert!(current);
        assert_eq!(actual, sidecar);
        assert_eq!(actual, expected);
        assert_eq!(actual.root_parent.as_deref(), Some(parent));

        let legacy = source.replace(&format!("id=\"{child_id}\""), "");
        let mut current = true;
        assert_eq!(
            persisted_fnx_sidecar_from_tree(
                &legacy,
                &fanta_fnx::parse_doc(&legacy)?,
                Some(&bytes),
                Some(&mut current),
            )?,
            sidecar
        );
        assert!(
            !current,
            "missing explicit identities require reconciliation"
        );
        assert!(persisted_fnx_sidecar(&legacy, None).is_err());

        let duplicate = source.replace(child_id, root_id);
        let mut duplicate_sidecar = sidecar;
        duplicate_sidecar.ids.get_mut(1).ok_or("child identity")?.id = root_id.into();
        let mut current = true;
        let error = persisted_fnx_sidecar_from_tree(
            &duplicate,
            &fanta_fnx::parse_doc(&duplicate)?,
            Some(&serde_json::to_vec(&duplicate_sidecar)?),
            Some(&mut current),
        )
        .expect_err("duplicate explicit ids must not take the exact-match path");
        assert!(!current);
        assert!(error.to_string().contains("duplicate explicit node id"));
        Ok(())
    }

    #[test]
    fn deferred_identity_generation_preserves_deterministic_missing_ids()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let source = "<Frame id=\"01ARZ3NDEKTSV4RRFFQ69G5FAV\" name=\"Root\"><Text id=\"01ARZ3NDEKTSV4RRFFQ69G5FAW\" name=\"Kept\" content=\"a\" /><Text name=\"New 雪\" content=\"b\" /></Frame>";
        let baseline = "<Frame id=\"01ARZ3NDEKTSV4RRFFQ69G5FAV\" name=\"Root\"><Text id=\"01ARZ3NDEKTSV4RRFFQ69G5FAW\" name=\"Kept\" content=\"a\" /></Frame>";
        let sidecar = fanta_fnx::reconcile_sidecar(
            baseline,
            &fanta_fnx::FnxSidecar {
                root_parent: Some("01ARZ3NDEKTSV4RRFFQ69G5FAX".into()),
                ids: Vec::new(),
            },
            || NodeId::new().0.to_string(),
        )?;
        let parsed = fanta_fnx::parse_doc(source)?;
        let actual = reconcile_fnx_sidecar_from_tree(source, &parsed, &sidecar)?;
        let expected = fanta_fnx::reconcile_sidecar_from_tree(&parsed, &sidecar, || {
            "5C7N6VFCCQAMM0V3VJGS4B5JB8".into()
        })?;
        assert_eq!(actual, expected);
        assert_eq!(actual.root_parent, sidecar.root_parent);
        assert_eq!(
            actual.ids.get(2).ok_or("inserted identity")?.id,
            "5C7N6VFCCQAMM0V3VJGS4B5JB8"
        );
        assert_eq!(
            reconcile_fnx_sidecar_from_tree(source, &parsed, &sidecar)?,
            actual
        );
        Ok(())
    }

    #[test]
    fn design_load_reuses_parsed_tree_for_identity_repair_and_named_references()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let page = NodeId::new();
        let vector = NodeId::new();
        let text = NodeId::new();
        let instance = NodeId::new();
        let component = ComponentId::new();
        let source = format!(
            "<Frame id=\"{}\" name=\"Root 雪\"><Rect id=\"{}\" name=\"Box\" width={{10}} height={{20}} /><Text id=\"{}\" name=\"Label\" content=\"héllo 雪🙂\" /><Instance id=\"{}\" name=\"Button\" component=\"Button\" local_size={{[20, 10]}} /></Frame>",
            page.0, vector.0, text.0, instance.0
        );
        let sidecar = persisted_fnx_sidecar(&source, None)?;
        let mut stale = sidecar.clone();
        stale.ids.first_mut().ok_or("missing root")?.id = NodeId::new().0.to_string();
        let mut refs = fanta_fnx::RefTable::new(true);
        refs.insert_component("Button", &component.0.to_string());
        let source_path = directory.path().join(PAGE_FNX);
        let sidecar_path = directory.path().join(PAGE_IDS);
        for variant in 0..5 {
            let mut current = source.clone();
            if variant == 4 {
                for entry in &sidecar.ids {
                    current = current.replace(&format!("id=\"{}\"", entry.id), "");
                }
            }
            let bytes = match variant {
                0 => None,
                1 => Some(b"{".to_vec()),
                2 => Some(serde_json::to_vec(&stale)?),
                _ => Some(serde_json::to_vec(&sidecar)?),
            };
            std::fs::write(&source_path, &current)?;
            match &bytes {
                Some(bytes) => std::fs::write(&sidecar_path, bytes)?,
                None if sidecar_path.exists() => std::fs::remove_file(&sidecar_path)?,
                None => {}
            }
            let expected_sidecar = persisted_fnx_sidecar(&current, bytes.as_deref())?;
            let mut expected = fanta_fnx::decode_subtree_with(&current, &expected_sidecar, &refs)?;
            let expected_pending = pending_layout_from_nodes(&expected, Some(page))?;
            for node in &mut expected {
                backfill_required_geometry(node);
            }
            let mut nodes = Map::new();
            let mut pending = BTreeSet::new();
            let mut identities = LoadedSourceIdentities::new();
            let root = read_design_nodes(
                directory.path(),
                PAGE_FNX,
                PAGE_IDS,
                5,
                true,
                None,
                None,
                &refs,
                &mut nodes,
                &mut pending,
                Some(&mut identities),
            )?;
            assert_eq!(root, Some(page));
            let identity = identities
                .get(&source_path)
                .ok_or("loaded source identity")?;
            assert_eq!(identity.current, variant == 3);
            assert_eq!(
                identity.source_hash,
                Some(<[u8; 32]>::from(Sha256::digest(current.as_bytes())))
            );
            assert_eq!(
                identity.sidecar_hash,
                bytes.as_deref().map(|bytes| Sha256::digest(bytes).into())
            );
            assert_eq!(pending, expected_pending);
            for node in expected {
                let key = node.get("id").and_then(Value::as_str).ok_or("node id")?;
                assert_eq!(nodes.get(key), Some(&node));
            }
            assert_eq!(std::fs::read_to_string(&source_path)?, current);
            let saved_sidecar = match std::fs::read(&sidecar_path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            assert_eq!(saved_sidecar, bytes);
        }

        std::fs::write(&source_path, &source)?;
        std::fs::write(&sidecar_path, serde_json::to_vec(&sidecar)?)?;
        let mut nodes = Map::new();
        let mut pending = BTreeSet::new();
        let supplied = FnxSourceOverride {
            path: &source_path,
            source: &source,
            sidecar: Some(&sidecar),
        };
        assert_eq!(
            read_design_nodes(
                directory.path(),
                PAGE_FNX,
                PAGE_IDS,
                5,
                true,
                Some(&supplied),
                None,
                &refs,
                &mut nodes,
                &mut pending,
                None,
            )?,
            Some(page)
        );
        let error = read_design_nodes(
            directory.path(),
            PAGE_FNX,
            PAGE_IDS,
            5,
            true,
            None,
            None,
            &fanta_fnx::RefTable::new(true),
            &mut Map::new(),
            &mut BTreeSet::new(),
            None,
        )
        .expect_err("unknown component still fails after identity recovery");
        assert!(error.to_string().contains("Button"), "{error}");
        assert!(
            error
                .to_string()
                .contains(&source_path.display().to_string()),
            "{error}"
        );
        let legacy = source.replace(&format!("id=\"{}\"", page.0), "");
        assert!(
            persisted_fnx_sidecar(&legacy, None)
                .expect_err("legacy needs a sidecar")
                .to_string()
                .contains("restore the sidecar")
        );
        Ok(())
    }

    #[test]
    fn assembled_project_load_preserves_content_and_saved_bytes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut doc = Doc::new();
        let parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let parent_id = parent.id;
        doc.scene.insert(parent)?;
        doc.pages.push(parent_id);
        let mut later = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            30.0,
            Color::BLACK,
        )));
        later.parent = Some(parent_id);
        later.index = IndexKey::from_raw(2.0000000000000004);
        later.opacity = UnitInterval::new(0.35179502);
        let later_id = later.id;
        doc.scene.insert(later)?;
        let mut earlier = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            1.0,
            2.0,
            3.0,
            4.0,
            Color::BLACK,
        )));
        earlier.parent = Some(parent_id);
        earlier.index = IndexKey::from_raw(0.9999999999999999);
        let earlier_id = earlier.id;
        doc.scene.insert(earlier)?;
        doc.metadata.title = "JSON value load 雪".into();
        doc.metadata.created_at = 9_007_199_254_740_993;
        let assets = BTreeMap::new();
        crate::write_project_tree(directory.path(), &doc, &assets)?;

        let (loaded, loaded_assets) = read_project_tree(directory.path())?;
        assert_eq!(serde_json::to_value(&loaded)?, serde_json::to_value(&doc)?);
        assert_eq!(
            loaded.scene.children_of(Some(parent_id)),
            &[earlier_id, later_id]
        );
        assert!(loaded.pending_layout.is_empty());
        assert_eq!(loaded_assets, assets);
        let report = crate::write_project_tree(directory.path(), &loaded, &loaded_assets)?;
        assert!(report.written.is_empty(), "{:?}", report.written);
        assert!(report.removed.is_empty(), "{:?}", report.removed);

        let manifest_path = directory.path().join(super::super::layout::FANTA_JSON);
        let mut manifest = read_json_file(&manifest_path)?;
        *manifest.get_mut("schema_version").ok_or("missing schema")? = 2.into();
        let (migrated, migrated_assets) = read_project_tree_with_json_override(
            directory.path(),
            &JsonSourceOverride {
                path: &manifest_path,
                value: &manifest,
            },
        )?;
        assert_eq!(
            serde_json::to_value(&migrated)?,
            serde_json::to_value(&doc)?
        );
        assert_eq!(
            migrated.scene.children_of(Some(parent_id)),
            &[earlier_id, later_id]
        );
        assert_eq!(migrated_assets, assets);
        Ok(())
    }

    #[test]
    fn assembled_project_load_preserves_field_diagnostics_and_schema_gate()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let doc = Doc::new();
        crate::write_project_tree(directory.path(), &doc, &BTreeMap::new())?;
        let metadata_path = directory.path().join(DOC_DIR).join(METADATA_JSON);
        let mut metadata = serde_json::to_value(&doc.metadata)?;
        *metadata.get_mut("title").ok_or("missing title")? = false.into();
        let result = read_project_tree_with_json_override(
            directory.path(),
            &JsonSourceOverride {
                path: &metadata_path,
                value: &metadata,
            },
        );
        match result {
            Err(FormatError::InvalidProjectTree(message)) => {
                assert!(message.starts_with(&directory.path().display().to_string()));
                assert!(message.contains("metadata.title"), "{message}");
                assert!(message.contains("invalid type: boolean"), "{message}");
            }
            other => return Err(format!("unexpected malformed metadata result: {other:?}").into()),
        }
        let manifest_path = directory.path().join(super::super::layout::FANTA_JSON);
        let mut manifest = read_json_file(&manifest_path)?;
        *manifest.get_mut("schema_version").ok_or("missing schema")? = (SCHEMA_VERSION + 1).into();
        assert!(matches!(
            read_project_tree_with_json_override(
                directory.path(),
                &JsonSourceOverride { path: &manifest_path, value: &manifest },
            ),
            Err(FormatError::UnsupportedSchema { found, supported })
                if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION
        ));
        let (unchanged, _) = read_project_tree(directory.path())?;
        assert_eq!(serde_json::to_value(unchanged)?, serde_json::to_value(doc)?);
        Ok(())
    }

    #[test]
    fn assembled_project_load_keeps_scene_validation_errors()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut doc = Doc::new();
        let page = doc
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))?;
        doc.add_page(page);
        crate::write_project_tree(directory.path(), &doc, &BTreeMap::new())?;
        let source_path = crate::locate_page_source(directory.path(), page).ok_or("page source")?;
        let sidecar_path = source_path.with_file_name(PAGE_IDS);
        let mut sidecar: fanta_fnx::FnxSidecar =
            serde_json::from_slice(&std::fs::read(&sidecar_path)?)?;
        let missing_parent = NodeId::new();
        sidecar.root_parent = Some(missing_parent.0.to_string());
        std::fs::write(&sidecar_path, serde_json::to_vec(&sidecar)?)?;

        let mut expected_value = serde_json::to_value(&doc)?;
        let page_key = json_key(&page)?;
        *expected_value
            .pointer_mut(&format!("/scene/nodes/{page_key}/parent"))
            .ok_or("page parent")? = serde_json::to_value(missing_parent)?;
        let expected = Doc::from_json_value(&mut expected_value)
            .expect_err("missing parent is invalid")
            .to_string();
        let error = read_project_tree(directory.path())
            .expect_err("project scene must still be validated")
            .to_string();
        assert!(error.contains(&expected), "{error}");
        assert!(
            error.contains(&directory.path().display().to_string()),
            "{error}"
        );
        Ok(())
    }
}

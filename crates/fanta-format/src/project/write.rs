//! `Doc` → project-tree projection.
//!
//! The projection is computed **in memory** as a pure function of the doc —
//! every file the tree should contain, byte-exact — and then applied to disk
//! as a **diff**: a file is written only when its bytes differ, and generated
//! files no longer in the projection are pruned. User files inside managed
//! directories are preserved. The generated disk state matches a full overwrite, but an
//! edit to one component touches only that component's files — file watchers
//! (and agents tailing them) see just the real change. `.git/`, `previews/`,
//! `exports/`, and root-level seeded/user files (`AGENTS.md`, `fnx.d.ts`,
//! prettier configs, …) are never touched. The projection is deterministic —
//! identical doc + assets produce a byte-identical tree (sorted iteration
//! everywhere, pretty JSON with a trailing newline, timestamps sourced from
//! the doc).
//!
//! Presence/UI state (`active_page`, `selection`, `viewport`, `history`) is
//! deliberately **not** written: spec 09 §A.2 moves it out of the persisted
//! schema entirely.
//!
//! ## Incremental projection
//!
//! The disk diff was always incremental; the in-memory projection was not —
//! every save re-serialized the whole document and re-printed every `.fnx`
//! even when one node moved, which on a 30k-node document costs seconds of
//! CPU and gigabytes of transient `serde_json::Value`. [`ProjectWriteCache`]
//! fixes that: nodes are bucketed by walking the typed scene, each design is
//! fingerprinted by streaming its nodes' serialization through a hash (no
//! `Value` is built for it), and a design whose fingerprint is unchanged
//! reuses the bytes produced last time. The bytes are exactly what a cold
//! projection yields, so the byte-determinism contract and the disk diff
//! semantics are untouched — a hit still runs through the on-disk byte check.

use crate::error::{FormatError, Result};
use fanta_doc::{AssetId, ComponentId, Doc, DocId, NodeId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use super::layout::{
    ACTIVE_MODES_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, EXPORTS_DIR, FANTA_JSON,
    FLOW_START_JSON, FLOWS_JSON, LOOSE_DIR, MASTER_FNX, MASTER_IDS, METADATA_JSON, MOTION_JSON,
    NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON, PAGES_DIR, PRESENTATION_JSON, PREVIEWS_DIR,
    PROJECT_VERSION, ProjectManifest, SETS_JSON, VARIABLES_JSON, WORKSPACE_FNX, id_from_key,
    json_bytes, json_key, slugify, sorted_entries,
};
use super::media::{ASSET_INDEX_FILE, MediaRegistry, asset_index_bytes, sniff_media};

const TRANSACTION_DIR: &str = ".fanta-transaction";
static PROJECT_LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
thread_local! {
    static HELD_PROJECT_READ_LOCKS: RefCell<BTreeSet<PathBuf>> = const { RefCell::new(BTreeSet::new()) };
}

fn project_lock_key(dir: &Path) -> PathBuf {
    fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

fn project_lock(dir: &Path) -> Result<Arc<Mutex<()>>> {
    let key = project_lock_key(dir);
    let mut locks = PROJECT_LOCKS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| FormatError::InvalidProjectTree("project lock registry is poisoned".into()))?;
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

struct ProjectReadScope(PathBuf);

impl Drop for ProjectReadScope {
    fn drop(&mut self) {
        HELD_PROJECT_READ_LOCKS.with(|held| {
            held.borrow_mut().remove(&self.0);
        });
    }
}

pub(crate) fn with_project_read_lock<T, E>(
    dir: &Path,
    read: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E>
where
    E: From<FormatError>,
{
    let key = project_lock_key(dir);
    if HELD_PROJECT_READ_LOCKS.with(|held| held.borrow().contains(&key)) {
        return read();
    }
    let lock = project_lock(dir).map_err(E::from)?;
    let guard = lock.lock().map_err(|_| {
        E::from(FormatError::InvalidProjectTree(
            "project I/O lock is poisoned".to_owned(),
        ))
    })?;
    recover_project_transaction_locked(dir, true).map_err(E::from)?;
    HELD_PROJECT_READ_LOCKS.with(|held| {
        held.borrow_mut().insert(key.clone());
    });
    let scope = ProjectReadScope(key);
    let result = read();
    drop(scope);
    drop(guard);
    result
}

/// What one [`write_project_tree`] call actually changed on disk. Paths are
/// relative to the project directory; both lists are sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Files created, or rewritten because their projected bytes differed.
    pub written: Vec<PathBuf>,
    /// Expected content hashes for written paths, for exact own-write watcher suppression.
    pub written_hashes: BTreeMap<PathBuf, [u8; 32]>,
    /// Stale files deleted because the projection no longer contains them.
    pub removed: Vec<PathBuf>,
}

/// The in-memory projection: every non-asset file the tree should contain,
/// keyed by project-relative path. Bytes are shared with the
/// [`ProjectWriteCache`] so a cache hit costs a refcount, not a copy.
type ProjectedFiles = BTreeMap<PathBuf, Arc<Vec<u8>>>;

/// Memo of the per-design projection across successive writes of the same
/// document. Owned by whoever saves repeatedly (the editor's autosave);
/// keyed by document id and reset when a different document is written
/// through it. Holding it is purely an optimization: a stale or fresh cache
/// yields byte-identical output, because an entry is only reused when the
/// design's content fingerprint matches.
#[derive(Default)]
pub struct ProjectWriteCache {
    doc_id: Option<DocId>,
    designs: BTreeMap<PathBuf, CachedDesign>,
}

struct CachedDesign {
    fingerprint: [u8; 32],
    files: Vec<(PathBuf, Arc<Vec<u8>>)>,
}

impl ProjectWriteCache {
    /// Number of designs whose projected bytes are currently memoized.
    pub fn cached_designs(&self) -> usize {
        self.designs.len()
    }

    fn retarget(&mut self, doc: DocId) {
        if self.doc_id != Some(doc) {
            self.designs.clear();
            self.doc_id = Some(doc);
        }
    }
}

/// Project `doc` + `assets` onto the directory tree at `dir` (spec 09 §A.2).
///
/// Creates `dir` if needed. Reconciles `fanta.json` and generated files under
/// `doc/`, `pages/`, `components/`, and `assets/`. Only differing files are
/// written, and only stale generated files are removed. Existing user files
/// and custom `.gitignore` rules are preserved. Asset bytes are compared before
/// skipping a write, including when the on-disk file has the expected length.
///
/// Projects from scratch; a caller that saves the same document repeatedly
/// should use [`write_project_tree_cached`].
pub fn write_project_tree(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<WriteReport> {
    write_project_tree_cached(dir, doc, assets, &mut ProjectWriteCache::default())
}

/// [`write_project_tree`] reusing the per-design projection memoized in
/// `cache` from the previous write of this document. Output and disk
/// semantics are identical to the uncached call — including rewriting a file
/// someone edited externally back to the projected bytes.
pub fn write_project_tree_cached(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(dir, doc, assets, cache, None, &BTreeMap::new(), None)
}

/// Write using registered binary probes to choose asset extensions and family
/// folders. The default writer uses the built-in probes.
pub fn write_project_tree_cached_with_media_registry(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    media_registry: &MediaRegistry,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(
        dir,
        doc,
        assets,
        cache,
        Some(media_registry),
        &BTreeMap::new(),
        None,
    )
}

/// Write with a custom media registry without keeping a projection cache.
pub fn write_project_tree_with_media_registry(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    media_registry: &MediaRegistry,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(
        dir,
        doc,
        assets,
        &mut ProjectWriteCache::default(),
        Some(media_registry),
        &BTreeMap::new(),
        None,
    )
}

/// Persist validated `.fnx` source and ID sidecars from a live source session.
/// Keys must name a projected `pages/<slug>` or `components/<slug>` directory;
/// other paths are rejected before any project file is touched.
pub fn write_project_tree_cached_with_sources(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(dir, doc, assets, cache, None, sources, None)
}

/// Save retained source bytes only if the caller's last reconciled disk hashes
/// still match. A `None` value requires the path to remain absent. This can
/// also guard singleton JSON and the asset index during a live canvas save.
pub fn write_project_tree_cached_with_sources_checked(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
    expected_disk: &BTreeMap<PathBuf, Option<[u8; 32]>>,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(dir, doc, assets, cache, None, sources, Some(expected_disk))
}

/// Apply live source and disk preconditions with a custom media registry.
pub fn write_project_tree_cached_with_sources_checked_and_media_registry(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
    expected_disk: &BTreeMap<PathBuf, Option<[u8; 32]>>,
    media_registry: &MediaRegistry,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(
        dir,
        doc,
        assets,
        cache,
        Some(media_registry),
        sources,
        Some(expected_disk),
    )
}

/// Preserve live sources while using registered binary format probes.
pub fn write_project_tree_cached_with_sources_and_media_registry(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
    media_registry: &MediaRegistry,
) -> Result<WriteReport> {
    write_project_tree_cached_impl(dir, doc, assets, cache, Some(media_registry), sources, None)
}

fn write_project_tree_cached_impl(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
    cache: &mut ProjectWriteCache,
    media_registry: Option<&MediaRegistry>,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
    expected_disk: Option<&BTreeMap<PathBuf, Option<[u8; 32]>>>,
) -> Result<WriteReport> {
    validate_source_override_keys(sources)?;
    if let Some(expected_disk) = expected_disk {
        validate_expected_disk_paths(expected_disk)?;
    }
    let mut files = project_files_cached_with_sources(doc, cache, sources)?;
    files.insert(
        PathBuf::from(ASSETS_DIR).join(ASSET_INDEX_FILE),
        Arc::new(asset_index_bytes(assets)?),
    );
    let asset_files = project_asset_files(assets, media_registry)?;

    fs::create_dir_all(dir)?;

    fs::create_dir_all(dir.join(PREVIEWS_DIR))?;
    fs::create_dir_all(dir.join(EXPORTS_DIR))?;

    let lock = project_lock(dir)?;
    let guard = lock
        .lock()
        .map_err(|_| FormatError::InvalidProjectTree("project I/O lock is poisoned".to_owned()))?;
    recover_project_transaction_locked(dir, true)?;
    if let Some(expected_disk) = expected_disk {
        validate_expected_disk_content(dir, expected_disk)?;
    }

    // Do not traverse a planted symlink or case-aliased design directory.
    heal_directory_case(dir, &files)?;
    remove_symlinks_on_projected_paths(dir, files.keys().chain(asset_files.keys()))?;

    let keep: BTreeSet<&Path> = files
        .keys()
        .chain(asset_files.keys())
        .map(PathBuf::as_path)
        .collect();

    let mut writes = Vec::new();
    for (relative, bytes) in &files {
        plan_write(dir, relative, bytes, &mut writes)?;
    }
    for (relative, bytes) in &asset_files {
        plan_write(dir, relative, bytes, &mut writes)?;
    }
    let removals = stale_managed_files(dir, &keep)?;
    if let Some(expected_disk) = expected_disk {
        if let Some(untracked) = removals
            .iter()
            .find(|removal| !expected_disk.contains_key(&removal.relative))
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "{} would be removed without a reconciled disk precondition",
                untracked.relative.display()
            )));
        }
        validate_expected_disk_content(dir, expected_disk)?;
    }
    let mut written: Vec<PathBuf> = writes.iter().map(|write| write.relative.clone()).collect();
    let written_hashes: BTreeMap<PathBuf, [u8; 32]> = writes
        .iter()
        .map(|write| (write.relative.clone(), write.new_hash))
        .collect();
    let mut removed: Vec<PathBuf> = removals
        .iter()
        .map(|remove| remove.relative.clone())
        .collect();

    if writes.len() + removals.len() == 1 {
        apply_single_change(dir, &writes, &removals)?;
    } else if !writes.is_empty() || !removals.is_empty() {
        apply_project_transaction(dir, &writes, &removals)?;
    }
    prune_removed_artifact_directories(dir, &removals)?;
    fs::create_dir_all(dir.join(ASSETS_DIR))?;
    drop(guard);

    crate::project::layout::ensure_project_editor_support(dir)?;

    written.sort();
    removed.sort();
    Ok(WriteReport {
        written,
        written_hashes,
        removed,
    })
}

/// Rename design directories whose on-disk name differs from a projected
/// design dir only by ASCII case. On a case-insensitive filesystem (APFS,
/// NTFS) such a directory ALIASES the projected path: the byte-diff succeeds
/// through the alias while the exact-case prune deletes the live files — a
/// case-only folder rename would silently destroy the design. Healing is a
/// two-step rename (via a temp name, since a direct case-only rename is a
/// no-op-or-error on some filesystems); on a case-sensitive filesystem where
/// the projected name is genuinely occupied the rename fails harmlessly and
/// the differently-cased dir is pruned as the stale dir it really is.
fn heal_directory_case(dir: &Path, files: &ProjectedFiles) -> Result<()> {
    let mut expected: BTreeSet<(&str, &str)> = BTreeSet::new();
    for relative in files.keys() {
        let mut components = relative.components();
        let (Some(managed), Some(design), Some(_file)) =
            (components.next(), components.next(), components.next())
        else {
            continue;
        };
        if let (Some(managed), Some(design)) =
            (managed.as_os_str().to_str(), design.as_os_str().to_str())
            && (managed == PAGES_DIR || managed == COMPONENTS_DIR)
        {
            expected.insert((managed, design));
        }
    }
    for (managed, design) in expected {
        let parent = dir.join(managed);
        let Ok(entries) = fs::read_dir(&parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name != design && name.eq_ignore_ascii_case(design) {
                let from = parent.join(name);
                let via = parent.join(format!("{design}.case-heal"));
                let to = parent.join(design);
                match fs::rename(&from, &via) {
                    Ok(()) => {
                        if let Err(error) = fs::rename(&via, &to) {
                            tracing::warn!(
                                target: "fanta::format",
                                "healing dir case for {} failed ({error}); restoring",
                                from.display()
                            );
                            if let Err(error) = fs::rename(&via, &from) {
                                tracing::warn!(
                                    target: "fanta::format",
                                    "restoring {} after failed case heal failed: {error}",
                                    from.display()
                                );
                            }
                        }
                    }
                    Err(error) => tracing::warn!(
                        target: "fanta::format",
                        "case heal of {} skipped: {error}",
                        from.display()
                    ),
                }
            }
        }
    }
    Ok(())
}

/// Unlink any symlink sitting on (or on an ancestor of) a projected path, so
/// no read or write ever travels through one. The subsequent diff then sees
/// the files as missing and rebuilds them as real files — the same net state
/// the old full-overwrite writer produced. Only the LINK is removed, never
/// its target.
fn remove_symlinks_on_projected_paths<'a>(
    dir: &Path,
    paths: impl Iterator<Item = &'a PathBuf>,
) -> Result<()> {
    let mut vetted: BTreeSet<PathBuf> = BTreeSet::new();
    for relative in paths {
        let mut ancestry = PathBuf::new();
        for component in relative.components() {
            ancestry.push(component);
            if !vetted.insert(ancestry.clone()) {
                continue;
            }
            let absolute = dir.join(&ancestry);
            if let Ok(metadata) = fs::symlink_metadata(&absolute)
                && metadata.file_type().is_symlink()
            {
                fs::remove_file(&absolute)?;
            }
        }
    }
    Ok(())
}

/// The complete in-memory projection of `doc`, computed from scratch.
#[cfg(test)]
fn project_files(doc: &Doc) -> Result<ProjectedFiles> {
    project_files_cached(doc, &mut ProjectWriteCache::default())
}

/// The complete in-memory projection of `doc`: every non-asset file the tree
/// should contain, keyed by project-relative path, with the exact bytes the
/// tree should hold. Designs whose fingerprint matches `cache` reuse their
/// previous bytes; everything else is (re)generated. The cache is left
/// holding exactly the designs this projection contains.
#[cfg(test)]
fn project_files_cached(doc: &Doc, cache: &mut ProjectWriteCache) -> Result<ProjectedFiles> {
    project_files_cached_with_sources(doc, cache, &BTreeMap::new())
}

fn project_files_cached_with_sources(
    doc: &Doc,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
) -> Result<ProjectedFiles> {
    cache.retarget(doc.id);
    let mut files = ProjectedFiles::new();
    files.insert(
        PathBuf::from(FANTA_JSON),
        Arc::new(json_bytes(&serde_json::to_value(
            ProjectManifest::for_doc(doc),
        )?)?),
    );
    project_doc_singletons(&mut files, doc)?;
    // Name-based reference emission follows the manifest this write stamps:
    // the tree is written at PROJECT_VERSION, and v4 is the layout where
    // `component="Button"` / `"$Collection/Name"` spellings became part of
    // the on-disk vocabulary. (`>=` so the guard reads as the layout rule,
    // not as a tautology of the current constant.)
    let refs = crate::project::refs_ctx::build_ref_table(
        &doc.components,
        &doc.variables,
        PROJECT_VERSION >= 4,
    );
    project_designs(&mut files, doc, &refs, cache, sources)?;
    Ok(files)
}

/// The small, mergeable doc-level files under `doc/`. Note what is *absent*:
/// `active_page`, `selection`, `viewport`, and `history` are presence state
/// and never reach disk.
///
/// Each file holds the field exactly as `Doc`'s own serialization would emit
/// it, including the `skip_serializing_if` cases, where the whole-document
/// projection had no key and the writer substituted an empty object — the
/// bytes must not move when a document gains or loses its first variable.
fn project_doc_singletons(files: &mut ProjectedFiles, doc: &Doc) -> Result<()> {
    let doc_dir = PathBuf::from(DOC_DIR);
    let variables = if doc.variables.is_empty() {
        json!({})
    } else {
        serde_json::to_value(&doc.variables)?
    };
    let motion = if doc.motion.is_empty() {
        json!({})
    } else {
        serde_json::to_value(&doc.motion)?
    };
    let singletons = [
        (METADATA_JSON, serde_json::to_value(&doc.metadata)?),
        (VARIABLES_JSON, variables),
        (ACTIVE_MODES_JSON, serde_json::to_value(&doc.active_modes)?),
        (MOTION_JSON, motion),
        (FLOW_START_JSON, serde_json::to_value(doc.flow_start)?),
        (FLOWS_JSON, serde_json::to_value(&doc.flows)?),
        (PRESENTATION_JSON, serde_json::to_value(&doc.presentation)?),
    ];
    for (name, value) in singletons {
        files.insert(doc_dir.join(name), Arc::new(json_bytes(&value)?));
    }
    Ok(())
}

/// Where one node's file belongs in the tree.
#[derive(Clone)]
enum Bucket {
    /// `components/<slug>/` — the nearest ancestor-or-self is a
    /// `ComponentDef.root`.
    Component(String),
    /// `pages/<slug>/` — the parent chain tops out at a page root.
    Page(String),
    /// `pages/_loose/nodes/` — orphans: neither of the above.
    Loose,
}

/// Assign every design a unique directory slug from its name. Collisions get
/// deterministic `-2`, `-3`, … suffixes in id order, so the slug set is a pure
/// function of the doc (the byte-determinism contract).
fn design_slugs<Id: Ord + Copy>(
    named: impl IntoIterator<Item = (Id, String)>,
    fallback: &str,
) -> BTreeMap<Id, String> {
    let mut items: Vec<(Id, String)> = named.into_iter().collect();
    items.sort_by_key(|(id, _)| *id);
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut slugs = BTreeMap::new();
    for (id, name) in items {
        let base = slugify(&name, fallback);
        let mut slug = base.clone();
        let mut suffix = 2u64;
        while !used.insert(slug.clone()) {
            slug = format!("{base}-{suffix}");
            suffix += 1;
        }
        slugs.insert(id, slug);
    }
    slugs
}

/// Predicted page and component directories for a document. Session source
/// buffers use stable ids; this maps them to the writer's current name slugs
/// before a save, including after a canvas rename.
pub fn projected_design_dirs(
    doc: &Doc,
) -> (BTreeMap<NodeId, PathBuf>, BTreeMap<ComponentId, PathBuf>) {
    let pages = design_slugs(
        doc.pages.iter().map(|page| {
            let name = doc
                .scene
                .get(*page)
                .map(|node| node.name.clone())
                .unwrap_or_default();
            (*page, name)
        }),
        "page",
    )
    .into_iter()
    .map(|(id, slug)| (id, PathBuf::from(PAGES_DIR).join(slug)))
    .collect();
    let components = design_slugs(
        doc.components
            .defs
            .iter()
            .map(|(id, def)| (*id, def.name.clone())),
        "component",
    )
    .into_iter()
    .map(|(id, slug)| (id, PathBuf::from(COMPONENTS_DIR).join(slug)))
    .collect();
    (pages, components)
}

/// Page headers, component defs/sets, and one source (or fallback node file)
/// per design.
fn project_designs(
    files: &mut ProjectedFiles,
    doc: &Doc,
    refs: &fanta_fnx::RefTable,
    cache: &mut ProjectWriteCache,
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
) -> Result<()> {
    // v3: design directories are named by a slug of the design's name; the
    // ids move into the JSON headers (`page.json` "id", `def.json`).
    let page_slugs: BTreeMap<NodeId, String> = design_slugs(
        doc.pages.iter().map(|page| {
            let name = doc
                .scene
                .get(*page)
                .map(|node| node.name.clone())
                .unwrap_or_default();
            (*page, name)
        }),
        "page",
    );
    let component_slugs: BTreeMap<ComponentId, String> = design_slugs(
        doc.components
            .defs
            .iter()
            .map(|(cid, def)| (*cid, def.name.clone())),
        "component",
    );
    let slug_of_page = |page: &NodeId| -> Result<&String> {
        page_slugs.get(page).ok_or_else(|| {
            FormatError::InvalidProjectTree(format!("page {page} has no directory slug"))
        })
    };
    let slug_of_component = |cid: &ComponentId| -> Result<&String> {
        component_slugs.get(cid).ok_or_else(|| {
            FormatError::InvalidProjectTree(format!("component {cid} has no directory slug"))
        })
    };

    // Design root → directory slug for the two kinds of design roots.
    let mut component_roots: BTreeMap<NodeId, String> = BTreeMap::new();
    for (cid, def) in &doc.components.defs {
        component_roots.insert(def.root, slug_of_component(cid)?.clone());
    }
    let mut page_dirs: BTreeMap<NodeId, String> = BTreeMap::new();
    for page in &doc.pages {
        page_dirs.insert(*page, slug_of_page(page)?.clone());
    }

    // pages/<slug>/page.json — id + name + order. The id is the page's
    // identity of record (the directory name is only a readable projection).
    // `order` is the position in `doc.pages` today; the fractional-IndexKey
    // ordering discipline (spec 02 §1, a shared prerequisite of spec 09)
    // replaces this u32 with an order key string when that migration lands.
    for (order, page) in doc.pages.iter().enumerate() {
        let mut header = Map::new();
        header.insert("id".to_owned(), Value::from(page.to_string()));
        if let Some(node) = doc.scene.get(*page) {
            header.insert("name".to_owned(), Value::from(node.name.clone()));
        }
        header.insert("order".to_owned(), Value::from(order as u32));
        files.insert(
            PathBuf::from(PAGES_DIR)
                .join(slug_of_page(page)?)
                .join(PAGE_JSON),
            Arc::new(json_bytes(&Value::Object(header))?),
        );
    }

    // components/<slug>/def.json + components/sets.json — each def exactly as
    // `Doc` serializes it (the def carries the component's id).
    for (cid, def) in &doc.components.defs {
        files.insert(
            PathBuf::from(COMPONENTS_DIR)
                .join(slug_of_component(cid)?)
                .join(DEF_JSON),
            Arc::new(json_bytes(&serde_json::to_value(def)?)?),
        );
    }
    files.insert(
        PathBuf::from(COMPONENTS_DIR).join(SETS_JSON),
        Arc::new(json_bytes(&serde_json::to_value(&doc.components.sets)?)?),
    );

    // v2: one readable `.fnx` source + an `.ids` sidecar per page / component
    // (was one JSON file per node). Keys are sorted (determinism contract) then
    // grouped by their design; `_loose` orphans stay per-node JSON because they
    // need not form a single-rooted tree.
    let mut keyed_nodes: Vec<(String, NodeId)> = Vec::with_capacity(doc.scene.len());
    for id in scene_node_ids(doc)? {
        keyed_nodes.push((json_key(&id)?, id));
    }
    keyed_nodes.sort();
    let mut page_nodes: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
    let mut comp_nodes: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
    let mut loose: Vec<NodeId> = Vec::new();
    let mut classified = BTreeMap::new();
    for (_, id) in keyed_nodes {
        match classify(
            &doc.scene,
            id,
            &component_roots,
            &page_dirs,
            &mut classified,
        ) {
            Bucket::Component(cdir) => comp_nodes.entry(cdir).or_default().push(id),
            Bucket::Page(pdir) => page_nodes.entry(pdir).or_default().push(id),
            Bucket::Loose => loose.push(id),
        }
    }

    let refs_digest = ref_table_digest(doc);
    let page_by_slug: BTreeMap<&String, NodeId> =
        page_slugs.iter().map(|(id, slug)| (slug, *id)).collect();
    let component_by_slug: BTreeMap<&String, ComponentId> = component_slugs
        .iter()
        .map(|(id, slug)| (slug, *id))
        .collect();
    let mut live_designs: BTreeSet<PathBuf> = BTreeSet::new();
    for (pdir, group) in &page_nodes {
        let name = page_by_slug
            .get(pdir)
            .and_then(|id| doc.scene.get(*id))
            .map(|n| n.name.as_str());
        let design_dir = PathBuf::from(PAGES_DIR).join(pdir);
        project_design_cached(
            files,
            doc,
            cache,
            &design_dir,
            PAGE_FNX,
            PAGE_IDS,
            group,
            &design_name(name, "Page"),
            refs,
            &refs_digest,
            sources,
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("page {pdir}: {e}")))?;
        live_designs.insert(design_dir);
    }
    for (cdir, group) in &comp_nodes {
        let name = component_by_slug
            .get(cdir)
            .and_then(|id| doc.components.defs.get(id))
            .map(|d| d.name.as_str());
        let design_dir = PathBuf::from(COMPONENTS_DIR).join(cdir);
        project_design_cached(
            files,
            doc,
            cache,
            &design_dir,
            MASTER_FNX,
            MASTER_IDS,
            group,
            &design_name(name, "Component"),
            refs,
            &refs_digest,
            sources,
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("component {cdir}: {e}")))?;
        live_designs.insert(design_dir);
    }
    cache
        .designs
        .retain(|design_dir, _| live_designs.contains(design_dir));
    if let Some(unknown) = sources
        .keys()
        .find(|design_dir| !live_designs.contains(*design_dir))
    {
        return Err(FormatError::InvalidProjectTree(format!(
            "source override names unknown design {}",
            unknown.display()
        )));
    }
    for id in loose {
        let node = doc
            .scene
            .get(id)
            .ok_or_else(|| FormatError::InvalidProjectTree(format!("node {id} vanished")))?;
        files.insert(
            PathBuf::from(PAGES_DIR)
                .join(LOOSE_DIR)
                .join(NODES_DIR)
                .join(format!("{id}.json")),
            Arc::new(json_bytes(&serde_json::to_value(node)?)?),
        );
    }
    Ok(())
}

/// Every node id in the scene. The scene exposes no whole-map iterator, so
/// the ids are gathered by walking down from the roots; a scene whose nodes
/// are all reachable (the invariant `Scene::validate` enforces on load and
/// `Scene::insert` on edit) is covered exactly. Should the counts disagree —
/// a node whose parent was edited out from under it — fall back to the
/// serialized key set so no node is silently dropped from the tree.
fn scene_node_ids(doc: &Doc) -> Result<Vec<NodeId>> {
    let scene = &doc.scene;
    let mut ids: Vec<NodeId> = Vec::with_capacity(scene.len());
    for root in scene.roots() {
        ids.extend(scene.descendants_of(*root));
    }
    if ids.len() == scene.len() {
        return Ok(ids);
    }
    tracing::warn!(
        target: "fanta::format",
        reachable = ids.len(),
        total = scene.len(),
        "scene has nodes unreachable from its roots; projecting from the serialized key set"
    );
    let value = serde_json::to_value(scene)?;
    let nodes = value
        .get("nodes")
        .and_then(Value::as_object)
        .ok_or_else(|| FormatError::InvalidProjectTree("scene projection missing nodes".into()))?;
    nodes.keys().map(|key| id_from_key(key)).collect()
}

/// A digest of everything besides the nodes that shapes a design's `.fnx`
/// text: the name↔id vocabulary the [`fanta_fnx::RefTable`] is built from,
/// and the layout version that decides whether names are emitted at all.
/// Mirrors [`crate::project::refs_ctx::build_ref_table`] input for input.
fn ref_table_digest(doc: &Doc) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROJECT_VERSION.to_le_bytes());
    for (id, def) in &doc.components.defs {
        hasher.update(id.0.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(def.name.as_bytes());
        hasher.update([0]);
    }
    for variable in doc.variables.variables.values() {
        let Some(collection) = doc.variables.collections.get(&variable.collection) else {
            continue;
        };
        hasher.update(variable.id.0.to_string().as_bytes());
        hasher.update([0]);
        hasher.update(collection.name.as_bytes());
        hasher.update([b'/']);
        hasher.update(variable.name.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

/// Content fingerprint of one design: its kind, display name, the ref-table
/// digest, and every node's serialization streamed straight into the hash.
/// Two designs with equal fingerprints project to identical files.
fn design_fingerprint(
    doc: &Doc,
    fnx_name: &str,
    fn_name: &str,
    nodes: &[NodeId],
    refs_digest: &[u8; 32],
) -> Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(fnx_name.as_bytes());
    hasher.update([0]);
    hasher.update(fn_name.as_bytes());
    hasher.update([0]);
    hasher.update(refs_digest);
    hasher.update((nodes.len() as u64).to_le_bytes());
    for id in nodes {
        let node = doc
            .scene
            .get(*id)
            .ok_or_else(|| FormatError::InvalidProjectTree(format!("node {id} vanished")))?;
        serde_json::to_writer(&mut hasher, node)?;
        hasher.update([0]);
    }
    Ok(hasher.finalize().into())
}

/// Project one design, reusing the cached bytes when its fingerprint is
/// unchanged since the last write and regenerating (and re-memoizing) it
/// otherwise.
#[allow(clippy::too_many_arguments)]
fn project_design_cached(
    files: &mut ProjectedFiles,
    doc: &Doc,
    cache: &mut ProjectWriteCache,
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    nodes: &[NodeId],
    fn_name: &str,
    refs: &fanta_fnx::RefTable,
    refs_digest: &[u8; 32],
    sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>,
) -> Result<()> {
    if let Some((source, sidecar)) = sources.get(design_dir) {
        files.insert(design_dir.join(fnx_name), Arc::new(source.clone()));
        files.insert(design_dir.join(ids_name), Arc::new(sidecar.clone()));
        cache.designs.remove(design_dir);
        return Ok(());
    }
    let fingerprint = design_fingerprint(doc, fnx_name, fn_name, nodes, refs_digest)?;
    if let Some(cached) = cache.designs.get(design_dir)
        && cached.fingerprint == fingerprint
    {
        for (relative, bytes) in &cached.files {
            files.insert(relative.clone(), bytes.clone());
        }
        return Ok(());
    }
    let mut values: Vec<Value> = Vec::with_capacity(nodes.len());
    for id in nodes {
        let node = doc
            .scene
            .get(*id)
            .ok_or_else(|| FormatError::InvalidProjectTree(format!("node {id} vanished")))?;
        values.push(serde_json::to_value(node)?);
    }
    let produced = project_fnx_design(design_dir, fnx_name, ids_name, &values, fn_name, refs)?;
    for (relative, bytes) in &produced {
        files.insert(relative.clone(), bytes.clone());
    }
    cache.designs.insert(
        design_dir.to_path_buf(),
        CachedDesign {
            fingerprint,
            files: produced,
        },
    );
    Ok(())
}

/// Project one design's `.fnx` source + `.ids` sidecar into `design_dir`. If
/// the bucket can't be encoded as a single-rooted `.fnx` tree (e.g. a
/// multi-root bucket from corrupt data, or a node whose `type` has no JSX tag
/// yet), fall back to per-node JSON under `nodes/` — which the reader still
/// loads — rather than aborting the whole save.
fn project_fnx_design(
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    nodes: &[Value],
    fn_name: &str,
    refs: &fanta_fnx::RefTable,
) -> Result<Vec<(PathBuf, Arc<Vec<u8>>)>> {
    match fanta_fnx::encode_subtree_with(nodes, fn_name, refs) {
        Ok((text, sidecar)) => Ok(vec![
            (design_dir.join(fnx_name), Arc::new(text.into_bytes())),
            (
                design_dir.join(ids_name),
                Arc::new(json_bytes(&serde_json::to_value(&sidecar)?)?),
            ),
        ]),
        Err(e) => {
            tracing::warn!(
                target: "fanta::format",
                dir = %design_dir.display(),
                "fnx encode failed ({e}); writing per-node JSON fallback"
            );
            project_nodes_fallback(&design_dir.join(NODES_DIR), nodes)
        }
    }
}

/// The per-node JSON escape hatch: one `<id>.json` per node, the v1 shape the
/// reader falls back to when a design has no `.fnx`.
fn project_nodes_fallback(
    nodes_dir: &Path,
    nodes: &[Value],
) -> Result<Vec<(PathBuf, Arc<Vec<u8>>)>> {
    let mut produced = Vec::with_capacity(nodes.len());
    for node in nodes {
        let key = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| FormatError::InvalidProjectTree("node missing id".into()))?;
        let node_id: NodeId = id_from_key(key)?;
        produced.push((
            nodes_dir.join(format!("{node_id}.json")),
            Arc::new(json_bytes(node)?),
        ));
    }
    Ok(produced)
}

/// A cosmetic function name for a `.fnx` file — the design's display name, or a
/// fallback. The authoritative name is the root node's `name` attribute.
fn design_name(name: Option<&str>, fallback: &str) -> String {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Walk the parent chain from `id` upward. The **nearest ancestor-or-self**
/// that is a component root wins (so component subtrees nested under a hidden
/// Components page still file under `components/`); otherwise the chain's top
/// decides: a page root files under that page, anything else is loose. The hop
/// cap guards against parent cycles in hand-edited JSON. Memoizing every
/// traversed ancestor prevents a deep scene from repeating the same walk for
/// each descendant.
fn classify(
    scene: &fanta_doc::Scene,
    id: NodeId,
    component_roots: &BTreeMap<NodeId, String>,
    page_dirs: &BTreeMap<NodeId, String>,
    classified: &mut BTreeMap<NodeId, Bucket>,
) -> Bucket {
    let mut current = id;
    let mut hops = 0usize;
    let mut traversed = Vec::new();
    let bucket = loop {
        if let Some(cdir) = component_roots.get(&current) {
            break Bucket::Component(cdir.clone());
        }
        if let Some(bucket) = classified.get(&current) {
            break bucket.clone();
        }
        traversed.push(current);
        let parent = scene.get(current).and_then(|node| node.parent);
        match parent {
            Some(p) if hops <= scene.len() && scene.contains(p) => {
                current = p;
                hops += 1;
            }
            _ => {
                break match page_dirs.get(&current) {
                    Some(pdir) => Bucket::Page(pdir.clone()),
                    None => Bucket::Loose,
                };
            }
        }
    };
    for traversed_id in traversed {
        classified.insert(traversed_id, bucket.clone());
    }
    bucket
}

/// `assets/<family>/<asset-id>.<ext>` — family and extension sniffed from the
/// bytes (see [`super::media`]); the id in the filename is the truth.
fn project_asset_files<'a>(
    assets: &'a BTreeMap<AssetId, Vec<u8>>,
    media_registry: Option<&MediaRegistry>,
) -> Result<BTreeMap<PathBuf, &'a [u8]>> {
    let mut files = BTreeMap::new();
    for (id, bytes) in assets {
        let (family, ext) = media_registry
            .and_then(|registry| {
                registry
                    .sniff(bytes)
                    .map(|format| (format.family, format.extension))
            })
            .unwrap_or_else(|| sniff_media(bytes));
        if ![family, ext].into_iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        }) {
            return Err(FormatError::InvalidProjectTree(format!(
                "media registry returned an unsafe asset path for {id}"
            )));
        }
        files.insert(
            PathBuf::from(ASSETS_DIR)
                .join(family)
                .join(format!("{id}.{ext}")),
            bytes.as_slice(),
        );
    }
    Ok(files)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "sha256")]
enum FileState {
    Missing,
    Regular(String),
    Symlink,
}

struct PlannedWrite<'a> {
    relative: PathBuf,
    bytes: &'a [u8],
    old: FileState,
    new_hash: [u8; 32],
}

struct PlannedRemoval {
    relative: PathBuf,
    old: FileState,
}

#[derive(Deserialize, Serialize)]
struct ProjectTransaction {
    version: u32,
    writes: Vec<TransactionWrite>,
    removals: Vec<TransactionRemoval>,
}

#[derive(Deserialize, Serialize)]
struct TransactionWrite {
    relative: PathBuf,
    old: FileState,
    new_sha256: String,
}

#[derive(Deserialize, Serialize)]
struct TransactionRemoval {
    relative: PathBuf,
    old: FileState,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hash_hex(&digest)
}

fn hash_hex(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in digest {
        result.push(HEX[usize::from(byte >> 4)] as char);
        result.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    result
}

fn file_state(path: &Path) -> Result<FileState> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileState::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Ok(FileState::Symlink);
    }
    if !metadata.is_file() {
        return Err(FormatError::InvalidProjectTree(format!(
            "a directory occupies generated file {}",
            path.display()
        )));
    }
    Ok(FileState::Regular(sha256_hex(&fs::read(path)?)))
}

fn plan_write<'a>(
    dir: &Path,
    relative: &Path,
    bytes: &'a [u8],
    writes: &mut Vec<PlannedWrite<'a>>,
) -> Result<()> {
    let path = dir.join(relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let old = match metadata {
        None => FileState::Missing,
        Some(metadata) if metadata.is_file() => {
            let existing = fs::read(&path)?;
            if existing == bytes {
                return Ok(());
            }
            FileState::Regular(sha256_hex(&existing))
        }
        Some(metadata) if metadata.file_type().is_symlink() => FileState::Symlink,
        Some(_) => {
            return Err(FormatError::InvalidProjectTree(format!(
                "a directory occupies generated file {}",
                path.display()
            )));
        }
    };
    writes.push(PlannedWrite {
        relative: relative.to_path_buf(),
        bytes,
        old,
        new_hash: Sha256::digest(bytes).into(),
    });
    Ok(())
}

fn is_generated_artifact(relative: &Path) -> bool {
    let Some(parts) = relative
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    match parts.as_slice() {
        [DOC_DIR, name] => [
            METADATA_JSON,
            VARIABLES_JSON,
            ACTIVE_MODES_JSON,
            MOTION_JSON,
            FLOW_START_JSON,
            FLOWS_JSON,
            PRESENTATION_JSON,
        ]
        .contains(name),
        [COMPONENTS_DIR, SETS_JSON] | [ASSETS_DIR, ASSET_INDEX_FILE] => true,
        [PAGES_DIR, _, name] => [PAGE_JSON, PAGE_FNX, PAGE_IDS].contains(name),
        [COMPONENTS_DIR, _, name] => [DEF_JSON, MASTER_FNX, MASTER_IDS].contains(name),
        [PAGES_DIR | COMPONENTS_DIR, _, NODES_DIR, name] => {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "json")
                && Path::new(name)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.parse::<NodeId>().is_ok())
        }
        [ASSETS_DIR, _, name] => Path::new(name)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| stem.parse::<AssetId>().is_ok()),
        _ => false,
    }
}

fn validate_source_override_keys(sources: &BTreeMap<PathBuf, (Vec<u8>, Vec<u8>)>) -> Result<()> {
    for design_dir in sources.keys() {
        let components: Vec<_> = design_dir.components().collect();
        let [
            std::path::Component::Normal(root),
            std::path::Component::Normal(slug),
        ] = components.as_slice()
        else {
            return Err(FormatError::InvalidProjectTree(format!(
                "source override path {} must be a page or component directory",
                design_dir.display()
            )));
        };
        let Some(root) = root.to_str() else {
            return Err(FormatError::InvalidProjectTree(format!(
                "source override path {} is not UTF-8",
                design_dir.display()
            )));
        };
        let Some(slug) = slug.to_str() else {
            return Err(FormatError::InvalidProjectTree(format!(
                "source override path {} is not UTF-8",
                design_dir.display()
            )));
        };
        if ![PAGES_DIR, COMPONENTS_DIR].contains(&root)
            || slug.is_empty()
            || !slug
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "source override path {} is not a projected design directory",
                design_dir.display()
            )));
        }
    }
    Ok(())
}

fn validate_expected_disk_paths(expected_disk: &BTreeMap<PathBuf, Option<[u8; 32]>>) -> Result<()> {
    for relative in expected_disk.keys() {
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
            || (relative != Path::new(FANTA_JSON)
                && relative != Path::new(WORKSPACE_FNX)
                && !is_generated_artifact(relative))
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "save precondition names an unmanaged path {}",
                relative.display()
            )));
        }
    }
    Ok(())
}

fn validate_expected_disk_content(
    dir: &Path,
    expected_disk: &BTreeMap<PathBuf, Option<[u8; 32]>>,
) -> Result<()> {
    for (relative, digest) in expected_disk {
        let expected = digest
            .as_ref()
            .map(|digest| FileState::Regular(hash_hex(digest)))
            .unwrap_or(FileState::Missing);
        validate_unchanged(dir, relative, &expected)?;
    }
    Ok(())
}

fn stale_managed_files(dir: &Path, keep: &BTreeSet<&Path>) -> Result<Vec<PlannedRemoval>> {
    let mut removals = Vec::new();
    let mut stack: Vec<PathBuf> = [DOC_DIR, PAGES_DIR, COMPONENTS_DIR, ASSETS_DIR]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    while let Some(relative_dir) = stack.pop() {
        let absolute_dir = dir.join(&relative_dir);
        let metadata = match fs::symlink_metadata(&absolute_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_dir() {
            continue;
        }
        let entries = match sorted_entries(&absolute_dir) {
            Ok(entries) => entries,
            Err(FormatError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                continue;
            }
            Err(error) => return Err(error),
        };
        for path in entries {
            let Some(name) = path.file_name() else {
                continue;
            };
            let relative = relative_dir.join(name);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.is_dir() {
                let root = relative_dir
                    .components()
                    .next()
                    .and_then(|part| part.as_os_str().to_str());
                let depth = relative_dir.components().count();
                let scan = match (root, depth) {
                    (Some(ASSETS_DIR), 1) => true,
                    (Some(PAGES_DIR), 1) if name == LOOSE_DIR => true,
                    (Some(PAGES_DIR), 1) | (Some(COMPONENTS_DIR), 1) => {
                        let header = if root == Some(PAGES_DIR) {
                            PAGE_JSON
                        } else {
                            DEF_JSON
                        };
                        let projected_header = relative.join(header);
                        keep.contains(projected_header.as_path())
                            || fs::symlink_metadata(path.join(header))
                                .is_ok_and(|metadata| metadata.is_file())
                    }
                    (Some(PAGES_DIR | COMPONENTS_DIR), 2) => name == NODES_DIR,
                    _ => false,
                };
                if scan {
                    stack.push(relative);
                }
            } else if !keep.contains(relative.as_path()) && is_generated_artifact(&relative) {
                removals.push(PlannedRemoval {
                    relative,
                    old: file_state(&path)?,
                });
            }
        }
    }
    removals.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(removals)
}

fn validate_unchanged(dir: &Path, relative: &Path, expected: &FileState) -> Result<()> {
    let actual = file_state(&dir.join(relative))?;
    if &actual != expected {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} changed while saving the project; retry after reconciling the external edit",
            relative.display()
        )));
    }
    Ok(())
}

fn apply_single_change(
    dir: &Path,
    writes: &[PlannedWrite<'_>],
    removals: &[PlannedRemoval],
) -> Result<()> {
    if let Some(write) = writes.first() {
        ensure_no_symlink_ancestors(dir, &write.relative)?;
        validate_unchanged(dir, &write.relative, &write.old)?;
        write_with_parents(&dir.join(&write.relative), write.bytes)?;
        sync_project_parents(dir, &dir.join(&write.relative))?;
    } else if let Some(removal) = removals.first() {
        ensure_no_symlink_ancestors(dir, &removal.relative)?;
        validate_unchanged(dir, &removal.relative, &removal.old)?;
        fs::remove_file(dir.join(&removal.relative))?;
        sync_project_parents(dir, &dir.join(&removal.relative))?;
    }
    Ok(())
}

fn sync_parent_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn project_parent_directories(dir: &Path, path: &Path, parents: &mut BTreeSet<PathBuf>) {
    let mut current = path.parent();
    while let Some(parent) = current.filter(|parent| parent.starts_with(dir)) {
        parents.insert(parent.to_path_buf());
        if parent == dir {
            break;
        }
        current = parent.parent();
    }
}

fn sync_project_parents(dir: &Path, path: &Path) -> Result<()> {
    let mut parents = BTreeSet::new();
    project_parent_directories(dir, path, &mut parents);
    sync_directories_deepest_first(parents)
}

fn sync_directories_deepest_first(parents: BTreeSet<PathBuf>) -> Result<()> {
    let mut parents: Vec<_> = parents.into_iter().collect();
    parents.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for parent in parents {
        sync_directory(&parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn apply_project_transaction(
    dir: &Path,
    writes: &[PlannedWrite<'_>],
    removals: &[PlannedRemoval],
) -> Result<()> {
    let staging = tempfile::Builder::new()
        .prefix(".fanta-transaction-build-")
        .tempdir_in(dir)?;
    let staged_dir = staging.path().join("staged");
    fs::create_dir(&staged_dir)?;
    let mut transaction = ProjectTransaction {
        version: 1,
        writes: Vec::with_capacity(writes.len()),
        removals: removals
            .iter()
            .map(|removal| TransactionRemoval {
                relative: removal.relative.clone(),
                old: removal.old.clone(),
            })
            .collect(),
    };
    for (index, write) in writes.iter().enumerate() {
        let staged = staged_dir.join(format!("{index:08}"));
        write_with_parents(&staged, write.bytes)?;
        if let Ok(metadata) = fs::metadata(dir.join(&write.relative)) {
            fs::set_permissions(&staged, metadata.permissions())?;
        }
        transaction.writes.push(TransactionWrite {
            relative: write.relative.clone(),
            old: write.old.clone(),
            new_sha256: hash_hex(&write.new_hash),
        });
    }
    write_with_parents(
        &staging.path().join("plan.json"),
        &serde_json::to_vec(&transaction)?,
    )?;
    sync_directory(&staged_dir)?;
    sync_parent_directory(&staging.path().join("plan.json"))?;
    fs::rename(staging.path(), dir.join(TRANSACTION_DIR))?;
    drop(staging.keep());
    sync_parent_directory(&dir.join(TRANSACTION_DIR))?;
    recover_project_transaction_locked(dir, false)
}

pub(super) fn recover_project_transaction(dir: &Path) -> Result<()> {
    if HELD_PROJECT_READ_LOCKS.with(|held| held.borrow().contains(&project_lock_key(dir))) {
        return Ok(());
    }
    let lock = project_lock(dir)?;
    let _guard = lock
        .lock()
        .map_err(|_| FormatError::InvalidProjectTree("project I/O lock is poisoned".to_owned()))?;
    recover_project_transaction_locked(dir, true)
}

fn recover_project_transaction_locked(dir: &Path, verify_staged: bool) -> Result<()> {
    let journal = dir.join(TRANSACTION_DIR);
    let metadata = match fs::symlink_metadata(&journal) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        return Err(FormatError::InvalidProjectTree(format!(
            "{} is not a transaction directory",
            journal.display()
        )));
    }
    let plan_path = journal.join("plan.json");
    let plan_bytes = match fs::read(&plan_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::remove_dir_all(&journal)?;
            sync_parent_directory(&journal)?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let transaction: ProjectTransaction = serde_json::from_slice(&plan_bytes).map_err(|error| {
        FormatError::InvalidProjectTree(format!("{} is invalid: {error}", plan_path.display()))
    })?;
    validate_transaction(&transaction)?;
    let mut parents = BTreeSet::new();
    for (index, write) in transaction.writes.iter().enumerate() {
        let destination = dir.join(&write.relative);
        ensure_no_symlink_ancestors(dir, &write.relative)?;
        let current = file_state(&destination)?;
        if current == FileState::Regular(write.new_sha256.clone()) {
            continue;
        }
        if current != write.old {
            return Err(FormatError::InvalidProjectTree(format!(
                "{} changed during transaction recovery; manual reconciliation is required",
                write.relative.display()
            )));
        }
        let staged = journal.join("staged").join(format!("{index:08}"));
        if !fs::symlink_metadata(&staged)?.file_type().is_file() {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction data for {} is not a regular file",
                write.relative.display()
            )));
        }
        if verify_staged && sha256_hex(&fs::read(&staged)?) != write.new_sha256 {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction data for {} is damaged",
                write.relative.display()
            )));
        }
        ensure_parent_directories(&destination)?;
        fs::rename(staged, &destination)?;
        project_parent_directories(dir, &destination, &mut parents);
    }
    for removal in &transaction.removals {
        let destination = dir.join(&removal.relative);
        ensure_no_symlink_ancestors(dir, &removal.relative)?;
        let current = file_state(&destination)?;
        if current == FileState::Missing {
            continue;
        }
        if current != removal.old {
            return Err(FormatError::InvalidProjectTree(format!(
                "{} changed during transaction recovery; manual reconciliation is required",
                removal.relative.display()
            )));
        }
        fs::remove_file(&destination)?;
        project_parent_directories(dir, &destination, &mut parents);
    }
    sync_directories_deepest_first(parents)?;
    fs::remove_dir_all(&journal)?;
    sync_parent_directory(&journal)?;
    Ok(())
}

fn validate_transaction(transaction: &ProjectTransaction) -> Result<()> {
    if transaction.version != 1 {
        return Err(FormatError::InvalidProjectTree(format!(
            "unsupported project transaction version {}",
            transaction.version
        )));
    }
    let mut seen = BTreeSet::new();
    if transaction
        .removals
        .iter()
        .any(|removal| removal.relative == Path::new(FANTA_JSON))
    {
        return Err(FormatError::InvalidProjectTree(
            "transaction cannot remove fanta.json".to_owned(),
        ));
    }
    for relative in transaction
        .writes
        .iter()
        .map(|write| &write.relative)
        .chain(transaction.removals.iter().map(|removal| &removal.relative))
    {
        if !relative
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction contains a non-project path {}",
                relative.display()
            )));
        }
        if relative != Path::new(FANTA_JSON) && !is_generated_artifact(relative) {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction names an unmanaged path {}",
                relative.display()
            )));
        }
        if !seen.insert(relative) {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction repeats path {}",
                relative.display()
            )));
        }
    }
    for write in &transaction.writes {
        if write.new_sha256.len() != 64
            || !write
                .new_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction has an invalid digest for {}",
                write.relative.display()
            )));
        }
    }
    Ok(())
}

fn ensure_no_symlink_ancestors(dir: &Path, relative: &Path) -> Result<()> {
    let mut ancestor = dir.to_path_buf();
    for component in relative
        .components()
        .take(relative.components().count().saturating_sub(1))
    {
        ancestor.push(component);
        if fs::symlink_metadata(&ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(FormatError::InvalidProjectTree(format!(
                "transaction path {} passes through a symlink",
                relative.display()
            )));
        }
    }
    Ok(())
}

fn prune_removed_artifact_directories(dir: &Path, removals: &[PlannedRemoval]) -> Result<()> {
    let mut candidates = BTreeSet::new();
    for removal in removals {
        let mut parent = removal.relative.parent();
        while let Some(relative_dir) = parent.filter(|path| path.components().count() >= 2) {
            candidates.insert(relative_dir.to_path_buf());
            parent = relative_dir.parent();
        }
    }
    let mut candidates: Vec<_> = candidates.into_iter().collect();
    candidates.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for relative_dir in candidates {
        let path = dir.join(relative_dir);
        match fs::remove_dir(&path) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => tracing::warn!(
                target: "fanta::format",
                path = %path.display(),
                "could not remove empty generated directory: {error}"
            ),
        }
    }
    Ok(())
}

pub(super) fn write_with_parents(path: &Path, bytes: &[u8]) -> Result<()> {
    write_with_parents_with(path, |file| file.write_all(bytes))
}

fn write_with_parents_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<()> {
    ensure_parent_directories(path)?;
    let permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let result = (|| {
        if let Some(permissions) = permissions {
            temporary.as_file().set_permissions(permissions)?;
        }
        write(temporary.as_file_mut())?;
        temporary.as_file().sync_all()
    })();
    if let Err(error) = result {
        return Err(discard_project_temporary(temporary, error).into());
    }
    if let Err(error) = temporary.persist(path) {
        return Err(discard_project_temporary(error.file, error.error).into());
    }
    Ok(())
}

fn discard_project_temporary(
    temporary: tempfile::NamedTempFile,
    primary: std::io::Error,
) -> std::io::Error {
    match temporary.close() {
        Ok(()) => primary,
        Err(cleanup) => std::io::Error::new(
            primary.kind(),
            format!("{primary}; also failed to remove the temporary project file: {cleanup}"),
        ),
    }
}

fn ensure_parent_directories(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prototype_presentation_survives_project_round_trip() {
        let directory = tempfile::tempdir().expect("project directory");
        let mut document = Doc::new();
        let page =
            fanta_doc::CanvasNode::new(fanta_doc::NodeData::Group(fanta_doc::GroupNode::default()));
        let page_id = page.id;
        document.scene.insert(page).expect("insert prototype page");
        document.add_page(page_id);
        document.flow_start = Some(page_id);
        document.flows.push(fanta_doc::Flow {
            name: "Onboarding".into(),
            start: page_id,
        });
        document.presentation = Some(fanta_doc::PresentationConfig {
            device_size: Some([390.0, 844.0]),
            preset: Some("phone".into()),
            landscape: false,
            frame_color: Some(fanta_doc::Color::rgb(0x12, 0x34, 0x56)),
        });
        write_project_tree(directory.path(), &document, &BTreeMap::new())
            .expect("save presentation");
        let (reopened, _) =
            super::super::read_project_tree(directory.path()).expect("reopen presentation");
        assert_eq!(reopened.presentation, document.presentation);
        assert_eq!(reopened.flows, document.flows);
    }

    fn assert_project_source_write_preserves_previous_bytes(name: &str, original: &[u8]) {
        let directory = tempfile::tempdir().expect("project directory");
        let destination = directory.path().join(name);
        write_with_parents(&destination, original).expect("existing project source");

        let error = write_with_parents_with(&destination, |file| {
            file.write_all(b"incomplete new source")?;
            Err(std::io::Error::other("injected source write failure"))
        })
        .expect_err("partial source write must fail");

        assert!(error.to_string().contains("injected source write failure"));
        assert_eq!(
            fs::read(&destination).expect("previous source remains readable"),
            original
        );
        assert_eq!(
            fs::read_dir(destination.parent().expect("source parent"))
                .expect("source directory")
                .count(),
            1,
            "failed writes must not leave temporary sources"
        );
    }

    #[test]
    fn project_source_partial_write_failure_preserves_existing_fnx() {
        assert_project_source_write_preserves_previous_bytes(
            "pages/home/page.fnx",
            b"export default <Frame name=\"Preserved design\" />;\n",
        );
    }

    #[test]
    fn project_source_partial_write_failure_preserves_existing_json() {
        assert_project_source_write_preserves_previous_bytes(
            "doc/metadata.json",
            b"{\"name\":\"Preserved design\",\"schema_version\":1}\n",
        );
    }

    #[test]
    fn asset_partial_write_failure_leaves_no_destination() {
        let directory = tempfile::tempdir().expect("project directory");
        let relative = Path::new("assets/video/clip.mp4");
        let error = write_with_parents_with(&directory.path().join(relative), |file| {
            file.write_all(b"partial video")?;
            Err(std::io::Error::other("injected asset write failure"))
        })
        .expect_err("partial asset write must fail");

        assert!(error.to_string().contains("injected asset write failure"));
        assert!(
            !directory.path().join(relative).exists(),
            "a failed write must not publish an incomplete asset"
        );
        assert_eq!(
            fs::read_dir(directory.path().join("assets/video"))
                .expect("asset directory")
                .count(),
            0,
            "failed writes must clean up their temporary files"
        );
    }

    #[test]
    fn asset_partial_write_retry_round_trips_exact_bytes() {
        let directory = tempfile::tempdir().expect("project directory");
        let bytes = b"\0\0\0\x18ftypisomcomplete generated video".to_vec();
        let asset = crate::asset_id_for_bytes(&bytes);
        let relative = PathBuf::from(format!("assets/video/{asset}.mp4"));
        let error = write_with_parents_with(&directory.path().join(&relative), |file| {
            file.write_all(&bytes[..12])?;
            Err(std::io::Error::other("injected asset write failure"))
        })
        .expect_err("partial asset write must fail");
        assert!(error.to_string().contains("injected asset write failure"));

        let assets = BTreeMap::from([(asset, bytes)]);
        write_project_tree(directory.path(), &Doc::new(), &assets).expect("retry project save");
        let (_, reopened_assets) =
            super::super::read_project_tree(directory.path()).expect("reopen saved project");
        assert_eq!(
            reopened_assets, assets,
            "retry must retain the complete media"
        );
    }

    #[test]
    fn asset_short_existing_file_is_repaired_before_reopen() {
        let directory = tempfile::tempdir().expect("project directory");
        let bytes = b"\x89PNG\r\n\x1a\ncomplete generated image".to_vec();
        let asset = crate::asset_id_for_bytes(&bytes);
        let relative = PathBuf::from(format!("assets/images/{asset}.png"));
        let assets = BTreeMap::from([(asset, bytes.clone())]);
        write_project_tree(directory.path(), &Doc::new(), &assets).expect("initial project save");
        fs::write(directory.path().join(&relative), &bytes[..8])
            .expect("simulate an interrupted write from an older app version");

        let report = write_project_tree(directory.path(), &Doc::new(), &assets)
            .expect("repair project save");
        let (_, reopened_assets) =
            super::super::read_project_tree(directory.path()).expect("reopen repaired project");
        assert_eq!(
            reopened_assets, assets,
            "repair must restore all media bytes"
        );
        assert!(report.written.contains(&relative));
    }

    #[test]
    fn asset_same_length_damage_is_repaired() {
        let directory = tempfile::tempdir().expect("project directory");
        let bytes = b"\x89PNG\r\n\x1a\ncomplete generated image".to_vec();
        let asset = crate::asset_id_for_bytes(&bytes);
        let relative = PathBuf::from(format!("assets/images/{asset}.png"));
        let assets = BTreeMap::from([(asset, bytes.clone())]);
        write_project_tree(directory.path(), &Doc::new(), &assets).expect("initial save");
        let mut damaged = bytes.clone();
        damaged[10] ^= 1;
        fs::write(directory.path().join(&relative), damaged).expect("damage existing asset");

        let report = write_project_tree(directory.path(), &Doc::new(), &assets).expect("repair");
        assert!(report.written.contains(&relative));
        assert_eq!(
            fs::read(directory.path().join(relative)).expect("asset"),
            bytes
        );
    }

    #[test]
    fn regeneration_preserves_unmanaged_files_and_has_an_empty_second_diff() {
        let directory = tempfile::tempdir().expect("project directory");
        let (doc, _, _, _) = cache_fixture();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");
        for relative in [
            "doc/README.md",
            "pages/home/notes.md",
            "components/button/guide.txt",
            "assets/images/license.txt",
            "pages/custom/page.fnx",
            "components/custom/master.fnx",
            "pages/home/notes/deep/plan.txt",
            "assets/images/archive/source.bin",
        ] {
            let path = directory.path().join(relative);
            fs::create_dir_all(path.parent().expect("authored file parent"))
                .expect("author directory");
            fs::write(path, b"hand-authored\n").expect("author file");
        }
        fs::write(directory.path().join(".gitignore"), b"# custom ignore\n")
            .expect("custom ignore");
        for relative in ["doc/empty", "pages/custom-empty", "assets/images/empty"] {
            fs::create_dir_all(directory.path().join(relative)).expect("authored directory");
        }

        let second = write_project_tree(directory.path(), &doc, &BTreeMap::new())
            .expect("second generation");
        assert_eq!(second, WriteReport::default());
        for relative in [
            "doc/README.md",
            "pages/home/notes.md",
            "components/button/guide.txt",
            "assets/images/license.txt",
            "pages/custom/page.fnx",
            "components/custom/master.fnx",
            "pages/home/notes/deep/plan.txt",
            "assets/images/archive/source.bin",
        ] {
            assert_eq!(
                fs::read(directory.path().join(relative)).expect("authored file"),
                b"hand-authored\n"
            );
        }
        assert_eq!(
            fs::read(directory.path().join(".gitignore")).expect("custom ignore"),
            b"# custom ignore\n"
        );
        for relative in ["doc/empty", "pages/custom-empty", "assets/images/empty"] {
            assert!(directory.path().join(relative).is_dir());
        }
        assert!(!directory.path().join(TRANSACTION_DIR).exists());
    }

    #[test]
    fn a_regular_file_at_a_projected_directory_slot_is_preserved() {
        let directory = tempfile::tempdir().expect("project directory");
        let blocker = directory.path().join("pages/home");
        fs::create_dir_all(blocker.parent().expect("pages root")).expect("pages root");
        fs::write(&blocker, b"external file").expect("external blocker");

        write_with_parents(&blocker.join("page.fnx"), b"new source")
            .expect_err("the directory collision must stop the write");
        assert_eq!(
            fs::read(&blocker).expect("external blocker"),
            b"external file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn stale_scan_does_not_follow_a_symlinked_managed_root() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("project directory");
        let doc = Doc::new();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");

        let outside = tempfile::tempdir().expect("outside directory");
        let external_page = outside.path().join("ghost/page.json");
        fs::create_dir_all(external_page.parent().expect("external design"))
            .expect("external design");
        fs::write(&external_page, b"external page").expect("external file");
        symlink(outside.path(), directory.path().join(PAGES_DIR)).expect("managed root link");

        let report = write_project_tree(directory.path(), &doc, &BTreeMap::new())
            .expect("save does not traverse symlink");
        assert_eq!(report, WriteReport::default());
        assert_eq!(
            fs::read(external_page).expect("external file"),
            b"external page"
        );
    }

    #[test]
    fn validated_source_override_preserves_authored_text_without_reprinting() {
        let directory = tempfile::tempdir().expect("project directory");
        let (doc, _, _, _) = cache_fixture();
        let mut cache = ProjectWriteCache::default();
        write_project_tree_cached(directory.path(), &doc, &BTreeMap::new(), &mut cache)
            .expect("initial save");
        let source_path = directory.path().join("pages/home/page.fnx");
        let mut source = b"// Author note retained across canvas saves\n".to_vec();
        source.extend(fs::read(&source_path).expect("generated source"));
        let sidecar =
            fs::read(directory.path().join("pages/home/page.ids.json")).expect("generated IDs");
        let sources = BTreeMap::from([(PathBuf::from("pages/home"), (source.clone(), sidecar))]);

        let report = write_project_tree_cached_with_sources(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut cache,
            &sources,
        )
        .expect("save authored source");
        assert_eq!(report.written, vec![PathBuf::from("pages/home/page.fnx")]);
        let source_hash: [u8; 32] = Sha256::digest(&source).into();
        assert_eq!(
            report.written_hashes[Path::new("pages/home/page.fnx")],
            source_hash
        );
        assert_eq!(fs::read(&source_path).expect("authored source"), source);
        assert_eq!(
            cache.cached_designs(),
            2,
            "override does not cache a reprint"
        );
        let second = write_project_tree_cached_with_sources(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut cache,
            &sources,
        )
        .expect("repeat save");
        assert_eq!(second, WriteReport::default());
        super::super::read_project_tree(directory.path()).expect("reopen authored source");
    }

    #[test]
    fn source_override_rejects_unprojected_and_escaping_paths() {
        let directory = tempfile::tempdir().expect("project directory");
        let (doc, _, _, _) = cache_fixture();
        for relative in ["pages/ghost", "pages/../../outside"] {
            let sources = BTreeMap::from([(
                PathBuf::from(relative),
                (b"<Frame />".to_vec(), b"{}".to_vec()),
            )]);
            let error = write_project_tree_cached_with_sources(
                directory.path(),
                &doc,
                &BTreeMap::new(),
                &mut ProjectWriteCache::default(),
                &sources,
            )
            .expect_err("unprojected source must be rejected");
            assert!(matches!(error, FormatError::InvalidProjectTree(_)));
            assert!(!directory.path().join(FANTA_JSON).exists());
        }
    }

    #[test]
    fn checked_write_rejects_an_external_source_edit_without_overwriting_it() {
        let directory = tempfile::tempdir().expect("project directory");
        let (doc, _, _, _) = cache_fixture();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");
        let relative = PathBuf::from("pages/home/page.fnx");
        let source = fs::read(directory.path().join(&relative)).expect("source");
        let sidecar = fs::read(directory.path().join("pages/home/page.ids.json")).expect("sidecar");
        let sources = BTreeMap::from([(PathBuf::from("pages/home"), (source.clone(), sidecar))]);
        let preconditions =
            BTreeMap::from([(relative.clone(), Some(Sha256::digest(&source).into()))]);
        let mut external = source;
        external.extend_from_slice(b"\n// external edit\n");
        fs::write(directory.path().join(&relative), &external).expect("external edit");

        let error = write_project_tree_cached_with_sources_checked(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut ProjectWriteCache::default(),
            &sources,
            &preconditions,
        )
        .expect_err("stale baseline must be rejected");
        assert!(error.to_string().contains("changed while saving"));
        assert_eq!(
            fs::read(directory.path().join(relative)).expect("external source"),
            external
        );
        assert!(!directory.path().join(TRANSACTION_DIR).exists());
    }

    #[test]
    fn checked_write_guards_singleton_files_and_rejects_unsafe_paths() {
        let directory = tempfile::tempdir().expect("project directory");
        let doc = Doc::new();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");
        let metadata = PathBuf::from("doc/metadata.json");
        let original = fs::read(directory.path().join(&metadata)).expect("metadata");
        fs::write(directory.path().join(&metadata), b"{}\n").expect("external metadata edit");
        let preconditions =
            BTreeMap::from([(metadata.clone(), Some(Sha256::digest(&original).into()))]);
        let error = write_project_tree_cached_with_sources_checked(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut ProjectWriteCache::default(),
            &BTreeMap::new(),
            &preconditions,
        )
        .expect_err("external singleton edit must be rejected");
        assert!(error.to_string().contains("changed while saving"));
        assert_eq!(
            fs::read(directory.path().join(&metadata)).expect("metadata"),
            b"{}\n"
        );

        let unsafe_preconditions = BTreeMap::from([(PathBuf::from("doc/../../outside"), None)]);
        let error = write_project_tree_cached_with_sources_checked(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut ProjectWriteCache::default(),
            &BTreeMap::new(),
            &unsafe_preconditions,
        )
        .expect_err("unsafe precondition must be rejected");
        assert!(error.to_string().contains("unmanaged path"));
    }

    #[test]
    fn checked_write_requires_a_baseline_for_each_generated_file_it_removes() {
        let directory = tempfile::tempdir().expect("project directory");
        let doc = Doc::new();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");
        let relative = PathBuf::from(format!("pages/_loose/nodes/{}.json", NodeId::from_u128(19)));
        let path = directory.path().join(&relative);
        fs::create_dir_all(path.parent().expect("generated file parent"))
            .expect("generated directory");
        let bytes = b"{\"id\":\"stale\"}\n";
        fs::write(&path, bytes).expect("external generated file");

        let error = write_project_tree_cached_with_sources_checked(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut ProjectWriteCache::default(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .expect_err("untracked removal must be rejected");
        assert!(
            error
                .to_string()
                .contains("without a reconciled disk precondition")
        );
        assert_eq!(fs::read(&path).expect("external file"), bytes);

        let preconditions =
            BTreeMap::from([(relative.clone(), Some(Sha256::digest(bytes).into()))]);
        let report = write_project_tree_cached_with_sources_checked(
            directory.path(),
            &doc,
            &BTreeMap::new(),
            &mut ProjectWriteCache::default(),
            &BTreeMap::new(),
            &preconditions,
        )
        .expect("reconciled removal");
        assert_eq!(report.removed, vec![relative]);
        assert!(!path.exists());
    }

    #[test]
    fn deleted_design_prunes_generated_artifacts_but_keeps_unmanaged_files() {
        let directory = tempfile::tempdir().expect("project directory");
        let (doc, _, _, _) = cache_fixture();
        write_project_tree(directory.path(), &doc, &BTreeMap::new()).expect("initial save");
        fs::write(directory.path().join("pages/home/notes.md"), b"keep")
            .expect("author page notes");
        fs::create_dir_all(directory.path().join("pages/custom-empty"))
            .expect("author empty page directory");
        let mut empty = Doc::new();
        empty.id = doc.id;
        empty.metadata = doc.metadata;
        let report =
            write_project_tree(directory.path(), &empty, &BTreeMap::new()).expect("remove designs");

        for generated in ["page.json", "page.fnx", "page.ids.json"] {
            assert!(
                report
                    .removed
                    .contains(&PathBuf::from("pages/home").join(generated))
            );
            assert!(!directory.path().join("pages/home").join(generated).exists());
        }
        assert_eq!(
            fs::read(directory.path().join("pages/home/notes.md")).expect("page notes"),
            b"keep"
        );
        assert!(directory.path().join("pages/custom-empty").is_dir());
        let (reopened, _) =
            super::super::read_project_tree(directory.path()).expect("reopen retained notes");
        assert!(reopened.pages().is_empty());
        assert!(!directory.path().join(TRANSACTION_DIR).exists());
    }

    #[test]
    fn opening_replays_an_interrupted_multi_file_transaction() {
        let directory = tempfile::tempdir().expect("project directory");
        write_project_tree(directory.path(), &Doc::new(), &BTreeMap::new()).expect("initial save");
        let journal = directory.path().join(TRANSACTION_DIR);
        fs::create_dir_all(journal.join("staged")).expect("journal staging");
        let mut writes = Vec::new();
        for (index, relative) in ["doc/variables.json", "doc/active_modes.json"]
            .into_iter()
            .enumerate()
        {
            let old = fs::read(directory.path().join(relative)).expect("old file");
            let mut new = old.clone();
            new.push(b'\n');
            fs::write(journal.join("staged").join(format!("{index:08}")), &new)
                .expect("stage file");
            writes.push(TransactionWrite {
                relative: PathBuf::from(relative),
                old: FileState::Regular(sha256_hex(&old)),
                new_sha256: sha256_hex(&new),
            });
        }
        fs::write(
            journal.join("plan.json"),
            serde_json::to_vec(&ProjectTransaction {
                version: 1,
                writes,
                removals: Vec::new(),
            })
            .expect("serialize plan"),
        )
        .expect("write plan");
        fs::rename(
            journal.join("staged/00000000"),
            directory.path().join("doc/variables.json"),
        )
        .expect("simulate first applied write");

        super::super::read_project_tree(directory.path()).expect("recovery on open");
        assert!(!journal.exists());
        for relative in ["doc/variables.json", "doc/active_modes.json"] {
            assert!(
                fs::read(directory.path().join(relative))
                    .expect("recovered file")
                    .ends_with(b"\n\n")
            );
        }
    }

    #[test]
    fn design_slugs_suffix_collisions_in_id_order() {
        let slugs = design_slugs(
            vec![
                (NodeId::from_u128(30), "Home".to_owned()),
                (NodeId::from_u128(10), "Home".to_owned()),
                (NodeId::from_u128(20), "Home".to_owned()),
                (NodeId::from_u128(40), "About Us".to_owned()),
            ],
            "page",
        );
        assert_eq!(slugs[&NodeId::from_u128(10)], "home");
        assert_eq!(slugs[&NodeId::from_u128(20)], "home-2");
        assert_eq!(slugs[&NodeId::from_u128(30)], "home-3");
        assert_eq!(slugs[&NodeId::from_u128(40)], "about-us");
    }

    #[test]
    fn design_slugs_never_collide_with_a_literal_suffixed_name() {
        // A page literally named "Home 2" plus two named "Home": the taken
        // "home-2" is skipped deterministically.
        let slugs = design_slugs(
            vec![
                (NodeId::from_u128(1), "Home 2".to_owned()),
                (NodeId::from_u128(2), "Home".to_owned()),
                (NodeId::from_u128(3), "Home".to_owned()),
            ],
            "page",
        );
        assert_eq!(slugs[&NodeId::from_u128(1)], "home-2");
        assert_eq!(slugs[&NodeId::from_u128(2)], "home");
        assert_eq!(slugs[&NodeId::from_u128(3)], "home-3");
    }

    #[test]
    fn design_slugs_are_a_pure_function_of_ids_and_names() {
        let named = || {
            vec![
                (NodeId::from_u128(7), "🎨".to_owned()),
                (NodeId::from_u128(9), String::new()),
                (NodeId::from_u128(8), "Page".to_owned()),
            ]
        };
        let first = design_slugs(named(), "page");
        let second = design_slugs(named(), "page");
        assert_eq!(first, second);
        // Fallback names collide with each other and with a real "Page".
        assert_eq!(first[&NodeId::from_u128(7)], "page");
        assert_eq!(first[&NodeId::from_u128(8)], "page-2");
        assert_eq!(first[&NodeId::from_u128(9)], "page-3");
    }

    #[test]
    fn classification_memo_preserves_nearest_component_and_deep_page_ancestry() {
        use fanta_doc::{CanvasNode, GroupNode, NodeData};

        let mut doc = Doc::new();
        let mut insert = |parent| {
            let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
            node.parent = parent;
            doc.scene.insert(node).expect("insert scene node")
        };
        let page = insert(None);
        let outer_component = insert(Some(page));
        let between_components = insert(Some(outer_component));
        let inner_component = insert(Some(between_components));
        let inner_child = insert(Some(inner_component));
        let outer_child = insert(Some(outer_component));
        let mut deep_page_child = insert(Some(page));
        for _ in 0..64 {
            deep_page_child = insert(Some(deep_page_child));
        }
        let orphan = insert(None);

        let components = BTreeMap::from([
            (outer_component, "outer".to_owned()),
            (inner_component, "inner".to_owned()),
        ]);
        let pages = BTreeMap::from([(page, "home".to_owned())]);
        let mut classified = BTreeMap::new();
        assert!(matches!(
            classify(&doc.scene, inner_child, &components, &pages, &mut classified),
            Bucket::Component(name) if name == "inner"
        ));
        assert!(matches!(
            classify(&doc.scene, outer_child, &components, &pages, &mut classified),
            Bucket::Component(name) if name == "outer"
        ));
        assert!(matches!(
            classify(
                &doc.scene,
                deep_page_child,
                &components,
                &pages,
                &mut classified
            ),
            Bucket::Page(name) if name == "home"
        ));
        assert!(classified.len() > 64, "deep ancestry is memoized");
        assert!(matches!(
            classify(&doc.scene, orphan, &components, &pages, &mut classified),
            Bucket::Loose
        ));
    }

    #[test]
    fn unencodable_bucket_falls_back_to_per_node_json() {
        // Two roots in one bucket can't form a single `.fnx` tree → encode
        // fails → the projection must fall back to per-node JSON, NOT abort
        // the save.
        let nodes = vec![
            json!({"type":"group","id":"AAAAAAAAAAAAAAAAAAAAAAAAAA","parent":null,"index":1.0,"name":"A"}),
            json!({"type":"group","id":"BBBBBBBBBBBBBBBBBBBBBBBBBB","parent":null,"index":2.0,"name":"B"}),
        ];
        let files: BTreeMap<PathBuf, Arc<Vec<u8>>> = project_fnx_design(
            Path::new("d"),
            "page.fnx",
            "page.ids.json",
            &nodes,
            "Multi",
            &fanta_fnx::RefTable::default(),
        )
        .unwrap()
        .into_iter()
        .collect();
        assert!(
            !files.contains_key(Path::new("d/page.fnx")),
            "multi-root bucket must not produce a .fnx"
        );
        assert_eq!(files.len(), 2, "both nodes projected as JSON");
        for key in files.keys() {
            assert!(
                key.starts_with(Path::new("d").join(NODES_DIR)),
                "fell back to nodes/: {key:?}"
            );
        }
    }

    // ---- incremental projection ---------------------------------------------

    /// Two pages with one child each, a component master, a variable, and a
    /// motion clip — enough that every `doc/` singleton takes its non-empty
    /// branch and both design kinds are exercised.
    fn cache_fixture() -> (Doc, NodeId, NodeId, NodeId) {
        use fanta_doc::{
            CanvasNode, ComponentDef, GroupNode, Mode, ModeId, NodeData, Operation, Transform2D,
            VarValue, Variable, VariableCollection, VariableCollectionId, VariableId, VariableType,
        };
        let mut doc = Doc::new();
        let mut page_a = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page_a.name = "Home".to_owned();
        let page_a = doc.scene.insert(page_a).unwrap();
        doc.add_page(page_a);
        let mut page_b = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page_b.name = "About".to_owned();
        let page_b = doc.scene.insert(page_b).unwrap();
        doc.add_page(page_b);
        let mut child_a = CanvasNode::new(NodeData::Group(GroupNode::default()));
        child_a.name = "Card".to_owned();
        child_a.parent = Some(page_a);
        doc.scene.insert(child_a).unwrap();
        let mut child_b = CanvasNode::new(NodeData::Group(GroupNode::default()));
        child_b.name = "Hero".to_owned();
        child_b.parent = Some(page_b);
        child_b.transform = Transform2D::translation(10.0, 10.0);
        let child_b = doc.scene.insert(child_b).unwrap();

        let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        master.name = "Button".to_owned();
        let master = doc.scene.insert(master).unwrap();
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef {
                id: ComponentId::new(),
                root: master,
                name: "Button".into(),
                variant_of: None,
                props: Vec::new(),
                rev: 0,
                preview_rev: 0,
            }),
        })
        .unwrap();

        let collection_id = VariableCollectionId::new();
        let mode_id = ModeId::new();
        doc.variables.collections.insert(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode_id,
                    name: "Light".into(),
                }],
                default_mode: mode_id,
                variable_order: Vec::new(),
            },
        );
        let variable_id = VariableId::new();
        doc.variables.variables.insert(
            variable_id,
            Variable {
                id: variable_id,
                collection: collection_id,
                name: "Spacing".into(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode_id, VarValue::Float { value: 8.0 })]),
                scopes: Vec::new(),
            },
        );
        doc.history = fanta_doc::History::new();
        (doc, page_a, page_b, child_b)
    }

    fn legacy_singleton(value: &Value, key: &str, fallback: Value) -> Vec<u8> {
        json_bytes(value.get(key).unwrap_or(&fallback)).unwrap()
    }

    #[test]
    fn singletons_match_the_whole_document_serialization() {
        // The typed per-field emission must reproduce what slicing the
        // whole-document `Value` produced, including the `{}` substituted
        // for a skipped empty registry.
        for doc in [Doc::new(), cache_fixture().0] {
            let value = serde_json::to_value(&doc).unwrap();
            let files = project_files(&doc).unwrap();
            let expect = |name: &str, key: &str, fallback: Value| {
                let bytes = files.get(&PathBuf::from(DOC_DIR).join(name)).unwrap();
                assert_eq!(
                    bytes.as_slice(),
                    legacy_singleton(&value, key, fallback).as_slice(),
                    "{name}"
                );
            };
            expect(METADATA_JSON, "metadata", json!({}));
            expect(VARIABLES_JSON, "variables", json!({}));
            expect(ACTIVE_MODES_JSON, "active_modes", json!({}));
            expect(MOTION_JSON, "motion", json!({}));
            expect(FLOW_START_JSON, "flow_start", Value::Null);
        }
    }

    #[test]
    fn cached_projection_is_byte_identical_and_shares_untouched_designs() {
        let (mut doc, _page_a, _page_b, child_b) = cache_fixture();
        let mut cache = ProjectWriteCache::default();
        let first = project_files_cached(&doc, &mut cache).unwrap();
        let cold = project_files(&doc).unwrap();
        assert_eq!(first, cold);
        assert_eq!(cache.cached_designs(), 3, "two pages + one component");

        // Unchanged document: every design is served from the cache (same
        // allocation), and the output is unchanged.
        let second = project_files_cached(&doc, &mut cache).unwrap();
        assert_eq!(second, first);
        for design_file in [
            "pages/home/page.fnx",
            "pages/about/page.fnx",
            "components/button/master.fnx",
        ] {
            assert!(
                Arc::ptr_eq(
                    &first[Path::new(design_file)],
                    &second[Path::new(design_file)]
                ),
                "{design_file} should be a cache hit"
            );
        }

        // One transform on page B re-projects page B only.
        doc.scene
            .set_transform(child_b, fanta_doc::Transform2D::translation(99.0, 0.0))
            .unwrap();
        let third = project_files_cached(&doc, &mut cache).unwrap();
        assert_eq!(third, project_files(&doc).unwrap());
        assert!(Arc::ptr_eq(
            &second[Path::new("pages/home/page.fnx")],
            &third[Path::new("pages/home/page.fnx")]
        ));
        assert!(Arc::ptr_eq(
            &second[Path::new("components/button/master.fnx")],
            &third[Path::new("components/button/master.fnx")]
        ));
        assert!(!Arc::ptr_eq(
            &second[Path::new("pages/about/page.fnx")],
            &third[Path::new("pages/about/page.fnx")]
        ));
        assert_ne!(
            second[Path::new("pages/about/page.fnx")],
            third[Path::new("pages/about/page.fnx")]
        );
    }

    #[test]
    fn renaming_a_component_reprojects_every_design_that_names_it() {
        // The ref table turns `component=<id>` into `component="Name"` in
        // every design, so a rename must invalidate all of them even though
        // their nodes did not change.
        let (mut doc, _, _, _) = cache_fixture();
        let mut cache = ProjectWriteCache::default();
        let before = project_files_cached(&doc, &mut cache).unwrap();
        let cid = *doc.components.defs.keys().next().unwrap();
        doc.components.defs.get_mut(&cid).unwrap().name = "PrimaryButton".into();
        let after = project_files_cached(&doc, &mut cache).unwrap();
        assert_eq!(after, project_files(&doc).unwrap());
        assert!(!Arc::ptr_eq(
            &before[Path::new("pages/home/page.fnx")],
            &after[Path::new("pages/home/page.fnx")]
        ));
        assert!(after.contains_key(Path::new("components/primarybutton/master.fnx")));
        assert!(!after.contains_key(Path::new("components/button/master.fnx")));
        assert_eq!(cache.cached_designs(), 3, "the stale slug entry is dropped");
    }

    #[test]
    fn cache_resets_when_a_different_document_is_written_through_it() {
        let (doc_a, _, _, _) = cache_fixture();
        let (doc_b, _, _, _) = cache_fixture();
        assert_ne!(doc_a.id, doc_b.id);
        let mut cache = ProjectWriteCache::default();
        project_files_cached(&doc_a, &mut cache).unwrap();
        let from_shared_cache = project_files_cached(&doc_b, &mut cache).unwrap();
        assert_eq!(from_shared_cache, project_files(&doc_b).unwrap());
        assert_eq!(cache.doc_id, Some(doc_b.id));
        assert_eq!(cache.cached_designs(), 3);
    }
}

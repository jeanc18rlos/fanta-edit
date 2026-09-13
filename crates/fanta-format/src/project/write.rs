//! `Doc` → project-tree projection.
//!
//! The projection is computed **in memory** as a pure function of the doc —
//! every file the tree should contain, byte-exact — and then applied to disk
//! as a **diff**: a file is written only when its bytes differ (assets, being
//! content-addressed, only when missing or incomplete), and files under the
//! managed subtrees (`doc/`, `pages/`, `components/`, `assets/`) that the projection
//! no longer contains are pruned, so a deleted node or page leaves no stale
//! file behind. The net disk state is identical to a full overwrite, but an
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
//! semantics are untouched — a hit still runs through `write_if_changed`.

use crate::error::{FormatError, Result};
use fanta_doc::{AssetId, ComponentId, Doc, DocId, NodeId};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::layout::{
    ACTIVE_MODES_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, EXPORTS_DIR, FANTA_JSON,
    FLOW_START_JSON, GITIGNORE, GITIGNORE_NAME, LOOSE_DIR, MASTER_FNX, MASTER_IDS, METADATA_JSON,
    MOTION_JSON, NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON, PAGES_DIR, PREVIEWS_DIR,
    PROJECT_VERSION, ProjectManifest, SETS_JSON, VARIABLES_JSON, id_from_key, json_bytes, json_key,
    slugify, sorted_entries,
};
use super::media::sniff_media;

/// What one [`write_project_tree`] call actually changed on disk. Paths are
/// relative to the project directory; both lists are sorted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Files created, or rewritten because their projected bytes differed.
    pub written: Vec<PathBuf>,
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
/// Creates `dir` if needed. Reconciles `fanta.json`, `.gitignore`, and the
/// `doc/`, `pages/`, `components/`, and `assets/` subtrees against the
/// in-memory projection — only differing files are written, only stale files
/// are removed; ensures `previews/` and `exports/` exist (their contents are
/// left alone). Asset files are content-addressed (the id in the filename
/// names the bytes), so an asset already on disk with the expected length is
/// not rewritten.
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
    fs::create_dir_all(dir)?;
    let files = project_files_cached(doc, cache)?;
    let asset_files = project_asset_files(assets);

    fs::create_dir_all(dir.join(PREVIEWS_DIR))?;
    fs::create_dir_all(dir.join(EXPORTS_DIR))?;

    // Reads and writes must never travel through a symlink or a case-aliased
    // directory: a planted symlink at a projected path would be written
    // THROUGH (clobbering its outside target), a symlinked design dir would
    // satisfy the byte-diff and then be pruned (silently dropping the
    // design), and on a case-insensitive filesystem a case-only dir rename
    // satisfies the diff through the alias while the exact-case prune deletes
    // the live files. Heal all three up front so the diff below sees a plain,
    // exactly-named tree — the old full-overwrite writer got this for free by
    // bulldozing the managed subtrees.
    heal_directory_case(dir, &files)?;
    remove_symlinks_on_projected_paths(dir, files.keys().chain(asset_files.keys()))?;

    let keep: BTreeSet<&Path> = files
        .keys()
        .chain(asset_files.keys())
        .map(PathBuf::as_path)
        .collect();

    // A slug change (rename, or a v2→v3 upgrade) moves a design to a new
    // directory; until the old one is pruned, BOTH exist and a reload sees a
    // duplicate design. Pruning only in the trailing phase would hold that
    // window open across the whole write — a mid-save failure could leave
    // every renamed design duplicated. Instead each design's superseded dir
    // is pruned immediately after its replacement dir is fully written, so a
    // failure leaves at most ONE design with duplicate dirs (which the reader
    // dedupes). The trailing prune phase remains the authoritative catch-all.
    let superseded = find_superseded_design_dirs(dir, &files);
    let mut last_design_file: BTreeMap<PathBuf, &Path> = BTreeMap::new();
    for relative in files.keys() {
        if let Some(design_dir) = design_dir_of(relative) {
            last_design_file.insert(design_dir, relative);
        }
    }

    let mut written = Vec::new();
    let mut removed = Vec::new();
    for (relative, bytes) in &files {
        if write_if_changed(dir, relative, bytes)? {
            written.push(relative.clone());
        }
        if let Some(design_dir) = design_dir_of(relative)
            && last_design_file.get(&design_dir) == Some(&relative.as_path())
            && let Some(old_dirs) = superseded.get(&design_dir)
        {
            for old_dir in old_dirs {
                prune_superseded_dir(dir, old_dir, &keep, &mut removed);
            }
        }
    }
    for (relative, bytes) in &asset_files {
        if write_if_missing(dir, relative, bytes)? {
            written.push(relative.clone());
        }
    }

    for managed in [DOC_DIR, PAGES_DIR, COMPONENTS_DIR, ASSETS_DIR] {
        let root = dir.join(managed);
        if !root.is_dir() {
            continue;
        }
        if prune_stale(dir, Path::new(managed), &keep, &mut removed)? {
            remove_dir_logging_failure(&root);
        }
    }
    // The old full overwrite always recreated a bare `assets/` root even for
    // a doc with no assets; keep that tree shape.
    fs::create_dir_all(dir.join(ASSETS_DIR))?;

    // Seed the write-if-absent editor support files (fnx.d.ts, Prettier
    // config, AGENTS.md). Previously only snapshot imports scaffolded these,
    // so an app-written project never carried the agent guide or the TSX
    // declarations that make `.fnx` a first-class authoring surface.
    crate::project::layout::ensure_project_editor_support(dir)?;

    written.sort();
    removed.sort();
    Ok(WriteReport { written, removed })
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
fn project_files_cached(doc: &Doc, cache: &mut ProjectWriteCache) -> Result<ProjectedFiles> {
    cache.retarget(doc.id);
    let mut files = ProjectedFiles::new();
    files.insert(
        PathBuf::from(FANTA_JSON),
        Arc::new(json_bytes(&serde_json::to_value(
            ProjectManifest::for_doc(doc),
        )?)?),
    );
    files.insert(
        PathBuf::from(GITIGNORE_NAME),
        Arc::new(GITIGNORE.as_bytes().to_vec()),
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
    project_designs(&mut files, doc, &refs, cache)?;
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
    ];
    for (name, value) in singletons {
        files.insert(doc_dir.join(name), Arc::new(json_bytes(&value)?));
    }
    Ok(())
}

/// Where one node's file belongs in the tree.
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

/// Page headers, component defs/sets, and one source (or fallback node file)
/// per design.
fn project_designs(
    files: &mut ProjectedFiles,
    doc: &Doc,
    refs: &fanta_fnx::RefTable,
    cache: &mut ProjectWriteCache,
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
    for (_, id) in keyed_nodes {
        match classify(&doc.scene, id, &component_roots, &page_dirs) {
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
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("component {cdir}: {e}")))?;
        live_designs.insert(design_dir);
    }
    cache
        .designs
        .retain(|design_dir, _| live_designs.contains(design_dir));
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
) -> Result<()> {
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
/// cap guards against parent cycles in hand-edited JSON.
fn classify(
    scene: &fanta_doc::Scene,
    id: NodeId,
    component_roots: &BTreeMap<NodeId, String>,
    page_dirs: &BTreeMap<NodeId, String>,
) -> Bucket {
    let mut current = id;
    let mut hops = 0usize;
    loop {
        if let Some(cdir) = component_roots.get(&current) {
            return Bucket::Component(cdir.clone());
        }
        let parent = scene.get(current).and_then(|node| node.parent);
        match parent {
            Some(p) if hops <= scene.len() && scene.contains(p) => {
                current = p;
                hops += 1;
            }
            _ => break,
        }
    }
    match page_dirs.get(&current) {
        Some(pdir) => Bucket::Page(pdir.clone()),
        None => Bucket::Loose,
    }
}

/// `assets/<family>/<asset-id>.<ext>` — family and extension sniffed from the
/// bytes (see [`super::media`]); the id in the filename is the truth.
fn project_asset_files(assets: &BTreeMap<AssetId, Vec<u8>>) -> BTreeMap<PathBuf, &[u8]> {
    let mut files = BTreeMap::new();
    for (id, bytes) in assets {
        let (family, ext) = sniff_media(bytes);
        files.insert(
            PathBuf::from(ASSETS_DIR)
                .join(family)
                .join(format!("{id}.{ext}")),
            bytes.as_slice(),
        );
    }
    files
}

/// Write `relative` only when its on-disk bytes differ (or it doesn't exist).
/// Returns whether the file was written.
fn write_if_changed(project_dir: &Path, relative: &Path, bytes: &[u8]) -> Result<bool> {
    let path = project_dir.join(relative);
    match fs::read(&path) {
        Ok(existing) if existing == bytes => return Ok(false),
        Ok(_) => {}
        Err(_) => {
            // Not readable as a file — a directory may occupy the slot (a
            // hand-made mess the old full overwrite would have bulldozed).
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            }
        }
    }
    write_with_parents(&path, bytes)?;
    Ok(true)
}

/// Content-addressed assets never change under the same filename. Checking
/// length also repairs partial files left by older non-atomic writers without
/// rereading every large binary; same-length corruption is not detected here.
fn write_if_missing(project_dir: &Path, relative: &Path, bytes: &[u8]) -> Result<bool> {
    write_if_missing_with(project_dir, relative, bytes.len(), |file| {
        file.write_all(bytes)
    })
}

fn write_if_missing_with(
    project_dir: &Path,
    relative: &Path,
    expected_length: usize,
    write: impl FnOnce(&mut fs::File) -> std::io::Result<()>,
) -> Result<bool> {
    let path = project_dir.join(relative);
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if metadata
        .as_ref()
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == expected_length as u64)
    {
        return Ok(false);
    }
    if metadata.as_ref().is_some_and(|metadata| metadata.is_dir()) {
        fs::remove_dir_all(&path)?;
    }
    write_with_parents_with(&path, write)?;
    Ok(true)
}

fn write_with_parents(path: &Path, bytes: &[u8]) -> Result<()> {
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
        // A stale regular file can occupy an ancestor directory slot (the old
        // full overwrite bulldozed these); clear the blockers and retry so the
        // second error, if any, is the real one.
        if fs::create_dir_all(parent).is_err() {
            remove_file_ancestors(parent)?;
            fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// Remove any regular file sitting where a directory is needed, from the top
/// of the chain down.
fn remove_file_ancestors(dir: &Path) -> Result<()> {
    let mut chain: Vec<&Path> = dir.ancestors().collect();
    chain.reverse();
    for ancestor in chain {
        if ancestor.is_file() {
            fs::remove_file(ancestor)?;
        }
    }
    Ok(())
}

/// The `pages/<dir>` or `components/<dir>` prefix of a projected path that
/// lies inside a design directory, or `None` for everything else (doc
/// singletons, root files, `components/sets.json`, assets).
fn design_dir_of(relative: &Path) -> Option<PathBuf> {
    let mut components = relative.components();
    let managed = components.next()?.as_os_str().to_str()?;
    let design = components.next()?;
    components.next()?;
    if managed == PAGES_DIR || managed == COMPONENTS_DIR {
        Some(Path::new(managed).join(design))
    } else {
        None
    }
}

/// Map each projected design directory to the on-disk directories it
/// supersedes: dirs under the same managed root whose name is no longer in
/// the projection but whose design id (from their JSON header, or their v2
/// id-shaped name) IS — the old homes of renamed (or v2→v3 upgraded)
/// designs. Detection is best-effort and infallible: a dir that cannot be
/// resolved is simply left for the trailing prune phase.
fn find_superseded_design_dirs(
    dir: &Path,
    files: &ProjectedFiles,
) -> BTreeMap<PathBuf, Vec<PathBuf>> {
    let mut projected_pages: BTreeMap<NodeId, PathBuf> = BTreeMap::new();
    let mut projected_components: BTreeMap<ComponentId, PathBuf> = BTreeMap::new();
    let mut projected_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for (relative, bytes) in files {
        let Some(design_dir) = design_dir_of(relative) else {
            continue;
        };
        let header: Option<Value> = match relative.file_name().and_then(|n| n.to_str()) {
            Some(name) if name == PAGE_JSON || name == DEF_JSON => {
                serde_json::from_slice(bytes).ok()
            }
            _ => None,
        };
        if let Some(header) = header
            && let Some(id) = header.get("id").and_then(Value::as_str)
        {
            if relative.starts_with(PAGES_DIR) {
                if let Ok(page) = id.parse::<NodeId>() {
                    projected_pages.insert(page, design_dir.clone());
                }
            } else if let Ok(component) = id_from_key::<ComponentId>(id) {
                projected_components.insert(component, design_dir.clone());
            }
        }
        projected_dirs.insert(design_dir);
    }

    let mut superseded: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut scan = |managed: &str, new_dir_of: &dyn Fn(&Path) -> Option<PathBuf>| {
        let Ok(entries) = fs::read_dir(dir.join(managed)) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Symlinked dirs are not descended into — the trailing prune
            // unlinks them as the files they really are.
            let is_real_dir =
                fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_dir());
            if !is_real_dir {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name == LOOSE_DIR {
                continue;
            }
            let relative = Path::new(managed).join(name);
            if projected_dirs.contains(&relative) {
                continue;
            }
            if let Some(new_dir) = new_dir_of(&path) {
                superseded.entry(new_dir).or_default().push(relative);
            }
        }
    };
    scan(PAGES_DIR, &|path| {
        super::read::page_id_of_dir(path).and_then(|id| projected_pages.get(&id).cloned())
    });
    scan(COMPONENTS_DIR, &|path| {
        super::read::component_id_of_dir(path).and_then(|id| projected_components.get(&id).cloned())
    });
    for old_dirs in superseded.values_mut() {
        old_dirs.sort();
    }
    superseded
}

/// Best-effort removal of one superseded design directory during the write
/// phase. Failures are logged, never propagated: the trailing prune phase
/// (and the next save) retries, and the reader dedupes duplicate dirs in the
/// meantime.
fn prune_superseded_dir(
    project_dir: &Path,
    relative: &Path,
    keep: &BTreeSet<&Path>,
    removed: &mut Vec<PathBuf>,
) {
    match prune_stale(project_dir, relative, keep, removed) {
        Ok(true) => remove_dir_logging_failure(&project_dir.join(relative)),
        Ok(false) => {}
        Err(error) => tracing::warn!(
            target: "fanta::format",
            "pruning superseded dir {} failed ({error}); the trailing prune retries",
            relative.display()
        ),
    }
}

/// `fs::remove_dir` that downgrades failure to a warning. A concurrently
/// created file (ENOTEMPTY) — or a managed root that is really a symlink —
/// must not abort a save whose files are already safely written; the next
/// save's prune retries.
fn remove_dir_logging_failure(path: &Path) {
    if let Err(error) = fs::remove_dir(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            target: "fanta::format",
            "could not remove directory {}: {error}",
            path.display()
        );
    }
}

/// Remove every file under `relative_dir` whose relative path is not in
/// `keep`, then every directory left empty. Returns whether `relative_dir`
/// itself is empty afterwards (the caller removes it). Symlinks are treated as
/// files — removed when stale, never followed — matching what the old
/// `remove_dir_all` wipe did.
///
/// Concurrent modification (an agent creating or deleting files while the
/// prune walks its snapshot) must not abort the prune: a failed removal is
/// logged, the entry's directory is simply reported non-empty, and everything
/// that WAS removed still lands in `removed`.
fn prune_stale(
    project_dir: &Path,
    relative_dir: &Path,
    keep: &BTreeSet<&Path>,
    removed: &mut Vec<PathBuf>,
) -> Result<bool> {
    use std::io::ErrorKind;
    let entries = match sorted_entries(&project_dir.join(relative_dir)) {
        Ok(entries) => entries,
        // Concurrently deleted out from under us — nothing left to prune.
        Err(FormatError::Io(error)) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error),
    };
    let mut empty = true;
    for path in entries {
        let Some(name) = path.file_name() else {
            continue;
        };
        let relative = relative_dir.join(name);
        let file_type = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata.file_type(),
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!(
                    target: "fanta::format",
                    "prune skipping unreadable {}: {error}",
                    path.display()
                );
                empty = false;
                continue;
            }
        };
        if file_type.is_dir() {
            if prune_stale(project_dir, &relative, keep, removed)? {
                // ENOTEMPTY here means a file appeared after the snapshot was
                // taken (a concurrent agent) — leave the dir for the next save.
                if let Err(error) = fs::remove_dir(&path)
                    && error.kind() != ErrorKind::NotFound
                {
                    tracing::warn!(
                        target: "fanta::format",
                        "could not remove directory {}: {error}",
                        path.display()
                    );
                    empty = false;
                }
            } else {
                empty = false;
            }
        } else if keep.contains(relative.as_path()) {
            empty = false;
        } else {
            match fs::remove_file(&path) {
                Ok(()) => removed.push(relative),
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(
                        target: "fanta::format",
                        "could not remove stale file {}: {error}",
                        path.display()
                    );
                    empty = false;
                }
            }
        }
    }
    Ok(empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let error = write_if_missing_with(directory.path(), relative, 64, |file| {
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
        let error = write_if_missing_with(directory.path(), &relative, bytes.len(), |file| {
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
    fn asset_complete_existing_file_keeps_the_fast_path() {
        let directory = tempfile::tempdir().expect("project directory");
        let relative = Path::new("assets/video/clip.mp4");
        let bytes = b"complete video";
        assert!(write_if_missing(directory.path(), relative, bytes).expect("initial asset save"));

        let rewritten = write_if_missing_with(directory.path(), relative, bytes.len(), |_| {
            Err(std::io::Error::other(
                "a complete asset must not be rewritten",
            ))
        })
        .expect("unchanged asset save");

        assert!(!rewritten);
        assert_eq!(
            fs::read(directory.path().join(relative)).expect("asset"),
            bytes
        );
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
    fn find_superseded_maps_old_design_dirs_to_their_replacement() {
        use fanta_doc::{CanvasNode, GroupNode, NodeData};
        let mut doc = Doc::new();
        let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
        node.name = "Home".to_owned();
        let page = node.id;
        doc.scene.insert(node).unwrap();
        doc.add_page(page);
        let files = project_files(&doc).unwrap();

        let dir = tempfile::tempdir().unwrap();
        // A crashed v3→v3 rename leftover: an old slug dir whose header still
        // carries the page id.
        let old_slug = dir.path().join("pages/old-home");
        fs::create_dir_all(&old_slug).unwrap();
        fs::write(
            old_slug.join(PAGE_JSON),
            format!("{{\"id\": \"{page}\", \"order\": 0}}"),
        )
        .unwrap();
        // A v2 leftover: id-named dir, no header.
        fs::create_dir_all(dir.path().join(format!("pages/{page}"))).unwrap();
        // A stale dir for an UNKNOWN design is not superseded — the trailing
        // prune phase owns it.
        fs::create_dir_all(dir.path().join(format!("pages/{}", NodeId::new()))).unwrap();

        let superseded = find_superseded_design_dirs(dir.path(), &files);
        assert_eq!(
            superseded,
            BTreeMap::from([(
                PathBuf::from("pages/home"),
                vec![
                    PathBuf::from(format!("pages/{page}")),
                    PathBuf::from("pages/old-home"),
                ],
            )])
        );
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

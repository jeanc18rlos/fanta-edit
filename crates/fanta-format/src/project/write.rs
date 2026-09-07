//! `Doc` → project-tree projection.
//!
//! The projection is computed **in memory** as a pure function of the doc —
//! every file the tree should contain, byte-exact — and then applied to disk
//! as a **diff**: a file is written only when its bytes differ (assets, being
//! content-addressed, only when missing), and files under the managed
//! subtrees (`doc/`, `pages/`, `components/`, `assets/`) that the projection
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

use crate::error::{FormatError, Result};
use fanta_doc::{AssetId, ComponentId, Doc, NodeId};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

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

/// Project `doc` + `assets` onto the directory tree at `dir` (spec 09 §A.2).
///
/// Creates `dir` if needed. Reconciles `fanta.json`, `.gitignore`, and the
/// `doc/`, `pages/`, `components/`, and `assets/` subtrees against the
/// in-memory projection — only differing files are written, only stale files
/// are removed; ensures `previews/` and `exports/` exist (their contents are
/// left alone). Asset files are content-addressed (the id in the filename
/// names the bytes), so an asset that already exists on disk is never
/// rewritten.
pub fn write_project_tree(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<WriteReport> {
    fs::create_dir_all(dir)?;
    let files = project_files(doc)?;
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
fn heal_directory_case(dir: &Path, files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
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

/// The complete in-memory projection of `doc`: every non-asset file the tree
/// should contain, keyed by project-relative path, with the exact bytes the
/// tree should hold.
fn project_files(doc: &Doc) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
    let value = serde_json::to_value(doc)?;
    let mut files = BTreeMap::new();
    files.insert(
        PathBuf::from(FANTA_JSON),
        json_bytes(&serde_json::to_value(ProjectManifest::for_doc(doc))?)?,
    );
    files.insert(PathBuf::from(GITIGNORE_NAME), GITIGNORE.as_bytes().to_vec());
    project_doc_singletons(&mut files, &value)?;
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
    project_designs(&mut files, doc, &value, &refs)?;
    Ok(files)
}

/// The small, mergeable doc-level files under `doc/`. Note what is *absent*:
/// `active_page`, `selection`, `viewport`, and `history` are presence state
/// and never reach disk.
fn project_doc_singletons(files: &mut BTreeMap<PathBuf, Vec<u8>>, value: &Value) -> Result<()> {
    let doc_dir = PathBuf::from(DOC_DIR);
    let empty = json!({});
    files.insert(
        doc_dir.join(METADATA_JSON),
        json_bytes(value.get("metadata").unwrap_or(&empty))?,
    );
    files.insert(
        doc_dir.join(VARIABLES_JSON),
        json_bytes(value.get("variables").unwrap_or(&empty))?,
    );
    files.insert(
        doc_dir.join(ACTIVE_MODES_JSON),
        json_bytes(value.get("active_modes").unwrap_or(&empty))?,
    );
    files.insert(
        doc_dir.join(MOTION_JSON),
        json_bytes(value.get("motion").unwrap_or(&empty))?,
    );
    files.insert(
        doc_dir.join(FLOW_START_JSON),
        json_bytes(value.get("flow_start").unwrap_or(&Value::Null))?,
    );
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
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    doc: &Doc,
    value: &Value,
    refs: &fanta_fnx::RefTable,
) -> Result<()> {
    let nodes = value
        .get("scene")
        .and_then(|s| s.get("nodes"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            FormatError::InvalidProjectTree("doc projection missing scene.nodes".into())
        })?;

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

    // Scene-key (bare ULID) → directory slug for the two kinds of design roots.
    let mut component_roots: BTreeMap<String, String> = BTreeMap::new();
    for (cid, def) in &doc.components.defs {
        component_roots.insert(json_key(&def.root)?, slug_of_component(cid)?.clone());
    }
    let mut page_dirs: BTreeMap<String, String> = BTreeMap::new();
    for page in &doc.pages {
        page_dirs.insert(json_key(page)?, slug_of_page(page)?.clone());
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
            json_bytes(&Value::Object(header))?,
        );
    }

    // components/<slug>/def.json + components/sets.json. The maps come from the
    // doc projection so the def JSON is exactly what `Doc` serializes (the def
    // carries the component's id).
    let empty = Map::new();
    let defs = value
        .get("components")
        .and_then(|c| c.get("defs"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (key, def) in defs {
        let cid: ComponentId = id_from_key(key)?;
        files.insert(
            PathBuf::from(COMPONENTS_DIR)
                .join(slug_of_component(&cid)?)
                .join(DEF_JSON),
            json_bytes(def)?,
        );
    }
    let sets = value
        .get("components")
        .and_then(|c| c.get("sets"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    files.insert(
        PathBuf::from(COMPONENTS_DIR).join(SETS_JSON),
        json_bytes(&sets)?,
    );

    // v2: one readable `.fnx` source + an `.ids` sidecar per page / component
    // (was one JSON file per node). Keys are sorted (determinism contract) then
    // grouped by their design; `_loose` orphans stay per-node JSON because they
    // need not form a single-rooted tree.
    let mut keys: Vec<&String> = nodes.keys().collect();
    keys.sort();
    let mut page_nodes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut comp_nodes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut loose: Vec<&String> = Vec::new();
    for key in keys {
        match classify(nodes, key, &component_roots, &page_dirs) {
            Bucket::Component(cdir) => comp_nodes
                .entry(cdir)
                .or_default()
                .push(nodes[key.as_str()].clone()),
            Bucket::Page(pdir) => page_nodes
                .entry(pdir)
                .or_default()
                .push(nodes[key.as_str()].clone()),
            Bucket::Loose => loose.push(key),
        }
    }

    let page_by_slug: BTreeMap<&String, NodeId> =
        page_slugs.iter().map(|(id, slug)| (slug, *id)).collect();
    let component_by_slug: BTreeMap<&String, ComponentId> = component_slugs
        .iter()
        .map(|(id, slug)| (slug, *id))
        .collect();
    for (pdir, group) in &page_nodes {
        let name = page_by_slug
            .get(pdir)
            .and_then(|id| doc.scene.get(*id))
            .map(|n| n.name.as_str());
        project_fnx_design(
            files,
            &PathBuf::from(PAGES_DIR).join(pdir),
            PAGE_FNX,
            PAGE_IDS,
            group,
            &design_name(name, "Page"),
            refs,
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("page {pdir}: {e}")))?;
    }
    for (cdir, group) in &comp_nodes {
        let name = component_by_slug
            .get(cdir)
            .and_then(|id| doc.components.defs.get(id))
            .map(|d| d.name.as_str());
        project_fnx_design(
            files,
            &PathBuf::from(COMPONENTS_DIR).join(cdir),
            MASTER_FNX,
            MASTER_IDS,
            group,
            &design_name(name, "Component"),
            refs,
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("component {cdir}: {e}")))?;
    }
    for key in loose {
        let node_id: NodeId = id_from_key(key)?;
        files.insert(
            PathBuf::from(PAGES_DIR)
                .join(LOOSE_DIR)
                .join(NODES_DIR)
                .join(format!("{node_id}.json")),
            json_bytes(&nodes[key.as_str()])?,
        );
    }
    Ok(())
}

/// Project one design's `.fnx` source + `.ids` sidecar into `design_dir`. If
/// the bucket can't be encoded as a single-rooted `.fnx` tree (e.g. a
/// multi-root bucket from corrupt data, or a node whose `type` has no JSX tag
/// yet), fall back to per-node JSON under `nodes/` — which the reader still
/// loads — rather than aborting the whole save.
fn project_fnx_design(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    nodes: &[Value],
    fn_name: &str,
    refs: &fanta_fnx::RefTable,
) -> Result<()> {
    match fanta_fnx::encode_subtree_with(nodes, fn_name, refs) {
        Ok((text, sidecar)) => {
            files.insert(design_dir.join(fnx_name), text.into_bytes());
            files.insert(
                design_dir.join(ids_name),
                json_bytes(&serde_json::to_value(&sidecar)?)?,
            );
        }
        Err(e) => {
            tracing::warn!(
                target: "fanta::format",
                dir = %design_dir.display(),
                "fnx encode failed ({e}); writing per-node JSON fallback"
            );
            project_nodes_fallback(files, &design_dir.join(NODES_DIR), nodes)?;
        }
    }
    Ok(())
}

/// The per-node JSON escape hatch: one `<id>.json` per node, the v1 shape the
/// reader falls back to when a design has no `.fnx`.
fn project_nodes_fallback(
    files: &mut BTreeMap<PathBuf, Vec<u8>>,
    nodes_dir: &Path,
    nodes: &[Value],
) -> Result<()> {
    for node in nodes {
        let key = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| FormatError::InvalidProjectTree("node missing id".into()))?;
        let node_id: NodeId = id_from_key(key)?;
        files.insert(nodes_dir.join(format!("{node_id}.json")), json_bytes(node)?);
    }
    Ok(())
}

/// A cosmetic function name for a `.fnx` file — the design's display name, or a
/// fallback. The authoritative name is the root node's `name` attribute.
fn design_name(name: Option<&str>, fallback: &str) -> String {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Walk the parent chain from `key` upward. The **nearest ancestor-or-self**
/// that is a component root wins (so component subtrees nested under a hidden
/// Components page still file under `components/`); otherwise the chain's top
/// decides: a page root files under that page, anything else is loose. The hop
/// cap guards against parent cycles in hand-edited JSON.
fn classify(
    nodes: &Map<String, Value>,
    key: &str,
    component_roots: &BTreeMap<String, String>,
    page_dirs: &BTreeMap<String, String>,
) -> Bucket {
    let mut current = key;
    let mut hops = 0usize;
    loop {
        if let Some(cdir) = component_roots.get(current) {
            return Bucket::Component(cdir.clone());
        }
        let parent = nodes
            .get(current)
            .and_then(|n| n.get("parent"))
            .and_then(Value::as_str);
        match parent {
            Some(p) if hops <= nodes.len() && nodes.contains_key(p) => {
                current = p;
                hops += 1;
            }
            _ => break,
        }
    }
    match page_dirs.get(current) {
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

/// Write `relative` only when nothing exists there. Content-addressed assets
/// never change under the same filename, so existence is the whole diff — no
/// need to read large binaries back.
fn write_if_missing(project_dir: &Path, relative: &Path, bytes: &[u8]) -> Result<bool> {
    let path = project_dir.join(relative);
    if path.is_file() {
        return Ok(false);
    }
    if path.is_dir() {
        fs::remove_dir_all(&path)?;
    }
    write_with_parents(&path, bytes)?;
    Ok(true)
}

fn write_with_parents(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        // A stale regular file can occupy an ancestor directory slot (the old
        // full overwrite bulldozed these); clear the blockers and retry so the
        // second error, if any, is the real one.
        if fs::create_dir_all(parent).is_err() {
            remove_file_ancestors(parent)?;
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(path, bytes)?;
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
    files: &BTreeMap<PathBuf, Vec<u8>>,
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
        let mut files = BTreeMap::new();
        project_fnx_design(
            &mut files,
            Path::new("d"),
            "page.fnx",
            "page.ids.json",
            &nodes,
            "Multi",
            &fanta_fnx::RefTable::default(),
        )
        .unwrap();
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
}

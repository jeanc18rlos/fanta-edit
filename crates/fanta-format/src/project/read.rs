//! Project-tree → `Doc` projection (the inverse of [`super::write`]).
//!
//! The doc is reassembled in `serde_json::Value` space from the granular
//! files, run through the schema-migration ladder if the tree was written by
//! an older build, and only then handed to `Doc::from_json_str` — which
//! rebuilds the scene's child index and validates. Presence state
//! (`active_page`, `selection`, `viewport`, `history`) is never on disk, so
//! the loaded doc carries serde defaults for all of it.

use crate::error::{FormatError, Result};
use crate::migrate::migrate;
use fanta_doc::{AssetId, ComponentId, Doc, DocId, NodeId, SCHEMA_VERSION};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::layout::{
    ACTIVE_MODES_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, FLOW_START_JSON, LOOSE_DIR,
    MASTER_FNX, MASTER_IDS, METADATA_JSON, MOTION_JSON, NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON,
    PAGES_DIR, SETS_JSON, VARIABLES_JSON, id_from_key, is_project_dir, json_key, read_json_file,
    read_json_or, read_manifest, sorted_entries,
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

pub(super) struct FnxSourceOverride<'a> {
    pub path: &'a Path,
    pub source: &'a str,
    pub sidecar: Option<&'a fanta_fnx::FnxSidecar>,
}

pub(super) fn read_project_tree_with_source_override(
    dir: &Path,
    source_override: Option<&FnxSourceOverride<'_>>,
) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
    let manifest = read_manifest(dir)?;
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
        read_json_file(&doc_dir.join(METADATA_JSON))?,
    );
    let variables = read_json_or(&doc_dir.join(VARIABLES_JSON), json!({}))?;
    root.insert("variables".to_owned(), variables.clone());
    root.insert(
        "active_modes".to_owned(),
        read_json_or(&doc_dir.join(ACTIVE_MODES_JSON), json!({}))?,
    );
    root.insert(
        "motion".to_owned(),
        read_json_or(&doc_dir.join(MOTION_JSON), json!({}))?,
    );
    let flow_start = read_json_or(&doc_dir.join(FLOW_START_JSON), Value::Null)?;
    if !flow_start.is_null() {
        root.insert("flow_start".to_owned(), flow_start);
    }

    // Index component defs (and the variables read above) BEFORE decoding any
    // design source: name-based references — `component="Button"`,
    // `"$Collection/Name"` binding paths — may appear in ANY page or master,
    // so the resolution vocabulary must exist first. The scan collects
    // headers only; masters materialize after pages, below, from the same
    // deduped winner set. Names are emitted back only for layout v4+ trees;
    // resolution is accepted on read regardless (like the width/height
    // sugar), so a hand-named reference in an older tree still loads.
    let component_scan = scan_components(dir)?;
    let refs = crate::project::refs_ctx::build_ref_table_json(
        &component_scan.defs,
        &variables,
        manifest.version >= 4,
    );

    let mut nodes = Map::new();
    root.insert(
        "pages".to_owned(),
        read_pages(dir, manifest.version, source_override, &refs, &mut nodes)?,
    );
    read_component_masters(
        &component_scan,
        manifest.version,
        source_override,
        &refs,
        &mut nodes,
    )?;
    root.insert(
        "components".to_owned(),
        json!({ "defs": component_scan.defs, "sets": component_scan.sets }),
    );
    root.insert("scene".to_owned(), json!({ "nodes": nodes }));

    let mut value = Value::Object(root);
    if manifest.schema_version < SCHEMA_VERSION {
        value = migrate(value, manifest.schema_version, SCHEMA_VERSION)?;
    }
    let doc = Doc::from_json_str(&value.to_string())
        .map_err(|e| FormatError::InvalidProjectTree(format!("doc failed to reassemble: {e}")))?;

    let assets = read_assets(dir)?;
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
fn read_pages(
    dir: &Path,
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
) -> Result<Value> {
    struct PageDir {
        id: NodeId,
        id_from_header: bool,
        entry: PathBuf,
        name: String,
        header: Value,
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
                read_nodes_into(&entry.join(NODES_DIR), nodes)?;
                continue;
            }
            let header = read_json_file(&entry.join(PAGE_JSON))?;
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
            // A page always has at least its root node, so a missing `.fnx`
            // (with no `nodes/` fallback) in a v2 tree is corruption, not empty.
            read_design_nodes(
                &candidate.entry,
                PAGE_FNX,
                PAGE_IDS,
                version,
                true,
                source_override,
                refs,
                nodes,
            )?;
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
fn select_design_winners<Id: Ord + Copy>(candidates: &[(Id, bool, &Path)]) -> HashSet<usize> {
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
    winner_dirs: Vec<PathBuf>,
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
fn scan_components(dir: &Path) -> Result<ComponentScan> {
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
        let mut candidates: Vec<ComponentDir> = Vec::new();
        for entry in sorted_entries(&comp_dir)? {
            if entry.is_file() {
                if entry.file_name().is_some_and(|n| n == SETS_JSON) {
                    sets = read_json_file(&entry)?;
                }
                continue;
            }
            if !entry.is_dir() {
                continue;
            }
            let name = dir_name(&entry)?;
            let def = read_json_file(&entry.join(DEF_JSON))?;
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
            defs.insert(json_key(&candidate.id)?, candidate.def);
            winner_dirs.push(candidate.entry);
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
fn read_component_masters(
    scan: &ComponentScan,
    version: u32,
    source_override: Option<&FnxSourceOverride<'_>>,
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
) -> Result<()> {
    for entry in &scan.winner_dirs {
        // A component master can be legitimately absent (a dangling def whose
        // master was deleted), so a missing `master.fnx` is tolerated, not
        // an error.
        read_design_nodes(
            entry,
            MASTER_FNX,
            MASTER_IDS,
            version,
            false,
            source_override,
            refs,
            nodes,
        )?;
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
    refs: &fanta_fnx::RefTable,
    nodes: &mut Map<String, Value>,
) -> Result<()> {
    let fnx_path = design_dir.join(fnx_name);
    let source_override = source_override.filter(|source| source.path == fnx_path);
    if source_override.is_none() && !fnx_path.is_file() {
        // No `.fnx`. A `nodes/` dir means either a v1 tree or a v2 design that
        // fell back to per-node JSON (an un-encodable bucket) — read it either
        // way. With neither file present, a `required` design (a page) in a v2
        // tree is corruption (a dropped `.fnx` from a bad merge/partial write),
        // NOT a silently-empty page that the next save would erase forever.
        let nodes_dir = design_dir.join(NODES_DIR);
        if nodes_dir.is_dir() {
            return read_nodes_into(&nodes_dir, nodes);
        }
        if required && version >= 2 {
            return Err(FormatError::MissingFile {
                name: fnx_path.display().to_string(),
            });
        }
        return Ok(());
    }
    let text = match source_override {
        Some(source_override) => source_override.source.to_owned(),
        None => std::fs::read_to_string(&fnx_path)?,
    };
    let sidecar: fanta_fnx::FnxSidecar =
        match source_override.and_then(|source_override| source_override.sidecar) {
            Some(sidecar) => sidecar.clone(),
            None => serde_json::from_value(read_json_file(&design_dir.join(ids_name))?).map_err(
                |error| {
                    FormatError::InvalidProjectTree(format!(
                        "{}: bad sidecar: {error}",
                        fnx_path.display()
                    ))
                },
            )?,
        };
    let sidecar = reconcile_fnx_sidecar(&fnx_path, &text, &sidecar)?;
    let decoded = fanta_fnx::decode_subtree_with(&text, &sidecar, refs)
        .map_err(|e| FormatError::InvalidProjectTree(format!("{}: {e}", fnx_path.display())))?;
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
        nodes.insert(key, node);
    }
    Ok(())
}

/// Backfill `local_size` on decoded nodes whose payload struct requires it.
/// `.fnx` source is hand- and agent-editable; an authored `<Text>` (or media /
/// `<Instance>`) element that omits both `local_size` and the `width`/`height`
/// sugar must still reassemble instead of failing the whole design with
/// "missing field `local_size`". Media nodes fall back to their intrinsic
/// `natural_size`; text estimates a box from its style and content; everything
/// else gets a visible placeholder box the user can resize.
pub(crate) fn backfill_required_geometry(node: &mut Value) {
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
    fnx_path: &Path,
    source: &str,
    sidecar: &fanta_fnx::FnxSidecar,
) -> Result<fanta_fnx::FnxSidecar> {
    let mut seed_hasher = Sha256::new();
    let canonical_path = fnx_path.canonicalize()?;
    seed_hasher.update(canonical_path.as_os_str().as_encoded_bytes());
    for entry in &sidecar.ids {
        seed_hasher.update(entry.id.as_bytes());
    }
    let seed = seed_hasher.finalize();
    let mut ordinal = sidecar.ids.len() as u64;
    let mut used_ids: HashSet<String> = sidecar.ids.iter().map(|entry| entry.id.clone()).collect();
    fanta_fnx::reconcile_sidecar(source, sidecar, || {
        loop {
            let mut id_hasher = Sha256::new();
            id_hasher.update(seed);
            id_hasher.update(ordinal.to_le_bytes());
            ordinal += 1;
            let digest = id_hasher.finalize();
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            let id = NodeId::from_u128(u128::from_be_bytes(bytes)).0.to_string();
            if used_ids.insert(id.clone()) {
                break id;
            }
        }
    })
    .map_err(|error| FormatError::InvalidProjectTree(format!("{}: {error}", fnx_path.display())))
}

/// Read every `<node-id>.json` in `nodes_dir` into the flat scene map, keyed
/// by the id's serde form. The filename is the id of record.
fn read_nodes_into(nodes_dir: &Path, nodes: &mut Map<String, Value>) -> Result<()> {
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
        nodes.insert(json_key(&id)?, read_json_file(&path)?);
    }
    Ok(())
}

/// Scan `assets/**` and parse each filename stem back into an [`AssetId`].
/// Family folder and extension are projections only; files whose stem isn't an
/// id (OS noise like `.DS_Store`) are skipped with a warning.
fn read_assets(dir: &Path) -> Result<BTreeMap<AssetId, Vec<u8>>> {
    let mut assets = BTreeMap::new();
    let assets_dir = dir.join(ASSETS_DIR);
    if assets_dir.is_dir() {
        collect_assets(&assets_dir, &mut assets)?;
    }
    Ok(assets)
}

fn collect_assets(dir: &Path, assets: &mut BTreeMap<AssetId, Vec<u8>>) -> Result<()> {
    for path in sorted_entries(dir)? {
        if path.is_dir() {
            collect_assets(&path, assets)?;
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(id) = stem.parse::<AssetId>() else {
            tracing::warn!(path = %path.display(), "skipping non-asset file in assets/");
            continue;
        };
        assets.insert(id, std::fs::read(&path)?);
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
/// the component's directory is named (v3 slug or v2 id).
pub fn locate_master_source(project_root: &Path, component: ComponentId) -> Option<PathBuf> {
    locate_design_source(
        &project_root.join(COMPONENTS_DIR),
        MASTER_FNX,
        component,
        component_id_of_dir,
    )
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
    let designs_dir = design_dir.parent()?;
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

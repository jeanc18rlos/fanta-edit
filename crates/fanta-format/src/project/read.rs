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
use std::collections::BTreeMap;
use std::path::Path;

use super::layout::{
    ACTIVE_MODES_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, FLOW_START_JSON, LOOSE_DIR,
    MASTER_FNX, MASTER_IDS, METADATA_JSON, NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON, PAGES_DIR,
    SETS_JSON, VARIABLES_JSON, json_key, read_json_file, read_json_or, read_manifest,
    sorted_entries,
};

/// Load a project directory back into a [`Doc`] plus its asset blobs.
///
/// Errors with [`FormatError::NotAProject`] when `dir` has no tagged
/// `fanta.json`, and [`FormatError::UnsupportedSchema`] when the tree was
/// written by a newer doc schema than this build understands. Older schemas
/// are migrated in JSON space before deserialization.
pub fn read_project_tree(dir: &Path) -> Result<(Doc, BTreeMap<AssetId, Vec<u8>>)> {
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
    root.insert(
        "variables".to_owned(),
        read_json_or(&doc_dir.join(VARIABLES_JSON), json!({}))?,
    );
    root.insert(
        "active_modes".to_owned(),
        read_json_or(&doc_dir.join(ACTIVE_MODES_JSON), json!({}))?,
    );
    let flow_start = read_json_or(&doc_dir.join(FLOW_START_JSON), Value::Null)?;
    if !flow_start.is_null() {
        root.insert("flow_start".to_owned(), flow_start);
    }

    let mut nodes = Map::new();
    root.insert(
        "pages".to_owned(),
        read_pages(dir, manifest.version, &mut nodes)?,
    );
    root.insert(
        "components".to_owned(),
        read_components(dir, manifest.version, &mut nodes)?,
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
fn read_pages(dir: &Path, version: u32, nodes: &mut Map<String, Value>) -> Result<Value> {
    let pages_dir = dir.join(PAGES_DIR);
    let mut pages: Vec<(u64, String, Value)> = Vec::new();
    if pages_dir.is_dir() {
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
            // A page always has at least its root node, so a missing `.fnx`
            // (with no `nodes/` fallback) in a v2 tree is corruption, not empty.
            read_design_nodes(&entry, PAGE_FNX, PAGE_IDS, version, true, nodes)?;
            let page_id: NodeId = name.parse().map_err(|e| {
                FormatError::InvalidProjectTree(format!("page dir {name:?} is not a NodeId: {e}"))
            })?;
            let header = read_json_file(&entry.join(PAGE_JSON))?;
            let order = header.get("order").and_then(Value::as_u64).ok_or_else(|| {
                FormatError::InvalidProjectTree(format!("page {name}: page.json missing order"))
            })?;
            pages.push((order, name, serde_json::to_value(page_id)?));
        }
    }
    pages.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    Ok(Value::Array(
        pages.into_iter().map(|(_, _, id)| id).collect(),
    ))
}

/// Scan `components/`: `sets.json` plus one `def.json` (and a `nodes/` pile)
/// per component directory. Returns the `ComponentLibrary` JSON.
fn read_components(dir: &Path, version: u32, nodes: &mut Map<String, Value>) -> Result<Value> {
    let comp_dir = dir.join(COMPONENTS_DIR);
    let mut defs = Map::new();
    let mut sets = json!({});
    if comp_dir.is_dir() {
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
            let cid: ComponentId = name.parse().map_err(|e| {
                FormatError::InvalidProjectTree(format!(
                    "component dir {name:?} is not a ComponentId: {e}"
                ))
            })?;
            defs.insert(json_key(&cid)?, read_json_file(&entry.join(DEF_JSON))?);
            // A component master can be legitimately absent (a dangling def whose
            // master was deleted), so a missing `master.fnx` is tolerated, not
            // an error.
            read_design_nodes(&entry, MASTER_FNX, MASTER_IDS, version, false, nodes)?;
        }
    }
    Ok(json!({ "defs": defs, "sets": sets }))
}

/// Read a design's nodes into the flat scene map. v2 decodes the `.fnx` source
/// and its `.ids` sidecar; the v1 fallback reads the per-node `nodes/` JSON pile
/// (so older project trees still load and upgrade to `.fnx` on the next save).
fn read_design_nodes(
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    version: u32,
    required: bool,
    nodes: &mut Map<String, Value>,
) -> Result<()> {
    let fnx_path = design_dir.join(fnx_name);
    if !fnx_path.is_file() {
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
    let text = std::fs::read_to_string(&fnx_path)?;
    let sidecar: fanta_fnx::FnxSidecar =
        serde_json::from_value(read_json_file(&design_dir.join(ids_name))?).map_err(|e| {
            FormatError::InvalidProjectTree(format!("{}: bad sidecar: {e}", fnx_path.display()))
        })?;
    let decoded = fanta_fnx::decode_subtree(&text, &sidecar)
        .map_err(|e| FormatError::InvalidProjectTree(format!("{}: {e}", fnx_path.display())))?;
    for node in decoded {
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

//! `Doc` → project-tree projection.
//!
//! The write is a **full overwrite** of the design subtrees (`doc/`, `pages/`,
//! `components/`, `assets/`): they are removed and rewritten from scratch so a
//! deleted node or page leaves no stale file behind. `.git/`, `previews/`, and
//! `exports/` are never touched. The projection is deterministic — identical
//! doc + assets produce a byte-identical tree (sorted iteration everywhere,
//! pretty JSON with a trailing newline, timestamps sourced from the doc).
//!
//! Presence/UI state (`active_page`, `selection`, `viewport`, `history`) is
//! deliberately **not** written: spec 09 §A.2 moves it out of the persisted
//! schema entirely.

use crate::error::{FormatError, Result};
use fanta_doc::{AssetId, ComponentId, Doc, NodeId};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::layout::{
    ACTIVE_MODES_JSON, ASSETS_DIR, COMPONENTS_DIR, DEF_JSON, DOC_DIR, EXPORTS_DIR, FANTA_JSON,
    FLOW_START_JSON, GITIGNORE, GITIGNORE_NAME, LOOSE_DIR, MASTER_FNX, MASTER_IDS, METADATA_JSON,
    NODES_DIR, PAGE_FNX, PAGE_IDS, PAGE_JSON, PAGES_DIR, PREVIEWS_DIR, ProjectManifest, SETS_JSON,
    VARIABLES_JSON, id_from_key, json_key, write_json_file,
};
use super::media::sniff_media;

/// Project `doc` + `assets` onto the directory tree at `dir` (spec 09 §A.2).
///
/// Creates `dir` if needed. Rewrites `fanta.json`, `.gitignore`, and the whole
/// `doc/`, `pages/`, `components/`, and `assets/` subtrees; ensures
/// `previews/` and `exports/` exist (their contents are left alone).
pub fn write_project_tree(
    dir: &Path,
    doc: &Doc,
    assets: &BTreeMap<AssetId, Vec<u8>>,
) -> Result<()> {
    fs::create_dir_all(dir)?;
    // Full overwrite of the design subtrees — this is what makes deletes
    // project correctly. Everything else in `dir` (.git, previews, exports,
    // user files) is preserved.
    for stale in [DOC_DIR, PAGES_DIR, COMPONENTS_DIR, ASSETS_DIR] {
        let path = dir.join(stale);
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
    }

    let value = serde_json::to_value(doc)?;
    write_json_file(
        &dir.join(FANTA_JSON),
        &serde_json::to_value(ProjectManifest::for_doc(doc))?,
    )?;
    fs::write(dir.join(GITIGNORE_NAME), GITIGNORE)?;
    fs::create_dir_all(dir.join(PREVIEWS_DIR))?;
    fs::create_dir_all(dir.join(EXPORTS_DIR))?;

    write_doc_singletons(dir, &value)?;
    write_designs(dir, doc, &value)?;
    write_assets(dir, assets)?;
    Ok(())
}

/// The small, mergeable doc-level files under `doc/`. Note what is *absent*:
/// `active_page`, `selection`, `viewport`, and `history` are presence state
/// and never reach disk.
fn write_doc_singletons(dir: &Path, value: &Value) -> Result<()> {
    let doc_dir = dir.join(DOC_DIR);
    let empty = json!({});
    write_json_file(
        &doc_dir.join(METADATA_JSON),
        value.get("metadata").unwrap_or(&empty),
    )?;
    write_json_file(
        &doc_dir.join(VARIABLES_JSON),
        value.get("variables").unwrap_or(&empty),
    )?;
    write_json_file(
        &doc_dir.join(ACTIVE_MODES_JSON),
        value.get("active_modes").unwrap_or(&empty),
    )?;
    write_json_file(
        &doc_dir.join(FLOW_START_JSON),
        value.get("flow_start").unwrap_or(&Value::Null),
    )?;
    Ok(())
}

/// Where one node's file belongs in the tree.
enum Bucket {
    /// `components/<dir>/nodes/` — the nearest ancestor-or-self is a
    /// `ComponentDef.root`.
    Component(String),
    /// `pages/<dir>/nodes/` — the parent chain tops out at a page root.
    Page(String),
    /// `pages/_loose/nodes/` — orphans: neither of the above.
    Loose,
}

/// Page headers, component defs/sets, and one file per node.
fn write_designs(dir: &Path, doc: &Doc, value: &Value) -> Result<()> {
    let nodes = value
        .get("scene")
        .and_then(|s| s.get("nodes"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            FormatError::InvalidProjectTree("doc projection missing scene.nodes".into())
        })?;

    // Scene-key (bare ULID) → directory name (prefixed Display form) for the
    // two kinds of design roots.
    let mut component_roots: BTreeMap<String, String> = BTreeMap::new();
    for (cid, def) in &doc.components.defs {
        component_roots.insert(json_key(&def.root)?, cid.to_string());
    }
    let mut page_dirs: BTreeMap<String, String> = BTreeMap::new();
    for page in &doc.pages {
        page_dirs.insert(json_key(page)?, page.to_string());
    }

    // pages/<id>/page.json — name + order. `order` is the position in
    // `doc.pages` today; the fractional-IndexKey ordering discipline (spec 02
    // §1, a shared prerequisite of spec 09) replaces this u32 with an order
    // key string when that migration lands.
    for (order, page) in doc.pages.iter().enumerate() {
        let mut header = Map::new();
        if let Some(node) = doc.scene.get(*page) {
            header.insert("name".to_owned(), Value::from(node.name.clone()));
        }
        header.insert("order".to_owned(), Value::from(order as u32));
        write_json_file(
            &dir.join(PAGES_DIR).join(page.to_string()).join(PAGE_JSON),
            &Value::Object(header),
        )?;
    }

    // components/<cid>/def.json + components/sets.json. The maps come from the
    // doc projection so the def JSON is exactly what `Doc` serializes.
    let empty = Map::new();
    let defs = value
        .get("components")
        .and_then(|c| c.get("defs"))
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (key, def) in defs {
        let cid: ComponentId = id_from_key(key)?;
        write_json_file(
            &dir.join(COMPONENTS_DIR)
                .join(cid.to_string())
                .join(DEF_JSON),
            def,
        )?;
    }
    let sets = value
        .get("components")
        .and_then(|c| c.get("sets"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    write_json_file(&dir.join(COMPONENTS_DIR).join(SETS_JSON), &sets)?;

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

    for (pdir, group) in &page_nodes {
        let name = pdir
            .parse::<NodeId>()
            .ok()
            .and_then(|id| doc.scene.get(id))
            .map(|n| n.name.as_str());
        write_fnx_design(
            &dir.join(PAGES_DIR).join(pdir),
            PAGE_FNX,
            PAGE_IDS,
            group,
            &design_name(name, "Page"),
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("page {pdir}: {e}")))?;
    }
    for (cdir, group) in &comp_nodes {
        let name = cdir
            .parse::<ComponentId>()
            .ok()
            .and_then(|id| doc.components.defs.get(&id))
            .map(|d| d.name.as_str());
        write_fnx_design(
            &dir.join(COMPONENTS_DIR).join(cdir),
            MASTER_FNX,
            MASTER_IDS,
            group,
            &design_name(name, "Component"),
        )
        .map_err(|e| FormatError::InvalidProjectTree(format!("component {cdir}: {e}")))?;
    }
    for key in loose {
        let node_id: NodeId = id_from_key(key)?;
        write_json_file(
            &dir.join(PAGES_DIR)
                .join(LOOSE_DIR)
                .join(NODES_DIR)
                .join(format!("{node_id}.json")),
            &nodes[key.as_str()],
        )?;
    }
    Ok(())
}

/// Write one design's `.fnx` source + `.ids` sidecar into `design_dir`. If the
/// bucket can't be encoded as a single-rooted `.fnx` tree (e.g. a multi-root
/// bucket from corrupt data, or a node whose `type` has no JSX tag yet), fall
/// back to per-node JSON under `nodes/` — which the reader still loads — rather
/// than aborting the whole save and leaving the just-wiped tree half-written.
fn write_fnx_design(
    design_dir: &Path,
    fnx_name: &str,
    ids_name: &str,
    nodes: &[Value],
    fn_name: &str,
) -> Result<()> {
    match fanta_fnx::encode_subtree(nodes, fn_name) {
        Ok((text, sidecar)) => {
            fs::create_dir_all(design_dir)?;
            fs::write(design_dir.join(fnx_name), text)?;
            write_json_file(&design_dir.join(ids_name), &serde_json::to_value(&sidecar)?)?;
        }
        Err(e) => {
            tracing::warn!(
                target: "fanta::format",
                dir = %design_dir.display(),
                "fnx encode failed ({e}); writing per-node JSON fallback"
            );
            write_nodes_fallback(&design_dir.join(NODES_DIR), nodes)?;
        }
    }
    Ok(())
}

/// The per-node JSON escape hatch: one `<id>.json` per node, the v1 shape the
/// reader falls back to when a design has no `.fnx`.
fn write_nodes_fallback(nodes_dir: &Path, nodes: &[Value]) -> Result<()> {
    for node in nodes {
        let key = node
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| FormatError::InvalidProjectTree("node missing id".into()))?;
        let node_id: NodeId = id_from_key(key)?;
        write_json_file(&nodes_dir.join(format!("{node_id}.json")), node)?;
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
fn write_assets(dir: &Path, assets: &BTreeMap<AssetId, Vec<u8>>) -> Result<()> {
    fs::create_dir_all(dir.join(ASSETS_DIR))?;
    let mut ids: Vec<AssetId> = assets.keys().copied().collect();
    ids.sort_by_key(|id| id.to_u128());
    for id in ids {
        let bytes = &assets[&id];
        let (family, ext) = sniff_media(bytes);
        let family_dir = dir.join(ASSETS_DIR).join(family);
        fs::create_dir_all(&family_dir)?;
        fs::write(family_dir.join(format!("{id}.{ext}")), bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn unencodable_bucket_falls_back_to_per_node_json() {
        // Two roots in one bucket can't form a single `.fnx` tree → encode
        // fails → we must fall back to per-node JSON, NOT abort the save.
        let dir = tempdir().unwrap();
        let design = dir.path().join("d");
        let nodes = vec![
            json!({"type":"group","id":"AAAAAAAAAAAAAAAAAAAAAAAAAA","parent":null,"index":1.0,"name":"A"}),
            json!({"type":"group","id":"BBBBBBBBBBBBBBBBBBBBBBBBBB","parent":null,"index":2.0,"name":"B"}),
        ];
        write_fnx_design(&design, "page.fnx", "page.ids.json", &nodes, "Multi").unwrap();
        assert!(
            !design.join("page.fnx").exists(),
            "multi-root bucket must not produce a .fnx"
        );
        assert!(design.join(NODES_DIR).is_dir(), "fell back to nodes/");
        assert_eq!(
            fs::read_dir(design.join(NODES_DIR)).unwrap().count(),
            2,
            "both nodes written as JSON"
        );
    }
}

//! Independent spec-conformance checks for the project-tree projection
//! (spec 09 §A.2/§A.3), written by the verifier rather than the implementer.
//!
//! Covers: presence exclusion (grep-equivalent over the whole tree),
//! id-is-truth round-trip with a derived-path spot check, byte-determinism via
//! an external tree walk, and `.git`/`previews/` preservation across rewrites.

use fanta_doc::{
    AssetId, CanvasNode, ComponentDef, ComponentId, Doc, GroupNode, NodeData, NodeId, Viewport,
};
use fanta_format::{asset_id_for_bytes, read_project_tree, write_project_tree};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn group(parent: Option<NodeId>, name: &str) -> CanvasNode {
    let mut n = CanvasNode::new(NodeData::Group(GroupNode::default()));
    n.parent = parent;
    n.name = name.to_owned();
    n
}

struct Fix {
    doc: Doc,
    page: NodeId,
    child: NodeId,
    comp_root: NodeId,
    comp_id: ComponentId,
}

/// A doc with one page, a nested child, a component master, and *live presence
/// state*: an active page, a non-empty selection, and a moved viewport.
fn doc_with_presence() -> Fix {
    let mut doc = Doc::new();
    doc.metadata.title = "Verifier Fixture".into();
    doc.metadata.created_at = 1_750_000_000;
    doc.metadata.modified_at = 1_750_000_001;

    let mut insert = |node: CanvasNode| -> NodeId {
        let id = node.id;
        doc.scene.insert(node).unwrap();
        id
    };
    let page = insert(group(None, "Page One"));
    let child = insert(group(Some(page), "Child Frame"));
    let comp_root = insert(group(Some(page), "Master"));
    doc.add_page(page);

    let comp_id = ComponentId::new();
    doc.components
        .defs
        .insert(comp_id, ComponentDef::new(comp_id, comp_root, "Master"));

    // Presence state that must never reach disk.
    doc.set_active_page(Some(page));
    doc.selection.select_only(child);
    doc.viewport = Viewport {
        center: [123.0, -45.0],
        zoom: 3.5,
    };

    Fix {
        doc,
        page,
        child,
        comp_root,
        comp_id,
    }
}

fn fixture_assets() -> BTreeMap<AssetId, Vec<u8>> {
    let blobs: [&[u8]; 2] = [b"\x89PNG\r\n\x1a\nverifier-image", b"verifier raw bytes"];
    blobs
        .iter()
        .map(|b| (asset_id_for_bytes(b), b.to_vec()))
        .collect()
}

/// Every *file* under `root`, keyed by `/`-joined relative path — plus an
/// entry per directory (trailing `/`) so structural drift is caught too.
fn walk_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn rec(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if path.is_dir() {
                out.insert(format!("{rel}/"), Vec::new());
                rec(root, &path, out);
            } else {
                out.insert(rel, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    rec(root, root, &mut out);
    out
}

/// (a) Presence exclusion: the grep-equivalent over every byte of the tree —
/// paths *and* contents — finds no `active_page`, `selection`, or `viewport`.
#[test]
fn verify_presence_grep_finds_nothing() {
    let f = doc_with_presence();
    let dir = tempdir().unwrap();
    write_project_tree(dir.path(), &f.doc, &fixture_assets()).unwrap();

    let tree = walk_tree(dir.path());
    assert!(!tree.is_empty(), "tree was written");
    for (rel, bytes) in &tree {
        // Root-level seeded *documentation* (the agent guide, TSX
        // declarations, Prettier config) may legitimately use these words to
        // EXPLAIN that presence is never persisted — the exclusion this test
        // pins is about managed data files, not prose. Everything under
        // doc/ pages/ components/ assets/ stays fully grepped.
        if matches!(rel.as_str(), "AGENTS.md" | "fnx.d.ts" | ".prettierrc.json") {
            continue;
        }
        let haystack = format!("{rel}\n{}", String::from_utf8_lossy(bytes));
        for needle in ["active_page", "selection", "viewport"] {
            assert!(
                !haystack.contains(needle),
                "grep hit: {needle:?} found in {rel}"
            );
        }
    }
}

/// (b) Id-is-truth: every node/component/asset id survives the round trip
/// byte-exactly, and a node's identity is recorded in its design's `.ids`
/// sidecar (`pages/<slug>/page.ids.json`, not a per-node file) while the
/// design's own identity lives in the header (`page.json` "id" / `def.json`).
#[test]
fn verify_ids_survive_and_paths_are_derivable() {
    let f = doc_with_presence();
    let assets = fixture_assets();
    let dir = tempdir().unwrap();
    write_project_tree(dir.path(), &f.doc, &assets).unwrap();

    let bare = |id| {
        serde_json::to_value(id)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    };
    let sidecar = |path: &Path| -> fanta_fnx::FnxSidecar {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    };
    let has = |sc: &fanta_fnx::FnxSidecar, id: &str| sc.ids.iter().any(|e| e.id == id);

    // v2: one readable source + id sidecar per page; the page root and its
    // child are both recorded in the page's sidecar. v3: the directory name is
    // the page name's slug and page.json records the id.
    assert!(
        dir.path().join("pages/page-one/page.fnx").is_file(),
        "page source missing"
    );
    let page_header: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join("pages/page-one/page.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        page_header.get("id").and_then(serde_json::Value::as_str),
        Some(f.page.to_string().as_str()),
        "page.json records the page id"
    );
    let page_sc = sidecar(&dir.path().join("pages/page-one/page.ids.json"));
    assert!(has(&page_sc, &bare(f.page)), "page root id in sidecar");
    assert!(has(&page_sc, &bare(f.child)), "child id in sidecar");
    // The component master files under components/<slug>/, not the page.
    assert!(
        dir.path().join("components/master/master.fnx").is_file(),
        "component source missing"
    );
    let comp_sc = sidecar(&dir.path().join("components/master/master.ids.json"));
    assert!(
        has(&comp_sc, &bare(f.comp_root)),
        "component root id in sidecar"
    );

    let (doc2, assets2) = read_project_tree(dir.path()).unwrap();
    assert_eq!(doc2.id, f.doc.id, "doc id byte-exact");
    for id in [f.page, f.child, f.comp_root] {
        let orig = f.doc.scene.get(id).unwrap();
        let back = doc2.scene.get(id).expect("node id survives");
        assert_eq!(back.id, orig.id);
        assert_eq!(back.parent, orig.parent);
        assert_eq!(back.name, orig.name);
    }
    let def = doc2.components.defs.get(&f.comp_id).expect("component id");
    assert_eq!(def.id, f.comp_id);
    assert_eq!(def.root, f.comp_root);
    assert_eq!(doc2.pages(), f.doc.pages());
    assert_eq!(assets2, assets, "asset ids and bytes byte-exact");
}

/// (c) Determinism: the same doc + assets written to two directories produce
/// recursively byte-identical trees (path sets and file bytes).
#[test]
fn verify_two_writes_are_byte_identical() {
    let f = doc_with_presence();
    let assets = fixture_assets();
    let a = tempdir().unwrap();
    let b = tempdir().unwrap();
    write_project_tree(a.path(), &f.doc, &assets).unwrap();
    write_project_tree(b.path(), &f.doc, &assets).unwrap();

    let ta = walk_tree(a.path());
    let tb = walk_tree(b.path());
    assert_eq!(
        ta.keys().collect::<Vec<_>>(),
        tb.keys().collect::<Vec<_>>(),
        "relative path sets differ"
    );
    for (rel, bytes) in &ta {
        assert_eq!(bytes, &tb[rel], "bytes differ at {rel}");
    }
}

/// (d) .git safety: pre-existing `.git/marker` and `previews/keep.png` survive
/// two consecutive full rewrites.
#[test]
fn verify_git_and_previews_survive_rewrites() {
    let f = doc_with_presence();
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/marker"), b"do not delete").unwrap();
    fs::create_dir_all(dir.path().join("previews")).unwrap();
    fs::write(dir.path().join("previews/keep.png"), b"png bytes").unwrap();

    write_project_tree(dir.path(), &f.doc, &fixture_assets()).unwrap();
    write_project_tree(dir.path(), &f.doc, &fixture_assets()).unwrap();

    assert_eq!(
        fs::read(dir.path().join(".git/marker")).unwrap(),
        b"do not delete"
    );
    assert_eq!(
        fs::read(dir.path().join("previews/keep.png")).unwrap(),
        b"png bytes"
    );
}

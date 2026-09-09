//! The incremental project writer: a `ProjectWriteCache` carried across
//! saves must change nothing about the tree on disk — only how much of it is
//! recomputed.

use fanta_doc::{
    CanvasNode, ComponentDef, ComponentId, Doc, GroupNode, NodeData, NodeId, Operation, Transform2D,
};
use fanta_format::{
    ProjectWriteCache, read_project_tree, write_project_tree, write_project_tree_cached,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

struct Fixture {
    doc: Doc,
    page_a: NodeId,
    page_b: NodeId,
    child_b: NodeId,
}

fn fixture() -> Fixture {
    let mut doc = Doc::new();
    let mut page_a = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_a.name = "Home".into();
    let page_a = doc.scene.insert(page_a).unwrap();
    doc.add_page(page_a);
    let mut page_b = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_b.name = "About".into();
    let page_b = doc.scene.insert(page_b).unwrap();
    doc.add_page(page_b);

    let mut child_a = CanvasNode::new(NodeData::Group(GroupNode::default()));
    child_a.name = "Card".into();
    child_a.parent = Some(page_a);
    doc.scene.insert(child_a).unwrap();
    let mut child_b = CanvasNode::new(NodeData::Group(GroupNode::default()));
    child_b.name = "Hero".into();
    child_b.parent = Some(page_b);
    child_b.transform = Transform2D::translation(10.0, 20.0);
    let child_b = doc.scene.insert(child_b).unwrap();

    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.name = "Button".into();
    let master = doc.scene.insert(master).unwrap();
    doc.apply(Operation::DefineComponent {
        def: Box::new(ComponentDef {
            id: ComponentId::new(),
            root: master,
            name: "Button".into(),
            variant_of: None,
            props: Vec::new(),
            rev: 0,
        }),
    })
    .unwrap();
    Fixture {
        doc,
        page_a,
        page_b,
        child_b,
    }
}

fn assets() -> BTreeMap<fanta_doc::AssetId, Vec<u8>> {
    let mut assets = BTreeMap::new();
    assets.insert(
        fanta_doc::AssetId::from_u128(0xCAFE),
        b"\x89PNG\r\n\x1a\nnot really a png".to_vec(),
    );
    assets
}

/// Every regular file under `root` (relative path → bytes), skipping the
/// write-if-absent editor support files whose content is not the doc's.
fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                out.insert(relative, fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn cached_rewrite_is_byte_identical_to_a_cold_write_and_touches_nothing() {
    let fixture = fixture();
    let assets = assets();
    let cached_dir = tempdir().unwrap();
    let cold_dir = tempdir().unwrap();

    let mut cache = ProjectWriteCache::default();
    write_project_tree_cached(cached_dir.path(), &fixture.doc, &assets, &mut cache).unwrap();
    assert_eq!(cache.cached_designs(), 3);
    let second =
        write_project_tree_cached(cached_dir.path(), &fixture.doc, &assets, &mut cache).unwrap();
    assert!(second.written.is_empty(), "nothing changed: {second:?}");
    assert!(second.removed.is_empty(), "nothing stale: {second:?}");

    write_project_tree(cold_dir.path(), &fixture.doc, &assets).unwrap();
    assert_eq!(
        tree_snapshot(cached_dir.path()),
        tree_snapshot(cold_dir.path()),
        "the cached tree must be byte-identical to a cold projection"
    );
}

#[test]
fn a_cache_hit_still_restores_an_externally_modified_file() {
    let fixture = fixture();
    let dir = tempdir().unwrap();
    let mut cache = ProjectWriteCache::default();
    write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();

    let source = dir.path().join("pages/home/page.fnx");
    let projected = fs::read(&source).unwrap();
    fs::write(&source, "// hand edit\n").unwrap();

    let report =
        write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();
    assert_eq!(report.written, vec![PathBuf::from("pages/home/page.fnx")]);
    assert_eq!(fs::read(&source).unwrap(), projected);
}

#[test]
fn one_transform_change_rewrites_only_that_design() {
    let mut fixture = fixture();
    let dir = tempdir().unwrap();
    let mut cache = ProjectWriteCache::default();
    write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();

    let before = fixture.doc.scene.get(fixture.child_b).unwrap().transform;
    fixture
        .doc
        .apply(Operation::SetTransform {
            id: fixture.child_b,
            old: before,
            new: Transform2D::translation(300.0, 20.0),
        })
        .unwrap();
    let report =
        write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();

    assert!(
        report
            .written
            .contains(&PathBuf::from("pages/about/page.fnx")),
        "the edited page's source is rewritten: {report:?}"
    );
    for path in &report.written {
        assert!(
            path.starts_with("pages/about") || path.starts_with("doc"),
            "only the edited design (and the doc-level metadata whose timestamp moved) may change, got {path:?}"
        );
    }
    assert!(report.removed.is_empty());

    let (reloaded, _) = read_project_tree(dir.path()).unwrap();
    assert_eq!(
        reloaded.scene.get(fixture.child_b).unwrap().transform,
        Transform2D::translation(300.0, 20.0)
    );
    assert!(reloaded.scene.contains(fixture.page_a));
    assert!(reloaded.scene.contains(fixture.page_b));
}

#[test]
fn renaming_a_page_moves_its_directory_through_the_cache() {
    let mut fixture = fixture();
    let dir = tempdir().unwrap();
    let mut cache = ProjectWriteCache::default();
    write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();
    assert!(dir.path().join("pages/about/page.fnx").is_file());

    fixture.doc.scene.get_mut(fixture.page_b).unwrap().name = "Team".into();
    let report =
        write_project_tree_cached(dir.path(), &fixture.doc, &BTreeMap::new(), &mut cache).unwrap();

    assert!(dir.path().join("pages/team/page.fnx").is_file());
    assert!(dir.path().join("pages/team/page.json").is_file());
    assert!(
        !dir.path().join("pages/about").exists(),
        "old slug dir pruned"
    );
    assert!(
        report
            .removed
            .contains(&PathBuf::from("pages/about/page.fnx")),
        "{report:?}"
    );
    assert_eq!(cache.cached_designs(), 3, "the stale slug is not memoized");

    let cold = tempdir().unwrap();
    write_project_tree(cold.path(), &fixture.doc, &BTreeMap::new()).unwrap();
    assert_eq!(tree_snapshot(dir.path()), tree_snapshot(cold.path()));

    let (reloaded, _) = read_project_tree(dir.path()).unwrap();
    assert_eq!(reloaded.pages(), fixture.doc.pages());
    assert_eq!(reloaded.page_name(fixture.page_b), Some("Team"));
}

#[test]
fn a_cache_is_safe_to_reuse_across_documents() {
    let first = fixture();
    let second = fixture();
    assert_ne!(first.doc.id, second.doc.id);
    let mut cache = ProjectWriteCache::default();

    let first_dir = tempdir().unwrap();
    write_project_tree_cached(first_dir.path(), &first.doc, &BTreeMap::new(), &mut cache).unwrap();

    let second_dir = tempdir().unwrap();
    write_project_tree_cached(second_dir.path(), &second.doc, &BTreeMap::new(), &mut cache)
        .unwrap();
    let cold = tempdir().unwrap();
    write_project_tree(cold.path(), &second.doc, &BTreeMap::new()).unwrap();
    assert_eq!(tree_snapshot(second_dir.path()), tree_snapshot(cold.path()));

    // Going back to the first document is a full re-projection, still exact.
    let again = tempdir().unwrap();
    write_project_tree_cached(again.path(), &first.doc, &BTreeMap::new(), &mut cache).unwrap();
    assert_eq!(tree_snapshot(again.path()), tree_snapshot(first_dir.path()));
}

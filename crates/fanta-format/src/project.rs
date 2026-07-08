//! The git-native project directory — spec 09 §A.2's granular on-disk tree.
//!
//! Where a `.fant` is one zip with one `doc.json` blob, a *project* is a plain
//! directory in which every design unit is its own file subtree:
//!
//! ```text
//! <project>/
//!   fanta.json                       # ProjectManifest (format tag, versions, ids)
//!   doc/                             # doc-level singletons (small, mergeable)
//!     metadata.json  variables.json  active_modes.json  flow_start.json
//!   pages/<page-id>/page.json        # { name?, order }
//!   pages/<page-id>/page.fnx         # the page tree as one React/JSX source
//!   pages/<page-id>/page.ids.json    # id/index sidecar for page.fnx
//!   pages/_loose/nodes/<id>.json     # orphan nodes (no page / component root)
//!   components/<cid>/def.json        # ComponentDef
//!   components/<cid>/master.fnx      # the master subtree as one source file
//!   components/<cid>/master.ids.json # id/index sidecar for master.fnx
//!   components/sets.json             # ComponentSet registry
//!   assets/<family>/<asset-id>.<ext> # sniffed family folder + extension
//!   previews/  exports/              # derived; git-ignored
//!   .gitignore
//! ```
//!
//! Invariants (spec 09 §A.2 rules):
//!
//! - **Ids are truth, paths are projection.** Directory and file names are the
//!   ids' `Display` form (`n_<ULID>`, `c_<ULID>`, `a_<ULID>`); readers parse
//!   them back and never trust anything else about a path.
//! - **A design = one directory.** Every scene node files under exactly one
//!   `pages/<id>/` or `components/<id>/` (orphans under `pages/_loose/`),
//!   chosen by walking its parent chain — see `write::classify`.
//! - **Presence state is not persisted.** `active_page`, `selection`,
//!   `viewport`, and `history` never reach disk; reads restore serde defaults.
//! - **Deterministic bytes.** Identical doc + assets project to a
//!   byte-identical tree: sorted iteration, pretty JSON + trailing newline,
//!   manifest timestamps sourced from the doc's own metadata.
//!
//! This module is the projection layer only — git plumbing and app wiring live
//! upstream (`fanta-app`), per spec 09 §A.1/§A.3.

mod layout;
mod media;
mod read;
mod snapshot;
mod write;

pub use layout::{ProjectManifest, is_project_dir, scaffold_project_tree};
pub use read::read_project_tree;
pub use snapshot::{export_fant_snapshot, import_fant_snapshot};
pub use write::write_project_tree;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_id_for_bytes;
    use crate::error::FormatError;
    use fanta_doc::{
        AssetId, CanvasNode, ComponentDef, ComponentId, Doc, GroupNode, History, Mode, ModeId,
        NodeData, NodeId, Selection, VarValue, Variable, VariableCollection, VariableCollectionId,
        VariableId, VariableType, Viewport,
    };
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    // ---- fixtures ----------------------------------------------------------

    fn group(parent: Option<NodeId>, name: &str) -> CanvasNode {
        let mut n = CanvasNode::new(NodeData::Group(GroupNode::default()));
        n.parent = parent;
        n.name = name.to_owned();
        n
    }

    struct Fixture {
        doc: Doc,
        page1: NodeId,
        page2: NodeId,
        frame: NodeId,
        inner: NodeId,
        comp_page: NodeId,
        comp_root: NodeId,
        comp_child: NodeId,
        comp_id: ComponentId,
        orphan: NodeId,
        orphan_child: NodeId,
    }

    /// Two real pages, nested groups, a component master under a hidden
    /// Components page, variables/modes/flow_start, an orphan subtree, and
    /// presence state (active page, selection, viewport) that must NOT persist.
    fn fixture() -> Fixture {
        let mut doc = Doc::new();
        doc.metadata.title = "Project Fixture".into();
        doc.metadata.created_at = 1_700_000_000;
        doc.metadata.modified_at = 1_700_000_001;

        let mut insert = |node: CanvasNode| -> NodeId {
            let id = node.id;
            doc.scene.insert(node).unwrap();
            id
        };
        let page1 = insert(group(None, "Page 1"));
        let frame = insert(group(Some(page1), "Hero"));
        let inner = insert(group(Some(frame), "Hero / Inner"));
        let page2 = insert(group(None, "Page 2"));
        let comp_page = insert(group(None, "Components"));
        let comp_root = insert(group(Some(comp_page), "Button"));
        let comp_child = insert(group(Some(comp_root), "Button / Label"));
        let orphan = insert(group(None, "Orphan"));
        let orphan_child = insert(group(Some(orphan), "Orphan / Child"));

        doc.add_page(page1);
        doc.add_page(page2);
        doc.add_page(comp_page);

        let comp_id = ComponentId::new();
        doc.components
            .defs
            .insert(comp_id, ComponentDef::new(comp_id, comp_root, "Button"));

        let coll = VariableCollectionId::new();
        let mode = ModeId::new();
        let var = VariableId::new();
        doc.variables.collections.insert(
            coll,
            VariableCollection {
                id: coll,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "Light".into(),
                }],
                default_mode: mode,
                variable_order: Vec::new(),
            },
        );
        doc.variables.variables.insert(
            var,
            Variable {
                id: var,
                collection: coll,
                name: "radius".into(),
                ty: VariableType::Float,
                values_by_mode: BTreeMap::from([(mode, VarValue::Float { value: 8.0 })]),
                scopes: Vec::new(),
            },
        );
        doc.active_modes.insert(coll, mode);
        doc.flow_start = Some(page1);

        // Presence state — written by the editor, dropped by the projection.
        doc.set_active_page(Some(page2));
        doc.selection.select_only(frame);
        doc.viewport = Viewport {
            center: [10.0, 20.0],
            zoom: 2.0,
        };

        Fixture {
            doc,
            page1,
            page2,
            frame,
            inner,
            comp_page,
            comp_root,
            comp_child,
            comp_id,
            orphan,
            orphan_child,
        }
    }

    /// Assets covering five sniffed families. Returns (map, expected
    /// `(id, family, ext)` table).
    #[allow(clippy::type_complexity)]
    fn fixture_assets() -> (
        BTreeMap<AssetId, Vec<u8>>,
        Vec<(AssetId, &'static str, &'static str)>,
    ) {
        let blobs: Vec<(&[u8], &str, &str)> = vec![
            (b"\x89PNG\r\n\x1a\nfixture-image", "images", "png"),
            (b"ID3\x03\x00fixture-audio", "audio", "mp3"),
            (b"glTF\x02\x00\x00\x00fixture-model", "models", "glb"),
            (b"<svg xmlns='x'>fixture</svg>", "svg", "svg"),
            (b"fixture: no magic at all", "other", "bin"),
        ];
        let mut map = BTreeMap::new();
        let mut expected = Vec::new();
        for (bytes, family, ext) in blobs {
            let id = asset_id_for_bytes(bytes);
            map.insert(id, bytes.to_vec());
            expected.push((id, family, ext));
        }
        (map, expected)
    }

    /// The persisted projection of a doc: its JSON with the four
    /// presence-state fields removed.
    fn persisted(doc: &Doc) -> Value {
        let mut v = serde_json::to_value(doc).unwrap();
        if let Value::Object(map) = &mut v {
            for key in ["selection", "history", "viewport", "active_page"] {
                map.remove(key);
            }
        }
        v
    }

    /// Every file in `root`, keyed by `/`-joined relative path. Directories
    /// appear too (with a trailing `/`) so empty dirs count in comparisons.
    fn tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if path.is_dir() {
                    out.insert(format!("{rel}/"), Vec::new());
                    walk(root, &path, out);
                } else {
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(root, root, &mut out);
        out
    }

    /// The bare-ULID serde form of a node id (the form `.ids` sidecars store).
    fn bare(id: NodeId) -> String {
        serde_json::to_value(id)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Deserialize a `.ids` sidecar JSON file.
    fn read_sidecar(path: &Path) -> fanta_fnx::FnxSidecar {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    // ---- tests -------------------------------------------------------------

    #[test]
    fn write_files_page_roots_and_nodes_under_their_page() {
        // Page roots serialize as node files of their own page, alongside
        // their nested descendants.
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let p = dir.path();
        assert!(p.join(format!("pages/{}/page.json", f.page1)).is_file());
        // v2: one readable source + id sidecar per page (not nodes/*.json).
        assert!(
            p.join(format!("pages/{}/page.fnx", f.page1)).is_file(),
            "page source file"
        );
        assert!(
            p.join(format!("pages/{}/page.ids.json", f.page1)).is_file(),
            "page id sidecar"
        );
        assert!(p.join(format!("pages/{}/page.fnx", f.page2)).is_file());
        assert!(
            !p.join(format!("pages/{}/nodes", f.page1)).exists(),
            "v1 nodes/ dir must not be written"
        );
        // The page root + its descendants are all captured in the one source.
        let sc = read_sidecar(&p.join(format!("pages/{}/page.ids.json", f.page1)));
        for node in [f.page1, f.frame, f.inner] {
            assert!(
                sc.ids.iter().any(|e| e.id == bare(node)),
                "page1 sidecar missing a node"
            );
        }
    }

    #[test]
    fn write_files_component_subtrees_and_components_page_root() {
        // Component subtrees file under components/ even though they sit
        // beneath the (hidden) Components page; the Components page root
        // itself stays a page node.
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let p = dir.path();
        assert!(
            p.join(format!("components/{}/def.json", f.comp_id))
                .is_file()
        );
        // v2: the master subtree is one readable source + id sidecar.
        assert!(
            p.join(format!("components/{}/master.fnx", f.comp_id))
                .is_file(),
            "component source file"
        );
        let sc = read_sidecar(&p.join(format!("components/{}/master.ids.json", f.comp_id)));
        for node in [f.comp_root, f.comp_child] {
            assert!(
                sc.ids.iter().any(|e| e.id == bare(node)),
                "component sidecar missing a node"
            );
        }
        assert!(p.join("components/sets.json").is_file());
        // The components page root is page-owned → its own page source.
        assert!(
            p.join(format!("pages/{}/page.fnx", f.comp_page)).is_file(),
            "components page root is page-owned"
        );
    }

    #[test]
    fn write_files_assets_under_sniffed_family_folders() {
        let f = fixture();
        let (assets, expected_paths) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let p = dir.path();
        for (id, family, ext) in &expected_paths {
            let path = p.join(format!("assets/{family}/{id}.{ext}"));
            assert!(path.is_file(), "missing {}", path.display());
        }
    }

    #[test]
    fn round_trip_preserves_persisted_projection_and_assets() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let (doc2, assets2) = read_project_tree(dir.path()).unwrap();
        assert_eq!(persisted(&doc2), persisted(&f.doc));
        assert_eq!(assets2, assets);
        assert_eq!(doc2.pages(), f.doc.pages(), "page order survives");
    }

    #[test]
    fn write_is_deterministic() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        write_project_tree(a.path(), &f.doc, &assets).unwrap();
        write_project_tree(b.path(), &f.doc, &assets).unwrap();
        assert_eq!(tree_snapshot(a.path()), tree_snapshot(b.path()));
    }

    #[test]
    fn rewrite_removes_stale_node_and_page_files() {
        let mut f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let page1_ids = dir.path().join(format!("pages/{}/page.ids.json", f.page1));
        let page2_dir = dir.path().join(format!("pages/{}", f.page2));
        assert!(
            read_sidecar(&page1_ids)
                .ids
                .iter()
                .any(|e| e.id == bare(f.inner)),
            "inner is captured in page1's source before deletion"
        );
        assert!(page2_dir.is_dir());

        f.doc.scene.remove(f.inner).unwrap();
        f.doc.remove_page(f.page2);
        f.doc.scene.remove(f.page2).unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        assert!(
            !read_sidecar(&page1_ids)
                .ids
                .iter()
                .any(|e| e.id == bare(f.inner)),
            "deleted node must be gone from the page source"
        );
        assert!(!page2_dir.exists(), "deleted page's directory must be gone");
        let (doc2, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(persisted(&doc2), persisted(&f.doc));
    }

    #[test]
    fn v2_page_with_missing_fnx_errors_instead_of_loading_empty() {
        // A bad merge / partial write that drops a page's `.fnx` (keeping
        // `page.json`) must be a hard error, not a silently-empty page that the
        // next save would erase. The v1 `nodes/` fallback is version-gated.
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let page_dir = dir.path().join(format!("pages/{}", f.page1));
        fs::remove_file(page_dir.join("page.fnx")).unwrap();
        fs::remove_file(page_dir.join("page.ids.json")).unwrap();
        assert!(
            !page_dir.join("nodes").exists(),
            "no nodes/ fallback present"
        );

        let err = read_project_tree(dir.path()).unwrap_err();
        assert!(
            matches!(err, FormatError::MissingFile { .. }),
            "expected MissingFile, got {err:?}"
        );
    }

    #[test]
    fn reads_legacy_v1_nodes_dir_layout() {
        // v1 stored one JSON file per node under `<design>/nodes/`. The v2
        // loader must still read that shape (a save then upgrades it to `.fnx`).
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        let baseline = {
            let (doc2, _) = read_project_tree(dir.path()).unwrap();
            persisted(&doc2)
        };

        downgrade_to_v1(dir.path());

        let (doc_v1, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(
            persisted(&doc_v1),
            baseline,
            "v1 nodes/ fallback must read identically to the v2 .fnx tree"
        );
    }

    /// Rewrite a v2 tree into the v1 shape in place: explode each `.fnx`/`.ids`
    /// back into `<design>/nodes/<id>.json` and stamp the manifest to v1.
    fn downgrade_to_v1(root: &Path) {
        for (design_root, fnx_name, ids_name) in [
            ("pages", "page.fnx", "page.ids.json"),
            ("components", "master.fnx", "master.ids.json"),
        ] {
            let base = root.join(design_root);
            if !base.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&base).unwrap() {
                let design_dir = entry.unwrap().path();
                let fnx = design_dir.join(fnx_name);
                if !fnx.is_file() {
                    continue;
                }
                let sidecar: fanta_fnx::FnxSidecar =
                    serde_json::from_str(&fs::read_to_string(design_dir.join(ids_name)).unwrap())
                        .unwrap();
                let decoded =
                    fanta_fnx::decode_subtree(&fs::read_to_string(&fnx).unwrap(), &sidecar)
                        .unwrap();
                let nodes_dir = design_dir.join("nodes");
                fs::create_dir_all(&nodes_dir).unwrap();
                for node in decoded {
                    let id: NodeId =
                        serde_json::from_value(node.get("id").unwrap().clone()).unwrap();
                    let mut s = serde_json::to_string_pretty(&node).unwrap();
                    s.push('\n');
                    fs::write(nodes_dir.join(format!("{id}.json")), s).unwrap();
                }
                fs::remove_file(&fnx).unwrap();
                fs::remove_file(design_dir.join(ids_name)).unwrap();
            }
        }
        let manifest = root.join("fanta.json");
        let mut m: Value = serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
        m["version"] = Value::from(1u32);
        let mut s = serde_json::to_string_pretty(&m).unwrap();
        s.push('\n');
        fs::write(&manifest, s).unwrap();
    }

    #[test]
    fn presence_state_never_reaches_disk_and_reads_back_as_defaults() {
        let f = fixture();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();

        for (rel, bytes) in tree_snapshot(dir.path()) {
            let text = String::from_utf8_lossy(&bytes);
            for needle in [
                "\"active_page\"",
                "\"viewport\"",
                "\"selection\"",
                "\"history\"",
            ] {
                assert!(
                    !text.contains(needle),
                    "{rel} contains presence key {needle}"
                );
            }
        }

        let (doc2, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(doc2.active_page(), None);
        assert_eq!(
            serde_json::to_value(&doc2.selection).unwrap(),
            serde_json::to_value(Selection::new()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(doc2.viewport).unwrap(),
            serde_json::to_value(Viewport::default()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&doc2.history).unwrap(),
            serde_json::to_value(History::new()).unwrap()
        );
    }

    #[test]
    fn orphan_nodes_round_trip_via_loose() {
        let f = fixture();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();

        for id in [f.orphan, f.orphan_child] {
            assert!(
                dir.path()
                    .join(format!("pages/_loose/nodes/{id}.json"))
                    .is_file(),
                "orphan {id} must file under pages/_loose/"
            );
        }
        let (doc2, _) = read_project_tree(dir.path()).unwrap();
        let orphan = doc2.scene.get(f.orphan).expect("orphan restored");
        assert_eq!(orphan.parent, None);
        assert_eq!(
            doc2.scene
                .get(f.orphan_child)
                .expect("orphan child restored")
                .parent,
            Some(f.orphan)
        );
        assert!(!doc2.pages().contains(&f.orphan), "_loose is not a page");
    }

    #[test]
    fn reading_a_non_project_dir_errors_cleanly() {
        let dir = tempdir().unwrap();
        let err = read_project_tree(dir.path()).unwrap_err();
        assert!(
            matches!(err, FormatError::NotAProject { .. }),
            "got {err:?}"
        );
        assert!(!is_project_dir(dir.path()));
    }

    #[test]
    fn newer_doc_schema_is_rejected_older_is_migrated() {
        let f = fixture();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
        let manifest_path = dir.path().join("fanta.json");
        let manifest_text = fs::read_to_string(&manifest_path).unwrap();

        // Future doc schema → refuse to load.
        let mut v: Value = serde_json::from_str(&manifest_text).unwrap();
        v["schema_version"] = Value::from(fanta_doc::SCHEMA_VERSION + 1);
        fs::write(&manifest_path, v.to_string()).unwrap();
        let err = read_project_tree(dir.path()).unwrap_err();
        assert!(
            matches!(err, FormatError::UnsupportedSchema { found, .. }
                if found == fanta_doc::SCHEMA_VERSION + 1),
            "got {err:?}"
        );

        // Older doc schema → migrated up before deserialization (the v1→v2
        // step is idempotent on already-v2 node shapes).
        let mut v: Value = serde_json::from_str(&manifest_text).unwrap();
        v["schema_version"] = Value::from(1);
        fs::write(&manifest_path, v.to_string()).unwrap();
        let (doc2, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(doc2.schema_version, fanta_doc::SCHEMA_VERSION);
        assert_eq!(persisted(&doc2), persisted(&f.doc));
    }

    #[test]
    fn write_preserves_git_previews_and_exports() {
        let f = fixture();
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/dev\n").unwrap();
        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
        fs::write(dir.path().join("previews/p.png"), b"png").unwrap();
        fs::write(dir.path().join("exports/e.fant"), b"zip").unwrap();

        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".git/HEAD")).unwrap(),
            "ref: refs/heads/dev\n"
        );
        assert!(dir.path().join("previews/p.png").is_file());
        assert!(dir.path().join("exports/e.fant").is_file());
    }
}

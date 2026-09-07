//! The git-native project directory — spec 09 §A.2's granular on-disk tree.
//!
//! Where a `.fant` is one zip with one `doc.json` blob, a *project* is a plain
//! directory in which every design unit is its own file subtree:
//!
//! ```text
//! <project>/
//!   fanta.json                        # ProjectManifest (format tag, versions, ids)
//!   doc/                              # doc-level singletons (small, mergeable)
//!     metadata.json  variables.json  active_modes.json  motion.json  flow_start.json
//!   pages/<slug>/page.json            # { id, name?, order }
//!   pages/<slug>/page.fnx             # the page tree as one React/JSX source
//!   pages/<slug>/page.ids.json        # id/index sidecar for page.fnx
//!   pages/_loose/nodes/<id>.json      # orphan nodes (no page / component root)
//!   components/<slug>/def.json        # ComponentDef
//!   components/<slug>/master.fnx      # the master subtree as one source file
//!   components/<slug>/master.ids.json # id/index sidecar for master.fnx
//!   components/sets.json              # ComponentSet registry
//!   assets/<family>/<asset-id>.<ext>  # sniffed family folder + extension
//!   previews/  exports/               # derived; git-ignored
//!   .gitignore
//! ```
//!
//! Invariants (spec 09 §A.2 rules):
//!
//! - **Ids are truth, paths are projection.** Design directories are named by
//!   a slug of the design's name (layout v3; name collisions get `-2`, `-3`
//!   suffixes in id order); identity lives in the JSON headers (`page.json`'s
//!   `"id"`, `def.json`). Asset file names are still the ids' `Display` form
//!   (`a_<ULID>`). Readers take identity from the header and fall back to
//!   parsing v2's id-named directories; [`locate_page_source`] /
//!   [`locate_master_source`] / [`page_scope_of_source`] resolve between ids
//!   and source paths without callers knowing the slug rules.
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
mod merge;
mod read;
mod refs_ctx;
pub mod session;
mod snapshot;
mod source_edit;
mod write;

pub use fanta_fnx::canonicalize_legacy_source;
pub use layout::{
    ProjectManifest, ensure_project_editor_support, is_project_dir, scaffold_project_tree,
};
pub use merge::{
    ArtifactAddress, ArtifactMerge, DocMerge, JsonPathSegment, NodeMapEdition, PresenceValue,
    PropertyConflict, merge_artifact, merge_docs,
};
pub use read::{
    ScopedDesign, component_id_of_dir, locate_master_source, locate_page_source, page_id_of_dir,
    page_scope_of_source, read_project_tree,
};
pub use session::{
    ApplyReport, ArtifactDirty, ArtifactId, ArtifactMeta, ArtifactOpImpact, ArtifactRenderRevision,
    ArtifactRenderSnapshot, ArtifactSession, ClosePolicy, ConflictResolution, ContentHash,
    DependencyGraph, DocMutGuard, FsEvent, MergeReview, MotionIndex, MotionSource, ProposalApplied,
    ReviewConflict, ReviewResolution, SaveBlocked, SaveResult, ScopedDoc, SessionError,
    SessionEvent, SourceDiagnostic, SourceProposalOutcome, SourceRebuildReason, SourceSeverity,
    SourceSync, WorkspaceDirty, WorkspaceIr, WorkspaceSession, WorkspaceSharedState,
    artifact_op_impact, hash_file_set, read_motion_dual, synthesize_workspace_fnx,
    write_artifact_files,
};
pub use snapshot::{export_fant_snapshot, import_fant_snapshot};
pub use source_edit::{
    ProjectSourceEdit, apply_project_source_edit, apply_project_source_edit_with_diagnostics,
    validate_project_source_edit, validate_project_source_edit_with_diagnostics,
};
pub use write::{WriteReport, write_project_tree};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_id_for_bytes;
    use crate::error::FormatError;
    use fanta_doc::{
        AnimationClip, AnimationClipId, AnimationTrack, AnimationTrackId, AssetId, CanvasNode,
        ComponentDef, ComponentId, Doc, Easing, GroupNode, History, Interpolation, Keyframe,
        KeyframeId, Mode, ModeId, MotionProperty, MotionTarget, NodeData, NodeId, ResolvedVarValue,
        Selection, VarValue, Variable, VariableCollection, VariableCollectionId, VariableId,
        VariableType, Viewport,
    };
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};
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

    fn add_motion_fixture(doc: &mut Doc, node: NodeId) {
        let clip_id = AnimationClipId::new();
        let track_id = AnimationTrackId::new();
        let target = MotionTarget::new(node, MotionProperty::PositionX);
        let mut track = AnimationTrack::new(track_id, target);

        let first_id = KeyframeId::new();
        track.keyframes.insert(
            first_id,
            Keyframe {
                id: first_id,
                time_ms: 0,
                value: ResolvedVarValue::Float { value: 0.0 },
                interpolation: Interpolation::Linear,
                easing: Easing::CubicBezier {
                    x1: 0.25,
                    y1: 0.1,
                    x2: 0.25,
                    y2: 1.0,
                },
            },
        );
        let second_id = KeyframeId::new();
        track.keyframes.insert(
            second_id,
            Keyframe {
                id: second_id,
                time_ms: 500,
                value: ResolvedVarValue::Float { value: 50.0 },
                interpolation: Interpolation::Hold,
                easing: Easing::Linear,
            },
        );
        let third_id = KeyframeId::new();
        track.keyframes.insert(
            third_id,
            Keyframe {
                id: third_id,
                time_ms: 1_000,
                value: ResolvedVarValue::Float { value: 100.0 },
                interpolation: Interpolation::Linear,
                easing: Easing::EaseOut,
            },
        );

        let mut clip = AnimationClip::new(clip_id, "Entrance", 1_000);
        clip.tracks.insert(track_id, track);
        doc.motion.clips.insert(clip_id, clip);
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

    /// The bare-ULID serde form of a component id (the form `def.json` stores).
    fn bare_component(id: ComponentId) -> String {
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

    /// Copy the regular files directly inside `src` into `dst` (design dirs
    /// are flat: header + source + sidecar).
    fn copy_design_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::copy(&path, dst.join(path.file_name().unwrap())).unwrap();
            }
        }
    }

    /// Backdate every file in `dir` by an hour — a crash leftover predates
    /// the save that superseded it.
    fn backdate_design_dir(dir: &Path) {
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_file() {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(past)
                    .unwrap();
            }
        }
    }

    // ---- tests -------------------------------------------------------------

    #[test]
    fn write_files_page_roots_and_nodes_under_their_page() {
        // Page roots serialize as node files of their own page, alongside
        // their nested descendants. v3: the directory name is the page name's
        // slug and the page's id lives in page.json.
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let p = dir.path();
        assert!(p.join("pages/page-1/page.json").is_file());
        let header: Value =
            serde_json::from_str(&fs::read_to_string(p.join("pages/page-1/page.json")).unwrap())
                .unwrap();
        assert_eq!(
            header.get("id").and_then(Value::as_str),
            Some(f.page1.to_string().as_str()),
            "page.json carries the page id"
        );
        // v2: one readable source + id sidecar per page (not nodes/*.json).
        assert!(
            p.join("pages/page-1/page.fnx").is_file(),
            "page source file"
        );
        assert!(
            p.join("pages/page-1/page.ids.json").is_file(),
            "page id sidecar"
        );
        assert!(p.join("pages/page-2/page.fnx").is_file());
        assert!(
            !p.join("pages/page-1/nodes").exists(),
            "v1 nodes/ dir must not be written"
        );
        // The page root + its descendants are all captured in the one source.
        let sc = read_sidecar(&p.join("pages/page-1/page.ids.json"));
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
        // v3: the component dir is the def name's slug; def.json's id is the
        // identity of record.
        assert!(p.join("components/button/def.json").is_file());
        let def: Value = serde_json::from_str(
            &fs::read_to_string(p.join("components/button/def.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            def.get("id").and_then(Value::as_str),
            Some(bare_component(f.comp_id).as_str()),
            "def.json carries the component id"
        );
        // v2: the master subtree is one readable source + id sidecar.
        assert!(
            p.join("components/button/master.fnx").is_file(),
            "component source file"
        );
        let sc = read_sidecar(&p.join("components/button/master.ids.json"));
        for node in [f.comp_root, f.comp_child] {
            assert!(
                sc.ids.iter().any(|e| e.id == bare(node)),
                "component sidecar missing a node"
            );
        }
        assert!(p.join("components/sets.json").is_file());
        // The components page root is page-owned → its own page source.
        assert!(
            p.join("pages/components/page.fnx").is_file(),
            "components page root is page-owned"
        );
        let page_header: Value = serde_json::from_str(
            &fs::read_to_string(p.join("pages/components/page.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            page_header.get("id").and_then(Value::as_str),
            Some(f.comp_page.to_string().as_str())
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
    fn motion_round_trips_with_interpolation_and_custom_easing() {
        let mut fixture = fixture();
        let target = fixture.frame;
        add_motion_fixture(&mut fixture.doc, target);
        let expected_motion = fixture.doc.motion.clone();
        let dir = tempdir().unwrap();

        write_project_tree(dir.path(), &fixture.doc, &BTreeMap::new()).unwrap();

        let motion_path = dir.path().join("doc/motion.json");
        let written_motion: Value =
            serde_json::from_str(&fs::read_to_string(&motion_path).unwrap()).unwrap();
        assert_eq!(
            written_motion,
            serde_json::to_value(&expected_motion).unwrap()
        );
        let (restored, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(restored.motion, expected_motion);
    }

    #[test]
    fn missing_motion_singleton_defaults_to_an_empty_library() {
        let mut fixture = fixture();
        let target = fixture.frame;
        add_motion_fixture(&mut fixture.doc, target);
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &fixture.doc, &BTreeMap::new()).unwrap();
        fs::remove_file(dir.path().join("doc/motion.json")).unwrap();

        let (restored, _) = read_project_tree(dir.path()).unwrap();

        assert!(restored.motion.is_empty());
        assert!(restored.scene.get(target).is_some());
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

        let page1_ids = dir.path().join("pages/page-1/page.ids.json");
        let page2_dir = dir.path().join("pages/page-2");
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

        let page_dir = dir.path().join("pages/page-1");
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

    /// Rewrite a v3 tree into the v2 shape in place: rename each design dir
    /// from its slug to its id, strip the `page.json` "id" field, and stamp
    /// the manifest to v2.
    fn downgrade_to_v2(root: &Path) {
        let pages = root.join("pages");
        for entry in fs::read_dir(&pages).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() || dir.file_name().unwrap() == "_loose" {
                continue;
            }
            let header_path = dir.join("page.json");
            let mut header: Value =
                serde_json::from_str(&fs::read_to_string(&header_path).unwrap()).unwrap();
            let id = header.as_object_mut().unwrap().remove("id").unwrap();
            let mut s = serde_json::to_string_pretty(&header).unwrap();
            s.push('\n');
            fs::write(&header_path, s).unwrap();
            fs::rename(&dir, pages.join(id.as_str().unwrap())).unwrap();
        }
        let components = root.join("components");
        for entry in fs::read_dir(&components).unwrap() {
            let dir = entry.unwrap().path();
            if !dir.is_dir() {
                continue;
            }
            let def: Value =
                serde_json::from_str(&fs::read_to_string(dir.join("def.json")).unwrap()).unwrap();
            let key = def.get("id").unwrap().as_str().unwrap().to_owned();
            fs::rename(&dir, components.join(format!("c_{key}"))).unwrap();
        }
        let manifest = root.join("fanta.json");
        let mut m: Value = serde_json::from_str(&fs::read_to_string(&manifest).unwrap()).unwrap();
        m["version"] = Value::from(2u32);
        let mut s = serde_json::to_string_pretty(&m).unwrap();
        s.push('\n');
        fs::write(&manifest, s).unwrap();
    }

    #[test]
    fn reads_v2_id_named_dir_layout_and_upgrades_on_save() {
        // v2 named the design dirs by id and had no id in page.json. The v3
        // loader must read that shape identically (dir-name fallback), and the
        // next full-overwrite save must upgrade the dirs to slugs.
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        let baseline = {
            let (doc2, _) = read_project_tree(dir.path()).unwrap();
            persisted(&doc2)
        };

        downgrade_to_v2(dir.path());
        assert!(
            dir.path().join(format!("pages/{}", f.page1)).is_dir(),
            "downgrade produced id-named page dirs"
        );
        assert!(
            dir.path()
                .join(format!("components/{}", f.comp_id))
                .is_dir(),
            "downgrade produced id-named component dirs"
        );

        let (doc_v2, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(
            persisted(&doc_v2),
            baseline,
            "v2 id-named dirs must read identically to the v3 slug tree"
        );

        write_project_tree(dir.path(), &doc_v2, &assets).unwrap();
        assert!(
            dir.path().join("pages/page-1/page.fnx").is_file(),
            "save upgrades page dirs to slugs"
        );
        assert!(
            !dir.path().join(format!("pages/{}", f.page1)).exists(),
            "id-named page dir is gone after the upgrade save"
        );
        assert!(
            dir.path().join("components/button/master.fnx").is_file(),
            "save upgrades component dirs to slugs"
        );
    }

    /// One page ("Home" → "Home Renamed") saved twice, with a snapshot of the
    /// first save's design dir so crash leftovers can be reconstructed.
    /// Returns the project dir, the snapshot dir, the doc in its RENAMED
    /// state, and the (page, child) ids.
    fn renamed_page_project() -> (tempfile::TempDir, tempfile::TempDir, Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        doc.metadata.created_at = 1_700_000_000;
        doc.metadata.modified_at = 1_700_000_001;
        let mut insert = |node: CanvasNode| -> NodeId {
            let id = node.id;
            doc.scene.insert(node).unwrap();
            id
        };
        let page = insert(group(None, "Home"));
        let child = insert(group(Some(page), "Old Child"));
        doc.add_page(page);
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

        let snapshot = tempdir().unwrap();
        copy_design_dir(
            &dir.path().join("pages/home"),
            &snapshot.path().join("home"),
        );

        doc.scene.get_mut(page).unwrap().name = "Home Renamed".to_owned();
        doc.scene.get_mut(child).unwrap().name = "New Child".to_owned();
        write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();
        assert!(
            !dir.path().join("pages/home").exists(),
            "the rename save pruned the old dir"
        );
        (dir, snapshot, doc, page, child)
    }

    #[test]
    fn stale_v2_dir_from_a_crashed_rename_save_loses_to_the_fresh_v3_dir() {
        // A save that dies between writing pages/<new-slug>/ and pruning the
        // old id-named dir leaves both claiming the same page. The reader must
        // load the fresh header-carrying dir once and skip the stale node
        // batch entirely. The stale copy is deliberately NEWER on disk, so
        // only the header-over-dir-name rule can pick the winner.
        let (dir, snapshot, doc, page, child) = renamed_page_project();

        let stale = dir.path().join(format!("pages/{page}"));
        copy_design_dir(&snapshot.path().join("home"), &stale);
        let header_path = stale.join("page.json");
        let mut header: Value =
            serde_json::from_str(&fs::read_to_string(&header_path).unwrap()).unwrap();
        header.as_object_mut().unwrap().remove("id").unwrap();
        fs::write(&header_path, serde_json::to_string_pretty(&header).unwrap()).unwrap();

        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(reread.pages(), doc.pages(), "one page entry, the real id");
        assert_eq!(reread.scene.get(page).unwrap().name, "Home Renamed");
        assert_eq!(reread.scene.get(child).unwrap().name, "New Child");
        assert_eq!(persisted(&reread), persisted(&doc));
    }

    #[test]
    fn stale_v3_dir_from_a_crashed_rename_save_loses_on_freshness() {
        // A v3→v3 rename leftover carries a page.json header too, so the
        // header rule can't discriminate — the newer-mtime rule must pick the
        // replacement (a crash leftover always predates it).
        let (dir, snapshot, doc, page, child) = renamed_page_project();

        let stale = dir.path().join("pages/home");
        copy_design_dir(&snapshot.path().join("home"), &stale);
        backdate_design_dir(&stale);

        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(reread.pages(), doc.pages(), "one page entry, the real id");
        assert_eq!(reread.scene.get(page).unwrap().name, "Home Renamed");
        assert_eq!(reread.scene.get(child).unwrap().name, "New Child");
        assert_eq!(persisted(&reread), persisted(&doc));
    }

    #[test]
    fn stale_component_dir_from_a_crashed_rename_save_is_deduped() {
        let mut doc = Doc::new();
        doc.metadata.created_at = 1_700_000_000;
        doc.metadata.modified_at = 1_700_000_001;
        let mut insert = |node: CanvasNode| -> NodeId {
            let id = node.id;
            doc.scene.insert(node).unwrap();
            id
        };
        let comp_page = insert(group(None, "Components"));
        let comp_root = insert(group(Some(comp_page), "Button"));
        let label = insert(group(Some(comp_root), "Old Label"));
        doc.add_page(comp_page);
        let cid = ComponentId::new();
        doc.components
            .defs
            .insert(cid, ComponentDef::new(cid, comp_root, "Button"));
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

        let snapshot = tempdir().unwrap();
        copy_design_dir(
            &dir.path().join("components/button"),
            &snapshot.path().join("button"),
        );

        doc.components.defs.get_mut(&cid).unwrap().name = "Button Renamed".to_owned();
        doc.scene.get_mut(label).unwrap().name = "New Label".to_owned();
        write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();
        assert!(!dir.path().join("components/button").exists());

        // The v2-style leftover: id-named dir whose def.json still carries the
        // id (defs always did) — both dirs are header-carrying, so freshness
        // must decide.
        let stale = dir.path().join(format!("components/{cid}"));
        copy_design_dir(&snapshot.path().join("button"), &stale);
        backdate_design_dir(&stale);

        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(reread.components.defs.len(), 1, "one def entry");
        assert_eq!(
            reread.components.defs.get(&cid).unwrap().name,
            "Button Renamed"
        );
        assert_eq!(reread.scene.get(label).unwrap().name, "New Label");
        assert_eq!(persisted(&reread), persisted(&doc));
    }

    /// A concurrent agent can drop a file into a directory the prune is about
    /// to remove — `fs::remove_dir` then fails (ENOTEMPTY) after the sorted
    /// snapshot was taken. Simulate the same failure deterministically by
    /// denying the prune the right to unlink entries of `pages/` itself: the
    /// stale page's files are removable but the dir removal fails, and the
    /// save must log-and-continue — completing, and still reporting what WAS
    /// removed.
    #[cfg(unix)]
    #[test]
    fn failed_dir_removal_during_prune_does_not_abort_the_save() {
        use std::os::unix::fs::PermissionsExt;
        let mut f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        f.doc.remove_page(f.page2);
        f.doc.scene.remove(f.page2).unwrap();

        let pages_root = dir.path().join("pages");
        fs::set_permissions(&pages_root, fs::Permissions::from_mode(0o555)).unwrap();
        let result = write_project_tree(dir.path(), &f.doc, &assets);
        fs::set_permissions(&pages_root, fs::Permissions::from_mode(0o755)).unwrap();

        let report = result.expect("a failed dir removal must not abort the save");
        assert_eq!(
            rel_strings(&report.removed),
            BTreeSet::from([
                "pages/page-2/page.fnx".to_owned(),
                "pages/page-2/page.ids.json".to_owned(),
                "pages/page-2/page.json".to_owned(),
            ]),
            "everything that WAS removed is still reported"
        );
        assert!(
            dir.path().join("pages/page-2").is_dir(),
            "the dir removal itself failed"
        );

        // With the obstacle gone, the next save finishes the prune.
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        assert!(!dir.path().join("pages/page-2").exists());
        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(persisted(&reread), persisted(&f.doc));
    }

    #[test]
    fn duplicate_and_unsluggable_page_names_get_stable_dirs() {
        let mut doc = Doc::new();
        doc.metadata.created_at = 1_700_000_000;
        doc.metadata.modified_at = 1_700_000_001;
        let mut add_page = |id_value: u128, name: &str| -> NodeId {
            let mut node = group(None, name);
            node.id = NodeId::from_u128(id_value);
            let id = node.id;
            doc.scene.insert(node).unwrap();
            doc.add_page(id);
            id
        };
        let low = add_page(1, "Home");
        let high = add_page(2, "Home");
        let symbols = add_page(3, "🎨🎨");
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

        // Collisions get -2, -3 suffixes in id order; unsluggable names use
        // the fallback.
        for slug in ["home", "home-2", "page"] {
            assert!(
                dir.path().join(format!("pages/{slug}/page.fnx")).is_file(),
                "missing pages/{slug}"
            );
        }
        let header_id = |slug: &str| -> String {
            let header: Value = serde_json::from_str(
                &fs::read_to_string(dir.path().join(format!("pages/{slug}/page.json"))).unwrap(),
            )
            .unwrap();
            header.get("id").unwrap().as_str().unwrap().to_owned()
        };
        assert_eq!(
            header_id("home"),
            low.to_string(),
            "lower id gets the bare slug"
        );
        assert_eq!(header_id("home-2"), high.to_string());
        assert_eq!(header_id("page"), symbols.to_string());

        let (doc2, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(persisted(&doc2), persisted(&doc));
        assert_eq!(doc2.pages(), doc.pages());
    }

    #[test]
    fn locate_sources_resolve_on_both_layouts() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        // v3 slug dirs: identity comes from the headers.
        assert_eq!(
            locate_page_source(dir.path(), f.page1),
            Some(dir.path().join("pages/page-1/page.fnx"))
        );
        assert_eq!(
            locate_master_source(dir.path(), f.comp_id),
            Some(dir.path().join("components/button/master.fnx"))
        );
        assert_eq!(locate_page_source(dir.path(), f.orphan), None);
        assert_eq!(locate_master_source(dir.path(), ComponentId::new()), None);

        // v2 id dirs: identity falls back to the directory name.
        downgrade_to_v2(dir.path());
        assert_eq!(
            locate_page_source(dir.path(), f.page1),
            Some(dir.path().join(format!("pages/{}/page.fnx", f.page1)))
        );
        assert_eq!(
            locate_master_source(dir.path(), f.comp_id),
            Some(
                dir.path()
                    .join(format!("components/{}/master.fnx", f.comp_id))
            )
        );
    }

    #[test]
    fn page_scope_of_source_resolves_on_both_layouts() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let root = dir.path().to_path_buf();
        assert_eq!(
            page_scope_of_source(&root.join("pages/page-1/page.fnx")),
            Some((root.clone(), ScopedDesign::Page(f.page1)))
        );
        assert_eq!(
            page_scope_of_source(&root.join("components/button/master.fnx")),
            Some((root.clone(), ScopedDesign::Component(f.comp_id)))
        );
        assert_eq!(page_scope_of_source(&root.join("fanta.json")), None);
        assert_eq!(
            page_scope_of_source(&root.join("pages/page-1/page.ids.json")),
            None,
            "only the .fnx source resolves"
        );

        downgrade_to_v2(dir.path());
        assert_eq!(
            page_scope_of_source(&root.join(format!("pages/{}/page.fnx", f.page1))),
            Some((root.clone(), ScopedDesign::Page(f.page1)))
        );
        assert_eq!(
            page_scope_of_source(&root.join(format!("components/{}/master.fnx", f.comp_id))),
            Some((root.clone(), ScopedDesign::Component(f.comp_id)))
        );
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

    /// `/`-joined relative path strings for report assertions.
    fn rel_strings(paths: &[std::path::PathBuf]) -> BTreeSet<String> {
        paths
            .iter()
            .map(|p| {
                p.components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect()
    }

    fn mtime(path: &Path) -> std::time::SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    /// A case-only rename of a design directory (hand/agent/Finder) on a
    /// case-insensitive filesystem used to alias the byte-diff while the
    /// exact-case prune deleted the live files — one successful save wiped
    /// the design. The healer renames the dir back before diffing; on a
    /// case-sensitive filesystem the odd-cased dir is genuinely stale and is
    /// pruned instead. Either way the design must survive.
    #[test]
    fn case_only_dir_rename_never_deletes_the_design() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        let slugged = dir.path().join("pages/page-1");
        assert!(slugged.join("page.fnx").is_file());

        // Two-step rename so the case change applies on both fs kinds.
        let via = dir.path().join("pages/case-tmp");
        fs::rename(&slugged, &via).unwrap();
        fs::rename(&via, dir.path().join("pages/Page-1")).unwrap();

        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        assert!(
            slugged.join("page.fnx").is_file(),
            "the design's source must exist at its projected path after healing"
        );
        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert_eq!(reread.pages().len(), f.doc.pages().len());
        assert!(
            reread.scene.get(f.page1).is_some(),
            "page survived the save"
        );
    }

    /// A symlink planted at a projected file path (e.g. from a cloned repo)
    /// must be replaced with a real file — never written THROUGH, which would
    /// clobber whatever the link points at outside the project.
    #[cfg(unix)]
    #[test]
    fn symlink_at_projected_file_is_replaced_not_written_through() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let outside = tempdir().unwrap();
        let target = outside.path().join("precious.txt");
        fs::write(&target, b"do not clobber").unwrap();
        let fnx = dir.path().join("pages/page-1/page.fnx");
        fs::remove_file(&fnx).unwrap();
        std::os::unix::fs::symlink(&target, &fnx).unwrap();

        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        assert_eq!(
            fs::read(&target).unwrap(),
            b"do not clobber",
            "the symlink target outside the project must be untouched"
        );
        assert!(
            !fs::symlink_metadata(&fnx).unwrap().file_type().is_symlink(),
            "the projected path holds a real file again"
        );
        read_project_tree(dir.path()).unwrap();
    }

    /// A design directory replaced by a symlink must be rebuilt as a real
    /// tree — the old behavior pruned the link (its files satisfied the
    /// byte-diff through it) and silently dropped the design.
    #[cfg(unix)]
    #[test]
    fn symlinked_design_dir_is_rebuilt_as_real_files() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let design = dir.path().join("pages/page-1");
        let external = tempdir().unwrap();
        let moved = external.path().join("page-1");
        fs::rename(&design, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &design).unwrap();

        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let metadata = fs::symlink_metadata(&design).unwrap();
        assert!(metadata.file_type().is_dir(), "real directory again");
        assert!(design.join("page.fnx").is_file());
        let (reread, _) = read_project_tree(dir.path()).unwrap();
        assert!(reread.scene.get(f.page1).is_some(), "design survived");
    }

    #[test]
    fn resave_without_changes_writes_and_removes_nothing() {
        let f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        let before = tree_snapshot(dir.path());
        let probe = dir.path().join("pages/page-1/page.fnx");
        let probe_mtime = mtime(&probe);

        let report = write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        assert!(report.written.is_empty(), "written: {:?}", report.written);
        assert!(report.removed.is_empty(), "removed: {:?}", report.removed);
        assert_eq!(tree_snapshot(dir.path()), before);
        assert_eq!(mtime(&probe), probe_mtime, "untouched file keeps its mtime");
    }

    #[test]
    fn editing_one_component_rewrites_only_its_files() {
        let mut f = fixture();
        let (assets, _) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        let untouched = [
            "pages/page-1/page.fnx",
            "pages/page-1/page.ids.json",
            "pages/page-1/page.json",
            "pages/page-2/page.fnx",
        ];
        let mtimes: Vec<_> = untouched
            .iter()
            .map(|rel| mtime(&dir.path().join(rel)))
            .collect();

        f.doc.scene.get_mut(f.comp_child).unwrap().name = "Button / Relabeled".to_owned();
        f.doc.metadata.modified_at = 1_700_000_002;
        let report = write_project_tree(dir.path(), &f.doc, &assets).unwrap();

        // The sidecar rewrites too: it fingerprints element names.
        assert_eq!(
            rel_strings(&report.written),
            BTreeSet::from([
                "components/button/master.fnx".to_owned(),
                "components/button/master.ids.json".to_owned(),
                "doc/metadata.json".to_owned(),
                "fanta.json".to_owned(),
            ])
        );
        assert!(report.removed.is_empty(), "removed: {:?}", report.removed);
        for (rel, before) in untouched.iter().zip(&mtimes) {
            assert_eq!(
                mtime(&dir.path().join(rel)),
                *before,
                "{rel} belongs to an untouched page and must keep its mtime"
            );
        }

        // The net disk state is byte-identical to a fresh full projection.
        let fresh = tempdir().unwrap();
        write_project_tree(fresh.path(), &f.doc, &assets).unwrap();
        assert_eq!(tree_snapshot(dir.path()), tree_snapshot(fresh.path()));
    }

    #[test]
    fn renaming_a_page_moves_its_directory_and_prunes_the_old() {
        let mut f = fixture();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
        assert!(dir.path().join("pages/page-2/page.fnx").is_file());

        f.doc.scene.get_mut(f.page2).unwrap().name = "Landing".to_owned();
        let report = write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();

        assert!(
            !dir.path().join("pages/page-2").exists(),
            "old slug directory must be pruned"
        );
        assert!(dir.path().join("pages/landing/page.fnx").is_file());
        assert_eq!(
            rel_strings(&report.written),
            BTreeSet::from([
                "pages/landing/page.fnx".to_owned(),
                "pages/landing/page.ids.json".to_owned(),
                "pages/landing/page.json".to_owned(),
            ])
        );
        assert_eq!(
            rel_strings(&report.removed),
            BTreeSet::from([
                "pages/page-2/page.fnx".to_owned(),
                "pages/page-2/page.ids.json".to_owned(),
                "pages/page-2/page.json".to_owned(),
            ])
        );

        let fresh = tempdir().unwrap();
        write_project_tree(fresh.path(), &f.doc, &BTreeMap::new()).unwrap();
        assert_eq!(tree_snapshot(dir.path()), tree_snapshot(fresh.path()));
    }

    #[test]
    fn assets_write_once_and_prune_when_removed() {
        let f = fixture();
        let (mut assets, expected) = fixture_assets();
        let dir = tempdir().unwrap();
        write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        let (png_id, png_family, png_ext) = expected[0];
        let png_path = dir
            .path()
            .join(format!("assets/{png_family}/{png_id}.{png_ext}"));
        let png_mtime = mtime(&png_path);

        // Content-addressed: an existing asset file is never rewritten.
        let report = write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        assert!(report.written.is_empty(), "written: {:?}", report.written);
        assert_eq!(mtime(&png_path), png_mtime);

        // Dropping an asset removes exactly its file, and its now-empty
        // family directory.
        let (gone_id, gone_family, gone_ext) = expected[1];
        assets.remove(&gone_id);
        let report = write_project_tree(dir.path(), &f.doc, &assets).unwrap();
        assert!(report.written.is_empty(), "written: {:?}", report.written);
        assert_eq!(
            rel_strings(&report.removed),
            BTreeSet::from([format!("assets/{gone_family}/{gone_id}.{gone_ext}")])
        );
        assert!(!dir.path().join(format!("assets/{gone_family}")).exists());

        let fresh = tempdir().unwrap();
        write_project_tree(fresh.path(), &f.doc, &assets).unwrap();
        assert_eq!(tree_snapshot(dir.path()), tree_snapshot(fresh.path()));
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

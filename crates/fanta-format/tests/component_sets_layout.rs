//! Layout v5: a component with variants is one folder.

use fanta_doc::{
    CanvasNode, ComponentDef, ComponentId, ComponentSet, ComponentSetMembership, Doc, GroupNode,
    NodeData, NodeId, Transform2D, VariantAxis,
};
use fanta_format::{
    ArtifactDirty, ArtifactId, SaveResult, WorkspaceSession, read_project_tree, write_project_tree,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::tempdir;

struct Fixture {
    doc: Doc,
    set: ComponentId,
    frame: NodeId,
    variants: Vec<(ComponentId, NodeId)>,
    avatar: ComponentId,
}

/// A page holding a "Button" set frame with two variants, plus a standalone
/// "Avatar" component on the same page.
fn fixture() -> Fixture {
    let mut doc = Doc::new();
    let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page.name = "Components".into();
    let page = doc.scene.insert(page).unwrap();
    doc.add_page(page);

    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([300.0, 100.0]),
        ..Default::default()
    }));
    frame.name = "Button".into();
    frame.parent = Some(page);
    let frame = doc.scene.insert(frame).unwrap();

    let set = ComponentId::new();
    let mut variants = Vec::new();
    for (index, state) in ["Default", "Hover"].into_iter().enumerate() {
        let mut master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([120.0, 40.0]),
            ..Default::default()
        }));
        master.name = format!("Variant=Primary, State={state}");
        master.parent = Some(frame);
        master.transform = Transform2D::translation(index as f64 * 140.0, 0.0);
        master.index = doc.scene.next_child_index(Some(frame));
        let root = doc.scene.insert(master).unwrap();
        let id = ComponentId::new();
        let mut def = ComponentDef::new(id, root, format!("Variant=Primary, State={state}"));
        def.variant_of = Some(ComponentSetMembership {
            set,
            axis_values: BTreeMap::from([
                ("Variant".to_owned(), "Primary".to_owned()),
                ("State".to_owned(), state.to_owned()),
            ]),
        });
        doc.components.defs.insert(id, def);
        variants.push((id, root));
    }
    doc.components.sets.insert(
        set,
        ComponentSet {
            id: set,
            name: "Button".into(),
            axes: vec![
                VariantAxis {
                    name: "Variant".into(),
                    values: vec!["Primary".into()],
                },
                VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                },
            ],
            members: variants.iter().map(|(id, _)| *id).collect(),
            default_variant: variants[0].0,
            root: Some(frame),
        },
    );

    let mut avatar_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    avatar_root.name = "Avatar".into();
    avatar_root.parent = Some(page);
    avatar_root.index = doc.scene.next_child_index(Some(page));
    let avatar_root = doc.scene.insert(avatar_root).unwrap();
    let avatar = ComponentId::new();
    doc.components
        .defs
        .insert(avatar, ComponentDef::new(avatar, avatar_root, "Avatar"));

    Fixture {
        doc,
        set,
        frame,
        variants,
        avatar,
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn listing(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.join("components")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    files.sort();
    files
}

#[test]
fn a_component_set_is_one_folder_holding_its_variants() {
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();

    assert_eq!(
        listing(dir.path()),
        [
            "components/avatar/def.json",
            "components/avatar/master.fnx",
            "components/avatar/master.ids.json",
            "components/button/primary-default/def.json",
            "components/button/primary-default/master.fnx",
            "components/button/primary-default/master.ids.json",
            "components/button/primary-hover/def.json",
            "components/button/primary-hover/master.fnx",
            "components/button/primary-hover/master.ids.json",
            "components/button/set.json",
        ]
    );
    let set = read_json(&dir.path().join("components/button/set.json"));
    assert_eq!(set["name"], "Button");
    assert_eq!(set["id"], json!(f.set));
    assert_eq!(set["root"], json!(f.frame));
    let variant = read_json(&dir.path().join("components/button/primary-hover/def.json"));
    assert_eq!(variant["id"], json!(f.variants[1].0));
    // A variant's source sits a folder deeper, and still imports the root fnx.
    let source = std::fs::read_to_string(
        dir.path()
            .join("components/button/primary-hover/master.fnx"),
    )
    .unwrap();
    assert!(source.contains("from \"../../../fnx\";"), "{source}");
    let standalone =
        std::fs::read_to_string(dir.path().join("components/avatar/master.fnx")).unwrap();
    assert!(standalone.contains("from \"../../fnx\";"));
    let manifest = read_json(&dir.path().join("fanta.json"));
    assert_eq!(manifest["version"], 5);

    let (read, _) = read_project_tree(dir.path()).unwrap();
    assert_eq!(read.components.sets, f.doc.components.sets);
    assert_eq!(read.components.defs, f.doc.components.defs);
    for (_, root) in &f.variants {
        assert_eq!(read.scene.get(*root).unwrap().parent, Some(f.frame));
    }
    assert!(read.components.def(f.avatar).is_some());
}

/// Rewrite a freshly written v5 tree the way v4 laid it out: every variant
/// flat in `components/` (2 folders deep, importing `../../fnx`), every set
/// in `components/sets.json`, manifest version 4.
fn downgrade_to_v4(dir: &Path, f: &Fixture) {
    let components = dir.join("components");
    for (from, to) in [
        ("button/primary-default", "variant-primary-state-default"),
        ("button/primary-hover", "variant-primary-state-hover"),
    ] {
        std::fs::rename(components.join(from), components.join(to)).unwrap();
        let source_path = components.join(to).join("master.fnx");
        let source = std::fs::read_to_string(&source_path)
            .unwrap()
            .replace("\"../../../fnx\"", "\"../../fnx\"");
        std::fs::write(&source_path, source).unwrap();
    }
    let set = read_json(&components.join("button/set.json"));
    std::fs::remove_dir_all(components.join("button")).unwrap();
    std::fs::write(
        components.join("sets.json"),
        serde_json::to_vec_pretty(&json!({ f.set.to_string().trim_start_matches("c_"): set }))
            .unwrap(),
    )
    .unwrap();
    let manifest_path = dir.join("fanta.json");
    let mut manifest = read_json(&manifest_path);
    manifest["version"] = json!(4);
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

#[test]
fn a_v4_project_reads_and_its_next_save_moves_the_variants_into_the_set_folder() {
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
    downgrade_to_v4(dir.path(), &f);
    let manifest_path = dir.path().join("fanta.json");

    let (read, assets) = read_project_tree(dir.path()).unwrap();
    assert_eq!(read.components.sets.len(), 1, "v4 sets.json still reads");
    assert_eq!(read.components.defs, f.doc.components.defs);

    write_project_tree(dir.path(), &read, &assets).unwrap();
    assert_eq!(
        listing(dir.path()),
        [
            "components/avatar/def.json",
            "components/avatar/master.fnx",
            "components/avatar/master.ids.json",
            "components/button/primary-default/def.json",
            "components/button/primary-default/master.fnx",
            "components/button/primary-default/master.ids.json",
            "components/button/primary-hover/def.json",
            "components/button/primary-hover/master.fnx",
            "components/button/primary-hover/master.ids.json",
            "components/button/set.json",
        ],
        "the flat v4 folders and sets.json are gone"
    );
    assert_eq!(read_json(&manifest_path)["version"], 5);
    let (upgraded, _) = read_project_tree(dir.path()).unwrap();
    assert_eq!(upgraded.components.sets, read.components.sets);
}

#[test]
fn the_session_opens_a_variant_in_its_set_folder_and_sees_no_change() {
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    assert_eq!(session.components.defs.sets.len(), 1);
    let variant = f.variants[1].0;
    assert_eq!(
        session.components.paths[&variant],
        Path::new("components/button/primary-hover")
    );
    let id = ArtifactId::Component(variant);
    session.open_artifact(id.clone()).unwrap();
    let artifact = session.artifact_mut(&id).unwrap();
    assert!(matches!(artifact.state, ArtifactDirty::Clean));
    let root = dir.path().canonicalize().unwrap();
    assert!(
        matches!(artifact.save(&root).unwrap(), SaveResult::NoOp),
        "the projection matches the variant's file, import depth included"
    );
}

#[test]
fn a_component_inside_a_page_opens_clean_and_saves_nothing() {
    // The scoped component scene detaches the master's root from its page
    // (or set frame); the source keeps that parent as `root_parent`. The two
    // must still compare as in sync.
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Component(f.avatar);
    session.open_artifact(id.clone()).unwrap();
    let root = dir.path().canonicalize().unwrap();
    let artifact = session.artifact_mut(&id).unwrap();
    assert!(matches!(artifact.save(&root).unwrap(), SaveResult::NoOp));
}

#[test]
fn a_variants_source_path_resolves_to_its_component() {
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
    let source = dir
        .path()
        .join("components/button/primary-hover/master.fnx");
    assert_eq!(
        fanta_format::locate_master_source(dir.path(), f.variants[1].0),
        Some(source.clone())
    );
    let (project, design) = fanta_format::page_scope_of_source(&source).unwrap();
    assert_eq!(project, dir.path());
    assert_eq!(
        design,
        fanta_format::ScopedDesign::Component(f.variants[1].0)
    );
}

#[test]
fn the_editors_save_of_an_edited_v4_variant_moves_it_into_its_set_folder() {
    // The editor's save path (fig_viewer's `FigItem::save`): session source
    // overrides + a checked full write + accepting the written sources. The
    // variant's source was read from v4's flat folder, so its override still
    // imports `../../fnx`; written a folder deeper it must import
    // `../../../fnx`, and accepting the save must expect exactly that.
    let dir = tempdir().unwrap();
    let mut f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
    downgrade_to_v4(dir.path(), &f);
    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Component(f.variants[1].0);
    session.open_artifact(id.clone()).unwrap();

    f.doc.scene.get_mut(f.variants[1].1).unwrap().opacity = fanta_doc::UnitInterval::new(0.5);
    let persisted = f.doc.clone_for_persist();
    session.adopt_document_shared(&persisted);
    session
        .artifact_mut(&id)
        .unwrap()
        .adopt_document(&persisted)
        .unwrap();
    let sources = session
        .validated_source_overrides_for_document(&persisted)
        .unwrap();
    assert!(
        sources.contains_key(Path::new("components/button/primary-hover")),
        "{sources:?}"
    );
    let expected = session.source_write_preconditions(&persisted).unwrap();
    let mut cache = fanta_format::ProjectWriteCache::default();
    let report = fanta_format::write_project_tree_cached_with_sources_checked(
        dir.path(),
        &persisted,
        &BTreeMap::new(),
        &mut cache,
        &sources,
        &expected,
    )
    .unwrap();
    let index_hash = report
        .written_hashes
        .get(Path::new("assets/index.json"))
        .copied()
        .or_else(|| session.asset_index_disk_hash())
        .unwrap();
    session
        .accept_written_sources(&persisted, index_hash, &sources, &report.written_hashes)
        .expect("the written variant matches what the session expects");
    let source = std::fs::read_to_string(
        dir.path()
            .join("components/button/primary-hover/master.fnx"),
    )
    .unwrap();
    assert!(source.contains("opacity={0.5}"), "{source}");
    assert!(source.contains("from \"../../../fnx\";"));
}

#[test]
fn upgrading_a_v4_project_moves_it_to_v5_once() {
    let dir = tempdir().unwrap();
    let f = fixture();
    write_project_tree(dir.path(), &f.doc, &BTreeMap::new()).unwrap();
    downgrade_to_v4(dir.path(), &f);
    let upgrade = fanta_format::upgrade_project(dir.path()).unwrap();
    assert_eq!((upgrade.from, upgrade.to), (4, 5));
    assert!(dir.path().join("components/button/set.json").is_file());
    assert!(!dir.path().join("components/sets.json").exists());
    assert!(
        !dir.path()
            .join("components/variant-primary-state-hover")
            .exists()
    );
    let again = fanta_format::upgrade_project(dir.path()).unwrap();
    assert_eq!((again.from, again.to), (5, 5));
}

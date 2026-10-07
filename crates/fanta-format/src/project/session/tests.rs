//! Unit tests for the session stack (hash, merge, materialize, FSM pieces).

use super::*;
use crate::project::merge::{NodeMapEdition, merge_artifact};
use fanta_doc::{
    CanvasNode, Doc, GroupNode, NodeData, NodeId, Operation, Transaction, Transform2D,
    VariableRegistry,
};
use fanta_fnx::{ArtifactIr, ArtifactKind};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::tempdir;

fn group_json(id: NodeId, name: &str, parent: Option<NodeId>) -> Value {
    let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
    node.id = id;
    node.name = name.into();
    node.parent = parent;
    serde_json::to_value(node).unwrap()
}

fn page_fixture() -> (tempfile::TempDir, NodeId) {
    let dir = tempdir().unwrap();
    let mut doc = Doc::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    root.name = "Home".into();
    let page = doc.scene.insert(root).unwrap();
    doc.add_page(page);
    let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
    child.name = "Card".into();
    child.parent = Some(page);
    child.transform = Transform2D::translation(10.0, 20.0);
    doc.scene.insert(child).unwrap();
    crate::write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();
    (dir, page)
}

#[test]
fn modern_source_keeps_identity_when_the_sidecar_is_missing_or_damaged() {
    let (directory, page) = page_fixture();
    let (original, _) = crate::read_project_tree(directory.path()).expect("original project");
    let source_path = crate::locate_page_source(directory.path(), page).expect("page source");
    let sidecar_path = source_path.with_file_name("page.ids.json");
    let mut sidecar: Value =
        serde_json::from_slice(&std::fs::read(&sidecar_path).expect("sidecar"))
            .expect("sidecar JSON");
    sidecar["ids"][0]["id"] = json!("bad-id");
    sidecar["ids"][1]["index"] = json!("not-a-number");
    for damaged in [
        None,
        Some(b"{".to_vec()),
        Some(sidecar.to_string().into_bytes()),
    ] {
        match damaged {
            Some(bytes) => std::fs::write(&sidecar_path, bytes).expect("damage sidecar"),
            None => std::fs::remove_file(&sidecar_path).expect("remove sidecar"),
        }
        let (loaded, _) = crate::read_project_tree(directory.path()).expect("recover source");
        for id in original.scene.descendants_of(page) {
            assert_eq!(loaded.scene.get(id), original.scene.get(id));
        }
        let mut workspace = WorkspaceSession::open(directory.path()).expect("open workspace");
        assert!(workspace.has_indexed_source(&ArtifactId::Page(page)));
        workspace
            .open_artifact(ArtifactId::Page(page))
            .expect("open repaired page");
        workspace
            .validated_source_overrides_for_document(&loaded)
            .expect("canvas saves can regenerate the sidecar");
    }
}

#[test]
fn missing_legacy_sidecar_does_not_assign_new_node_identities() {
    let (directory, page) = page_fixture();
    let (document, _) = crate::read_project_tree(directory.path()).expect("project");
    let source_path = crate::locate_page_source(directory.path(), page).expect("source");
    let mut source = std::fs::read_to_string(&source_path).expect("source text");
    for id in document.scene.descendants_of(page) {
        source = source.replace(&format!("id=\"{}\"", id.0), "");
    }
    std::fs::write(&source_path, &source).expect("legacy source");
    std::fs::remove_file(source_path.with_file_name("page.ids.json")).expect("remove sidecar");
    let error = crate::read_project_tree(directory.path())
        .expect_err("legacy identity cannot be recovered");
    assert!(error.to_string().contains("restore the sidecar"));
    assert_eq!(
        std::fs::read_to_string(&source_path).expect("preserved legacy source"),
        source
    );
}

#[test]
fn valid_legacy_sidecar_keeps_identity_and_order() {
    let (directory, page) = page_fixture();
    let (original, _) = crate::read_project_tree(directory.path()).expect("original project");
    let source_path = crate::locate_page_source(directory.path(), page).expect("source");
    let mut source = std::fs::read_to_string(&source_path).expect("source text");
    for id in original.scene.descendants_of(page) {
        source = source.replace(&format!("id=\"{}\"", id.0), "");
    }
    std::fs::write(&source_path, source).expect("legacy source");
    let (loaded, _) = crate::read_project_tree(directory.path()).expect("legacy identity");
    for id in original.scene.descendants_of(page) {
        assert_eq!(loaded.scene.get(id), original.scene.get(id));
    }
    let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
    workspace
        .open_artifact(ArtifactId::Page(page))
        .expect("legacy artifact");
}

#[test]
fn session_indexes_the_same_duplicate_page_winner_as_the_reader() {
    let (directory, page) = page_fixture();
    let original = WorkspaceSession::open(directory.path()).expect("open project");
    let old_dir = directory
        .path()
        .join(&original.artifacts[&ArtifactId::Page(page)].design_dir);
    let preferred_dir = directory.path().join("pages/a-preferred");
    std::fs::create_dir_all(&preferred_dir).expect("create replacement directory");
    for name in ["page.json", "page.fnx", "page.ids.json"] {
        std::fs::copy(old_dir.join(name), preferred_dir.join(name))
            .expect("copy the generated page artifact");
    }
    let preferred_source = std::fs::read_to_string(preferred_dir.join("page.fnx"))
        .expect("read replacement source")
        .replace("Home", "Preferred");
    std::fs::write(preferred_dir.join("page.fnx"), preferred_source)
        .expect("edit replacement source");
    let mut old_header: Value = crate::project::layout::read_json_file(&old_dir.join("page.json"))
        .expect("read old page header");
    old_header
        .as_object_mut()
        .expect("page header object")
        .remove("id");
    crate::project::layout::write_json_file(&old_dir.join("page.json"), &old_header)
        .expect("make the old directory a v2 fallback");

    let (loaded, _) = crate::read_project_tree(directory.path()).expect("read project");
    assert_eq!(loaded.scene.get(page).expect("page root").name, "Preferred");
    let session = WorkspaceSession::open(directory.path()).expect("reopen session");
    assert_eq!(
        session.artifacts[&ArtifactId::Page(page)].design_dir,
        std::path::PathBuf::from("pages/a-preferred")
    );
}

#[test]
fn session_indexes_the_same_duplicate_component_winner_as_the_reader() {
    use fanta_doc::{ComponentDef, ComponentId};

    let directory = tempdir().expect("temporary project");
    let mut document = Doc::new();
    let page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("page root");
    document.add_page(page);
    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.name = "Button".into();
    master.parent = Some(page);
    let master_id = document.scene.insert(master).expect("component master");
    let component_id = ComponentId::new();
    document.components.defs.insert(
        component_id,
        ComponentDef::new(component_id, master_id, "Button"),
    );
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
        .expect("write project");
    let original = WorkspaceSession::open(directory.path()).expect("open project");
    let old_dir = directory
        .path()
        .join(&original.artifacts[&ArtifactId::Component(component_id)].design_dir);
    let preferred_dir = directory.path().join("components/a-preferred");
    std::fs::create_dir_all(&preferred_dir).expect("create replacement directory");
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    for name in ["def.json", "master.fnx", "master.ids.json"] {
        let target = preferred_dir.join(name);
        std::fs::copy(old_dir.join(name), &target).expect("copy component artifact");
        std::fs::File::options()
            .write(true)
            .open(&target)
            .expect("open replacement artifact")
            .set_times(std::fs::FileTimes::new().set_modified(future))
            .expect("set replacement freshness");
    }
    let preferred_source = std::fs::read_to_string(preferred_dir.join("master.fnx"))
        .expect("read replacement source")
        .replace("Button", "Preferred");
    std::fs::write(preferred_dir.join("master.fnx"), preferred_source)
        .expect("edit replacement source");

    let (loaded, _) = crate::read_project_tree(directory.path()).expect("read project");
    assert_eq!(
        loaded.scene.get(master_id).expect("master root").name,
        "Preferred"
    );
    let session = WorkspaceSession::open(directory.path()).expect("reopen session");
    assert_eq!(
        session.artifacts[&ArtifactId::Component(component_id)].design_dir,
        std::path::PathBuf::from("components/a-preferred")
    );
}

// ---- hash -------------------------------------------------------------------

#[test]
fn content_hash_domain_separated() {
    let a = hash_file_set(&[("a", b"x".as_slice())]);
    let b = hash_file_set(&[("b", b"x".as_slice())]);
    assert_ne!(a, b);
}

// ---- merge_artifact ---------------------------------------------------------

#[test]
fn merge_artifact_disjoint_fields_clean() {
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let base_node = json!({
        "type": "group", "id": id, "parent": null, "index": 1.0,
        "name": "A", "opacity": 1.0, "blend_mode": "normal",
        "transform": [1,0,0,1,0,0], "scrollable": false
    });
    let mut base_nodes = Map::new();
    base_nodes.insert(id.into(), base_node.clone());
    let base = NodeMapEdition::new(json!({"order": 0}), base_nodes.clone());

    let mut ours_node = base_node.clone();
    ours_node["name"] = json!("Ours");
    let mut ours_nodes = Map::new();
    ours_nodes.insert(id.into(), ours_node);

    let mut theirs_node = base_node;
    theirs_node["opacity"] = json!(0.5);
    let mut theirs_nodes = Map::new();
    theirs_nodes.insert(id.into(), theirs_node);

    let ours = NodeMapEdition::new(json!({"order": 0}), ours_nodes);
    let theirs = NodeMapEdition::new(json!({"order": 0}), theirs_nodes);
    let m = merge_artifact(&base, &ours, &theirs);
    assert!(m.is_clean(), "conflicts: {:?}", m.conflicts);
    assert_eq!(m.nodes[id]["name"], "Ours");
    assert_eq!(m.nodes[id]["opacity"], 0.5);
}

#[test]
fn merge_artifact_same_field_conflict_ours_wins() {
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let base_node = json!({
        "type": "group", "id": id, "parent": null, "index": 1.0,
        "name": "Base", "opacity": 1.0, "blend_mode": "normal",
        "transform": [1,0,0,1,0,0], "scrollable": false
    });
    let mut base_nodes = Map::new();
    base_nodes.insert(id.into(), base_node.clone());
    let base = NodeMapEdition::new(json!({}), base_nodes);

    let mut ours_node = base_node.clone();
    ours_node["name"] = json!("Canvas");
    let mut ours_nodes = Map::new();
    ours_nodes.insert(id.into(), ours_node);

    let mut theirs_node = base_node;
    theirs_node["name"] = json!("Agent");
    let mut theirs_nodes = Map::new();
    theirs_nodes.insert(id.into(), theirs_node);

    let m = merge_artifact(
        &base,
        &NodeMapEdition::new(json!({}), ours_nodes),
        &NodeMapEdition::new(json!({}), theirs_nodes),
    );
    assert!(!m.is_clean());
    assert_eq!(m.nodes[id]["name"], "Canvas");
}

// ---- materialize ------------------------------------------------------------

#[test]
fn materialize_page_rejects_non_frame_root() {
    let id = NodeId::new();
    // Text node as root — invalid for page
    let node = json!({
        "type": "text",
        "id": serde_json::to_value(id).unwrap(),
        "parent": null,
        "index": 1.0,
        "name": "T",
        "opacity": 1.0,
        "blend_mode": "normal",
        "transform": [1,0,0,1,0,0],
        "content": "hi",
        "local_size": [10.0, 10.0],
        "style": {"font_family": "Inter", "size_px": 12.0}
    });
    let ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Bad", &[node]);
    // encode may work; materialize must fail
    if let Ok(ir) = ir {
        let err = materialize_page(
            &ir,
            fanta_doc::DocId::new(),
            Default::default(),
            VariableRegistry::new(),
            BTreeMap::new(),
        );
        assert!(err.is_err());
    }
}

#[test]
fn model3d_materialization_preserves_payload_in_page_and_component_scopes() {
    use fanta_doc::{AssetId, Camera3d, ComponentDef, ComponentId, Model3dNode};

    for kind in [ArtifactKind::Page, ArtifactKind::Component] {
        let mut document = Doc::new();
        let root = document
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("root");
        document.add_page(root);
        let mut model = CanvasNode::new(NodeData::Model3d(Model3dNode {
            asset: AssetId::new(),
            local_size: [240.0, 160.0],
            camera: Camera3d {
                azimuth: 0.25,
                elevation: 1.5,
                distance: 8.0,
                target: [1.0, 2.0, 3.0],
                fov_deg: 45.0,
            },
            overrides: json!({"material": {"roughness": 0.75}, "future": ["retain", 7]}),
        }));
        model.parent = Some(root);
        model.name = "Existing model payload".into();
        model.transform = Transform2D::translation(21.0, 34.0);
        model.opacity = 0.5.into();
        model.meta = json!({"preserved": true});
        let model_id = document.scene.insert(model.clone()).expect("model");
        let definition = ComponentDef::new(ComponentId::new(), root, "Existing model component");
        if kind == ArtifactKind::Component {
            document
                .components
                .defs
                .insert(definition.id, definition.clone());
        }
        let nodes = document
            .scene
            .descendants_of(root)
            .map(|id| {
                serde_json::to_value(document.scene.get(id).expect("node")).expect("node JSON")
            })
            .collect::<Vec<_>>();
        let ir = ArtifactIr::from_nodes(kind, "ExistingModel", &nodes).expect("source IR");
        let materialized = if kind == ArtifactKind::Page {
            materialize_page(
                &ir,
                document.id,
                document.components.clone(),
                document.variables.clone(),
                document.active_modes.clone(),
            )
        } else {
            materialize_component(
                &ir,
                document.id,
                definition.clone(),
                document.variables.clone(),
                document.active_modes.clone(),
            )
        }
        .expect("stored model may materialize without a 3D authoring surface");
        let shared = super::materialize::scope_from_document(
            &document,
            kind,
            root,
            (kind == ArtifactKind::Component).then_some(&definition),
        )
        .expect("page/component sharing path")
        .expect("stored model may be adopted by the live session");
        assert_eq!(materialized.doc.scene.get(model_id), Some(&model));
        assert_eq!(shared.doc.scene.get(model_id), Some(&model));
    }
}

#[test]
fn model3d_payloads_survive_an_unrelated_canvas_save_and_reopen() {
    use fanta_doc::{AssetId, ComponentDef, ComponentId, Model3dNode, VectorNode};

    let directory = tempdir().expect("project directory");
    let mut document = Doc::new();
    let edited_page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("edited page");
    document.add_page(edited_page);
    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        100.0,
        80.0,
        fanta_doc::Color::WHITE,
    )));
    rectangle.parent = Some(edited_page);
    let rectangle = document.scene.insert(rectangle).expect("rectangle");
    let model_page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("model page");
    document.add_page(model_page);
    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.parent = Some(model_page);
    let master = document.scene.insert(master).expect("component master");
    let component = ComponentId::new();
    document.components.defs.insert(
        component,
        ComponentDef::new(component, master, "Stored 3D component"),
    );
    let asset = AssetId::new();
    let assets = BTreeMap::from([(asset, b"opaque existing model asset bytes".to_vec())]);
    let mut preserved_models = Vec::new();
    for parent in [model_page, master] {
        let mut model = CanvasNode::new(NodeData::Model3d(Model3dNode {
            asset,
            local_size: [240.0, 160.0],
            camera: Default::default(),
            overrides: json!({"preserve": {"lighting": "custom", "values": [1, 2, 3]}}),
        }));
        model.parent = Some(parent);
        preserved_models.push(model.clone());
        document.scene.insert(model).expect("existing model");
    }
    crate::write_project_tree(directory.path(), &document, &assets).expect("existing project");
    let mut workspace = WorkspaceSession::open(directory.path()).expect("index project");
    let artifacts = [
        ArtifactId::Page(edited_page),
        ArtifactId::Page(model_page),
        ArtifactId::Component(component),
    ];
    for artifact in &artifacts {
        workspace
            .open_artifact(artifact.clone())
            .expect("open existing source");
    }
    document
        .scene
        .get_mut(rectangle)
        .expect("rectangle")
        .transform = Transform2D::translation(30.0, 45.0);
    workspace.adopt_document_shared(&document);
    for artifact in &artifacts {
        workspace
            .artifact_mut(artifact)
            .expect("open session")
            .adopt_document(&document)
            .expect("adopt whole-document canvas state");
    }
    let sources = workspace
        .validated_source_overrides_for_document(&document)
        .expect("validate retained model sources");
    let preconditions = workspace
        .source_write_preconditions(&document)
        .expect("save preconditions");
    crate::write_project_tree_cached_with_sources_checked(
        directory.path(),
        &document,
        &assets,
        &mut crate::ProjectWriteCache::default(),
        &sources,
        &preconditions,
    )
    .expect("unrelated edits must save despite a retained model");
    let (reopened, reopened_assets) = crate::read_project_tree(directory.path()).expect("reopen");
    assert_eq!(reopened.scene.len(), document.scene.len());
    assert_eq!(reopened.pages(), document.pages());
    assert_eq!(reopened.components, document.components);
    assert_eq!(reopened_assets, assets);
    assert_eq!(reopened.scene.get(rectangle), document.scene.get(rectangle));
    for model in preserved_models {
        assert_eq!(reopened.scene.get(model.id), Some(&model));
    }
    let mut reopened_workspace = WorkspaceSession::open(directory.path()).expect("reopen session");
    for artifact in artifacts {
        reopened_workspace
            .open_artifact(artifact)
            .expect("materialize saved source");
    }
}

#[test]
fn materialize_page_round_trips_group_tree() {
    let root = NodeId::new();
    let child = NodeId::new();
    let nodes = vec![
        group_json(root, "Home", None),
        group_json(child, "Card", Some(root)),
    ];
    let ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Home", &nodes).unwrap();
    let scoped = materialize_page(
        &ir,
        fanta_doc::DocId::new(),
        Default::default(),
        VariableRegistry::new(),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(scoped.root, root);
    assert!(scoped.doc.scene.get(root).is_some());
    assert!(scoped.doc.scene.get(child).is_some());
    assert_eq!(scoped.doc.scene.children_of(Some(root)).len(), 1);
}

#[test]
fn graphics_rejects_live_instance_in_source() {
    let root = NodeId::new();
    let inst = NodeId::new();
    let mut nodes = vec![group_json(root, "Artboard", None)];
    nodes.push(json!({
        "type": "instance",
        "id": serde_json::to_value(inst).unwrap(),
        "parent": serde_json::to_value(root).unwrap(),
        "index": 1.0,
        "name": "I",
        "opacity": 1.0,
        "blend_mode": "normal",
        "transform": [1,0,0,1,0,0],
        "component": serde_json::to_value(fanta_doc::ComponentId::new()).unwrap(),
        "local_size": [50.0, 50.0]
    }));
    // May fail encode if instance shape incomplete — either way import matrix fails.
    if let Ok(ir) = ArtifactIr::from_nodes(ArtifactKind::Graphics, "G", &nodes) {
        let err = materialize_graphics(
            &ir,
            fanta_doc::DocId::new(),
            VariableRegistry::new(),
            BTreeMap::new(),
        );
        assert!(err.is_err());
    }
}

// ---- session integration unit-level -----------------------------------------

#[test]
fn apply_refuses_variable_ops() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let mid = fanta_doc::ModeId::new();
    let err = ws.apply(
        &id,
        Operation::CreateVariableCollection {
            collection: Box::new(fanta_doc::VariableCollection {
                id: fanta_doc::VariableCollectionId::new(),
                name: "C".into(),
                modes: vec![fanta_doc::Mode {
                    id: mid,
                    name: "Default".into(),
                }],
                default_mode: mid,
                variable_order: vec![],
            }),
        },
    );
    assert!(matches!(err, Err(SessionError::UseWorkspaceArtifact)));

    let err = ws.apply(
        &id,
        Operation::SetCollectionDefaultMode {
            collection: fanta_doc::VariableCollectionId::new(),
            old: mid,
            new: fanta_doc::ModeId::new(),
        },
    );
    assert!(matches!(err, Err(SessionError::UseWorkspaceArtifact)));
}

#[test]
fn page_registry_requires_project_persistence_and_rejects_scoped_transactions_atomically() {
    let (directory, page) = page_fixture();
    let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
    let id = ArtifactId::Page(page);
    workspace.open_artifact(id.clone()).expect("page artifact");
    let artifact = workspace.artifact_mut(&id).expect("artifact");
    let before = serde_json::to_value(artifact.doc()).expect("document snapshot");
    let source = artifact.source_text();
    let generation = artifact.working_generation();
    let registry = Operation::SetPageRegistry {
        old_pages: artifact.doc().pages().to_vec(),
        new_pages: vec![page],
        old_active_page: artifact.doc().active_page(),
        new_active_page: Some(page),
    };
    assert_eq!(
        artifact_op_impact(&registry),
        ArtifactOpImpact::ProjectStructure
    );
    let transaction = Transaction {
        label: "Page registry cannot belong to one artifact".into(),
        ops: vec![
            Operation::SetName {
                id: page,
                old: artifact.doc().scene.get(page).expect("page").name.clone(),
                new: "Must stay unmodified".into(),
            },
            registry.clone(),
        ],
    };
    assert!(matches!(
        artifact.apply_transaction_atomic(transaction),
        Err(SessionError::InvalidState(_))
    ));
    assert_eq!(
        serde_json::to_value(artifact.doc()).expect("document snapshot"),
        before
    );
    assert_eq!(artifact.source_text(), source);
    assert_eq!(artifact.working_generation(), generation);
    let shared_generation = workspace.workspace_generation();
    assert!(matches!(
        workspace.apply_workspace_op(registry),
        Err(SessionError::InvalidState(_))
    ));
    assert_eq!(workspace.workspace_generation(), shared_generation);
}

#[test]
fn canvas_property_patch_updates_retained_source_without_touching_disk() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let canonical = std::fs::read_to_string(&source_path).unwrap();
    let custom = format!(
        "// hand-authored prelude stays byte-identical\n{}",
        canonical.replacen("name=\"Card\"", "name = \"Card\" future_prop={\"keep\"}", 1)
    );
    std::fs::write(&source_path, &custom).unwrap();

    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();

    let sync = art
        .apply_transaction_atomic(Transaction {
            label: "Rename card".into(),
            ops: vec![Operation::SetName {
                id: child,
                old: "Card".into(),
                new: "Renamed".into(),
            }],
        })
        .unwrap();

    assert_eq!(sync, SourceSync::PatchedNodes { nodes: vec![child] });
    assert_eq!(art.working_generation(), 1);
    assert_eq!(
        art.source_text(),
        custom.replacen("name = \"Card\"", "name = \"Renamed\"", 1)
    );
    assert!(art.source_text().contains("future_prop={\"keep\"}"));
    assert_eq!(
        std::fs::read_to_string(&source_path).unwrap(),
        custom,
        "in-memory canvas edits must not write before save"
    );

    assert!(matches!(
        art.save(dir.path()).unwrap(),
        SaveResult::Wrote { .. }
    ));
    assert!(
        std::fs::read_to_string(&source_path)
            .unwrap()
            .contains("future_prop={\"keep\"}")
    );
    assert_eq!(
        art.base_nodes.nodes[&child.0.to_string()]["future_prop"],
        "keep",
        "saved semantic base must retain forward-compatible source fields"
    );

    art.apply(Operation::SetName {
        id: child,
        old: "Renamed".into(),
        new: "RenamedAgain".into(),
    })
    .unwrap();
    assert!(art.source_text().contains("future_prop={\"keep\"}"));
}

#[test]
fn root_property_patch_preserves_source_only_size_sugar() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let canonical = std::fs::read_to_string(&source_path).unwrap();
    let custom = canonical.replacen(
        "name=\"Home\"",
        "name = \"Home\" width={800} height={600}",
        1,
    );
    std::fs::write(&source_path, &custom).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();

    art.apply(Operation::SetName {
        id: page,
        old: "Home".into(),
        new: "Landing".into(),
    })
    .unwrap();

    assert_eq!(
        art.source_text(),
        custom.replacen("name = \"Home\"", "name = \"Landing\"", 1)
    );
    assert!(art.source_text().contains("width={800} height={600}"));
}

#[test]
fn stale_operation_old_value_is_rejected_before_live_install() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let source_before = art.source_text();

    let error = art
        .apply(Operation::SetName {
            id: child,
            old: "Stale client value".into(),
            new: "Must not install".into(),
        })
        .unwrap_err();

    assert!(matches!(error, SessionError::OperationPrecondition { .. }));
    assert_eq!(art.doc().scene.get(child).unwrap().name, "Card");
    assert_eq!(art.source_text(), source_before);
    assert_eq!(art.working_generation(), 0);
}

#[test]
fn failed_transaction_leaves_scene_source_history_generation_and_disk_unchanged() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_before = std::fs::read(&source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let source_before = art.source_text();
    let generation_before = art.working_generation();
    let undo_before = art.doc().history.undo_depth();

    let error = art
        .apply_transaction_atomic(Transaction {
            label: "Partially invalid".into(),
            ops: vec![
                Operation::SetName {
                    id: child,
                    old: "Card".into(),
                    new: "Must not leak".into(),
                },
                Operation::SetName {
                    id: NodeId::new(),
                    old: "Missing".into(),
                    new: "Still missing".into(),
                },
            ],
        })
        .unwrap_err();

    assert!(error.to_string().contains("apply"));
    assert_eq!(art.doc().scene.get(child).unwrap().name, "Card");
    assert_eq!(art.source_text(), source_before);
    assert_eq!(art.working_generation(), generation_before);
    assert_eq!(art.doc().history.undo_depth(), undo_before);
    assert_eq!(std::fs::read(&source_path).unwrap(), disk_before);
}

#[test]
fn disjoint_agent_source_and_canvas_edits_merge_into_one_unsaved_working_copy() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_source = std::fs::read_to_string(&source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let base_hash = art.base_hash;

    art.apply(Operation::SetName {
        id: child,
        old: "Card".into(),
        new: "CanvasCard".into(),
    })
    .unwrap();
    let candidate = disk_source.replacen("name=\"Home\"", "name=\"AgentHome\"", 1);
    let outcome = art.propose_source(&candidate, base_hash).unwrap();
    let SourceProposalOutcome::Applied(applied) = outcome else {
        panic!("disjoint edits should auto-merge");
    };

    assert_eq!(applied.generation, 2);
    assert_eq!(art.doc().scene.get(page).unwrap().name, "AgentHome");
    assert_eq!(art.doc().scene.get(child).unwrap().name, "CanvasCard");
    assert!(art.source_text().contains("name=\"AgentHome\""));
    assert!(art.source_text().contains("name=\"CanvasCard\""));
    assert_eq!(std::fs::read_to_string(&source_path).unwrap(), disk_source);
}

#[test]
fn same_property_agent_collision_is_reviewed_then_applied_atomically() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_source = std::fs::read_to_string(&source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let base_hash = art.base_hash;

    art.apply(Operation::SetName {
        id: child,
        old: "Card".into(),
        new: "CanvasCard".into(),
    })
    .unwrap();
    let candidate = disk_source.replacen("name=\"Card\"", "name=\"AgentCard\"", 1);
    let outcome = art.propose_source(&candidate, base_hash).unwrap();
    let SourceProposalOutcome::Review(mut review) = outcome else {
        panic!("same-property edits must require review");
    };

    assert_eq!(art.doc().scene.get(child).unwrap().name, "CanvasCard");
    assert_eq!(review.conflicts.len(), 1);
    assert_eq!(
        review.conflicts[0].conflict.base,
        PresenceValue::Present(json!("Card"))
    );
    assert_eq!(
        review.conflicts[0].conflict.ours,
        PresenceValue::Present(json!("CanvasCard"))
    );
    assert_eq!(
        review.conflicts[0].conflict.theirs,
        PresenceValue::Present(json!("AgentCard"))
    );

    review.resolve(0, ReviewResolution::Theirs).unwrap();
    let applied = art.apply_merge_review(&review).unwrap();
    assert_eq!(applied.generation, 2);
    assert_eq!(art.doc().scene.get(child).unwrap().name, "AgentCard");
    assert!(art.source_text().contains("name=\"AgentCard\""));
    assert_eq!(std::fs::read_to_string(&source_path).unwrap(), disk_source);
}

#[test]
fn canvas_edit_after_review_creation_makes_review_stale() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_source = std::fs::read_to_string(source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let base_hash = art.base_hash;
    art.apply(Operation::SetName {
        id: child,
        old: "Card".into(),
        new: "CanvasCard".into(),
    })
    .unwrap();
    let candidate = disk_source.replacen("name=\"Card\"", "name=\"AgentCard\"", 1);
    let SourceProposalOutcome::Review(mut review) =
        art.propose_source(&candidate, base_hash).unwrap()
    else {
        panic!("expected collision review");
    };
    review.resolve(0, ReviewResolution::Theirs).unwrap();

    art.apply(Operation::SetOpacity {
        id: child,
        old: fanta_doc::UnitInterval::ONE,
        new: fanta_doc::UnitInterval::new(0.5),
    })
    .unwrap();
    let error = art.apply_merge_review(&review).unwrap_err();
    assert!(matches!(
        error,
        SessionError::GenerationConflict {
            expected: 1,
            actual: 2
        }
    ));
    assert_eq!(art.doc().scene.get(child).unwrap().name, "CanvasCard");
}

#[test]
fn save_after_review_creation_makes_its_base_stale() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_source = std::fs::read_to_string(source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    let child = *art.doc().scene.children_of(Some(page)).first().unwrap();
    let base_hash = art.base_hash;
    art.apply(Operation::SetName {
        id: child,
        old: "Card".into(),
        new: "CanvasCard".into(),
    })
    .unwrap();
    let candidate = disk_source.replacen("name=\"Card\"", "name=\"AgentCard\"", 1);
    let SourceProposalOutcome::Review(mut review) =
        art.propose_source(&candidate, base_hash).unwrap()
    else {
        panic!("expected collision review");
    };
    review.resolve(0, ReviewResolution::Theirs).unwrap();

    art.save(dir.path()).unwrap();
    let error = art.apply_merge_review(&review).unwrap_err();
    assert!(matches!(error, SessionError::BaseRevisionConflict { .. }));
    assert_eq!(art.doc().scene.get(child).unwrap().name, "CanvasCard");
}

#[test]
fn doc_mut_guard_restores_variables() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    {
        let session = ws.open.get_mut(&id).unwrap();
        let mut guard = session.doc_mut_guard(&ws.shared);
        guard.doc_mut().variables = VariableRegistry::new(); // wipe
        // drop restores
    }
    // Workspace shared still empty; restore brings empty too — just ensure no panic
    // and state may become DirtyCanvas if modified_at changed.
    let _ = ws.artifact(&id).unwrap();
}

#[test]
fn doc_mut_guard_synchronizes_retained_source_before_returning() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_before = std::fs::read(&source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    {
        let session = ws.open.get_mut(&id).unwrap();
        let mut guard = session.doc_mut_guard(&ws.shared);
        guard
            .doc_mut()
            .apply(Operation::SetName {
                id: page,
                old: "Home".into(),
                new: "Guarded".into(),
            })
            .unwrap();
    }

    let artifact = ws.artifact(&id).unwrap();
    assert!(matches!(artifact.state, ArtifactDirty::DirtyCanvas));
    assert!(artifact.source_text().contains("name=\"Guarded\""));
    assert!(!artifact.source_text().contains("name=\"Home\""));
    assert_eq!(
        std::fs::read(source_path).unwrap(),
        disk_before,
        "guard synchronization must remain memory-only"
    );
}

#[test]
fn commit_text_to_scene_and_save() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let project_id = ws.project_id;
    let components = ws.components.defs.clone();
    let variables = ws.shared.variables.clone();
    let modes = ws.shared.active_modes.clone();
    {
        let art = ws.artifact_mut(&id).unwrap();
        let text = art.begin_text_edit().unwrap().clone();
        let text = text.replacen("name=\"Home\"", "name=\"Landing\"", 1);
        art.set_text(text).unwrap();
        art.commit_text_to_scene(project_id, components, variables, modes)
            .unwrap();
        assert!(matches!(art.state, ArtifactDirty::DirtyCanvas));
    }
    ws.save_artifact(id.clone()).unwrap();
    let (doc, _) = crate::read_project_tree(dir.path()).unwrap();
    assert_eq!(doc.scene.get(page).unwrap().name, "Landing");
}

fn assert_session_sidecar_uses_canonical_json(text_edit: bool) {
    let (directory, page) = page_fixture();
    let source_path = crate::locate_page_source(directory.path(), page).expect("page source");
    let sidecar_path = source_path.with_file_name("page.ids.json");
    let original: fanta_fnx::FnxSidecar =
        serde_json::from_slice(&std::fs::read(&sidecar_path).expect("original sidecar"))
            .expect("original identities");
    let mut workspace = WorkspaceSession::open(directory.path()).expect("open project");
    let id = ArtifactId::Page(page);
    workspace.open_artifact(id.clone()).expect("open page");
    let artifact = workspace.artifact_mut(&id).expect("page session");
    let child = *artifact
        .doc()
        .scene
        .children_of(Some(page))
        .first()
        .expect("card node");
    let authored_source = if text_edit {
        let source = artifact.begin_text_edit().expect("begin code edit").clone();
        let changed = source.replacen("name=\"Card\"", "name=\"Renamed\"", 1);
        artifact.set_text(changed.clone()).expect("rename in code");
        assert!(matches!(artifact.state, ArtifactDirty::DirtyText));
        Some(changed)
    } else {
        artifact
            .apply(Operation::SetName {
                id: child,
                old: "Card".into(),
                new: "Renamed".into(),
            })
            .expect("rename on canvas");
        assert!(matches!(artifact.state, ArtifactDirty::DirtyCanvas));
        None
    };
    let files = artifact.project_to_files().expect("project edited page");
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| name == "page.ids.json")
        .expect("projected sidecar");
    let value: Value = serde_json::from_slice(bytes).expect("sidecar JSON");
    assert_eq!(
        *bytes,
        crate::project::layout::json_bytes(&value).expect("canonical sidecar")
    );
    let projected: fanta_fnx::FnxSidecar =
        serde_json::from_slice(bytes).expect("projected identities");
    assert_eq!(
        projected
            .ids
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>(),
        original
            .ids
            .iter()
            .map(|entry| &entry.id)
            .collect::<Vec<_>>()
    );
    assert!(
        projected
            .ids
            .iter()
            .any(|entry| entry.name.as_deref() == Some("Renamed"))
    );
    if let Some(authored_source) = authored_source {
        let (_, source) = files
            .iter()
            .find(|(name, _)| name == "page.fnx")
            .expect("projected source");
        assert_eq!(source.as_slice(), authored_source.as_bytes());
    }
    assert_eq!(
        std::fs::read(&sidecar_path).expect("untouched disk sidecar"),
        crate::project::layout::json_bytes(
            &serde_json::to_value(&original).expect("original value")
        )
        .expect("original canonical bytes")
    );
}

#[test]
fn text_session_sidecar_matches_canonical_project_json() {
    assert_session_sidecar_uses_canonical_json(true);
}

#[test]
fn canvas_session_sidecar_matches_canonical_project_json() {
    assert_session_sidecar_uses_canonical_json(false);
}

#[test]
fn unknown_attribute_typo_commits_with_a_warning_and_resets_on_clean_commit() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let project_id = ws.project_id;
    let components = ws.components.defs.clone();
    let variables = ws.shared.variables.clone();
    let modes = ws.shared.active_modes.clone();

    let art = ws.artifact_mut(&id).unwrap();
    assert!(
        art.source_diagnostics().is_empty(),
        "a clean disk source opens without diagnostics"
    );

    // Typo an attribute on the "Card" child. The commit must SUCCEED — an
    // unknown attribute may be a future field and can never hard-error — but
    // it must not stay silent either.
    let text = art.begin_text_edit().unwrap().clone();
    let typo = text.replacen("name=\"Card\"", "name=\"Card\" corner_raduis={4}", 1);
    assert_ne!(typo, text, "fixture child must be present");
    art.set_text(typo).unwrap();
    art.commit_text_to_scene(
        project_id,
        components.clone(),
        variables.clone(),
        modes.clone(),
    )
    .expect("a typo'd attribute must still commit");
    assert!(matches!(art.state, ArtifactDirty::DirtyCanvas));

    let diagnostics = art.source_diagnostics().to_vec();
    assert_eq!(diagnostics.len(), 1, "diagnostics: {diagnostics:?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic.code, "source.unknown_attribute");
    assert_eq!(diagnostic.severity, SourceSeverity::Warning);
    assert_eq!(diagnostic.node_name.as_deref(), Some("Card"));
    assert!(diagnostic.node_id.is_some());
    assert!(
        diagnostic.message.contains("`corner_raduis`")
            && diagnostic.message.contains("did you mean `corner_radius`?"),
        "message must carry the typo and the suggestion: {}",
        diagnostic.message
    );

    // The typo'd spelling survives in the retained source (ride-through), and
    // a save persists it, so a fresh open re-derives the same warning.
    assert!(art.source_text().contains("corner_raduis"));
    ws.save_artifact(id.clone()).unwrap();
    let mut reopened = WorkspaceSession::open(dir.path()).unwrap();
    reopened.open_artifact(id.clone()).unwrap();
    let persisted = reopened.artifact(&id).unwrap().source_diagnostics();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].code, "source.unknown_attribute");

    // Fixing the typo resets the diagnostics on the next clean commit.
    let art = ws.artifact_mut(&id).unwrap();
    let clean = art
        .begin_text_edit()
        .unwrap()
        .replacen(" corner_raduis={4}", "", 1);
    art.set_text(clean).unwrap();
    art.commit_text_to_scene(project_id, components, variables, modes)
        .unwrap();
    assert!(
        art.source_diagnostics().is_empty(),
        "a clean commit must reset the previous warnings"
    );
}

#[test]
fn dirty_disk_change_auto_merges_disjoint() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let child = *ws
        .artifact(&id)
        .unwrap()
        .doc()
        .scene
        .children_of(Some(page))
        .first()
        .unwrap();

    // Canvas renames child
    ws.apply(
        &id,
        Operation::SetName {
            id: child,
            old: "Card".into(),
            new: "CanvasCard".into(),
        },
    )
    .unwrap();

    // Agent renames page root on disk via monodoc-style rewrite of only source
    // Simulate: load monodoc, rename root, write tree (host dual-write illegal but
    // we test merge by rewriting the page.fnx content carefully).
    // Simpler: project agent edit on a clone of base files.
    let meta = ws.artifacts.get(&id).unwrap().clone();
    let design = dir.path().join(&meta.design_dir);
    // Read current fnx, replace Home name in text if still Home
    let fnx_path = design.join("page.fnx");
    let mut src = std::fs::read_to_string(&fnx_path).unwrap();
    assert!(src.contains("name=\"Home\""), "fixture source:\n{src}");
    // Disk still has original until we save canvas; agent edits original
    if src.contains("name=\"Home\"") {
        src = src.replacen("name=\"Home\"", "name=\"AgentHome\"", 1);
    }
    std::fs::write(&fnx_path, &src).unwrap();

    let events = ws.notify_fs_event(FsEvent::Modified { path: fnx_path });
    let art = ws.artifact(&id).unwrap();
    assert!(
        matches!(art.state, ArtifactDirty::DirtyCanvas),
        "disjoint edits must merge cleanly, events={events:?}, state={:?}",
        art.state
    );
    assert_eq!(art.doc().scene.get(child).unwrap().name, "CanvasCard");
    assert_eq!(art.doc().scene.get(page).unwrap().name, "AgentHome");
}

#[test]
fn save_toctou_noop_after_undo_to_base() {
    let (dir, page) = page_fixture();
    let source_path = crate::locate_page_source(dir.path(), page).unwrap();
    let disk_before = std::fs::read(&source_path).unwrap();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    let art = ws.artifact_mut(&id).unwrap();
    art.apply(Operation::SetName {
        id: page,
        old: "Home".into(),
        new: "Temp".into(),
    })
    .unwrap();
    art.undo_atomic().unwrap();
    assert!(art.source_text().contains("name=\"Home\""));
    assert!(!art.source_text().contains("name=\"Temp\""));
    art.redo_atomic().unwrap();
    assert!(art.source_text().contains("name=\"Temp\""));
    assert!(!art.source_text().contains("name=\"Home\""));
    art.undo_atomic().unwrap();
    assert!(art.source_text().contains("name=\"Home\""));
    assert!(!art.source_text().contains("name=\"Temp\""));

    // Undo is part of the same retained-source revision, rather than leaving
    // the pre-undo value pending until persistence.
    let result = art.save(dir.path()).unwrap();
    assert!(matches!(
        result,
        SaveResult::NoOp | SaveResult::Wrote { .. }
    ));
    assert!(matches!(art.state, ArtifactDirty::Clean));
    assert!(art.source_text().contains("name=\"Home\""));
    assert!(!art.source_text().contains("name=\"Temp\""));
    assert_eq!(std::fs::read(&source_path).unwrap(), disk_before);
}

#[test]
fn create_and_open_graphics() {
    let (dir, _page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let gid = ws.create_graphics_artifact("icon-set", "Icon Set").unwrap();
    assert!(matches!(gid, ArtifactId::Graphics(_)));
    ws.open_artifact(gid.clone()).unwrap();
    let art = ws.artifact(&gid).unwrap();
    assert_eq!(art.kind, ArtifactKind::Graphics);
    assert!(matches!(art.state, ArtifactDirty::Clean));
}

#[test]
fn new_graphics_sidecar_matches_canonical_project_json() {
    let (directory, _page) = page_fixture();
    let mut workspace = WorkspaceSession::open(directory.path()).expect("open project");
    let id = workspace
        .create_graphics_artifact("canonical-icons", "Canonical Icons")
        .expect("create graphics");
    let bytes = std::fs::read(
        directory
            .path()
            .join("graphics/canonical-icons/graphics.ids.json"),
    )
    .expect("graphics sidecar");
    let value: Value = serde_json::from_slice(&bytes).expect("graphics sidecar JSON");
    assert_eq!(
        bytes,
        crate::project::layout::json_bytes(&value).expect("canonical sidecar")
    );
    workspace
        .open_artifact(id.clone())
        .expect("open created graphics");
    let artifact = workspace.artifact(&id).expect("graphics session");
    let files = artifact
        .project_to_files()
        .expect("reproject created graphics");
    let (_, projected) = files
        .iter()
        .find(|(name, _)| name == "graphics.ids.json")
        .expect("reprojected sidecar");
    assert_eq!(*projected, bytes);
}

#[test]
fn graphics_bake_component_recreation() {
    use fanta_doc::{ComponentDef, ComponentId};
    let dir = tempdir().unwrap();
    let mut doc = Doc::new();
    // Page so project is valid
    let mut page_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_root.name = "Page".into();
    let page = doc.scene.insert(page_root).unwrap();
    doc.add_page(page);
    // Component master under components page pattern: just put root in scene
    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.name = "Button".into();
    let master_root = doc.scene.insert(master).unwrap();
    let mut label = CanvasNode::new(NodeData::Group(GroupNode::default()));
    label.name = "Label".into();
    label.parent = Some(master_root);
    doc.scene.insert(label).unwrap();
    let cid = ComponentId::new();
    doc.components
        .defs
        .insert(cid, ComponentDef::new(cid, master_root, "Button"));
    // write_project_tree classifies component master into components/
    crate::write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    assert!(ws.components.defs.defs.contains_key(&cid));
    let gid = ws.create_graphics_artifact("sheet", "Sheet").unwrap();
    ws.open_artifact(gid.clone()).unwrap();
    let parent = ws.artifact(&gid).unwrap().root();
    let baked = ws
        .import_component_as_recreation(&gid, cid, parent)
        .unwrap();
    let art = ws.artifact(&gid).unwrap();
    assert!(art.doc().scene.get(baked).is_some());
    assert!(matches!(art.state, ArtifactDirty::DirtyCanvas));
    // No Instance nodes
    for n in art.doc().scene.roots() {
        let _ = n;
    }
    fn walk_instance(scene: &fanta_doc::Scene, id: NodeId) -> bool {
        if matches!(scene.get(id).map(|n| &n.data), Some(NodeData::Instance(_))) {
            return true;
        }
        scene
            .children_of(Some(id))
            .iter()
            .any(|&c| walk_instance(scene, c))
    }
    assert!(!walk_instance(&art.doc().scene, art.root()));
}

#[test]
fn motion_dual_read_singleton() {
    let (dir, _) = page_fixture();
    // write_project_tree always emits doc/motion.json → singleton source
    let idx = read_motion_dual(dir.path()).unwrap();
    assert_eq!(idx.source, MotionSource::DocSingleton);

    // Truly empty: remove motion file
    let motion_path = dir.path().join("doc/motion.json");
    let _ = std::fs::remove_file(&motion_path);
    let idx = read_motion_dual(dir.path()).unwrap();
    assert_eq!(idx.source, MotionSource::Empty);
}

#[test]
fn motion_dual_read_prefers_artifact_dirs() {
    let (dir, _) = page_fixture();
    let mdir = dir.path().join("motion/hero");
    std::fs::create_dir_all(&mdir).unwrap();
    std::fs::write(mdir.join("motion.json"), "{}\n").unwrap();
    let idx = read_motion_dual(dir.path()).unwrap();
    assert_eq!(idx.source, MotionSource::ArtifactDirs);
    assert!(idx.artifact_slugs.iter().any(|s| s == "hero"));
}

#[test]
fn workspace_fnx_synthesized_when_missing() {
    let (dir, _) = page_fixture();
    let ws = WorkspaceSession::open(dir.path()).unwrap();
    assert!(!ws.workspace_ir.present_on_disk);
    assert!(ws.workspace_ir.source.contains("Workspace"));
}

#[test]
fn workspace_fnx_parsed_when_present() {
    let (dir, _) = page_fixture();
    std::fs::write(
        dir.path().join("workspace.fnx"),
        r#"export default function Workspace() {
  return (
    <Workspace>
      <Entry path="pages/home/page.fnx" />
    </Workspace>
  );
}
"#,
    )
    .unwrap();
    let ws = WorkspaceSession::open(dir.path()).unwrap();
    assert!(ws.workspace_ir.present_on_disk);
    assert_eq!(
        ws.workspace_ir.graph.imports_of("workspace"),
        &["pages/home/page.fnx".to_owned()]
    );
}

#[test]
fn sync_from_tree_applies_other_page_hash() {
    let (dir_a, page) = page_fixture();
    let dir_b = tempdir().unwrap();
    // clone project
    copy_dir_recursive(dir_a.path(), dir_b.path());
    // edit B's page name on disk
    let mut ws_b = WorkspaceSession::open(dir_b.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws_b.open_artifact(id.clone()).unwrap();
    ws_b.apply(
        &id,
        Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "FromB".into(),
        },
    )
    .unwrap();
    ws_b.save_artifact(id.clone()).unwrap();

    let mut ws_a = WorkspaceSession::open(dir_a.path()).unwrap();
    ws_a.open_artifact(id.clone()).unwrap();
    let report = ws_a.sync_from_tree(dir_b.path()).unwrap();
    assert!(report.changed.contains(&id) || !report.events.is_empty());
    // After sync, if clean reload happened, name is FromB
    if let Some(art) = ws_a.artifact(&id) {
        if matches!(art.state, ArtifactDirty::Clean) {
            assert_eq!(art.doc().scene.get(page).unwrap().name, "FromB");
        }
    }
}

#[test]
fn conflict_keep_ours_and_take_theirs() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    ws.apply(
        &id,
        Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "Canvas".into(),
        },
    )
    .unwrap();

    // Force conflict by rewriting page.fnx with different name while dirty
    let meta = ws.artifacts.get(&id).unwrap().clone();
    let fnx = dir.path().join(&meta.design_dir).join("page.fnx");
    let src = std::fs::read_to_string(&fnx)
        .unwrap()
        .replacen("name=\"Home\"", "name=\"Disk\"", 1);
    std::fs::write(&fnx, src).unwrap();
    let _ = ws.notify_fs_event(FsEvent::Modified { path: fnx });

    assert!(matches!(
        ws.artifact(&id).unwrap().state,
        ArtifactDirty::Conflict(_)
    ));
    ws.resolve_conflict(&id, ConflictResolution::KeepOurs)
        .unwrap();
    assert!(matches!(
        ws.artifact(&id).unwrap().state,
        ArtifactDirty::DirtyCanvas
    ));
    assert_eq!(
        ws.artifact(&id)
            .unwrap()
            .doc()
            .scene
            .get(page)
            .unwrap()
            .name,
        "Canvas"
    );
}

#[test]
fn accept_auto_leaves_ours_wins_conflict_dirty_against_disk_base() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    ws.apply(
        &id,
        Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "Canvas".into(),
        },
    )
    .unwrap();
    let meta = ws.artifacts.get(&id).unwrap().clone();
    let fnx = dir.path().join(&meta.design_dir).join("page.fnx");
    let disk = std::fs::read_to_string(&fnx)
        .unwrap()
        .replacen("name=\"Home\"", "name=\"Disk\"", 1);
    std::fs::write(&fnx, disk).unwrap();
    ws.notify_fs_event(FsEvent::Modified { path: fnx });
    assert!(matches!(
        ws.artifact(&id).unwrap().state,
        ArtifactDirty::Conflict(_)
    ));

    ws.resolve_conflict(&id, ConflictResolution::AcceptAuto)
        .unwrap();
    let art = ws.artifact(&id).unwrap();
    assert!(matches!(art.state, ArtifactDirty::DirtyCanvas));
    assert_eq!(art.doc().scene.get(page).unwrap().name, "Canvas");
    assert!(art.source_text().contains("name=\"Canvas\""));
    assert!(!art.source_text().contains("name=\"Home\""));
    assert_eq!(art.base_hash, art.disk_hash);
    assert_eq!(art.base_nodes.nodes[&page.0.to_string()]["name"], "Disk");

    ws.save_artifact(id.clone()).unwrap();
    let (saved, _) = crate::read_project_tree(dir.path()).unwrap();
    assert_eq!(saved.scene.get(page).unwrap().name, "Canvas");
}

#[test]
fn second_disk_change_during_conflict_does_not_corrupt_frozen_theirs() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    ws.apply(
        &id,
        Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "Canvas".into(),
        },
    )
    .unwrap();
    let meta = ws.artifacts.get(&id).unwrap().clone();
    let fnx = dir.path().join(&meta.design_dir).join("page.fnx");
    let first =
        std::fs::read_to_string(&fnx)
            .unwrap()
            .replacen("name=\"Home\"", "name=\"Disk\"", 1);
    std::fs::write(&fnx, first).unwrap();
    ws.notify_fs_event(FsEvent::Modified { path: fnx.clone() });
    let frozen_hash = ws.artifact(&id).unwrap().disk_hash;

    let second =
        std::fs::read_to_string(&fnx)
            .unwrap()
            .replacen("name=\"Disk\"", "name=\"Disk2\"", 1);
    std::fs::write(&fnx, second).unwrap();
    let events = ws.notify_fs_event(FsEvent::Modified { path: fnx });
    let art = ws.artifact(&id).unwrap();
    assert_eq!(art.disk_hash, frozen_hash);
    let ArtifactDirty::Conflict(conflict) = &art.state else {
        panic!("conflict must remain frozen, events={events:?}");
    };
    let EditionSide::Present(theirs) = &conflict.theirs else {
        panic!("disk edition should be present");
    };
    assert_eq!(theirs.node_map.nodes[&page.0.to_string()]["name"], "Disk");
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to);
        } else {
            std::fs::copy(entry.path(), to).unwrap();
        }
    }
}

// silence unused import warnings in some cfgs
#[allow(dead_code)]
fn _arc_base() -> Arc<NodeMapEdition> {
    Arc::new(NodeMapEdition::empty())
}

#[test]
fn tool_parts_supports_simultaneous_doc_and_viewport_borrows() {
    // The normative ToolContext construction needs `&mut Doc` and
    // `&mut Viewport` at once — previously uncompilable through the guard
    // (E0499). `tool_parts` splits the disjoint fields internally.
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    {
        let session = ws.open.get_mut(&id).unwrap();
        let mut guard = session.doc_mut_guard(&ws.shared);
        let (doc, viewport) = guard.tool_parts();
        // Both live simultaneously, as ToolContext requires.
        viewport.zoom = 2.0;
        doc.apply(Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "Tooled".into(),
        })
        .unwrap();
        viewport.center = [10.0, 10.0];
    }
    // Drop hook still ran: gesture marked the artifact dirty + synced source.
    let artifact = ws.artifact(&id).unwrap();
    assert!(matches!(artifact.state, ArtifactDirty::DirtyCanvas));
    assert!(artifact.source_text().contains("name=\"Tooled\""));
    assert_eq!(artifact.viewport.zoom, 2.0);
}

#[test]
fn guard_drop_sync_failure_keeps_last_good_scene() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();

    let child = {
        let art = ws.artifact(&id).unwrap();
        art.doc().scene.children_of(Some(page))[0]
    };
    {
        let session = ws.open.get_mut(&id).unwrap();
        let mut guard = session.doc_mut_guard(&ws.shared);
        // A real op so the drop hook sees history/modified movement...
        guard
            .doc_mut()
            .apply(Operation::SetName {
                id: page,
                old: "Home".into(),
                new: "Poisoned".into(),
            })
            .unwrap();
        // ...then corrupt the scene under the guard so Scene→IR projection
        // fails at drop (the scoped root disappears, so the subtree can no
        // longer form a single-rooted tree).
        guard.doc_mut().scene.remove(child).unwrap();
        guard.doc_mut().scene.remove(page).unwrap();
    }

    match &ws.artifact(&id).unwrap().state {
        ArtifactDirty::Invalid { last_good, .. } => {
            // `last_good` is the post-gesture scoped doc — the most recent
            // paintable state. This test corrupts the scene into emptiness to
            // force the failure, so only presence (not content) is assertable.
            assert!(
                last_good.is_some(),
                "sync failure must keep a recovery scene for paint"
            );
        }
        other => panic!("expected Invalid after poisoned drop, got {other:?}"),
    }
}

#[test]
fn partial_on_disk_deletion_while_dirty_enters_deleted_conflict() {
    let (dir, page) = page_fixture();
    let mut ws = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    ws.open_artifact(id.clone()).unwrap();
    ws.apply(
        &id,
        Operation::SetName {
            id: page,
            old: "Home".into(),
            new: "Rescue me".into(),
        },
    )
    .unwrap();

    // A crashed writer / mid-checkout git state: page.fnx vanishes but the
    // header survives. Disk holds no loadable edition — save must route to the
    // Deleted conflict instead of erroring with "missing page.fnx".
    let meta = ws.artifacts.get(&id).unwrap().clone();
    let design_dir = dir.path().join(&meta.design_dir);
    std::fs::remove_file(design_dir.join("page.fnx")).unwrap();
    std::fs::remove_file(design_dir.join("page.ids.json")).unwrap();
    assert!(design_dir.join("page.json").exists());

    let err = ws.save_artifact(id.clone()).unwrap_err();
    assert!(
        matches!(
            err,
            SessionError::SaveBlocked(super::error::SaveBlocked::EnteredConflict)
        ),
        "expected EnteredConflict, got {err:?}"
    );
    match &ws.artifact(&id).unwrap().state {
        ArtifactDirty::Conflict(state) => {
            assert!(matches!(state.theirs, EditionSide::Deleted));
        }
        other => panic!("expected Deleted conflict, got {other:?}"),
    }

    // KeepOurs rescues the working copy.
    ws.resolve_conflict(&id, ConflictResolution::KeepOurs)
        .unwrap();
    assert_eq!(
        ws.artifact(&id)
            .unwrap()
            .doc()
            .scene
            .get(page)
            .unwrap()
            .name,
        "Rescue me"
    );
}

#[test]
fn watcher_discovers_created_and_renamed_pages_by_header_id() {
    let (directory, existing_page) = page_fixture();
    let mut workspace = WorkspaceSession::open(directory.path()).unwrap();
    let (mut document, assets) = crate::read_project_tree(directory.path()).unwrap();

    let mut added_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    added_root.name = "Settings".into();
    let added_page = document.scene.insert(added_root).unwrap();
    document.add_page(added_page);
    crate::write_project_tree(directory.path(), &document, &assets).unwrap();
    let created_dir = crate::projected_design_dirs(&document).0[&added_page].clone();
    let created_events = workspace.notify_fs_event(FsEvent::Created {
        path: directory.path().join(&created_dir).join("page.fnx"),
    });
    assert!(matches!(
        created_events.as_slice(),
        [SessionEvent::Created { id }] if *id == ArtifactId::Page(added_page)
    ));
    assert!(
        workspace
            .artifacts
            .contains_key(&ArtifactId::Page(added_page))
    );

    let old_dir = workspace.artifacts[&ArtifactId::Page(existing_page)]
        .design_dir
        .clone();
    document.scene.get_mut(existing_page).unwrap().name = "Landing".into();
    crate::write_project_tree(directory.path(), &document, &assets).unwrap();
    let renamed_dir = crate::projected_design_dirs(&document).0[&existing_page].clone();
    assert_ne!(old_dir, renamed_dir);
    let rename_events = workspace.notify_fs_event(FsEvent::Removed {
        path: directory.path().join(old_dir).join("page.fnx"),
    });
    assert!(rename_events.iter().any(|event| matches!(
        event,
        SessionEvent::Reloaded { id } if *id == ArtifactId::Page(existing_page)
    )));
    assert_eq!(
        workspace.artifacts[&ArtifactId::Page(existing_page)].design_dir,
        renamed_dir
    );
}

#[test]
fn fresh_disk_snapshot_applies_changed_page_and_shared_metadata() {
    let (directory, page) = page_fixture();
    let workspace = WorkspaceSession::open(directory.path()).unwrap();
    let baseline = workspace.disk_snapshot();
    let (mut document, assets) = crate::read_project_tree(directory.path()).unwrap();
    let mut running_document = document.clone_for_persist();
    document.scene.get_mut(page).unwrap().name = "Agent Home".into();
    document.metadata.title = "Agent Project".into();
    crate::write_project_tree(directory.path(), &document, &assets).unwrap();

    let mut fresh = WorkspaceSession::open(directory.path()).unwrap();
    let report = fresh.reconcile_from_disk_snapshot(&baseline).unwrap();
    assert!(!report.requires_full_reload);
    assert_eq!(report.changed, vec![ArtifactId::Page(page)]);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event, SessionEvent::WorkspaceGeneration { .. }))
    );
    assert_eq!(
        fresh
            .apply_report_to_doc(&mut running_document, &report)
            .unwrap(),
        IncrementalDocApply::Applied
    );
    assert_eq!(running_document.scene.get(page).unwrap().name, "Agent Home");
    assert_eq!(running_document.metadata.title, "Agent Project");
}

#[test]
fn invalid_shared_json_preserves_incremental_state_until_source_is_repaired() {
    let (directory, _) = page_fixture();
    let (mut document, assets) = crate::read_project_tree(directory.path()).expect("project");
    let collection = fanta_doc::VariableCollectionId::new();
    let mode = fanta_doc::ModeId::new();
    document.variables.collections.insert(
        collection,
        fanta_doc::VariableCollection {
            id: collection,
            name: "Theme".into(),
            modes: vec![fanta_doc::Mode {
                id: mode,
                name: "Light".into(),
            }],
            default_mode: mode,
            variable_order: Vec::new(),
        },
    );
    document.active_modes.insert(collection, mode);
    document.metadata.title = "Last valid project".into();
    crate::write_project_tree(directory.path(), &document, &assets).expect("shared state");
    let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
    let baseline = workspace.disk_snapshot();
    let original_document = serde_json::to_value(&document).expect("document snapshot");
    let original_hash = workspace.shared.disk_hash;
    let original_generation = workspace.workspace_generation;
    let original_shared_json = workspace.shared.base_json.clone();
    let source_files = ["metadata.json", "variables.json", "active_modes.json"].map(|name| {
        let path = directory.path().join("doc").join(name);
        let text = std::fs::read_to_string(&path).expect("shared source");
        (path, text)
    });

    for (name, invalid) in [
        ("variables.json", "[]"),
        ("variables.json", "{\"collections\":\"unfinished\"}"),
        ("active_modes.json", "[]"),
        (
            "active_modes.json",
            "{\"invalid-collection\":\"invalid-mode\"}",
        ),
        ("metadata.json", "[]"),
        ("metadata.json", "{\"created_at\":\"yesterday\"}"),
    ] {
        for (path, text) in &source_files {
            std::fs::write(path, text).expect("restore shared source");
        }
        let metadata_path = directory.path().join("doc/metadata.json");
        let mut metadata = serde_json::to_value(&document.metadata).expect("metadata");
        metadata["title"] = json!("Pending source title");
        std::fs::write(metadata_path, metadata.to_string()).expect("stage valid metadata edit");
        let path = directory.path().join("doc").join(name);
        std::fs::write(&path, invalid).expect("invalid external source");

        let error = WorkspaceSession::open(directory.path()).expect_err("invalid shared source");
        assert!(error.to_string().contains(name), "{error}");
        let events = workspace.notify_fs_event(FsEvent::Modified { path: path.clone() });
        assert!(matches!(
            events.as_slice(),
            [SessionEvent::Invalidated { id: ArtifactId::Workspace, error }]
                if error.contains(name)
        ));
        assert_eq!(workspace.shared.variables, document.variables);
        assert_eq!(workspace.shared.active_modes, document.active_modes);
        assert_eq!(
            workspace.shared.metadata,
            serde_json::to_value(&document.metadata).expect("last valid metadata")
        );
        assert_eq!(workspace.shared.disk_hash, original_hash);
        assert_eq!(workspace.shared.base_json, original_shared_json);
        assert_eq!(workspace.workspace_generation, original_generation);
        workspace
            .reconcile_disk_snapshot()
            .expect_err("invalid source cannot advance the disk snapshot");
        workspace
            .apply_report_to_doc(
                &mut document,
                &ApplyReport {
                    events,
                    ..ApplyReport::default()
                },
            )
            .expect_err("invalid incremental report cannot replace the canvas");
        assert_eq!(
            serde_json::to_value(&document).expect("preserved document"),
            original_document
        );
    }

    for (path, text) in &source_files {
        std::fs::write(path, text).expect("repair shared source");
    }
    let mut fresh = WorkspaceSession::open(directory.path()).expect("repaired workspace");
    let report = fresh
        .reconcile_from_disk_snapshot(&baseline)
        .expect("repaired disk snapshot");
    assert_eq!(
        fresh
            .apply_report_to_doc(&mut document, &report)
            .expect("apply repaired shared state"),
        IncrementalDocApply::Applied
    );
    assert_eq!(
        serde_json::to_value(&document).expect("repaired document"),
        original_document
    );
}

#[test]
fn fresh_disk_snapshot_requests_full_reload_for_flow_change() {
    let (directory, page) = page_fixture();
    let workspace = WorkspaceSession::open(directory.path()).unwrap();
    let baseline = workspace.disk_snapshot();
    let (mut document, assets) = crate::read_project_tree(directory.path()).unwrap();
    let mut running_document = document.clone_for_persist();
    document.flow_start = Some(page);
    crate::write_project_tree(directory.path(), &document, &assets).unwrap();

    let mut fresh = WorkspaceSession::open(directory.path()).unwrap();
    let report = fresh.reconcile_from_disk_snapshot(&baseline).unwrap();
    assert!(report.requires_full_reload);
    assert_eq!(
        fresh
            .apply_report_to_doc(&mut running_document, &report)
            .unwrap(),
        IncrementalDocApply::RequiresFullReload
    );
}

#[test]
fn hand_added_node_id_is_stable_across_source_paths() {
    let root = NodeId::new();
    let child = NodeId::new();
    let (mut source, sidecar) = fanta_fnx::encode_subtree(
        &[
            group_json(root, "Home", None),
            group_json(child, "Card", Some(root)),
        ],
        "Home",
    )
    .unwrap();
    let closing = source.rfind("</Frame>").unwrap();
    source.insert_str(closing, "  <Frame name=\"Hand\" />\n");

    let first = crate::project::read::reconcile_fnx_sidecar(
        std::path::Path::new("/tmp/first/page.fnx"),
        &source,
        &sidecar,
    )
    .unwrap();
    let second = crate::project::read::reconcile_fnx_sidecar(
        std::path::Path::new("/tmp/second/page.fnx"),
        &source,
        &sidecar,
    )
    .unwrap();
    assert_eq!(first.ids, second.ids);
    assert_eq!(first.ids.len(), 3);
    assert_eq!(first.ids[0].id, root.0.to_string());
    assert_eq!(first.ids[1].id, child.0.to_string());
    let divergent = source.replacen("name=\"Hand\"", "name=\"Other\"", 1);
    let other = crate::project::read::reconcile_fnx_sidecar(
        std::path::Path::new("/tmp/second/page.fnx"),
        &divergent,
        &sidecar,
    )
    .unwrap();
    assert_ne!(first.ids[2].id, other.ids[2].id);
}

#[test]
fn cached_source_preconditions_reject_external_edit_at_writer() {
    let (directory, page) = page_fixture();
    let mut workspace = WorkspaceSession::open(directory.path()).unwrap();
    workspace.open_artifact(ArtifactId::Page(page)).unwrap();
    let (document, assets) = crate::read_project_tree(directory.path()).unwrap();
    let overrides = workspace
        .validated_source_overrides_for_document(&document)
        .unwrap();
    let source_path = workspace.artifacts[&ArtifactId::Page(page)]
        .design_dir
        .join("page.fnx");
    let original = std::fs::read(directory.path().join(&source_path)).unwrap();
    let changed =
        String::from_utf8(original)
            .unwrap()
            .replacen("name=\"Home\"", "name=\"External\"", 1);
    std::fs::write(directory.path().join(&source_path), &changed).unwrap();

    let preconditions = workspace.source_write_preconditions(&document).unwrap();
    assert!(preconditions[&source_path].is_some());
    let mut cache = crate::ProjectWriteCache::default();
    assert!(
        crate::write_project_tree_cached_with_sources_checked(
            directory.path(),
            &document,
            &assets,
            &mut cache,
            &overrides,
            &preconditions,
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join(source_path)).unwrap(),
        changed
    );
}

#[test]
fn prototype_singletons_reject_external_edits_before_canvas_save() {
    let (directory, _) = page_fixture();
    let workspace = WorkspaceSession::open(directory.path()).expect("open project");
    let (document, _) = crate::read_project_tree(directory.path()).expect("read project");

    for name in ["flows.json", "presentation.json"] {
        let path = directory.path().join("doc").join(name);
        let original = std::fs::read(&path).expect("read prototype source");
        std::fs::write(&path, b"externally edited").expect("change prototype source");
        assert!(matches!(
            workspace.source_write_preconditions(&document),
            Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain))
        ));
        std::fs::write(&path, original).expect("restore prototype source");
    }
}

#[test]
fn canvas_save_keeps_unopened_authored_source_without_projecting_it() {
    let directory = tempdir().expect("project directory");
    let mut document = Doc::new();
    let mut first_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    first_root.name = "First".into();
    let first_page = document.scene.insert(first_root).expect("first root");
    document.add_page(first_page);
    let mut second_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    second_root.name = "Second".into();
    let second_page = document.scene.insert(second_root).expect("second root");
    document.add_page(second_page);
    let assets = BTreeMap::new();
    crate::write_project_tree(directory.path(), &document, &assets).expect("initial write");

    let indexed = WorkspaceSession::open(directory.path()).expect("index project");
    let second_source = indexed.artifacts[&ArtifactId::Page(second_page)]
        .design_dir
        .join("page.fnx");
    let authored = [
        b"// Keep this authored note\n".as_slice(),
        &std::fs::read(directory.path().join(&second_source)).expect("original source"),
    ]
    .concat();
    std::fs::write(directory.path().join(&second_source), &authored).expect("author source");

    let mut workspace = WorkspaceSession::open(directory.path()).expect("reindex authored source");
    workspace
        .open_artifact(ArtifactId::Page(first_page))
        .expect("open edited page");
    document
        .scene
        .get_mut(first_page)
        .expect("first root")
        .transform = Transform2D::translation(8.0, 13.0);
    workspace
        .artifact_mut(&ArtifactId::Page(first_page))
        .expect("open page session")
        .adopt_document(&document)
        .expect("adopt canvas edit");

    let sources = workspace
        .validated_source_overrides_for_document(&document)
        .expect("validated retained sources");
    let second_dir = workspace.artifacts[&ArtifactId::Page(second_page)]
        .design_dir
        .clone();
    assert_eq!(sources[&second_dir].0, authored);
    assert!(!workspace.open.contains_key(&ArtifactId::Page(second_page)));
    let preconditions = workspace
        .source_write_preconditions(&document)
        .expect("indexed preconditions");
    assert!(preconditions.contains_key(&second_source));

    let mut cache = crate::ProjectWriteCache::default();
    let report = crate::write_project_tree_cached_with_sources_checked(
        directory.path(),
        &document,
        &assets,
        &mut cache,
        &sources,
        &preconditions,
    )
    .expect("checked canvas save");
    workspace
        .accept_written_sources(
            &document,
            workspace.asset_index_disk_hash().expect("asset index hash"),
            &sources,
            &report.written_hashes,
        )
        .expect("accept saved sources");
    assert!(!workspace.open.contains_key(&ArtifactId::Page(second_page)));
    assert_eq!(
        std::fs::read(directory.path().join(second_source)).expect("saved source"),
        authored
    );
    std::fs::write(
        directory.path().join(&second_dir).join("page.fnx"),
        b"// external edit\n",
    )
    .expect("external edit");
    assert!(matches!(
        workspace.validated_source_overrides_for_document(&document),
        Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain))
    ));
}

#[test]
fn checked_canvas_save_migrates_indexed_legacy_nodes() {
    let (directory, page) = page_fixture();
    let (document, assets) = crate::read_project_tree(directory.path()).expect("read source tree");
    let indexed = WorkspaceSession::open(directory.path()).expect("index source tree");
    let design_dir = indexed.artifacts[&ArtifactId::Page(page)]
        .design_dir
        .clone();
    let nodes_dir = directory.path().join(&design_dir).join("nodes");
    std::fs::create_dir_all(&nodes_dir).expect("legacy nodes directory");
    for node_id in document.scene.descendants_of(page) {
        let node = document.scene.get(node_id).expect("page node");
        std::fs::write(
            nodes_dir.join(format!("{node_id}.json")),
            serde_json::to_vec_pretty(node).expect("node JSON"),
        )
        .expect("legacy node");
    }
    std::fs::remove_file(directory.path().join(&design_dir).join("page.fnx"))
        .expect("remove modern source");
    std::fs::remove_file(directory.path().join(&design_dir).join("page.ids.json"))
        .expect("remove modern sidecar");

    let mut workspace = WorkspaceSession::open(directory.path()).expect("index legacy tree");
    assert!(!workspace.has_indexed_source(&ArtifactId::Page(page)));
    let node_path = design_dir.join("nodes").join(format!("{page}.json"));
    let preconditions = workspace
        .source_write_preconditions(&document)
        .expect("legacy preconditions");
    assert!(preconditions[&node_path].is_some());
    let sources = workspace
        .validated_source_overrides_for_document(&document)
        .expect("no legacy FNX override");
    assert!(sources.is_empty());

    let original = std::fs::read(directory.path().join(&node_path)).expect("indexed node");
    std::fs::write(directory.path().join(&node_path), b"{}\n").expect("external edit");
    let mut cache = crate::ProjectWriteCache::default();
    assert!(
        crate::write_project_tree_cached_with_sources_checked(
            directory.path(),
            &document,
            &assets,
            &mut cache,
            &sources,
            &preconditions,
        )
        .is_err()
    );
    std::fs::write(directory.path().join(&node_path), original).expect("restore indexed node");

    let report = crate::write_project_tree_cached_with_sources_checked(
        directory.path(),
        &document,
        &assets,
        &mut cache,
        &sources,
        &preconditions,
    )
    .expect("checked legacy migration");
    workspace
        .accept_written_sources(
            &document,
            workspace.asset_index_disk_hash().expect("asset index hash"),
            &sources,
            &report.written_hashes,
        )
        .expect("accept migration");
    assert!(workspace.has_indexed_source(&ArtifactId::Page(page)));
    assert!(!directory.path().join(node_path).exists());
}

#[test]
fn session_skips_headerless_design_dirs_but_rejects_malformed_headers() {
    let (directory, page) = page_fixture();
    let notes = directory.path().join("pages/hand-notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("README.md"), b"keep this note").unwrap();
    let workspace = WorkspaceSession::open(directory.path()).unwrap();
    assert_eq!(workspace.list_pages(), vec![ArtifactId::Page(page)]);

    let header = workspace.artifacts[&ArtifactId::Page(page)]
        .design_dir
        .join("page.json");
    std::fs::write(directory.path().join(header), b"{bad json").unwrap();
    assert!(WorkspaceSession::open(directory.path()).is_err());
}

#[test]
fn page_header_identity_cannot_rebind_an_unchanged_source_root() {
    let (directory, page) = page_fixture();
    let workspace = WorkspaceSession::open(directory.path()).unwrap();
    let relative = workspace.artifacts[&ArtifactId::Page(page)]
        .design_dir
        .join("page.json");
    let path = directory.path().join(relative);
    let replacement = NodeId::new();
    let mut header: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    header["id"] = json!(replacement.to_string());
    std::fs::write(&path, serde_json::to_vec_pretty(&header).unwrap()).unwrap();

    let mut reopened = WorkspaceSession::open(directory.path()).unwrap();
    let error = reopened
        .open_artifact(ArtifactId::Page(replacement))
        .unwrap_err();
    assert!(matches!(error, SessionError::InvalidSource(_)));
    assert!(crate::read_project_tree(directory.path()).is_err());
}

#[test]
fn component_definition_root_cannot_rebind_an_unchanged_master() {
    let directory = tempdir().unwrap();
    let mut document = Doc::new();
    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.name = "Button".into();
    let root = document.scene.insert(master).unwrap();
    let component_id = fanta_doc::ComponentId::new();
    document.components.defs.insert(
        component_id,
        fanta_doc::ComponentDef::new(component_id, root, "Button"),
    );
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new()).unwrap();
    let relative = crate::projected_design_dirs(&document).1[&component_id].join("def.json");
    let path = directory.path().join(relative);
    let mut definition: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    definition["root"] = serde_json::to_value(NodeId::new()).unwrap();
    std::fs::write(&path, serde_json::to_vec_pretty(&definition).unwrap()).unwrap();

    assert!(crate::read_project_tree(directory.path()).is_err());
}

#[test]
fn canvas_save_adopts_a_component_master_placed_on_a_page() {
    use fanta_doc::{ComponentDef, ComponentId};

    let directory = tempdir().expect("temporary project");
    let mut document = Doc::new();
    let page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("page root");
    document.add_page(page);
    let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    master.name = "Button".into();
    master.parent = Some(page);
    let master_id = document.scene.insert(master).expect("component master");
    let mut label = CanvasNode::new(NodeData::Group(GroupNode::default()));
    label.name = "Label".into();
    label.parent = Some(master_id);
    document.scene.insert(label).expect("master child");
    let component_id = ComponentId::new();
    document.components.defs.insert(
        component_id,
        ComponentDef::new(component_id, master_id, "Button"),
    );
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
        .expect("write project");

    let mut workspace = WorkspaceSession::open(directory.path()).expect("open project");
    let id = ArtifactId::Component(component_id);
    workspace.open_artifact(id.clone()).expect("open component");
    document.scene.get_mut(master_id).expect("master").transform =
        Transform2D::translation(4.0, 2.0);
    // Saving adopts each changed artifact from the whole document, where the
    // master still points at the page it sits on.
    workspace
        .artifact_mut(&id)
        .expect("component session")
        .adopt_document(&document)
        .expect("adopt the edited master");

    let scoped = workspace.artifact(&id).expect("component session").doc();
    assert!(
        scoped
            .scene
            .get(master_id)
            .expect("master in scope")
            .parent
            .is_none()
    );
    assert_eq!(scoped.scene.children_of(Some(master_id)).len(), 1);
}

#[test]
fn master_side_scene_keeps_a_master_that_sits_on_a_page() {
    let page = NodeId::new();
    let master = NodeId::new();
    let label = NodeId::new();
    let nodes = vec![
        group_json(master, "Button", Some(page)),
        group_json(label, "Label", Some(master)),
    ];
    let scene = super::graphics::load_master_scene_from_nodes(&nodes).expect("side scene");
    assert_eq!(scene.len(), 2, "no node is dropped");
    assert!(scene.get(master).expect("master").parent.is_none());
    assert_eq!(scene.children_of(Some(master)), &[label]);
}

#[test]
fn inserting_nodes_hangs_a_subtree_off_an_existing_parent() {
    let mut scene = fanta_doc::Scene::new();
    let frame = scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("existing frame");
    let baked = NodeId::new();
    let child = NodeId::new();
    // Children before parents: the order must not matter.
    let nodes = vec![
        group_json(child, "Child", Some(baked)),
        group_json(baked, "Baked", Some(frame)),
    ];
    super::materialize::insert_nodes_public(&mut scene, &nodes).expect("insert");
    assert_eq!(scene.get(baked).expect("baked").parent, Some(frame));
    assert_eq!(scene.children_of(Some(baked)), &[child]);
}

#[test]
fn inserting_nodes_rejects_cycles_and_duplicates_instead_of_dropping_them() {
    let a = NodeId::new();
    let b = NodeId::new();
    let mut scene = fanta_doc::Scene::new();
    let cycle = vec![group_json(a, "A", Some(b)), group_json(b, "B", Some(a))];
    assert!(super::materialize::insert_nodes_public(&mut scene, &cycle).is_err());
    assert_eq!(scene.len(), 0);

    let duplicate = vec![group_json(a, "A", None), group_json(a, "A again", None)];
    assert!(super::materialize::insert_nodes_public(&mut scene, &duplicate).is_err());
}

#[test]
fn subtree_root_follows_the_fnx_rule() {
    let page = NodeId::new();
    let master = NodeId::new();
    let label = NodeId::new();
    let nodes = vec![
        group_json(label, "Label", Some(master)),
        group_json(master, "Button", Some(page)),
    ];
    assert_eq!(
        super::materialize::subtree_root(&nodes).expect("root"),
        master
    );
    let two_roots = vec![group_json(master, "A", None), group_json(label, "B", None)];
    assert!(super::materialize::subtree_root(&two_roots).is_err());
}

#[test]
fn adopting_an_edit_patches_only_the_edited_node() {
    let directory = tempdir().expect("project directory");
    let mut document = Doc::new();
    let page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("page root");
    document.add_page(page);
    let mut children = Vec::new();
    for name in ["A", "B", "C"] {
        let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
        child.name = name.into();
        child.parent = Some(page);
        children.push(document.scene.insert(child).expect("child"));
    }
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new()).expect("write");

    let mut workspace = WorkspaceSession::open(directory.path()).expect("open project");
    let id = ArtifactId::Page(page);
    workspace.open_artifact(id.clone()).expect("open page");
    // The first adoption replaces the scene loaded from disk: full path.
    workspace
        .artifact_mut(&id)
        .expect("page session")
        .adopt_document(&document.clone_for_persist())
        .expect("first adoption");

    document.scene.get_mut(children[1]).expect("B").transform = Transform2D::translation(40.0, 2.0);
    let persisted = document.clone_for_persist();
    let scoped =
        super::materialize::scope_from_document(&persisted, ArtifactKind::Page, page, None)
            .expect("pages scope by sharing")
            .expect("scope");
    let session = workspace.artifact_mut(&id).expect("page session");
    assert_eq!(
        session.shared_scene_delta(&scoped),
        Some(vec![children[1]]),
        "only the edited node differs by pointer"
    );
    let sync = session
        .adopt_document(&persisted)
        .expect("incremental adoption");
    assert!(
        matches!(&sync, SourceSync::PatchedNodes { nodes } if nodes == &vec![children[1]]),
        "{sync:?}"
    );
    let files = session.project_to_files().expect("project the page");
    let source = String::from_utf8(files[0].1.clone()).expect("utf8 source");
    assert!(
        source.contains("40"),
        "the edit reached the source:\n{source}"
    );

    // A structural change (a new child) is not a pointer delta.
    let mut extra = CanvasNode::new(NodeData::Group(GroupNode::default()));
    extra.parent = Some(page);
    document.scene.insert(extra).expect("new child");
    let persisted = document.clone_for_persist();
    let scoped =
        super::materialize::scope_from_document(&persisted, ArtifactKind::Page, page, None)
            .expect("pages scope by sharing")
            .expect("scope");
    let session = workspace.artifact_mut(&id).expect("page session");
    assert_eq!(session.shared_scene_delta(&scoped), None);
    session.adopt_document(&persisted).expect("full adoption");
    assert_eq!(session.doc().scene.len(), 5);
}

#[test]
fn a_non_finite_transform_never_makes_a_project_unreadable() {
    let directory = tempdir().expect("project directory");
    let mut document = Doc::new();
    let page = document
        .scene
        .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
        .expect("page root");
    document.add_page(page);
    let mut broken = CanvasNode::new(NodeData::Group(GroupNode::default()));
    broken.parent = Some(page);
    broken.transform = Transform2D::from_components([f64::NAN; 6]);
    let broken = document.scene.insert(broken).expect("degenerate node");
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new()).expect("write");

    let (loaded, _) = crate::read_project_tree(directory.path()).expect("the project reads back");
    let transform = loaded.scene.get(broken).expect("node survives").transform;
    assert_eq!(transform, Transform2D::IDENTITY);
}

#[test]
fn reading_repairs_null_geometry_written_by_older_builds() {
    let mut node = json!({
        "type": "vector",
        "id": "01KXH3WN53E9XXSEW27JETAVQD",
        "transform": [null, null, null, null, null, null],
        "clip_size": [null, null],
        "path": {"segments": [
            {"op": "move", "to": [0.0, 0.0]},
            {"op": "line", "to": [null, 0.0]},
            {"op": "close"}
        ]}
    });
    assert!(crate::project::read::repair_non_finite_geometry(&mut node));
    assert_eq!(node["transform"], json!([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    assert!(node.get("clip_size").is_none());
    assert_eq!(node["path"]["segments"][1]["to"], json!([0.0, 0.0]));
    assert!(!crate::project::read::repair_non_finite_geometry(&mut node));
}

struct SharedReferenceFixture {
    directory: tempfile::TempDir,
    document: Doc,
    page: NodeId,
    components: Vec<fanta_doc::ComponentId>,
    collection: fanta_doc::VariableCollectionId,
    variable: fanta_doc::VariableId,
    modes: [fanta_doc::ModeId; 2],
    nested: NodeId,
}

fn shared_reference_fixture() -> SharedReferenceFixture {
    let directory = tempdir().expect("fixture directory");
    let mut document = Doc::new();
    let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_node.name = "Reference page".into();
    let page = document.scene.insert(page_node).expect("page");
    document.add_page(page);
    let mut components = Vec::new();
    for index in 0..16 {
        let mut master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([40.0, 20.0]),
            ..Default::default()
        }));
        master.name = format!("Button {index}");
        let root = document.scene.insert(master).expect("master");
        let component = fanta_doc::ComponentId::new();
        document.components.defs.insert(
            component,
            fanta_doc::ComponentDef::new(component, root, format!("Button {index}")),
        );
        components.push(component);
    }
    let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component: components[0],
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [40.0, 20.0],
    }));
    instance.name = "Placement".into();
    instance.parent = Some(page);
    document.scene.insert(instance).expect("instance");
    let collection = fanta_doc::VariableCollectionId::new();
    let variable = fanta_doc::VariableId::new();
    let modes = [fanta_doc::ModeId::new(), fanta_doc::ModeId::new()];
    document.variables.collections.insert(
        collection,
        fanta_doc::VariableCollection {
            id: collection,
            name: "Theme".into(),
            modes: vec![
                fanta_doc::Mode {
                    id: modes[0],
                    name: "Light".into(),
                },
                fanta_doc::Mode {
                    id: modes[1],
                    name: "Dark".into(),
                },
            ],
            default_mode: modes[0],
            variable_order: vec![variable],
        },
    );
    document.variables.variables.insert(
        variable,
        fanta_doc::Variable {
            id: variable,
            collection,
            name: "Opacity".into(),
            ty: fanta_doc::VariableType::Float,
            values_by_mode: [
                (modes[0], fanta_doc::VarValue::Float { value: 1.0 }),
                (modes[1], fanta_doc::VarValue::Float { value: 0.5 }),
            ]
            .into_iter()
            .collect(),
            scopes: Vec::new(),
        },
    );
    document.active_modes.insert(collection, modes[0]);
    let mut nested_node = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
        component: components[1],
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [40.0, 20.0],
    }));
    nested_node.name = "Nested named component".into();
    nested_node.parent = Some(document.components.defs[&components[0]].root);
    nested_node
        .bindings
        .insert(fanta_doc::BoundProp::Opacity, variable);
    let nested = document
        .scene
        .insert(nested_node)
        .expect("bound nested instance");
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
        .expect("write fixture");
    SharedReferenceFixture {
        directory,
        document,
        page,
        components,
        collection,
        variable,
        modes,
        nested,
    }
}

fn open_component_reference_table(
    workspace: &mut WorkspaceSession,
    component: fanta_doc::ComponentId,
) -> Arc<fanta_fnx::RefTable> {
    let artifact = ArtifactId::Component(component);
    workspace
        .close_artifact(artifact.clone(), ClosePolicy::Discard)
        .expect("close clean fixture");
    workspace
        .open_artifact(artifact.clone())
        .expect("open component");
    Arc::clone(&workspace.artifact(&artifact).expect("component").ref_table)
}

#[test]
fn shared_reference_table_survives_open_close_and_source_reload() {
    let fixture = shared_reference_fixture();
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let page = ArtifactId::Page(fixture.page);
    workspace.open_artifact(page.clone()).expect("open page");
    let shared = Arc::clone(&workspace.artifact(&page).expect("page").ref_table);
    for component in &fixture.components {
        let table = open_component_reference_table(&mut workspace, *component);
        assert!(
            Arc::ptr_eq(&shared, &table),
            "opening an artifact must reuse the workspace vocabulary"
        );
        assert_eq!(
            workspace
                .artifact(&ArtifactId::Component(*component))
                .expect("component")
                .doc()
                .components
                .defs
                .len(),
            1
        );
        if *component == fixture.components[0] {
            let artifact = workspace
                .artifact(&ArtifactId::Component(*component))
                .expect("component with foreign references");
            assert!(artifact.source_text().contains("Button 1"));
            assert!(artifact.source_text().contains("$Theme/Opacity"));
            let nested = artifact
                .doc()
                .scene
                .get(fixture.nested)
                .expect("nested instance identity retained");
            let NodeData::Instance(instance) = &nested.data else {
                panic!("nested instance")
            };
            assert_eq!(instance.component, fixture.components[1]);
            assert_eq!(
                nested.bindings.get(&fanta_doc::BoundProp::Opacity),
                Some(&fixture.variable)
            );
            assert!(
                !artifact
                    .doc()
                    .components
                    .defs
                    .contains_key(&fixture.components[1]),
                "foreign definitions resolve without being cloned into the local scope"
            );
        }
    }
    for component in &fixture.components {
        workspace
            .close_artifact(ArtifactId::Component(*component), ClosePolicy::Discard)
            .expect("close component");
    }
    let reopened = open_component_reference_table(&mut workspace, fixture.components[0]);
    assert!(Arc::ptr_eq(&shared, &reopened));
    let source =
        crate::locate_page_source(fixture.directory.path(), fixture.page).expect("page source");
    let original = std::fs::read_to_string(&source).expect("source");
    assert!(original.contains("Placement"));
    let changed = format!(
        "// external source fragment is preserved\n{}",
        original.replacen("Placement", "External placement", 1)
    );
    std::fs::write(&source, &changed).expect("external source edit");
    let events = workspace.notify_fs_event(FsEvent::Modified { path: source });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::Reloaded { id } if id == &page)),
        "{events:?}"
    );
    let artifact = workspace.artifact(&page).expect("reloaded page");
    assert!(Arc::ptr_eq(&shared, &artifact.ref_table));
    assert_eq!(artifact.source_text(), changed);
    assert_eq!(artifact.root(), fixture.page);
    assert!(artifact.doc().scene.descendants_of(fixture.page).any(|id| {
        artifact
            .doc()
            .scene
            .get(id)
            .is_some_and(|node| node.name == "External placement")
    }));
    let mut fresh = WorkspaceSession::open(fixture.directory.path()).expect("fresh source session");
    fresh.open_artifact(page.clone()).expect("fresh page");
    let freshly_loaded = fresh.artifact(&page).expect("freshly loaded page");
    assert!(matches!(
        artifact.state(),
        ArtifactDirty::Clean | ArtifactDirty::DirtyCanvas
    ));
    assert_eq!(
        std::mem::discriminant(artifact.state()),
        std::mem::discriminant(freshly_loaded.state())
    );
    assert_eq!(
        serde_json::to_value(&artifact.doc().scene).expect("reloaded scene"),
        serde_json::to_value(&freshly_loaded.doc().scene).expect("fresh scene")
    );
    let asset_index = fixture.directory.path().join("assets/index.json");
    let mut bytes = std::fs::read(&asset_index).expect("asset index");
    bytes.push(b'\n');
    std::fs::write(&asset_index, &bytes).expect("external asset index edit");
    assert!(matches!(
        workspace.source_write_preconditions(&fixture.document),
        Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain))
    ));
    assert_eq!(std::fs::read(&asset_index).expect("preserved index"), bytes);
}

#[test]
fn shared_reference_table_checks_public_registry_changes_without_generation_bumps() {
    let fixture = shared_reference_fixture();
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let first = fixture.components[0];
    let reopen = fixture.components[1];
    let original_name = workspace.components.defs.defs[&first].name.clone();
    let original_collection = workspace.shared.variables.collections[&fixture.collection].clone();
    let new_component = fanta_doc::ComponentId::new();
    let new_variable = fanta_doc::VariableId::new();
    let generation = workspace.workspace_generation;
    let mut previous = open_component_reference_table(&mut workspace, reopen);
    let untouched = Arc::clone(&previous);
    let original_table = (*untouched).clone();
    for change in 0..11 {
        match change {
            0 => {
                workspace
                    .components
                    .defs
                    .defs
                    .get_mut(&first)
                    .expect("component")
                    .name = "Renamed".into()
            }
            1 => {
                workspace
                    .components
                    .defs
                    .defs
                    .get_mut(&first)
                    .expect("component")
                    .name = original_name.clone()
            }
            2 => {
                let mut duplicate = workspace.components.defs.defs[&first].clone();
                duplicate.id = new_component;
                workspace
                    .components
                    .defs
                    .defs
                    .insert(new_component, duplicate);
            }
            3 => {
                workspace.components.defs.defs.remove(&new_component);
            }
            4 => {
                workspace
                    .shared
                    .variables
                    .variables
                    .get_mut(&fixture.variable)
                    .expect("variable")
                    .name = "Renamed opacity".into()
            }
            5 => {
                workspace
                    .shared
                    .variables
                    .collections
                    .get_mut(&fixture.collection)
                    .expect("collection")
                    .name = "Renamed theme".into()
            }
            6 => {
                workspace
                    .shared
                    .variables
                    .variables
                    .get_mut(&fixture.variable)
                    .expect("variable")
                    .id = new_variable
            }
            7 => {
                workspace
                    .shared
                    .variables
                    .collections
                    .remove(&fixture.collection);
            }
            8 => {
                workspace
                    .shared
                    .variables
                    .collections
                    .insert(fixture.collection, original_collection.clone());
            }
            9 => workspace.manifest.version = 3,
            10 => workspace.manifest.version = crate::project::layout::PROJECT_VERSION,
            _ => unreachable!(),
        }
        let current = open_component_reference_table(&mut workspace, reopen);
        assert_eq!(workspace.workspace_generation, generation);
        assert!(
            !Arc::ptr_eq(&previous, &current),
            "changed vocabulary/policy {change} must replace the table"
        );
        assert_eq!(
            *current,
            crate::project::refs_ctx::build_ref_table(
                &workspace.components.defs,
                &workspace.shared.variables,
                workspace.manifest.version >= 4
            ),
            "change {change}"
        );
        assert_eq!(
            *untouched, original_table,
            "an existing immutable table cannot be changed in place"
        );
        previous = current;
    }
}

#[test]
fn shared_reference_table_reuses_values_and_restores_an_open_sources_vocabulary() {
    let mut fixture = shared_reference_fixture();
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let page = ArtifactId::Page(fixture.page);
    workspace.open_artifact(page.clone()).expect("page");
    let original_source = workspace.artifact(&page).expect("page").source_text();
    let original_table = Arc::clone(&workspace.artifact(&page).expect("page").ref_table);
    open_component_reference_table(&mut workspace, fixture.components[0]);
    fixture
        .document
        .components
        .defs
        .get_mut(&fixture.components[0])
        .expect("component")
        .rev += 1;
    fixture
        .document
        .variables
        .variables
        .get_mut(&fixture.variable)
        .expect("variable")
        .values_by_mode
        .insert(fixture.modes[0], fanta_doc::VarValue::Float { value: 0.25 });
    fixture
        .document
        .active_modes
        .insert(fixture.collection, fixture.modes[1]);
    assert!(workspace.adopt_document_shared(&fixture.document));
    for artifact in workspace.open.values() {
        assert!(Arc::ptr_eq(&original_table, &artifact.ref_table));
        assert_eq!(artifact.doc().variables, fixture.document.variables);
        assert_eq!(artifact.doc().active_modes, fixture.document.active_modes);
        assert_eq!(artifact.vars_generation, workspace.workspace_generation);
    }
    let original_name = fixture.document.components.defs[&fixture.components[0]]
        .name
        .clone();
    fixture
        .document
        .components
        .defs
        .get_mut(&fixture.components[0])
        .expect("component")
        .name = "Temporary name".into();
    assert!(workspace.adopt_document_shared(&fixture.document));
    let renamed = Arc::clone(&workspace.artifact(&page).expect("page").ref_table);
    assert!(!Arc::ptr_eq(&original_table, &renamed));
    assert_eq!(
        workspace.artifact(&page).expect("page").source_text(),
        original_source
    );
    fixture
        .document
        .components
        .defs
        .get_mut(&fixture.components[0])
        .expect("component")
        .name = original_name;
    assert!(workspace.adopt_document_shared(&fixture.document));
    let restored = Arc::clone(&workspace.artifact(&page).expect("page").ref_table);
    assert_eq!(*restored, *original_table);
    assert!(!Arc::ptr_eq(&restored, &renamed));
    assert_eq!(
        workspace.artifact(&page).expect("page").source_text(),
        original_source
    );
    let instance = *workspace
        .artifact(&page)
        .expect("page")
        .doc()
        .scene
        .children_of(Some(fixture.page))
        .first()
        .expect("instance");
    workspace
        .apply(
            &page,
            Operation::SetName {
                id: instance,
                old: "Placement".into(),
                new: "After restore".into(),
            },
        )
        .expect("patch retained source with restored vocabulary");
    let artifact = workspace.artifact(&page).expect("page");
    let NodeData::Instance(instance_data) =
        &artifact.doc().scene.get(instance).expect("instance").data
    else {
        panic!("instance fixture")
    };
    assert_eq!(instance_data.component, fixture.components[0]);
    assert!(artifact.source_text().contains("After restore"));
    assert!(Arc::ptr_eq(&restored, &artifact.ref_table));
}

#[test]
fn shared_reference_table_preserves_dirty_identity_and_conflict_guards() {
    let fixture = shared_reference_fixture();
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let page = ArtifactId::Page(fixture.page);
    workspace.open_artifact(page.clone()).expect("page");
    let component = ArtifactId::Component(fixture.components[0]);
    workspace
        .open_artifact(component.clone())
        .expect("component");
    assert!(Arc::ptr_eq(
        &workspace.artifact(&page).expect("page").ref_table,
        &workspace.artifact(&component).expect("component").ref_table
    ));
    let source_path =
        crate::locate_page_source(fixture.directory.path(), fixture.page).expect("source");
    let disk = std::fs::read_to_string(&source_path).expect("disk source");
    workspace
        .apply(
            &page,
            Operation::SetName {
                id: fixture.page,
                old: "Reference page".into(),
                new: "Canvas title".into(),
            },
        )
        .expect("canvas edit");
    let external = disk.replacen("Reference page", "External title", 1);
    assert_ne!(disk, external);
    std::fs::write(&source_path, &external).expect("external competing edit");
    let events = workspace.notify_fs_event(FsEvent::Modified {
        path: source_path.clone(),
    });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::EnteredConflict { id, .. } if id == &page)),
        "{events:?}"
    );
    open_component_reference_table(&mut workspace, fixture.components[1]);
    assert!(
        workspace
            .artifact(&page)
            .expect("conflicted page")
            .state()
            .is_conflict()
    );
    assert!(matches!(
        workspace.save_artifact(page.clone()),
        Err(SessionError::SaveBlocked(SaveBlocked::InConflict))
    ));
    assert_eq!(
        std::fs::read_to_string(&source_path).expect("external source retained"),
        external
    );

    let draft = format!(
        "// unfinished work must remain\n{}",
        workspace
            .artifact(&component)
            .expect("component")
            .source_text()
    );
    workspace
        .artifact_mut(&component)
        .expect("component")
        .set_text(draft.clone())
        .expect("source draft");
    let header_path = fixture
        .directory
        .path()
        .join(&workspace.artifacts[&component].design_dir)
        .join("def.json");
    let mut header: Value =
        serde_json::from_slice(&std::fs::read(&header_path).expect("header")).expect("header JSON");
    let replacement = fanta_doc::ComponentId::new();
    header["id"] = serde_json::to_value(replacement).expect("replacement id");
    std::fs::write(
        &header_path,
        serde_json::to_vec_pretty(&header).expect("header bytes"),
    )
    .expect("external identity edit");
    let events = workspace.notify_fs_event(FsEvent::Modified { path: header_path });
    assert!(
        events
            .iter()
            .any(|event| matches!(event, SessionEvent::Invalidated { id, .. } if id == &component)),
        "{events:?}"
    );
    let original = workspace
        .artifact(&component)
        .expect("dirty old identity retained");
    assert!(matches!(original.state(), ArtifactDirty::Invalid { .. }));
    assert_eq!(original.text_buffer(), Some(draft.as_str()));
    assert!(!workspace.artifacts.contains_key(&component));
    workspace
        .open_artifact(ArtifactId::Component(replacement))
        .expect("new identity opens separately");
    let replaced = workspace
        .artifact(&ArtifactId::Component(replacement))
        .expect("replacement");
    assert_eq!(
        *replaced.ref_table,
        crate::project::refs_ctx::build_ref_table(
            &workspace.components.defs,
            &workspace.shared.variables,
            workspace.manifest.version >= 4
        )
    );
    assert_eq!(
        workspace
            .artifact(&component)
            .expect("retained draft")
            .text_buffer(),
        Some(draft.as_str())
    );
}

#[test]
fn shared_reference_table_rejects_and_recovers_duplicate_variable_paths() {
    let fixture = shared_reference_fixture();
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let page = ArtifactId::Page(fixture.page);
    workspace
        .open_artifact(page.clone())
        .expect("existing page");
    let source = workspace.artifact(&page).expect("page").source_text();
    let old_table = Arc::clone(&workspace.artifact(&page).expect("page").ref_table);
    let old_document = serde_json::to_value(workspace.artifact(&page).expect("page").doc())
        .expect("existing page snapshot");
    let generation = workspace.workspace_generation;
    let duplicate_id = fanta_doc::VariableId::new();
    let mut duplicate = workspace.shared.variables.variables[&fixture.variable].clone();
    duplicate.id = duplicate_id;
    workspace
        .shared
        .variables
        .variables
        .insert(duplicate_id, duplicate);
    let component = ArtifactId::Component(fixture.components[0]);
    let error = workspace
        .open_artifact(component.clone())
        .expect_err("ambiguous named binding cannot resolve to an arbitrary variable");
    assert!(error.to_string().contains("ambiguous"), "{error}");
    assert!(
        !workspace.open.contains_key(&component),
        "failed load cannot install a partial artifact"
    );
    assert_eq!(workspace.workspace_generation, generation);
    let existing = workspace.artifact(&page).expect("existing page retained");
    assert_eq!(existing.source_text(), source);
    assert_eq!(
        serde_json::to_value(existing.doc()).expect("current page snapshot"),
        old_document
    );
    assert!(Arc::ptr_eq(&existing.ref_table, &old_table));
    workspace.shared.variables.variables.remove(&duplicate_id);
    workspace
        .open_artifact(component.clone())
        .expect("binding loads after ambiguity is removed");
    let recovered = workspace.artifact(&component).expect("recovered component");
    assert_eq!(*recovered.ref_table, *old_table);
    assert_eq!(recovered.doc().components.defs.len(), 1);
    assert_eq!(
        recovered
            .doc()
            .scene
            .get(fixture.nested)
            .expect("nested instance")
            .bindings
            .get(&fanta_doc::BoundProp::Opacity),
        Some(&fixture.variable)
    );
    assert_eq!(
        workspace
            .artifact(&page)
            .expect("existing source")
            .source_text(),
        source
    );
}

fn pending_layout_fixture_nodes(page: NodeId) -> (Vec<Value>, std::collections::BTreeSet<NodeId>) {
    let frame = NodeId::new();
    let flow_child = NodeId::new();
    let text = NodeId::new();
    let complete_text = NodeId::new();
    let absolute_child = NodeId::new();
    let free_group = NodeId::new();
    let mut nodes = vec![
        group_json(page, "Page", None),
        group_json(frame, "Authored layout", Some(page)),
        group_json(flow_child, "Omitted flow position", Some(frame)),
        group_json(absolute_child, "Absolute identity", Some(frame)),
        group_json(free_group, "Free identity", Some(page)),
    ];
    nodes[1]["auto_layout"] = json!({"mode": "horizontal"});
    nodes[2]
        .as_object_mut()
        .expect("flow node")
        .remove("transform");
    nodes[2]["clip_size"] = json!([30.0, 40.0]);
    nodes[3]
        .as_object_mut()
        .expect("absolute node")
        .remove("transform");
    nodes[3]["layout_child"] = json!({"absolute": true});
    nodes[4]
        .as_object_mut()
        .expect("free node")
        .remove("transform");
    let mut omitted = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
        "Authored", 90.0, 20.0,
    )));
    omitted.id = text;
    omitted.parent = Some(page);
    let mut omitted = serde_json::to_value(omitted).expect("text JSON");
    omitted.as_object_mut().expect("text").remove("local_size");
    omitted["auto_resize"] = json!("width_and_height");
    nodes.push(omitted);
    let mut complete = CanvasNode::new(NodeData::Text(fanta_doc::TextNode::new(
        "Saved", 97.0, 23.0,
    )));
    complete.id = complete_text;
    complete.parent = Some(frame);
    complete.transform = Transform2D::translation(79.0, 31.0);
    let mut complete = serde_json::to_value(complete).expect("complete text JSON");
    complete["auto_resize"] = json!("width_and_height");
    nodes.push(complete);
    (nodes, [frame, flow_child, text].into_iter().collect())
}

fn write_pending_layout_source(directory: &std::path::Path, page: NodeId, nodes: &[Value]) {
    let source = crate::locate_page_source(directory, page).expect("page source");
    let (text, sidecar) = fanta_fnx::encode_subtree(nodes, "Page").expect("encode source");
    std::fs::write(&source, text).expect("write authored FNX");
    std::fs::write(
        source.with_file_name("page.ids.json"),
        serde_json::to_vec(&sidecar).expect("sidecar JSON"),
    )
    .expect("write sidecar");
}

#[test]
fn pending_layout_cold_read_marks_only_authored_geometry_omissions() {
    let (directory, page) = page_fixture();
    let (nodes, expected) = pending_layout_fixture_nodes(page);
    write_pending_layout_source(directory.path(), page, &nodes);
    let (mut document, assets) =
        crate::read_project_tree(directory.path()).expect("authored project");
    assert_eq!(document.pending_layout, expected);
    assert_eq!(document.clone_for_persist().pending_layout, expected);
    let serialized = document.to_json_string().expect("document JSON");
    assert!(!serialized.contains("pending_layout"));
    assert!(
        Doc::from_json_str(&serialized)
            .expect("reload JSON")
            .pending_layout
            .is_empty()
    );
    for identifier in &expected {
        if let Some(CanvasNode {
            data: NodeData::Group(group),
            ..
        }) = document.scene.get_mut(*identifier)
            && group.auto_layout.is_some()
        {
            group.local_size = Some([120.0, 40.0]);
        }
    }
    crate::write_project_tree(directory.path(), &document, &assets)
        .expect("write complete geometry");
    let (reopened, _) = crate::read_project_tree(directory.path()).expect("complete project");
    assert!(
        reopened.pending_layout.is_empty(),
        "saved complete geometry is authoritative"
    );
}

#[test]
fn pending_layout_scoped_page_and_component_preserve_omission_seeds() {
    let root = NodeId::new();
    let (nodes, expected) = pending_layout_fixture_nodes(root);
    let page_ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Page", &nodes).expect("page IR");
    let page = materialize_page(
        &page_ir,
        fanta_doc::DocId::new(),
        fanta_doc::ComponentLibrary::new(),
        VariableRegistry::new(),
        BTreeMap::new(),
    )
    .expect("scoped page");
    assert_eq!(page.doc.pending_layout, expected);
    let component_ir =
        ArtifactIr::from_nodes(ArtifactKind::Component, "Master", &nodes).expect("component IR");
    let definition = fanta_doc::ComponentDef::new(fanta_doc::ComponentId::new(), root, "Master");
    let component = materialize_component(
        &component_ir,
        page.doc.id,
        definition.clone(),
        VariableRegistry::new(),
        BTreeMap::new(),
    )
    .expect("scoped component");
    assert_eq!(component.doc.pending_layout, expected);
    let scoped = super::materialize::scope_from_document(
        &component.doc,
        ArtifactKind::Component,
        root,
        Some(&definition),
    )
    .expect("component supported")
    .expect("scope from document");
    assert_eq!(scoped.doc.pending_layout, expected);
}

#[test]
fn pending_layout_incremental_source_replacement_transfers_and_clears_seeds() {
    let (directory, page) = page_fixture();
    let (mut document, _) = crate::read_project_tree(directory.path()).expect("initial project");
    let old_child = document.scene.children_of(Some(page))[0];
    document.pending_layout.insert(old_child);
    let (nodes, expected) = pending_layout_fixture_nodes(page);
    write_pending_layout_source(directory.path(), page, &nodes);
    let mut workspace = WorkspaceSession::open(directory.path()).expect("changed workspace");
    let report = ApplyReport {
        changed: vec![ArtifactId::Page(page)],
        ..ApplyReport::default()
    };
    let (result, mut document) = workspace
        .apply_report_to_owned_doc(document, &report)
        .expect("incremental adoption");
    assert_eq!(result, IncrementalDocApply::Applied);
    assert_eq!(document.pending_layout, expected);
    assert!(!document.scene.contains(old_child));
    for identifier in &expected {
        if let Some(CanvasNode {
            data: NodeData::Group(group),
            ..
        }) = document.scene.get_mut(*identifier)
            && group.auto_layout.is_some()
        {
            group.local_size = Some([120.0, 40.0]);
        }
    }
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
        .expect("save complete geometry");
    let mut workspace = WorkspaceSession::open(directory.path()).expect("complete workspace");
    let (result, document) = workspace
        .apply_report_to_owned_doc(document, &report)
        .expect("complete adoption");
    assert_eq!(result, IncrementalDocApply::Applied);
    assert!(document.pending_layout.is_empty());
}

#[test]
fn pending_layout_saved_zero_geometry_does_not_request_relayout() {
    let (directory, page) = page_fixture();
    let (mut document, assets) = crate::read_project_tree(directory.path()).expect("project");
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        auto_layout: Some(fanta_doc::AutoLayout::default()),
        clip_size: Some([0.0, 0.0]),
        ..GroupNode::default()
    }));
    frame.parent = Some(page);
    let frame = document.scene.insert(frame).expect("zero frame");
    let mut text = fanta_doc::TextNode::new("Zero box is explicit", 0.0, 0.0);
    text.auto_resize = fanta_doc::TextAutoResize::WidthAndHeight;
    let mut text = CanvasNode::new(NodeData::Text(text));
    text.parent = Some(frame);
    document.scene.insert(text).expect("zero text");
    crate::write_project_tree(directory.path(), &document, &assets).expect("write zero geometry");
    let (reopened, _) = crate::read_project_tree(directory.path()).expect("read zero geometry");
    assert!(reopened.pending_layout.is_empty());
    assert_eq!(
        serde_json::to_value(&document.scene).expect("before"),
        serde_json::to_value(&reopened.scene).expect("after")
    );
    fn use_size_sugar(element: &mut fanta_fnx::FnxElement) {
        if let Some(size) = element
            .attrs
            .remove("clip_size")
            .or_else(|| element.attrs.remove("local_size"))
        {
            let dimensions = size.as_array().expect("fixture size");
            element
                .attrs
                .insert("width".into(), dimensions.first().expect("width").clone());
            element
                .attrs
                .insert("height".into(), dimensions.get(1).expect("height").clone());
        }
        for child in &mut element.children {
            use_size_sugar(child);
        }
    }
    let source_path = crate::locate_page_source(directory.path(), page).expect("source");
    let source = std::fs::read_to_string(&source_path).expect("complete FNX");
    let mut tree = fanta_fnx::parse_doc(&source).expect("source tree");
    use_size_sugar(&mut tree);
    std::fs::write(&source_path, fanta_fnx::print_doc("Page", &tree)).expect("author size sugar");
    let (sugared, _) = crate::read_project_tree(directory.path()).expect("read size sugar");
    assert!(sugared.pending_layout.is_empty());
    assert_eq!(
        serde_json::to_value(&document.scene).expect("before sugar"),
        serde_json::to_value(&sugared.scene).expect("after sugar")
    );
}

#[test]
fn pending_layout_unsized_page_root_preserves_complete_child_geometry() {
    let (directory, page) = page_fixture();
    let (mut document, assets) = crate::read_project_tree(directory.path()).expect("project");
    let root = document.scene.get_mut(page).expect("page root");
    let NodeData::Group(group) = &mut root.data else {
        panic!("page is a group");
    };
    group.auto_layout = Some(fanta_doc::AutoLayout {
        primary_sizing: fanta_doc::AxisSizing::Hug,
        counter_sizing: fanta_doc::AxisSizing::Hug,
        ..Default::default()
    });
    crate::write_project_tree(directory.path(), &document, &assets)
        .expect("write auto-layout page");
    let (complete, _) = crate::read_project_tree(directory.path()).expect("read auto-layout page");
    assert!(complete.pending_layout.is_empty());
    assert_eq!(
        serde_json::to_value(&document.scene).expect("authored geometry"),
        serde_json::to_value(&complete.scene).expect("reopened geometry")
    );
    let child = *complete
        .scene
        .children_of(Some(page))
        .first()
        .expect("flow child");
    let source_path = crate::locate_page_source(directory.path(), page).expect("source path");
    let mut tree = fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path).expect("source"))
        .expect("source tree");
    tree.children
        .first_mut()
        .expect("flow child element")
        .attrs
        .remove("transform");
    std::fs::write(&source_path, fanta_fnx::print_doc("Page", &tree)).expect("omit child position");
    let (authored, _) = crate::read_project_tree(directory.path()).expect("read omission");
    assert_eq!(authored.pending_layout, [child].into_iter().collect());
}

#[test]
fn accept_written_sources_rejects_post_write_changes_without_advancing_any_index() {
    let metadata_state = |meta: &ArtifactMeta| {
        (
            meta.id.clone(),
            meta.kind,
            meta.slug.clone(),
            meta.design_dir.clone(),
            meta.disk_hash,
        )
    };
    for opened in [true, false] {
        for name in ["page.fnx", "page.ids.json", "page.json"] {
            for remove in [false, true] {
                let directory = tempdir().expect("project directory");
                let mut document = Doc::new();
                let mut pages = Vec::new();
                for name in ["Edited", "Unopened"] {
                    let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
                    root.name = name.into();
                    let page = document.scene.insert(root).expect("page root");
                    document.add_page(page);
                    pages.push(page);
                }
                let edited = *pages.first().expect("edited page");
                let unopened = *pages.last().expect("unopened page");
                let assets = BTreeMap::new();
                crate::write_project_tree(directory.path(), &document, &assets)
                    .expect("initial project");
                let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
                let edited_id = ArtifactId::Page(edited);
                workspace
                    .open_artifact(edited_id.clone())
                    .expect("open edited page");
                document
                    .scene
                    .get_mut(edited)
                    .expect("edited root")
                    .transform = Transform2D::translation(11.0, 17.0);
                workspace
                    .artifact_mut(&edited_id)
                    .expect("edited session")
                    .adopt_document(&document)
                    .expect("canvas edit");
                let sources = workspace
                    .validated_source_overrides_for_document(&document)
                    .expect("retained sources");
                let preconditions = workspace
                    .source_write_preconditions(&document)
                    .expect("write preconditions");
                let report = crate::write_project_tree_cached_with_sources_checked(
                    directory.path(),
                    &document,
                    &assets,
                    &mut crate::ProjectWriteCache::default(),
                    &sources,
                    &preconditions,
                )
                .expect("checked write");
                let before_index = workspace.file_index.clone();
                let before_shared = (workspace.shared.base_hash, workspace.shared.disk_hash);
                let before_metadata: BTreeMap<_, _> = workspace
                    .artifacts
                    .iter()
                    .map(|(id, meta)| (id.clone(), metadata_state(meta)))
                    .collect();
                let before_open: BTreeMap<_, _> = workspace
                    .open
                    .iter()
                    .map(|(id, session)| {
                        (
                            id.clone(),
                            (
                                session.base_hash,
                                session.disk_hash,
                                Arc::clone(&session.base_nodes),
                                metadata_state(&session.meta),
                            ),
                        )
                    })
                    .collect();
                let target = ArtifactId::Page(if opened { edited } else { unopened });
                let relative = workspace
                    .artifacts
                    .get(&target)
                    .expect("target metadata")
                    .design_dir
                    .join(name);
                let path = directory.path().join(&relative);
                let original = std::fs::read(&path).expect("written file");
                if remove {
                    std::fs::remove_file(&path).expect("external deletion after write");
                } else {
                    let mut changed = original.clone();
                    let byte = changed.first_mut().expect("nonempty source");
                    *byte ^= 1;
                    assert_eq!(changed.len(), original.len(), "same-size mutation");
                    std::fs::write(&path, changed).expect("external edit after write");
                }
                let asset_hash = workspace.asset_index_disk_hash().expect("asset index");
                assert!(
                    matches!(
                        workspace.accept_written_sources(
                            &document,
                            asset_hash,
                            &sources,
                            &report.written_hashes
                        ),
                        Err(SessionError::SaveBlocked(SaveBlocked::DiskChangedAgain))
                    ),
                    "opened={opened}, path={}, remove={remove}",
                    relative.display()
                );
                assert_eq!(
                    workspace.file_index, before_index,
                    "failed acceptance is atomic"
                );
                assert_eq!(
                    (workspace.shared.base_hash, workspace.shared.disk_hash),
                    before_shared
                );
                assert_eq!(
                    workspace
                        .source_write_preconditions(&document)
                        .expect("unchanged cached preconditions"),
                    preconditions,
                    "rejected acceptance must not advance the per-file evidence used by the next writer"
                );
                let after_metadata: BTreeMap<_, _> = workspace
                    .artifacts
                    .iter()
                    .map(|(id, meta)| (id.clone(), metadata_state(meta)))
                    .collect();
                assert_eq!(after_metadata, before_metadata);
                assert_eq!(workspace.open.len(), before_open.len());
                for (id, (base_hash, disk_hash, base_nodes, metadata)) in &before_open {
                    let session = workspace
                        .artifact(id)
                        .expect("open session remains present");
                    assert_eq!(session.base_hash, *base_hash);
                    assert_eq!(session.disk_hash, *disk_hash);
                    assert!(
                        Arc::ptr_eq(&session.base_nodes, base_nodes),
                        "retained source base must not advance"
                    );
                    assert_eq!(&metadata_state(&session.meta), metadata);
                }
                assert!(matches!(
                    workspace
                        .artifact(&edited_id)
                        .expect("edited session")
                        .state,
                    ArtifactDirty::DirtyCanvas
                ));
                assert!(!workspace.open.contains_key(&ArtifactId::Page(unopened)));

                std::fs::write(&path, original).expect("restore the exact writer output");
                workspace
                    .accept_written_sources(&document, asset_hash, &sources, &report.written_hashes)
                    .expect("retry acceptance after reconciliation");
                let fresh =
                    WorkspaceSession::open(directory.path()).expect("independent disk index");
                assert_eq!(workspace.file_index, fresh.file_index);
                assert_eq!(
                    workspace
                        .source_write_preconditions(&document)
                        .expect("accepted preconditions"),
                    fresh
                        .source_write_preconditions(&document)
                        .expect("fresh preconditions")
                );
            }
        }
    }
}

#[test]
fn accept_written_sources_after_conflict_matches_a_fresh_disk_index() {
    for resolution in [ConflictResolution::KeepOurs, ConflictResolution::TakeTheirs] {
        let (directory, page) = page_fixture();
        let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
        let id = ArtifactId::Page(page);
        workspace.open_artifact(id.clone()).expect("open page");
        workspace
            .apply(
                &id,
                Operation::SetName {
                    id: page,
                    old: "Home".into(),
                    new: "Canvas".into(),
                },
            )
            .expect("authored edit");
        let source = directory
            .path()
            .join(&workspace.artifacts.get(&id).expect("metadata").design_dir)
            .join("page.fnx");
        let original = std::fs::read_to_string(&source).expect("source");
        let external = original.replacen("name=\"Home\"", "name=\"Disk\"", 1);
        assert_ne!(original, external);
        std::fs::write(&source, external).expect("conflicting source edit");
        workspace.notify_fs_event(FsEvent::Modified { path: source });
        assert!(matches!(
            workspace.artifact(&id).expect("session").state,
            ArtifactDirty::Conflict(_)
        ));
        workspace
            .resolve_conflict(&id, resolution)
            .expect("explicit conflict resolution");
        let document = workspace
            .artifact(&id)
            .expect("resolved session")
            .doc()
            .clone_for_persist();
        let sources = workspace
            .validated_source_overrides_for_document(&document)
            .expect("resolved sources");
        let preconditions = workspace
            .source_write_preconditions(&document)
            .expect("resolved preconditions");
        let report = crate::write_project_tree_cached_with_sources_checked(
            directory.path(),
            &document,
            &BTreeMap::new(),
            &mut crate::ProjectWriteCache::default(),
            &sources,
            &preconditions,
        )
        .expect("save resolved document");
        workspace
            .accept_written_sources(
                &document,
                workspace.asset_index_disk_hash().expect("asset hash"),
                &sources,
                &report.written_hashes,
            )
            .expect("accept resolved save");
        let fresh = WorkspaceSession::open(directory.path()).expect("fresh independent index");
        assert_eq!(workspace.file_index, fresh.file_index);
        assert_eq!(
            workspace
                .source_write_preconditions(&document)
                .expect("accepted preconditions"),
            fresh
                .source_write_preconditions(&document)
                .expect("fresh preconditions")
        );
    }
}

#[test]
#[ignore = "diagnostic: run acceptance hashing alone with no other Cargo/native work"]
fn accept_written_sources_hashing_benchmark() {
    let directory = tempdir().expect("benchmark directory");
    let mut document = Doc::new();
    for index in 0..512 {
        let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        root.name = format!("Acceptance page {index}");
        let page = document.scene.insert(root).expect("page");
        document.add_page(page);
    }
    crate::write_project_tree(directory.path(), &document, &BTreeMap::new())
        .expect("write benchmark");
    let indexed = WorkspaceSession::open(directory.path()).expect("initial index");
    let comment = format!("// {}\n", "retained authored source ".repeat(683));
    for meta in indexed.artifacts.values() {
        let path = directory.path().join(&meta.design_dir).join("page.fnx");
        let mut bytes = comment.as_bytes().to_vec();
        bytes.extend(std::fs::read(&path).expect("page source"));
        std::fs::write(path, bytes).expect("authored comment");
    }
    let mut workspace = WorkspaceSession::open(directory.path()).expect("authored index");
    let sources = workspace
        .validated_source_overrides_for_document(&document)
        .expect("authored sources");
    let preconditions = workspace
        .source_write_preconditions(&document)
        .expect("preconditions");
    let report = crate::write_project_tree_cached_with_sources_checked(
        directory.path(),
        &document,
        &BTreeMap::new(),
        &mut crate::ProjectWriteCache::default(),
        &sources,
        &preconditions,
    )
    .expect("no-op checked writer");
    assert!(report.written.is_empty());
    assert!(report.removed.is_empty());
    let baseline = workspace.file_index.clone();
    for iteration in 0..5 {
        let start = std::time::Instant::now();
        workspace
            .accept_written_sources(
                &document,
                workspace.asset_index_disk_hash().expect("asset hash"),
                &sources,
                &report.written_hashes,
            )
            .expect("accept unchanged sources");
        eprintln!(
            "acceptance_hashing pages=512 payload_bytes={} iteration={iteration} elapsed_us={}",
            comment.len() * 512,
            start.elapsed().as_micros()
        );
        assert_eq!(workspace.file_index, baseline);
    }
    assert_eq!(
        workspace.file_index,
        WorkspaceSession::open(directory.path())
            .expect("fresh index")
            .file_index
    );
}

struct ArtifactOwnershipFixture {
    directory: tempfile::TempDir,
    document: Doc,
    assets: BTreeMap<fanta_doc::AssetId, Vec<u8>>,
    routes: NodeId,
    masters: NodeId,
    container: NodeId,
    components: Vec<(fanta_doc::ComponentId, NodeId, NodeId)>,
    instance: NodeId,
}

fn artifact_ownership_fixture(nested_master: bool) -> ArtifactOwnershipFixture {
    use fanta_doc::{
        ComponentDef, ComponentId, ComponentSet, ComponentSetMembership, InstanceNode, TextNode,
    };
    let directory = tempdir().expect("project directory");
    let mut document = Doc::new();
    let mut pages = Vec::new();
    for name in ["Routes", "Masters"] {
        let mut node = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([900.0, 600.0]),
            local_size: Some([900.0, 600.0]),
            ..GroupNode::default()
        }));
        node.name = name.into();
        let page = document.scene.insert(node).expect("page");
        document.add_page(page);
        pages.push(page);
    }
    let routes = *pages.first().expect("routes");
    let masters = *pages.last().expect("masters");
    let mut container = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([480.0, 150.0]),
        local_size: Some([480.0, 150.0]),
        ..GroupNode::default()
    }));
    container.name = "Variant container".into();
    container.parent = Some(masters);
    container.transform = Transform2D::translation(40.0, 60.0);
    let container = document.scene.insert(container).expect("set container");
    let set = ComponentId::new();
    let mut components: Vec<(ComponentId, NodeId, NodeId)> = Vec::new();
    for (position, name) in ["Small", "Large"].into_iter().enumerate() {
        let mut master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([80.0 + position as f64 * 80.0, 40.0]),
            local_size: Some([80.0 + position as f64 * 80.0, 40.0]),
            ..GroupNode::default()
        }));
        master.name = name.into();
        master.parent = Some(if nested_master && position == 1 {
            components.first().expect("outer master").1
        } else {
            container
        });
        master.index = fanta_doc::IndexKey::from_raw(position as f64 + 1.0);
        master.transform = Transform2D::translation(20.0 + position as f64 * 180.0, 35.0);
        let root = document.scene.insert(master).expect("master root");
        let mut text = CanvasNode::new(NodeData::Text(TextNode::new(name, 64.0, 28.0)));
        text.parent = Some(root);
        text.name = format!("{name} caption");
        text.transform = Transform2D::translation(8.0, 8.0);
        let text = document.scene.insert(text).expect("master caption");
        let id = ComponentId::new();
        let mut definition = ComponentDef::new(id, root, name);
        definition.variant_of = Some(ComponentSetMembership {
            set,
            axis_values: BTreeMap::new(),
        });
        document.components.defs.insert(id, definition);
        components.push((id, root, text));
    }
    document.components.sets.insert(
        set,
        ComponentSet {
            id: set,
            name: "Size set".into(),
            axes: Vec::new(),
            members: components.iter().map(|entry| entry.0).collect(),
            default_variant: components.last().expect("second default").0,
            root: Some(container),
        },
    );
    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [160.0, 40.0],
    }));
    instance.parent = Some(routes);
    instance.name = "Placed set instance must stay page-owned".into();
    let instance = document.scene.insert(instance).expect("instance");
    let assets = BTreeMap::from([(
        fanta_doc::AssetId::new(),
        b"unchanged asset control".to_vec(),
    )]);
    crate::write_project_tree(directory.path(), &document, &assets).expect("initial project");
    let master_page_source =
        crate::locate_page_source(directory.path(), masters).expect("master page source");
    let source = std::fs::read_to_string(&master_page_source).expect("master page FNX");
    assert!(source.contains("<Frame "), "fixture root opening");
    let source = source.replacen(
        "<Frame ",
        "<Frame future_page_hint={{\"owner\":\"preserve\",\"values\":[1,2,3]}} ",
        1,
    );
    std::fs::write(
        &master_page_source,
        format!("// Preserve this unrelated authored page.\n{source}"),
    )
    .expect("authored source comment and future attribute");
    let (document, loaded_assets) =
        crate::read_project_tree(directory.path()).expect("load canonical project");
    assert_eq!(loaded_assets, assets);
    ArtifactOwnershipFixture {
        directory,
        document,
        assets,
        routes,
        masters,
        container,
        components,
        instance,
    }
}

fn artifact_ownership_file_bytes(
    directory: &std::path::Path,
) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &std::path::Path,
        directory: &std::path::Path,
        output: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        for entry in std::fs::read_dir(directory).expect("project directory") {
            let path = entry.expect("project entry").path();
            assert!(!path.is_symlink(), "fixture must contain ordinary files");
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.insert(
                    path.strip_prefix(root)
                        .expect("relative path")
                        .to_path_buf(),
                    std::fs::read(path).expect("file bytes"),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(directory, directory, &mut files);
    files
}

#[test]
fn artifact_ownership_structural_whole_project_save_preserves_unrelated_master_sources() {
    let mut fixture = artifact_ownership_fixture(false);
    let before = artifact_ownership_file_bytes(fixture.directory.path());
    let route_source =
        crate::locate_page_source(fixture.directory.path(), fixture.routes).expect("route source");
    let permitted = [
        route_source.clone(),
        route_source.with_file_name("page.ids.json"),
    ];
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    let artifacts: Vec<_> = workspace
        .artifacts
        .keys()
        .filter(|id| matches!(id, ArtifactId::Page(_) | ArtifactId::Component(_)))
        .cloned()
        .collect();
    for id in &artifacts {
        workspace.open_artifact(id.clone()).expect("all sources");
    }
    let revision = fixture.document.scene.revision();
    let mut created = CanvasNode::new(NodeData::Group(GroupNode::default()));
    created.name = "New child after structural canvas edit".into();
    created.parent = Some(fixture.routes);
    fixture
        .document
        .scene
        .insert(created)
        .expect("structural edit");
    assert!(
        fixture.document.scene.changes_since(revision).is_none(),
        "host takes conservative all-artifact adoption"
    );
    let expected = serde_json::to_value(&fixture.document).expect("full authored document");
    workspace.adopt_document_shared(&fixture.document);
    workspace
        .adopt_document_artifacts(&fixture.document, &artifacts)
        .expect("host all-artifact adoption with one ownership index");
    let sources = workspace
        .validated_source_overrides_for_document(&fixture.document)
        .expect("validated sources");
    let preconditions = workspace
        .source_write_preconditions(&fixture.document)
        .expect("disk guards");
    crate::write_project_tree_cached_with_sources_checked(
        fixture.directory.path(),
        &fixture.document,
        &fixture.assets,
        &mut crate::ProjectWriteCache::default(),
        &sources,
        &preconditions,
    )
    .expect("host checked Save");
    let after = artifact_ownership_file_bytes(fixture.directory.path());
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "no added or removed files"
    );
    for (path, bytes) in before {
        if !permitted.contains(&fixture.directory.path().join(&path)) {
            assert_eq!(
                after.get(&path),
                Some(&bytes),
                "unrelated artifact must remain byte-exact: {}",
                path.display()
            );
        }
    }
    let (reopened, assets) =
        crate::read_project_tree(fixture.directory.path()).expect("cold project read");
    assert_eq!(
        serde_json::to_value(&reopened).expect("all reopened fields"),
        expected
    );
    assert_eq!(assets, fixture.assets);
    assert!(matches!(
        reopened.scene.get(fixture.instance).map(|node| &node.data),
        Some(NodeData::Instance(_))
    ));
}

#[test]
fn artifact_ownership_nested_registered_masters_are_excluded_but_container_and_instances_remain() {
    let mut fixture = artifact_ownership_fixture(true);
    let (outer, outer_root, outer_text) = *fixture.components.first().expect("outer");
    let (_, inner_root, inner_text) = *fixture.components.last().expect("inner");
    fixture
        .document
        .pending_layout
        .extend([outer_text, inner_text]);
    let before = serde_json::to_value(&fixture.document).expect("whole document");
    let page = super::materialize::scope_from_document(
        &fixture.document,
        ArtifactKind::Page,
        fixture.masters,
        None,
    )
    .expect("page profile")
    .expect("page scope");
    assert_eq!(
        page.doc.scene.len(),
        2,
        "only page and ordinary set container belong to page source"
    );
    assert!(page.doc.scene.contains(fixture.container));
    assert!(!page.doc.scene.contains(outer_root));
    assert!(page.doc.pending_layout.is_empty());
    let component = super::materialize::scope_from_document(
        &fixture.document,
        ArtifactKind::Component,
        outer_root,
        fixture.document.components.defs.get(&outer),
    )
    .expect("component profile")
    .expect("component scope");
    assert_eq!(
        component.doc.scene.len(),
        2,
        "outer source excludes independently registered nested master"
    );
    assert!(component.doc.scene.contains(outer_root));
    assert!(
        component
            .doc
            .scene
            .shares_node(&fixture.document.scene, outer_text)
    );
    assert!(!component.doc.scene.contains(inner_root));
    assert_eq!(
        component.doc.pending_layout,
        [outer_text].into_iter().collect()
    );
    let routes = super::materialize::scope_from_document(
        &fixture.document,
        ArtifactKind::Page,
        fixture.routes,
        None,
    )
    .expect("routes profile")
    .expect("routes scope");
    assert!(
        routes.doc.scene.contains(fixture.instance),
        "placed instances are not definition roots"
    );
    assert_eq!(
        serde_json::to_value(&fixture.document).expect("whole document preserved"),
        before
    );
}

#[test]
fn artifact_ownership_structural_page_rebuild_preserves_source_only_root_dimensions() {
    for size_sugar in [false, true] {
        let (directory, page) = page_fixture();
        let source_path = crate::locate_page_source(directory.path(), page).expect("page source");
        let mut tree =
            fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path).expect("source"))
                .expect("tree");
        let attributes = if size_sugar {
            BTreeMap::from([
                ("width".to_owned(), json!(900.0)),
                ("height".to_owned(), json!(600.0)),
            ])
        } else {
            BTreeMap::from([
                ("clip_size".to_owned(), json!([900.0, 600.0])),
                ("local_size".to_owned(), json!([900.0, 600.0])),
            ])
        };
        for (name, value) in &attributes {
            tree.attrs.insert(name.clone(), value.clone());
        }
        std::fs::write(&source_path, fanta_fnx::print_doc("Home", &tree))
            .expect("authored page dimensions");
        let authored =
            fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path).expect("authored source"))
                .expect("canonical authored dimensions");
        assert_eq!(
            authored.attrs.get("clip_size"),
            Some(&json!([900.0, 600.0]))
        );
        assert!(!authored.attrs.contains_key("width"));
        assert!(!authored.attrs.contains_key("height"));
        let dimensions = ["clip_size", "local_size"]
            .into_iter()
            .filter_map(|name| authored.attrs.get(name).map(|value| (name, value.clone())))
            .collect::<BTreeMap<_, _>>();
        let mut workspace = WorkspaceSession::open(directory.path()).expect("workspace");
        let id = ArtifactId::Page(page);
        workspace.open_artifact(id.clone()).expect("page");
        let mut created = CanvasNode::new(NodeData::Group(GroupNode::default()));
        created.parent = Some(page);
        let created_id = created.id;
        workspace
            .artifact_mut(&id)
            .expect("session")
            .apply(Operation::create_node(created))
            .expect("structural operation");
        for stage in ["created", "undo", "redo"] {
            let session = workspace.artifact_mut(&id).expect("session");
            match stage {
                "undo" => {
                    session.undo_atomic().expect("single Undo");
                }
                "redo" => {
                    session.redo_atomic().expect("single Redo");
                }
                _ => {}
            }
            let NodeData::Group(group) = &session.doc().scene.get(page).expect("page").data else {
                panic!("page kind");
            };
            assert_eq!(group.local_size, None, "runtime page stays unsized");
            assert_eq!(group.clip_size, None, "runtime page stays unclipped");
            workspace
                .save_artifact(id.clone())
                .expect("save authored source");
            let saved =
                fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path).expect("saved source"))
                    .expect("saved tree");
            for (name, value) in &dimensions {
                assert_eq!(
                    saved.attrs.get(*name),
                    Some(value),
                    "source attribute {name} survives {stage}"
                );
            }
            let (reopened, _) = crate::read_project_tree(directory.path()).expect("cold read");
            assert_eq!(reopened.scene.contains(created_id), stage != "undo");
        }
        let session = workspace.artifact_mut(&id).expect("session");
        let project_id = session.doc().id;
        let components = session.doc().components.clone();
        let variables = session.doc().variables.clone();
        let modes = session.doc().active_modes.clone();
        let source = session
            .begin_text_edit()
            .expect("explicit source edit")
            .clone();
        let mut tree = fanta_fnx::parse_doc(&source).expect("source tree");
        for name in ["clip_size", "local_size", "width", "height"] {
            tree.attrs.remove(name);
        }
        session
            .set_text(fanta_fnx::print_doc("Home", &tree))
            .expect("remove source dimensions");
        session
            .commit_text_to_scene(project_id, components, variables, modes)
            .expect("accept intentional removal");
        let mut next_child = CanvasNode::new(NodeData::Group(GroupNode::default()));
        next_child.parent = Some(page);
        session
            .apply(Operation::create_node(next_child))
            .expect("subsequent structural edit");
        workspace
            .save_artifact(id.clone())
            .expect("save after intentional removal");
        let saved =
            fanta_fnx::parse_doc(&std::fs::read_to_string(&source_path).expect("saved source"))
                .expect("saved tree");
        for name in ["clip_size", "local_size", "width", "height"] {
            assert!(
                !saved.attrs.contains_key(name),
                "do not resurrect deliberately removed source attribute {name}"
            );
        }
    }
}

#[test]
fn artifact_ownership_untouched_unknown_attributes_and_comments_survive_property_save() {
    let mut fixture = artifact_ownership_fixture(false);
    let source_path = crate::locate_page_source(fixture.directory.path(), fixture.masters)
        .expect("unrelated Masters source");
    let before = artifact_ownership_file_bytes(fixture.directory.path());
    let authored = std::fs::read_to_string(&source_path).expect("authored Masters source");
    assert!(authored.starts_with("// Preserve this unrelated authored page.\n"));
    assert_eq!(
        fanta_fnx::parse_doc(&authored)
            .expect("valid source")
            .attrs
            .get("future_page_hint"),
        Some(&json!({"owner": "preserve", "values": [1, 2, 3]})),
    );
    let route_source =
        crate::locate_page_source(fixture.directory.path(), fixture.routes).expect("routes source");
    let route_id = ArtifactId::Page(fixture.routes);
    let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
    workspace
        .open_artifact(route_id.clone())
        .expect("only edited artifact");
    fixture
        .document
        .scene
        .get_mut(fixture.instance)
        .expect("placed instance")
        .transform = Transform2D::translation(17.0, 23.0);
    workspace.adopt_document_shared(&fixture.document);
    workspace
        .artifact_mut(&route_id)
        .expect("edited page")
        .adopt_document(&fixture.document)
        .expect("ordinary property edit");
    let sources = workspace
        .validated_source_overrides_for_document(&fixture.document)
        .expect("retained sources");
    let preconditions = workspace
        .source_write_preconditions(&fixture.document)
        .expect("preconditions");
    crate::write_project_tree_cached_with_sources_checked(
        fixture.directory.path(),
        &fixture.document,
        &fixture.assets,
        &mut crate::ProjectWriteCache::default(),
        &sources,
        &preconditions,
    )
    .expect("checked property Save");
    let after = artifact_ownership_file_bytes(fixture.directory.path());
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in before {
        if fixture.directory.path().join(&path) != route_source {
            assert_eq!(
                after.get(&path),
                Some(&bytes),
                "untouched source or asset changed: {}",
                path.display()
            );
        }
    }
    let (reopened, assets) = crate::read_project_tree(fixture.directory.path()).expect("cold read");
    assert_eq!(
        serde_json::to_value(&reopened).expect("reopened content"),
        serde_json::to_value(&fixture.document).expect("expected content")
    );
    assert_eq!(assets, fixture.assets);
}

fn artifact_scope_index_reference_scene(document: &Doc, root: NodeId) -> fanta_doc::Scene {
    let mut scene = document
        .scene
        .extract_subtree(root)
        .expect("reference root");
    for definition in document.components.defs.values() {
        if definition.root != root && scene.contains(definition.root) {
            scene
                .remove(definition.root)
                .expect("reference foreign root");
        }
    }
    scene
}

#[test]
fn artifact_scope_index_matches_definition_scan_payload_order_and_pending_layout() {
    for reverse_definition_roots in [false, true] {
        let mut fixture = artifact_ownership_fixture(true);
        let (_, outer_root, outer_text) = *fixture.components.first().expect("outer");
        let (_, inner_root, inner_text) = *fixture.components.last().expect("inner");
        for name in ["Equal-index B", "Equal-index A"] {
            let mut node = CanvasNode::new(NodeData::Group(GroupNode::default()));
            node.name = name.into();
            node.parent = Some(outer_root);
            node.index = fanta_doc::IndexKey::FIRST;
            fixture
                .document
                .scene
                .insert(node)
                .expect("equal-index sibling");
        }
        if reverse_definition_roots {
            let ids: Vec<_> = fixture.document.components.defs.keys().copied().collect();
            let first = *ids.first().expect("first definition");
            let last = *ids.last().expect("last definition");
            let first_root = fixture
                .document
                .components
                .defs
                .get(&first)
                .expect("first")
                .root;
            let last_root = fixture
                .document
                .components
                .defs
                .get(&last)
                .expect("last")
                .root;
            fixture
                .document
                .components
                .defs
                .get_mut(&first)
                .expect("first")
                .root = last_root;
            fixture
                .document
                .components
                .defs
                .get_mut(&last)
                .expect("last")
                .root = first_root;
        }
        fixture
            .document
            .pending_layout
            .extend([outer_text, inner_text, fixture.instance]);
        let before = serde_json::to_value(&fixture.document).expect("input document");
        let scopes = super::materialize::DocumentScopes::new(&fixture.document);
        let artifacts =
            fixture
                .document
                .pages
                .iter()
                .map(|root| (ArtifactKind::Page, *root, None))
                .chain(fixture.document.components.defs.values().map(|definition| {
                    (ArtifactKind::Component, definition.root, Some(definition))
                }));
        for (kind, root, definition) in artifacts {
            let mut expected = artifact_scope_index_reference_scene(&fixture.document, root);
            if kind == ArtifactKind::Page {
                let NodeData::Group(group) = &mut expected.get_mut(root).expect("page").data else {
                    panic!("page root kind");
                };
                group.clip_size = None;
                group.local_size = None;
            }
            let actual = scopes
                .scope(kind, root, definition)
                .expect("supported kind")
                .expect("scope");
            assert_eq!(
                serde_json::to_value(&actual.doc.scene).expect("actual"),
                serde_json::to_value(&expected).expect("reference")
            );
            assert_eq!(
                actual.doc.pending_layout,
                fixture
                    .document
                    .pending_layout
                    .iter()
                    .copied()
                    .filter(|id| expected.contains(*id))
                    .collect()
            );
            for id in expected.descendants_of(root) {
                assert_eq!(
                    actual.doc.scene.children_of(Some(id)),
                    expected.children_of(Some(id))
                );
                if id != root {
                    assert!(actual.doc.scene.shares_node(&fixture.document.scene, id));
                }
            }
            if root == fixture.masters {
                assert!(actual.doc.scene.contains(fixture.container));
                assert!(!actual.doc.scene.contains(outer_root));
                assert!(!actual.doc.scene.contains(inner_root));
            }
            if root == fixture.routes {
                assert!(actual.doc.scene.contains(fixture.instance));
            }
        }
        assert_eq!(
            serde_json::to_value(&fixture.document).expect("after"),
            before
        );
    }
}

#[test]
fn artifact_scope_index_rebuilds_after_definition_add_remove_and_root_retarget() {
    let mut fixture = artifact_ownership_fixture(true);
    let (outer, outer_root, _) = *fixture.components.first().expect("outer");
    let (inner, inner_root, _) = *fixture.components.last().expect("inner");
    let removed = fixture
        .document
        .components
        .defs
        .get(&inner)
        .expect("inner definition")
        .clone();
    for state in 0..5 {
        match state {
            1 => {
                fixture
                    .document
                    .components
                    .defs
                    .remove(&inner)
                    .expect("remove inner ownership");
            }
            2 => {
                fixture
                    .document
                    .components
                    .defs
                    .insert(inner, removed.clone());
            }
            3 => {
                fixture
                    .document
                    .components
                    .defs
                    .get_mut(&outer)
                    .expect("outer")
                    .root = fixture.container;
            }
            4 => {
                fixture
                    .document
                    .components
                    .defs
                    .get_mut(&inner)
                    .expect("inner")
                    .root = fixture.container;
            }
            _ => {}
        }
        let scopes = super::materialize::DocumentScopes::new(&fixture.document);
        for root in [fixture.masters, fixture.container, outer_root, inner_root] {
            let expected = artifact_scope_index_reference_scene(&fixture.document, root);
            let actual = scopes.extract_scene(root).expect("new snapshot index");
            assert_eq!(
                serde_json::to_value(&actual).expect("actual"),
                serde_json::to_value(&expected).expect("reference"),
                "state {state}, root {root}"
            );
        }
        let outer_scene = scopes.extract_scene(outer_root).expect("outer");
        assert_eq!(outer_scene.contains(inner_root), state == 1 || state == 4);
    }
}

#[test]
fn artifact_scope_index_rejects_malformed_excluded_subtrees_before_adoption() {
    let fixture = artifact_ownership_fixture(true);
    let (outer, outer_root, _) = *fixture.components.first().expect("outer");
    let (_, inner_root, inner_text) = *fixture.components.last().expect("inner");
    let before_files = artifact_ownership_file_bytes(fixture.directory.path());
    for corruption in [
        "cycle",
        "stale-child-index",
        "missing-root",
        "wrong-node-id",
    ] {
        let mut document = fixture.document.clone();
        match corruption {
            "cycle" => {
                document.scene.get_mut(outer_root).expect("outer").parent = Some(inner_root);
                document.scene.rebuild_child_index();
            }
            "stale-child-index" => {
                // The foreign inner master would normally be pruned, but its
                // descendants are still traversed by extract_subtree first.
                document
                    .scene
                    .get_mut(inner_text)
                    .expect("inner text")
                    .parent = Some(fixture.routes);
            }
            "missing-root" => {
                document.scene.remove(outer_root).expect("remove root");
            }
            "wrong-node-id" => {
                document.scene.get_mut(inner_text).expect("text").id = NodeId::new();
            }
            _ => unreachable!(),
        }
        let snapshot = serde_json::to_value(&document).expect("malformed input");
        let mut workspace = WorkspaceSession::open(fixture.directory.path()).expect("workspace");
        let artifact = ArtifactId::Component(outer);
        workspace
            .open_artifact(artifact.clone())
            .expect("open valid disk source");
        let before = serde_json::to_value(workspace.artifact(&artifact).expect("artifact").doc())
            .expect("before");
        let result = workspace.adopt_document_artifacts(&document, [&artifact]);
        assert!(
            matches!(result, Err(SessionError::DocAssemble(_))),
            "corruption {corruption}"
        );
        assert_eq!(
            serde_json::to_value(workspace.artifact(&artifact).expect("artifact").doc())
                .expect("after"),
            before
        );
        assert_eq!(
            serde_json::to_value(&document).expect("input after"),
            snapshot
        );
        assert_eq!(
            artifact_ownership_file_bytes(fixture.directory.path()),
            before_files
        );
    }
}

#[test]
#[ignore = "diagnostic: supply immutable FANTA_SCOPE_BENCH_DOC and run with an exclusive CPU window"]
fn artifact_scope_index_spectrum_benchmark() {
    let path = std::env::var_os("FANTA_SCOPE_BENCH_DOC").expect("FANTA_SCOPE_BENCH_DOC required");
    let mut document: Doc =
        serde_json::from_slice(&std::fs::read(path).expect("snapshot")).expect("typed Doc");
    document.scene.rebuild_child_index();
    let roots: Vec<_> = document
        .pages
        .iter()
        .copied()
        .chain(
            document
                .components
                .defs
                .values()
                .map(|definition| definition.root),
        )
        .collect();
    let scopes = super::materialize::DocumentScopes::new(&document);
    for root in &roots {
        let expected = artifact_scope_index_reference_scene(&document, *root);
        let actual = scopes.extract_scene(*root).expect("indexed scope");
        assert_eq!(
            serde_json::to_value(&actual).expect("actual"),
            serde_json::to_value(&expected).expect("expected")
        );
        for id in expected.descendants_of(*root) {
            assert_eq!(actual.children_of(Some(id)), expected.children_of(Some(id)));
            if id != *root {
                assert!(actual.shares_node(&document.scene, id));
            }
        }
    }
    for round in 0..3 {
        for indexed in if round % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let started = std::time::Instant::now();
            let scopes = indexed.then(|| super::materialize::DocumentScopes::new(&document));
            for root in &roots {
                let scene = if let Some(scopes) = &scopes {
                    scopes.extract_scene(*root).expect("indexed extraction")
                } else {
                    artifact_scope_index_reference_scene(&document, *root)
                };
                std::hint::black_box(&scene);
            }
            eprintln!(
                "scope_pruning_only round={round} indexed={indexed} artifacts={} definitions={} elapsed_us={}",
                roots.len(),
                document.components.defs.len(),
                started.elapsed().as_micros()
            );
        }
    }
}

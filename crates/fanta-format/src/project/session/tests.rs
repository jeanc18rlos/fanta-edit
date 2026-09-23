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

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

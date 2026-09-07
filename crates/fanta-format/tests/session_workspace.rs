//! Integration tests for the artifact-scoped WorkspaceSession.

use fanta_doc::{
    CanvasNode, ComponentDef, ComponentId, ComponentPropDef, ComponentPropKind, ComponentSet,
    ComponentSetMembership, Doc, GroupNode, InstanceNode, NodeData, Operation, Transform2D,
    VarValue, VariantAxis, resolved_component_root,
};
use fanta_format::{
    ArtifactDirty, ArtifactId, ClosePolicy, SaveResult, WorkspaceSession, write_project_tree,
};
use std::collections::BTreeMap;
use tempfile::tempdir;

fn page_doc() -> (Doc, fanta_doc::NodeId) {
    let mut doc = Doc::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    root.name = "Home".into();
    let page = doc.scene.insert(root).unwrap();
    doc.add_page(page);

    let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
    child.name = "Card".into();
    child.parent = Some(page);
    child.transform = Transform2D::translation(40.0, 40.0);
    let _ = doc.scene.insert(child).unwrap();
    (doc, page)
}

#[test]
fn open_workspace_indexes_pages_without_loading_scene() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let session = WorkspaceSession::open(dir.path()).unwrap();
    assert!(session.list_pages().contains(&ArtifactId::Page(page)));
    assert!(session.open.is_empty(), "open must not auto-load pages");
}

#[test]
fn open_page_tab_loads_scoped_scene() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    let art = session.artifact(&id).unwrap();
    assert!(matches!(art.state, ArtifactDirty::Clean));
    assert_eq!(art.root(), page);
    assert!(art.doc().scene.get(page).is_some());
    // Child present
    assert_eq!(art.doc().scene.children_of(Some(page)).len(), 1);
}

#[test]
fn render_snapshot_loads_selected_component_set_member_and_nested_master() {
    let dir = tempdir().unwrap();
    let mut doc = Doc::new();

    let mut page_node = CanvasNode::new(NodeData::Group(GroupNode::default()));
    page_node.name = "Variants".into();
    let page = doc.scene.insert(page_node).unwrap();
    doc.add_page(page);

    let nested_component = ComponentId::new();
    let mut nested_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    nested_master.name = "Icon".into();
    let nested_root = doc.scene.insert(nested_master).unwrap();
    let mut nested_child = CanvasNode::new(NodeData::Group(GroupNode::default()));
    nested_child.name = "Icon Shape".into();
    nested_child.parent = Some(nested_root);
    doc.scene.insert(nested_child).unwrap();
    doc.components.defs.insert(
        nested_component,
        ComponentDef::new(nested_component, nested_root, "Icon"),
    );

    let set = ComponentId::new();
    let axis = fanta_doc::ComponentPropId::new();
    let small_component = ComponentId::new();
    let mut small_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    small_master.name = "Size=Small".into();
    let small_root = doc.scene.insert(small_master).unwrap();
    let mut small_def = ComponentDef::new(small_component, small_root, "Size=Small");
    small_def.variant_of = Some(ComponentSetMembership {
        set,
        axis_values: BTreeMap::from([("Size".into(), "Small".into())]),
    });
    small_def.props.push(ComponentPropDef {
        id: axis,
        name: "Size".into(),
        kind: ComponentPropKind::Variant {
            axis: "Size".into(),
        },
        formatter: Default::default(),
        default: VarValue::String {
            value: "Small".into(),
        },
        bindings: Vec::new(),
    });
    doc.components.defs.insert(small_component, small_def);

    let large_component = ComponentId::new();
    let mut large_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
    large_master.name = "Size=Large".into();
    let large_root = doc.scene.insert(large_master).unwrap();
    let mut nested_instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: nested_component,
        overrides: Vec::new(),
        prop_values: BTreeMap::new(),
        derived: Vec::new(),
        local_size: [16.0, 16.0],
    }));
    nested_instance.name = "Nested Icon".into();
    nested_instance.parent = Some(large_root);
    doc.scene.insert(nested_instance).unwrap();
    let mut large_def = ComponentDef::new(large_component, large_root, "Size=Large");
    large_def.variant_of = Some(ComponentSetMembership {
        set,
        axis_values: BTreeMap::from([("Size".into(), "Large".into())]),
    });
    large_def.props.push(ComponentPropDef {
        id: axis,
        name: "Size".into(),
        kind: ComponentPropKind::Variant {
            axis: "Size".into(),
        },
        formatter: Default::default(),
        default: VarValue::String {
            value: "Large".into(),
        },
        bindings: Vec::new(),
    });
    doc.components.defs.insert(large_component, large_def);

    doc.components.sets.insert(
        set,
        ComponentSet {
            id: set,
            name: "Button".into(),
            axes: vec![VariantAxis {
                name: "Size".into(),
                values: vec!["Small".into(), "Large".into()],
            }],
            members: vec![small_component, large_component],
            default_variant: small_component,
        },
    );

    let mut page_instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: set,
        overrides: Vec::new(),
        prop_values: BTreeMap::from([(
            axis,
            VarValue::String {
                value: "Large".into(),
            },
        )]),
        derived: Vec::new(),
        local_size: [80.0, 40.0],
    }));
    page_instance.name = "Large Button".into();
    page_instance.parent = Some(page);
    let page_instance_id = doc.scene.insert(page_instance).unwrap();

    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let page_id = ArtifactId::Page(page);
    let snapshot = session.render_snapshot(&page_id).unwrap();

    let NodeData::Instance(instance) = &snapshot
        .doc
        .scene
        .get(page_instance_id)
        .expect("page instance remains in the scoped snapshot")
        .data
    else {
        panic!("page node should remain an instance");
    };
    assert_eq!(
        resolved_component_root(&snapshot.doc.components, instance),
        Some(large_root),
        "the set selection should resolve to the Large member loaded from sets.json"
    );
    assert!(snapshot.doc.components.sets.contains_key(&set));
    assert!(snapshot.doc.scene.get(large_root).is_some());
    assert!(
        snapshot.doc.scene.get(small_root).is_none(),
        "an unselected set member should not be pulled into the scoped render scene"
    );
    assert!(
        snapshot.doc.scene.get(nested_root).is_some(),
        "the selected member's nested component master should be loaded transitively"
    );
    assert!(
        snapshot
            .revision
            .dependencies
            .contains_key(&large_component)
    );
    assert!(
        snapshot
            .revision
            .dependencies
            .contains_key(&nested_component)
    );
    assert!(
        !snapshot
            .revision
            .dependencies
            .contains_key(&small_component)
    );
}

#[test]
fn canvas_op_marks_dirty_without_writing_disk() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();

    let child = *session
        .artifact(&id)
        .unwrap()
        .doc()
        .scene
        .children_of(Some(page))
        .first()
        .unwrap();
    let old_hash = session.artifact(&id).unwrap().disk_hash;

    session
        .apply(
            &id,
            Operation::SetName {
                id: child,
                old: "Card".into(),
                new: "Renamed".into(),
            },
        )
        .unwrap();

    assert!(matches!(
        session.artifact(&id).unwrap().state,
        ArtifactDirty::DirtyCanvas
    ));
    assert_eq!(
        session.artifact(&id).unwrap().disk_hash,
        old_hash,
        "disk hash unchanged until save"
    );
}

#[test]
fn save_writes_only_that_page_and_clears_dirty() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    let child = *session
        .artifact(&id)
        .unwrap()
        .doc()
        .scene
        .children_of(Some(page))
        .first()
        .unwrap();
    session
        .apply(
            &id,
            Operation::SetName {
                id: child,
                old: "Card".into(),
                new: "Renamed".into(),
            },
        )
        .unwrap();

    let result = session.save_artifact(id.clone()).unwrap();
    assert!(matches!(result, SaveResult::Wrote { .. }));
    assert!(matches!(
        session.artifact(&id).unwrap().state,
        ArtifactDirty::Clean
    ));

    // Reload via monodoc and check rename stuck.
    let (reloaded, _) = fanta_format::read_project_tree(dir.path()).unwrap();
    assert_eq!(reloaded.scene.get(child).unwrap().name, "Renamed");
}

#[test]
fn save_noop_when_hash_matches() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    // Clean save is NoOp
    assert!(matches!(
        session.save_artifact(id).unwrap(),
        SaveResult::NoOp
    ));
}

#[test]
fn hash_equal_fs_event_is_noop() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();

    // Touch the page.fnx with identical content via re-read path
    let meta = session.artifacts.get(&id).unwrap().clone();
    let path = dir.path().join(&meta.design_dir).join("page.fnx");
    let events = session.notify_fs_event(fanta_format::FsEvent::Modified { path });
    assert!(
        events.is_empty()
            || events
                .iter()
                .all(|e| !matches!(e, fanta_format::SessionEvent::Reloaded { .. })),
        "identical hash should not reload: {events:?}"
    );
}

#[test]
fn second_open_returns_same_session() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    session
        .apply(
            &id,
            Operation::SetName {
                id: page,
                old: "Home".into(),
                new: "Home2".into(),
            },
        )
        .unwrap();
    session.open_artifact(id.clone()).unwrap();
    assert!(matches!(
        session.artifact(&id).unwrap().state,
        ArtifactDirty::DirtyCanvas
    ));
}

#[test]
fn close_discard_drops_dirty() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    session
        .apply(
            &id,
            Operation::SetName {
                id: page,
                old: "Home".into(),
                new: "X".into(),
            },
        )
        .unwrap();
    session
        .close_artifact(id.clone(), ClosePolicy::Discard)
        .unwrap();
    assert!(session.artifact(&id).is_none());
}

#[test]
fn text_edit_path_marks_dirty_text() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    let art = session.artifact_mut(&id).unwrap();
    let mut text = art.begin_text_edit().unwrap().clone();
    assert!(text.contains("Home") || text.contains("Frame"));
    text = text.replacen("Home", "Landing", 1);
    art.set_text(text).unwrap();
    assert!(matches!(art.state, ArtifactDirty::DirtyText));
}

#[test]
fn concurrent_two_pages_independent_sessions() {
    let dir = tempdir().unwrap();
    let mut doc = Doc::new();
    let mut p1 = CanvasNode::new(NodeData::Group(GroupNode::default()));
    p1.name = "P1".into();
    let page1 = doc.scene.insert(p1).unwrap();
    doc.add_page(page1);
    let mut p2 = CanvasNode::new(NodeData::Group(GroupNode::default()));
    p2.name = "P2".into();
    let page2 = doc.scene.insert(p2).unwrap();
    doc.add_page(page2);
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let a = ArtifactId::Page(page1);
    let b = ArtifactId::Page(page2);
    session.open_artifact(a.clone()).unwrap();
    session.open_artifact(b.clone()).unwrap();
    session
        .apply(
            &a,
            Operation::SetName {
                id: page1,
                old: "P1".into(),
                new: "One".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        session.artifact(&a).unwrap().state,
        ArtifactDirty::DirtyCanvas
    ));
    assert!(matches!(
        session.artifact(&b).unwrap().state,
        ArtifactDirty::Clean
    ));
    session.save_artifact(a).unwrap();
    session.save_artifact(b).unwrap();
}

#[test]
fn golden_session_save_then_monodoc_read() {
    let dir = tempdir().unwrap();
    let (doc, page) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let id = ArtifactId::Page(page);
    session.open_artifact(id.clone()).unwrap();
    let child = *session
        .artifact(&id)
        .unwrap()
        .doc()
        .scene
        .children_of(Some(page))
        .first()
        .unwrap();
    session
        .apply(
            &id,
            Operation::SetName {
                id: child,
                old: "Card".into(),
                new: "Golden".into(),
            },
        )
        .unwrap();
    session.save_artifact(id).unwrap();

    let (mono, _) = fanta_format::read_project_tree(dir.path()).unwrap();
    assert_eq!(mono.scene.get(child).unwrap().name, "Golden");
    assert_eq!(mono.scene.get(page).unwrap().name, "Home");
}

#[test]
fn graphics_create_open_save_round_trip() {
    let dir = tempdir().unwrap();
    let (doc, _) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();

    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let gid = session.create_graphics_artifact("icons", "Icons").unwrap();
    session.open_artifact(gid.clone()).unwrap();
    let root = session.artifact(&gid).unwrap().root();
    session
        .apply(
            &gid,
            Operation::SetName {
                id: root,
                old: "Icons".into(),
                new: "Icon Set".into(),
            },
        )
        .unwrap();
    session.save_artifact(gid.clone()).unwrap();
    assert!(dir.path().join("graphics/icons/graphics.fnx").is_file());
    // Re-open fresh session
    let mut session2 = WorkspaceSession::open(dir.path()).unwrap();
    session2.open_artifact(gid.clone()).unwrap();
    assert_eq!(
        session2
            .artifact(&gid)
            .unwrap()
            .doc()
            .scene
            .get(root)
            .unwrap()
            .name,
        "Icon Set"
    );
}

#[test]
fn workspace_apply_var_op_marks_dirty_shared() {
    use fanta_doc::{Mode, ModeId, VariableCollection, VariableCollectionId};
    let dir = tempdir().unwrap();
    let (doc, _) = page_doc();
    write_project_tree(dir.path(), &doc, &BTreeMap::new()).unwrap();
    let mut session = WorkspaceSession::open(dir.path()).unwrap();
    let mid = ModeId::new();
    session
        .apply_workspace_op(Operation::CreateVariableCollection {
            collection: Box::new(VariableCollection {
                id: VariableCollectionId::new(),
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mid,
                    name: "Light".into(),
                }],
                default_mode: mid,
                variable_order: vec![],
            }),
        })
        .unwrap();
    assert!(matches!(
        session.shared.dirty,
        fanta_format::WorkspaceDirty::DirtyShared
    ));
    session.save_workspace_shared().unwrap();
    assert!(matches!(
        session.shared.dirty,
        fanta_format::WorkspaceDirty::Clean
    ));
}

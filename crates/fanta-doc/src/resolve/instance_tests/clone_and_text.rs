//! Clone-and-rewire basics: deep-clone a master subtree with fresh ids + def
//! paths, and apply a single sparse text override to the right descendant.

use super::*;

#[test]
fn cancelled_expansion_discards_partial_clones_and_distinguishes_a_missing_master() {
    let mut scene = Scene::new();
    let (library, component, root, _) = master(&mut scene);
    for _ in 0..256 {
        let mut child = CanvasNode::new(NodeData::Group(GroupNode::default()));
        child.parent = Some(root);
        scene.insert(child).expect("master descendant");
    }
    let instance = InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    };
    let variables = VariableRegistry::default();
    let modes = BTreeMap::new();
    let context = InstanceExpansionContext::new(&variables, &modes, root);
    let revision = scene.revision();
    let node_count = scene.descendants_of(root).count();
    for cancellation_check in [12, node_count * 2 + 12] {
        let checks = std::cell::Cell::new(0);
        let cancelled = expand_instance_with_context_cancellable(
            &scene,
            &library,
            &instance,
            &context,
            &|| {
                checks.set(checks.get() + 1);
                checks.get() == cancellation_check
            },
        );
        assert!(
            cancelled.is_none(),
            "cancellation latches even if its predicate changes back"
        );
        assert_eq!(checks.get(), cancellation_check);
        assert_eq!(scene.revision(), revision);
    }
    let completed =
        expand_instance_with_context_cancellable(&scene, &library, &instance, &context, &|| false)
            .expect("complete expansion");
    let original = expand_instance_with_context(&scene, &library, &instance, &context);
    assert_eq!(completed.len(), original.len());
    for (actual, expected) in completed.iter().zip(&original) {
        assert_eq!(actual.def_path, expected.def_path);
        assert_eq!(actual.node.data, expected.node.data);
        assert_eq!(actual.node.transform, expected.node.transform);
    }
    let missing = ComponentLibrary::new();
    assert_eq!(
        expand_instance_with_context_cancellable(&scene, &missing, &instance, &context, &|| false,),
        Some(Vec::new()),
    );
}

#[test]
fn expand_clones_subtree_with_fresh_ids_and_def_paths() {
    let mut scene = Scene::new();
    let (lib, comp_id, root_id, label_id) = master(&mut scene);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    assert_eq!(expanded.len(), 2);

    let root = expanded.iter().find(|e| e.def_path.is_empty()).unwrap();
    assert!(root.node.parent.is_none());
    assert_ne!(root.node.id, root_id, "root clone gets a fresh id");

    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    assert_eq!(
        child.node.parent,
        Some(root.node.id),
        "child reparented to clone root"
    );
    assert_ne!(child.node.id, label_id);
}

#[test]
fn expand_applies_a_text_override_to_the_right_descendant() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    let inst = InstanceNode {
        component: comp_id,
        overrides: vec![Override {
            target_path: smallvec![label_id],
            target_prop: crate::binding::BoundProp::TextContent,
            value: OverrideValue::Text {
                value: "Submit".into(),
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    match &child.node.data {
        NodeData::Text(t) => assert_eq!(t.content, "Submit"),
        other => panic!("expected text, got {other:?}"),
    }
}

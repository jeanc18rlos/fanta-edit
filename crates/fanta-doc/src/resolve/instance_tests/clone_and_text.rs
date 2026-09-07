//! Clone-and-rewire basics: deep-clone a master subtree with fresh ids + def
//! paths, and apply a single sparse text override to the right descendant.

use super::*;

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

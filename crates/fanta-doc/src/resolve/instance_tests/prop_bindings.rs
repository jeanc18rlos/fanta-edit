//! Component property → descendant binding: a Bool prop drives a child's
//! visibility, a Text prop drives a text child's content, and an explicit
//! instance override still beats a prop default for the same target.

use super::*;
use crate::binding::BoundProp;
use crate::component::{ComponentPropDef, ComponentPropKind, PropBindingTarget};
use crate::id::ComponentPropId;
use crate::node::NodeFlags;
use crate::value::VarValue;

fn label_clone(expanded: &[crate::resolve::ExpandedNode], label_id: NodeId) -> &CanvasNode {
    &expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .expect("label clone present")
        .node
}

fn instance(
    comp_id: ComponentId,
    prop_values: BTreeMap<ComponentPropId, VarValue>,
) -> InstanceNode {
    InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values,
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    }
}

#[test]
fn bool_prop_default_keeps_descendant_visible_false_hides_it() {
    let mut scene = Scene::new();
    let (mut lib, comp_id, _root, label_id) = master(&mut scene);
    let pid = ComponentPropId::new();
    lib.defs
        .get_mut(&comp_id)
        .unwrap()
        .props
        .push(ComponentPropDef {
            id: pid,
            name: "Show label".into(),
            kind: ComponentPropKind::Bool,
            formatter: Default::default(),
            default: VarValue::Boolean { value: true },
            bindings: vec![PropBindingTarget {
                path: smallvec![label_id],
                prop: BoundProp::Visible,
            }],
        });

    // Unset → default true → visible.
    let exp = expand_instance(&scene, &lib, &instance(comp_id, Default::default()));
    assert!(
        !label_clone(&exp, label_id)
            .flags
            .contains(NodeFlags::HIDDEN)
    );

    // prop_values { pid: false } → the bound descendant is hidden.
    let mut pv = BTreeMap::new();
    pv.insert(pid, VarValue::Boolean { value: false });
    let exp = expand_instance(&scene, &lib, &instance(comp_id, pv));
    assert!(
        label_clone(&exp, label_id)
            .flags
            .contains(NodeFlags::HIDDEN)
    );
}

#[test]
fn text_prop_drives_bound_content_with_default_fallback() {
    let mut scene = Scene::new();
    let (mut lib, comp_id, _root, label_id) = master(&mut scene);
    let pid = ComponentPropId::new();
    lib.defs
        .get_mut(&comp_id)
        .unwrap()
        .props
        .push(ComponentPropDef {
            id: pid,
            name: "Caption".into(),
            kind: ComponentPropKind::Text,
            formatter: Default::default(),
            default: VarValue::String {
                value: "Default".into(),
            },
            bindings: vec![PropBindingTarget {
                path: smallvec![label_id],
                prop: BoundProp::TextContent,
            }],
        });

    // Unset → the prop default drives the content.
    let exp = expand_instance(&scene, &lib, &instance(comp_id, Default::default()));
    let NodeData::Text(t) = &label_clone(&exp, label_id).data else {
        panic!("text node");
    };
    assert_eq!(t.content, "Default");

    // Set → the instance value drives the content.
    let mut pv = BTreeMap::new();
    pv.insert(
        pid,
        VarValue::String {
            value: "Hello".into(),
        },
    );
    let exp = expand_instance(&scene, &lib, &instance(comp_id, pv));
    let NodeData::Text(t) = &label_clone(&exp, label_id).data else {
        panic!("text node");
    };
    assert_eq!(t.content, "Hello");
}

#[test]
fn explicit_override_beats_prop_default() {
    let mut scene = Scene::new();
    let (mut lib, comp_id, _root, label_id) = master(&mut scene);
    let pid = ComponentPropId::new();
    lib.defs
        .get_mut(&comp_id)
        .unwrap()
        .props
        .push(ComponentPropDef {
            id: pid,
            name: "Caption".into(),
            kind: ComponentPropKind::Text,
            formatter: Default::default(),
            default: VarValue::String {
                value: "FromProp".into(),
            },
            bindings: vec![PropBindingTarget {
                path: smallvec![label_id],
                prop: BoundProp::TextContent,
            }],
        });

    // A per-instance override on the same target must win over the prop default
    // (the prop pass runs first, the override loop after).
    let inst = InstanceNode {
        component: comp_id,
        overrides: vec![Override {
            target_path: smallvec![label_id],
            target_prop: BoundProp::TextContent,
            value: OverrideValue::Text {
                value: "FromOverride".into(),
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    };
    let exp = expand_instance(&scene, &lib, &inst);
    let NodeData::Text(t) = &label_clone(&exp, label_id).data else {
        panic!("text node");
    };
    assert_eq!(t.content, "FromOverride");
}

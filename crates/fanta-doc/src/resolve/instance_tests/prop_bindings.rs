//! Component property → descendant binding: a Bool prop drives a child's
//! visibility, a Text prop drives a text child's content, and an explicit
//! instance override still beats a prop default for the same target.

use super::*;
use crate::binding::BoundProp;
use crate::component::{ComponentPropDef, ComponentPropKind, PropBindingTarget};
use crate::id::{ComponentPropId, ModeId, VariableCollectionId, VariableId};
use crate::node::{GroupNode, NodeFlags};
use crate::value::{VarValue, VariableType};
use crate::variables::{Mode, Variable, VariableCollection, VariableRegistry};

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

fn text_variable_registry() -> (
    VariableRegistry,
    VariableCollectionId,
    ModeId,
    ModeId,
    VariableId,
) {
    let collection = VariableCollectionId::new();
    let light = ModeId::new();
    let dark = ModeId::new();
    let caption = VariableId::new();
    let registry = VariableRegistry {
        collections: BTreeMap::from([(
            collection,
            VariableCollection {
                id: collection,
                name: "Theme".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "Light".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Dark".into(),
                    },
                ],
                default_mode: light,
                variable_order: vec![caption],
            },
        )]),
        variables: BTreeMap::from([(
            caption,
            Variable {
                id: caption,
                collection,
                name: "Caption".into(),
                ty: VariableType::String,
                values_by_mode: BTreeMap::from([
                    (
                        light,
                        VarValue::String {
                            value: "Light caption".into(),
                        },
                    ),
                    (
                        dark,
                        VarValue::String {
                            value: "Dark caption".into(),
                        },
                    ),
                ]),
                scopes: Vec::new(),
            },
        )]),
    };
    (registry, collection, light, dark, caption)
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

#[test]
fn alias_backed_prop_default_uses_the_placed_instances_effective_mode() {
    let mut scene = Scene::new();
    let (mut library, component, _root, label) = master(&mut scene);
    let property = ComponentPropId::new();
    let (registry, collection, light, dark, caption) = text_variable_registry();
    let Some(definition) = library.defs.get_mut(&component) else {
        panic!("component definition");
    };
    definition.props.push(ComponentPropDef {
        id: property,
        name: "Caption".into(),
        kind: ComponentPropKind::Text,
        formatter: Default::default(),
        default: VarValue::Alias { variable: caption },
        bindings: vec![PropBindingTarget {
            path: smallvec![label],
            prop: BoundProp::TextContent,
        }],
    });

    let pinned_frame = CanvasNode::new(NodeData::Group(GroupNode {
        explicit_modes: BTreeMap::from([(collection, dark)]),
        ..Default::default()
    }));
    let pinned_frame_id = pinned_frame.id;
    assert!(scene.insert(pinned_frame).is_ok());

    let placed_instance = instance(component, BTreeMap::new());
    let mut placed_node = CanvasNode::new(NodeData::Instance(placed_instance.clone()));
    placed_node.parent = Some(pinned_frame_id);
    let placed_node_id = placed_node.id;
    assert!(scene.insert(placed_node).is_ok());

    let active_modes = BTreeMap::from([(collection, light)]);
    let context = InstanceExpansionContext::new(&registry, &active_modes, placed_node_id);
    let expanded = expand_instance_with_context(&scene, &library, &placed_instance, &context);
    let NodeData::Text(text) = &label_clone(&expanded, label).data else {
        panic!("text node");
    };
    assert_eq!(text.content, "Dark caption");
}

#[test]
fn alias_backed_instance_prop_value_uses_the_active_mode() {
    let mut scene = Scene::new();
    let (mut library, component, _root, label) = master(&mut scene);
    let property = ComponentPropId::new();
    let (registry, collection, _light, dark, caption) = text_variable_registry();
    let Some(definition) = library.defs.get_mut(&component) else {
        panic!("component definition");
    };
    definition.props.push(ComponentPropDef {
        id: property,
        name: "Caption".into(),
        kind: ComponentPropKind::Text,
        formatter: Default::default(),
        default: VarValue::String {
            value: "Literal default".into(),
        },
        bindings: vec![PropBindingTarget {
            path: smallvec![label],
            prop: BoundProp::TextContent,
        }],
    });

    let placed_instance = instance(
        component,
        BTreeMap::from([(property, VarValue::Alias { variable: caption })]),
    );
    let placed_node = CanvasNode::new(NodeData::Instance(placed_instance.clone()));
    let placed_node_id = placed_node.id;
    assert!(scene.insert(placed_node).is_ok());

    let active_modes = BTreeMap::from([(collection, dark)]);
    let context = InstanceExpansionContext::new(&registry, &active_modes, placed_node_id);
    let expanded = expand_instance_with_context(&scene, &library, &placed_instance, &context);
    let NodeData::Text(text) = &label_clone(&expanded, label).data else {
        panic!("text node");
    };
    assert_eq!(text.content, "Dark caption");
}

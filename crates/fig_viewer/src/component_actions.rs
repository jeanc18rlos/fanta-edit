//! Working with components from the canvas: make an instance of a main
//! component or of a whole variant set, and find an instance's main
//! component.

use anyhow::{Context as _, Result, bail};
use fanta_doc::{
    CanvasNode, ComponentId, Doc, InstanceNode, NodeData, NodeId, Operation, Transform2D,
};

/// How far to the right of its source a new instance lands.
const INSTANCE_GAP: f64 = 40.0;

/// The component an instance of `node` uses: the variant set whose frame
/// `node` is, or the component whose master `node` is.
pub(crate) fn instantiable(doc: &Doc, node: NodeId) -> Option<ComponentId> {
    doc.components
        .sets
        .values()
        .find(|set| set.root == Some(node))
        .map(|set| set.id)
        .or_else(|| {
            doc.components
                .defs
                .values()
                .find(|def| def.root == node)
                .map(|def| def.id)
        })
}

/// The master an instance draws (for an instance of a variant set, the
/// variant it shows), when that master is in the scene.
pub(crate) fn main_component_root(doc: &Doc, instance: NodeId) -> Option<NodeId> {
    let NodeData::Instance(value) = &doc.scene.get(instance)?.data else {
        return None;
    };
    resolved_instance_root(doc, instance, value)
}

pub(crate) fn resolved_instance_root(
    doc: &Doc,
    id: NodeId,
    instance: &InstanceNode,
) -> Option<NodeId> {
    fanta_doc::resolved_component_with_context(
        &doc.scene,
        &doc.components,
        instance,
        &fanta_doc::InstanceExpansionContext::new(&doc.variables, &doc.active_modes, id),
    )
    .map(|resolved| resolved.resolved_root)
    .filter(|root| doc.scene.contains(*root))
}

/// The operations that place a new instance of the component behind `node`
/// (a master, or a variant set's frame) to its right, and the new instance.
/// An instance of a variant inside its set's frame lands beside the frame,
/// not inside it.
pub(crate) fn create_instance_operations(
    doc: &Doc,
    node: NodeId,
) -> Result<(Vec<Operation>, NodeId)> {
    let Some(component) = instantiable(doc, node) else {
        bail!("Choose a main component or a component set");
    };
    let (name, master) = match doc.components.sets.get(&component) {
        Some(set) => (
            set.name.clone(),
            doc.components
                .def(set.default_variant)
                .map(|def| def.root)
                .context("the component set has no default variant")?,
        ),
        None => {
            let def = doc.components.def(component).context("missing component")?;
            (def.name.clone(), def.root)
        }
    };
    // A variant sits in its set's frame; its instance goes beside the frame.
    let anchor = doc
        .components
        .def(component)
        .and_then(|def| def.variant_of.as_ref())
        .and_then(|membership| doc.components.sets.get(&membership.set))
        .and_then(|set| set.root)
        .filter(|frame| doc.scene.get(node).and_then(|n| n.parent) == Some(*frame))
        .unwrap_or(node);
    let bounds = doc
        .scene
        .world_bounds(anchor)
        .context("the component has no bounds")?;
    let parent = doc
        .scene
        .get(anchor)
        .and_then(|anchor| anchor.parent)
        .or_else(|| doc.active_page());
    let parent_world = parent
        .and_then(|parent| doc.scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let size = master_size(doc, master).unwrap_or([bounds.width(), bounds.height()]);
    let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: size,
    }));
    instance.name = name;
    instance.parent = parent;
    instance.index = doc.scene.next_child_index(parent);
    instance.transform = Transform2D::translation(bounds.max_x + INSTANCE_GAP, bounds.min_y)
        .then(&parent_world.inverse());
    let id = instance.id;
    Ok((
        vec![Operation::CreateInstance {
            node: Box::new(instance),
        }],
        id,
    ))
}

/// A master's box: its frame size, or its content bounds for a plain group.
pub(crate) fn master_size(doc: &Doc, root: NodeId) -> Option<[f64; 2]> {
    let node = doc.scene.get(root)?;
    if let NodeData::Group(group) = &node.data
        && let Some(size) = group.clip_size.or(group.local_size)
    {
        return Some(size);
    }
    let bounds = doc.scene.local_bounds(root)?;
    Some([bounds.width(), bounds.height()])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use fanta_doc::{ComponentDef, GroupNode};

    pub(crate) struct SetInstanceFixture {
        pub doc: Doc,
        pub instance: NodeId,
        pub parent: NodeId,
        pub roots: [NodeId; 2],
        pub texts: [NodeId; 2],
        pub members: [ComponentId; 2],
        pub set: ComponentId,
        pub property: fanta_doc::ComponentPropId,
        pub collection: fanta_doc::VariableCollectionId,
        pub modes: [fanta_doc::ModeId; 2],
        pub variable: fanta_doc::VariableId,
    }

    pub(crate) fn set_instance_fixture() -> SetInstanceFixture {
        use fanta_doc::{
            Color, ComponentPropDef, ComponentPropId, ComponentPropKind, ComponentSet,
            ComponentSetMembership, Fill, IndexKey, Mode, ModeId, TextNode, VarValue, Variable,
            VariableCollection, VariableCollectionId, VariableId, VariableType, VariantAxis,
        };
        use std::collections::BTreeMap;
        let mut doc = Doc::new();
        let page = doc
            .scene
            .insert(CanvasNode::new(NodeData::Group(GroupNode::default())))
            .expect("page");
        doc.add_page(page);
        doc.set_active_page(Some(page));
        let set = ComponentId::new();
        let members = [ComponentId::new(), ComponentId::new()];
        let property = ComponentPropId::new();
        let mut roots = Vec::new();
        let mut texts = Vec::new();
        for (index, (label, size)) in [("Small", [80.0, 40.0]), ("Large", [160.0, 80.0])]
            .into_iter()
            .enumerate()
        {
            let mut root = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some(size),
                background: Some(Fill::solid(Color::WHITE)),
                ..Default::default()
            }));
            root.name = label.into();
            root.parent = Some(page);
            root.index = doc.scene.next_child_index(Some(page));
            root.transform = Transform2D::translation(1000.0 + index as f64 * 200.0, 0.0);
            let root = doc.scene.insert(root).expect("master");
            roots.push(root);
            let mut text = CanvasNode::new(NodeData::Text(TextNode::new(label, 60.0, 24.0)));
            text.parent = Some(root);
            text.transform = Transform2D::translation(10.0, 8.0);
            texts.push(doc.scene.insert(text).expect("master text"));
            let mut definition = ComponentDef::new(members[index], root, label);
            definition.variant_of = Some(ComponentSetMembership {
                set,
                axis_values: BTreeMap::from([("Size".into(), label.into())]),
            });
            definition.props.push(ComponentPropDef {
                id: property,
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
            doc.components.defs.insert(members[index], definition);
        }
        doc.components.sets.insert(
            set,
            ComponentSet {
                id: set,
                name: "Menu variants".into(),
                axes: vec![VariantAxis {
                    name: "Size".into(),
                    values: vec!["Small".into(), "Large".into()],
                }],
                members: members.to_vec(),
                default_variant: members[1],
                root: None,
            },
        );
        let collection = VariableCollectionId::new();
        let modes = [ModeId::new(), ModeId::new()];
        let variable = VariableId::new();
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Variant modes".into(),
                modes: modes
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| Mode {
                        id,
                        name: format!("Mode {index}"),
                    })
                    .collect(),
                default_mode: modes[0],
                variable_order: vec![variable],
            },
        );
        doc.variables.variables.insert(
            variable,
            Variable {
                id: variable,
                collection,
                name: "Size alias".into(),
                ty: VariableType::String,
                values_by_mode: BTreeMap::from([
                    (
                        modes[0],
                        VarValue::String {
                            value: "Small".into(),
                        },
                    ),
                    (
                        modes[1],
                        VarValue::String {
                            value: "Large".into(),
                        },
                    ),
                ]),
                scopes: Vec::new(),
            },
        );
        doc.active_modes.insert(collection, modes[1]);
        let mut parent = CanvasNode::new(NodeData::Group(GroupNode::default()));
        parent.parent = Some(page);
        parent.index = doc.scene.next_child_index(Some(page));
        let parent = doc.scene.insert(parent).expect("placed parent");
        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component: set,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [160.0, 80.0],
        }));
        instance.parent = Some(parent);
        instance.index = IndexKey::FIRST;
        let instance = doc.scene.insert(instance).expect("instance");
        doc.selection.select_only(instance);
        doc.history = Default::default();
        SetInstanceFixture {
            doc,
            instance,
            parent,
            roots: roots.try_into().expect("two roots"),
            texts: texts.try_into().expect("two texts"),
            members,
            set,
            property,
            collection,
            modes,
            variable,
        }
    }

    #[test]
    fn variant_instance_main_uses_default_literal_alias_and_placed_mode() {
        use fanta_doc::VarValue;
        let mut fixture = set_instance_fixture();
        assert_eq!(
            main_component_root(&fixture.doc, fixture.instance),
            Some(fixture.roots[1]),
            "default is not the first member"
        );
        for (value, active, pin, expected) in [
            (
                VarValue::String {
                    value: "Small".into(),
                },
                1,
                None,
                0,
            ),
            (
                VarValue::Alias {
                    variable: fixture.variable,
                },
                1,
                None,
                1,
            ),
            (
                VarValue::Alias {
                    variable: fixture.variable,
                },
                1,
                Some(0),
                0,
            ),
            (
                VarValue::Alias {
                    variable: fixture.variable,
                },
                0,
                Some(1),
                1,
            ),
        ] {
            let NodeData::Instance(instance) = &mut fixture
                .doc
                .scene
                .get_mut(fixture.instance)
                .expect("instance")
                .data
            else {
                panic!("instance")
            };
            instance.prop_values.insert(fixture.property, value);
            fixture
                .doc
                .active_modes
                .insert(fixture.collection, fixture.modes[active]);
            let NodeData::Group(parent) = &mut fixture
                .doc
                .scene
                .get_mut(fixture.parent)
                .expect("parent")
                .data
            else {
                panic!("parent")
            };
            parent.explicit_modes.clear();
            if let Some(pin) = pin {
                parent
                    .explicit_modes
                    .insert(fixture.collection, fixture.modes[pin]);
            }
            assert_eq!(
                main_component_root(&fixture.doc, fixture.instance),
                Some(fixture.roots[expected])
            );
        }
    }

    /// A page with two 100×40 masters named after their values.
    fn doc_with_masters() -> (Doc, NodeId, Vec<NodeId>) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut roots = Vec::new();
        for (index, name) in ["Size=S", "Size=L"].into_iter().enumerate() {
            let mut root = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([100.0, 40.0]),
                ..Default::default()
            }));
            root.name = name.into();
            root.parent = Some(page_id);
            root.index = doc.scene.next_child_index(Some(page_id));
            root.transform = Transform2D::translation(index as f64 * 150.0, 0.0);
            let root_id = root.id;
            doc.apply(Operation::create_node(root)).unwrap();
            let component = ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(ComponentDef::new(component, root_id, name)),
            })
            .unwrap();
            roots.push(root_id);
        }
        (doc, page_id, roots)
    }

    fn instance_of(doc: &Doc, id: NodeId) -> &InstanceNode {
        match &doc.scene.get(id).unwrap().data {
            NodeData::Instance(instance) => instance,
            _ => panic!("expected an instance"),
        }
    }

    #[test]
    fn an_instance_of_a_main_component_lands_beside_it() {
        let (mut doc, page, roots) = doc_with_masters();
        let (operations, instance) = create_instance_operations(&doc, roots[0]).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        let node = doc.scene.get(instance).unwrap();
        assert_eq!(node.parent, Some(page));
        assert_eq!(node.name, "Size=S");
        let bounds = doc.scene.world_bounds(instance).unwrap();
        assert_eq!((bounds.min_x, bounds.min_y), (140.0, 0.0));
        assert_eq!(instance_of(&doc, instance).local_size, [100.0, 40.0]);
        assert_eq!(main_component_root(&doc, instance), Some(roots[0]));
        // Only components can be instanced.
        assert!(create_instance_operations(&doc, page).is_err());
        assert!(create_instance_operations(&doc, instance).is_err());
    }

    #[test]
    fn an_instance_of_a_set_shows_its_default_and_lands_beside_the_set_frame() {
        let (mut doc, page, roots) = doc_with_masters();
        let edit = crate::variant_sets::combine_variants(&doc, &roots, Some("Chip")).unwrap();
        for operation in edit.operations {
            doc.apply(operation).unwrap();
        }
        let frame = edit.frame.unwrap();
        fanta_doc::solve_auto_layout(&mut doc.scene, frame, &mut |_| (0.0, 0.0));
        let frame_bounds = doc.scene.world_bounds(frame).unwrap();

        // From the set's frame: an instance of the whole set.
        let (operations, of_set) = create_instance_operations(&doc, frame).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        assert_eq!(instance_of(&doc, of_set).component, edit.set);
        assert_eq!(doc.scene.get(of_set).unwrap().name, "Chip");
        assert_eq!(doc.scene.get(of_set).unwrap().parent, Some(page));
        assert_eq!(
            main_component_root(&doc, of_set),
            Some(roots[0]),
            "Go to main component lands on the variant it shows"
        );

        // From a variant inside the frame: an instance of that variant,
        // beside the frame rather than inside it.
        let (operations, of_variant) = create_instance_operations(&doc, roots[1]).unwrap();
        for operation in operations {
            doc.apply(operation).unwrap();
        }
        assert_eq!(doc.scene.get(of_variant).unwrap().parent, Some(page));
        let bounds = doc.scene.world_bounds(of_variant).unwrap();
        assert_eq!(bounds.min_x, frame_bounds.max_x + INSTANCE_GAP);
        assert_eq!(main_component_root(&doc, of_variant), Some(roots[1]));
    }
}

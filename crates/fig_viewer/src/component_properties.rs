use fanta_doc::{
    BoundProp, CanvasNode, ComponentId, ComponentPropDef, ComponentPropFormatter, ComponentPropId,
    ComponentPropKind, Doc, NodeData, NodeId, Operation, PropBindingTarget, VarValue,
};

/// The property kinds offered by the component-master authoring menu.
///
/// `Variant` is intentionally represented without an axis here. The builder
/// chooses the first axis in the owning component set that is not already
/// exposed, keeping the UI choice independent from document topology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateComponentPropertyKind {
    Text,
    Boolean,
    Number,
    Color,
    InstanceSwap,
    Variant,
}

impl CreateComponentPropertyKind {
    pub(crate) const ALL: [Self; 6] = [
        Self::Text,
        Self::Boolean,
        Self::Number,
        Self::Color,
        Self::InstanceSwap,
        Self::Variant,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Boolean => "Boolean",
            Self::Number => "Number",
            Self::Color => "Color",
            Self::InstanceSwap => "Instance swap",
            Self::Variant => "Variant",
        }
    }

    fn component_kind(self) -> Option<ComponentPropKind> {
        match self {
            Self::Text => Some(ComponentPropKind::Text),
            Self::Boolean => Some(ComponentPropKind::Bool),
            Self::Number => Some(ComponentPropKind::Number),
            Self::Color => Some(ComponentPropKind::Color),
            Self::InstanceSwap => Some(ComponentPropKind::InstanceSwap),
            Self::Variant => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ComponentBindingCandidate {
    pub(crate) prop: BoundProp,
    pub(crate) label: String,
}

/// Build the schema replacement needed to append one component property.
///
/// Variant properties are authored across every member of their component set
/// with one shared property id. The resolver uses that shared id to select a
/// member, while each member keeps its own axis value as the default.
pub(crate) fn create_component_property_operations(
    doc: &Doc,
    component: ComponentId,
    requested_kind: CreateComponentPropertyKind,
) -> Vec<Operation> {
    let Some(definition) = doc.components.def(component) else {
        return Vec::new();
    };

    if requested_kind == CreateComponentPropertyKind::Variant {
        return create_variant_property_operations(doc, component);
    }

    let Some(kind) = requested_kind.component_kind() else {
        return Vec::new();
    };
    let old = definition.props.clone();
    let mut new = old.clone();
    new.push(ComponentPropDef {
        id: ComponentPropId::new(),
        name: unique_property_name(&old, requested_kind.label()),
        default: kind.default_value(),
        kind,
        formatter: ComponentPropFormatter::default(),
        bindings: Vec::new(),
    });
    vec![Operation::SetComponentProps {
        component,
        old,
        new,
    }]
}

/// Whether the selected component has another set axis that can be exposed as
/// a variant property. Standalone components have no representable variant
/// selector in the current engine and therefore return `false`.
pub(crate) fn can_create_variant_property(doc: &Doc, component: ComponentId) -> bool {
    next_variant_axis(doc, component).is_some()
}

fn create_variant_property_operations(doc: &Doc, component: ComponentId) -> Vec<Operation> {
    let Some((set_id, axis)) = next_variant_axis(doc, component) else {
        return Vec::new();
    };
    let Some(set) = doc.components.sets.get(&set_id) else {
        return Vec::new();
    };
    let property_id = set
        .members
        .iter()
        .filter_map(|member| doc.components.def(*member))
        .flat_map(|definition| &definition.props)
        .find_map(|property| match &property.kind {
            ComponentPropKind::Variant {
                axis: existing_axis,
            } if existing_axis == &axis => Some(property.id),
            _ => None,
        })
        .unwrap_or_else(ComponentPropId::new);

    set.members
        .iter()
        .filter_map(|member| {
            let definition = doc.components.def(*member)?;
            if definition.props.iter().any(|property| {
                matches!(
                    &property.kind,
                    ComponentPropKind::Variant { axis: existing_axis }
                        if existing_axis == &axis
                )
            }) {
                return None;
            }
            let old = definition.props.clone();
            let mut new = old.clone();
            let default = definition
                .variant_of
                .as_ref()
                .and_then(|membership| membership.axis_values.get(&axis))
                .cloned()
                .unwrap_or_default();
            new.push(ComponentPropDef {
                id: property_id,
                name: unique_property_name(&old, &axis),
                kind: ComponentPropKind::Variant { axis: axis.clone() },
                formatter: ComponentPropFormatter::default(),
                default: VarValue::String { value: default },
                bindings: Vec::new(),
            });
            Some(Operation::SetComponentProps {
                component: *member,
                old,
                new,
            })
        })
        .collect()
}

fn next_variant_axis(doc: &Doc, component: ComponentId) -> Option<(ComponentId, String)> {
    let definition = doc.components.def(component)?;
    let membership = definition.variant_of.as_ref()?;
    let set = doc.components.sets.get(&membership.set)?;
    let axis = set.axes.iter().find(|axis| {
        !definition.props.iter().any(|property| {
            matches!(
                &property.kind,
                ComponentPropKind::Variant { axis: existing_axis }
                    if existing_axis == &axis.name
            )
        })
    })?;
    Some((set.id, axis.name.clone()))
}

fn unique_property_name(properties: &[ComponentPropDef], base: &str) -> String {
    if properties.iter().all(|property| property.name != base) {
        return base.to_string();
    }
    for suffix in 2..=properties.len() + 2 {
        let candidate = format!("{base} {suffix}");
        if properties.iter().all(|property| property.name != candidate) {
            return candidate;
        }
    }
    base.to_string()
}

/// Fields on one selected master descendant that can be driven by a component
/// property in the current document model.
pub(crate) fn component_binding_candidates(node: &CanvasNode) -> Vec<ComponentBindingCandidate> {
    let mut candidates = vec![
        ComponentBindingCandidate {
            prop: BoundProp::Visible,
            label: "Visibility".to_string(),
        },
        ComponentBindingCandidate {
            prop: BoundProp::Opacity,
            label: "Opacity".to_string(),
        },
    ];

    let (fill_count, stroke_count) = match &node.data {
        NodeData::Vector(vector) => (vector.fills.len(), vector.strokes.len()),
        NodeData::Group(group) => (usize::from(group.background.is_some()), group.strokes.len()),
        NodeData::Text(_) => (1, 0),
        _ => (0, 0),
    };
    for index in 0..fill_count.min(usize::from(u16::MAX) + 1) {
        let display_index = index + 1;
        let index = index as u16;
        candidates.push(ComponentBindingCandidate {
            prop: BoundProp::FillColor { index },
            label: format!("Fill {display_index} color"),
        });
    }
    for index in 0..stroke_count.min(usize::from(u16::MAX) + 1) {
        let display_index = index + 1;
        let index = index as u16;
        candidates.push(ComponentBindingCandidate {
            prop: BoundProp::StrokeColor { index },
            label: format!("Stroke {display_index} color"),
        });
        candidates.push(ComponentBindingCandidate {
            prop: BoundProp::StrokeWidth { index },
            label: format!("Stroke {display_index} width"),
        });
    }

    let scalar_candidates = [
        (BoundProp::CornerRadius, "Corner radius"),
        (BoundProp::TextContent, "Text"),
        (BoundProp::ClipWidth, "Width"),
        (BoundProp::ClipHeight, "Height"),
    ];
    candidates.extend(
        scalar_candidates
            .into_iter()
            .filter(|(property, _)| property.applies_to(node))
            .map(|(prop, label)| ComponentBindingCandidate {
                prop,
                label: label.to_string(),
            }),
    );
    candidates
}

pub(crate) fn component_property_accepts_binding(
    kind: &ComponentPropKind,
    target: BoundProp,
) -> bool {
    match kind {
        ComponentPropKind::Bool => matches!(target, BoundProp::Visible),
        ComponentPropKind::Text => matches!(target, BoundProp::TextContent),
        ComponentPropKind::Number => matches!(
            target,
            BoundProp::StrokeWidth { .. }
                | BoundProp::CornerRadius
                | BoundProp::Opacity
                | BoundProp::ClipWidth
                | BoundProp::ClipHeight
        ),
        ComponentPropKind::Color => matches!(
            target,
            BoundProp::FillColor { .. } | BoundProp::StrokeColor { .. }
        ),
        ComponentPropKind::InstanceSwap | ComponentPropKind::Variant { .. } => false,
    }
}

/// Bind (or unbind) one selected descendant field. A target is owned by at
/// most one component property: selecting a new property removes a stale
/// binding to the same target from every other property first.
pub(crate) fn set_component_property_binding_operations(
    doc: &Doc,
    component: ComponentId,
    target_node: NodeId,
    target_prop: BoundProp,
    property: Option<ComponentPropId>,
) -> Vec<Operation> {
    let Some(definition) = doc.components.def(component) else {
        return Vec::new();
    };
    if target_node == definition.root
        || !doc
            .scene
            .ancestors_of(target_node)
            .any(|ancestor| ancestor.id == definition.root)
    {
        return Vec::new();
    }
    let Some(node) = doc.scene.get(target_node) else {
        return Vec::new();
    };
    if !target_prop.applies_to(node) {
        return Vec::new();
    }
    if property.is_some_and(|property_id| {
        !definition.props.iter().any(|definition| {
            definition.id == property_id
                && component_property_accepts_binding(&definition.kind, target_prop)
        })
    }) {
        return Vec::new();
    }

    let path = fanta_doc::def_local_path(&doc.scene, definition.root, target_node);
    let target = PropBindingTarget {
        path,
        prop: target_prop,
    };
    let old = definition.props.clone();
    let mut new = old.clone();
    for definition in &mut new {
        definition.bindings.retain(|binding| binding != &target);
    }
    if let Some(property_id) = property
        && let Some(definition) = new
            .iter_mut()
            .find(|definition| definition.id == property_id)
    {
        definition.bindings.push(target);
    }
    if new == old {
        return Vec::new();
    }
    vec![Operation::SetComponentProps {
        component,
        old,
        new,
    }]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fanta_doc::{
        Color, ComponentDef, ComponentSet, ComponentSetMembership, GroupNode, NodeData, TextNode,
        VariantAxis,
    };

    use super::*;

    fn master_with_text() -> (Doc, ComponentId, NodeId, NodeId) {
        let mut doc = Doc::new();
        let root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        doc.scene.insert(root).expect("insert component root");
        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Label", 100.0, 20.0)));
        text.parent = Some(root_id);
        let text_id = text.id;
        doc.scene.insert(text).expect("insert component text");
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Button"));
        (doc, component, root_id, text_id)
    }

    fn replacement(operations: &[Operation]) -> &[ComponentPropDef] {
        match operations.first() {
            Some(Operation::SetComponentProps { new, .. }) => new,
            _ => panic!("expected a component schema replacement"),
        }
    }

    #[test]
    fn creates_every_literal_component_property_kind_with_typed_defaults() {
        let (doc, component, _, _) = master_with_text();
        let cases = [
            (
                CreateComponentPropertyKind::Text,
                ComponentPropKind::Text,
                VarValue::String {
                    value: String::new(),
                },
            ),
            (
                CreateComponentPropertyKind::Boolean,
                ComponentPropKind::Bool,
                VarValue::Boolean { value: false },
            ),
            (
                CreateComponentPropertyKind::Number,
                ComponentPropKind::Number,
                VarValue::Float { value: 0.0 },
            ),
            (
                CreateComponentPropertyKind::Color,
                ComponentPropKind::Color,
                VarValue::Color {
                    value: Color::rgb(153, 153, 153),
                },
            ),
            (
                CreateComponentPropertyKind::InstanceSwap,
                ComponentPropKind::InstanceSwap,
                VarValue::String {
                    value: String::new(),
                },
            ),
        ];

        for (requested, expected_kind, expected_default) in cases {
            let operations = create_component_property_operations(&doc, component, requested);
            assert_eq!(operations.len(), 1);
            let property = replacement(&operations)
                .last()
                .expect("new component property");
            assert_eq!(property.kind, expected_kind);
            assert_eq!(property.default, expected_default);
            assert!(property.bindings.is_empty());
        }
    }

    #[test]
    fn variant_property_is_shared_across_the_set_with_member_defaults() {
        let (mut doc, first, _, _) = master_with_text();
        let second_root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let second_root_id = second_root.id;
        doc.scene
            .insert(second_root)
            .expect("insert second component root");
        let second = ComponentId::new();
        let set_id = ComponentId::new();
        doc.components.defs.insert(
            second,
            ComponentDef::new(second, second_root_id, "Button / Hover"),
        );
        for (member, value) in [(first, "Default"), (second, "Hover")] {
            doc.components
                .defs
                .get_mut(&member)
                .expect("component member")
                .variant_of = Some(ComponentSetMembership {
                set: set_id,
                axis_values: BTreeMap::from([("State".to_string(), value.to_string())]),
            });
        }
        doc.components.sets.insert(
            set_id,
            ComponentSet {
                id: set_id,
                name: "Button".to_string(),
                axes: vec![VariantAxis {
                    name: "State".to_string(),
                    values: vec!["Default".to_string(), "Hover".to_string()],
                }],
                members: vec![first, second],
                default_variant: first,
            },
        );

        let operations =
            create_component_property_operations(&doc, first, CreateComponentPropertyKind::Variant);
        assert_eq!(operations.len(), 2);
        let first_property = replacement(&operations[..1])
            .last()
            .expect("first variant property");
        let second_property = replacement(&operations[1..])
            .last()
            .expect("second variant property");
        assert_eq!(first_property.id, second_property.id);
        assert_eq!(
            first_property.default,
            VarValue::String {
                value: "Default".to_string()
            }
        );
        assert_eq!(
            second_property.default,
            VarValue::String {
                value: "Hover".to_string()
            }
        );
    }

    #[test]
    fn binding_builder_accepts_only_compatible_descendant_fields() {
        let (mut doc, component, root, text) = master_with_text();
        let text_property = ComponentPropId::new();
        let color_property = ComponentPropId::new();
        let definition = doc
            .components
            .defs
            .get_mut(&component)
            .expect("component definition");
        definition.props = vec![
            ComponentPropDef {
                id: text_property,
                name: "Label".to_string(),
                kind: ComponentPropKind::Text,
                formatter: ComponentPropFormatter::default(),
                default: VarValue::String {
                    value: "Label".to_string(),
                },
                bindings: Vec::new(),
            },
            ComponentPropDef {
                id: color_property,
                name: "Color".to_string(),
                kind: ComponentPropKind::Color,
                formatter: ComponentPropFormatter::default(),
                default: VarValue::Color {
                    value: Color::BLACK,
                },
                bindings: Vec::new(),
            },
        ];

        let operations = set_component_property_binding_operations(
            &doc,
            component,
            text,
            BoundProp::TextContent,
            Some(text_property),
        );
        let properties = replacement(&operations);
        assert_eq!(properties[0].bindings.len(), 1);
        assert_eq!(properties[0].bindings[0].path.as_slice(), &[text]);
        assert_eq!(properties[0].bindings[0].prop, BoundProp::TextContent);

        assert!(
            set_component_property_binding_operations(
                &doc,
                component,
                text,
                BoundProp::TextContent,
                Some(color_property),
            )
            .is_empty()
        );
        assert!(
            set_component_property_binding_operations(
                &doc,
                component,
                root,
                BoundProp::Visible,
                Some(text_property),
            )
            .is_empty()
        );
    }

    #[test]
    fn rebinding_a_target_removes_its_previous_owner_and_unbinds_cleanly() {
        let (mut doc, component, _, text) = master_with_text();
        let first = ComponentPropId::new();
        let second = ComponentPropId::new();
        let target = PropBindingTarget {
            path: fanta_doc::OverridePath::from_slice(&[text]),
            prop: BoundProp::TextContent,
        };
        doc.components
            .defs
            .get_mut(&component)
            .expect("component definition")
            .props = vec![
            ComponentPropDef {
                id: first,
                name: "First".to_string(),
                kind: ComponentPropKind::Text,
                formatter: ComponentPropFormatter::default(),
                default: VarValue::String {
                    value: String::new(),
                },
                bindings: vec![target.clone()],
            },
            ComponentPropDef {
                id: second,
                name: "Second".to_string(),
                kind: ComponentPropKind::Text,
                formatter: ComponentPropFormatter::default(),
                default: VarValue::String {
                    value: String::new(),
                },
                bindings: Vec::new(),
            },
        ];

        let rebound = set_component_property_binding_operations(
            &doc,
            component,
            text,
            BoundProp::TextContent,
            Some(second),
        );
        let properties = replacement(&rebound);
        assert!(properties[0].bindings.is_empty());
        assert_eq!(properties[1].bindings, vec![target]);

        let mut rebound_doc = doc;
        rebound_doc
            .components
            .defs
            .get_mut(&component)
            .expect("def")
            .props = properties.to_vec();
        let unbound = set_component_property_binding_operations(
            &rebound_doc,
            component,
            text,
            BoundProp::TextContent,
            None,
        );
        assert!(
            replacement(&unbound)
                .iter()
                .all(|property| property.bindings.is_empty())
        );
    }

    #[test]
    fn candidate_fields_match_the_selected_node_shape() {
        let text = CanvasNode::new(NodeData::Text(TextNode::new("Label", 100.0, 20.0)));
        let properties: Vec<BoundProp> = component_binding_candidates(&text)
            .into_iter()
            .map(|candidate| candidate.prop)
            .collect();
        assert!(properties.contains(&BoundProp::TextContent));
        assert!(properties.contains(&BoundProp::FillColor { index: 0 }));
        assert!(properties.contains(&BoundProp::Opacity));
        assert!(!properties.contains(&BoundProp::CornerRadius));
    }
}

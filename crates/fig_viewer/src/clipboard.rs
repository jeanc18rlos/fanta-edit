//! Clipboard edits that need the editor's layer actions. The clipboard model
//! and its operations live in `fanta_headless::clipboard`, which the cloud
//! engine shares.

pub(crate) use fanta_headless::clipboard::*;

use anyhow::{Context as _, Result};
use fanta_doc::{Doc, IndexKey, NodeId, Operation, Transform2D};

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn paste_to_replace_operations(
    doc: &Doc,
    id: NodeId,
    payload: &CanvasClipboard,
) -> Result<Vec<Operation>> {
    let targets = crate::layer_context_ops::targets(doc, id);
    let mut scratch = doc.clone();
    let mut operations = Vec::new();
    for id in targets {
        anyhow::ensure!(
            crate::layer_context_ops::editable(&scratch, id),
            "A selected layer is locked"
        );
        let target = scratch
            .scene
            .get(id)
            .context("Missing replacement target")?
            .clone();
        let bounds = scratch
            .scene
            .world_bounds(id)
            .context("The replacement target has no bounds")?;
        scratch.selection.replace_with([id]);
        let mut pasted = payload.instantiate(&scratch, 0., ClipboardPlacement::Paste)?;
        let mut preview = scratch.clone();
        let mut preview_nodes = pasted.clone();
        for operation in replacement_component_operations(&scratch, payload, &mut preview_nodes)? {
            preview.apply(operation)?;
        }
        let source = pasted
            .roots
            .iter()
            .filter_map(|id| preview.scene.world_bounds(*id))
            .reduce(|a, b| a.union(&b))
            .context("The clipboard has no geometry")?;
        let offset =
            Transform2D::translation(bounds.min_x - source.min_x, bounds.min_y - source.min_y);
        let parent = target
            .parent
            .and_then(|id| scratch.scene.world_transform(id))
            .unwrap_or(Transform2D::IDENTITY);
        let next = scratch
            .scene
            .children_of(target.parent)
            .iter()
            .filter_map(|id| scratch.scene.get(*id))
            .find(|node| node.index > target.index)
            .map(|node| node.index);
        let mut index = target.index;
        for node in &mut pasted.nodes {
            if !pasted.roots.contains(&node.id) {
                continue;
            }
            node.transform = preview
                .scene
                .world_transform(node.id)
                .context("Missing clipboard transform")?
                .then(&offset)
                .then(&parent.inverse());
            anyhow::ensure!(
                node.transform.is_finite(),
                "The target parent transform is singular"
            );
            node.parent = target.parent;
            node.index = index;
            index = if let Some(next) = next {
                anyhow::ensure!(
                    !IndexKey::near_precision_limit(index, next),
                    "The stacking order needs rebalancing"
                );
                IndexKey::between(index, next)
            } else {
                IndexKey::after(index)
            };
        }
        let edits = crate::layer_context_ops::delete_layers(&scratch, id)?;
        for operation in &edits {
            scratch.apply(operation.clone())?;
        }
        let replacements = replacement_component_operations(&scratch, payload, &mut pasted)?;
        for operation in &replacements {
            scratch.apply(operation.clone())?;
        }
        operations.extend(edits);
        operations.extend(replacements);
    }
    Ok(operations)
}

#[cfg(feature = "fanta-gpui-ui")]
pub(crate) fn duplicate_layer_operations(doc: &Doc, id: NodeId) -> Result<Vec<Operation>> {
    let targets = crate::layer_context_ops::targets(doc, id);
    for target in &targets {
        anyhow::ensure!(
            crate::layer_context_ops::editable(doc, *target),
            "A selected layer is locked"
        );
    }
    let clipboard = CanvasClipboard::capture_roots(doc, targets).context("Nothing to duplicate")?;
    let mut pasted = clipboard.instantiate(doc, 16., ClipboardPlacement::Duplicate)?;
    clone_component_operations(doc, &clipboard, &mut pasted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{
        Action, CanvasNode, Color, ComponentDef, ComponentId, GroupNode, NodeData, TextNode,
        VectorNode,
    };

    fn subtree_doc() -> (Doc, NodeId, NodeId, NodeId) {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).unwrap();
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));

        let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([200.0, 120.0]),
            ..Default::default()
        }));
        frame.parent = Some(page_id);
        frame.transform = Transform2D::translation(20.0, 30.0);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).unwrap();

        let mut first = CanvasNode::new(NodeData::Text(TextNode::new("First", 80.0, 24.0)));
        first.parent = Some(frame_id);
        first.index = IndexKey::from_raw(1.0);
        first.transform = Transform2D::translation(8.0, 12.0);
        let first_id = first.id;
        doc.apply(Operation::create_node(first)).unwrap();

        let mut second = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            20.0,
            20.0,
            Color::WHITE,
        )));
        second.parent = Some(frame_id);
        second.index = IndexKey::from_raw(2.0);
        let second_id = second.id;
        doc.apply(Operation::create_node(second)).unwrap();
        doc.history = Default::default();
        (doc, frame_id, first_id, second_id)
    }

    #[test]
    fn duplicating_a_variant_set_frame_makes_a_set_framed_by_the_copy() {
        let mut doc = Doc::new();
        let page = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode::default()));
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut roots = Vec::new();
        for name in ["Size=S", "Size=L"] {
            let mut master = CanvasNode::new(NodeData::Group(fanta_doc::GroupNode {
                clip_size: Some([40.0, 20.0]),
                ..Default::default()
            }));
            master.name = name.into();
            master.parent = Some(page_id);
            master.index = doc.scene.next_child_index(Some(page_id));
            let root = master.id;
            doc.apply(Operation::create_node(master)).expect("master");
            let component = fanta_doc::ComponentId::new();
            doc.apply(Operation::DefineComponent {
                def: Box::new(fanta_doc::ComponentDef::new(component, root, name)),
            })
            .expect("component");
            roots.push(root);
        }
        let edit = crate::variant_sets::combine_variants(&doc, &roots, Some("Chip")).expect("set");
        for operation in edit.operations {
            doc.apply(operation).expect("combine");
        }
        let frame = edit.frame.expect("frame");

        for operation in duplicate_layer_operations(&doc, frame).expect("duplicate") {
            doc.apply(operation).expect("apply duplicate");
        }
        let copy = doc
            .components
            .sets
            .values()
            .find(|set| set.id != edit.set)
            .expect("the copied set");
        let copied_frame = copy.root.expect("the copied set has a frame");
        assert_ne!(copied_frame, frame);
        let copied_roots: Vec<_> = copy
            .members
            .iter()
            .map(|member| doc.components.def(*member).expect("copied variant").root)
            .collect();
        assert_eq!(
            doc.scene.children_of(Some(copied_frame)),
            copied_roots.as_slice()
        );
        assert_eq!(doc.components.sets[&edit.set].root, Some(frame));
    }

    fn component_variant_actions(
        set: ComponentId,
        member: ComponentId,
        external: ComponentId,
        member_first: bool,
    ) -> Vec<Action> {
        let (first, second) = if member_first {
            (member, set)
        } else {
            (set, member)
        };
        vec![
            Action::UpdateVariant {
                component: first,
                variant: "Hover".into(),
            },
            Action::OpenLink {
                url: "https://example.com/unchanged".into(),
            },
            Action::UpdateVariant {
                component: second,
                variant: "Default".into(),
            },
            Action::UpdateVariant {
                component: external,
                variant: "External".into(),
            },
            Action::Close,
        ]
    }

    fn component_variant_reference_doc()
    -> (Doc, NodeId, NodeId, ComponentId, ComponentId, ComponentId) {
        let mut doc = Doc::new();
        let mut page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        page.name = "Component page".into();
        let page_id = page.id;
        doc.apply(Operation::create_node(page)).expect("page");
        doc.add_page(page_id);
        doc.set_active_page(Some(page_id));
        let mut section = CanvasNode::new(NodeData::Group(GroupNode::default()));
        section.name = "Component section".into();
        section.parent = Some(page_id);
        let section_id = section.id;
        doc.apply(Operation::create_node(section)).expect("section");
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        frame.name = "Variant set".into();
        frame.parent = Some(section_id);
        let frame_id = frame.id;
        doc.apply(Operation::create_node(frame)).expect("set frame");
        let set = ComponentId::new();
        let member = ComponentId::new();
        let hover = ComponentId::new();
        let external = ComponentId::new();
        let mut external_master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        external_master.name = "External master".into();
        let external_root = external_master.id;
        doc.apply(Operation::create_node(external_master))
            .expect("external master");
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(external, external_root, "External")),
        })
        .expect("external definition");
        for (component, state) in [(member, "Default"), (hover, "Hover")] {
            let mut master = CanvasNode::new(NodeData::Group(GroupNode {
                clip_size: Some([40.0, 20.0]),
                ..Default::default()
            }));
            master.name = format!("State={state}");
            master.parent = Some(frame_id);
            master.index = doc.scene.next_child_index(Some(frame_id));
            let actions = component_variant_actions(set, member, external, false);
            master.reactions.push(fanta_doc::Reaction {
                id: fanta_doc::ReactionId::new(),
                trigger: fanta_doc::Trigger::Click,
                action: actions[0].clone(),
                extra_actions: actions[1..].to_vec(),
                transition: None,
                animation: None,
            });
            let root = master.id;
            doc.apply(Operation::create_node(master))
                .expect("variant master");
            let mut definition = ComponentDef::new(component, root, format!("State={state}"));
            definition.variant_of = Some(fanta_doc::ComponentSetMembership {
                set,
                axis_values: [("State".into(), state.into())].into_iter().collect(),
            });
            doc.apply(Operation::DefineComponent {
                def: Box::new(definition),
            })
            .expect("member definition");
        }
        doc.apply(Operation::DefineComponentSet {
            set: Box::new(fanta_doc::ComponentSet {
                id: set,
                name: "Button".into(),
                axes: vec![fanta_doc::VariantAxis {
                    name: "State".into(),
                    values: vec!["Default".into(), "Hover".into()],
                }],
                members: vec![member, hover],
                default_variant: member,
                root: Some(frame_id),
            }),
        })
        .expect("component set");
        let mut external_nested = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component: member,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [40.0, 20.0],
        }));
        external_nested.name = "External override target".into();
        external_nested.parent = Some(external_root);
        let external_nested_id = external_nested.id;
        doc.apply(Operation::create_node(external_nested))
            .expect("external nested instance");
        for (name, component) in [
            ("Set placement", set),
            ("Member placement", member),
            ("External placement", external),
        ] {
            let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
                component,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [40.0, 20.0],
            }));
            if component == external {
                let NodeData::Instance(placement) = &mut instance.data else {
                    panic!("fixture instance")
                };
                placement.overrides.push(fanta_doc::Override {
                    target_path: [external_nested_id].into_iter().collect(),
                    target_prop: fanta_doc::BoundProp::Visible,
                    value: fanta_doc::OverrideValue::SwapInstance { component: set },
                });
            }
            instance.name = name.into();
            instance.parent = Some(section_id);
            instance.index = doc.scene.next_child_index(Some(section_id));
            let actions = component_variant_actions(set, member, external, true);
            instance.reactions.push(fanta_doc::Reaction {
                id: fanta_doc::ReactionId::new(),
                trigger: fanta_doc::Trigger::Hover,
                action: actions[0].clone(),
                extra_actions: actions[1..].to_vec(),
                transition: None,
                animation: None,
            });
            doc.apply(Operation::create_node(instance))
                .expect("placement");
        }
        let container = ComponentId::new();
        let mut master = CanvasNode::new(NodeData::Group(GroupNode::default()));
        master.name = "Swap container".into();
        master.parent = Some(section_id);
        master.index = doc.scene.next_child_index(Some(section_id));
        let master_id = master.id;
        doc.apply(Operation::create_node(master))
            .expect("swap master");
        let mut overrides = Vec::new();
        for (index, component) in [set, member, external].into_iter().enumerate() {
            let mut nested = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
                component: external,
                overrides: Vec::new(),
                prop_values: Default::default(),
                derived: Vec::new(),
                local_size: [40.0, 20.0],
            }));
            nested.name = format!("Nested swap {index}");
            nested.parent = Some(master_id);
            nested.index = doc.scene.next_child_index(Some(master_id));
            overrides.push(fanta_doc::Override {
                target_path: [nested.id].into_iter().collect(),
                target_prop: fanta_doc::BoundProp::Visible,
                value: fanta_doc::OverrideValue::SwapInstance { component },
            });
            doc.apply(Operation::create_node(nested))
                .expect("nested instance");
        }
        doc.apply(Operation::DefineComponent {
            def: Box::new(ComponentDef::new(container, master_id, "Swap container")),
        })
        .expect("swap container definition");
        let mut placement = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component: container,
            overrides,
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [120.0, 20.0],
        }));
        placement.name = "Swap placement".into();
        placement.parent = Some(section_id);
        placement.index = doc.scene.next_child_index(Some(section_id));
        doc.apply(Operation::create_node(placement))
            .expect("swap placement");
        doc.selection.select_only(section_id);
        doc.history = Default::default();
        (doc, page_id, section_id, set, member, external)
    }

    fn component_copy_state(doc: &Doc) -> serde_json::Value {
        serde_json::json!({
            "scene": doc.scene, "components": doc.components,
            "pages": doc.pages(), "active_page": doc.active_page(), "selection": doc.selection,
        })
    }

    #[test]
    fn component_aware_copy_remaps_variant_actions_and_set_instances_with_undo() {
        for duplicate_page in [true, false] {
            let (mut doc, page, section, set, member, external) = component_variant_reference_doc();
            let before = component_copy_state(&doc);
            let source = doc.clone();
            let operations = if duplicate_page {
                duplicate_page_operations(&doc, page).expect("duplicate page")
            } else {
                duplicate_layer_operations(&doc, section).expect("duplicate component section")
            };
            assert!(
                apply_transaction(&mut doc, "Duplicate component content", operations)
                    .expect("apply duplicate")
            );
            assert_eq!(
                doc.components.defs.len(),
                source.components.defs.len() * 2 - 1,
                "the external component dependency is not cloned"
            );
            assert_eq!(doc.components.sets.len(), 2);
            let copied_set = doc
                .components
                .sets
                .values()
                .find(|candidate| candidate.id != set)
                .expect("copied set");
            let copied_member = copied_set.default_variant;
            let copied_root = if duplicate_page {
                doc.pages()[1]
            } else {
                *doc.scene
                    .children_of(Some(page))
                    .iter()
                    .find(|id| **id != section)
                    .expect("copied section")
            };
            let copy_nodes: Vec<_> = doc
                .scene
                .descendants_of(copied_root)
                .filter_map(|id| doc.scene.get(id))
                .collect();
            for source_id in source.scene.descendants_of(section) {
                let original = source.scene.get(source_id).expect("original node");
                assert_eq!(
                    doc.scene.get(source_id),
                    Some(original),
                    "copy must not retarget the original"
                );
                if original.reactions.is_empty() {
                    continue;
                }
                let copy = copy_nodes
                    .iter()
                    .find(|node| node.name == original.name)
                    .expect("copied interactive node");
                let expected = component_variant_actions(
                    copied_set.id,
                    copied_member,
                    external,
                    matches!(&original.data, NodeData::Instance(_)),
                );
                let mut expected_reaction = original.reactions[0].clone();
                expected_reaction.action = expected[0].clone();
                expected_reaction.extra_actions = expected[1..].to_vec();
                assert_eq!(
                    copy.reactions.as_slice(),
                    &[expected_reaction],
                    "{}",
                    original.name
                );
                if let NodeData::Instance(instance) = &copy.data {
                    let expected_component = match original.name.as_str() {
                        "Set placement" => copied_set.id,
                        "Member placement" => copied_member,
                        "External placement" => external,
                        _ => panic!("unexpected fixture instance"),
                    };
                    assert_eq!(instance.component, expected_component, "{}", original.name);
                    if original.name == "External placement" {
                        let NodeData::Instance(original_instance) = &original.data else {
                            panic!("fixture instance")
                        };
                        let mut expected_override = original_instance.overrides[0].clone();
                        expected_override.value = fanta_doc::OverrideValue::SwapInstance {
                            component: copied_set.id,
                        };
                        assert_eq!(
                            instance.overrides,
                            vec![expected_override],
                            "the external master path is retained while the copied swap destination changes"
                        );
                    }
                }
            }
            let copied_container = doc
                .components
                .defs
                .values()
                .find(|definition| {
                    !source.components.defs.contains_key(&definition.id)
                        && definition.name == "Swap container"
                })
                .expect("copied swap container");
            let copy = copy_nodes
                .iter()
                .find(|node| node.name == "Swap placement")
                .expect("copied swap placement");
            let NodeData::Instance(instance) = &copy.data else {
                panic!("fixture instance")
            };
            assert_eq!(instance.component, copied_container.id);
            assert_eq!(instance.overrides.len(), 3);
            for (index, component) in [copied_set.id, copied_member, external]
                .into_iter()
                .enumerate()
            {
                let nested = copy_nodes
                    .iter()
                    .find(|node| node.name == format!("Nested swap {index}"))
                    .expect("copied override target");
                assert_eq!(
                    instance.overrides[index].target_path.as_slice(),
                    &[nested.id]
                );
                assert_eq!(
                    instance.overrides[index].value,
                    fanta_doc::OverrideValue::SwapInstance { component }
                );
            }
            assert_eq!(
                doc.components.defs.get(&member),
                source.components.defs.get(&member)
            );
            assert_eq!(
                doc.components.defs.get(&external),
                source.components.defs.get(&external)
            );
            let after = component_copy_state(&doc);
            assert!(doc.undo().expect("undo duplicate"));
            assert_eq!(
                component_copy_state(&doc),
                before,
                "one Undo restores all content and references"
            );
            assert!(!doc.undo().expect("no second duplicate transaction"));
            assert!(doc.redo().expect("redo duplicate"));
            assert_eq!(
                component_copy_state(&doc),
                after,
                "Redo restores the same copied IDs and references"
            );
        }
    }

    #[test]
    fn component_aware_copy_keeps_uncopied_variant_sets_and_external_targets() {
        let (mut doc, _, _, set, member, external) = component_variant_reference_doc();
        let master = doc.components.def(member).expect("original member").root;
        doc.selection.select_only(master);
        let before = component_copy_state(&doc);
        let operations = duplicate_layer_operations(&doc, master).expect("copy only one variant");
        assert!(apply_transaction(&mut doc, "Duplicate variant", operations).expect("apply"));
        assert_eq!(
            doc.components.sets.len(),
            1,
            "a partial variant set is not copied"
        );
        let copied_member = doc
            .components
            .defs
            .values()
            .find(|definition| definition.id != member && definition.name == "State=Default")
            .expect("copied member");
        assert!(copied_member.variant_of.is_none());
        let copy = doc.scene.get(copied_member.root).expect("copied master");
        assert_eq!(
            copy.reactions[0].actions().cloned().collect::<Vec<_>>(),
            component_variant_actions(set, copied_member.id, external, false)
        );
        let after = component_copy_state(&doc);
        assert!(doc.undo().expect("undo"));
        assert_eq!(component_copy_state(&doc), before);
        assert!(doc.redo().expect("redo"));
        assert_eq!(component_copy_state(&doc), after);
    }

    fn clipboard_swap_dependency_fixture() -> (Doc, NodeId, NodeId) {
        let (mut doc, removed_master, _, _) = subtree_doc();
        let removed = ComponentId::new();
        doc.components.defs.insert(
            removed,
            ComponentDef::new(removed, removed_master, "Removed swap target"),
        );
        let mut surviving_master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([40.0, 20.0]),
            ..Default::default()
        }));
        surviving_master.parent = doc.active_page();
        surviving_master.index = doc.scene.next_child_index(surviving_master.parent);
        let surviving_root = surviving_master.id;
        doc.scene
            .insert(surviving_master)
            .expect("surviving master");
        let surviving = ComponentId::new();
        doc.components.defs.insert(
            surviving,
            ComponentDef::new(surviving, surviving_root, "Surviving master"),
        );
        let mut default_master = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([40.0, 20.0]),
            ..Default::default()
        }));
        default_master.parent = doc.active_page();
        default_master.index = doc.scene.next_child_index(default_master.parent);
        let default_root = default_master.id;
        doc.scene.insert(default_master).expect("default master");
        let default_component = ComponentId::new();
        doc.components.defs.insert(
            default_component,
            ComponentDef::new(default_component, default_root, "Default nested content"),
        );
        let instance = |component| fanta_doc::InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [40.0, 20.0],
        };
        let mut nested = CanvasNode::new(NodeData::Instance(instance(default_component)));
        nested.parent = Some(surviving_root);
        let nested_id = nested.id;
        doc.scene.insert(nested).expect("nested default instance");
        let mut placement = instance(surviving);
        placement.overrides.push(fanta_doc::Override {
            target_path: [nested_id].into_iter().collect(),
            target_prop: fanta_doc::BoundProp::Visible,
            value: fanta_doc::OverrideValue::SwapInstance { component: removed },
        });
        let mut placement = CanvasNode::new(NodeData::Instance(placement));
        placement.parent = doc.active_page();
        placement.index = doc.scene.next_child_index(placement.parent);
        let placement_id = placement.id;
        doc.scene.insert(placement).expect("placed swap instance");
        doc.history = Default::default();
        (doc, removed_master, placement_id)
    }

    #[test]
    fn clipboard_replace_refuses_a_swap_dependency_removed_by_replacement() {
        let (mut doc, master, placement) = clipboard_swap_dependency_fixture();
        doc.selection.select_only(placement);
        let payload = CanvasClipboard::capture(&doc).expect("copy placement only");
        doc.scene
            .remove(placement)
            .expect("copied source removed before replacement");
        doc.selection.select_only(master);
        let before = serde_json::to_value(&doc).expect("snapshot");
        let error = paste_to_replace_operations(&doc, master, &payload)
            .expect_err("swap target must exist after deletion");
        assert!(
            error
                .to_string()
                .contains("swap component is no longer available")
        );
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }

    #[test]
    fn clipboard_delete_refuses_surviving_sparse_swap_dependency_atomically() {
        let (mut doc, master, _) = clipboard_swap_dependency_fixture();
        doc.selection.select_only(master);
        let before = serde_json::to_value(&doc).expect("snapshot");
        let error = crate::layer_context_ops::delete_layers(&doc, master)
            .expect_err("surviving virtual swap requires removed master");
        assert!(error.to_string().contains("swapped component"));
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }

    #[test]
    fn clipboard_replace_refuses_instance_over_its_own_master_atomically() {
        let (mut doc, master, _, _) = subtree_doc();
        let component = ComponentId::new();
        doc.components.defs.insert(
            component,
            ComponentDef::new(component, master, "Replaced master"),
        );
        let mut instance = CanvasNode::new(NodeData::Instance(fanta_doc::InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [200.0, 120.0],
        }));
        instance.parent = doc.active_page();
        instance.index = doc.scene.next_child_index(instance.parent);
        let instance_id = instance.id;
        doc.scene.insert(instance).expect("placed instance");
        doc.selection.select_only(instance_id);
        let payload = CanvasClipboard::capture(&doc).expect("copy placement only");
        doc.selection.select_only(master);
        let before = serde_json::to_value(&doc).expect("snapshot");
        let error = paste_to_replace_operations(&doc, master, &payload)
            .expect_err("the replacement cannot depend on the master it removes");
        assert!(
            error
                .to_string()
                .contains("component is no longer available")
        );
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }

    #[test]
    fn clipboard_replace_copied_master_over_itself_keeps_usable_definition() {
        let (mut doc, master, _, _) = subtree_doc();
        let component = ComponentId::new();
        doc.components.defs.insert(
            component,
            ComponentDef::new(component, master, "Replaced master"),
        );
        doc.selection.select_only(master);
        let payload = CanvasClipboard::capture(&doc).expect("copy master and definition");
        let before = serde_json::to_value(&doc).expect("snapshot");
        let operations = paste_to_replace_operations(&doc, master, &payload)
            .expect("self replacement with complete master");
        apply_transaction(&mut doc, "Replace", operations).expect("commit");
        assert!(!doc.scene.contains(master));
        assert_eq!(doc.components.defs.len(), 1);
        let restored = doc.components.defs.values().next().expect("definition");
        assert_ne!(restored.id, component);
        assert_eq!(restored.name, "Replaced master");
        assert!(doc.scene.contains(restored.root));
        assert_eq!(doc.scene.descendants_of(restored.root).count(), 3);
        assert!(doc.undo().expect("undo"));
        let mut actual = serde_json::to_value(&doc).expect("after undo");
        let mut expected = before;
        actual.as_object_mut().expect("doc").remove("history");
        expected.as_object_mut().expect("doc").remove("history");
        assert_eq!(actual, expected);
    }

    #[test]
    fn clipboard_cut_then_paste_to_replace_restores_components() {
        let (mut doc, page, section, _, _, external) = component_variant_reference_doc();
        let payload = CanvasClipboard::capture(&doc).expect("whole component subtree");
        payload.validate_cut().expect("full sets captured");
        let operations = crate::layer_context_ops::delete_layers(&doc, section).expect("cut");
        apply_transaction(&mut doc, "Cut", operations).expect("commit cut");
        let mut target = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([100.0, 100.0]),
            ..Default::default()
        }));
        target.parent = Some(page);
        let target_id = target.id;
        doc.scene.insert(target).expect("replacement target");
        doc.selection.select_only(target_id);
        let before = serde_json::to_value(&doc).expect("before");
        let operations = paste_to_replace_operations(&doc, target_id, &payload).expect("replace");
        apply_transaction(&mut doc, "Replace", operations).expect("commit");
        assert!(!doc.scene.contains(target_id));
        assert_eq!(
            doc.components.defs.len(),
            4,
            "restore three copied masters beside unchanged external dependency"
        );
        assert_eq!(doc.components.sets.len(), 1);
        let set = doc.components.sets.values().next().expect("restored set");
        assert!(
            set.members
                .iter()
                .all(|id| doc.components.def(*id).is_some())
        );
        assert!(doc.components.def(external).is_some());
        for node in doc
            .scene
            .roots()
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .filter_map(|id| doc.scene.get(id))
        {
            if let NodeData::Instance(instance) = &node.data {
                assert!(
                    doc.components.def(instance.component).is_some()
                        || doc.components.sets.contains_key(&instance.component),
                    "pasted instances must retain valid component references"
                );
            }
        }
        assert!(doc.undo().expect("undo"));
        let mut actual = serde_json::to_value(&doc).expect("after undo");
        let mut expected = before;
        actual.as_object_mut().expect("doc").remove("history");
        expected.as_object_mut().expect("doc").remove("history");
        assert_eq!(actual, expected);
    }

    #[test]
    fn clipboard_paste_keeps_current_registry_after_intervening_edits() {
        let (mut doc, _, section, set, member, _) = component_variant_reference_doc();
        let payload = CanvasClipboard::capture(&doc).expect("copy complete section");
        doc.components.defs.get_mut(&member).expect("member").name = "Edited after Copy".into();
        let set = doc.components.sets.get_mut(&set).expect("set");
        set.name = "Edited set".into();
        set.default_variant = *set.members.last().expect("other member");
        let registry = doc.components.clone();
        let original_nodes = doc.scene.clone();
        let mut pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Paste)
            .expect("instantiate");
        let operations = paste_operations(&doc, &payload, &mut pasted).expect("paste");
        apply_transaction(&mut doc, "Paste", operations).expect("commit");
        assert_eq!(
            doc.components, registry,
            "Copy snapshots must never overwrite live definitions"
        );
        for id in original_nodes.descendants_of(section) {
            assert_eq!(doc.scene.get(id), original_nodes.get(id));
        }
        assert!(doc.undo().expect("undo"));
        assert_eq!(doc.components, registry);
        assert_eq!(
            serde_json::to_value(&doc.scene).expect("scene"),
            serde_json::to_value(&original_nodes).expect("original scene")
        );
    }

    #[test]
    fn clipboard_partial_variant_copy_refuses_restore_after_definition_removal() {
        let (mut doc, _, _, _, member, _) = component_variant_reference_doc();
        let root = doc.components.def(member).expect("member").root;
        doc.selection.select_only(root);
        let payload = CanvasClipboard::capture(&doc).expect("copy partial set");
        assert!(payload.validate_cut().is_err());
        let ids = doc
            .scene
            .roots()
            .iter()
            .flat_map(|root| doc.scene.descendants_of(*root))
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(CanvasNode {
                data: NodeData::Instance(instance),
                ..
            }) = doc.scene.get_mut(id)
            {
                instance.overrides.clear();
            }
        }
        let operations =
            crate::layer_context_ops::delete_layers(&doc, root).expect("delete remains allowed");
        apply_transaction(&mut doc, "Delete", operations).expect("delete");
        let before = serde_json::to_value(&doc).expect("snapshot");
        let mut pasted = payload
            .instantiate(&doc, 16.0, ClipboardPlacement::Paste)
            .expect("instantiate");
        let error = paste_operations(&doc, &payload, &mut pasted)
            .expect_err("cannot restore incomplete old set");
        assert!(error.to_string().contains("complete set"));
        assert_eq!(serde_json::to_value(&doc).expect("snapshot"), before);
    }
}

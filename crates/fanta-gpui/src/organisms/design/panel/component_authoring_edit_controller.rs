//! Guarded edit sessions for component-property authoring.
//!
//! Component authoring renders from host-controlled definitions, but rename
//! and reorder gestures are phased host transactions. Keeping those sessions
//! in this controller prevents presentation state from dropping a Begin
//! without producing exactly one Commit or Cancel.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::organisms::design::panel) enum ComponentAuthoringNameEditor {
    Property {
        node_id: SharedString,
        property_id: SharedString,
        original_name: SharedString,
        expected_name: SharedString,
        last_preview: Option<SharedString>,
    },
    VariantOption {
        node_id: SharedString,
        property_id: SharedString,
        option_id: SharedString,
        original_name: SharedString,
        expected_name: SharedString,
        last_preview: Option<SharedString>,
    },
    NewVariantOption {
        node_id: SharedString,
        property_id: SharedString,
        after_option_id: Option<SharedString>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::organisms::design::panel) struct ComponentPropertyReorderSession {
    pub(in crate::organisms::design::panel) node_id: SharedString,
    pub(in crate::organisms::design::panel) property_id: SharedString,
    pub(in crate::organisms::design::panel) partition: DesignComponentPropertyPartition,
    pub(in crate::organisms::design::panel) original_order: Vec<SharedString>,
    expected_order: Vec<SharedString>,
    pub(in crate::organisms::design::panel) last_before_property_id: Option<SharedString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::organisms::design::panel) struct ComponentVariantOptionReorderSession {
    pub(in crate::organisms::design::panel) node_id: SharedString,
    pub(in crate::organisms::design::panel) property_id: SharedString,
    pub(in crate::organisms::design::panel) option_id: SharedString,
    pub(in crate::organisms::design::panel) original_order: Vec<SharedString>,
    expected_order: Vec<SharedString>,
    pub(in crate::organisms::design::panel) last_before_option_id: Option<SharedString>,
}

#[derive(Default)]
pub(in crate::organisms::design::panel) struct DesignComponentAuthoringEditController {
    name_editor: Option<ComponentAuthoringNameEditor>,
    property_reorder: Option<ComponentPropertyReorderSession>,
    variant_option_reorder: Option<ComponentVariantOptionReorderSession>,
}

impl DesignComponentAuthoringEditController {
    pub(in crate::organisms::design::panel) fn name_editor(
        &self,
    ) -> Option<&ComponentAuthoringNameEditor> {
        self.name_editor.as_ref()
    }

    pub(in crate::organisms::design::panel) fn property_reorder(
        &self,
    ) -> Option<&ComponentPropertyReorderSession> {
        self.property_reorder.as_ref()
    }

    pub(in crate::organisms::design::panel) fn variant_option_reorder(
        &self,
    ) -> Option<&ComponentVariantOptionReorderSession> {
        self.variant_option_reorder.as_ref()
    }

    pub(in crate::organisms::design::panel) fn has_name_editor(&self) -> bool {
        self.name_editor.is_some()
    }

    pub(in crate::organisms::design::panel) fn has_property_reorder(&self) -> bool {
        self.property_reorder.is_some()
    }

    pub(in crate::organisms::design::panel) fn has_variant_option_reorder(&self) -> bool {
        self.variant_option_reorder.is_some()
    }

    pub(in crate::organisms::design::panel) fn has_active_session(&self) -> bool {
        matches!(
            self.name_editor,
            Some(
                ComponentAuthoringNameEditor::Property { .. }
                    | ComponentAuthoringNameEditor::VariantOption { .. }
            )
        ) || self.property_reorder.is_some()
            || self.variant_option_reorder.is_some()
    }

    fn has_any_interaction(&self) -> bool {
        self.name_editor.is_some()
            || self.property_reorder.is_some()
            || self.variant_option_reorder.is_some()
    }

    pub(in crate::organisms::design::panel) fn begin_new_variant_option(
        &mut self,
        node_id: SharedString,
        property_id: SharedString,
        after_option_id: Option<SharedString>,
    ) -> bool {
        if self.has_any_interaction() {
            return false;
        }
        self.name_editor = Some(ComponentAuthoringNameEditor::NewVariantOption {
            node_id,
            property_id,
            after_option_id,
        });
        true
    }

    pub(in crate::organisms::design::panel) fn clear_unphased_name_editor(&mut self) -> bool {
        if matches!(
            self.name_editor,
            Some(ComponentAuthoringNameEditor::NewVariantOption { .. })
        ) {
            self.name_editor = None;
            true
        } else {
            false
        }
    }

    /// Accepts one already access-validated host action into its lifecycle.
    ///
    /// Returning `false` rejects previews before Begin, duplicate Begin and
    /// terminal events after the transaction has already been consumed.
    pub(in crate::organisms::design::panel) fn track_action(
        &mut self,
        action: &DesignPanelAction,
    ) -> bool {
        match action {
            DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                node_id,
                property_id,
                original_name,
                expected_name,
                name,
                phase,
            } => self.track_property_rename(
                node_id,
                property_id,
                original_name,
                expected_name,
                name,
                *phase,
            ),
            DesignPanelAction::ComponentVariantOptionRenameRequested {
                node_id,
                property_id,
                option_id,
                original_name,
                expected_name,
                name,
                phase,
            } => self.track_variant_option_rename(
                node_id,
                property_id,
                option_id,
                original_name,
                expected_name,
                name,
                *phase,
            ),
            DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
                node_id,
                property_id,
                partition,
                original_property_order,
                expected_property_order,
                before_property_id,
                phase,
            } => self.track_property_reorder(
                node_id,
                property_id,
                *partition,
                original_property_order,
                expected_property_order,
                before_property_id,
                *phase,
            ),
            DesignPanelAction::ComponentVariantOptionReorderRequested {
                node_id,
                property_id,
                option_id,
                original_option_order,
                expected_option_order,
                before_option_id,
                phase,
            } => self.track_variant_option_reorder(
                node_id,
                property_id,
                option_id,
                original_option_order,
                expected_option_order,
                before_option_id,
                *phase,
            ),
            _ => false,
        }
    }

    fn track_property_rename(
        &mut self,
        node_id: &SharedString,
        property_id: &SharedString,
        original_name: &SharedString,
        expected_name: &SharedString,
        name: &SharedString,
        phase: DesignPanelEditPhase,
    ) -> bool {
        match phase {
            DesignPanelEditPhase::Begin if !self.has_any_interaction() => {
                self.name_editor = Some(ComponentAuthoringNameEditor::Property {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    original_name: original_name.clone(),
                    expected_name: expected_name.clone(),
                    last_preview: None,
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview => {
                let Some(ComponentAuthoringNameEditor::Property {
                    node_id: active_node,
                    property_id: active_property,
                    original_name: active_original,
                    expected_name: active_expected,
                    last_preview,
                }) = self.name_editor.as_mut()
                else {
                    return false;
                };
                if active_node != node_id
                    || active_property != property_id
                    || active_original != original_name
                    || last_preview.as_ref() == Some(name)
                    || active_original == name
                {
                    return false;
                }
                *active_expected = expected_name.clone();
                *last_preview = Some(name.clone());
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => {
                let matches = matches!(
                    self.name_editor.as_ref(),
                    Some(ComponentAuthoringNameEditor::Property {
                        node_id: active_node,
                        property_id: active_property,
                        original_name: active_original,
                        ..
                    }) if active_node == node_id
                        && active_property == property_id
                        && active_original == original_name
                );
                if matches {
                    self.name_editor = None;
                }
                matches
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn track_variant_option_rename(
        &mut self,
        node_id: &SharedString,
        property_id: &SharedString,
        option_id: &SharedString,
        original_name: &SharedString,
        expected_name: &SharedString,
        name: &SharedString,
        phase: DesignPanelEditPhase,
    ) -> bool {
        match phase {
            DesignPanelEditPhase::Begin if !self.has_any_interaction() => {
                self.name_editor = Some(ComponentAuthoringNameEditor::VariantOption {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    option_id: option_id.clone(),
                    original_name: original_name.clone(),
                    expected_name: expected_name.clone(),
                    last_preview: None,
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview => {
                let Some(ComponentAuthoringNameEditor::VariantOption {
                    node_id: active_node,
                    property_id: active_property,
                    option_id: active_option,
                    original_name: active_original,
                    expected_name: active_expected,
                    last_preview,
                }) = self.name_editor.as_mut()
                else {
                    return false;
                };
                if active_node != node_id
                    || active_property != property_id
                    || active_option != option_id
                    || active_original != original_name
                    || last_preview.as_ref() == Some(name)
                    || active_original == name
                {
                    return false;
                }
                *active_expected = expected_name.clone();
                *last_preview = Some(name.clone());
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => {
                let matches = matches!(
                    self.name_editor.as_ref(),
                    Some(ComponentAuthoringNameEditor::VariantOption {
                        node_id: active_node,
                        property_id: active_property,
                        option_id: active_option,
                        original_name: active_original,
                        ..
                    }) if active_node == node_id
                        && active_property == property_id
                        && active_option == option_id
                        && active_original == original_name
                );
                if matches {
                    self.name_editor = None;
                }
                matches
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn track_property_reorder(
        &mut self,
        node_id: &SharedString,
        property_id: &SharedString,
        partition: DesignComponentPropertyPartition,
        original_order: &[SharedString],
        expected_order: &[SharedString],
        before_property_id: &Option<SharedString>,
        phase: DesignPanelEditPhase,
    ) -> bool {
        match phase {
            DesignPanelEditPhase::Begin if !self.has_any_interaction() => {
                self.property_reorder = Some(ComponentPropertyReorderSession {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    partition,
                    original_order: original_order.to_vec(),
                    expected_order: expected_order.to_vec(),
                    last_before_property_id: before_property_id.clone(),
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview => {
                let Some(active) = self.property_reorder.as_mut() else {
                    return false;
                };
                if active.node_id != *node_id
                    || active.property_id != *property_id
                    || active.partition != partition
                    || active.original_order != original_order
                    || active.last_before_property_id == *before_property_id
                    || before_property_id.as_ref() == Some(property_id)
                {
                    return false;
                }
                active.expected_order = expected_order.to_vec();
                active.last_before_property_id = before_property_id.clone();
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => {
                let matches = self.property_reorder.as_ref().is_some_and(|active| {
                    active.node_id == *node_id
                        && active.property_id == *property_id
                        && active.partition == partition
                        && active.original_order == original_order
                });
                if matches {
                    self.property_reorder = None;
                }
                matches
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn track_variant_option_reorder(
        &mut self,
        node_id: &SharedString,
        property_id: &SharedString,
        option_id: &SharedString,
        original_order: &[SharedString],
        expected_order: &[SharedString],
        before_option_id: &Option<SharedString>,
        phase: DesignPanelEditPhase,
    ) -> bool {
        match phase {
            DesignPanelEditPhase::Begin if !self.has_any_interaction() => {
                self.variant_option_reorder = Some(ComponentVariantOptionReorderSession {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    option_id: option_id.clone(),
                    original_order: original_order.to_vec(),
                    expected_order: expected_order.to_vec(),
                    last_before_option_id: before_option_id.clone(),
                });
                true
            }
            DesignPanelEditPhase::Begin => false,
            DesignPanelEditPhase::Preview => {
                let Some(active) = self.variant_option_reorder.as_mut() else {
                    return false;
                };
                if active.node_id != *node_id
                    || active.property_id != *property_id
                    || active.option_id != *option_id
                    || active.original_order != original_order
                    || active.last_before_option_id == *before_option_id
                    || before_option_id.as_ref() == Some(option_id)
                {
                    return false;
                }
                active.expected_order = expected_order.to_vec();
                active.last_before_option_id = before_option_id.clone();
                true
            }
            DesignPanelEditPhase::Commit | DesignPanelEditPhase::Cancel => {
                let matches = self.variant_option_reorder.as_ref().is_some_and(|active| {
                    active.node_id == *node_id
                        && active.property_id == *property_id
                        && active.option_id == *option_id
                        && active.original_order == original_order
                });
                if matches {
                    self.variant_option_reorder = None;
                }
                matches
            }
        }
    }

    pub(in crate::organisms::design::panel) fn cancel_name_edit(
        &mut self,
        expected_name: Option<SharedString>,
    ) -> Option<DesignPanelAction> {
        let editor = self.name_editor.take()?;
        match editor {
            ComponentAuthoringNameEditor::Property {
                node_id,
                property_id,
                original_name,
                expected_name: stored_expected,
                ..
            } => Some(
                DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                    node_id,
                    property_id,
                    original_name: original_name.clone(),
                    expected_name: expected_name.unwrap_or(stored_expected),
                    name: original_name,
                    phase: DesignPanelEditPhase::Cancel,
                },
            ),
            ComponentAuthoringNameEditor::VariantOption {
                node_id,
                property_id,
                option_id,
                original_name,
                expected_name: stored_expected,
                ..
            } => Some(DesignPanelAction::ComponentVariantOptionRenameRequested {
                node_id,
                property_id,
                option_id,
                original_name: original_name.clone(),
                expected_name: expected_name.unwrap_or(stored_expected),
                name: original_name,
                phase: DesignPanelEditPhase::Cancel,
            }),
            ComponentAuthoringNameEditor::NewVariantOption { .. } => None,
        }
    }

    pub(in crate::organisms::design::panel) fn reconcile_name_edit(
        &mut self,
        current_node_id: &SharedString,
        current_name: Option<SharedString>,
        target_editable: bool,
    ) -> Option<DesignPanelAction> {
        let editor = self.name_editor.as_mut()?;
        let editor_node_id = match editor {
            ComponentAuthoringNameEditor::Property { node_id, .. }
            | ComponentAuthoringNameEditor::VariantOption { node_id, .. }
            | ComponentAuthoringNameEditor::NewVariantOption { node_id, .. } => node_id,
        };
        if editor_node_id != current_node_id || !target_editable {
            let expected = (editor_node_id == current_node_id)
                .then_some(current_name)
                .flatten();
            return self.cancel_name_edit(expected);
        }
        match editor {
            ComponentAuthoringNameEditor::Property {
                original_name,
                expected_name,
                last_preview,
                ..
            }
            | ComponentAuthoringNameEditor::VariantOption {
                original_name,
                expected_name,
                last_preview,
                ..
            } => {
                let Some(current_name) = current_name else {
                    return self.cancel_name_edit(None);
                };
                if current_name == *original_name
                    || last_preview
                        .as_ref()
                        .is_some_and(|preview| *preview == current_name)
                {
                    *expected_name = current_name;
                    None
                } else {
                    self.cancel_name_edit(Some(current_name))
                }
            }
            ComponentAuthoringNameEditor::NewVariantOption { .. } => None,
        }
    }

    pub(in crate::organisms::design::panel) fn cancel_property_reorder(
        &mut self,
        expected_order: Option<Vec<SharedString>>,
    ) -> Option<DesignPanelAction> {
        let session = self.property_reorder.take()?;
        let before_property_id = order_successor(&session.original_order, &session.property_id);
        Some(
            DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
                node_id: session.node_id,
                property_id: session.property_id,
                partition: session.partition,
                original_property_order: session.original_order,
                expected_property_order: expected_order.unwrap_or(session.expected_order),
                before_property_id,
                phase: DesignPanelEditPhase::Cancel,
            },
        )
    }

    pub(in crate::organisms::design::panel) fn reconcile_property_reorder(
        &mut self,
        current_node_id: &SharedString,
        current_order: Option<Vec<SharedString>>,
        target_editable: bool,
    ) -> Option<DesignPanelAction> {
        let session = self.property_reorder.as_mut()?;
        let same_node = session.node_id == *current_node_id;
        let survives = same_node
            && target_editable
            && current_order.as_ref().is_some_and(|order| {
                order.contains(&session.property_id)
                    && component_authoring_orders_have_same_unique_members(
                        order,
                        &session.original_order,
                    )
            });
        if survives {
            session.expected_order = current_order.expect("a surviving reorder has an order");
            None
        } else {
            self.cancel_property_reorder(same_node.then_some(current_order).flatten())
        }
    }

    pub(in crate::organisms::design::panel) fn cancel_variant_option_reorder(
        &mut self,
        expected_order: Option<Vec<SharedString>>,
    ) -> Option<DesignPanelAction> {
        let session = self.variant_option_reorder.take()?;
        let before_option_id = order_successor(&session.original_order, &session.option_id);
        Some(DesignPanelAction::ComponentVariantOptionReorderRequested {
            node_id: session.node_id,
            property_id: session.property_id,
            option_id: session.option_id,
            original_option_order: session.original_order,
            expected_option_order: expected_order.unwrap_or(session.expected_order),
            before_option_id,
            phase: DesignPanelEditPhase::Cancel,
        })
    }

    pub(in crate::organisms::design::panel) fn reconcile_variant_option_reorder(
        &mut self,
        current_node_id: &SharedString,
        current_order: Option<Vec<SharedString>>,
        target_editable: bool,
    ) -> Option<DesignPanelAction> {
        let session = self.variant_option_reorder.as_mut()?;
        let same_node = session.node_id == *current_node_id;
        let survives = same_node
            && target_editable
            && current_order.as_ref().is_some_and(|order| {
                order.contains(&session.option_id)
                    && component_authoring_orders_have_same_unique_members(
                        order,
                        &session.original_order,
                    )
            });
        if survives {
            session.expected_order = current_order.expect("a surviving reorder has an order");
            None
        } else {
            self.cancel_variant_option_reorder(same_node.then_some(current_order).flatten())
        }
    }

    pub(in crate::organisms::design::panel) fn clear_after_cancellation(&mut self) {
        debug_assert!(
            !self.has_active_session(),
            "component-authoring edits must emit Cancel before their controller is cleared"
        );
        self.name_editor = None;
    }
}

fn order_successor(order: &[SharedString], id: &str) -> Option<SharedString> {
    order
        .iter()
        .position(|candidate| candidate.as_ref() == id)
        .and_then(|index| order.get(index + 1))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property_rename(
        phase: DesignPanelEditPhase,
        expected_name: &str,
        name: &str,
    ) -> DesignPanelAction {
        DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
            node_id: "node".into(),
            property_id: "label".into(),
            original_name: "Label".into(),
            expected_name: expected_name.into(),
            name: name.into(),
            phase,
        }
    }

    fn option_rename(
        phase: DesignPanelEditPhase,
        expected_name: &str,
        name: &str,
    ) -> DesignPanelAction {
        DesignPanelAction::ComponentVariantOptionRenameRequested {
            node_id: "node".into(),
            property_id: "state".into(),
            option_id: "hover".into(),
            original_name: "Hover".into(),
            expected_name: expected_name.into(),
            name: name.into(),
            phase,
        }
    }

    fn property_reorder(
        phase: DesignPanelEditPhase,
        expected_order: &[&str],
        before: Option<&str>,
    ) -> DesignPanelAction {
        DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            node_id: "node".into(),
            property_id: "a".into(),
            partition: DesignComponentPropertyPartition::Regular,
            original_property_order: vec!["a".into(), "b".into(), "c".into()],
            expected_property_order: expected_order.iter().map(|id| (*id).into()).collect(),
            before_property_id: before.map(Into::into),
            phase,
        }
    }

    fn option_reorder(
        phase: DesignPanelEditPhase,
        expected_order: &[&str],
        before: Option<&str>,
    ) -> DesignPanelAction {
        DesignPanelAction::ComponentVariantOptionReorderRequested {
            node_id: "node".into(),
            property_id: "state".into(),
            option_id: "hover".into(),
            original_option_order: vec!["default".into(), "hover".into()],
            expected_option_order: expected_order.iter().map(|id| (*id).into()).collect(),
            before_option_id: before.map(Into::into),
            phase,
        }
    }

    #[test]
    fn rename_requires_one_begin_and_consumes_one_terminal() {
        let mut controller = DesignComponentAuthoringEditController::default();
        let preview = property_rename(DesignPanelEditPhase::Preview, "Label", "Primary");
        assert!(!controller.track_action(&preview));
        assert!(controller.track_action(&property_rename(
            DesignPanelEditPhase::Begin,
            "Label",
            "Label",
        )));
        assert!(!controller.track_action(&property_rename(
            DesignPanelEditPhase::Begin,
            "Label",
            "Label",
        )));
        assert!(controller.track_action(&preview));
        assert!(
            !controller.track_action(&preview),
            "duplicate Preview is ignored"
        );
        assert!(controller.track_action(&property_rename(
            DesignPanelEditPhase::Commit,
            "Primary",
            "Primary",
        )));
        assert!(!controller.has_active_session());
        assert!(controller.cancel_name_edit(None).is_none());
        assert!(!controller.track_action(&property_rename(
            DesignPanelEditPhase::Commit,
            "Primary",
            "Primary",
        )));
    }

    #[test]
    fn rename_echo_rebases_expected_name_and_context_or_access_loss_cancels_once() {
        let mut controller = DesignComponentAuthoringEditController::default();
        assert!(controller.track_action(&property_rename(
            DesignPanelEditPhase::Begin,
            "Label",
            "Label",
        )));
        assert!(controller.track_action(&property_rename(
            DesignPanelEditPhase::Preview,
            "Label",
            "Primary",
        )));
        assert!(
            controller
                .reconcile_name_edit(&"node".into(), Some("Primary".into()), true)
                .is_none(),
            "the accepted Preview echo keeps the transaction"
        );
        let cancel = controller
            .reconcile_name_edit(&"other-node".into(), None, false)
            .expect("a context change consumes one Cancel");
        assert!(matches!(
            cancel,
            DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                expected_name,
                name,
                phase: DesignPanelEditPhase::Cancel,
                ..
            } if expected_name.as_ref() == "Primary" && name.as_ref() == "Label"
        ));
        assert!(
            controller
                .reconcile_name_edit(&"other-node".into(), None, false)
                .is_none()
        );

        assert!(controller.track_action(&option_rename(
            DesignPanelEditPhase::Begin,
            "Hover",
            "Hover",
        )));
        assert!(matches!(
            controller
                .reconcile_name_edit(&"node".into(), Some("Hover".into()), false)
                .expect("access loss cancels the Variant option rename"),
            DesignPanelAction::ComponentVariantOptionRenameRequested {
                phase: DesignPanelEditPhase::Cancel,
                ..
            }
        ));
        assert!(controller.cancel_name_edit(None).is_none());
    }

    #[test]
    fn property_reorder_keeps_stable_members_and_has_one_cancel() {
        let mut controller = DesignComponentAuthoringEditController::default();
        assert!(!controller.track_action(&property_reorder(
            DesignPanelEditPhase::Preview,
            &["a", "b", "c"],
            Some("c"),
        )));
        assert!(controller.track_action(&property_reorder(
            DesignPanelEditPhase::Begin,
            &["a", "b", "c"],
            Some("b"),
        )));
        assert!(!controller.track_action(&property_reorder(
            DesignPanelEditPhase::Begin,
            &["a", "b", "c"],
            Some("b"),
        )));
        assert!(controller.track_action(&property_reorder(
            DesignPanelEditPhase::Preview,
            &["a", "b", "c"],
            Some("c"),
        )));
        let echoed = vec!["b".into(), "a".into(), "c".into()];
        assert!(
            controller
                .reconcile_property_reorder(&"node".into(), Some(echoed.clone()), true)
                .is_none()
        );
        let cancel = controller
            .reconcile_property_reorder(&"node".into(), Some(echoed.clone()), false)
            .expect("access loss consumes one reorder Cancel");
        assert!(matches!(
            cancel,
            DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
                expected_property_order,
                before_property_id: Some(before),
                phase: DesignPanelEditPhase::Cancel,
                ..
            } if expected_property_order == echoed && before.as_ref() == "b"
        ));
        assert!(
            controller
                .reconcile_property_reorder(&"node".into(), None, false)
                .is_none()
        );
    }

    #[test]
    fn variant_option_reorder_stale_members_cancel_once_and_allow_a_new_commit() {
        let mut controller = DesignComponentAuthoringEditController::default();
        assert!(controller.track_action(&option_reorder(
            DesignPanelEditPhase::Begin,
            &["default", "hover"],
            None,
        )));
        let stale = vec!["default".into(), "pressed".into()];
        assert!(matches!(
            controller
                .reconcile_variant_option_reorder(&"node".into(), Some(stale.clone()), true)
                .expect("stale stable members consume one Cancel"),
            DesignPanelAction::ComponentVariantOptionReorderRequested {
                expected_option_order,
                phase: DesignPanelEditPhase::Cancel,
                ..
            } if expected_option_order == stale
        ));
        assert!(controller.cancel_variant_option_reorder(None).is_none());

        assert!(controller.track_action(&option_reorder(
            DesignPanelEditPhase::Begin,
            &["default", "hover"],
            None,
        )));
        assert!(controller.track_action(&option_reorder(
            DesignPanelEditPhase::Commit,
            &["default", "hover"],
            None,
        )));
        assert!(!controller.has_active_session());
        assert!(!controller.track_action(&option_reorder(
            DesignPanelEditPhase::Commit,
            &["default", "hover"],
            None,
        )));
    }
}

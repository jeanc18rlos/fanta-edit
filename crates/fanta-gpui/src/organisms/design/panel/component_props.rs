use super::*;

mod dialogs;
mod editors;
mod state;

pub(super) use state::ComponentAuthoringState;

/// Internal Component/instance controller surface used by the thin `DesignPanel` facade.
pub(super) trait DesignComponentController: Sized + 'static {
    fn emit_component_multiline_event(
        &self,
        event: DesignComponentMultilineEditEvent,
        cx: &mut Context<Self>,
    );
    fn component_property_index(&self, property_id: &str) -> Option<usize>;
    fn component_property_variable_target(
        &self,
        index: usize,
    ) -> Option<DesignComponentPropertyVariableTarget>;
    fn component_property_variable_can_change(&self, index: usize) -> bool;
    fn emit_component_property_variable_apply(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    );
    fn emit_component_property_variable_import(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    );
    fn emit_component_property_variable_detach(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    );
    fn open_component_property_variable_picker(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn component_swap_can_change(&self, index: usize) -> bool;
    fn emit_component_swap_apply(
        &mut self,
        index: usize,
        selection: Option<DesignComponentSwapSelection>,
        cx: &mut Context<Self>,
    );
    fn emit_component_swap_import(
        &mut self,
        index: usize,
        selection: DesignComponentSwapSelection,
        cx: &mut Context<Self>,
    );
    fn set_component_swap_preview(
        &mut self,
        index: usize,
        selection: Option<DesignComponentSwapSelection>,
        cx: &mut Context<Self>,
    );
    fn clear_component_swap_preview(&mut self, property_id: &str, cx: &mut Context<Self>);
    fn open_component_swap_browser(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn open_component_multiline_editor(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn open_component_multiline_editor_from_control(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn preview_component_multiline(&mut self, cx: &mut Context<Self>);
    fn finish_component_multiline_editor(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn component_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(SharedString, DesignComponentPropertyValue)>;
    fn slot_settings_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(
        SharedString,
        super::super::DesignSlotSettings,
        DesignSlotSettingsChange,
    )>;
    fn component_property_with_index(
        property: DesignPanelProperty,
        index: usize,
    ) -> DesignPanelProperty;
    fn cancel_component_multiline_transaction(&mut self, cx: &mut Context<Self>);
    fn cancel_component_swap_preview(&mut self, cx: &mut Context<Self>);
    fn render_component_property_variable_button(
        &self,
        index: usize,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement>;
    fn render_component_swap_browser(
        &self,
        index: usize,
        property: &super::super::DesignComponentProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn component_authoring_view_data(&self) -> Option<&DesignComponentAuthoringViewData>;
    fn component_property_partition_order(
        &self,
        partition: DesignComponentPropertyPartition,
    ) -> Vec<SharedString>;
    fn reconcile_slot_limits_state(&mut self);
    fn component_variant_option_order(&self, property_id: &str) -> Option<Vec<SharedString>>;
    fn component_property_name(&self, property_id: &str) -> Option<SharedString>;
    fn component_variant_option_name(
        &self,
        property_id: &str,
        option_id: &str,
    ) -> Option<SharedString>;
    fn order_successor(order: &[SharedString], id: &str) -> Option<SharedString>;
    fn reconcile_component_authoring_edit_sessions(&mut self, cx: &mut Context<Self>);
    fn reconcile_component_authoring_state(&mut self, cx: &mut Context<Self>);
    fn open_component_authoring_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn cancel_component_authoring_dialog_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn begin_component_property_create(
        &mut self,
        kind: DesignComponentPropertyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn cancel_component_property_create(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn close_component_authoring_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn submit_component_property_create(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn component_authoring_slot_limits(&self, cx: &App) -> Option<(Option<u32>, Option<u32>)>;
    fn open_component_property_edit(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn finish_component_property_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn cancel_component_authoring_edit_transactions(&mut self, cx: &mut Context<Self>);
    fn cancel_component_authoring_for_workspace_change(&mut self, cx: &mut Context<Self>);
    fn begin_component_property_rename(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn begin_component_variant_option_rename(
        &mut self,
        property_id: SharedString,
        option_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn begin_component_variant_option_create(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn preview_component_authoring_name(&mut self, cx: &mut Context<Self>);
    fn finish_component_authoring_name_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn request_component_property_delete(
        &mut self,
        property_id: SharedString,
        cx: &mut Context<Self>,
    );
    fn begin_component_property_reorder(
        &mut self,
        drag: &ComponentPropertyDefinitionDrag,
        cx: &mut Context<Self>,
    );
    fn preview_component_property_reorder(
        &mut self,
        before_property_id: Option<SharedString>,
        cx: &mut Context<Self>,
    );
    fn finish_component_property_reorder(&mut self, commit: bool, cx: &mut Context<Self>);
    fn begin_component_variant_option_reorder(
        &mut self,
        drag: &ComponentVariantOptionDrag,
        cx: &mut Context<Self>,
    );
    fn preview_component_variant_option_reorder(
        &mut self,
        before_option_id: Option<SharedString>,
        cx: &mut Context<Self>,
    );
    fn finish_component_variant_option_reorder(&mut self, commit: bool, cx: &mut Context<Self>);
    fn component_authoring_action_is_enabled(&self, action: &DesignPanelAction) -> bool;
    fn emit_component_authoring_edit_action(
        &mut self,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> bool;
    fn emit_component_authoring_action(&self, action: DesignPanelAction, cx: &mut Context<Self>);
    fn render_applied_component_property_controls(
        &self,
        surface: DesignComponentPropertyApplicationSurface,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement>;
    fn render_component_property_create_popover(&self, cx: &mut Context<Self>) -> AnyElement;
    fn component_authoring_uses_inline_modal_fallback(&self) -> bool;
    fn render_component_property_create_modal(
        &self,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement>;
    fn render_component_property_edit_modal(
        &self,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement>;
    fn render_component_definition_authoring(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn render_nested_component_property_exposures(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement>;
    fn render_slot_limits(
        &self,
        property: &DesignComponentProperty,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement>;
    fn render_component_property_row(
        &self,
        role: DesignComponentRole,
        index: usize,
        property: DesignComponentProperty,
        multiline_editor_active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn component_projection(&self) -> Option<sections::component::ComponentProjection>;
    fn render_component(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn component_action_is_enabled(&self, action: &DesignPanelAction) -> bool;
    fn emit_component_action(&self, action: DesignPanelAction, cx: &mut Context<Self>);
    fn render_component_action_button(
        &self,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

impl DesignComponentController for DesignPanel {
    fn emit_component_multiline_event(
        &self,
        event: DesignComponentMultilineEditEvent,
        cx: &mut Context<Self>,
    ) {
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentPropertyEditRequested {
                node_id: self.host.inspected_node().id.clone(),
                property_id: event.property_id,
                value: event.value,
                phase: event.phase,
            },
        );
    }

    fn component_property_index(&self, property_id: &str) -> Option<usize> {
        self.host
            .inspected_node()
            .component_properties
            .iter()
            .position(|property| property.id.as_ref() == property_id)
    }

    fn component_property_variable_target(
        &self,
        index: usize,
    ) -> Option<DesignComponentPropertyVariableTarget> {
        let role = self.host.inspected_node().component_role()?;
        self.host
            .inspected_node()
            .component_properties
            .get(index)?
            .variable_target(role)
    }

    fn component_property_variable_can_change(&self, index: usize) -> bool {
        if !self.can_edit() {
            return false;
        }
        let Some(role) = self.host.inspected_node().component_role() else {
            return false;
        };
        let Some(property) = self.host.inspected_node().component_properties.get(index) else {
            return false;
        };
        if property.variable_target(role).is_none() {
            return false;
        }
        self.host
            .property_states
            .get(&DesignPanelProperty::ComponentProperty(index))
            .is_none_or(|state| !state.is_read_only() && state.binding().is_none())
    }

    fn emit_component_property_variable_apply(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.component_property_variable_target(index) else {
            return;
        };
        let Some(variable) = self
            .resources
            .property_variables
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.component_property_variable_can_change(index)
            || variable.resolved_type != target.resolved_type
            || variable.disabled_reason.is_some()
            || variable.import_state == DesignVariableImportState::Available
        {
            return;
        }
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyVariable);
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentPropertyVariableApplyRequested {
                node_id: self.host.inspected_node().id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    fn emit_component_property_variable_import(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.component_property_variable_target(index) else {
            return;
        };
        let Some(variable) = self
            .resources
            .property_variables
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.component_property_variable_can_change(index)
            || variable.resolved_type != target.resolved_type
            || variable.disabled_reason.is_some()
            || variable.import_state != DesignVariableImportState::Available
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentPropertyVariableImportRequested {
                node_id: self.host.inspected_node().id.clone(),
                target,
                variable_id,
            },
        );
    }

    fn emit_component_property_variable_detach(
        &mut self,
        index: usize,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.component_property_variable_target(index) else {
            return;
        };
        let Some(binding) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .and_then(|property| property.variable_binding(target.field))
            .filter(|binding| {
                binding.can_detach && binding.variable_id.as_ref() == variable_id.as_ref()
            })
        else {
            return;
        };
        if !self.component_property_variable_can_change(index) {
            return;
        }
        let variable_id = binding.variable_id.clone();
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyVariable);
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentPropertyVariableDetachRequested {
                node_id: self.host.inspected_node().id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    fn open_component_property_variable_picker(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.component_property_variable_target(index) else {
            return;
        };
        if self.overlays.component_property_variable_picker().as_ref() != Some(&target) {
            self.remember_overlay_focus_return(
                DesignOpenOverlay::ComponentPropertyVariable,
                window,
                cx,
            );
        }
        self.overlays
            .open(DesignOverlayState::ComponentPropertyVariable(target));
        self.cancel_menu_preview(cx);
        self.retained
            .inputs
            .component_property_variable_search
            .update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        cx.notify();
    }

    fn component_swap_can_change(&self, index: usize) -> bool {
        self.host
            .inspected_node()
            .component_properties
            .get(index)
            .is_some_and(|property| {
                matches!(
                    property.definition,
                    DesignComponentPropertyDefinition::InstanceSwap { .. }
                )
            })
            && self.property_is_editable(DesignPanelProperty::ComponentProperty(index))
    }

    fn emit_component_swap_apply(
        &mut self,
        index: usize,
        selection: Option<DesignComponentSwapSelection>,
        cx: &mut Context<Self>,
    ) {
        if !self.component_swap_can_change(index)
            || selection.as_ref().is_some_and(|selection| {
                !self
                    .resources
                    .component_swaps
                    .candidate(selection)
                    .is_some_and(DesignComponentSwapCandidate::can_apply)
            })
        {
            return;
        }
        let Some(property_id) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .map(|property| property.id.clone())
        else {
            return;
        };
        self.clear_component_swap_preview(property_id.as_ref(), cx);
        self.overlays.discard(DesignOpenOverlay::ComponentSwap);
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentSwapApplyRequested {
                node_id: self.host.inspected_node().id.clone(),
                property_id,
                selection,
            },
        );
        cx.notify();
    }

    fn emit_component_swap_import(
        &mut self,
        index: usize,
        selection: DesignComponentSwapSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.component_swap_can_change(index)
            || !self
                .resources
                .component_swaps
                .candidate(&selection)
                .is_some_and(DesignComponentSwapCandidate::can_import)
        {
            return;
        }
        let Some(property_id) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .map(|property| property.id.clone())
        else {
            return;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ComponentSwapImportRequested {
                node_id: self.host.inspected_node().id.clone(),
                property_id,
                selection,
            },
        );
    }

    fn set_component_swap_preview(
        &mut self,
        index: usize,
        selection: Option<DesignComponentSwapSelection>,
        cx: &mut Context<Self>,
    ) {
        if !self.component_swap_can_change(index) {
            return;
        }
        let Some(property_id) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .map(|property| property.id.clone())
        else {
            return;
        };
        if selection.as_ref().is_some_and(|selection| {
            !self
                .resources
                .component_swaps
                .candidate(selection)
                .is_some_and(DesignComponentSwapCandidate::can_apply)
        }) {
            return;
        }
        if self.features.component.swap_hovered.as_ref()
            == selection
                .as_ref()
                .map(|selection| (property_id.clone(), selection.clone()))
                .as_ref()
        {
            return;
        }
        if let Some((previous_property_id, _)) = self.features.component.swap_hovered.take() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: previous_property_id,
                    selection: None,
                },
            );
        }
        if let Some(selection) = selection {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: property_id.clone(),
                    selection: Some(selection.clone()),
                },
            );
            self.features.component.swap_hovered = Some((property_id, selection));
        }
    }

    fn clear_component_swap_preview(&mut self, property_id: &str, cx: &mut Context<Self>) {
        let should_clear = self
            .features
            .component
            .swap_hovered
            .as_ref()
            .is_some_and(|(current, _)| current.as_ref() == property_id);
        if should_clear {
            self.features.component.swap_hovered = None;
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: property_id.to_owned().into(),
                    selection: None,
                },
            );
        }
    }

    fn open_component_swap_browser(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(property_id) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .filter(|property| {
                matches!(
                    property.definition,
                    DesignComponentPropertyDefinition::InstanceSwap { .. }
                )
            })
            .map(|property| property.id.clone())
        else {
            return;
        };
        if self.overlays.component_swap_browser().as_ref() != Some(&property_id) {
            self.remember_overlay_focus_return(DesignOpenOverlay::ComponentSwap, window, cx);
        }
        self.overlays
            .open(DesignOverlayState::ComponentSwap(property_id));
        self.cancel_menu_preview(cx);
        self.retained
            .inputs
            .component_swap_search
            .update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        cx.notify();
    }

    fn open_component_multiline_editor(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // This is the lowest-level entry point and is also used by tests and
        // non-pointer command paths. Guard it here—not only in the visual
        // wrapper—so a second activation can never overwrite an unmatched
        // Begin transaction.
        if self.edit.component_multiline.is_some() {
            return;
        }
        if !self.property_is_editable(DesignPanelProperty::ComponentProperty(index)) {
            return;
        }
        let Some(property) = self.host.inspected_node().component_properties.get(index) else {
            return;
        };
        if !matches!(
            property.definition,
            DesignComponentPropertyDefinition::Text {
                multiline: true,
                ..
            }
        ) {
            return;
        }
        let DesignComponentPropertyValue::Text(value) = property.effective_value() else {
            return;
        };
        let property_id = property.id.clone();
        let value = value.clone();
        self.edit.focus_return = None;
        let editor = ComponentMultilineEditor::new(property_id, value.clone());
        let Some(begin) = self
            .edit
            .begin_component_multiline_edit(editor, value.clone())
        else {
            return;
        };
        self.overlays.discard(DesignOpenOverlay::ComponentSwap);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyVariable);
        self.emit_component_multiline_event(begin, cx);
        self.retained
            .inputs
            .component_multiline
            .update(cx, |input, cx| {
                input.set_value(value, window, cx);
                input.focus(window, cx);
            });
        cx.notify();
    }

    fn open_component_multiline_editor_from_control(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edit.component_multiline.is_some() {
            return;
        }
        let Some(property_id) = self
            .host
            .inspected_node()
            .component_properties
            .get(index)
            .map(|property| property.id.clone())
        else {
            return;
        };
        let return_focus = window.focused(cx).map(|handle| EditorFocusReturn {
            origin: EditorFocusOrigin::ComponentMultiline(property_id.clone()),
            handle,
        });
        self.open_component_multiline_editor(index, window, cx);
        if self
            .edit
            .component_multiline
            .as_ref()
            .is_some_and(|editor| editor.property_id == property_id)
        {
            self.edit.focus_return = return_focus;
        }
    }

    fn preview_component_multiline(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.edit.component_multiline.as_ref() else {
            return;
        };
        let property_id = editor.property_id.clone();
        let Some(index) = self.component_property_index(property_id.as_ref()) else {
            return;
        };
        if !self.property_is_editable(DesignPanelProperty::ComponentProperty(index)) {
            return;
        }
        let value = self.retained.inputs.component_multiline.read(cx).value();
        let Some(preview) = self
            .edit
            .preview_component_multiline_edit(property_id.as_ref(), value)
        else {
            return;
        };
        self.emit_component_multiline_event(preview, cx);
        cx.notify();
    }

    fn finish_component_multiline_editor(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(property_id) = self
            .edit
            .component_multiline
            .as_ref()
            .map(|editor| editor.property_id.clone())
        else {
            return;
        };
        let return_focus = self.component_multiline_return_focus(property_id.as_ref());
        let value = commit.then(|| self.retained.inputs.component_multiline.read(cx).value());
        if let Some(event) = self.edit.finish_component_multiline_edit(value) {
            self.emit_component_multiline_event(event, cx);
        }
        Self::defer_editor_focus(
            return_focus.unwrap_or_else(|| self.focus_handle.clone()),
            window,
            cx,
        );
        cx.notify();
    }

    fn component_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(SharedString, DesignComponentPropertyValue)> {
        let DesignPanelProperty::ComponentProperty(index) = property else {
            return None;
        };
        let property = self.host.inspected_node().component_properties.get(index)?;
        let value = match value {
            DesignPanelValue::Text(value) => property.value_from_display(value.clone())?,
            DesignPanelValue::Bool(value)
                if matches!(
                    property.definition,
                    DesignComponentPropertyDefinition::Boolean { .. }
                ) =>
            {
                DesignComponentPropertyValue::Boolean(*value)
            }
            _ => return None,
        };
        Some((property.id.clone(), value))
    }

    fn slot_settings_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(
        SharedString,
        super::super::DesignSlotSettings,
        DesignSlotSettingsChange,
    )> {
        let (index, change) = match (property, value) {
            (
                DesignPanelProperty::SlotStretchChildOnInsert(index),
                DesignPanelValue::Bool(value),
            ) => (
                index,
                DesignSlotSettingsChange::StretchChildOnInsert(*value),
            ),
            (DesignPanelProperty::SlotDisplayEmpty(index), DesignPanelValue::Bool(value)) => {
                (index, DesignSlotSettingsChange::DisplayEmpty(*value))
            }
            (
                DesignPanelProperty::SlotMinimumInstances(index),
                DesignPanelValue::Integer(value),
            ) => (
                index,
                DesignSlotSettingsChange::MinimumInstances(Some(u32::try_from(*value).ok()?)),
            ),
            (DesignPanelProperty::SlotMinimumInstances(index), DesignPanelValue::Number(value)) => {
                (
                    index,
                    DesignSlotSettingsChange::MinimumInstances(Some(
                        u32::try_from(round_to_integer(f64::from(*value)).ok()? as i64).ok()?,
                    )),
                )
            }
            (
                DesignPanelProperty::SlotMinimumInstances(index),
                DesignPanelValue::OptionalNumber(value),
            ) => (
                index,
                DesignSlotSettingsChange::MinimumInstances(match value {
                    Some(value) => {
                        Some(u32::try_from(round_to_integer(f64::from(*value)).ok()? as i64).ok()?)
                    }
                    None => None,
                }),
            ),
            (
                DesignPanelProperty::SlotMaximumInstances(index),
                DesignPanelValue::OptionalNumber(value),
            ) => (
                index,
                DesignSlotSettingsChange::MaximumInstances(match value {
                    Some(value) => {
                        Some(u32::try_from(round_to_integer(f64::from(*value)).ok()? as i64).ok()?)
                    }
                    None => None,
                }),
            ),
            (
                DesignPanelProperty::SlotPreferredValuesOnly(index),
                DesignPanelValue::Bool(value),
            ) => (index, DesignSlotSettingsChange::PreferredValuesOnly(*value)),
            _ => return None,
        };
        let property = self.host.inspected_node().component_properties.get(index)?;
        let expected_settings = property.slot_settings()?.clone();
        Some((property.id.clone(), expected_settings, change))
    }

    fn component_property_with_index(
        property: DesignPanelProperty,
        index: usize,
    ) -> DesignPanelProperty {
        match property {
            DesignPanelProperty::ComponentProperty(_) => {
                DesignPanelProperty::ComponentProperty(index)
            }
            DesignPanelProperty::SlotStretchChildOnInsert(_) => {
                DesignPanelProperty::SlotStretchChildOnInsert(index)
            }
            DesignPanelProperty::SlotDisplayEmpty(_) => {
                DesignPanelProperty::SlotDisplayEmpty(index)
            }
            DesignPanelProperty::SlotMinimumInstances(_) => {
                DesignPanelProperty::SlotMinimumInstances(index)
            }
            DesignPanelProperty::SlotMaximumInstances(_) => {
                DesignPanelProperty::SlotMaximumInstances(index)
            }
            DesignPanelProperty::SlotPreferredValuesOnly(_) => {
                DesignPanelProperty::SlotPreferredValuesOnly(index)
            }
            property => property,
        }
    }

    fn cancel_component_multiline_transaction(&mut self, cx: &mut Context<Self>) {
        if let Some(event) = self.edit.finish_component_multiline_edit(None) {
            if self.edit.focus_return.as_ref().is_some_and(|return_focus| {
                matches!(
                    &return_focus.origin,
                    EditorFocusOrigin::ComponentMultiline(property_id)
                        if property_id == &event.property_id
                )
            }) {
                self.edit.focus_return = None;
            }
            self.emit_component_multiline_event(event, cx);
        }
    }

    fn cancel_component_swap_preview(&mut self, cx: &mut Context<Self>) {
        if let Some((property_id, _)) = self.features.component.swap_hovered.take() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id,
                    selection: None,
                },
            );
        }
    }

    fn render_component_property_variable_button(
        &self,
        index: usize,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement> {
        let target = self.component_property_variable_target(index)?;
        let property = self.host.inspected_node().component_properties.get(index)?;
        let binding = property.variable_binding(target.field).cloned();
        let open = self.overlays.component_property_variable_picker().as_ref() == Some(&target);
        let can_change = self.component_property_variable_can_change(index);
        let panel_for_open = panel.clone();
        let panel_for_content = panel.clone();
        let search_input = self
            .retained
            .inputs
            .component_property_variable_search
            .clone();
        let query = search_input.read(cx).value();
        let mut groups: Vec<(DesignVariableSource, Vec<DesignVariable>)> = Vec::new();
        for variable in self
            .resources
            .property_variables
            .variables
            .iter()
            .filter(|variable| {
                variable.resolved_type == target.resolved_type
                    && variable.matches_search(query.as_ref())
            })
            .cloned()
        {
            if let Some((_, variables)) = groups
                .iter_mut()
                .find(|(source, _)| *source == variable.source)
            {
                variables.push(variable);
            } else {
                groups.push((variable.source.clone(), vec![variable]));
            }
        }
        let property_id = target.property_id.clone();
        let target_for_open = target.clone();
        let trigger_tooltip = binding.as_ref().map_or_else(
            || format!("Apply variable to {}", target.field.api_name()).into(),
            |binding| binding.variable_name.clone(),
        );
        let trigger = Button::new(SharedString::from(format!(
            "{}-component-variable-{}-{}",
            self.id,
            property.id,
            target.field.api_name()
        )))
        .label("Select")
        .tooltip(trigger_tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(20.))
        .h(px(ROW_HEIGHT))
        .selected(open || binding.is_some())
        .on_activate(move |_, window, cx| {
            let property_id = property_id.clone();
            panel.update(cx, |this, cx| {
                if let Some(index) = this.component_property_index(property_id.as_ref()) {
                    this.open_component_property_variable_picker(index, window, cx);
                }
            });
        });

        Some(
            Popover::new(SharedString::from(format!(
                "{}-component-variable-popover-{}-{}",
                self.id,
                property.id,
                target.field.api_name()
            )))
            .anchor(Anchor::TopRight)
            .open(open)
            .overlay_closable(true)
            .on_open_change(move |is_open, window, cx| {
                let target = target_for_open.clone();
                panel_for_open.update(cx, |this, cx| {
                    if *is_open {
                        if let Some(index) =
                            this.component_property_index(target.property_id.as_ref())
                        {
                            this.open_component_property_variable_picker(index, window, cx);
                        }
                    } else if this.overlays.component_property_variable_picker().as_ref()
                        == Some(&target)
                    {
                        let _ = this.dismiss_overlay_from_outside_click(
                            DesignOpenOverlay::ComponentPropertyVariable,
                            window,
                            cx,
                        );
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let mut content = v_flex()
                    .w(popup_width(window, 292.))
                    .max_h(popup_height(window, 420.))
                    .gap_1()
                    .p_2()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .child(div().text_sm().font_semibold().child("Variables"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(target.field.api_name()),
                            ),
                    )
                    .child(
                        Input::new(&search_input)
                            .small()
                            .prefix(Icon::new(IconName::Search).small()),
                    );
                if let Some(binding) = binding.clone() {
                    let panel = panel_for_content.clone();
                    let property_id = target.property_id.clone();
                    let variable_id = binding.variable_id.clone();
                    content = content.child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .px_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .child(binding.variable_name),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "component-variable-detach-{property_id}"
                                )))
                                .label("Detach")
                                .xsmall()
                                .compact()
                                .ghost()
                                .disabled(!can_change || !binding.can_detach)
                                .on_activate(move |_, _, cx| {
                                    let property_id = property_id.clone();
                                    panel.update(cx, |this, cx| {
                                        if let Some(index) =
                                            this.component_property_index(property_id.as_ref())
                                        {
                                            this.emit_component_property_variable_detach(
                                                index,
                                                variable_id.clone(),
                                                cx,
                                            );
                                        }
                                    });
                                }),
                            ),
                    );
                }
                if groups.is_empty() {
                    content = content.child(
                        div()
                            .px_1()
                            .py_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("No compatible variables"),
                    );
                }
                let mut rows = v_flex()
                    .w_full()
                    .max_h(popup_height(window, 300.))
                    .overflow_y_scrollbar();
                for (source, variables) in groups.clone() {
                    let heading = match source {
                        DesignVariableSource::Page { page_name, .. } => {
                            format!("This page · {page_name}")
                        }
                        DesignVariableSource::Library { library_name, .. } => {
                            format!("Library · {library_name}")
                        }
                    };
                    rows = rows.child(
                        div()
                            .px_1()
                            .pt_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(heading),
                    );
                    for variable in variables {
                        let panel = panel_for_content.clone();
                        let property_id = target.property_id.clone();
                        let variable_id = variable.id.clone();
                        let available =
                            variable.import_state == DesignVariableImportState::Available;
                        let selected = binding.as_ref().is_some_and(|binding| {
                            binding.variable_id.as_ref() == variable.id.as_ref()
                        });
                        rows = rows.child(
                            Button::new(SharedString::from(format!(
                                "component-variable-{}-{}",
                                property_id, variable.id
                            )))
                            .label(if available {
                                format!("Import {} / {}", variable.collection_name, variable.name)
                            } else {
                                format!("{} / {}", variable.collection_name, variable.name)
                            })
                            .tooltip(
                                variable
                                    .disabled_reason
                                    .clone()
                                    .unwrap_or_else(|| target.resolved_type.label().into()),
                            )
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(selected)
                            .disabled(!can_change || variable.disabled_reason.is_some())
                            .on_activate(move |_, _, cx| {
                                let property_id = property_id.clone();
                                panel.update(cx, |this, cx| {
                                    if let Some(index) =
                                        this.component_property_index(property_id.as_ref())
                                    {
                                        if available {
                                            this.emit_component_property_variable_import(
                                                index,
                                                variable_id.clone(),
                                                cx,
                                            );
                                        } else {
                                            this.emit_component_property_variable_apply(
                                                index,
                                                variable_id.clone(),
                                                cx,
                                            );
                                        }
                                    }
                                });
                            }),
                        );
                    }
                }
                content.child(rows)
            })
            .w(px(20.))
            .h(px(ROW_HEIGHT))
            .into_any_element(),
        )
    }

    fn render_component_swap_browser(
        &self,
        index: usize,
        property: &super::super::DesignComponentProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = match property.effective_value() {
            DesignComponentPropertyValue::InstanceSwap(reference) => reference,
            _ => None,
        };
        let preferred_ids = match &property.definition {
            DesignComponentPropertyDefinition::InstanceSwap {
                preferred_values, ..
            } => preferred_values
                .iter()
                .map(|reference| reference.id.clone())
                .collect::<HashSet<_>>(),
            _ => HashSet::new(),
        };
        let query = self.retained.inputs.component_swap_search.read(cx).value();
        let matches = self
            .resources
            .component_swaps
            .matching(query.as_ref())
            .cloned()
            .collect::<Vec<_>>();
        let mut groups: Vec<(SharedString, Vec<DesignComponentSwapCandidate>)> = Vec::new();
        let preferred = matches
            .iter()
            .filter(|candidate| preferred_ids.contains(&candidate.reference.id))
            .cloned()
            .collect::<Vec<_>>();
        if !preferred.is_empty() {
            groups.push(("Preferred".into(), preferred));
        }
        for candidate in matches
            .into_iter()
            .filter(|candidate| !preferred_ids.contains(&candidate.reference.id))
        {
            let label: SharedString = match &candidate.source {
                DesignComponentSource::Page { page_name, .. } => {
                    format!("Local · {page_name}").into()
                }
                DesignComponentSource::Library { library_name, .. } => {
                    format!("Library · {library_name}").into()
                }
            };
            if let Some((_, candidates)) = groups.iter_mut().find(|(heading, _)| *heading == label)
            {
                candidates.push(candidate);
            } else {
                groups.push((label, vec![candidate]));
            }
        }
        let open = self.overlays.component_swap_browser().as_ref() == Some(&property.id);
        let can_change = self.component_swap_can_change(index);
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel.clone();
        let property_id = property.id.clone();
        let property_id_for_open = property.id.clone();
        let property_id_for_content = property.id.clone();
        let search_input = self.retained.inputs.component_swap_search.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-component-swap-{}",
            self.id, property.id
        )))
        .label(
            current
                .as_ref()
                .map_or_else(|| "None".into(), |reference| reference.name.clone()),
        )
        .tooltip("Swap instance")
        .xsmall()
        .compact()
        .w_full()
        .h(px(ROW_HEIGHT))
        .on_activate(move |_, window, cx| {
            let property_id = property_id.clone();
            panel.update(cx, |this, cx| {
                if let Some(index) = this.component_property_index(property_id.as_ref()) {
                    this.open_component_swap_browser(index, window, cx);
                }
            });
        });

        Popover::new(SharedString::from(format!(
            "{}-component-swap-popover-{}",
            self.id, property.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .on_open_change(move |is_open, window, cx| {
            let property_id = property_id_for_open.clone();
            panel_for_open.update(cx, |this, cx| {
                if *is_open {
                    if let Some(index) = this.component_property_index(property_id.as_ref()) {
                        this.open_component_swap_browser(index, window, cx);
                    }
                } else if this.overlays.component_swap_browser().as_ref() == Some(&property_id) {
                    let _ = this.dismiss_overlay_from_outside_click(
                        DesignOpenOverlay::ComponentSwap,
                        window,
                        cx,
                    );
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let browser_property_id = property_id_for_content.clone();
            let mut content = v_flex()
                .w(popup_width(window, 320.))
                .max_h(popup_height(window, 460.))
                .gap_1()
                .p_2()
                .child(div().text_sm().font_semibold().child("Swap instance"))
                .child(
                    Input::new(&search_input)
                        .small()
                        .prefix(Icon::new(IconName::Search).small()),
                );
            let panel = panel_for_content.clone();
            let property_id = browser_property_id.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "component-swap-none-{property_id}"
                )))
                .label("None")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .selected(current.is_none())
                .disabled(!can_change)
                .on_activate(move |_, _, cx| {
                    let property_id = property_id.clone();
                    panel.update(cx, |this, cx| {
                        if let Some(index) = this.component_property_index(property_id.as_ref()) {
                            this.emit_component_swap_apply(index, None, cx);
                        }
                    });
                }),
            );
            if groups.is_empty() {
                content = content.child(
                    div()
                        .px_1()
                        .py_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No components found"),
                );
            }
            let mut rows = v_flex()
                .w_full()
                .max_h(popup_height(window, 340.))
                .overflow_y_scrollbar();
            for (heading, candidates) in groups.clone() {
                rows = rows.child(
                    div()
                        .px_1()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(heading),
                );
                for candidate in candidates {
                    let selection = candidate.selection();
                    let applyable = candidate.can_apply();
                    let importable = candidate.can_import();
                    let panel_for_click = panel_for_content.clone();
                    let panel_for_hover = panel_for_content.clone();
                    let property_id_for_click = browser_property_id.clone();
                    let property_id_for_hover = browser_property_id.clone();
                    let selection_for_click = selection.clone();
                    let selection_for_hover = selection.clone();
                    let selected = current
                        .as_ref()
                        .is_some_and(|reference| reference.id == candidate.reference.id);
                    let label = if importable {
                        format!("Import {}", candidate.reference.name)
                    } else {
                        candidate.reference.name.to_string()
                    };
                    let tooltip = candidate.disabled_reason.clone().unwrap_or_else(|| {
                        format!(
                            "{} · {} · {}",
                            candidate.source.label(),
                            candidate.asset_kind.api_name(),
                            candidate.reference.availability.label()
                        )
                        .into()
                    });
                    rows = rows.child(
                        div()
                            .id(SharedString::from(format!(
                                "component-swap-hover-{}-{}",
                                browser_property_id, candidate.component_key
                            )))
                            .w_full()
                            .on_hover(move |hovered, _, cx| {
                                let property_id = property_id_for_hover.clone();
                                let selection = selection_for_hover.clone();
                                panel_for_hover.update(cx, |this, cx| {
                                    if let Some(index) =
                                        this.component_property_index(property_id.as_ref())
                                    {
                                        this.set_component_swap_preview(
                                            index,
                                            hovered.then_some(selection),
                                            cx,
                                        );
                                    }
                                });
                            })
                            .child(
                                Button::new(SharedString::from(format!(
                                    "component-swap-{}-{}",
                                    browser_property_id, candidate.component_key
                                )))
                                .label(label)
                                .tooltip(tooltip)
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .selected(selected)
                                .disabled(!can_change || (!applyable && !importable))
                                .on_activate(move |_, _, cx| {
                                    let property_id = property_id_for_click.clone();
                                    let selection = selection_for_click.clone();
                                    panel_for_click.update(cx, |this, cx| {
                                        if let Some(index) =
                                            this.component_property_index(property_id.as_ref())
                                        {
                                            if importable {
                                                this.emit_component_swap_import(
                                                    index, selection, cx,
                                                );
                                            } else {
                                                this.emit_component_swap_apply(
                                                    index,
                                                    Some(selection),
                                                    cx,
                                                );
                                            }
                                        }
                                    });
                                }),
                            ),
                    );
                }
            }
            content.child(rows)
        })
        .w_full()
        .h(px(ROW_HEIGHT))
        .into_any_element()
    }

    fn component_authoring_view_data(&self) -> Option<&DesignComponentAuthoringViewData> {
        self.host
            .inspected_node()
            .component_role()
            .filter(|role| role.can_author_component_properties())?;
        self.host
            .inspected_node()
            .component_context
            .as_ref()
            .and_then(|context| context.authoring.as_ref())
    }

    fn component_property_partition_order(
        &self,
        partition: DesignComponentPropertyPartition,
    ) -> Vec<SharedString> {
        self.host
            .inspected_node()
            .component_properties
            .iter()
            .filter(|property| {
                DesignComponentPropertyPartition::for_kind(property.definition.kind()) == partition
            })
            .map(|property| property.id.clone())
            .collect()
    }

    fn reconcile_slot_limits_state(&mut self) {
        let valid = self
            .component_authoring
            .open_slot_limits
            .as_ref()
            .is_some_and(|property_id| {
                self.host
                    .inspected_node()
                    .component_role()
                    .is_some_and(DesignComponentRole::can_modify_slot_instances)
                    && self
                        .host
                        .inspected_node()
                        .component_properties
                        .iter()
                        .find(|property| &property.id == property_id)
                        .is_some_and(|property| !slot_limit_guidelines(property).is_empty())
            });
        if !valid {
            self.component_authoring.open_slot_limits = None;
        }
    }

    fn component_variant_option_order(&self, property_id: &str) -> Option<Vec<SharedString>> {
        Some(
            self.component_authoring_view_data()?
                .definition(property_id)?
                .variant_options
                .iter()
                .map(|option| option.id.clone())
                .collect(),
        )
    }

    fn component_property_name(&self, property_id: &str) -> Option<SharedString> {
        self.host
            .inspected_node()
            .component_properties
            .iter()
            .find(|property| property.id.as_ref() == property_id)
            .map(|property| property.name.clone())
    }

    fn component_variant_option_name(
        &self,
        property_id: &str,
        option_id: &str,
    ) -> Option<SharedString> {
        self.component_authoring_view_data()?
            .definition(property_id)?
            .variant_options
            .iter()
            .find(|option| option.id.as_ref() == option_id)
            .map(|option| option.name.clone())
    }

    fn order_successor(order: &[SharedString], id: &str) -> Option<SharedString> {
        order
            .iter()
            .position(|candidate| candidate.as_ref() == id)
            .and_then(|index| order.get(index + 1))
            .cloned()
    }

    fn reconcile_component_authoring_edit_sessions(&mut self, cx: &mut Context<Self>) {
        let current_node_id = self.host.inspected_node().id.clone();

        let name_resolution = self
            .edit
            .component_authoring
            .name_editor()
            .cloned()
            .map(|editor| match editor {
                ComponentAuthoringNameEditor::Property { property_id, .. } => {
                    let current_name = self.component_property_name(property_id.as_ref());
                    let editable = self.can_edit()
                        && self
                            .component_authoring_view_data()
                            .and_then(|authoring| authoring.definition(property_id.as_ref()))
                            .is_some_and(|definition| definition.capabilities.rename);
                    (current_name, editable)
                }
                ComponentAuthoringNameEditor::VariantOption {
                    property_id,
                    option_id,
                    ..
                } => {
                    let current_name = self
                        .component_variant_option_name(property_id.as_ref(), option_id.as_ref());
                    let editable = self.can_edit()
                        && self
                            .component_authoring_view_data()
                            .and_then(|authoring| authoring.definition(property_id.as_ref()))
                            .is_some_and(|definition| {
                                definition.capabilities.edit_variant_options
                                    && definition
                                        .variant_options
                                        .iter()
                                        .find(|option| option.id == option_id)
                                        .is_some_and(|option| option.can_rename)
                            });
                    (current_name, editable)
                }
                ComponentAuthoringNameEditor::NewVariantOption { property_id, .. } => {
                    let editable = self.can_edit()
                        && self
                            .component_authoring_view_data()
                            .and_then(|authoring| authoring.definition(property_id.as_ref()))
                            .is_some_and(|definition| definition.capabilities.edit_variant_options);
                    (None, editable)
                }
            });
        if let Some((current_name, editable)) = name_resolution
            && let Some(action) = self.edit.component_authoring.reconcile_name_edit(
                &current_node_id,
                current_name,
                editable,
            )
        {
            cx.emit_design_panel_action(self, action);
        }

        let property_reorder_resolution = self
            .edit
            .component_authoring
            .property_reorder()
            .cloned()
            .map(|session| {
                let current_order = self
                    .component_authoring_view_data()
                    .map(|_| self.component_property_partition_order(session.partition));
                let editable = self.can_edit()
                    && self
                        .component_authoring_view_data()
                        .is_some_and(|authoring| {
                            authoring.preserves_variant_partition(
                                &self.host.inspected_node().component_properties,
                            ) && self
                                .host
                                .inspected_node()
                                .component_properties
                                .iter()
                                .find(|property| property.id == session.property_id)
                                .filter(|property| {
                                    DesignComponentPropertyPartition::for_kind(
                                        property.definition.kind(),
                                    ) == session.partition
                                })
                                .and_then(|property| authoring.definition(property.id.as_ref()))
                                .is_some_and(|definition| definition.capabilities.reorder)
                        });
                (current_order, editable)
            });
        if let Some((current_order, editable)) = property_reorder_resolution
            && let Some(action) = self.edit.component_authoring.reconcile_property_reorder(
                &current_node_id,
                current_order,
                editable,
            )
        {
            cx.emit_design_panel_action(self, action);
        }

        let option_reorder_resolution = self
            .edit
            .component_authoring
            .variant_option_reorder()
            .cloned()
            .map(|session| {
                let current_order =
                    self.component_variant_option_order(session.property_id.as_ref());
                let editable = self.can_edit()
                    && self
                        .component_authoring_view_data()
                        .and_then(|authoring| authoring.definition(session.property_id.as_ref()))
                        .is_some_and(|definition| {
                            definition.capabilities.edit_variant_options
                                && definition
                                    .variant_options
                                    .iter()
                                    .find(|option| option.id == session.option_id)
                                    .is_some_and(|option| option.can_reorder)
                        });
                (current_order, editable)
            });
        if let Some((current_order, editable)) = option_reorder_resolution
            && let Some(action) = self
                .edit
                .component_authoring
                .reconcile_variant_option_reorder(&current_node_id, current_order, editable)
        {
            cx.emit_design_panel_action(self, action);
        }
    }

    fn reconcile_component_authoring_state(&mut self, cx: &mut Context<Self>) {
        self.reconcile_component_authoring_edit_sessions(cx);
        let authoring_available = self.can_edit() && self.component_authoring_view_data().is_some();
        if !authoring_available {
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyCreateMenu);
            self.component_authoring.create_draft = None;
            self.component_authoring.edit_draft = None;
            self.component_authoring.selected_property = None;
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
            self.edit.component_authoring.clear_after_cancellation();
            return;
        }

        let property_exists = |this: &Self, property_id: &SharedString| {
            this.component_property_name(property_id.as_ref()).is_some()
                && this
                    .component_authoring_view_data()
                    .and_then(|authoring| authoring.definition(property_id.as_ref()))
                    .is_some()
        };
        if self
            .component_authoring
            .edit_draft
            .as_ref()
            .is_some_and(|draft| !property_exists(self, &draft.property_id))
        {
            self.component_authoring.edit_draft = None;
        }
        if self
            .component_authoring
            .selected_property
            .as_ref()
            .is_some_and(|property_id| !property_exists(self, property_id))
        {
            self.component_authoring.selected_property = None;
        }
        if self
            .overlays
            .component_property_context_menu()
            .as_ref()
            .is_some_and(|property_id| !property_exists(self, property_id))
        {
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
        }
    }

    fn open_component_authoring_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlays.component_authoring_dialog_open() {
            return;
        }
        self.overlays
            .open(DesignOverlayState::ComponentPropertyEdit);
        // Native dialogs restore to the stable inspector root. The create
        // path has already prepared/focused its retained input before the
        // dialog is mounted, so sampling `window.focused` here would capture
        // a transient child instead of the opening surface.
        self.overlays.remember_focus_return(
            DesignOpenOverlay::ComponentPropertyEdit,
            self.focus_handle.clone(),
        );
        self.component_authoring.dialog_close_pending = false;
        #[cfg(test)]
        {
            self.component_authoring.dialog_last_rendered_kind = None;
        }
        let panel = cx.entity();
        let panel_for_cancel = panel.clone();
        let panel_for_close = panel.clone();
        window.open_dialog(cx, move |dialog: Dialog, window, cx| {
            let content = panel.update(cx, |panel_ref, cx| {
                #[cfg(test)]
                {
                    panel_ref.component_authoring.dialog_last_rendered_kind = panel_ref
                        .component_authoring
                        .create_draft
                        .as_ref()
                        .map(|draft| draft.kind)
                        .or_else(|| {
                            panel_ref
                                .component_authoring
                                .edit_draft
                                .as_ref()
                                .map(|draft| draft.definition.kind())
                        });
                }
                panel_ref
                    .render_component_property_create_modal(panel.clone(), cx)
                    .or_else(|| panel_ref.render_component_property_edit_modal(panel.clone(), cx))
                    .unwrap_or_else(|| dialogs::unavailable_editor(cx))
            });
            let panel_for_cancel = panel_for_cancel.clone();
            let panel_for_close = panel_for_close.clone();
            dialogs::configure_host(dialog, window)
                .on_cancel(move |_, window, cx| {
                    panel_for_cancel.update(cx, |this, cx| {
                        this.cancel_component_authoring_dialog_state(window, cx);
                    });
                    true
                })
                .on_close(move |_, _, cx| {
                    panel_for_close.update(cx, |this, cx| {
                        this.overlays
                            .discard(DesignOpenOverlay::ComponentPropertyEdit);
                        this.component_authoring.dialog_close_pending = false;
                        this.overlays
                            .forget_focus_return(DesignOpenOverlay::ComponentPropertyEdit);
                        #[cfg(test)]
                        {
                            this.component_authoring.dialog_last_rendered_kind = None;
                        }
                        cx.notify();
                    });
                })
                .child(content)
        });
    }

    /// Applies the state transition shared by the native Dialog's Cancel
    /// action and its Escape-key binding. The Dialog host closes the overlay
    /// after the callback returns `true`.
    fn cancel_component_authoring_dialog_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_component_authoring_edit_transactions(cx);
        let dismissal = self
            .overlays
            .escape_dismissal_intent_for(DesignOpenOverlay::ComponentPropertyEdit);
        self.component_authoring.create_draft = None;
        self.component_authoring.edit_draft = None;
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyEdit);
        self.component_authoring.dialog_close_pending = false;
        #[cfg(test)]
        {
            self.component_authoring.dialog_last_rendered_kind = None;
        }
        if let Some(dismissal) = dismissal {
            if let Some(focus_return) = self.overlays.finish_dismissal(&dismissal) {
                focus_return.restore(window, cx);
            }
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn begin_component_property_create(
        &mut self,
        kind: DesignComponentPropertyKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self
                .component_authoring_view_data()
                .is_some_and(|authoring| authoring.create_kinds.contains(&kind))
        {
            return;
        }
        self.cancel_component_authoring_edit_transactions(cx);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyCreateMenu);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
        let seed = editors::create_editor_seed(kind);
        self.component_authoring.create_draft = Some(seed.draft);
        self.component_authoring.name_input.update(cx, |input, cx| {
            input.set_value(seed.name, window, cx);
            input.focus(window, cx);
        });
        self.component_authoring
            .default_input
            .update(cx, |input, cx| {
                input.set_value(seed.default_or_description, window, cx);
            });
        self.component_authoring
            .slot_minimum_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.component_authoring
            .slot_maximum_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.open_component_authoring_dialog(window, cx);
        cx.notify();
    }

    fn cancel_component_property_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.component_authoring.create_draft.take().is_some() {
            self.close_component_authoring_dialog(window, cx);
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    fn close_component_authoring_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlays.component_authoring_dialog_open() {
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyEdit);
            self.component_authoring.dialog_close_pending = false;
            #[cfg(test)]
            {
                self.component_authoring.dialog_last_rendered_kind = None;
            }
            if window.has_active_dialog(cx) {
                window.close_dialog(cx);
            }
        }
    }

    fn submit_component_property_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.component_authoring.create_draft.clone() else {
            return;
        };
        let name = self.component_authoring.name_input.read(cx).value();
        let default = self.component_authoring.default_input.read(cx).value();
        let slot_minimum = self.component_authoring.slot_minimum_input.read(cx).value();
        let slot_maximum = self.component_authoring.slot_maximum_input.read(cx).value();
        let Some(prepared) =
            editors::prepare_create(draft, name, default, slot_minimum, slot_maximum)
        else {
            return;
        };
        let draft = prepared.draft;
        let partition = DesignComponentPropertyPartition::for_kind(draft.kind);
        let expected_property_order = self.component_property_partition_order(partition);
        let action = DesignPanelAction::ComponentPropertyDefinitionCreateRequested {
            node_id: self.host.inspected_node().id.clone(),
            kind: draft.kind,
            name: prepared.name,
            description: draft.description,
            documentation_links: draft.documentation_links,
            definition: draft.definition,
            default_variable_id: draft.default_variable_id,
            partition,
            after_property_id: expected_property_order.last().cloned(),
            expected_property_order,
        };
        if self.component_authoring_action_is_enabled(&action) {
            cx.emit_design_panel_action(self, action);
            self.component_authoring.create_draft = None;
            self.close_component_authoring_dialog(window, cx);
            self.focus_handle.focus(window, cx);
            cx.notify();
        }
    }

    fn component_authoring_slot_limits(&self, cx: &App) -> Option<(Option<u32>, Option<u32>)> {
        editors::parse_slot_limits(
            self.component_authoring.slot_minimum_input.read(cx).value(),
            self.component_authoring.slot_maximum_input.read(cx).value(),
        )
    }

    fn open_component_property_edit(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(property) = self
            .host
            .inspected_node()
            .component_properties
            .iter()
            .find(|property| property.id == property_id)
            .cloned()
        else {
            return;
        };
        let Some(definition) = self
            .component_authoring_view_data()
            .and_then(|authoring| authoring.definition(property_id.as_ref()))
        else {
            return;
        };
        if !(definition.capabilities.edit_metadata
            || definition.capabilities.edit_default_value
            || definition.capabilities.edit_preferred_values
            || definition.capabilities.edit_slot_settings
            || definition.capabilities.edit_variant_options)
        {
            return;
        }
        self.cancel_component_authoring_edit_transactions(cx);
        let seed = editors::edit_editor_seed(
            property_id,
            property.description,
            property.documentation_links,
            property.definition,
        );
        self.component_authoring.edit_draft = Some(seed.draft);
        self.component_authoring
            .default_input
            .update(cx, |input, cx| {
                input.set_value(seed.default_or_description, window, cx);
            });
        self.component_authoring
            .slot_minimum_input
            .update(cx, |input, cx| {
                input.set_value(seed.slot_minimum, window, cx);
            });
        self.component_authoring
            .slot_maximum_input
            .update(cx, |input, cx| {
                input.set_value(seed.slot_maximum, window, cx);
            });
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
        self.open_component_authoring_dialog(window, cx);
        cx.notify();
    }

    fn finish_component_property_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = self.component_authoring.edit_draft.take() else {
            return;
        };
        if commit {
            let default = self.component_authoring.default_input.read(cx).value();
            let slot_minimum = self.component_authoring.slot_minimum_input.read(cx).value();
            let slot_maximum = self.component_authoring.slot_maximum_input.read(cx).value();
            let draft = match editors::prepare_edit(draft, default, slot_minimum, slot_maximum) {
                editors::ComponentPropertyEditPreparation::Ready(draft) => draft,
                editors::ComponentPropertyEditPreparation::Invalid(draft) => {
                    self.component_authoring.edit_draft = Some(draft);
                    return;
                }
            };
            if editors::edit_has_changes(&draft) {
                self.emit_component_authoring_action(
                    DesignPanelAction::ComponentPropertyDefinitionEditRequested {
                        node_id: self.host.inspected_node().id.clone(),
                        property_id: draft.property_id,
                        expected_description: draft.expected_description,
                        description: draft.description,
                        expected_documentation_links: draft.expected_documentation_links,
                        documentation_links: draft.documentation_links,
                        expected_definition: draft.expected_definition,
                        definition: draft.definition,
                    },
                    cx,
                );
            }
        }
        self.cancel_component_authoring_edit_transactions(cx);
        self.close_component_authoring_dialog(window, cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_component_authoring_edit_transactions(&mut self, cx: &mut Context<Self>) {
        let expected_name = self
            .edit
            .component_authoring
            .name_editor()
            .and_then(|editor| match editor {
                ComponentAuthoringNameEditor::Property { property_id, .. } => {
                    self.component_property_name(property_id.as_ref())
                }
                ComponentAuthoringNameEditor::VariantOption {
                    property_id,
                    option_id,
                    ..
                } => self.component_variant_option_name(property_id.as_ref(), option_id.as_ref()),
                ComponentAuthoringNameEditor::NewVariantOption { .. } => None,
            });
        if let Some(action) = self
            .edit
            .component_authoring
            .cancel_name_edit(expected_name)
        {
            cx.emit_design_panel_action(self, action);
        }

        let property_order = self
            .edit
            .component_authoring
            .property_reorder()
            .map(|session| self.component_property_partition_order(session.partition));
        if let Some(action) = self
            .edit
            .component_authoring
            .cancel_property_reorder(property_order)
        {
            cx.emit_design_panel_action(self, action);
        }

        let option_order = self
            .edit
            .component_authoring
            .variant_option_reorder()
            .and_then(|session| self.component_variant_option_order(session.property_id.as_ref()));
        if let Some(action) = self
            .edit
            .component_authoring
            .cancel_variant_option_reorder(option_order)
        {
            cx.emit_design_panel_action(self, action);
        }
    }

    /// Cancels every phased component-authoring interaction before the
    /// workspace hides the Design inspector.
    ///
    /// This deliberately emits balanced `Cancel` intents while the matching
    /// transient session is still present, then drops presentation-only
    /// menus, drafts, and selection state.
    fn cancel_component_authoring_for_workspace_change(&mut self, cx: &mut Context<Self>) {
        self.cancel_component_authoring_edit_transactions(cx);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyCreateMenu);
        self.component_authoring.create_draft = None;
        self.component_authoring.edit_draft = None;
        self.component_authoring.selected_property = None;
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyVariable);
        self.overlays.discard(DesignOpenOverlay::ComponentSwap);
        self.features.component.swap_hovered = None;
        if self.overlays.component_authoring_dialog_open() {
            self.overlays
                .discard(DesignOpenOverlay::ComponentPropertyEdit);
            self.component_authoring.dialog_close_pending = true;
            #[cfg(test)]
            {
                self.component_authoring.dialog_last_rendered_kind = None;
            }
        }
        cx.notify();
    }

    fn begin_component_property_rename(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(original_name) = self.component_property_name(property_id.as_ref()) else {
            return;
        };
        let action = DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
            node_id: self.host.inspected_node().id.clone(),
            property_id,
            original_name: original_name.clone(),
            expected_name: original_name.clone(),
            name: original_name.clone(),
            phase: DesignPanelEditPhase::Begin,
        };
        if !self.emit_component_authoring_edit_action(action, cx) {
            return;
        }
        self.component_authoring.name_input.update(cx, |input, cx| {
            input.set_value(original_name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn begin_component_variant_option_rename(
        &mut self,
        property_id: SharedString,
        option_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(original_name) =
            self.component_variant_option_name(property_id.as_ref(), option_id.as_ref())
        else {
            return;
        };
        let action = DesignPanelAction::ComponentVariantOptionRenameRequested {
            node_id: self.host.inspected_node().id.clone(),
            property_id,
            option_id,
            original_name: original_name.clone(),
            expected_name: original_name.clone(),
            name: original_name.clone(),
            phase: DesignPanelEditPhase::Begin,
        };
        if !self.emit_component_authoring_edit_action(action, cx) {
            return;
        }
        self.component_authoring.name_input.update(cx, |input, cx| {
            input.set_value(original_name, window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn begin_component_variant_option_create(
        &mut self,
        property_id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(order) = self.component_variant_option_order(property_id.as_ref()) else {
            return;
        };
        if !self.edit.component_authoring.begin_new_variant_option(
            self.host.inspected_node().id.clone(),
            property_id,
            order.last().cloned(),
        ) {
            return;
        }
        self.component_authoring.name_input.update(cx, |input, cx| {
            input.set_value(format!("Value {}", order.len() + 1), window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn preview_component_authoring_name(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.edit.component_authoring.name_editor().cloned() else {
            return;
        };
        let candidate = self.component_authoring.name_input.read(cx).value();
        if candidate.trim().is_empty() {
            return;
        }
        let action = match &editor {
            ComponentAuthoringNameEditor::Property {
                node_id,
                property_id,
                original_name,
                last_preview,
                ..
            } => {
                if last_preview.as_ref() == Some(&candidate) || candidate == *original_name {
                    return;
                }
                let Some(expected_name) = self.component_property_name(property_id.as_ref()) else {
                    return;
                };
                DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    original_name: original_name.clone(),
                    expected_name,
                    name: candidate.clone(),
                    phase: DesignPanelEditPhase::Preview,
                }
            }
            ComponentAuthoringNameEditor::VariantOption {
                node_id,
                property_id,
                option_id,
                original_name,
                last_preview,
                ..
            } => {
                if last_preview.as_ref() == Some(&candidate) || candidate == *original_name {
                    return;
                }
                let Some(expected_name) =
                    self.component_variant_option_name(property_id.as_ref(), option_id.as_ref())
                else {
                    return;
                };
                DesignPanelAction::ComponentVariantOptionRenameRequested {
                    node_id: node_id.clone(),
                    property_id: property_id.clone(),
                    option_id: option_id.clone(),
                    original_name: original_name.clone(),
                    expected_name,
                    name: candidate.clone(),
                    phase: DesignPanelEditPhase::Preview,
                }
            }
            ComponentAuthoringNameEditor::NewVariantOption { .. } => return,
        };
        let _ = self.emit_component_authoring_edit_action(action, cx);
    }

    fn finish_component_authoring_name_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.edit.component_authoring.name_editor().cloned() else {
            return;
        };
        let candidate = self.component_authoring.name_input.read(cx).value();
        if let ComponentAuthoringNameEditor::NewVariantOption {
            node_id,
            property_id,
            after_option_id,
        } = editor
        {
            if commit && !candidate.trim().is_empty() {
                let Some(expected_option_order) =
                    self.component_variant_option_order(property_id.as_ref())
                else {
                    self.edit.component_authoring.clear_unphased_name_editor();
                    return;
                };
                let action = DesignPanelAction::ComponentVariantOptionCreateRequested {
                    node_id,
                    property_id,
                    name: candidate.trim().to_owned().into(),
                    expected_option_order,
                    after_option_id,
                };
                self.emit_component_authoring_action(action, cx);
            }
            self.edit.component_authoring.clear_unphased_name_editor();
            self.focus_handle.focus(window, cx);
            cx.notify();
            return;
        }

        let nonempty_commit = commit && !candidate.trim().is_empty();
        let action = match editor {
            ComponentAuthoringNameEditor::Property {
                node_id,
                property_id,
                original_name,
                ..
            } => {
                let Some(expected_name) = self.component_property_name(property_id.as_ref()) else {
                    if let Some(action) = self.edit.component_authoring.cancel_name_edit(None) {
                        cx.emit_design_panel_action(self, action);
                    }
                    return;
                };
                DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                    node_id,
                    property_id,
                    original_name: original_name.clone(),
                    expected_name,
                    name: if nonempty_commit {
                        candidate.trim().to_owned().into()
                    } else {
                        original_name
                    },
                    phase: if nonempty_commit {
                        DesignPanelEditPhase::Commit
                    } else {
                        DesignPanelEditPhase::Cancel
                    },
                }
            }
            ComponentAuthoringNameEditor::VariantOption {
                node_id,
                property_id,
                option_id,
                original_name,
                ..
            } => {
                let Some(expected_name) =
                    self.component_variant_option_name(property_id.as_ref(), option_id.as_ref())
                else {
                    if let Some(action) = self.edit.component_authoring.cancel_name_edit(None) {
                        cx.emit_design_panel_action(self, action);
                    }
                    return;
                };
                DesignPanelAction::ComponentVariantOptionRenameRequested {
                    node_id,
                    property_id,
                    option_id,
                    original_name: original_name.clone(),
                    expected_name,
                    name: if nonempty_commit {
                        candidate.trim().to_owned().into()
                    } else {
                        original_name
                    },
                    phase: if nonempty_commit {
                        DesignPanelEditPhase::Commit
                    } else {
                        DesignPanelEditPhase::Cancel
                    },
                }
            }
            ComponentAuthoringNameEditor::NewVariantOption { .. } => unreachable!(),
        };
        if !self.emit_component_authoring_edit_action(action, cx)
            && let Some(action) = self.edit.component_authoring.cancel_name_edit(None)
        {
            cx.emit_design_panel_action(self, action);
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn request_component_property_delete(
        &mut self,
        property_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(expected_name) = self.component_property_name(property_id.as_ref()) else {
            return;
        };
        self.emit_component_authoring_action(
            DesignPanelAction::ComponentPropertyDefinitionDeleteRequested {
                node_id: self.host.inspected_node().id.clone(),
                property_id,
                expected_name,
            },
            cx,
        );
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
    }

    fn begin_component_property_reorder(
        &mut self,
        drag: &ComponentPropertyDefinitionDrag,
        cx: &mut Context<Self>,
    ) {
        if drag.node_id != self.host.inspected_node().id {
            return;
        }
        let expected_property_order = self.component_property_partition_order(drag.partition);
        if expected_property_order != drag.original_order {
            return;
        }
        let before_property_id =
            Self::order_successor(&expected_property_order, drag.property_id.as_ref());
        let action = DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            node_id: self.host.inspected_node().id.clone(),
            property_id: drag.property_id.clone(),
            partition: drag.partition,
            original_property_order: drag.original_order.clone(),
            expected_property_order,
            before_property_id,
            phase: DesignPanelEditPhase::Begin,
        };
        let _ = self.emit_component_authoring_edit_action(action, cx);
    }

    fn preview_component_property_reorder(
        &mut self,
        before_property_id: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.edit.component_authoring.property_reorder().cloned() else {
            return;
        };
        if session.last_before_property_id == before_property_id
            || before_property_id.as_ref() == Some(&session.property_id)
        {
            return;
        }
        let action = DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            node_id: session.node_id,
            property_id: session.property_id,
            partition: session.partition,
            original_property_order: session.original_order,
            expected_property_order: self.component_property_partition_order(session.partition),
            before_property_id: before_property_id.clone(),
            phase: DesignPanelEditPhase::Preview,
        };
        let _ = self.emit_component_authoring_edit_action(action, cx);
    }

    fn finish_component_property_reorder(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self.edit.component_authoring.property_reorder().cloned() else {
            return;
        };
        let before_property_id = if commit {
            session.last_before_property_id.clone()
        } else {
            Self::order_successor(&session.original_order, session.property_id.as_ref())
        };
        let action = DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            node_id: session.node_id,
            property_id: session.property_id,
            partition: session.partition,
            original_property_order: session.original_order,
            expected_property_order: self.component_property_partition_order(session.partition),
            before_property_id,
            phase: if commit {
                DesignPanelEditPhase::Commit
            } else {
                DesignPanelEditPhase::Cancel
            },
        };
        if !self.emit_component_authoring_edit_action(action, cx)
            && let Some(action) = self.edit.component_authoring.cancel_property_reorder(None)
        {
            cx.emit_design_panel_action(self, action);
        }
        cx.notify();
    }

    fn begin_component_variant_option_reorder(
        &mut self,
        drag: &ComponentVariantOptionDrag,
        cx: &mut Context<Self>,
    ) {
        if drag.node_id != self.host.inspected_node().id {
            return;
        }
        let Some(expected_option_order) =
            self.component_variant_option_order(drag.property_id.as_ref())
        else {
            return;
        };
        if expected_option_order != drag.original_order {
            return;
        }
        let before_option_id =
            Self::order_successor(&expected_option_order, drag.option_id.as_ref());
        let action = DesignPanelAction::ComponentVariantOptionReorderRequested {
            node_id: self.host.inspected_node().id.clone(),
            property_id: drag.property_id.clone(),
            option_id: drag.option_id.clone(),
            original_option_order: drag.original_order.clone(),
            expected_option_order,
            before_option_id,
            phase: DesignPanelEditPhase::Begin,
        };
        let _ = self.emit_component_authoring_edit_action(action, cx);
    }

    fn preview_component_variant_option_reorder(
        &mut self,
        before_option_id: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .edit
            .component_authoring
            .variant_option_reorder()
            .cloned()
        else {
            return;
        };
        if session.last_before_option_id == before_option_id
            || before_option_id.as_ref() == Some(&session.option_id)
        {
            return;
        }
        let Some(expected_option_order) =
            self.component_variant_option_order(session.property_id.as_ref())
        else {
            return;
        };
        let action = DesignPanelAction::ComponentVariantOptionReorderRequested {
            node_id: session.node_id,
            property_id: session.property_id,
            option_id: session.option_id,
            original_option_order: session.original_order,
            expected_option_order,
            before_option_id: before_option_id.clone(),
            phase: DesignPanelEditPhase::Preview,
        };
        let _ = self.emit_component_authoring_edit_action(action, cx);
    }

    fn finish_component_variant_option_reorder(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(session) = self
            .edit
            .component_authoring
            .variant_option_reorder()
            .cloned()
        else {
            return;
        };
        let Some(expected_option_order) =
            self.component_variant_option_order(session.property_id.as_ref())
        else {
            if let Some(action) = self
                .edit
                .component_authoring
                .cancel_variant_option_reorder(None)
            {
                cx.emit_design_panel_action(self, action);
            }
            return;
        };
        let before_option_id = if commit {
            session.last_before_option_id.clone()
        } else {
            Self::order_successor(&session.original_order, session.option_id.as_ref())
        };
        let action = DesignPanelAction::ComponentVariantOptionReorderRequested {
            node_id: session.node_id,
            property_id: session.property_id,
            option_id: session.option_id,
            original_option_order: session.original_order,
            expected_option_order,
            before_option_id,
            phase: if commit {
                DesignPanelEditPhase::Commit
            } else {
                DesignPanelEditPhase::Cancel
            },
        };
        if !self.emit_component_authoring_edit_action(action, cx)
            && let Some(action) = self
                .edit
                .component_authoring
                .cancel_variant_option_reorder(None)
        {
            cx.emit_design_panel_action(self, action);
        }
        cx.notify();
    }

    fn component_authoring_action_is_enabled(&self, action: &DesignPanelAction) -> bool {
        if !self.can_edit() {
            return false;
        }
        let Some(authoring) = self.component_authoring_view_data() else {
            return false;
        };
        let property = |property_id: &SharedString| {
            self.host
                .inspected_node()
                .component_properties
                .iter()
                .find(|property| &property.id == property_id)
        };
        let authored_property = |property_id: &SharedString| {
            property(property_id).zip(authoring.definition(property_id.as_ref()))
        };
        let node_matches = |node_id: &SharedString| node_id == &self.host.inspected_node().id;
        let component_reference_is_current =
            |reference: &super::super::DesignComponentReference| {
                self.resources
                    .component_swaps
                    .candidates
                    .iter()
                    .any(|candidate| candidate.can_apply() && candidate.reference == *reference)
            };
        let component_references_are_current =
            |references: &[super::super::DesignComponentReference]| {
                let unique_ids = references
                    .iter()
                    .map(|reference| reference.id.as_ref())
                    .collect::<HashSet<_>>();
                unique_ids.len() == references.len()
                    && references.iter().all(component_reference_is_current)
            };
        match action {
            DesignPanelAction::ComponentPropertyDefinitionCreateRequested {
                node_id,
                kind,
                name,
                description,
                documentation_links,
                definition,
                default_variable_id,
                partition,
                expected_property_order,
                after_property_id,
            } => {
                let expected_partition = DesignComponentPropertyPartition::for_kind(*kind);
                let current_order = self.component_property_partition_order(expected_partition);
                let definition_is_valid = match definition {
                    DesignComponentPropertyDefinition::Variant {
                        default_value,
                        options,
                    } => {
                        !options.is_empty()
                            && options.iter().any(|option| option == default_value)
                            && options.iter().all(|option| !option.trim().is_empty())
                    }
                    DesignComponentPropertyDefinition::InstanceSwap {
                        default_value,
                        preferred_values,
                    } => {
                        default_value
                            .as_ref()
                            .is_none_or(component_reference_is_current)
                            && component_references_are_current(preferred_values)
                    }
                    DesignComponentPropertyDefinition::Slot { settings, .. } => {
                        settings.has_valid_child_range()
                            && component_references_are_current(&settings.preferred_values)
                    }
                    DesignComponentPropertyDefinition::Text { .. }
                    | DesignComponentPropertyDefinition::Boolean { .. } => true,
                };
                let variable_is_valid = match (kind, default_variable_id) {
                    (
                        DesignComponentPropertyKind::Boolean | DesignComponentPropertyKind::Text,
                        Some(variable_id),
                    ) => {
                        let expected_type = if *kind == DesignComponentPropertyKind::Boolean {
                            super::super::DesignVariableResolvedType::Boolean
                        } else {
                            super::super::DesignVariableResolvedType::String
                        };
                        self.resources
                            .property_variables
                            .variable(variable_id.as_ref())
                            .is_some_and(|variable| {
                                variable.resolved_type == expected_type
                                    && variable.import_state != DesignVariableImportState::Available
                                    && variable.disabled_reason.is_none()
                                    && variable.resolved_value.is_some()
                            })
                    }
                    (
                        DesignComponentPropertyKind::InstanceSwap
                        | DesignComponentPropertyKind::Variant
                        | DesignComponentPropertyKind::Slot,
                        Some(_),
                    ) => false,
                    (_, None) => true,
                };
                node_matches(node_id)
                    && authoring.preserves_variant_partition(
                        &self.host.inspected_node().component_properties,
                    )
                    && authoring.create_kinds.contains(kind)
                    && definition.kind() == *kind
                    && definition_is_valid
                    && variable_is_valid
                    && !name.trim().is_empty()
                    && (description.is_none()
                        || (*kind == DesignComponentPropertyKind::Slot
                            && description
                                .as_ref()
                                .is_some_and(|description| !description.trim().is_empty())))
                    && (*kind == DesignComponentPropertyKind::Slot
                        || documentation_links.is_empty())
                    && *partition == expected_partition
                    && *expected_property_order == current_order
                    && after_property_id.as_ref() == current_order.last()
            }
            DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                node_id,
                property_id,
                original_name,
                expected_name,
                name,
                phase,
            } => {
                let editor_matches = self
                    .edit
                    .component_authoring
                    .name_editor()
                    .is_some_and(|editor| {
                        matches!(
                            editor,
                            ComponentAuthoringNameEditor::Property {
                                property_id: active_property_id,
                                original_name: active_original,
                                ..
                            } if active_property_id == property_id && active_original == original_name
                        )
                    });
                let phase_is_valid = match phase {
                    DesignPanelEditPhase::Begin => {
                        name == original_name && expected_name == original_name
                    }
                    DesignPanelEditPhase::Preview | DesignPanelEditPhase::Commit => {
                        !name.trim().is_empty()
                    }
                    DesignPanelEditPhase::Cancel => name == original_name,
                };
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        definition.capabilities.rename && property.name == *expected_name
                    })
                    && (*phase == DesignPanelEditPhase::Begin || editor_matches)
                    && phase_is_valid
            }
            DesignPanelAction::ComponentPropertyDefinitionMetadataEditRequested {
                node_id,
                property_id,
                expected_description,
                description,
                expected_documentation_links,
                documentation_links,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        definition.capabilities.edit_metadata
                            && property.description == *expected_description
                            && property.documentation_links == *expected_documentation_links
                            && description
                                .as_ref()
                                .is_none_or(|description| !description.trim().is_empty())
                            && documentation_links.iter().all(|link| {
                                !link.label.trim().is_empty() && !link.url.trim().is_empty()
                            })
                    })
            }
            DesignPanelAction::ComponentPropertyDefinitionEditRequested {
                node_id,
                property_id,
                expected_description,
                description,
                expected_documentation_links,
                documentation_links,
                expected_definition,
                definition: next_definition,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(
                        |(property, authoring_definition)| {
                            if property.definition != *expected_definition
                                || property.description != *expected_description
                                || property.documentation_links != *expected_documentation_links
                                || next_definition.kind() != expected_definition.kind()
                            {
                                return false;
                            }
                            let metadata_changed = description != expected_description
                                || documentation_links != expected_documentation_links;
                            let definition_changed = next_definition != expected_definition;
                            if !metadata_changed && !definition_changed {
                                return false;
                            }
                            if metadata_changed
                                && (!authoring_definition.capabilities.edit_metadata
                                    || description
                                        .as_ref()
                                        .is_some_and(|description| description.trim().is_empty())
                                    || documentation_links.iter().any(|link| {
                                        link.label.trim().is_empty() || link.url.trim().is_empty()
                                    }))
                            {
                                return false;
                            }
                            if !definition_changed {
                                return true;
                            }
                            match (expected_definition, next_definition) {
                                (
                                    DesignComponentPropertyDefinition::Boolean { .. },
                                    DesignComponentPropertyDefinition::Boolean { .. },
                                ) => authoring_definition.capabilities.edit_default_value,
                                (
                                    DesignComponentPropertyDefinition::Text {
                                        multiline: expected_multiline,
                                        ..
                                    },
                                    DesignComponentPropertyDefinition::Text {
                                        multiline: next_multiline,
                                        ..
                                    },
                                ) => {
                                    authoring_definition.capabilities.edit_default_value
                                        && expected_multiline == next_multiline
                                }
                                (
                                    DesignComponentPropertyDefinition::InstanceSwap {
                                        default_value: expected_default,
                                        preferred_values: expected_preferred,
                                    },
                                    DesignComponentPropertyDefinition::InstanceSwap {
                                        default_value,
                                        preferred_values,
                                    },
                                ) => {
                                    (default_value == expected_default
                                        || authoring_definition.capabilities.edit_default_value)
                                        && (preferred_values == expected_preferred
                                            || authoring_definition
                                                .capabilities
                                                .edit_preferred_values)
                                        && default_value
                                            .as_ref()
                                            .is_none_or(component_reference_is_current)
                                        && component_references_are_current(preferred_values)
                                }
                                (
                                    DesignComponentPropertyDefinition::Slot {
                                        default_value: expected_default,
                                        ..
                                    },
                                    DesignComponentPropertyDefinition::Slot {
                                        default_value,
                                        settings,
                                    },
                                ) => {
                                    authoring_definition.capabilities.edit_slot_settings
                                        && default_value == expected_default
                                        && settings.has_valid_child_range()
                                        && component_references_are_current(
                                            &settings.preferred_values,
                                        )
                                }
                                _ => false,
                            }
                        },
                    )
            }
            DesignPanelAction::ComponentPropertyDefinitionDeleteRequested {
                node_id,
                property_id,
                expected_name,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        definition.capabilities.delete && property.name == *expected_name
                    })
            }
            DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
                node_id,
                property_id,
                partition,
                original_property_order,
                expected_property_order,
                before_property_id,
                phase,
            } => {
                let Some((property, definition)) = authored_property(property_id) else {
                    return false;
                };
                let expected_partition =
                    DesignComponentPropertyPartition::for_kind(property.definition.kind());
                let current_order = self.component_property_partition_order(expected_partition);
                let anchor_is_valid = before_property_id.as_ref().is_none_or(|before_id| {
                    before_id != property_id
                        && authored_property(before_id).is_some_and(|(before, _)| {
                            DesignComponentPropertyPartition::for_kind(before.definition.kind())
                                == expected_partition
                        })
                });
                let same_partition_members = component_authoring_orders_have_same_unique_members(
                    original_property_order,
                    &current_order,
                );
                let session_matches = self
                    .edit
                    .component_authoring
                    .property_reorder()
                    .is_some_and(|session| {
                        session.property_id == *property_id
                            && session.partition == *partition
                            && session.original_order == *original_property_order
                    });
                node_matches(node_id)
                    && authoring.preserves_variant_partition(
                        &self.host.inspected_node().component_properties,
                    )
                    && definition.capabilities.reorder
                    && *partition == expected_partition
                    && *expected_property_order == current_order
                    && same_partition_members
                    && (*phase == DesignPanelEditPhase::Begin || session_matches)
                    && anchor_is_valid
                    && matches!(
                        phase,
                        DesignPanelEditPhase::Begin
                            | DesignPanelEditPhase::Preview
                            | DesignPanelEditPhase::Commit
                            | DesignPanelEditPhase::Cancel
                    )
            }
            DesignPanelAction::ComponentVariantOptionCreateRequested {
                node_id,
                property_id,
                name,
                expected_option_order,
                after_option_id,
            } => {
                let Some((property, definition)) = authored_property(property_id) else {
                    return false;
                };
                let current_order = definition
                    .variant_options
                    .iter()
                    .map(|option| option.id.clone())
                    .collect::<Vec<_>>();
                node_matches(node_id)
                    && property.definition.kind() == DesignComponentPropertyKind::Variant
                    && definition.capabilities.edit_variant_options
                    && !name.trim().is_empty()
                    && *expected_option_order == current_order
                    && after_option_id.as_ref() == definition.variant_options.last().map(|o| &o.id)
            }
            DesignPanelAction::ComponentVariantOptionRenameRequested {
                node_id,
                property_id,
                option_id,
                original_name,
                expected_name,
                name,
                phase,
            } => {
                let editor_matches =
                    self.edit
                        .component_authoring
                        .name_editor()
                        .is_some_and(|editor| {
                            matches!(
                                editor,
                                ComponentAuthoringNameEditor::VariantOption {
                                    property_id: active_property_id,
                                    option_id: active_option_id,
                                    original_name: active_original,
                                    ..
                                } if active_property_id == property_id
                                    && active_option_id == option_id
                                    && active_original == original_name
                            )
                        });
                let phase_is_valid = match phase {
                    DesignPanelEditPhase::Begin => {
                        name == original_name && expected_name == original_name
                    }
                    DesignPanelEditPhase::Preview | DesignPanelEditPhase::Commit => {
                        !name.trim().is_empty()
                    }
                    DesignPanelEditPhase::Cancel => name == original_name,
                };
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        property.definition.kind() == DesignComponentPropertyKind::Variant
                            && definition.capabilities.edit_variant_options
                            && definition
                                .variant_options
                                .iter()
                                .find(|option| &option.id == option_id)
                                .is_some_and(|option| {
                                    option.can_rename && option.name == *expected_name
                                })
                    })
                    && (*phase == DesignPanelEditPhase::Begin || editor_matches)
                    && phase_is_valid
            }
            DesignPanelAction::ComponentVariantOptionDeleteRequested {
                node_id,
                property_id,
                option_id,
                expected_name,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        property.definition.kind() == DesignComponentPropertyKind::Variant
                            && definition.capabilities.edit_variant_options
                            && definition
                                .variant_options
                                .iter()
                                .find(|option| &option.id == option_id)
                                .is_some_and(|option| {
                                    option.can_delete && option.name == *expected_name
                                })
                    })
            }
            DesignPanelAction::ComponentVariantOptionReorderRequested {
                node_id,
                property_id,
                option_id,
                original_option_order,
                expected_option_order,
                before_option_id,
                phase,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        let current_order = definition
                            .variant_options
                            .iter()
                            .map(|option| option.id.clone())
                            .collect::<Vec<_>>();
                        let same_members = component_authoring_orders_have_same_unique_members(
                            original_option_order,
                            &current_order,
                        );
                        let session_matches = self
                            .edit
                            .component_authoring
                            .variant_option_reorder()
                            .is_some_and(|session| {
                                session.property_id == *property_id
                                    && session.option_id == *option_id
                                    && session.original_order == *original_option_order
                            });
                        property.definition.kind() == DesignComponentPropertyKind::Variant
                            && definition.capabilities.edit_variant_options
                            && *expected_option_order == current_order
                            && same_members
                            && (*phase == DesignPanelEditPhase::Begin || session_matches)
                            && definition
                                .variant_options
                                .iter()
                                .find(|option| &option.id == option_id)
                                .is_some_and(|option| option.can_reorder)
                            && before_option_id.as_ref().is_none_or(|before_id| {
                                before_id != option_id
                                    && definition
                                        .variant_options
                                        .iter()
                                        .any(|option| &option.id == before_id)
                            })
                            && matches!(
                                phase,
                                DesignPanelEditPhase::Begin
                                    | DesignPanelEditPhase::Preview
                                    | DesignPanelEditPhase::Commit
                                    | DesignPanelEditPhase::Cancel
                            )
                    })
            }
            DesignPanelAction::SlotSettingsChangeRequested {
                node_id,
                property_id,
                expected_settings,
                change: DesignSlotSettingsChange::Replace(settings),
                phase: DesignPanelEditPhase::Commit,
            } => {
                node_matches(node_id)
                    && authored_property(property_id).is_some_and(|(property, definition)| {
                        definition.capabilities.edit_slot_settings
                            && property.slot_settings() == Some(expected_settings)
                            && settings != expected_settings
                            && settings.has_valid_child_range()
                            && component_references_are_current(&settings.preferred_values)
                    })
            }
            DesignPanelAction::ComponentPropertyApplyToLayerRequested {
                node_id,
                control_id,
                layer_id,
                surface,
                property_id,
            } => {
                node_matches(node_id)
                    && authoring
                        .applied_control(control_id.as_ref())
                        .is_some_and(|control| {
                            &control.layer_id == layer_id
                                && control.surface == *surface
                                && control.applied_property_id.is_none()
                                && control.capabilities.apply
                                && control.candidate(property_id.as_ref()).is_some()
                                && property(property_id).is_some()
                        })
            }
            DesignPanelAction::ComponentPropertySwitchOnLayerRequested {
                node_id,
                control_id,
                layer_id,
                surface,
                from_property_id,
                to_property_id,
            } => {
                node_matches(node_id)
                    && from_property_id != to_property_id
                    && authoring
                        .applied_control(control_id.as_ref())
                        .is_some_and(|control| {
                            &control.layer_id == layer_id
                                && control.surface == *surface
                                && control.applied_property_id.as_ref() == Some(from_property_id)
                                && control.capabilities.switch
                                && control.candidate(to_property_id.as_ref()).is_some()
                                && property(to_property_id).is_some()
                        })
            }
            DesignPanelAction::ComponentPropertyDetachFromLayerRequested {
                node_id,
                control_id,
                layer_id,
                surface,
                property_id,
            } => {
                node_matches(node_id)
                    && authoring
                        .applied_control(control_id.as_ref())
                        .is_some_and(|control| {
                            &control.layer_id == layer_id
                                && control.surface == *surface
                                && control.applied_property_id.as_ref() == Some(property_id)
                                && control.capabilities.detach
                        })
            }
            DesignPanelAction::NestedComponentPropertyExposeRequested {
                node_id,
                candidate_id,
                nested_instance_id,
                nested_property_id,
            } => {
                node_matches(node_id)
                    && authoring
                        .exposure_candidate(candidate_id.as_ref())
                        .is_some_and(|candidate| {
                            &candidate.nested_instance_id == nested_instance_id
                                && &candidate.nested_property.property_id == nested_property_id
                                && candidate.exposed_property_id.is_none()
                                && candidate.capabilities.expose
                        })
            }
            DesignPanelAction::NestedComponentPropertyUnexposeRequested {
                node_id,
                candidate_id,
                nested_instance_id,
                nested_property_id,
                exposed_property_id,
            } => {
                node_matches(node_id)
                    && authoring
                        .exposure_candidate(candidate_id.as_ref())
                        .is_some_and(|candidate| {
                            &candidate.nested_instance_id == nested_instance_id
                                && &candidate.nested_property.property_id == nested_property_id
                                && candidate.exposed_property_id.as_ref()
                                    == Some(exposed_property_id)
                                && candidate.capabilities.unexpose
                        })
            }
            DesignPanelAction::NestedComponentPropertyPreviewRequested {
                node_id,
                candidate_id,
                nested_instance_id,
                nested_property_id,
                ..
            } => {
                node_matches(node_id)
                    && authoring
                        .exposure_candidate(candidate_id.as_ref())
                        .is_some_and(|candidate| {
                            &candidate.nested_instance_id == nested_instance_id
                                && &candidate.nested_property.property_id == nested_property_id
                                && candidate.capabilities.preview
                        })
            }
            _ => false,
        }
    }

    fn emit_component_authoring_edit_action(
        &mut self,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.component_authoring_action_is_enabled(&action)
            || !self.edit.component_authoring.track_action(&action)
        {
            return false;
        }
        cx.emit_design_panel_action(self, action);
        true
    }

    fn emit_component_authoring_action(&self, action: DesignPanelAction, cx: &mut Context<Self>) {
        if self.component_authoring_action_is_enabled(&action) {
            cx.emit_design_panel_action(self, action);
        }
    }

    fn render_applied_component_property_controls(
        &self,
        surface: DesignComponentPropertyApplicationSurface,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let controls = self
            .component_authoring_view_data()?
            .applied_properties
            .iter()
            .filter(|control| control.surface == surface)
            .cloned()
            .collect::<Vec<DesignAppliedComponentPropertyControl>>();
        if controls.is_empty() {
            return None;
        }
        let mut rows = v_flex().w_full().gap_1();
        for control in controls {
            let applied = control.applied_property().cloned();
            let next = control
                .candidates
                .iter()
                .find(|candidate| {
                    applied
                        .as_ref()
                        .is_none_or(|applied| candidate.property_id != applied.property_id)
                })
                .cloned();
            let pill_label: SharedString = applied.as_ref().map_or_else(
                || "Apply component property".into(),
                |property| format!("{} · {}", property.kind.label(), property.property_name).into(),
            );
            let mut actions = h_flex().gap_1();
            match (applied.as_ref(), next) {
                (None, Some(candidate)) => {
                    actions = actions.child(self.render_component_action_button(
                        format!("apply-property-{}", control.control_id),
                        "Apply",
                        DesignPanelAction::ComponentPropertyApplyToLayerRequested {
                            node_id: self.host.inspected_node().id.clone(),
                            control_id: control.control_id.clone(),
                            layer_id: control.layer_id.clone(),
                            surface,
                            property_id: candidate.property_id,
                        },
                        cx,
                    ));
                }
                (Some(current), Some(candidate)) => {
                    actions = actions.child(self.render_component_action_button(
                        format!("switch-property-{}", control.control_id),
                        "Switch",
                        DesignPanelAction::ComponentPropertySwitchOnLayerRequested {
                            node_id: self.host.inspected_node().id.clone(),
                            control_id: control.control_id.clone(),
                            layer_id: control.layer_id.clone(),
                            surface,
                            from_property_id: current.property_id.clone(),
                            to_property_id: candidate.property_id,
                        },
                        cx,
                    ));
                }
                _ => {}
            }
            if let Some(current) = applied {
                actions = actions.child(self.render_component_action_button(
                    format!("detach-property-{}", control.control_id),
                    "Detach",
                    DesignPanelAction::ComponentPropertyDetachFromLayerRequested {
                        node_id: self.host.inspected_node().id.clone(),
                        control_id: control.control_id.clone(),
                        layer_id: control.layer_id.clone(),
                        surface,
                        property_id: current.property_id,
                    },
                    cx,
                ));
            }
            rows = rows
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .w(px(88.))
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(control.layer_name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .h(px(ROW_HEIGHT))
                                .px_2()
                                .flex()
                                .items_center()
                                .rounded(px(999.))
                                .border_1()
                                .border_color(cx.theme().selection.opacity(0.55))
                                .bg(cx.theme().selection.opacity(0.16))
                                .text_color(cx.theme().selection)
                                .text_xs()
                                .child(div().truncate().child(pill_label)),
                        ),
                )
                .child(actions);
        }
        Some(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Applied property · {}", surface.label())),
                )
                .child(rows)
                .into_any_element(),
        )
    }

    fn render_component_property_create_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let authoring = self
            .component_authoring_view_data()
            .expect("create control requires authoring view data");
        let kinds = authoring.create_kinds.clone();
        let open = self.overlays.component_property_create_menu_open();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_for_keyboard = panel_for_open.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-component-property-create-menu-trigger",
            self.id
        )))
        .label("+")
        .tooltip("Create component property")
        .xsmall()
        .compact()
        .ghost()
        .selected(open)
        .disabled(kinds.is_empty())
        .on_keyboard_activate(move |_, cx| {
            panel_for_keyboard.update(cx, |this, cx| {
                this.overlays
                    .set_open(DesignOverlayState::ComponentPropertyCreateMenu, !open);
                cx.notify();
            });
        });

        Popover::new(SharedString::from(format!(
            "{}-component-property-create-menu",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .on_open_change(move |is_open, window, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *is_open {
                    this.remember_overlay_focus_return(
                        DesignOpenOverlay::ComponentPropertyCreateMenu,
                        window,
                        cx,
                    );
                    this.overlays
                        .open(DesignOverlayState::ComponentPropertyCreateMenu);
                    cx.notify();
                } else if this.overlays.component_property_create_menu_open() {
                    let _ = this.dismiss_overlay_from_outside_click(
                        DesignOpenOverlay::ComponentPropertyCreateMenu,
                        window,
                        cx,
                    );
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, _cx| {
            let mut menu = v_flex().w(popup_width(window, 208.)).p_1().gap_0p5().child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_semibold()
                    .child("Create component property"),
            );
            for kind in kinds.clone() {
                let panel = panel_for_content.clone();
                menu = menu.child(
                    Button::new(SharedString::from(format!(
                        "create-component-property-kind-{kind:?}"
                    )))
                    .label(kind.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .on_activate(move |_, window, cx| {
                        panel.update(cx, |this, cx| {
                            this.begin_component_property_create(kind, window, cx);
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
    }

    fn component_authoring_uses_inline_modal_fallback(&self) -> bool {
        !self.overlays.component_authoring_dialog_open()
            && !self.component_authoring.dialog_close_pending
            && (self.component_authoring.create_draft.is_some()
                || self.component_authoring.edit_draft.is_some())
    }

    fn render_component_property_create_modal(
        &self,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement> {
        let draft = self.component_authoring.create_draft.clone()?;
        let panel_for_cancel = panel.clone();
        let panel_for_submit = panel.clone();
        let name_invalid = self
            .component_authoring
            .name_input
            .read(cx)
            .value()
            .trim()
            .is_empty();
        let settings_invalid = draft.kind == DesignComponentPropertyKind::Slot
            && self.component_authoring_slot_limits(cx).is_none();
        let mut fields = v_flex()
            .w_full()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Name"),
            )
            .child(
                Input::new(&self.component_authoring.name_input)
                    .appearance(false)
                    .bordered(true)
                    .xsmall()
                    .w_full(),
            );

        match draft.kind {
            DesignComponentPropertyKind::Boolean => {
                let default_value = matches!(
                    draft.definition,
                    DesignComponentPropertyDefinition::Boolean {
                        default_value: true
                    }
                );
                let panel_for_toggle = panel.clone();
                fields = fields.child(
                    div()
                        .debug_selector(|| "component-property-default-boolean".to_owned())
                        .child(
                            Switch::new("component-property-default-boolean")
                                .label("Default value")
                                .tooltip("Boolean default value")
                                .xsmall()
                                .checked(default_value)
                                .on_click(move |checked, _, cx| {
                                    panel_for_toggle.update(cx, |this, cx| {
                                        if let Some(ComponentPropertyCreateDraft {
                                            definition:
                                                DesignComponentPropertyDefinition::Boolean {
                                                    default_value,
                                                },
                                            ..
                                        }) = this.component_authoring.create_draft.as_mut()
                                        {
                                            *default_value = *checked;
                                            cx.notify();
                                        }
                                    });
                                }),
                        ),
                );
            }
            DesignComponentPropertyKind::Text | DesignComponentPropertyKind::Variant => {
                fields = fields
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if draft.kind == DesignComponentPropertyKind::Variant {
                                "Default value and first Variant value"
                            } else {
                                "Default value"
                            }),
                    )
                    .child(
                        Input::new(&self.component_authoring.default_input)
                            .appearance(false)
                            .bordered(true)
                            .xsmall()
                            .w_full(),
                    );
            }
            DesignComponentPropertyKind::InstanceSwap => {
                let DesignComponentPropertyDefinition::InstanceSwap {
                    default_value,
                    preferred_values,
                } = &draft.definition
                else {
                    return None;
                };
                let panel_for_none = panel.clone();
                fields = fields
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Default and preferred instances"),
                    )
                    .child(
                        Button::new("component-property-instance-default-none")
                            .label("None")
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(default_value.is_none())
                            .on_activate(move |_, _, cx| {
                                panel_for_none.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyCreateDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::InstanceSwap {
                                                default_value,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.create_draft.as_mut()
                                    {
                                        *default_value = None;
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                for candidate in self
                    .resources
                    .component_swaps
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.can_apply())
                {
                    let reference = candidate.reference.clone();
                    let reference_for_default = reference.clone();
                    let reference_for_preferred = reference.clone();
                    let panel_for_default = panel.clone();
                    let panel_for_preferred = panel.clone();
                    let is_default = default_value.as_ref() == Some(&reference);
                    let is_preferred = preferred_values.contains(&reference);
                    fields = fields.child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .bg(cx.theme().secondary)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(cx.theme().selection)
                                    .child(render_lucide_icon(
                                        LucideIcon::Diamond,
                                        cx.theme().selection,
                                        16.,
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .child(reference.name.clone()),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "component-property-instance-default-{}",
                                    reference.id
                                )))
                                .label("Default")
                                .xsmall()
                                .compact()
                                .ghost()
                                .selected(is_default)
                                .on_activate(move |_, _, cx| {
                                    panel_for_default.update(cx, |this, cx| {
                                        if let Some(ComponentPropertyCreateDraft {
                                            definition:
                                                DesignComponentPropertyDefinition::InstanceSwap {
                                                    default_value,
                                                    ..
                                                },
                                            ..
                                        }) = this.component_authoring.create_draft.as_mut()
                                        {
                                            *default_value = Some(reference_for_default.clone());
                                            cx.notify();
                                        }
                                    });
                                }),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "component-property-instance-preferred-{}",
                                    reference.id
                                )))
                                .icon(if is_preferred {
                                    IconName::Check
                                } else {
                                    IconName::Plus
                                })
                                .tooltip("Preferred instance")
                                .xsmall()
                                .compact()
                                .ghost()
                                .selected(is_preferred)
                                .on_activate(move |_, _, cx| {
                                    panel_for_preferred.update(cx, |this, cx| {
                                        if let Some(ComponentPropertyCreateDraft {
                                            definition:
                                                DesignComponentPropertyDefinition::InstanceSwap {
                                                    preferred_values,
                                                    ..
                                                },
                                            ..
                                        }) = this.component_authoring.create_draft.as_mut()
                                        {
                                            if let Some(index) =
                                                preferred_values.iter().position(|value| {
                                                    value.id == reference_for_preferred.id
                                                })
                                            {
                                                preferred_values.remove(index);
                                            } else {
                                                preferred_values
                                                    .push(reference_for_preferred.clone());
                                            }
                                            cx.notify();
                                        }
                                    });
                                }),
                            ),
                    );
                }
            }
            DesignComponentPropertyKind::Slot => {
                let DesignComponentPropertyDefinition::Slot { settings, .. } = &draft.definition
                else {
                    return None;
                };
                let panel_for_fill = panel.clone();
                let panel_for_empty = panel.clone();
                let panel_for_preferred_only = panel.clone();
                fields = fields
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Description"),
                    )
                    .child(
                        Input::new(&self.component_authoring.default_input)
                            .appearance(false)
                            .bordered(true)
                            .xsmall()
                            .w_full(),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_0p5()
                                    .child(div().text_xs().child("Minimum layers"))
                                    .child(
                                        Input::new(&self.component_authoring.slot_minimum_input)
                                            .appearance(false)
                                            .bordered(true)
                                            .xsmall()
                                            .w_full(),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_0p5()
                                    .child(div().text_xs().child("Maximum layers"))
                                    .child(
                                        Input::new(&self.component_authoring.slot_maximum_input)
                                            .appearance(false)
                                            .bordered(true)
                                            .xsmall()
                                            .w_full(),
                                    ),
                            ),
                    )
                    .when(settings_invalid, |fields| {
                        fields.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().red)
                                .child("Use whole numbers and keep minimum ≤ maximum."),
                        )
                    })
                    .child(
                        div()
                            .debug_selector(|| {
                                "component-property-slot-fill-counter-axis".to_owned()
                            })
                            .child(
                                Switch::new("component-property-slot-fill-counter-axis")
                                    .label("Fill inserted items on counter-axis")
                                    .xsmall()
                                    .w_full()
                                    .checked(settings.stretch_child_on_insert)
                                    .on_click(move |checked, _, cx| {
                                        panel_for_fill.update(cx, |this, cx| {
                                            if let Some(ComponentPropertyCreateDraft {
                                                definition:
                                                    DesignComponentPropertyDefinition::Slot {
                                                        settings,
                                                        ..
                                                    },
                                                ..
                                            }) = this.component_authoring.create_draft.as_mut()
                                            {
                                                settings.stretch_child_on_insert = *checked;
                                                cx.notify();
                                            }
                                        });
                                    }),
                            ),
                    )
                    .child(
                        Switch::new("component-property-slot-display-empty")
                            .label("Display empty slots by default")
                            .xsmall()
                            .w_full()
                            .checked(settings.display_empty)
                            .on_click(move |checked, _, cx| {
                                panel_for_empty.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyCreateDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Slot {
                                                settings,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.create_draft.as_mut()
                                    {
                                        settings.display_empty = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        Switch::new("component-property-slot-preferred-only")
                            .label("Only allow preferred instances")
                            .xsmall()
                            .w_full()
                            .checked(settings.preferred_values_only)
                            .on_click(move |checked, _, cx| {
                                panel_for_preferred_only.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyCreateDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Slot {
                                                settings,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.create_draft.as_mut()
                                    {
                                        settings.preferred_values_only = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .font_semibold()
                            .child("Preferred instances"),
                    );
                for candidate in self
                    .resources
                    .component_swaps
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.can_apply())
                {
                    let reference = candidate.reference.clone();
                    let selected = settings.preferred_values.contains(&reference);
                    let reference_for_click = reference.clone();
                    let panel_for_click = panel.clone();
                    fields = fields.child(
                        Button::new(SharedString::from(format!(
                            "component-property-slot-preferred-{}",
                            reference.id
                        )))
                        .label(reference.name.clone())
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(selected)
                        .on_activate(move |_, _, cx| {
                            panel_for_click.update(cx, |this, cx| {
                                if let Some(ComponentPropertyCreateDraft {
                                    definition:
                                        DesignComponentPropertyDefinition::Slot { settings, .. },
                                    ..
                                }) = this.component_authoring.create_draft.as_mut()
                                {
                                    if let Some(index) = settings
                                        .preferred_values
                                        .iter()
                                        .position(|value| value.id == reference_for_click.id)
                                    {
                                        settings.preferred_values.remove(index);
                                    } else {
                                        settings.preferred_values.push(reference_for_click.clone());
                                    }
                                    cx.notify();
                                }
                            });
                        }),
                    );
                }
            }
        }

        if matches!(
            draft.kind,
            DesignComponentPropertyKind::Boolean | DesignComponentPropertyKind::Text
        ) {
            let resolved_type = if draft.kind == DesignComponentPropertyKind::Boolean {
                super::super::DesignVariableResolvedType::Boolean
            } else {
                super::super::DesignVariableResolvedType::String
            };
            let panel_for_none = panel.clone();
            fields = fields
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .font_semibold()
                        .child("Default variable"),
                )
                .child(
                    Button::new("component-property-create-variable-none")
                        .label("No variable")
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(draft.default_variable_id.is_none())
                        .on_activate(move |_, _, cx| {
                            panel_for_none.update(cx, |this, cx| {
                                if let Some(draft) = this.component_authoring.create_draft.as_mut()
                                {
                                    draft.default_variable_id = None;
                                    cx.notify();
                                }
                            });
                        }),
                );
            for variable in self
                .resources
                .property_variables
                .variables
                .iter()
                .filter(|variable| {
                    variable.resolved_type == resolved_type
                        && variable.import_state != DesignVariableImportState::Available
                        && variable.disabled_reason.is_none()
                        && variable.resolved_value.is_some()
                })
            {
                let variable_id = variable.id.clone();
                let selected = draft.default_variable_id.as_ref() == Some(&variable.id);
                let panel_for_variable = panel.clone();
                fields = fields.child(
                    Button::new(SharedString::from(format!(
                        "component-property-create-variable-{}",
                        variable.id
                    )))
                    .label(format!("{} / {}", variable.collection_name, variable.name))
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .selected(selected)
                    .on_activate(move |_, _, cx| {
                        panel_for_variable.update(cx, |this, cx| {
                            if let Some(draft) = this.component_authoring.create_draft.as_mut() {
                                draft.default_variable_id = Some(variable_id.clone());
                                cx.notify();
                            }
                        });
                    }),
                );
            }
        }

        let header = h_flex()
            .w_full()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child("Create component property"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(draft.kind.label()),
            )
            .into_any_element();
        let footer = h_flex()
            .w_full()
            .justify_end()
            .gap_1()
            .child(
                Button::new("component-property-create-cancel")
                    .label("Cancel")
                    .xsmall()
                    .compact()
                    .ghost()
                    .on_activate(move |_, window, cx| {
                        panel_for_cancel.update(cx, |this, cx| {
                            this.cancel_component_property_create(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("component-property-create-submit")
                    .label("Create")
                    .xsmall()
                    .compact()
                    .disabled(name_invalid || settings_invalid)
                    .on_activate(move |_, window, cx| {
                        panel_for_submit.update(cx, |this, cx| {
                            this.submit_component_property_create(window, cx);
                        });
                    }),
            )
            .into_any_element();
        Some(dialogs::render(
            dialogs::ComponentAuthoringDialogView {
                header,
                content: fields.into_any_element(),
                footer,
                spacing: dialogs::ComponentAuthoringDialogSpacing::Relaxed,
            },
            cx,
        ))
    }

    fn render_component_property_edit_modal(
        &self,
        panel: Entity<Self>,
        cx: &App,
    ) -> Option<AnyElement> {
        let draft = self.component_authoring.edit_draft.clone()?;
        let property_id = draft.property_id.clone();
        let property = self
            .host
            .inspected_node()
            .component_properties
            .iter()
            .find(|property| property.id == property_id)?
            .clone();
        let definition = self
            .component_authoring_view_data()?
            .definition(property_id.as_ref())?
            .clone();
        let panel_for_close = panel.clone();
        let header = h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child("Edit component property"),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "{} · {}",
                                property.definition.kind().label(),
                                property.name
                            )),
                    ),
            )
            .child(
                Button::new("component-property-edit-close")
                    .label("Cancel")
                    .xsmall()
                    .compact()
                    .ghost()
                    .on_activate(move |_, window, cx| {
                        panel_for_close.update(cx, |this, cx| {
                            this.finish_component_property_edit(false, window, cx);
                        });
                    }),
            )
            .into_any_element();
        let mut modal = v_flex().child(header);

        if definition.capabilities.edit_metadata
            && property.definition.kind() != DesignComponentPropertyKind::Slot
        {
            modal = modal.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Description editing is available for Slot properties."),
            );
        }

        if definition.capabilities.rename {
            let panel_for_rename = panel.clone();
            let property_id_for_rename = property_id.clone();
            modal = modal.child(
                Button::new("component-property-edit-rename")
                    .label("Rename property")
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .on_activate(move |_, window, cx| {
                        panel_for_rename.update(cx, |this, cx| {
                            this.begin_component_property_rename(
                                property_id_for_rename.clone(),
                                window,
                                cx,
                            );
                        });
                    }),
            );
        }

        let property_index = self.component_property_index(property_id.as_ref())?;
        match &draft.definition {
            DesignComponentPropertyDefinition::Boolean { default_value }
                if definition.capabilities.edit_default_value =>
            {
                let panel_for_toggle = panel.clone();
                modal = modal
                    .child(div().pt_1().text_xs().font_semibold().child("Value"))
                    .child(
                        Switch::new("component-property-edit-boolean-default")
                            .label("Default value")
                            .xsmall()
                            .checked(*default_value)
                            .on_click(move |checked, _, cx| {
                                panel_for_toggle.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyEditDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Boolean {
                                                default_value,
                                            },
                                        ..
                                    }) = this.component_authoring.edit_draft.as_mut()
                                    {
                                        *default_value = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    );
            }
            DesignComponentPropertyDefinition::Text { .. }
                if definition.capabilities.edit_default_value =>
            {
                modal = modal
                    .child(div().pt_1().text_xs().font_semibold().child("Value"))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div().flex_1().min_w(px(0.)).child(
                                    Input::new(&self.component_authoring.default_input)
                                        .appearance(false)
                                        .bordered(true)
                                        .xsmall()
                                        .w_full(),
                                ),
                            )
                            .when_some(
                                self.render_component_property_variable_button(
                                    property_index,
                                    panel.clone(),
                                    cx,
                                ),
                                |row, button| row.child(button),
                            ),
                    );
            }
            DesignComponentPropertyDefinition::InstanceSwap {
                default_value,
                preferred_values,
            } => {
                modal = modal.child(div().pt_1().text_xs().font_semibold().child("Value"));
                if definition.capabilities.edit_default_value {
                    let panel_for_none = panel.clone();
                    modal = modal.child(
                        Button::new("component-property-edit-instance-default-none")
                            .label("No default instance")
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(default_value.is_none())
                            .on_activate(move |_, _, cx| {
                                panel_for_none.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyEditDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::InstanceSwap {
                                                default_value,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.edit_draft.as_mut()
                                    {
                                        *default_value = None;
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                for candidate in self
                    .resources
                    .component_swaps
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.can_apply())
                {
                    let reference = candidate.reference.clone();
                    let is_default = default_value.as_ref() == Some(&reference);
                    let is_preferred = preferred_values.contains(&reference);
                    let panel_for_default = panel.clone();
                    let panel_for_preferred = panel.clone();
                    let reference_for_default = reference.clone();
                    let reference_for_preferred = reference.clone();
                    modal = modal.child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(20.))
                                    .rounded(px(4.))
                                    .bg(cx.theme().secondary)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(cx.theme().selection)
                                    .child(render_lucide_icon(
                                        LucideIcon::Diamond,
                                        cx.theme().selection,
                                        16.,
                                    )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .child(reference.name.clone()),
                            )
                            .when(definition.capabilities.edit_default_value, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!(
                                        "component-property-edit-instance-default-{}",
                                        reference.id
                                    )))
                                    .label("Default")
                                    .xsmall()
                                    .compact()
                                    .ghost()
                                    .selected(is_default)
                                    .on_activate(move |_, _, cx| {
                                        panel_for_default.update(cx, |this, cx| {
                                            if let Some(ComponentPropertyEditDraft {
                                                definition:
                                                    DesignComponentPropertyDefinition::InstanceSwap {
                                                        default_value,
                                                        ..
                                                    },
                                                ..
                                            }) = this.component_authoring.edit_draft.as_mut()
                                            {
                                                *default_value =
                                                    Some(reference_for_default.clone());
                                                cx.notify();
                                            }
                                        });
                                    }),
                                )
                            })
                            .when(definition.capabilities.edit_preferred_values, |row| {
                                row.child(
                                    Button::new(SharedString::from(format!(
                                        "component-property-edit-instance-preferred-{}",
                                        reference.id
                                    )))
                                    .icon(if is_preferred {
                                        IconName::Check
                                    } else {
                                        IconName::Plus
                                    })
                                    .tooltip("Preferred instance")
                                    .xsmall()
                                    .compact()
                                    .ghost()
                                    .selected(is_preferred)
                                    .on_activate(move |_, _, cx| {
                                        panel_for_preferred.update(cx, |this, cx| {
                                            if let Some(ComponentPropertyEditDraft {
                                                definition:
                                                    DesignComponentPropertyDefinition::InstanceSwap {
                                                        preferred_values,
                                                        ..
                                                    },
                                                ..
                                            }) = this.component_authoring.edit_draft.as_mut()
                                            {
                                                if let Some(index) = preferred_values
                                                    .iter()
                                                    .position(|value| {
                                                        value.id
                                                            == reference_for_preferred.id
                                                    })
                                                {
                                                    preferred_values.remove(index);
                                                } else {
                                                    preferred_values
                                                        .push(reference_for_preferred.clone());
                                                }
                                                cx.notify();
                                            }
                                        });
                                    }),
                                )
                            }),
                    );
                }
                if let Some(variable_button) = self.render_component_property_variable_button(
                    property_index,
                    panel.clone(),
                    cx,
                ) {
                    modal = modal.child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(div().text_xs().child("Default variable"))
                            .child(variable_button),
                    );
                }
            }
            DesignComponentPropertyDefinition::Slot { settings, .. }
                if definition.capabilities.edit_slot_settings =>
            {
                let settings_invalid = self.component_authoring_slot_limits(cx).is_none();
                let panel_for_fill = panel.clone();
                let panel_for_empty = panel.clone();
                let panel_for_preferred_only = panel.clone();
                modal = modal
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .font_semibold()
                            .child("Slot settings"),
                    )
                    .child(div().text_xs().child("Description"))
                    .child(
                        Input::new(&self.component_authoring.default_input)
                            .appearance(false)
                            .bordered(true)
                            .xsmall()
                            .w_full(),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_0p5()
                                    .child(div().text_xs().child("Minimum layers"))
                                    .child(
                                        Input::new(&self.component_authoring.slot_minimum_input)
                                            .appearance(false)
                                            .bordered(true)
                                            .xsmall()
                                            .w_full(),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_0p5()
                                    .child(div().text_xs().child("Maximum layers"))
                                    .child(
                                        Input::new(&self.component_authoring.slot_maximum_input)
                                            .appearance(false)
                                            .bordered(true)
                                            .xsmall()
                                            .w_full(),
                                    ),
                            ),
                    )
                    .when(settings_invalid, |modal| {
                        modal.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().red)
                                .child("Use whole numbers and keep minimum ≤ maximum."),
                        )
                    })
                    .child(
                        Switch::new("component-property-edit-slot-fill-counter-axis")
                            .label("Fill inserted items on counter-axis")
                            .xsmall()
                            .w_full()
                            .checked(settings.stretch_child_on_insert)
                            .on_click(move |checked, _, cx| {
                                panel_for_fill.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyEditDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Slot {
                                                settings,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.edit_draft.as_mut()
                                    {
                                        settings.stretch_child_on_insert = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        Switch::new("component-property-edit-slot-display-empty")
                            .label("Display empty slots by default")
                            .xsmall()
                            .w_full()
                            .checked(settings.display_empty)
                            .on_click(move |checked, _, cx| {
                                panel_for_empty.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyEditDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Slot {
                                                settings,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.edit_draft.as_mut()
                                    {
                                        settings.display_empty = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        Switch::new("component-property-edit-slot-preferred-only")
                            .label("Only allow preferred instances")
                            .xsmall()
                            .w_full()
                            .checked(settings.preferred_values_only)
                            .on_click(move |checked, _, cx| {
                                panel_for_preferred_only.update(cx, |this, cx| {
                                    if let Some(ComponentPropertyEditDraft {
                                        definition:
                                            DesignComponentPropertyDefinition::Slot {
                                                settings,
                                                ..
                                            },
                                        ..
                                    }) = this.component_authoring.edit_draft.as_mut()
                                    {
                                        settings.preferred_values_only = *checked;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .font_semibold()
                            .child("Preferred instances"),
                    );
                for candidate in self
                    .resources
                    .component_swaps
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.can_apply())
                {
                    let reference = candidate.reference.clone();
                    let selected = settings.preferred_values.contains(&reference);
                    let panel_for_click = panel.clone();
                    let reference_for_click = reference.clone();
                    modal = modal.child(
                        Button::new(SharedString::from(format!(
                            "component-property-edit-slot-preferred-{}",
                            reference.id
                        )))
                        .label(reference.name.clone())
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(selected)
                        .on_activate(move |_, _, cx| {
                            panel_for_click.update(cx, |this, cx| {
                                if let Some(ComponentPropertyEditDraft {
                                    definition:
                                        DesignComponentPropertyDefinition::Slot { settings, .. },
                                    ..
                                }) = this.component_authoring.edit_draft.as_mut()
                                {
                                    if let Some(index) = settings
                                        .preferred_values
                                        .iter()
                                        .position(|value| value.id == reference_for_click.id)
                                    {
                                        settings.preferred_values.remove(index);
                                    } else {
                                        settings.preferred_values.push(reference_for_click.clone());
                                    }
                                    cx.notify();
                                }
                            });
                        }),
                    );
                }
            }
            DesignComponentPropertyDefinition::Variant { .. }
            | DesignComponentPropertyDefinition::Slot { .. }
            | DesignComponentPropertyDefinition::Boolean { .. }
            | DesignComponentPropertyDefinition::Text { .. } => {}
        }

        if property.definition.kind() == DesignComponentPropertyKind::Variant {
            modal = modal.child(
                div()
                    .pt_1()
                    .text_xs()
                    .font_semibold()
                    .child("Variant values"),
            );
            let option_order = definition
                .variant_options
                .iter()
                .map(|option| option.id.clone())
                .collect::<Vec<_>>();
            for option in &definition.variant_options {
                let option_id = option.id.clone();
                let group_name = SharedString::from(format!(
                    "component-variant-option-{}-{}",
                    property_id, option.id
                ));
                let editing = matches!(
                    self.edit.component_authoring.name_editor(),
                    Some(ComponentAuthoringNameEditor::VariantOption {
                        property_id: active_property_id,
                        option_id: active_option_id,
                        ..
                    }) if active_property_id == &property_id && active_option_id == &option.id
                );
                let drag = ComponentVariantOptionDrag {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: property_id.clone(),
                    option_id: option.id.clone(),
                    option_name: option.name.clone(),
                    original_order: option_order.clone(),
                };
                let panel_for_down = panel.clone();
                let panel_for_up = panel.clone();
                let panel_for_up_out = panel.clone();
                let drag_for_down = drag.clone();
                let mut handle = div()
                    .id(SharedString::from(format!(
                        "component-variant-option-drag-{}-{}",
                        property_id, option.id
                    )))
                    .w(px(18.))
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground);
                if option.can_reorder {
                    handle = handle
                        .invisible()
                        .group_hover(group_name.clone(), |handle| handle.visible())
                        .cursor_move()
                        .child("⠿")
                        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| {
                            panel_for_down.update(cx, |this, cx| {
                                this.begin_component_variant_option_reorder(&drag_for_down, cx);
                            });
                        })
                        .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                            let panel = panel_for_up.clone();
                            window.defer(cx, move |_, cx| {
                                panel.update(cx, |this, cx| {
                                    this.finish_component_variant_option_reorder(false, cx);
                                });
                            });
                        })
                        .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                            let panel = panel_for_up_out.clone();
                            window.defer(cx, move |_, cx| {
                                panel.update(cx, |this, cx| {
                                    this.finish_component_variant_option_reorder(false, cx);
                                });
                            });
                        })
                        .on_drag(drag.clone(), |drag, _, _, cx| {
                            cx.new(|_| ComponentVariantOptionDragPreview { drag: drag.clone() })
                        });
                }

                let panel_for_click = panel.clone();
                let property_id_for_click = property_id.clone();
                let option_id_for_click = option.id.clone();
                let mut row = h_flex()
                    .id(SharedString::from(format!(
                        "{}-component-variant-option-{}-{}",
                        self.id, property_id, option.id
                    )))
                    .group(group_name)
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .gap_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.6)))
                    .child(handle);
                if editing {
                    row = row.child(
                        Input::new(&self.component_authoring.name_input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(true)
                            .xsmall()
                            .h(px(ROW_HEIGHT - 2.))
                            .flex_1()
                            .min_w(px(0.)),
                    );
                } else {
                    row = row.child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .child(option.name.clone()),
                    );
                }
                if option.can_delete {
                    let panel_for_delete = panel.clone();
                    let property_id_for_delete = property_id.clone();
                    let option_id_for_delete = option.id.clone();
                    let expected_name = option.name.clone();
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "delete-variant-option-{}-{}",
                            property_id, option.id
                        )))
                        .label("−")
                        .tooltip("Delete Variant value")
                        .xsmall()
                        .compact()
                        .ghost()
                        .on_activate(move |_, _, cx| {
                            panel_for_delete.update(cx, |this, cx| {
                                this.emit_component_authoring_action(
                                    DesignPanelAction::ComponentVariantOptionDeleteRequested {
                                        node_id: this.host.inspected_node().id.clone(),
                                        property_id: property_id_for_delete.clone(),
                                        option_id: option_id_for_delete.clone(),
                                        expected_name: expected_name.clone(),
                                    },
                                    cx,
                                );
                            });
                        }),
                    );
                }
                let target_option_id = option_id.clone();
                let can_drop_property_id = property_id.clone();
                let panel_for_move = panel.clone();
                let panel_for_drop = panel.clone();
                let option_id_for_move = option_id.clone();
                let option_id_for_style = option_id.clone();
                let option_id_for_drop = option_id;
                let property_id_for_style = property_id.clone();
                row = row
                    .on_click(move |event: &ClickEvent, window, cx| {
                        if event.click_count() >= 2 {
                            panel_for_click.update(cx, |this, cx| {
                                this.begin_component_variant_option_rename(
                                    property_id_for_click.clone(),
                                    option_id_for_click.clone(),
                                    window,
                                    cx,
                                );
                            });
                        }
                    })
                    .can_drop(move |candidate, _, _| {
                        candidate
                            .downcast_ref::<ComponentVariantOptionDrag>()
                            .is_some_and(|drag| {
                                drag.property_id == can_drop_property_id
                                    && drag.option_id != target_option_id
                            })
                    })
                    .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            panel_for_move.update(cx, |this, cx| {
                                this.preview_component_variant_option_reorder(
                                    Some(option_id_for_move.clone()),
                                    cx,
                                );
                            });
                        }
                    })
                    .drag_over::<ComponentVariantOptionDrag>(move |style, drag, _, cx| {
                        if drag.property_id == property_id_for_style
                            && drag.option_id != option_id_for_style
                        {
                            style
                                .bg(cx.theme().selection.opacity(0.18))
                                .border_color(cx.theme().selection)
                        } else {
                            style
                        }
                    })
                    .on_drop(move |drag: &ComponentVariantOptionDrag, _, cx| {
                        panel_for_drop.update(cx, |this, cx| {
                            if !this.edit.component_authoring.has_variant_option_reorder() {
                                this.begin_component_variant_option_reorder(drag, cx);
                            }
                            this.preview_component_variant_option_reorder(
                                Some(option_id_for_drop.clone()),
                                cx,
                            );
                            this.finish_component_variant_option_reorder(true, cx);
                        });
                    });
                modal = modal.child(row);
            }

            let creating_option = matches!(
                self.edit.component_authoring.name_editor(),
                Some(ComponentAuthoringNameEditor::NewVariantOption {
                    property_id: active_property_id,
                    ..
                }) if active_property_id == &property_id
            );
            if creating_option {
                modal = modal.child(
                    h_flex().w_full().h(px(ROW_HEIGHT)).pl(px(18.)).child(
                        Input::new(&self.component_authoring.name_input)
                            .appearance(false)
                            .bordered(true)
                            .xsmall()
                            .w_full(),
                    ),
                );
            } else if definition.capabilities.edit_variant_options {
                let panel = panel.clone();
                let property_id = property_id.clone();
                modal = modal.child(
                    Button::new("component-variant-option-create")
                        .label("+ Add value")
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .on_activate(move |_, window, cx| {
                            panel.update(cx, |this, cx| {
                                this.begin_component_variant_option_create(
                                    property_id.clone(),
                                    window,
                                    cx,
                                );
                            });
                        }),
                );
            }
        }
        let panel_for_save = panel;
        let settings_invalid = matches!(
            draft.definition,
            DesignComponentPropertyDefinition::Slot { .. }
        ) && self.component_authoring_slot_limits(cx).is_none();
        let footer = h_flex()
            .w_full()
            .justify_end()
            .child(
                Button::new("component-property-edit-save")
                    .label("Save")
                    .xsmall()
                    .compact()
                    .disabled(settings_invalid)
                    .on_activate(move |_, window, cx| {
                        panel_for_save.update(cx, |this, cx| {
                            this.finish_component_property_edit(true, window, cx);
                        });
                    }),
            )
            .into_any_element();
        modal = modal.child(footer);
        Some(dialogs::render_container(
            modal,
            dialogs::ComponentAuthoringDialogSpacing::Compact,
            cx,
        ))
    }

    fn render_component_definition_authoring(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let authoring = self.component_authoring_view_data()?.clone();
        let mut content = v_flex()
            .w_full()
            .gap_2()
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .justify_between()
                    .child(div().text_xs().font_semibold().child("Properties"))
                    .child(self.render_component_property_create_popover(cx)),
            );
        if !authoring.preserves_variant_partition(&self.host.inspected_node().component_properties)
        {
            return Some(
                content
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().red)
                            .child("Host definition order crosses the Variant partition"),
                    )
                    .into_any_element(),
            );
        }
        if self.component_authoring_uses_inline_modal_fallback()
            && let Some(create_modal) = self.render_component_property_create_modal(cx.entity(), cx)
        {
            content = content.child(create_modal);
        }

        let panel = cx.entity();
        for partition in [
            DesignComponentPropertyPartition::Variant,
            DesignComponentPropertyPartition::Regular,
        ] {
            let partition_properties = self
                .host
                .inspected_node()
                .component_properties
                .iter()
                .filter(|property| {
                    DesignComponentPropertyPartition::for_kind(property.definition.kind())
                        == partition
                        && authoring.definition(property.id.as_ref()).is_some()
                })
                .cloned()
                .collect::<Vec<_>>();
            if partition_properties.is_empty() {
                continue;
            }
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(partition.label()),
            );
            let property_order = partition_properties
                .iter()
                .map(|property| property.id.clone())
                .collect::<Vec<_>>();
            for property in &partition_properties {
                let definition = authoring
                    .definition(property.id.as_ref())
                    .expect("filtered authoring definition");
                let kind = property.definition.kind();
                let selected =
                    self.component_authoring.selected_property.as_ref() == Some(&property.id);
                let editing = matches!(
                    self.edit.component_authoring.name_editor(),
                    Some(ComponentAuthoringNameEditor::Property {
                        property_id: active_property_id,
                        ..
                    }) if active_property_id == &property.id
                );
                let group_name =
                    SharedString::from(format!("component-property-row-{}", property.id));
                let drag = ComponentPropertyDefinitionDrag {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: property.id.clone(),
                    property_name: property.name.clone(),
                    partition,
                    original_order: property_order.clone(),
                };
                let drag_for_down = drag.clone();
                let panel_for_down = panel.clone();
                let panel_for_up = panel.clone();
                let panel_for_up_out = panel.clone();
                let mut handle = div()
                    .id(SharedString::from(format!(
                        "{}-component-property-drag-{}",
                        self.id, property.id
                    )))
                    .w(px(18.))
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground);
                if definition.capabilities.reorder {
                    handle = handle
                        .invisible()
                        .group_hover(group_name.clone(), |handle| handle.visible())
                        .cursor_move()
                        .child("⠿")
                        .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            panel_for_down.update(cx, |this, cx| {
                                this.begin_component_property_reorder(&drag_for_down, cx);
                            });
                        })
                        .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                            let panel = panel_for_up.clone();
                            window.defer(cx, move |_, cx| {
                                panel.update(cx, |this, cx| {
                                    this.finish_component_property_reorder(false, cx);
                                });
                            });
                        })
                        .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                            let panel = panel_for_up_out.clone();
                            window.defer(cx, move |_, cx| {
                                panel.update(cx, |this, cx| {
                                    this.finish_component_property_reorder(false, cx);
                                });
                            });
                        })
                        .on_drag(drag.clone(), |drag, _, _, cx| {
                            cx.new(|_| ComponentPropertyDefinitionDragPreview {
                                drag: drag.clone(),
                            })
                        });
                }

                let panel_for_click = panel.clone();
                let property_id_for_click = property.id.clone();
                let panel_for_keyboard = panel.clone();
                let property_id_for_keyboard = property.id.clone();
                let panel_for_context = panel.clone();
                let property_id_for_context = property.id.clone();
                let mut row = h_flex()
                    .id(SharedString::from(format!(
                        "{}-component-property-row-{}",
                        self.id, property.id
                    )))
                    .group(group_name)
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .w_full()
                    .min_h(px(ROW_HEIGHT))
                    .gap_1()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(cx.theme().transparent)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.6)))
                    .focus(|style| style.border_color(cx.theme().selection))
                    .when(selected, |row| row.bg(cx.theme().sidebar_accent))
                    .child(handle)
                    .child(
                        div()
                            .w(px(16.))
                            .flex_none()
                            .text_xs()
                            .text_color(cx.theme().selection)
                            .child(match kind {
                                DesignComponentPropertyKind::Variant => "V",
                                DesignComponentPropertyKind::Text => "T",
                                DesignComponentPropertyKind::Boolean => "B",
                                DesignComponentPropertyKind::InstanceSwap => "I",
                                DesignComponentPropertyKind::Slot => "S",
                            }),
                    );
                if editing {
                    row = row.child(
                        Input::new(&self.component_authoring.name_input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(true)
                            .xsmall()
                            .h(px(ROW_HEIGHT - 2.))
                            .flex_1()
                            .min_w(px(0.)),
                    );
                } else {
                    row = row.child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(div().truncate().text_xs().child(property.name.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(kind.label()),
                            ),
                    );
                }
                if definition.capabilities.edit_metadata
                    || definition.capabilities.edit_default_value
                    || definition.capabilities.edit_preferred_values
                    || definition.capabilities.edit_slot_settings
                    || definition.capabilities.edit_variant_options
                {
                    let panel_for_edit = panel.clone();
                    let property_id_for_edit = property.id.clone();
                    row = row.child(
                        Button::new(SharedString::from(format!(
                            "component-property-edit-{}",
                            property.id
                        )))
                        .label("…")
                        .tooltip("Edit component property")
                        .xsmall()
                        .compact()
                        .ghost()
                        .on_activate(move |_, window, cx| {
                            panel_for_edit.update(cx, |this, cx| {
                                this.open_component_property_edit(
                                    property_id_for_edit.clone(),
                                    window,
                                    cx,
                                );
                            });
                        }),
                    );
                }

                let target_property_id = property.id.clone();
                let target_property_id_for_move = property.id.clone();
                let target_property_id_for_style = property.id.clone();
                let target_property_id_for_drop = property.id.clone();
                let can_drop_node_id = self.host.inspected_node().id.clone();
                let panel_for_move = panel.clone();
                let panel_for_drop = panel.clone();
                // Contract (§16): this row keeps an explicit action/click pair
                // instead of `on_activate` because the pointer path needs
                // `ClickEvent`'s `click_count` to distinguish double-click
                // rename from single-click selection.
                row = row
                    .on_action(move |_: &ActivateControl, _, cx| {
                        panel_for_keyboard.update(cx, |this, cx| {
                            this.component_authoring.selected_property =
                                Some(property_id_for_keyboard.clone());
                            this.overlays
                                .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
                            cx.notify();
                        });
                    })
                    .on_click(move |event: &ClickEvent, window, cx| {
                        if event.is_keyboard() {
                            return;
                        }
                        panel_for_click.update(cx, |this, cx| {
                            this.component_authoring.selected_property =
                                Some(property_id_for_click.clone());
                            this.overlays
                                .discard(DesignOpenOverlay::ComponentPropertyContextMenu);
                            if event.click_count() >= 2 {
                                this.begin_component_property_rename(
                                    property_id_for_click.clone(),
                                    window,
                                    cx,
                                );
                            } else {
                                cx.notify();
                            }
                        });
                    })
                    .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        panel_for_context.update(cx, |this, cx| {
                            this.component_authoring.selected_property =
                                Some(property_id_for_context.clone());
                            this.overlays
                                .open(DesignOverlayState::ComponentPropertyContextMenu(
                                    property_id_for_context.clone(),
                                ));
                            cx.notify();
                        });
                    })
                    .can_drop(move |candidate, _, _| {
                        candidate
                            .downcast_ref::<ComponentPropertyDefinitionDrag>()
                            .is_some_and(|drag| {
                                drag.node_id == can_drop_node_id
                                    && drag.partition == partition
                                    && drag.property_id != target_property_id
                            })
                    })
                    .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            panel_for_move.update(cx, |this, cx| {
                                this.preview_component_property_reorder(
                                    Some(target_property_id_for_move.clone()),
                                    cx,
                                );
                            });
                        }
                    })
                    .drag_over::<ComponentPropertyDefinitionDrag>(move |style, drag, _, cx| {
                        if drag.partition == partition
                            && drag.property_id != target_property_id_for_style
                        {
                            style
                                .bg(cx.theme().selection.opacity(0.18))
                                .border_color(cx.theme().selection)
                        } else {
                            style
                        }
                    })
                    .on_drop(move |drag: &ComponentPropertyDefinitionDrag, _, cx| {
                        panel_for_drop.update(cx, |this, cx| {
                            if !this.edit.component_authoring.has_property_reorder() {
                                this.begin_component_property_reorder(drag, cx);
                            }
                            this.preview_component_property_reorder(
                                Some(target_property_id_for_drop.clone()),
                                cx,
                            );
                            this.finish_component_property_reorder(true, cx);
                        });
                    });
                content = content.child(row);

                if self.overlays.component_property_context_menu().as_ref() == Some(&property.id) {
                    let panel_for_rename = panel.clone();
                    let property_id_for_rename = property.id.clone();
                    let panel_for_delete = panel.clone();
                    let property_id_for_delete = property.id.clone();
                    let mut menu = v_flex()
                        .ml(px(34.))
                        .w(px(176.))
                        .p_1()
                        .gap_0p5()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().popover)
                        .shadow_lg();
                    if definition.capabilities.rename {
                        menu = menu.child(
                            Button::new(SharedString::from(format!(
                                "component-property-context-rename-{}",
                                property.id
                            )))
                            .label("Rename property")
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, window, cx| {
                                panel_for_rename.update(cx, |this, cx| {
                                    this.begin_component_property_rename(
                                        property_id_for_rename.clone(),
                                        window,
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    if definition.capabilities.delete {
                        menu = menu.child(
                            Button::new(SharedString::from(format!(
                                "component-property-context-delete-{}",
                                property.id
                            )))
                            .label("Delete property")
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, _, cx| {
                                panel_for_delete.update(cx, |this, cx| {
                                    this.request_component_property_delete(
                                        property_id_for_delete.clone(),
                                        cx,
                                    );
                                });
                            }),
                        );
                    }
                    content = content.child(menu);
                }
            }

            let panel_for_end_move = panel.clone();
            let panel_for_end_drop = panel.clone();
            content = content.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-component-property-{:?}-end-drop",
                        self.id, partition
                    )))
                    .w_full()
                    .h(px(5.))
                    .rounded(px(2.))
                    .can_drop(move |candidate, _, _| {
                        candidate
                            .downcast_ref::<ComponentPropertyDefinitionDrag>()
                            .is_some_and(|drag| drag.partition == partition)
                    })
                    .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                        if event.dragging() {
                            panel_for_end_move.update(cx, |this, cx| {
                                this.preview_component_property_reorder(None, cx);
                            });
                        }
                    })
                    .drag_over::<ComponentPropertyDefinitionDrag>(move |style, drag, _, cx| {
                        if drag.partition == partition {
                            style.bg(cx.theme().selection)
                        } else {
                            style
                        }
                    })
                    .on_drop(move |drag: &ComponentPropertyDefinitionDrag, _, cx| {
                        panel_for_end_drop.update(cx, |this, cx| {
                            if !this.edit.component_authoring.has_property_reorder() {
                                this.begin_component_property_reorder(drag, cx);
                            }
                            this.preview_component_property_reorder(None, cx);
                            this.finish_component_property_reorder(true, cx);
                        });
                    }),
            );
        }
        if self.component_authoring_uses_inline_modal_fallback()
            && let Some(edit_modal) = self.render_component_property_edit_modal(cx.entity(), cx)
        {
            content = content.child(edit_modal);
        }
        if self.host.inspected_node().component_properties.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Create a property with +"),
            );
        }
        Some(content.into_any_element())
    }

    fn render_nested_component_property_exposures(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let candidates = self
            .component_authoring_view_data()?
            .exposure_candidates
            .clone();
        if candidates.is_empty() {
            return None;
        }
        let panel = cx.entity();
        let mut rows = v_flex()
            .w_full()
            .gap_1()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .child("Expose nested properties"),
            );
        for candidate in candidates {
            let candidate_for_hover = candidate.clone();
            let panel_for_hover = panel.clone();
            let node_id_for_hover = self.host.inspected_node().id.clone();
            let mut actions = h_flex().gap_1();
            if let Some(exposed_property_id) = candidate.exposed_property_id.clone() {
                actions = actions.child(self.render_component_action_button(
                    format!("unexpose-nested-{}", candidate.candidate_id),
                    "Unexpose",
                    DesignPanelAction::NestedComponentPropertyUnexposeRequested {
                        node_id: self.host.inspected_node().id.clone(),
                        candidate_id: candidate.candidate_id.clone(),
                        nested_instance_id: candidate.nested_instance_id.clone(),
                        nested_property_id: candidate.nested_property.property_id.clone(),
                        exposed_property_id,
                    },
                    cx,
                ));
            } else {
                actions = actions.child(self.render_component_action_button(
                    format!("expose-nested-{}", candidate.candidate_id),
                    "Expose",
                    DesignPanelAction::NestedComponentPropertyExposeRequested {
                        node_id: self.host.inspected_node().id.clone(),
                        candidate_id: candidate.candidate_id.clone(),
                        nested_instance_id: candidate.nested_instance_id.clone(),
                        nested_property_id: candidate.nested_property.property_id.clone(),
                    },
                    cx,
                ));
            }
            rows = rows
                .child(
                    h_flex()
                        .id(SharedString::from(format!(
                            "{}-nested-exposure-{}",
                            self.id, candidate.candidate_id
                        )))
                        .w_full()
                        .gap_2()
                        .on_hover(move |hovered, _, cx| {
                            let action =
                                DesignPanelAction::NestedComponentPropertyPreviewRequested {
                                    node_id: node_id_for_hover.clone(),
                                    candidate_id: candidate_for_hover.candidate_id.clone(),
                                    nested_instance_id: candidate_for_hover
                                        .nested_instance_id
                                        .clone(),
                                    nested_property_id: candidate_for_hover
                                        .nested_property
                                        .property_id
                                        .clone(),
                                    preview: *hovered,
                                };
                            panel_for_hover.update(cx, |this, cx| {
                                this.emit_component_authoring_action(action, cx);
                            });
                        })
                        .child(render_lucide_icon(
                            LucideIcon::Diamond,
                            cx.theme().selection,
                            16.,
                        ))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .child(candidate.nested_property.property_name.clone()),
                                )
                                .child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · {}",
                                            candidate.nested_instance_name,
                                            candidate.nested_property.kind.label()
                                        )),
                                ),
                        ),
                )
                .child(actions);
        }
        Some(rows.into_any_element())
    }

    fn render_slot_limits(
        &self,
        property: &DesignComponentProperty,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let guidelines = slot_limit_guidelines(property);
        if guidelines.is_empty() {
            return None;
        }
        let open = self.component_authoring.open_slot_limits.as_ref() == Some(&property.id);
        let unmet = guidelines
            .iter()
            .any(|guideline| guideline.status == SlotLimitGuidelineStatus::Unmet);
        let property_id_for_activate = property.id.clone();
        let mut limits = v_flex().w_full().gap_1().child(
            h_flex()
                .id(SharedString::from(format!(
                    "{}-slot-limits-{}",
                    self.id, property.id
                )))
                .debug_selector({
                    let property_id = property.id.clone();
                    move || format!("slot-limits-{property_id}")
                })
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .w_full()
                .h(px(ROW_HEIGHT))
                .px_2()
                .gap_2()
                .rounded(px(4.))
                .cursor_pointer()
                .text_xs()
                .text_color(if unmet {
                    cx.theme().warning
                } else {
                    cx.theme().muted_foreground
                })
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_1()
                        .border_color(cx.theme().selection)
                })
                .on_activate(cx.listener(move |this, _, _, cx| {
                    if this.component_authoring.open_slot_limits.as_ref()
                        == Some(&property_id_for_activate)
                    {
                        this.component_authoring.open_slot_limits = None;
                    } else {
                        this.component_authoring.open_slot_limits =
                            Some(property_id_for_activate.clone());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .w(px(14.))
                        .text_color(cx.theme().warning)
                        .when(unmet, |warning| {
                            warning.child(render_lucide_icon(
                                LucideIcon::TriangleAlert,
                                cx.theme().warning,
                                14.,
                            ))
                        }),
                )
                .child(div().flex_1().font_semibold().child("Limits"))
                .child(
                    Icon::new(if open {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .xsmall(),
                ),
        );
        if open {
            let offending_ids = slot_preferred_violation_layer_ids(property);
            let view_layers_action = (!offending_ids.is_empty()).then(|| {
                DesignPanelAction::SlotLimitLayersSelectRequested {
                    node_id: self.host.inspected_node().id.clone(),
                    property_id: property.id.clone(),
                    child_node_ids: offending_ids,
                }
            });
            let mut details = v_flex()
                .w_full()
                .gap_1()
                .px_2()
                .py_1()
                .rounded(px(4.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary);
            for guideline in guidelines {
                let (icon, color) = match guideline.status {
                    SlotLimitGuidelineStatus::Met => ("Met", cx.theme().green),
                    SlotLimitGuidelineStatus::Unmet => ("Unmet", cx.theme().warning),
                    SlotLimitGuidelineStatus::Unknown => ("Unknown", cx.theme().muted_foreground),
                };
                details = details.child(
                    h_flex()
                        .min_h(px(ROW_HEIGHT))
                        .gap_2()
                        .text_xs()
                        .child(div().w(px(14.)).text_color(color).child(icon))
                        .child(div().flex_1().min_w(px(0.)).child(guideline.label))
                        .when_some(
                            (guideline.kind == SlotLimitGuidelineKind::PreferredInstancesOnly
                                && guideline.status == SlotLimitGuidelineStatus::Unmet)
                                .then(|| view_layers_action.clone())
                                .flatten(),
                            |row, action| {
                                row.child(self.render_component_action_button(
                                    format!("slot-view-layers-{}", property.id),
                                    "View layers",
                                    action,
                                    cx,
                                ))
                            },
                        ),
                );
            }
            limits = limits.child(details);
        }
        Some(limits.into_any_element())
    }

    fn render_component_property_row(
        &self,
        role: DesignComponentRole,
        index: usize,
        property: DesignComponentProperty,
        multiline_editor_active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let kind = property.definition.kind();
        let value = property.effective_value();
        let next = property
            .definition
            .option_labels()
            .into_iter()
            .find(|option| option != &value.display_value())
            .unwrap_or_else(|| value.display_value());
        let multiline = matches!(
            property.definition,
            DesignComponentPropertyDefinition::Text {
                multiline: true,
                ..
            }
        );
        let value_control = match kind {
            DesignComponentPropertyKind::InstanceSwap => {
                self.render_component_swap_browser(index, &property, cx)
            }
            DesignComponentPropertyKind::Text if multiline => {
                let display_value: SharedString = match &value {
                    DesignComponentPropertyValue::Text(value) if value.is_empty() => "Empty".into(),
                    DesignComponentPropertyValue::Text(value) => value.replace('\n', " ↵ ").into(),
                    _ => value.display_value(),
                };
                Button::new(SharedString::from(format!(
                    "{}-component-multiline-{}",
                    self.id, property.id
                )))
                .label(display_value)
                .tooltip("Edit multiline text")
                .xsmall()
                .compact()
                .w_full()
                .h(px(ROW_HEIGHT))
                .disabled(!self.property_is_editable(DesignPanelProperty::ComponentProperty(index)))
                .on_activate(cx.listener(move |this, _, window, cx| {
                    this.open_component_multiline_editor_from_control(index, window, cx);
                }))
                .into_any_element()
            }
            _ => self.render_value_cell(
                format!("component-property-{index}"),
                match kind {
                    DesignComponentPropertyKind::Variant => "V",
                    DesignComponentPropertyKind::Text => "T",
                    DesignComponentPropertyKind::Boolean => "B",
                    DesignComponentPropertyKind::InstanceSwap => "I",
                    DesignComponentPropertyKind::Slot => "S",
                },
                value.display_value(),
                DesignPanelProperty::ComponentProperty(index),
                DesignPanelValue::Text(next),
                cx,
            ),
        };
        let mut property_content = v_flex().gap_1().child(
            h_flex()
                .min_h(px(ROW_HEIGHT))
                .gap_2()
                .child(
                    div()
                        .w(px(104.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(property.name.clone()),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap_1()
                        .child(div().flex_1().min_w(px(0.)).child(value_control))
                        .when_some(
                            self.render_component_property_variable_button(index, cx.entity(), cx),
                            |row, button| row.child(button),
                        ),
                ),
        );

        if multiline && multiline_editor_active {
            property_content = property_content.child(
                v_flex()
                    .pl(px(112.))
                    .gap_1()
                    .child(
                        div()
                            .h(px(88.))
                            .w_full()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(cx.theme().selection)
                            .child(
                                Input::new(&self.retained.inputs.component_multiline)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false)
                                    .small()
                                    .h_full()
                                    .w_full(),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-component-multiline-cancel-{}",
                                    self.id, property.id
                                )))
                                .label("Cancel")
                                .xsmall()
                                .compact()
                                .ghost()
                                .on_activate(cx.listener(
                                    |this, _, window, cx| {
                                        this.finish_component_multiline_editor(false, window, cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-component-multiline-apply-{}",
                                    self.id, property.id
                                )))
                                .label("Apply")
                                .xsmall()
                                .compact()
                                .on_activate(cx.listener(
                                    |this, _, window, cx| {
                                        this.finish_component_multiline_editor(true, window, cx);
                                    },
                                )),
                            ),
                    ),
            );
        }

        if matches!(
            property.definition,
            DesignComponentPropertyDefinition::Slot { .. }
        ) && let Some(description) = property.description.as_ref()
        {
            property_content = property_content.child(
                div()
                    .pl(px(112.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(description.clone()),
            );
        }
        for link in &property.documentation_links {
            property_content = property_content.child(
                h_flex()
                    .pl(px(112.))
                    .gap_1()
                    .text_xs()
                    .text_color(cx.theme().selection)
                    .child(render_lucide_icon(
                        LucideIcon::ExternalLink,
                        cx.theme().selection,
                        14.,
                    ))
                    .child(link.label.clone()),
            );
        }
        if property.override_state != DesignComponentPropertyOverrideState::Default {
            property_content = property_content.child(
                h_flex()
                    .pl(px(112.))
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match property.override_state {
                                DesignComponentPropertyOverrideState::Default => "",
                                DesignComponentPropertyOverrideState::Overridden => "Overridden",
                                DesignComponentPropertyOverrideState::Mixed => "Mixed override",
                            }),
                    )
                    .when(property.reset_state.can_reset(), |row| {
                        row.child(self.render_component_action_button(
                            format!("reset-component-property-{}", property.id),
                            "Reset",
                            DesignPanelAction::ComponentPropertyResetRequested {
                                node_id: self.host.inspected_node().id.clone(),
                                property_id: property.id.clone(),
                            },
                            cx,
                        ))
                    }),
            );
        }

        if let DesignComponentPropertyDefinition::Slot { settings, .. } = &property.definition {
            if role.can_configure_slot() {
                property_content = property_content
                    .child(self.render_toggle_row(
                        format!("slot-stretch-{index}"),
                        "Stretch child on insert",
                        settings.stretch_child_on_insert,
                        DesignPanelProperty::SlotStretchChildOnInsert(index),
                        cx,
                    ))
                    .child(self.render_toggle_row(
                        format!("slot-display-empty-{index}"),
                        "Display empty slot",
                        settings.display_empty,
                        DesignPanelProperty::SlotDisplayEmpty(index),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .gap_1()
                                    .child(self.render_group_label("Minimum layers", cx))
                                    .child(
                                        self.render_value_cell(
                                            format!("slot-minimum-{index}"),
                                            "Min",
                                            settings.minimum_children.map_or_else(
                                                || "None".into(),
                                                |value| value.to_string(),
                                            ),
                                            DesignPanelProperty::SlotMinimumInstances(index),
                                            DesignPanelValue::OptionalNumber(
                                                settings
                                                    .minimum_children
                                                    .map_or(Some(0.), |value| {
                                                        Some(value as f32 + 1.)
                                                    }),
                                            ),
                                            cx,
                                        ),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .gap_1()
                                    .child(self.render_group_label("Maximum layers", cx))
                                    .child(
                                        self.render_value_cell(
                                            format!("slot-maximum-{index}"),
                                            "Max",
                                            settings.maximum_children.map_or_else(
                                                || "None".into(),
                                                |value| value.to_string(),
                                            ),
                                            DesignPanelProperty::SlotMaximumInstances(index),
                                            DesignPanelValue::OptionalNumber(
                                                settings
                                                    .maximum_children
                                                    .map_or(Some(1.), |value| {
                                                        Some(value as f32 + 1.)
                                                    }),
                                            ),
                                            cx,
                                        ),
                                    ),
                            ),
                    )
                    .child(self.render_toggle_row(
                        format!("slot-preferred-only-{index}"),
                        "Only allow preferred instances",
                        settings.preferred_values_only,
                        DesignPanelProperty::SlotPreferredValuesOnly(index),
                        cx,
                    ));
            }

            if role.can_modify_slot_instances()
                && let Some(limits) = self.render_slot_limits(&property, cx)
            {
                property_content = property_content.child(limits);
            }

            if !settings.preferred_values.is_empty() {
                property_content = property_content.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Preferred instances"),
                );
                for preferred in &settings.preferred_values {
                    let action = DesignPanelAction::SlotAddInstanceRequested {
                        node_id: self.host.inspected_node().id.clone(),
                        property_id: property.id.clone(),
                        preferred_component: Some(preferred.clone()),
                    };
                    property_content = property_content.child(
                        h_flex()
                            .gap_2()
                            .child(render_lucide_icon(
                                LucideIcon::Diamond,
                                cx.theme().selection,
                                16.,
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_xs()
                                    .child(preferred.name.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(preferred.origin.label()),
                            )
                            .when(role.can_modify_slot_instances(), |row| {
                                row.child(self.render_component_action_button(
                                    format!("slot-add-preferred-{}-{}", property.id, preferred.id),
                                    "Add",
                                    action,
                                    cx,
                                ))
                            }),
                    );
                }
            }

            if let DesignComponentPropertyValue::Slot(slot_value) = &value {
                for (child_index, child) in slot_value.children.iter().enumerate() {
                    let previous_index = child_index.saturating_sub(1);
                    let next_index =
                        (child_index + 1).min(slot_value.children.len().saturating_sub(1));
                    let replacement = settings
                        .preferred_values
                        .iter()
                        .find(|candidate| {
                            candidate.availability.is_available()
                                && child
                                    .main_component
                                    .as_ref()
                                    .is_none_or(|current| current.id != candidate.id)
                        })
                        .cloned();
                    property_content = property_content.child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_color(cx.theme().selection).child(
                                        render_lucide_icon(
                                            child.kind.lucide_icon(),
                                            cx.theme().selection,
                                            16.,
                                        ),
                                    ))
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .text_xs()
                                            .child(child.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(child.kind.label()),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .when(child.capabilities.select, |row| {
                                        row.child(self.render_component_action_button(
                                            format!(
                                                "slot-child-select-{}-{}",
                                                property.id, child.node_id
                                            ),
                                            "Select",
                                            DesignPanelAction::SlotChildSelectRequested {
                                                node_id: self.host.inspected_node().id.clone(),
                                                property_id: property.id.clone(),
                                                child_node_id: child.node_id.clone(),
                                            },
                                            cx,
                                        ))
                                    })
                                    .when(child.capabilities.reorder && child_index > 0, |row| {
                                        row.child(self.render_component_action_button(
                                            format!(
                                                "slot-child-up-{}-{}",
                                                property.id, child.node_id
                                            ),
                                            "↑",
                                            DesignPanelAction::SlotChildReorderRequested {
                                                node_id: self.host.inspected_node().id.clone(),
                                                property_id: property.id.clone(),
                                                child_node_id: child.node_id.clone(),
                                                from_index: child_index,
                                                to_index: previous_index,
                                            },
                                            cx,
                                        ))
                                    })
                                    .when(
                                        child.capabilities.reorder
                                            && child_index + 1 < slot_value.children.len(),
                                        |row| {
                                            row.child(self.render_component_action_button(
                                                format!(
                                                    "slot-child-down-{}-{}",
                                                    property.id, child.node_id
                                                ),
                                                "↓",
                                                DesignPanelAction::SlotChildReorderRequested {
                                                    node_id: self.host.inspected_node().id.clone(),
                                                    property_id: property.id.clone(),
                                                    child_node_id: child.node_id.clone(),
                                                    from_index: child_index,
                                                    to_index: next_index,
                                                },
                                                cx,
                                            ))
                                        },
                                    )
                                    .when_some(
                                        child
                                            .capabilities
                                            .replace_instance
                                            .then_some(replacement)
                                            .flatten(),
                                        |row, replacement| {
                                            row.child(self.render_component_action_button(
                                                format!(
                                                    "slot-child-replace-{}-{}",
                                                    property.id, child.node_id
                                                ),
                                                "Swap",
                                                DesignPanelAction::SlotChildReplaceRequested {
                                                    node_id: self.host.inspected_node().id.clone(),
                                                    property_id: property.id.clone(),
                                                    child_node_id: child.node_id.clone(),
                                                    replacement,
                                                },
                                                cx,
                                            ))
                                        },
                                    )
                                    .when(child.capabilities.remove, |row| {
                                        row.child(self.render_component_action_button(
                                            format!(
                                                "slot-child-remove-{}-{}",
                                                property.id, child.node_id
                                            ),
                                            "Remove",
                                            DesignPanelAction::SlotChildRemoveRequested {
                                                node_id: self.host.inspected_node().id.clone(),
                                                property_id: property.id.clone(),
                                                child_node_id: child.node_id.clone(),
                                                index: child_index,
                                            },
                                            cx,
                                        ))
                                    }),
                            ),
                    );
                }
            }

            if role.can_modify_slot_instances() {
                property_content = property_content.child(
                    h_flex()
                        .gap_2()
                        .when(
                            property
                                .slot_state
                                .as_ref()
                                .is_some_and(|state| state.reset_state.can_reset()),
                            |row| {
                                row.child(self.render_component_action_button(
                                    format!("slot-reset-{}", property.id),
                                    "Reset",
                                    DesignPanelAction::SlotResetRequested {
                                        node_id: self.host.inspected_node().id.clone(),
                                        property_id: property.id.clone(),
                                    },
                                    cx,
                                ))
                            },
                        )
                        .child(self.render_component_action_button(
                            format!("slot-clear-{}", property.id),
                            "Delete contents",
                            DesignPanelAction::SlotClearRequested {
                                node_id: self.host.inspected_node().id.clone(),
                                property_id: property.id.clone(),
                            },
                            cx,
                        ))
                        .child(self.render_component_action_button(
                            format!("slot-add-{}", property.id),
                            "Add instances",
                            DesignPanelAction::SlotAddInstanceRequested {
                                node_id: self.host.inspected_node().id.clone(),
                                property_id: property.id.clone(),
                                preferred_component: None,
                            },
                            cx,
                        )),
                );
            }
        }

        property_content
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .into_any_element()
    }

    fn component_projection(&self) -> Option<sections::component::ComponentProjection> {
        let role = self.host.inspected_node().component_role()?;
        let identity = sections::component::ComponentIdentityProjection::new(
            self.id.clone(),
            self.host.inspected_node().id.clone(),
            self.command_target(),
        );
        let context = self.host.inspected_node().component_context.clone();
        let authoring = self.component_authoring_view_data();
        let authoring_projection = sections::component::ComponentAuthoringProjection::new(
            authoring.is_some_and(|authoring| {
                authoring.applied_properties.iter().any(|control| {
                    control.surface == DesignComponentPropertyApplicationSurface::NestedInstance
                })
            }),
            authoring.is_some(),
            authoring.is_some_and(|authoring| !authoring.exposure_candidates.is_empty()),
        );
        let rows = self
            .host
            .inspected_node()
            .component_properties
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, property)| {
                let (select_enabled, go_to_main_enabled) = match &property.origin {
                    DesignComponentPropertyOrigin::NestedInstance {
                        instance_id,
                        main_component,
                        ..
                    } => {
                        let select =
                            DesignPanelAction::ComponentPropertyNestedInstanceSelectRequested {
                                node_id: self.host.inspected_node().id.clone(),
                                property_id: property.id.clone(),
                                instance_id: instance_id.clone(),
                            };
                        let go_to_main = main_component.as_ref().map(|main_component| {
                            DesignPanelAction::ComponentPropertyNestedInstanceGoToMainRequested {
                                node_id: self.host.inspected_node().id.clone(),
                                property_id: property.id.clone(),
                                instance_id: instance_id.clone(),
                                main_component_id: main_component.id.clone(),
                            }
                        });
                        (
                            self.component_action_is_enabled(&select),
                            go_to_main
                                .as_ref()
                                .is_some_and(|action| self.component_action_is_enabled(action)),
                        )
                    }
                    DesignComponentPropertyOrigin::SelectedNode => (false, false),
                };
                sections::component::ComponentPropertyProjection::new(
                    index,
                    property,
                    sections::component::NestedInstanceActionAccess::new(
                        select_enabled,
                        go_to_main_enabled,
                    ),
                )
            })
            .collect();

        let reset_action = DesignPanelAction::ResetInstanceOverridesRequested {
            node_id: self.host.inspected_node().id.clone(),
        };
        let go_to_main_action = DesignPanelAction::GoToMainComponentRequested {
            node_id: self.host.inspected_node().id.clone(),
        };
        let detach_action = DesignPanelAction::DetachInstanceRequested {
            node_id: self.host.inspected_node().id.clone(),
        };
        let reset_visible = role.can_reset_instance_overrides()
            && context
                .as_ref()
                .is_some_and(|context| context.overrides.reset_state.can_reset());
        let go_to_main_visible = role.can_reset_instance_overrides()
            && context
                .as_ref()
                .and_then(|context| context.main_component.as_ref())
                .is_some();
        let detach_visible = role.can_reset_instance_overrides() && role.can_detach_instance();
        let multiline_property_id = self
            .edit
            .component_multiline
            .as_ref()
            .map(|editor| editor.property_id.clone());

        Some(sections::component::ComponentProjection::new(
            identity,
            sections::component::ComponentRoleProjection::new(role),
            sections::component::ComponentContextProjection::new(context),
            sections::component::ComponentPropertiesProjection::new(rows),
            authoring_projection,
            sections::component::ComponentPresentationProjection::new(
                multiline_property_id,
                sections::component::ComponentFooterPresentation::new(
                    (
                        reset_visible,
                        self.component_action_is_enabled(&reset_action),
                    ),
                    (
                        go_to_main_visible,
                        self.component_action_is_enabled(&go_to_main_action),
                    ),
                    (
                        detach_visible,
                        self.component_action_is_enabled(&detach_action),
                    ),
                ),
            ),
        ))
    }

    fn render_component(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = self.component_projection()?;
        Some(sections::component::render_component(
            &projection,
            self,
            sections::component::ComponentEventSink::new(cx.entity()),
            cx,
        ))
    }

    fn component_action_is_enabled(&self, action: &DesignPanelAction) -> bool {
        if !self
            .host
            .inspected_node()
            .supports_section(DesignPanelSection::Component)
            && !self
                .host
                .inspected_node()
                .supports_section(DesignPanelSection::Instance)
        {
            return false;
        }
        let Some(role) = self.host.inspected_node().component_role() else {
            return false;
        };
        if matches!(
            action,
            DesignPanelAction::ComponentPropertyDefinitionCreateRequested { .. }
                | DesignPanelAction::ComponentPropertyDefinitionRenameRequested { .. }
                | DesignPanelAction::ComponentPropertyDefinitionMetadataEditRequested { .. }
                | DesignPanelAction::ComponentPropertyDefinitionEditRequested { .. }
                | DesignPanelAction::ComponentPropertyDefinitionDeleteRequested { .. }
                | DesignPanelAction::ComponentPropertyDefinitionReorderRequested { .. }
                | DesignPanelAction::ComponentVariantOptionCreateRequested { .. }
                | DesignPanelAction::ComponentVariantOptionRenameRequested { .. }
                | DesignPanelAction::ComponentVariantOptionDeleteRequested { .. }
                | DesignPanelAction::ComponentVariantOptionReorderRequested { .. }
                | DesignPanelAction::ComponentPropertyApplyToLayerRequested { .. }
                | DesignPanelAction::ComponentPropertySwitchOnLayerRequested { .. }
                | DesignPanelAction::ComponentPropertyDetachFromLayerRequested { .. }
                | DesignPanelAction::NestedComponentPropertyExposeRequested { .. }
                | DesignPanelAction::NestedComponentPropertyUnexposeRequested { .. }
                | DesignPanelAction::NestedComponentPropertyPreviewRequested { .. }
        ) {
            return self.component_authoring_action_is_enabled(action);
        }
        let property = |property_id: &SharedString| {
            self.host
                .inspected_node()
                .component_properties
                .iter()
                .find(|property| &property.id == property_id)
        };
        let slot_child = |property_id: &SharedString, child_node_id: &SharedString| {
            let property = property(property_id)?;
            let DesignComponentPropertyValue::Slot(value) = property.effective_value() else {
                return None;
            };
            value
                .children
                .into_iter()
                .find(|child| &child.node_id == child_node_id)
        };
        if let DesignPanelAction::SlotChildSelectRequested {
            property_id,
            child_node_id,
            ..
        } = action
        {
            return role.can_modify_slot_instances()
                && slot_child(property_id, child_node_id)
                    .is_some_and(|child| child.capabilities.select);
        }
        if let DesignPanelAction::SlotLimitLayersSelectRequested {
            node_id,
            property_id,
            child_node_ids,
        } = action
        {
            let expected = property(property_id)
                .map(slot_preferred_violation_layer_ids)
                .unwrap_or_default();
            return node_id == &self.host.inspected_node().id
                && role.can_modify_slot_instances()
                && !expected.is_empty()
                && *child_node_ids == expected
                && child_node_ids.iter().all(|child_node_id| {
                    slot_child(property_id, child_node_id)
                        .is_some_and(|child| child.capabilities.select)
                });
        }
        if let DesignPanelAction::ComponentPropertyNestedInstanceSelectRequested {
            property_id,
            instance_id,
            ..
        } = action
        {
            return property(property_id)
                .is_some_and(|property| property.origin.nested_instance_id() == Some(instance_id));
        }
        if let DesignPanelAction::ComponentPropertyNestedInstanceGoToMainRequested {
            property_id,
            instance_id,
            main_component_id,
            ..
        } = action
        {
            return property(property_id).is_some_and(|property| {
                matches!(
                    &property.origin,
                    DesignComponentPropertyOrigin::NestedInstance {
                        instance_id: current_instance_id,
                        main_component: Some(main_component),
                        ..
                    } if current_instance_id == instance_id
                        && &main_component.id == main_component_id
                        && main_component.availability.is_available()
                )
            });
        }
        if !self.can_edit() {
            return false;
        }
        match action {
            DesignPanelAction::ResetInstanceOverridesRequested { .. } => {
                role.can_reset_instance_overrides()
                    && self
                        .host
                        .inspected_node()
                        .component_context
                        .as_ref()
                        .is_some_and(|context| context.overrides.reset_state.can_reset())
            }
            DesignPanelAction::GoToMainComponentRequested { .. } => {
                role.uses_instance_section()
                    && self
                        .host
                        .inspected_node()
                        .component_context
                        .as_ref()
                        .and_then(|context| context.main_component.as_ref())
                        .is_some_and(|main| main.availability.is_available())
            }
            DesignPanelAction::DetachInstanceRequested { .. } => role.can_detach_instance(),
            DesignPanelAction::ComponentPropertyResetRequested { property_id, .. } => {
                role.can_reset_instance_overrides()
                    && property(property_id)
                        .is_some_and(|property| property.reset_state.can_reset())
            }
            DesignPanelAction::SlotResetRequested { property_id, .. } => {
                role.can_modify_slot_instances()
                    && property(property_id)
                        .and_then(|property| property.slot_state.as_ref())
                        .is_some_and(|state| state.reset_state.can_reset())
            }
            DesignPanelAction::SlotClearRequested { property_id, .. } => {
                role.can_modify_slot_instances()
                    && property(property_id).is_some_and(|property| {
                        matches!(
                            property.effective_value(),
                            DesignComponentPropertyValue::Slot(_)
                        )
                    })
            }
            DesignPanelAction::SlotAddInstanceRequested {
                property_id,
                preferred_component,
                ..
            } => {
                if !role.can_modify_slot_instances() {
                    return false;
                }
                let Some(property) = property(property_id) else {
                    return false;
                };
                let Some(settings) = property.slot_settings() else {
                    return false;
                };
                let DesignComponentPropertyValue::Slot(_) = property.effective_value() else {
                    return false;
                };
                match preferred_component {
                    Some(reference) => {
                        reference.availability.is_available()
                            && (!settings.preferred_values_only
                                || settings
                                    .preferred_values
                                    .iter()
                                    .any(|preferred| preferred.id == reference.id))
                    }
                    None => {
                        !settings.preferred_values_only
                            || settings
                                .preferred_values
                                .iter()
                                .any(|preferred| preferred.availability.is_available())
                    }
                }
            }
            DesignPanelAction::SlotChildRemoveRequested {
                property_id,
                child_node_id,
                ..
            } => {
                role.can_modify_slot_instances()
                    && slot_child(property_id, child_node_id)
                        .is_some_and(|child| child.capabilities.remove)
            }
            DesignPanelAction::SlotChildReorderRequested {
                property_id,
                child_node_id,
                to_index,
                ..
            } => {
                role.can_modify_slot_instances()
                    && property(property_id).is_some_and(|property| {
                        let DesignComponentPropertyValue::Slot(value) = property.effective_value()
                        else {
                            return false;
                        };
                        *to_index < value.children.len()
                            && value
                                .children
                                .iter()
                                .find(|child| &child.node_id == child_node_id)
                                .is_some_and(|child| child.capabilities.reorder)
                    })
            }
            DesignPanelAction::SlotChildReplaceRequested {
                property_id,
                child_node_id,
                replacement,
                ..
            } => {
                if !role.can_modify_slot_instances() || !replacement.availability.is_available() {
                    return false;
                }
                let Some(property) = property(property_id) else {
                    return false;
                };
                property.slot_settings().is_some_and(|settings| {
                    (!settings.preferred_values_only
                        || settings
                            .preferred_values
                            .iter()
                            .any(|preferred| preferred.id == replacement.id))
                        && slot_child(property_id, child_node_id).is_some_and(|child| {
                            child.is_instance() && child.capabilities.replace_instance
                        })
                })
            }
            _ => false,
        }
    }

    fn emit_component_action(&self, action: DesignPanelAction, cx: &mut Context<Self>) {
        if self.component_action_is_enabled(&action) {
            cx.emit_design_panel_action(self, action);
        }
    }

    fn render_component_action_button(
        &self,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let label = label.into();
        let enabled = self.component_action_is_enabled(&action);
        let button_id = SharedString::from(format!("{}-{id_suffix}", self.id));
        let debug_button_id = button_id.clone();
        let mut button = div()
            .id(button_id)
            .debug_selector(move || debug_button_id.to_string())
            .h(px(ROW_HEIGHT))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(cx.theme().secondary)
            .text_xs()
            .when(enabled, |button| {
                button
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!enabled, |button| {
                button.text_color(cx.theme().muted_foreground).opacity(0.62)
            });
        if enabled {
            button = button.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_component_action(action.clone(), cx);
            }));
        }
        button.child(label).into_any_element()
    }
}

use super::*;

impl DesignPanel {
    pub(super) fn can_show_constraints(&self) -> bool {
        self.node.supports_constraints()
            && self.node.supports_section(DesignPanelSection::Position)
            && matches!(
                self.inspection_context.parent_layout(),
                DesignPanelParentLayout::Freeform
                    | DesignPanelParentLayout::AutoLayout {
                        participation: DesignPanelAutoLayoutParticipation::Ignored,
                        ..
                    }
            )
    }

    pub(super) fn smart_selection_axis(
        property: DesignPanelProperty,
    ) -> Option<DesignSmartSelectionAxis> {
        match property {
            DesignPanelProperty::SmartSelectionHorizontalSpacing => {
                Some(DesignSmartSelectionAxis::Horizontal)
            }
            DesignPanelProperty::SmartSelectionVerticalSpacing => {
                Some(DesignSmartSelectionAxis::Vertical)
            }
            _ => None,
        }
    }

    pub(super) fn smart_selection_property(axis: DesignSmartSelectionAxis) -> DesignPanelProperty {
        match axis {
            DesignSmartSelectionAxis::Horizontal => {
                DesignPanelProperty::SmartSelectionHorizontalSpacing
            }
            DesignSmartSelectionAxis::Vertical => {
                DesignPanelProperty::SmartSelectionVerticalSpacing
            }
        }
    }

    pub(super) fn smart_selection_view_data_for_context(
        &self,
    ) -> Option<&DesignSmartSelectionViewData> {
        let view_data = self.smart_selection_view_data.as_ref()?;
        (self.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
            && self.node.supports_arrange()
            && self.node.supports_section(DesignPanelSection::Position)
            && view_data.target == self.command_target()
            && view_data.is_valid())
        .then_some(view_data)
    }

    pub(super) fn smart_selection_spacing_is_editable(
        &self,
        axis: DesignSmartSelectionAxis,
    ) -> bool {
        self.can_edit()
            && self
                .smart_selection_view_data_for_context()
                .is_some_and(|view_data| view_data.spacing_is_editable(axis))
    }

    pub(super) fn smart_selection_property_is_mixed(&self, property: DesignPanelProperty) -> bool {
        Self::smart_selection_axis(property)
            .and_then(|axis| {
                self.smart_selection_view_data_for_context()
                    .and_then(|view_data| view_data.spacing(axis))
            })
            .is_some_and(|spacing| spacing.value.is_mixed())
    }

    pub(super) fn emit_smart_selection_arrange(
        &mut self,
        operation: DesignSmartSelectionOperation,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self
            .smart_selection_view_data_for_context()
            .filter(|view_data| view_data.operation_is_available(operation))
            .map(|view_data| view_data.target.clone())
        else {
            return;
        };
        if !self.can_edit() {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SmartSelectionArrangeRequested { target, operation },
        );
    }

    /// Returns whether this is a Smart Selection spacing property, including
    /// stale or rejected edits that must never fall through to a node leaf.
    pub(super) fn emit_smart_selection_spacing_edit(
        &mut self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(axis) = Self::smart_selection_axis(property) else {
            return false;
        };
        let DesignPanelValue::Number(value) = value else {
            return true;
        };
        let Some(target) = self
            .smart_selection_view_data_for_context()
            .map(|view_data| view_data.target.clone())
        else {
            return true;
        };
        if !value.is_finite()
            || (phase != DesignPanelEditPhase::Cancel
                && !self.smart_selection_spacing_is_editable(axis))
        {
            return true;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target,
                axis,
                value: *value,
                phase,
            },
        );
        true
    }

    pub(super) fn render_vector_edit_selection(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let view_data = self.active_vector_edit()?;
        let selected_ids = view_data.selected_vertex_ids();
        let selected_count = selected_ids.len();
        let x = view_data.selected_x();
        let y = view_data.selected_y();
        let corner_radius = view_data.selected_corner_radius();
        let handle_mirroring = view_data.selected_handle_mirroring();
        let first_selected = view_data.selected_vertices().next()?;
        let x_next = first_selected.position.x + 1.;
        let y_next = first_selected.position.y + 1.;
        let radius_next = first_selected.corner_radius.unwrap_or(0.) + 1.;
        let mirroring_next = match first_selected
            .handle_mirroring
            .unwrap_or(DesignHandleMirroring::None)
        {
            DesignHandleMirroring::None => DesignHandleMirroring::Angle,
            DesignHandleMirroring::Angle => DesignHandleMirroring::AngleAndLength,
            DesignHandleMirroring::AngleAndLength => DesignHandleMirroring::None,
        };
        let mut vertex_chips = h_flex().w_full().flex_wrap().gap_1();
        for vertex in &view_data.vertices {
            let vertex_id = vertex.id.clone();
            let selection_for_action = vec![vertex_id.clone()];
            let can_select = view_data.can_select_vertices();
            let chip = div()
                .id(SharedString::from(format!(
                    "{}-vector-vertex-{}",
                    self.id, vertex.id
                )))
                .h(px(ROW_HEIGHT))
                .max_w(px(120.))
                .px_2()
                .flex()
                .items_center()
                .rounded(px(4.))
                .border_1()
                .border_color(if vertex.selected {
                    cx.theme().selection
                } else {
                    cx.theme().transparent
                })
                .bg(if vertex.selected {
                    cx.theme().selection.opacity(0.2)
                } else {
                    cx.theme().secondary
                })
                .text_xs()
                .when(can_select, |chip| {
                    chip.key_context(CONTROL_KEY_CONTEXT)
                        .tab_index(0)
                        .cursor_pointer()
                        .hover(|style| style.border_color(cx.theme().muted_foreground))
                        .focus(|style| style.border_color(cx.theme().selection))
                        .on_activate(cx.listener(move |this, _, _, cx| {
                            this.emit_vector_vertex_selection(
                                selection_for_action.clone(),
                                DesignPanelEditPhase::Commit,
                                cx,
                            );
                        }))
                })
                .when(!can_select, |chip| chip.opacity(0.58))
                .child(div().truncate().child(vertex_id));
            vertex_chips = vertex_chips.child(chip);
        }

        let single_path_controls = !corner_radius.is_unset() || !handle_mirroring.is_unset();
        let mut controls = v_flex()
            .w_full()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Vector selection")
                    .child(format!("{selected_count} selected")),
            )
            .child(vertex_chips)
            .child(
                h_flex()
                    .gap_2()
                    .child(self.render_value_cell(
                        "vector-vertex-x",
                        "X",
                        format_vector_number_state(x),
                        DesignPanelProperty::VectorVertexX,
                        DesignPanelValue::Number(x_next),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "vector-vertex-y",
                        "Y",
                        format_vector_number_state(y),
                        DesignPanelProperty::VectorVertexY,
                        DesignPanelValue::Number(y_next),
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(self.render_value_cell(
                        "vector-vertex-corner-radius",
                        "R",
                        format_vector_number_state(corner_radius),
                        DesignPanelProperty::VectorVertexCornerRadius,
                        DesignPanelValue::Number(radius_next),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "vector-handle-mirroring",
                        "H",
                        format_vector_handle_state(handle_mirroring),
                        DesignPanelProperty::VectorHandleMirroring,
                        DesignPanelValue::HandleMirroring(mirroring_next),
                        cx,
                    )),
            );
        if !single_path_controls {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Branch topology: radius and handle mirroring are unavailable"),
            );
        }
        if view_data.read_only {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        view_data
                            .disabled_reason
                            .clone()
                            .unwrap_or_else(|| "Vector selection is read only".into()),
                    ),
            );
        } else if view_data.selected_vertices().any(|vertex| vertex.read_only) {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("One or more selected vertices are read only"),
            );
        }
        Some(controls.into_any_element())
    }

    pub(super) fn render_smart_selection_spacing_field(
        &self,
        view_data: &DesignSmartSelectionViewData,
        axis: DesignSmartSelectionAxis,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(spacing) = view_data.spacing(axis) else {
            return div().into_any_element();
        };
        let property = Self::smart_selection_property(axis);
        let (value, next) = match spacing.value {
            DesignSmartSelectionSpacingValue::Mixed => ("Mixed".into(), 0.),
            DesignSmartSelectionSpacingValue::Uniform(value) => {
                (SharedString::from(format_number(value)), value + 1.)
            }
        };
        let tooltip = if !self.can_edit() {
            "Editing is unavailable with the current permission".into()
        } else if let Some(reason) = view_data.read_only_reason.clone() {
            reason
        } else if let Some(reason) = spacing.availability.disabled_reason().cloned() {
            reason
        } else {
            axis.label().into()
        };
        div()
            .id(SharedString::from(format!(
                "{}-smart-selection-{:?}-field",
                self.id, axis
            )))
            .flex_1()
            .min_w(px(0.))
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .child(self.render_value_cell(
                match axis {
                    DesignSmartSelectionAxis::Horizontal => "smart-selection-horizontal-spacing",
                    DesignSmartSelectionAxis::Vertical => "smart-selection-vertical-spacing",
                },
                axis.glyph(),
                value,
                property,
                DesignPanelValue::Number(next),
                cx,
            ))
            .into_any_element()
    }

    pub(super) fn render_smart_selection_operation_button(
        &self,
        view_data: &DesignSmartSelectionViewData,
        operation: DesignSmartSelectionOperation,
        availability: &DesignSmartSelectionAvailability,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled =
            self.can_edit() && view_data.read_only_reason.is_none() && availability.is_available();
        let tooltip = if !self.can_edit() {
            "Editing is unavailable with the current permission".into()
        } else if let Some(reason) = view_data.read_only_reason.clone() {
            reason
        } else if let Some(reason) = availability.disabled_reason().cloned() {
            reason
        } else {
            operation.label().into()
        };
        let (id_suffix, label) = match operation {
            DesignSmartSelectionOperation::DistributeHorizontal => {
                ("smart-selection-distribute-horizontal", "↔")
            }
            DesignSmartSelectionOperation::DistributeVertical => {
                ("smart-selection-distribute-vertical", "↕")
            }
            DesignSmartSelectionOperation::TidyUp => ("smart-selection-tidy-up", "Tidy up"),
        };
        let selector = SharedString::from(format!("{}-{id_suffix}", self.id));
        let debug_selector = selector.to_string();
        let mut button = div()
            .id(selector)
            .debug_selector(move || debug_selector)
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(cx.theme().secondary)
            .text_xs()
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
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
                this.emit_smart_selection_arrange(operation, cx);
            }));
        }
        button.child(label).into_any_element()
    }

    pub(super) fn render_smart_selection_controls(
        &self,
        view_data: &DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut controls = v_flex().gap_1();
        let mut spacing_row = h_flex().w_full().gap_2();
        let mut has_spacing = false;
        for axis in DesignSmartSelectionAxis::ALL {
            if view_data.spacing(axis).is_some() {
                has_spacing = true;
                spacing_row = spacing_row
                    .child(self.render_smart_selection_spacing_field(view_data, axis, cx));
            }
        }
        if has_spacing {
            controls = controls
                .child(self.render_group_label("Space between", cx))
                .child(spacing_row);
        }

        let mut operation_row = h_flex().w_full().gap_2();
        let mut has_operation = false;
        for operation in DesignSmartSelectionOperation::ALL {
            if let Some(availability) = view_data.operation_availability(operation) {
                has_operation = true;
                operation_row = operation_row.child(self.render_smart_selection_operation_button(
                    view_data,
                    operation,
                    availability,
                    cx,
                ));
            }
        }
        if has_operation {
            controls = controls
                .child(self.render_group_label("Arrange", cx))
                .child(operation_row);
        }
        if let Some(reason) = view_data.read_only_reason.clone() {
            controls = controls.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(reason),
            );
        }
        controls.into_any_element()
    }

    pub(super) fn render_position(&self, cx: &mut Context<Self>) -> AnyElement {
        let target = self.command_target();
        let grid_child_item = (self.node.supports_auto_layout_child()
            && self
                .inspection_context
                .parent_layout()
                .auto_layout_direction()
                == Some(DesignPanelAutoLayoutDirection::Grid)
            && self
                .inspection_context
                .parent_layout()
                .participates_in_auto_layout())
        .then(|| self.node.layout.as_ref().map(|layout| layout.item.clone()))
        .flatten();
        let can_arrange = self.node.supports_arrange()
            && self.can_edit()
            && self.inspection_context.parent_layout() != DesignPanelParentLayout::Canvas;
        let horizontal_alignment = if let Some(item) = grid_child_item.as_ref() {
            h_flex()
                .flex_1()
                .gap_0p5()
                .rounded(px(5.))
                .bg(cx.theme().secondary)
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-left",
                    PositionActionIcon::AlignLeft,
                    DesignPanelProperty::GridHorizontalAlignment,
                    DesignGridItemAlignment::Start,
                    item.grid_horizontal_alignment == DesignGridItemAlignment::Start,
                    cx,
                ))
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-horizontal-center",
                    PositionActionIcon::AlignHorizontalCenter,
                    DesignPanelProperty::GridHorizontalAlignment,
                    DesignGridItemAlignment::Center,
                    item.grid_horizontal_alignment == DesignGridItemAlignment::Center,
                    cx,
                ))
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-right",
                    PositionActionIcon::AlignRight,
                    DesignPanelProperty::GridHorizontalAlignment,
                    DesignGridItemAlignment::End,
                    item.grid_horizontal_alignment == DesignGridItemAlignment::End,
                    cx,
                ))
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .gap_0p5()
                .rounded(px(5.))
                .bg(cx.theme().secondary)
                .child(self.render_position_action_button_with_enabled(
                    "align-left",
                    PositionActionIcon::AlignLeft,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignLeft,
                    },
                    can_arrange,
                    cx,
                ))
                .child(self.render_position_action_button_with_enabled(
                    "align-horizontal-center",
                    PositionActionIcon::AlignHorizontalCenter,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignHorizontalCenter,
                    },
                    can_arrange,
                    cx,
                ))
                .child(self.render_position_action_button_with_enabled(
                    "align-right",
                    PositionActionIcon::AlignRight,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignRight,
                    },
                    can_arrange,
                    cx,
                ))
                .into_any_element()
        };
        let vertical_alignment = if let Some(item) = grid_child_item.as_ref() {
            h_flex()
                .flex_1()
                .gap_0p5()
                .rounded(px(5.))
                .bg(cx.theme().secondary)
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-top",
                    PositionActionIcon::AlignTop,
                    DesignPanelProperty::GridVerticalAlignment,
                    DesignGridItemAlignment::Start,
                    item.grid_vertical_alignment == DesignGridItemAlignment::Start,
                    cx,
                ))
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-vertical-center",
                    PositionActionIcon::AlignVerticalCenter,
                    DesignPanelProperty::GridVerticalAlignment,
                    DesignGridItemAlignment::Center,
                    item.grid_vertical_alignment == DesignGridItemAlignment::Center,
                    cx,
                ))
                .child(self.render_grid_child_alignment_button(
                    "grid-child-align-bottom",
                    PositionActionIcon::AlignBottom,
                    DesignPanelProperty::GridVerticalAlignment,
                    DesignGridItemAlignment::End,
                    item.grid_vertical_alignment == DesignGridItemAlignment::End,
                    cx,
                ))
                .into_any_element()
        } else {
            h_flex()
                .flex_1()
                .gap_0p5()
                .rounded(px(5.))
                .bg(cx.theme().secondary)
                .child(self.render_position_action_button_with_enabled(
                    "align-top",
                    PositionActionIcon::AlignTop,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignTop,
                    },
                    can_arrange,
                    cx,
                ))
                .child(self.render_position_action_button_with_enabled(
                    "align-vertical-center",
                    PositionActionIcon::AlignVerticalCenter,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignVerticalCenter,
                    },
                    can_arrange,
                    cx,
                ))
                .child(self.render_position_action_button_with_enabled(
                    "align-bottom",
                    PositionActionIcon::AlignBottom,
                    DesignPanelAction::ArrangeRequested {
                        target: target.clone(),
                        operation: DesignArrangeOperation::AlignBottom,
                    },
                    can_arrange,
                    cx,
                ))
                .into_any_element()
        };
        let can_constrain = self.can_show_constraints();
        let disclosure = div()
            .id(SharedString::from(format!(
                "{}-constraints-disclosure",
                self.id
            )))
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .when(can_constrain, |button| {
                button
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_1()
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!can_constrain, |button| button.opacity(0.38))
            .on_activate(cx.listener(move |this, _, _, cx| {
                if can_constrain {
                    this.constraints_expanded = !this.constraints_expanded;
                    cx.notify();
                }
            }))
            .when(self.constraints_expanded, |button| {
                button
                    .bg(cx.theme().selection.opacity(0.22))
                    .text_color(cx.theme().selection)
            })
            .child(Icon::new(IconName::Inspector).xsmall());
        let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_1();
        if let Some(vector_edit) = self.render_vector_edit_selection(cx) {
            content = content
                .child(vector_edit)
                .child(div().h(px(6.)).flex_none());
        }
        if self.node.supports_arrange() || grid_child_item.is_some() {
            content = content
                .child(self.render_group_label("Alignment", cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(horizontal_alignment)
                        .child(vertical_alignment)
                        .child(div().size(px(24.)).flex_none()),
                );
        }
        if self.node.supports_position_coordinates() {
            content = content
                .child(self.render_group_label("Position", cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "x",
                            "X",
                            format_number(self.node.x),
                            DesignPanelProperty::X,
                            DesignPanelValue::Number(self.node.x + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "y",
                            "Y",
                            format_number(self.node.y),
                            DesignPanelProperty::Y,
                            DesignPanelValue::Number(self.node.y + 1.),
                            cx,
                        ))
                        .when(can_constrain, |row| row.child(disclosure))
                        .when(!can_constrain, |row| {
                            row.child(div().size(px(24.)).flex_none())
                        }),
                );
        } else if can_constrain {
            content = content
                .child(self.render_group_label("Constraints", cx))
                .child(h_flex().justify_end().child(disclosure));
        }
        if can_constrain && self.constraints_expanded {
            content = content
                .child(self.render_group_label("Constraints", cx))
                .child(self.render_constraints_controls(cx));
        }
        if self.node.supports_transforms() {
            content = content
                .child(self.render_group_label("Rotation", cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "rotation",
                            "∟",
                            format!("{}°", format_number(normalize_rotation(self.node.rotation))),
                            DesignPanelProperty::Rotation,
                            DesignPanelValue::Number((self.node.rotation + 15.) % 360.),
                            cx,
                        ))
                        .child(
                            h_flex()
                                .h(px(ROW_HEIGHT))
                                .flex_1()
                                .overflow_hidden()
                                .rounded(px(4.))
                                .bg(cx.theme().secondary)
                                .child(self.render_position_action_button(
                                    "rotate-clockwise-90",
                                    PositionActionIcon::RotateClockwise90,
                                    DesignPanelAction::TransformRequested {
                                        target: target.clone(),
                                        operation: DesignTransformOperation::RotateClockwise90,
                                    },
                                    cx,
                                ))
                                .child(self.render_position_action_button(
                                    "flip-horizontal",
                                    PositionActionIcon::FlipHorizontal,
                                    DesignPanelAction::TransformRequested {
                                        target: target.clone(),
                                        operation: DesignTransformOperation::FlipHorizontal,
                                    },
                                    cx,
                                ))
                                .child(self.render_position_action_button(
                                    "flip-vertical",
                                    PositionActionIcon::FlipVertical,
                                    DesignPanelAction::TransformRequested {
                                        target,
                                        operation: DesignTransformOperation::FlipVertical,
                                    },
                                    cx,
                                )),
                        )
                        .child(div().size(px(24.)).flex_none()),
                );
        }
        if self.node.supports_arrange()
            && self.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
        {
            if let Some(view_data) = self.smart_selection_view_data_for_context() {
                content = content.child(self.render_smart_selection_controls(view_data, cx));
            } else if self.smart_selection_view_data.is_none() {
                content = content.child(
                    h_flex()
                        .gap_2()
                        .child(self.render_action_button(
                            "distribute-horizontal",
                            "Distribute ↔",
                            DesignPanelAction::ArrangeRequested {
                                target: self.command_target(),
                                operation: DesignArrangeOperation::DistributeHorizontal,
                            },
                            cx,
                        ))
                        .child(self.render_action_button(
                            "distribute-vertical",
                            "Distribute ↕",
                            DesignPanelAction::ArrangeRequested {
                                target: self.command_target(),
                                operation: DesignArrangeOperation::DistributeVertical,
                            },
                            cx,
                        ))
                        .child(self.render_action_button(
                            "tidy-selection",
                            "Tidy",
                            DesignPanelAction::ArrangeRequested {
                                target: self.command_target(),
                                operation: DesignArrangeOperation::TidyUp,
                            },
                            cx,
                        )),
                );
            }
        }
        if self.renders_draw_workspace() {
            let id = SharedString::from(format!("{}-draw-position-content", self.id));
            let selector = id.to_string();
            v_flex()
                .id(id)
                .debug_selector(move || selector.clone())
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .child(content.into_any_element())
                .into_any_element()
        } else {
            self.render_section(
                DesignPanelSection::Position,
                None,
                content.into_any_element(),
                cx,
            )
        }
    }

    pub(super) fn render_constraints_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let horizontal = next_horizontal_constraint(self.node.horizontal_constraint);
        let vertical = next_vertical_constraint(self.node.vertical_constraint);
        let diagram = div()
            .w(px(96.))
            .h(px(56.))
            .relative()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .absolute()
                    .left(px(14.))
                    .top(px(10.))
                    .w(px(42.))
                    .h(px(34.))
                    .rounded(px(3.))
                    .border_1()
                    .border_color(cx.theme().selection),
            )
            .child(
                div()
                    .absolute()
                    .left(px(34.))
                    .top(px(4.))
                    .w(px(2.))
                    .h(px(48.))
                    .bg(cx.theme().selection.opacity(0.65)),
            );
        h_flex()
            .pt_1()
            .gap_3()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .gap_2()
                    .child(self.render_value_cell(
                        "horizontal-constraint",
                        "↔",
                        self.node.horizontal_constraint.label(),
                        DesignPanelProperty::HorizontalConstraint,
                        DesignPanelValue::Constraint(horizontal),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "vertical-constraint",
                        "↕",
                        self.node.vertical_constraint.label(),
                        DesignPanelProperty::VerticalConstraint,
                        DesignPanelValue::Constraint(vertical),
                        cx,
                    )),
            )
            .child(diagram)
            .into_any_element()
    }

    pub(super) fn render_constraints(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_section(
            DesignPanelSection::Constraints,
            None,
            self.render_constraints_controls(cx),
            cx,
        )
    }

    pub(super) fn apply_alignment_grid_shortcut(
        &mut self,
        shortcut: AlignmentGridShortcut,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(layout) = self.node.layout.clone() else {
            return false;
        };
        match shortcut {
            AlignmentGridShortcut::Up
            | AlignmentGridShortcut::Right
            | AlignmentGridShortcut::Down
            | AlignmentGridShortcut::Left
            | AlignmentGridShortcut::SetTopEdge
            | AlignmentGridShortcut::SetRightEdge
            | AlignmentGridShortcut::SetBottomEdge
            | AlignmentGridShortcut::SetLeftEdge => {
                if !self.property_is_editable(DesignPanelProperty::AutoLayoutAlignment) {
                    return false;
                }
                if let Some(alignment) = alignment_for_shortcut(&layout, shortcut) {
                    self.emit_property(
                        DesignPanelProperty::AutoLayoutAlignment,
                        DesignPanelValue::AutoLayoutAlignment(alignment),
                        cx,
                    );
                }
                true
            }
            AlignmentGridShortcut::ToggleSpacing => {
                if !self.property_is_editable(DesignPanelProperty::ItemSpacingMode) {
                    return false;
                }
                let next = match layout.item_spacing_mode {
                    DesignItemSpacingMode::Fixed => DesignItemSpacingMode::Auto,
                    DesignItemSpacingMode::Auto => DesignItemSpacingMode::Fixed,
                };
                self.emit_property(
                    DesignPanelProperty::ItemSpacingMode,
                    DesignPanelValue::ItemSpacingMode(next),
                    cx,
                );
                true
            }
            AlignmentGridShortcut::ToggleBaseline => {
                if !self.property_is_editable(DesignPanelProperty::BaselineAlignment) {
                    return false;
                }
                let next = match layout.baseline_alignment {
                    DesignBaselineAlignment::Bounds => DesignBaselineAlignment::Baseline,
                    DesignBaselineAlignment::Baseline => DesignBaselineAlignment::Bounds,
                };
                self.emit_property(
                    DesignPanelProperty::BaselineAlignment,
                    DesignPanelValue::BaselineAlignment(next),
                    cx,
                );
                true
            }
        }
    }

    pub(super) fn handle_alignment_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shortcut) = AlignmentGridShortcut::from_key_down(event) else {
            return;
        };
        if self.apply_alignment_grid_shortcut(shortcut, cx) {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn render_alignment_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let layout = self.node.layout.clone().unwrap_or_default();
        let selected_x = layout.alignment_x;
        let selected_y = layout.alignment_y;
        let (rows, width, height) = alignment_grid_candidates(&layout);
        let editable = self.property_is_editable(DesignPanelProperty::AutoLayoutAlignment);
        let keyboard_enabled = editable
            || self.property_is_editable(DesignPanelProperty::ItemSpacingMode)
            || self.property_is_editable(DesignPanelProperty::BaselineAlignment);
        let mut grid = v_flex()
            .id(SharedString::from(format!("{}-alignment-grid", self.id)))
            .w(px(width))
            .h(px(height))
            .p_1()
            .gap_1()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(cx.theme().secondary)
            .when(keyboard_enabled, |grid| {
                grid.key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .focus(|style| style.border_color(cx.theme().selection))
                    .on_key_down(cx.listener(Self::handle_alignment_grid_key_down))
            })
            .when(!keyboard_enabled, |grid| grid.opacity(0.62));
        for candidates in rows {
            let mut row = h_flex().flex_1().gap_1();
            for (x, y) in candidates {
                let selected = selected_x == x && selected_y == y;
                let alignment = DesignAutoLayoutAlignment::new(x, y);
                let cell = div()
                    .id(SharedString::from(format!("{}-align-{x}-{y}", self.id)))
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .when(!editable, |cell| cell.opacity(0.62))
                    .when(selected, |cell| cell.bg(cx.theme().selection.opacity(0.38)))
                    .when(editable, |cell| {
                        cell.cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                if !event.is_keyboard() {
                                    this.emit_property(
                                        DesignPanelProperty::AutoLayoutAlignment,
                                        DesignPanelValue::AutoLayoutAlignment(alignment),
                                        cx,
                                    );
                                }
                            }))
                    })
                    .child(
                        div()
                            .size(px(4.))
                            .rounded(px(2.))
                            .bg(cx.theme().muted_foreground),
                    );
                row = row.child(cell);
            }
            grid = grid.child(row);
        }
        grid.into_any_element()
    }

    pub(super) fn render_position_action_icon(
        &self,
        icon: PositionActionIcon,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = if enabled {
            cx.theme().foreground
        } else {
            cx.theme().muted_foreground
        };
        render_icon_canvas(color, 16., move |path| match icon {
            PositionActionIcon::AlignLeft
            | PositionActionIcon::AlignHorizontalCenter
            | PositionActionIcon::AlignRight => {
                let anchor_x = match icon {
                    PositionActionIcon::AlignLeft => 2.,
                    PositionActionIcon::AlignHorizontalCenter => 8.,
                    PositionActionIcon::AlignRight => 14.,
                    _ => unreachable!(),
                };
                path.line((anchor_x, 1.5), (anchor_x, 14.5));
                for (line_y, width) in [(4., 10.), (8., 7.), (12., 11.)] {
                    let start_x = match icon {
                        PositionActionIcon::AlignLeft => anchor_x + 2.,
                        PositionActionIcon::AlignHorizontalCenter => anchor_x - width / 2.,
                        PositionActionIcon::AlignRight => anchor_x - width - 2.,
                        _ => unreachable!(),
                    };
                    path.line((start_x, line_y), (start_x + width, line_y));
                }
            }
            PositionActionIcon::AlignTop
            | PositionActionIcon::AlignVerticalCenter
            | PositionActionIcon::AlignBottom => {
                let anchor_y = match icon {
                    PositionActionIcon::AlignTop => 2.,
                    PositionActionIcon::AlignVerticalCenter => 8.,
                    PositionActionIcon::AlignBottom => 14.,
                    _ => unreachable!(),
                };
                path.line((1.5, anchor_y), (14.5, anchor_y));
                for (line_x, height) in [(4., 10.), (8., 7.), (12., 11.)] {
                    let start_y = match icon {
                        PositionActionIcon::AlignTop => anchor_y + 2.,
                        PositionActionIcon::AlignVerticalCenter => anchor_y - height / 2.,
                        PositionActionIcon::AlignBottom => anchor_y - height - 2.,
                        _ => unreachable!(),
                    };
                    path.line((line_x, start_y), (line_x, start_y + height));
                }
            }
            PositionActionIcon::RotateClockwise90 => {
                path.diamond(8., 9., 5.);
                path.move_to(3., 5.);
                path.cubic_to((12., 3.), (5., 1.), (10., 1.));
                path.line((10., 1.), (8., 3.));
                path.line((10., 1.), (12., 3.));
            }
            PositionActionIcon::FlipHorizontal => {
                path.line((8., 1.5), (8., 14.5));
                path.poly([(2., 4.), (6.5, 8.), (2., 12.)], true);
                path.poly([(14., 4.), (9.5, 8.), (14., 12.)], true);
            }
            PositionActionIcon::FlipVertical => {
                path.line((1.5, 8.), (14.5, 8.));
                path.poly([(4., 2.), (8., 6.5), (12., 2.)], true);
                path.poly([(4., 14.), (8., 9.5), (12., 14.)], true);
            }
            PositionActionIcon::LockAspectRatio => {
                path.rect(2., 2., 14., 14.);
                path.poly([(7., 5.), (5., 5.), (5., 7.)], false);
                path.poly([(9., 11.), (11., 11.), (11., 9.)], false);
            }
        })
    }

    pub(super) fn render_position_action_button(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_position_action_button_with_enabled(
            id_suffix,
            icon,
            action,
            self.can_edit(),
            cx,
        )
    }

    pub(super) fn render_position_action_button_with_enabled(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        action: DesignPanelAction,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut button = div()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .h(px(ROW_HEIGHT))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
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
                cx.emit_design_panel_action(this, action.clone());
            }));
        }
        button
            .child(self.render_position_action_icon(icon, enabled, cx))
            .into_any_element()
    }

    pub(super) fn render_grid_child_alignment_button(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        property: DesignPanelProperty,
        value: DesignGridItemAlignment,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.property_is_editable(property);
        let mut button = div()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .h(px(ROW_HEIGHT))
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .when(selected, |button| {
                button
                    .bg(cx.theme().selection.opacity(0.22))
                    .text_color(cx.theme().selection)
            })
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
            .when(!enabled, |button| button.opacity(0.62));
        if enabled {
            button = button.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property(property, DesignPanelValue::GridItemAlignment(value), cx);
            }));
        }
        button
            .child(self.render_position_action_icon(icon, enabled, cx))
            .into_any_element()
    }
}

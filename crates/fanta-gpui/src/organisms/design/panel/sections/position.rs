use super::super::*;

/// Immutable read model consumed by the Position surface.
///
/// This deliberately projects only Position-owned values and capabilities;
/// the renderer cannot reach back into the Design-panel facade. Every event
/// returns through [`dispatch`], where the latest host snapshot is checked.
#[derive(Clone)]
pub(in super::super) struct PositionProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
    can_edit: bool,
    can_arrange: bool,
    can_constrain: bool,
    constraints_expanded: bool,
    renders_draw_workspace: bool,
    supports_arrange: bool,
    supports_position_coordinates: bool,
    supports_transforms: bool,
    grid_horizontal_alignment: Option<DesignGridItemAlignment>,
    grid_vertical_alignment: Option<DesignGridItemAlignment>,
    grid_horizontal_editable: bool,
    grid_vertical_editable: bool,
    x: f32,
    y: f32,
    rotation: f32,
    horizontal_constraint: DesignConstraint,
    vertical_constraint: DesignConstraint,
    selection_is_multiple: bool,
    smart_selection: Option<DesignSmartSelectionViewData>,
    smart_selection_projection_present: bool,
    vector_edit: Option<DesignVectorEditViewData>,
}

#[derive(Clone)]
pub(in super::super) struct PositionIdentityProjection {
    panel_id: SharedString,
    target: DesignPanelTarget,
}

impl PositionIdentityProjection {
    pub(in super::super) fn new(panel_id: SharedString, target: DesignPanelTarget) -> Self {
        Self { panel_id, target }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct PositionAccessProjection {
    can_edit: bool,
    can_arrange: bool,
    can_constrain: bool,
    grid_horizontal_editable: bool,
    grid_vertical_editable: bool,
}

impl PositionAccessProjection {
    pub(in super::super) const fn new(
        can_edit: bool,
        can_arrange: bool,
        can_constrain: bool,
        grid_horizontal_editable: bool,
        grid_vertical_editable: bool,
    ) -> Self {
        Self {
            can_edit,
            can_arrange,
            can_constrain,
            grid_horizontal_editable,
            grid_vertical_editable,
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct PositionCapabilityProjection {
    supports_arrange: bool,
    supports_position_coordinates: bool,
    supports_transforms: bool,
}

impl PositionCapabilityProjection {
    pub(in super::super) const fn new(
        supports_arrange: bool,
        supports_position_coordinates: bool,
        supports_transforms: bool,
    ) -> Self {
        Self {
            supports_arrange,
            supports_position_coordinates,
            supports_transforms,
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct PositionPresentationProjection {
    constraints_expanded: bool,
    renders_draw_workspace: bool,
}

impl PositionPresentationProjection {
    pub(in super::super) const fn new(
        constraints_expanded: bool,
        renders_draw_workspace: bool,
    ) -> Self {
        Self {
            constraints_expanded,
            renders_draw_workspace,
        }
    }
}

#[derive(Clone, Copy)]
pub(in super::super) struct PositionValueProjection {
    grid_horizontal_alignment: Option<DesignGridItemAlignment>,
    grid_vertical_alignment: Option<DesignGridItemAlignment>,
    x: f32,
    y: f32,
    rotation: f32,
    horizontal_constraint: DesignConstraint,
    vertical_constraint: DesignConstraint,
}

impl PositionValueProjection {
    pub(in super::super) const fn new(
        grid_horizontal_alignment: Option<DesignGridItemAlignment>,
        grid_vertical_alignment: Option<DesignGridItemAlignment>,
        x: f32,
        y: f32,
        rotation: f32,
        horizontal_constraint: DesignConstraint,
        vertical_constraint: DesignConstraint,
    ) -> Self {
        Self {
            grid_horizontal_alignment,
            grid_vertical_alignment,
            x,
            y,
            rotation,
            horizontal_constraint,
            vertical_constraint,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct PositionSelectionProjection {
    selection_is_multiple: bool,
    smart_selection: Option<DesignSmartSelectionViewData>,
    smart_selection_projection_present: bool,
    vector_edit: Option<DesignVectorEditViewData>,
}

impl PositionSelectionProjection {
    pub(in super::super) fn new(
        selection_is_multiple: bool,
        smart_selection: Option<DesignSmartSelectionViewData>,
        smart_selection_projection_present: bool,
        vector_edit: Option<DesignVectorEditViewData>,
    ) -> Self {
        Self {
            selection_is_multiple,
            smart_selection,
            smart_selection_projection_present,
            vector_edit,
        }
    }
}

impl PositionProjection {
    pub(in super::super) fn new(
        identity: PositionIdentityProjection,
        access: PositionAccessProjection,
        capabilities: PositionCapabilityProjection,
        presentation: PositionPresentationProjection,
        values: PositionValueProjection,
        selection: PositionSelectionProjection,
    ) -> Self {
        Self {
            panel_id: identity.panel_id,
            target: identity.target,
            can_edit: access.can_edit,
            can_arrange: access.can_arrange,
            can_constrain: access.can_constrain,
            constraints_expanded: presentation.constraints_expanded,
            renders_draw_workspace: presentation.renders_draw_workspace,
            supports_arrange: capabilities.supports_arrange,
            supports_position_coordinates: capabilities.supports_position_coordinates,
            supports_transforms: capabilities.supports_transforms,
            grid_horizontal_alignment: values.grid_horizontal_alignment,
            grid_vertical_alignment: values.grid_vertical_alignment,
            grid_horizontal_editable: access.grid_horizontal_editable,
            grid_vertical_editable: access.grid_vertical_editable,
            x: values.x,
            y: values.y,
            rotation: values.rotation,
            horizontal_constraint: values.horizontal_constraint,
            vertical_constraint: values.vertical_constraint,
            selection_is_multiple: selection.selection_is_multiple,
            smart_selection: selection.smart_selection,
            smart_selection_projection_present: selection.smart_selection_projection_present,
            vector_edit: selection.vector_edit,
        }
    }
}

/// Narrow access to established field and section chrome.
pub(in super::super) trait PositionInspectorChrome {
    fn position_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn position_group_label(
        &self,
        label: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn position_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl PositionInspectorChrome for DesignPanel {
    fn position_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_cell(id_suffix, prefix, value, property, next, cx)
    }

    fn position_group_label(
        &self,
        label: &'static str,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_group_label(label, cx)
    }

    fn position_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(section, None, content, cx)
    }
}

#[derive(Clone)]
enum PositionEvent {
    Action(Box<DesignPanelAction>),
    Property(DesignPanelProperty, DesignPanelValue),
    ToggleConstraints,
    SelectVertices(Vec<SharedString>),
    SmartSelectionArrange(DesignSmartSelectionOperation),
}

/// Typed event channel retained by Position renderers.
#[derive(Clone)]
pub(in super::super) struct PositionEventSink {
    panel: Entity<DesignPanel>,
}

impl PositionEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(&self, event: PositionEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }
}

fn action_event(action: DesignPanelAction) -> PositionEvent {
    PositionEvent::Action(Box::new(action))
}

fn dispatch(panel: &mut DesignPanel, event: PositionEvent, cx: &mut Context<DesignPanel>) {
    match event {
        PositionEvent::Action(action) => {
            let action = *action;
            let target_is_current = match &action {
                DesignPanelAction::ArrangeRequested { target, .. }
                | DesignPanelAction::TransformRequested { target, .. } => {
                    *target == panel.command_target()
                }
                DesignPanelAction::PropertyChangeRequested { node_id, .. } => {
                    *node_id == panel.host.inspected_node().id
                }
                _ => false,
            };
            if panel.can_edit() && target_is_current && panel.node_capability_allows_action(&action)
            {
                cx.emit_design_panel_action(panel, action);
            }
        }
        PositionEvent::Property(property, value) => panel.emit_property(property, value, cx),
        PositionEvent::ToggleConstraints => {
            if can_show_constraints(panel) {
                panel.sections.toggle_constraints();
                cx.notify();
            }
        }
        PositionEvent::SelectVertices(selected_vertex_ids) => panel.emit_vector_vertex_selection(
            selected_vertex_ids,
            DesignPanelEditPhase::Commit,
            cx,
        ),
        PositionEvent::SmartSelectionArrange(operation) => {
            emit_smart_selection_arrange(panel, operation, cx);
        }
    }
}

pub(in super::super) fn can_show_constraints(panel: &DesignPanel) -> bool {
    panel.host.inspected_node().supports_constraints()
        && panel
            .host
            .inspected_node()
            .supports_section(DesignPanelSection::Position)
        && matches!(
            panel.host.inspection_context.parent_layout(),
            DesignPanelParentLayout::Freeform
                | DesignPanelParentLayout::AutoLayout {
                    participation: DesignPanelAutoLayoutParticipation::Ignored,
                    ..
                }
        )
}

pub(in super::super) fn smart_selection_axis(
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

pub(in super::super) fn smart_selection_property(
    axis: DesignSmartSelectionAxis,
) -> DesignPanelProperty {
    match axis {
        DesignSmartSelectionAxis::Horizontal => {
            DesignPanelProperty::SmartSelectionHorizontalSpacing
        }
        DesignSmartSelectionAxis::Vertical => DesignPanelProperty::SmartSelectionVerticalSpacing,
    }
}

pub(in super::super) fn smart_selection_view_data_for_context(
    panel: &DesignPanel,
) -> Option<&DesignSmartSelectionViewData> {
    let view_data = panel.host.projections.smart_selection.as_ref()?;
    (panel.host.inspection_context.selection().kind() == DesignPanelSelectionKind::Multiple
        && panel.host.inspected_node().supports_arrange()
        && panel
            .host
            .inspected_node()
            .supports_section(DesignPanelSection::Position)
        && view_data.target == panel.command_target()
        && view_data.is_valid())
    .then_some(view_data)
}

pub(in super::super) fn smart_selection_spacing_is_editable(
    panel: &DesignPanel,
    axis: DesignSmartSelectionAxis,
) -> bool {
    panel.can_edit()
        && smart_selection_view_data_for_context(panel)
            .is_some_and(|view_data| view_data.spacing_is_editable(axis))
}

pub(in super::super) fn smart_selection_property_is_mixed(
    panel: &DesignPanel,
    property: DesignPanelProperty,
) -> bool {
    smart_selection_axis(property)
        .and_then(|axis| {
            smart_selection_view_data_for_context(panel)
                .and_then(|view_data| view_data.spacing(axis))
        })
        .is_some_and(|spacing| spacing.value.is_mixed())
}

pub(in super::super) fn emit_smart_selection_arrange(
    panel: &mut DesignPanel,
    operation: DesignSmartSelectionOperation,
    cx: &mut Context<DesignPanel>,
) {
    let Some(target) = smart_selection_view_data_for_context(panel)
        .filter(|view_data| view_data.operation_is_available(operation))
        .map(|view_data| view_data.target.clone())
    else {
        return;
    };
    if !panel.can_edit() {
        return;
    }
    cx.emit_design_panel_action(
        panel,
        DesignPanelAction::SmartSelectionArrangeRequested { target, operation },
    );
}

pub(in super::super) fn emit_smart_selection_spacing_edit(
    panel: &mut DesignPanel,
    property: DesignPanelProperty,
    value: &DesignPanelValue,
    phase: DesignPanelEditPhase,
    cx: &mut Context<DesignPanel>,
) -> bool {
    let Some(axis) = smart_selection_axis(property) else {
        return false;
    };
    let DesignPanelValue::Number(value) = value else {
        return true;
    };
    let Some(target) =
        smart_selection_view_data_for_context(panel).map(|view_data| view_data.target.clone())
    else {
        return true;
    };
    if !value.is_finite()
        || (phase != DesignPanelEditPhase::Cancel
            && !smart_selection_spacing_is_editable(panel, axis))
    {
        return true;
    }
    cx.emit_design_panel_action(
        panel,
        DesignPanelAction::SmartSelectionSpacingEditRequested {
            target,
            axis,
            value: *value,
            phase,
        },
    );
    true
}

fn render_action_icon(
    icon: PositionActionIcon,
    enabled: bool,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let color = if enabled {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    let icon = match icon {
        PositionActionIcon::AlignLeft => LucideIcon::AlignHorizontalJustifyStart,
        PositionActionIcon::AlignHorizontalCenter => LucideIcon::AlignHorizontalJustifyCenter,
        PositionActionIcon::AlignRight => LucideIcon::AlignHorizontalJustifyEnd,
        PositionActionIcon::AlignTop => LucideIcon::AlignVerticalJustifyStart,
        PositionActionIcon::AlignVerticalCenter => LucideIcon::AlignVerticalJustifyCenter,
        PositionActionIcon::AlignBottom => LucideIcon::AlignVerticalJustifyEnd,
        PositionActionIcon::RotateClockwise90 => LucideIcon::RotateCw,
        PositionActionIcon::FlipHorizontal => LucideIcon::FoldHorizontal,
        PositionActionIcon::FlipVertical => LucideIcon::FoldVertical,
        PositionActionIcon::LockAspectRatio => LucideIcon::Ratio,
    };
    render_lucide_icon(icon, color, 16.)
}

fn render_position_action_button(
    panel_id: &SharedString,
    id_suffix: &'static str,
    icon: PositionActionIcon,
    event: PositionEvent,
    enabled: bool,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut button = div()
        .id(SharedString::from(format!("{panel_id}-{id_suffix}")))
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
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            let event = event.clone();
            events.send(event, cx);
        });
    }
    button
        .child(render_action_icon(icon, enabled, cx))
        .into_any_element()
}

fn render_text_action_button(
    panel_id: &SharedString,
    id_suffix: &'static str,
    label: &'static str,
    action: DesignPanelAction,
    enabled: bool,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    if !enabled {
        return div()
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
            .text_color(cx.theme().muted_foreground)
            .opacity(0.62)
            .child(label)
            .into_any_element();
    }
    let events = events.clone();
    div()
        .id(SharedString::from(format!("{panel_id}-{id_suffix}")))
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .h(px(ROW_HEIGHT))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().transparent)
        .bg(cx.theme().secondary)
        .cursor_pointer()
        .text_xs()
        .hover(|style| style.bg(cx.theme().accent))
        .focus(|style| {
            style
                .bg(cx.theme().accent)
                .border_color(cx.theme().selection)
        })
        .on_activate(move |_, _, cx| {
            let action = action.clone();
            events.send(action_event(action), cx);
        })
        .child(label)
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_grid_child_alignment_button(
    projection: &PositionProjection,
    id_suffix: &'static str,
    icon: PositionActionIcon,
    property: DesignPanelProperty,
    value: DesignGridItemAlignment,
    selected: bool,
    enabled: bool,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut button = div()
        .id(SharedString::from(format!(
            "{}-{id_suffix}",
            projection.panel_id
        )))
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
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            events.send(
                PositionEvent::Property(property, DesignPanelValue::GridItemAlignment(value)),
                cx,
            );
        });
    }
    button
        .child(render_action_icon(icon, enabled, cx))
        .into_any_element()
}

pub(in super::super) fn render_vector_edit_selection(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let view_data = projection.vector_edit.as_ref()?;
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
        let events = events.clone();
        let chip = div()
            .id(SharedString::from(format!(
                "{}-vector-vertex-{}",
                projection.panel_id, vertex.id
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
                    .on_activate(move |_, _, cx| {
                        events.send(
                            PositionEvent::SelectVertices(selection_for_action.clone()),
                            cx,
                        );
                    })
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
                .child(chrome.position_value_cell(
                    "vector-vertex-x",
                    "X",
                    format_vector_number_state(x),
                    DesignPanelProperty::VectorVertexX,
                    DesignPanelValue::Number(x_next),
                    cx,
                ))
                .child(chrome.position_value_cell(
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
                .child(chrome.position_value_cell(
                    "vector-vertex-corner-radius",
                    "R",
                    format_vector_number_state(corner_radius),
                    DesignPanelProperty::VectorVertexCornerRadius,
                    DesignPanelValue::Number(radius_next),
                    cx,
                ))
                .child(chrome.position_value_cell(
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

fn render_smart_selection_spacing_field(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    view_data: &DesignSmartSelectionViewData,
    axis: DesignSmartSelectionAxis,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let Some(spacing) = view_data.spacing(axis) else {
        return div().into_any_element();
    };
    let property = smart_selection_property(axis);
    let (value, next) = match spacing.value {
        DesignSmartSelectionSpacingValue::Mixed => ("Mixed".into(), 0.),
        DesignSmartSelectionSpacingValue::Uniform(value) => {
            (SharedString::from(format_number(value)), value + 1.)
        }
    };
    let tooltip = if !projection.can_edit {
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
            projection.panel_id, axis
        )))
        .flex_1()
        .min_w(px(0.))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(chrome.position_value_cell(
            match axis {
                DesignSmartSelectionAxis::Horizontal => "smart-selection-horizontal-spacing",
                DesignSmartSelectionAxis::Vertical => "smart-selection-vertical-spacing",
            },
            axis.short_label(),
            value,
            property,
            DesignPanelValue::Number(next),
            cx,
        ))
        .into_any_element()
}

fn render_smart_selection_operation_button(
    projection: &PositionProjection,
    view_data: &DesignSmartSelectionViewData,
    operation: DesignSmartSelectionOperation,
    availability: &DesignSmartSelectionAvailability,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let enabled =
        projection.can_edit && view_data.read_only_reason.is_none() && availability.is_available();
    let tooltip = if !projection.can_edit {
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
            ("smart-selection-distribute-horizontal", "H")
        }
        DesignSmartSelectionOperation::DistributeVertical => {
            ("smart-selection-distribute-vertical", "V")
        }
        DesignSmartSelectionOperation::TidyUp => ("smart-selection-tidy-up", "Tidy up"),
    };
    let selector = SharedString::from(format!("{}-{id_suffix}", projection.panel_id));
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
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            events.send(PositionEvent::SmartSelectionArrange(operation), cx);
        });
    }
    button.child(label).into_any_element()
}

fn render_smart_selection_controls(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    view_data: &DesignSmartSelectionViewData,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let mut controls = v_flex().gap_1();
    let mut spacing_row = h_flex().w_full().gap_2();
    let mut has_spacing = false;
    for axis in DesignSmartSelectionAxis::ALL {
        if view_data.spacing(axis).is_some() {
            has_spacing = true;
            spacing_row = spacing_row.child(render_smart_selection_spacing_field(
                projection, chrome, view_data, axis, cx,
            ));
        }
    }
    if has_spacing {
        controls = controls
            .child(chrome.position_group_label("Space between", cx))
            .child(spacing_row);
    }

    let mut operation_row = h_flex().w_full().gap_2();
    let mut has_operation = false;
    for operation in DesignSmartSelectionOperation::ALL {
        if let Some(availability) = view_data.operation_availability(operation) {
            has_operation = true;
            operation_row = operation_row.child(render_smart_selection_operation_button(
                projection,
                view_data,
                operation,
                availability,
                events,
                cx,
            ));
        }
    }
    if has_operation {
        controls = controls
            .child(chrome.position_group_label("Arrange", cx))
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

fn render_constraints_controls(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let horizontal = next_horizontal_constraint(projection.horizontal_constraint);
    let vertical = next_vertical_constraint(projection.vertical_constraint);
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
                .child(chrome.position_value_cell(
                    "horizontal-constraint",
                    "H",
                    projection.horizontal_constraint.label(),
                    DesignPanelProperty::HorizontalConstraint,
                    DesignPanelValue::Constraint(horizontal),
                    cx,
                ))
                .child(chrome.position_value_cell(
                    "vertical-constraint",
                    "V",
                    projection.vertical_constraint.label(),
                    DesignPanelProperty::VerticalConstraint,
                    DesignPanelValue::Constraint(vertical),
                    cx,
                )),
        )
        .child(diagram)
        .into_any_element()
}

pub(in super::super) fn render(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    events: &PositionEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let horizontal_alignment = if let Some(current) = projection.grid_horizontal_alignment {
        h_flex()
            .flex_1()
            .gap_0p5()
            .rounded(px(5.))
            .bg(cx.theme().secondary)
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-left",
                PositionActionIcon::AlignLeft,
                DesignPanelProperty::GridHorizontalAlignment,
                DesignGridItemAlignment::Start,
                current == DesignGridItemAlignment::Start,
                projection.grid_horizontal_editable,
                events,
                cx,
            ))
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-horizontal-center",
                PositionActionIcon::AlignHorizontalCenter,
                DesignPanelProperty::GridHorizontalAlignment,
                DesignGridItemAlignment::Center,
                current == DesignGridItemAlignment::Center,
                projection.grid_horizontal_editable,
                events,
                cx,
            ))
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-right",
                PositionActionIcon::AlignRight,
                DesignPanelProperty::GridHorizontalAlignment,
                DesignGridItemAlignment::End,
                current == DesignGridItemAlignment::End,
                projection.grid_horizontal_editable,
                events,
                cx,
            ))
            .into_any_element()
    } else {
        h_flex()
            .flex_1()
            .gap_0p5()
            .rounded(px(5.))
            .bg(cx.theme().secondary)
            .child(render_position_action_button(
                &projection.panel_id,
                "align-left",
                PositionActionIcon::AlignLeft,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignLeft,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .child(render_position_action_button(
                &projection.panel_id,
                "align-horizontal-center",
                PositionActionIcon::AlignHorizontalCenter,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignHorizontalCenter,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .child(render_position_action_button(
                &projection.panel_id,
                "align-right",
                PositionActionIcon::AlignRight,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignRight,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .into_any_element()
    };
    let vertical_alignment = if let Some(current) = projection.grid_vertical_alignment {
        h_flex()
            .flex_1()
            .gap_0p5()
            .rounded(px(5.))
            .bg(cx.theme().secondary)
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-top",
                PositionActionIcon::AlignTop,
                DesignPanelProperty::GridVerticalAlignment,
                DesignGridItemAlignment::Start,
                current == DesignGridItemAlignment::Start,
                projection.grid_vertical_editable,
                events,
                cx,
            ))
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-vertical-center",
                PositionActionIcon::AlignVerticalCenter,
                DesignPanelProperty::GridVerticalAlignment,
                DesignGridItemAlignment::Center,
                current == DesignGridItemAlignment::Center,
                projection.grid_vertical_editable,
                events,
                cx,
            ))
            .child(render_grid_child_alignment_button(
                projection,
                "grid-child-align-bottom",
                PositionActionIcon::AlignBottom,
                DesignPanelProperty::GridVerticalAlignment,
                DesignGridItemAlignment::End,
                current == DesignGridItemAlignment::End,
                projection.grid_vertical_editable,
                events,
                cx,
            ))
            .into_any_element()
    } else {
        h_flex()
            .flex_1()
            .gap_0p5()
            .rounded(px(5.))
            .bg(cx.theme().secondary)
            .child(render_position_action_button(
                &projection.panel_id,
                "align-top",
                PositionActionIcon::AlignTop,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignTop,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .child(render_position_action_button(
                &projection.panel_id,
                "align-vertical-center",
                PositionActionIcon::AlignVerticalCenter,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignVerticalCenter,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .child(render_position_action_button(
                &projection.panel_id,
                "align-bottom",
                PositionActionIcon::AlignBottom,
                action_event(DesignPanelAction::ArrangeRequested {
                    target: projection.target.clone(),
                    operation: DesignArrangeOperation::AlignBottom,
                }),
                projection.can_arrange,
                events,
                cx,
            ))
            .into_any_element()
    };

    let disclosure_events = events.clone();
    let can_constrain = projection.can_constrain;
    let disclosure = div()
        .id(SharedString::from(format!(
            "{}-constraints-disclosure",
            projection.panel_id
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
        .on_activate(move |_, _, cx| {
            if can_constrain {
                disclosure_events.send(PositionEvent::ToggleConstraints, cx);
            }
        })
        .when(projection.constraints_expanded, |button| {
            button
                .bg(cx.theme().selection.opacity(0.22))
                .text_color(cx.theme().selection)
        })
        .child(Icon::new(IconName::Inspector).xsmall());

    let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_1();
    if let Some(vector_edit) = render_vector_edit_selection(projection, chrome, events, cx) {
        content = content
            .child(vector_edit)
            .child(div().h(px(6.)).flex_none());
    }
    if projection.supports_arrange || projection.grid_horizontal_alignment.is_some() {
        content = content
            .child(chrome.position_group_label("Alignment", cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(horizontal_alignment)
                    .child(vertical_alignment)
                    .child(div().size(px(24.)).flex_none()),
            );
    }
    if projection.supports_position_coordinates {
        content = content
            .child(chrome.position_group_label("Position", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.position_value_cell(
                        "x",
                        "X",
                        format_number(projection.x),
                        DesignPanelProperty::X,
                        DesignPanelValue::Number(projection.x + 1.),
                        cx,
                    ))
                    .child(chrome.position_value_cell(
                        "y",
                        "Y",
                        format_number(projection.y),
                        DesignPanelProperty::Y,
                        DesignPanelValue::Number(projection.y + 1.),
                        cx,
                    ))
                    .when(can_constrain, |row| row.child(disclosure))
                    .when(!can_constrain, |row| {
                        row.child(div().size(px(24.)).flex_none())
                    }),
            );
    } else if can_constrain {
        content = content
            .child(chrome.position_group_label("Constraints", cx))
            .child(h_flex().justify_end().child(disclosure));
    }
    if can_constrain && projection.constraints_expanded {
        content = content
            .child(chrome.position_group_label("Constraints", cx))
            .child(render_constraints_controls(projection, chrome, cx));
    }
    if projection.supports_transforms {
        content = content
            .child(chrome.position_group_label("Rotation", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(chrome.position_value_cell(
                        "rotation",
                        "∟",
                        format!(
                            "{}°",
                            format_number(normalize_rotation(projection.rotation))
                        ),
                        DesignPanelProperty::Rotation,
                        DesignPanelValue::Number((projection.rotation + 15.) % 360.),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .h(px(ROW_HEIGHT))
                            .flex_1()
                            .overflow_hidden()
                            .rounded(px(4.))
                            .bg(cx.theme().secondary)
                            .child(render_position_action_button(
                                &projection.panel_id,
                                "rotate-clockwise-90",
                                PositionActionIcon::RotateClockwise90,
                                action_event(DesignPanelAction::TransformRequested {
                                    target: projection.target.clone(),
                                    operation: DesignTransformOperation::RotateClockwise90,
                                }),
                                projection.can_edit,
                                events,
                                cx,
                            ))
                            .child(render_position_action_button(
                                &projection.panel_id,
                                "flip-horizontal",
                                PositionActionIcon::FlipHorizontal,
                                action_event(DesignPanelAction::TransformRequested {
                                    target: projection.target.clone(),
                                    operation: DesignTransformOperation::FlipHorizontal,
                                }),
                                projection.can_edit,
                                events,
                                cx,
                            ))
                            .child(render_position_action_button(
                                &projection.panel_id,
                                "flip-vertical",
                                PositionActionIcon::FlipVertical,
                                action_event(DesignPanelAction::TransformRequested {
                                    target: projection.target.clone(),
                                    operation: DesignTransformOperation::FlipVertical,
                                }),
                                projection.can_edit,
                                events,
                                cx,
                            )),
                    )
                    .child(div().size(px(24.)).flex_none()),
            );
    }
    if projection.supports_arrange && projection.selection_is_multiple {
        if let Some(view_data) = projection.smart_selection.as_ref() {
            content = content.child(render_smart_selection_controls(
                projection, chrome, view_data, events, cx,
            ));
        } else if !projection.smart_selection_projection_present {
            content = content.child(
                h_flex()
                    .gap_2()
                    .child(render_text_action_button(
                        &projection.panel_id,
                        "distribute-horizontal",
                        "Distribute ↔",
                        DesignPanelAction::ArrangeRequested {
                            target: projection.target.clone(),
                            operation: DesignArrangeOperation::DistributeHorizontal,
                        },
                        projection.can_edit,
                        events,
                        cx,
                    ))
                    .child(render_text_action_button(
                        &projection.panel_id,
                        "distribute-vertical",
                        "Distribute ↕",
                        DesignPanelAction::ArrangeRequested {
                            target: projection.target.clone(),
                            operation: DesignArrangeOperation::DistributeVertical,
                        },
                        projection.can_edit,
                        events,
                        cx,
                    ))
                    .child(render_text_action_button(
                        &projection.panel_id,
                        "tidy-selection",
                        "Tidy",
                        DesignPanelAction::ArrangeRequested {
                            target: projection.target.clone(),
                            operation: DesignArrangeOperation::TidyUp,
                        },
                        projection.can_edit,
                        events,
                        cx,
                    )),
            );
        }
    }
    if projection.renders_draw_workspace {
        let id = SharedString::from(format!("{}-draw-position-content", projection.panel_id));
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
        chrome.position_section(DesignPanelSection::Position, content.into_any_element(), cx)
    }
}

pub(in super::super) fn render_constraints(
    projection: &PositionProjection,
    chrome: &impl PositionInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    chrome.position_section(
        DesignPanelSection::Constraints,
        render_constraints_controls(projection, chrome, cx),
        cx,
    )
}

/// Temporary compatibility surface for Position-adjacent helpers that are
/// also consumed by the Layout and controlled-property controllers.
///
/// The Position renderer itself is standalone below; this trait keeps those
/// internal call sites stable without adding another inherent facade impl.
#[allow(dead_code)]
pub(in super::super) trait PositionPanelCompat: Sized {
    fn can_show_constraints(&self) -> bool;
    fn smart_selection_axis(property: DesignPanelProperty) -> Option<DesignSmartSelectionAxis>;
    fn smart_selection_property(axis: DesignSmartSelectionAxis) -> DesignPanelProperty;
    fn smart_selection_view_data_for_context(&self) -> Option<&DesignSmartSelectionViewData>;
    fn smart_selection_spacing_is_editable(&self, axis: DesignSmartSelectionAxis) -> bool;
    fn smart_selection_property_is_mixed(&self, property: DesignPanelProperty) -> bool;
    fn emit_smart_selection_arrange(
        &mut self,
        operation: DesignSmartSelectionOperation,
        cx: &mut Context<Self>,
    );
    fn emit_smart_selection_spacing_edit(
        &mut self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool;
    fn render_vector_edit_selection(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
    fn render_smart_selection_spacing_field(
        &self,
        view_data: &DesignSmartSelectionViewData,
        axis: DesignSmartSelectionAxis,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_smart_selection_operation_button(
        &self,
        view_data: &DesignSmartSelectionViewData,
        operation: DesignSmartSelectionOperation,
        availability: &DesignSmartSelectionAvailability,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_smart_selection_controls(
        &self,
        view_data: &DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_position(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_constraints_controls(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_constraints(&self, cx: &mut Context<Self>) -> AnyElement;
    fn apply_alignment_grid_shortcut(
        &mut self,
        shortcut: AlignmentGridShortcut,
        cx: &mut Context<Self>,
    ) -> bool;
    fn handle_alignment_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn render_alignment_grid(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_position_action_icon(
        &self,
        icon: PositionActionIcon,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_position_action_button(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_position_action_button_with_enabled(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        action: DesignPanelAction,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_grid_child_alignment_button(
        &self,
        id_suffix: &'static str,
        icon: PositionActionIcon,
        property: DesignPanelProperty,
        value: DesignGridItemAlignment,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

impl PositionPanelCompat for DesignPanel {
    fn can_show_constraints(&self) -> bool {
        can_show_constraints(self)
    }

    fn smart_selection_axis(property: DesignPanelProperty) -> Option<DesignSmartSelectionAxis> {
        smart_selection_axis(property)
    }

    fn smart_selection_property(axis: DesignSmartSelectionAxis) -> DesignPanelProperty {
        smart_selection_property(axis)
    }

    fn smart_selection_view_data_for_context(&self) -> Option<&DesignSmartSelectionViewData> {
        smart_selection_view_data_for_context(self)
    }

    fn smart_selection_spacing_is_editable(&self, axis: DesignSmartSelectionAxis) -> bool {
        smart_selection_spacing_is_editable(self, axis)
    }

    fn smart_selection_property_is_mixed(&self, property: DesignPanelProperty) -> bool {
        smart_selection_property_is_mixed(self, property)
    }

    fn emit_smart_selection_arrange(
        &mut self,
        operation: DesignSmartSelectionOperation,
        cx: &mut Context<Self>,
    ) {
        emit_smart_selection_arrange(self, operation, cx);
    }

    fn emit_smart_selection_spacing_edit(
        &mut self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        emit_smart_selection_spacing_edit(self, property, value, phase, cx)
    }

    fn render_vector_edit_selection(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = super::super::position::projection(self);
        let events = PositionEventSink::new(cx.entity());
        render_vector_edit_selection(&projection, self, &events, cx)
    }

    fn render_smart_selection_spacing_field(
        &self,
        view_data: &DesignSmartSelectionViewData,
        axis: DesignSmartSelectionAxis,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let projection = super::super::position::projection(self);
        render_smart_selection_spacing_field(&projection, self, view_data, axis, cx)
    }

    fn render_smart_selection_operation_button(
        &self,
        view_data: &DesignSmartSelectionViewData,
        operation: DesignSmartSelectionOperation,
        availability: &DesignSmartSelectionAvailability,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let projection = super::super::position::projection(self);
        let events = PositionEventSink::new(cx.entity());
        render_smart_selection_operation_button(
            &projection,
            view_data,
            operation,
            availability,
            &events,
            cx,
        )
    }

    fn render_smart_selection_controls(
        &self,
        view_data: &DesignSmartSelectionViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let projection = super::super::position::projection(self);
        let events = PositionEventSink::new(cx.entity());
        render_smart_selection_controls(&projection, self, view_data, &events, cx)
    }

    fn render_position(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::position::projection(self);
        let events = PositionEventSink::new(cx.entity());
        render(&projection, self, &events, cx)
    }

    fn render_constraints_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::position::projection(self);
        render_constraints_controls(&projection, self, cx)
    }

    fn render_constraints(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = super::super::position::projection(self);
        render_constraints(&projection, self, cx)
    }
    fn apply_alignment_grid_shortcut(
        &mut self,
        shortcut: AlignmentGridShortcut,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(layout) = self.host.inspected_node().layout.clone() else {
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

    fn handle_alignment_grid_key_down(
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

    fn render_alignment_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let layout = self
            .host
            .inspected_node()
            .layout
            .clone()
            .unwrap_or_default();
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

    fn render_position_action_icon(
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
        let icon = match icon {
            PositionActionIcon::AlignLeft => LucideIcon::AlignHorizontalJustifyStart,
            PositionActionIcon::AlignHorizontalCenter => LucideIcon::AlignHorizontalJustifyCenter,
            PositionActionIcon::AlignRight => LucideIcon::AlignHorizontalJustifyEnd,
            PositionActionIcon::AlignTop => LucideIcon::AlignVerticalJustifyStart,
            PositionActionIcon::AlignVerticalCenter => LucideIcon::AlignVerticalJustifyCenter,
            PositionActionIcon::AlignBottom => LucideIcon::AlignVerticalJustifyEnd,
            PositionActionIcon::RotateClockwise90 => LucideIcon::RotateCw,
            PositionActionIcon::FlipHorizontal => LucideIcon::FoldHorizontal,
            PositionActionIcon::FlipVertical => LucideIcon::FoldVertical,
            PositionActionIcon::LockAspectRatio => LucideIcon::Ratio,
        };
        render_lucide_icon(icon, color, 16.)
    }

    fn render_position_action_button(
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

    fn render_position_action_button_with_enabled(
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

    fn render_grid_child_alignment_button(
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

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use crate::design::{
        DesignPanelAutoLayoutWrap, DesignPanelNodeCapabilities, DesignSmartSelectionSpacingViewData,
    };
    use crate::test_support::{Mounted, ProbeHost, mount_component};

    use super::*;
    // The renderer's gates are only reachable through the projection the facade
    // derives from live host state, so every test below drives that factory
    // rather than hand-building a `PositionProjection`.
    use super::super::super::position::projection as position_projection;

    type Host = Entity<ProbeHost<DesignPanel, DesignPanelAction>>;

    /// The constraint cells are option cells, and an editable option cell is a
    /// retained `Select` whose only debug selector is the one the labelled
    /// presentation carries (`options::render_option_cell`). Every test that
    /// looks at the painted constraint block therefore turns the inspector's
    /// Additional labels preference on first and reads these selectors, which
    /// name the same cells in the read-only presentation too.
    const HORIZONTAL_CONSTRAINT_CELL: &str = "design-horizontal-constraint-additional-label";
    const VERTICAL_CONSTRAINT_CELL: &str = "design-vertical-constraint-additional-label";

    /// The panel facade's own harness (`panel::tests::setup`) is private to
    /// that module, so Position mounts the real `DesignPanel` through the
    /// shared crate probe host instead. Same contract: one real panel in a
    /// rooted window, rendered at least once before the test looks at it.
    fn setup(
        node: DesignPanelNode,
        cx: &mut TestAppContext,
    ) -> Mounted<'_, DesignPanel, DesignPanelAction> {
        let (host, actions, visual_cx) = mount_component(cx, move |window, cx| {
            DesignPanel::new("design", node, window, cx)
        });
        visual_cx.run_until_parked();
        (host, actions, visual_cx)
    }

    fn panel(host: &Host, cx: &VisualTestContext) -> Entity<DesignPanel> {
        cx.read(|app| host.read(app).component.clone())
    }

    fn grid_parent(participation: DesignPanelAutoLayoutParticipation) -> DesignPanelParentLayout {
        DesignPanelParentLayout::auto_layout(
            DesignPanelAutoLayoutDirection::Grid,
            DesignPanelAutoLayoutWrap::NoWrap,
            participation,
        )
    }

    fn inspect(
        panel: &mut DesignPanel,
        node: DesignPanelNode,
        parent_layout: DesignPanelParentLayout,
        permissions: DesignPanelPermissions,
        cx: &mut Context<DesignPanel>,
    ) {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(node, parent_layout, permissions),
            cx,
        );
    }

    #[gpui::test]
    fn constraints_hidden_without_constrain_capability(cx: &mut TestAppContext) {
        let node = DesignPanelNode::new("rect", "Rectangle", DesignPanelNodeKind::Rectangle);
        let (host, _actions, visual_cx) = setup(node.clone(), cx);
        let panel = panel(&host, visual_cx);
        // Makes the constraint cells observable at all; see the selector
        // constants above.
        panel.update(visual_cx, |panel, cx| panel.set_additional_labels(true, cx));
        visual_cx.run_until_parked();

        // Positive control first: everything below asserts an absence, which
        // proves nothing unless this exact surface really paints when the
        // capability and the disclosure both say yes.
        assert!(
            visual_cx.debug_bounds(HORIZONTAL_CONSTRAINT_CELL).is_some(),
            "a freeform child the host can constrain paints the horizontal constraint cell",
        );
        assert!(
            visual_cx.debug_bounds(VERTICAL_CONSTRAINT_CELL).is_some(),
            "a freeform child the host can constrain paints the vertical constraint cell",
        );

        panel.update(visual_cx, |panel, cx| {
            assert!(
                can_show_constraints(panel),
                "a freeform rectangle carries the host's constrain capability",
            );
            let projection = position_projection(panel);
            assert!(projection.can_constrain && projection.constraints_expanded);
            dispatch(panel, PositionEvent::ToggleConstraints, cx);
            assert!(
                !panel.sections.constraints_expanded(),
                "the disclosure a capable node owns is togglable",
            );
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx.debug_bounds(HORIZONTAL_CONSTRAINT_CELL).is_none(),
            "the collapsed disclosure takes the constraint controls with it",
        );

        panel.update(visual_cx, |panel, cx| {
            dispatch(panel, PositionEvent::ToggleConstraints, cx);
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx.debug_bounds(HORIZONTAL_CONSTRAINT_CELL).is_some(),
            "re-opening the disclosure restores the same controls",
        );

        let unconstrained = DesignPanelNode::new(
            "unconstrained",
            "Unconstrained",
            DesignPanelNodeKind::Rectangle,
        )
        .with_capabilities(
            DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Rectangle)
                .with_constraints(false),
        );
        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                unconstrained,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
                cx,
            );
            assert!(
                !can_show_constraints(panel),
                "a node the host refuses constraints for never reports the constrain capability",
            );
            assert!(!position_projection(panel).can_constrain);
            assert!(
                panel.sections.constraints_expanded(),
                "the retained disclosure state is presentation only, so it stays expanded while \
                 the capability alone hides the surface",
            );
            dispatch(panel, PositionEvent::ToggleConstraints, cx);
            assert!(
                panel.sections.constraints_expanded(),
                "a node without the capability cannot toggle a disclosure it does not own",
            );
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx.debug_bounds(HORIZONTAL_CONSTRAINT_CELL).is_none(),
            "an expanded disclosure never resurrects constraints the host withheld",
        );

        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                node.clone(),
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Vertical,
                    DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
                cx,
            );
            assert!(
                !can_show_constraints(panel),
                "an in-flow auto-layout child is positioned by its parent, so the same capable \
                 node reports no constrain capability there",
            );
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx.debug_bounds(HORIZONTAL_CONSTRAINT_CELL).is_none(),
            "the parent layout hides the constraint controls without any capability change",
        );

        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
                cx,
            );
            // A read-only inspection is projected by the Viewer band rather
            // than this renderer, so the guarantee here is the projection's:
            // the constrain answer is structural and never an alias of the
            // write permission.
            let projection = position_projection(panel);
            assert!(!projection.can_edit);
            assert!(
                projection.can_constrain,
                "constraints stay reachable for a read-only inspection",
            );
        });
    }

    #[gpui::test]
    fn grid_alignment_editable_only_when_access_and_value_present(cx: &mut TestAppContext) {
        let mut node =
            DesignPanelNode::new("grid-child", "Grid child", DesignPanelNodeKind::Rectangle);
        let layout = node
            .layout
            .as_mut()
            .expect("an auto-layout child carries grid item data");
        layout.item.grid_horizontal_alignment = DesignGridItemAlignment::Center;
        layout.item.grid_vertical_alignment = DesignGridItemAlignment::End;
        let (host, actions, visual_cx) = setup(node.clone(), cx);
        let panel = panel(&host, visual_cx);

        panel.update(visual_cx, |panel, _| {
            let projection = position_projection(panel);
            assert_eq!(
                projection.grid_horizontal_alignment, None,
                "a freeform child is not a grid child, so it keeps the arrange buttons even \
                 though the node still carries grid item data",
            );
            assert_eq!(projection.grid_vertical_alignment, None);
            assert!(
                !projection.grid_horizontal_editable && !projection.grid_vertical_editable,
                "the access answer follows the same applicability rule as the value: an editor's \
                 write permission alone never grants grid alignment access to a child that no \
                 grid parent positions",
            );
        });

        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                node.clone(),
                grid_parent(DesignPanelAutoLayoutParticipation::InFlow),
                DesignPanelPermissions::editor(),
                cx,
            );
            let projection = position_projection(panel);
            assert_eq!(
                projection.grid_horizontal_alignment,
                Some(DesignGridItemAlignment::Center),
                "an in-flow child of a grid parent reads its own alignment out of the node",
            );
            assert_eq!(
                projection.grid_vertical_alignment,
                Some(DesignGridItemAlignment::End),
                "each axis keeps its own alignment value",
            );
            assert!(projection.grid_horizontal_editable && projection.grid_vertical_editable);
            dispatch(
                panel,
                PositionEvent::Property(
                    DesignPanelProperty::GridHorizontalAlignment,
                    DesignPanelValue::GridItemAlignment(DesignGridItemAlignment::Start),
                ),
                cx,
            );
            assert_eq!(
                position_projection(panel).grid_horizontal_alignment,
                Some(DesignGridItemAlignment::Center),
                "the control is fully host controlled: the request never edits the projection",
            );
        });
        visual_cx.run_until_parked();
        assert_eq!(
            actions.borrow().as_slice(),
            [DesignPanelAction::PropertyChangeRequested {
                node_id: "grid-child".into(),
                property: DesignPanelProperty::GridHorizontalAlignment,
                value: DesignPanelValue::GridItemAlignment(DesignGridItemAlignment::Start),
            }],
            "an editable grid alignment control emits exactly one exact-identity intent",
        );
        actions.borrow_mut().clear();

        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                node.clone(),
                grid_parent(DesignPanelAutoLayoutParticipation::Ignored),
                DesignPanelPermissions::editor(),
                cx,
            );
            assert_eq!(
                position_projection(panel).grid_horizontal_alignment,
                None,
                "a child excluded from its parent's flow is not a grid item, so the alignment \
                 value disappears with the participation and not with the value on the node",
            );
            inspect(
                panel,
                node.clone(),
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Horizontal,
                    DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
                cx,
            );
            assert_eq!(
                position_projection(panel).grid_horizontal_alignment,
                None,
                "only a grid parent produces grid alignment: a stacked auto-layout child keeps \
                 the arrange buttons",
            );
        });

        panel.update(visual_cx, |panel, cx| {
            inspect(
                panel,
                node.clone(),
                grid_parent(DesignPanelAutoLayoutParticipation::InFlow),
                DesignPanelPermissions::viewer(),
                cx,
            );
            let projection = position_projection(panel);
            assert_eq!(
                projection.grid_horizontal_alignment,
                Some(DesignGridItemAlignment::Center),
                "the locked control still reads out the current alignment",
            );
            assert!(
                !projection.grid_horizontal_editable && !projection.grid_vertical_editable,
                "enablement follows the per-property access answer, which a read-only inspection \
                 withholds on both axes",
            );
            dispatch(
                panel,
                PositionEvent::Property(
                    DesignPanelProperty::GridHorizontalAlignment,
                    DesignPanelValue::GridItemAlignment(DesignGridItemAlignment::Start),
                ),
                cx,
            );
        });
        visual_cx.run_until_parked();
        assert!(
            actions.borrow().is_empty(),
            "a present alignment value alone never authorises an edit: the terminal dispatch \
             re-checks the access answer the disabled control reported",
        );
    }

    #[gpui::test]
    fn multiple_selection_hides_smart_selection_when_projection_absent(cx: &mut TestAppContext) {
        let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
        let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
        let selection_target = DesignPanelTarget::Nodes {
            node_ids: vec!["first".into(), "second".into()],
        };
        let other_target = DesignPanelTarget::Nodes {
            node_ids: vec!["first".into(), "third".into()],
        };
        let view_data = |target: DesignPanelTarget| {
            DesignSmartSelectionViewData::horizontal(
                target,
                DesignSmartSelectionSpacingViewData::uniform(12.),
            )
            .with_operation(
                DesignSmartSelectionOperation::DistributeHorizontal,
                DesignSmartSelectionAvailability::Available,
            )
        };
        let (host, _actions, visual_cx) = setup(first.clone(), cx);
        let panel = panel(&host, visual_cx);

        panel.update(visual_cx, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::multiple(
                    DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                    DesignPanelParentLayout::Mixed,
                    DesignPanelPermissions::editor(),
                ),
                cx,
            );
            let projection = position_projection(panel);
            assert!(
                projection.selection_is_multiple && projection.supports_arrange,
                "this multiple selection supports arrange, so only the host projection can \
                 decide which arrange surface it gets",
            );
            assert!(
                !projection.smart_selection_projection_present
                    && projection.smart_selection.is_none(),
                "a host that publishes no smart-selection projection keeps the legacy arrange \
                 buttons",
            );
            panel.set_smart_selection_view_data(view_data(selection_target.clone()), cx);
            assert!(position_projection(panel).smart_selection.is_some());
        });
        visual_cx.run_until_parked();
        // Positive control: the two absences asserted below are only evidence
        // because this exact selection paints both leaves here.
        assert!(
            visual_cx
                .debug_bounds("design-smart-selection-horizontal-spacing")
                .is_some(),
            "a matching host projection reaches the renderer and paints Space between",
        );
        assert!(
            visual_cx
                .debug_bounds("design-smart-selection-distribute-horizontal")
                .is_some(),
            "a matching host projection paints the Arrange operations it published",
        );

        panel.update(visual_cx, |panel, cx| {
            panel.set_smart_selection_view_data(view_data(other_target.clone()), cx);
            let projection = position_projection(panel);
            assert!(
                projection.smart_selection_projection_present,
                "the presence flag records that the host owns smart selection independently of \
                 whether it answered for this selection, and that is what suppresses the legacy \
                 fallback",
            );
            assert!(
                projection.smart_selection.is_none(),
                "a projection that answers for a different selection never reaches the renderer",
            );
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx
                .debug_bounds("design-smart-selection-horizontal-spacing")
                .is_none(),
            "Space between belongs to the exact selection the host answered for",
        );
        assert!(
            visual_cx
                .debug_bounds("design-smart-selection-distribute-horizontal")
                .is_none(),
            "Arrange belongs to the exact selection the host answered for",
        );

        panel.update(visual_cx, |panel, cx| {
            panel.set_smart_selection_view_data(view_data(selection_target.clone()), cx);
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    first.clone(),
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                ),
                cx,
            );
            let projection = position_projection(panel);
            assert!(!projection.selection_is_multiple);
            assert!(
                projection.smart_selection.is_none(),
                "smart-selection view data never implies a multiple selection",
            );
        });
        visual_cx.run_until_parked();
        assert!(
            visual_cx
                .debug_bounds("design-smart-selection-horizontal-spacing")
                .is_none(),
            "a single selection hides the smart-selection block even when the host left view \
             data behind",
        );
    }
}

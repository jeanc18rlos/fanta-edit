//! Projection-driven field chrome for the Design inspector.
//!
//! The Design facade remains the GPUI entity and host-facing action source,
//! but field rendering does not need access to that complete entity. Each
//! compatibility entry point below first captures an owned projection, then
//! renders it with an explicit event sink. Callbacks return to the live entity
//! so access, capabilities, and targets are revalidated before an intent is
//! emitted.

use super::*;

fn common_field_layout() -> crate::molecules::InspectorGridLayout {
    crate::molecules::InspectorGridLayout::compact(InspectorMetrics::current())
}

#[derive(Clone)]
pub(super) struct DesignInspectorEventSink {
    panel: Entity<DesignPanel>,
}

impl DesignInspectorEventSink {
    fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch(&self, event: DesignInspectorFieldEvent, window: &mut Window, cx: &mut App) {
        self.panel.update(cx, |panel, cx| match event {
            DesignInspectorFieldEvent::ActivateProperty {
                origin,
                property,
                fallback,
            } => panel.activate_property_from_control(origin, property, fallback, window, cx),
            DesignInspectorFieldEvent::CopyProperty { property, display } => {
                panel.emit_property_copy(property, display, cx)
            }
            DesignInspectorFieldEvent::CommitBoolean { property, value } => {
                panel.emit_property(property, DesignPanelValue::Bool(value), cx);
            }
        });
    }
}

#[derive(Clone)]
enum DesignInspectorFieldEvent {
    ActivateProperty {
        origin: EditorFocusOrigin,
        property: DesignPanelProperty,
        fallback: DesignPanelValue,
    },
    CopyProperty {
        property: DesignPanelProperty,
        display: SharedString,
    },
    CommitBoolean {
        property: DesignPanelProperty,
        value: bool,
    },
}

struct DesignCompactPropertyLabelProjection {
    selector: SharedString,
    label: SharedString,
}

impl DesignCompactPropertyLabelProjection {
    fn for_property(
        panel_id: &SharedString,
        id_suffix: &SharedString,
        property: DesignPanelProperty,
    ) -> Self {
        Self {
            selector: SharedString::from(format!("{panel_id}-{id_suffix}-additional-label")),
            label: DesignPanel::compact_property_label(property).into(),
        }
    }
}

struct DesignStandardValueCellProjection {
    panel_id: SharedString,
    id_suffix: SharedString,
    prefix: SharedString,
    display: SharedString,
    property: DesignPanelProperty,
    fallback: DesignPanelValue,
    field: crate::molecules::InspectorFieldFrame<DesignPanelValue>,
    editable: bool,
    copyable: bool,
    editing: bool,
    scrubbing: bool,
    scrub_enabled: bool,
    invalid: bool,
    left_padding: f32,
    additional_label: Option<DesignCompactPropertyLabelProjection>,
    retained_focus: Option<FocusHandle>,
    property_input: Entity<InputState>,
    variable_button: Option<AnyElement>,
}

enum DesignValueCellProjection {
    /// Option controls retain their existing `SelectState` controller during
    /// this slice. The resulting element is still owned by the projection,
    /// keeping the standalone renderer independent from the facade.
    RetainedOption(AnyElement),
    Standard(Box<DesignStandardValueCellProjection>),
}

struct DesignValueCellRequest {
    id_suffix: SharedString,
    prefix: &'static str,
    value: SharedString,
    property: DesignPanelProperty,
    fallback: DesignPanelValue,
    left_padding: f32,
}

struct DesignIconValueCellProjection {
    panel_id: SharedString,
    id_suffix: &'static str,
    icon: ValueFieldIcon,
    editable: bool,
    additional_labels: bool,
    value: DesignValueCellProjection,
}

struct DesignToggleRowProjection {
    panel_id: SharedString,
    id_suffix: SharedString,
    label: &'static str,
    checked: bool,
    property: DesignPanelProperty,
    field: crate::molecules::InspectorToggleField,
    variable_button: Option<AnyElement>,
}

struct DesignCheckboxRowProjection {
    panel_id: SharedString,
    id_suffix: SharedString,
    label: &'static str,
    checked: bool,
    property: DesignPanelProperty,
    field: crate::molecules::InspectorCheckboxField,
}

pub(super) fn inspector_property_value(
    panel: &DesignPanel,
    property: DesignPanelProperty,
) -> crate::molecules::InspectorValue<DesignPanelValue> {
    match panel.host.property_states.get(&property) {
        Some(state) if state.is_mixed() => crate::molecules::InspectorValue::Mixed,
        Some(state) if state.is_unset() => crate::molecules::InspectorValue::Unset,
        Some(state) => state.resolved().cloned().map_or(
            crate::molecules::InspectorValue::Unset,
            crate::molecules::InspectorValue::Uniform,
        ),
        None if panel.smart_selection_property_is_mixed(property) => {
            crate::molecules::InspectorValue::Mixed
        }
        None => panel.current_property_value(property).map_or(
            crate::molecules::InspectorValue::Unset,
            crate::molecules::InspectorValue::Uniform,
        ),
    }
}

pub(super) fn inspector_property_presentation(
    panel: &DesignPanel,
    property: DesignPanelProperty,
    copyable: bool,
    invalid: bool,
) -> crate::molecules::InspectorFieldPresentation {
    let controlled_state = panel.host.property_states.get(&property);
    let access = if panel.property_is_editable(property) {
        crate::molecules::InspectorFieldAccess::Editable
    } else if copyable {
        crate::molecules::InspectorFieldAccess::read_only(
            controlled_state
                .and_then(DesignPanelPropertyValueState::read_only_reason)
                .map(SharedString::from),
        )
    } else {
        crate::molecules::InspectorFieldAccess::disabled(
            controlled_state
                .and_then(DesignPanelPropertyValueState::read_only_reason)
                .map(SharedString::from),
        )
    };
    crate::molecules::InspectorFieldPresentation::new(access)
        .bound(controlled_state.is_some_and(|state| state.binding().is_some()))
        .invalid(invalid)
}

fn inspector_boolean_value(
    panel: &DesignPanel,
    property: DesignPanelProperty,
    fallback: bool,
) -> crate::molecules::InspectorValue<bool> {
    match inspector_property_value(panel, property) {
        crate::molecules::InspectorValue::Uniform(DesignPanelValue::Bool(value)) => {
            crate::molecules::InspectorValue::Uniform(value)
        }
        crate::molecules::InspectorValue::Mixed => crate::molecules::InspectorValue::Mixed,
        crate::molecules::InspectorValue::Unset => crate::molecules::InspectorValue::Unset,
        crate::molecules::InspectorValue::Uniform(_) => {
            crate::molecules::InspectorValue::Uniform(fallback)
        }
    }
}

fn project_value_cell(
    panel: &DesignPanel,
    request: DesignValueCellRequest,
    cx: &mut Context<DesignPanel>,
) -> DesignValueCellProjection {
    let DesignValueCellRequest {
        id_suffix,
        prefix,
        value,
        property,
        fallback,
        left_padding,
    } = request;
    let value = panel.display_property_value(property, value);
    let editable = panel.property_is_editable(property);
    if editable && let Some(options) = panel.property_options(property) {
        return DesignValueCellProjection::RetainedOption(
            panel.render_option_cell(id_suffix, prefix, value, property, options, cx),
        );
    }

    let copyable = panel.generic_property_is_copyable(property);
    let editing = editable
        && panel
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == property);
    let scrubbing = panel.numeric_scrub_is_active(property);
    let retained_focus = panel
        .edit
        .focus_return
        .as_ref()
        .filter(|return_focus| {
            editable
                && matches!(
                    return_focus.origin,
                    EditorFocusOrigin::ValueCell(origin_property)
                        if origin_property == property
                )
        })
        .map(|return_focus| return_focus.handle.clone());
    let invalid = editing && panel.edit.property_invalid;
    let additional_label = panel.preferences.additional_labels.then(|| {
        DesignCompactPropertyLabelProjection::for_property(&panel.id, &id_suffix, property)
    });

    DesignValueCellProjection::Standard(Box::new(DesignStandardValueCellProjection {
        panel_id: panel.id.clone(),
        id_suffix,
        prefix: prefix.into(),
        display: value,
        property,
        fallback,
        field: crate::molecules::InspectorFieldFrame::new(
            inspector_property_value(panel, property),
            inspector_property_presentation(panel, property, copyable, invalid),
        ),
        editable,
        copyable,
        editing,
        scrubbing,
        scrub_enabled: panel.numeric_scrub_surface_is_enabled(property),
        invalid,
        left_padding,
        additional_label,
        retained_focus,
        property_input: panel.retained.inputs.property.clone(),
        variable_button: panel.render_property_variable_button(property, cx),
    }))
}

fn render_compact_property_label(
    projection: &DesignCompactPropertyLabelProjection,
    cx: &App,
) -> AnyElement {
    let debug_selector = projection.selector.to_string();
    div()
        .id(projection.selector.clone())
        .debug_selector(move || debug_selector.clone())
        .min_w(px(0.))
        .max_w(px(64.))
        .overflow_hidden()
        .whitespace_nowrap()
        .truncate()
        .text_size(px(tokens::TypeScale::MICRO))
        .text_color(cx.theme().muted_foreground)
        .child(projection.label.clone())
        .into_any_element()
}

fn render_value_cell_projection(
    projection: DesignValueCellProjection,
    sink: DesignInspectorEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let layout = common_field_layout();
    let metrics = layout.metrics;
    let projection = match projection {
        DesignValueCellProjection::RetainedOption(element) => return element,
        DesignValueCellProjection::Standard(projection) => *projection,
    };

    let interactive = projection.editable || projection.copyable;
    let cell_id = SharedString::from(format!("{}-{}", projection.panel_id, projection.id_suffix));
    let debug_selector = cell_id.to_string();
    let copy_value = projection.display.clone();
    let mut cell = projection
        .field
        .render(cell_id, metrics, cx)
        .debug_selector(move || debug_selector.clone())
        .flex_1()
        .min_w(px(0.))
        .pl(px(projection.left_padding))
        .pr_2()
        .gap_1()
        .rounded(metrics.radius)
        .border_1()
        .when(projection.editing && !projection.invalid, |cell| {
            cell.border_color(cx.theme().selection)
        })
        .bg(cx.theme().secondary)
        .when(!interactive, |cell| {
            cell.text_color(cx.theme().muted_foreground).opacity(0.78)
        });
    if interactive {
        cell = if let Some(focus) = projection.retained_focus {
            cell.track_focus(&focus.tab_index(0).tab_stop(true))
        } else {
            cell.tab_index(0)
        };
    }
    if projection.editable {
        let activation_sink = sink.clone();
        let event = DesignInspectorFieldEvent::ActivateProperty {
            origin: EditorFocusOrigin::ValueCell(projection.property),
            property: projection.property,
            fallback: projection.fallback,
        };
        cell = cell.on_activate(move |_, window, cx| {
            activation_sink.dispatch(event.clone(), window, cx);
        });
    } else if projection.copyable {
        let copy_sink = sink.clone();
        let event = DesignInspectorFieldEvent::CopyProperty {
            property: projection.property,
            display: copy_value,
        };
        cell = cell.on_activate(move |_, window, cx| {
            copy_sink.dispatch(event.clone(), window, cx);
        });
    }

    if projection.editing && !projection.scrubbing {
        if let Some(label) = projection.additional_label.as_ref() {
            cell = cell.child(render_compact_property_label(label, cx));
        }
        cell = cell.child(
            Input::new(&projection.property_input)
                .appearance(false)
                .bordered(false)
                .focus_bordered(false)
                .xsmall()
                .h(metrics.row_height - px(2.))
                .flex_1()
                .min_w(px(0.)),
        );
    } else {
        let mut readout = h_flex().h_full().flex_1().min_w(px(0.)).gap_1();
        if !projection.prefix.is_empty() {
            readout = readout.child(
                div()
                    .w(px(12.))
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(projection.prefix),
            );
        }
        if let Some(label) = projection.additional_label.as_ref() {
            readout = readout.child(render_compact_property_label(label, cx));
        }
        readout = readout.child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .child(projection.display),
        );
        cell = cell.child(render_numeric_scrub_surface(
            SharedString::from(format!(
                "{}-{}-scrub",
                projection.panel_id, projection.id_suffix
            )),
            sink.panel.clone(),
            projection.property,
            EditorFocusOrigin::ValueCell(projection.property),
            projection.scrub_enabled,
            readout.into_any_element(),
        ));
    }
    if let Some(variable_button) = projection.variable_button {
        cell = cell.child(variable_button);
    }
    cell.into_any_element()
}

fn render_value_field_icon(
    icon: ValueFieldIcon,
    metrics: InspectorMetrics,
    cx: &App,
) -> AnyElement {
    render_lucide_icon(
        match icon {
            ValueFieldIcon::Opacity => LucideIcon::Blend,
            ValueFieldIcon::Corners => LucideIcon::Scan,
        },
        cx.theme().muted_foreground,
        metrics.icon_size.as_f32(),
    )
}

fn render_icon_value_cell_projection(
    projection: DesignIconValueCellProjection,
    sink: DesignInspectorEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let layout = common_field_layout();
    let metrics = layout.metrics;
    let labeled_cell_selector = SharedString::from(format!(
        "{}-{}-additional-label",
        projection.panel_id, projection.id_suffix
    ));
    let labeled_cell_debug_selector = labeled_cell_selector.to_string();
    let prefix = div()
        .w(metrics.row_height)
        .h(layout.row_height())
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(!projection.editable, |prefix| prefix.opacity(0.62))
        .child(render_value_field_icon(projection.icon, metrics, cx));

    h_flex()
        .id(SharedString::from(format!(
            "{}-{}-icon-value-cell",
            projection.panel_id, projection.id_suffix
        )))
        .when(projection.additional_labels, |cell| {
            cell.debug_selector(move || labeled_cell_debug_selector.clone())
        })
        .h(layout.row_height())
        .flex_1()
        .min_w(px(0.))
        .overflow_hidden()
        .rounded(metrics.radius)
        .bg(cx.theme().secondary)
        .child(prefix)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(render_value_cell_projection(projection.value, sink, cx)),
        )
        .into_any_element()
}

fn render_toggle_row_projection(
    projection: DesignToggleRowProjection,
    sink: DesignInspectorEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let layout = common_field_layout();
    let metrics = layout.metrics;
    let editable = projection.field.frame().is_editable();
    let field_for_activate = projection.field;
    let selector = SharedString::from(format!("{}-{}", projection.panel_id, projection.id_suffix));
    let debug_selector = selector.to_string();
    let mut row = crate::molecules::inspector_row_with_layout(layout)
        .id(selector)
        .debug_selector(move || debug_selector.clone())
        .h(layout.row_height())
        .w_full()
        .gap_0()
        .justify_between()
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .when(editable, |row| {
            row.key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent.opacity(0.55)))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_color(cx.theme().selection)
                })
        })
        .when(!editable, |row| {
            row.text_color(cx.theme().muted_foreground).opacity(0.78)
        });
    if editable {
        let property = projection.property;
        let checked = projection.checked;
        row = row.on_activate(move |_, window, cx| {
            let Some(edit) = field_for_activate.set(!checked) else {
                return;
            };
            let crate::molecules::InspectorValue::Uniform(next) = edit.value else {
                return;
            };
            sink.dispatch(
                DesignInspectorFieldEvent::CommitBoolean {
                    property,
                    value: next,
                },
                window,
                cx,
            );
        });
    }
    row.child(div().flex_1().text_xs().child(projection.label))
        .when_some(projection.variable_button, |row, button| row.child(button))
        .child(
            h_flex()
                .w(px(30.))
                .h(px(18.))
                .p(px(2.))
                .justify_end()
                .when(!projection.checked, |toggle| toggle.justify_start())
                .rounded(px(9.))
                .bg(if projection.checked {
                    cx.theme().selection
                } else {
                    cx.theme().border
                })
                .child(
                    div()
                        .size(px(14.))
                        .rounded(px(7.))
                        .bg(cx.theme().background),
                ),
        )
        .into_any_element()
}

fn render_checkbox_row_projection(
    projection: DesignCheckboxRowProjection,
    sink: DesignInspectorEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let layout = common_field_layout();
    let metrics = layout.metrics;
    let editable = projection.field.frame().is_editable();
    let field_for_action = projection.field.clone();
    let field_for_click = projection.field;
    let action_sink = sink.clone();
    let action_property = projection.property;
    let checked = projection.checked;
    let click_property = projection.property;
    let id_suffix = projection.id_suffix;
    crate::molecules::inspector_row_with_layout(layout)
        .id(SharedString::from(format!(
            "{}-{id_suffix}-row",
            projection.panel_id
        )))
        .h(layout.row_height())
        .gap_0()
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .when(editable, |row| {
            row.key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .focus(|style| style.border_color(cx.theme().selection))
                .on_action(move |_: &ActivateControl, window, cx| {
                    let Some(edit) = field_for_action.set(!checked) else {
                        return;
                    };
                    let crate::molecules::InspectorValue::Uniform(next) = edit.value else {
                        return;
                    };
                    action_sink.dispatch(
                        DesignInspectorFieldEvent::CommitBoolean {
                            property: action_property,
                            value: next,
                        },
                        window,
                        cx,
                    );
                })
        })
        .child(
            Checkbox::new(SharedString::from(format!(
                "{}-{id_suffix}",
                projection.panel_id
            )))
            .xsmall()
            .label(projection.label)
            .checked(projection.checked)
            .disabled(!editable)
            .tab_stop(false)
            .on_click(move |next, window, cx| {
                let Some(edit) = field_for_click.set(*next) else {
                    return;
                };
                let crate::molecules::InspectorValue::Uniform(next) = edit.value else {
                    return;
                };
                sink.dispatch(
                    DesignInspectorFieldEvent::CommitBoolean {
                        property: click_property,
                        value: next,
                    },
                    window,
                    cx,
                );
            })
            .h(layout.row_height()),
        )
        .into_any_element()
}

/// Temporary compatibility surface while retained feature controllers migrate
/// to accept `DesignValueCellProjection` directly. All rendering and event
/// dispatch are standalone; this trait only constructs owned snapshots at the
/// facade boundary and preserves existing in-crate call sites.
pub(super) trait DesignInspectorFieldRenderer {
    fn render_compact_property_label(
        &self,
        id_suffix: &SharedString,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn render_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    #[allow(clippy::too_many_arguments)]
    fn render_value_cell_with_left_padding(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        left_padding: f32,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn render_value_field_icon(
        &self,
        icon: ValueFieldIcon,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn render_icon_value_cell(
        &self,
        id_suffix: &'static str,
        icon: ValueFieldIcon,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn render_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn render_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn render_checkbox_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl DesignInspectorFieldRenderer for DesignPanel {
    fn render_compact_property_label(
        &self,
        id_suffix: &SharedString,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        render_compact_property_label(
            &DesignCompactPropertyLabelProjection::for_property(&self.id, id_suffix, property),
            cx,
        )
    }

    fn render_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_cell_with_left_padding(id_suffix, prefix, value, property, next, 8., cx)
    }

    fn render_value_cell_with_left_padding(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        left_padding: f32,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let projection = project_value_cell(
            self,
            DesignValueCellRequest {
                id_suffix: id_suffix.into(),
                prefix,
                value: value.into(),
                property,
                fallback: next,
                left_padding,
            },
            cx,
        );
        render_value_cell_projection(projection, DesignInspectorEventSink::new(cx.entity()), cx)
    }

    fn render_value_field_icon(
        &self,
        icon: ValueFieldIcon,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        render_value_field_icon(icon, common_field_layout().metrics, cx)
    }

    fn render_icon_value_cell(
        &self,
        id_suffix: &'static str,
        icon: ValueFieldIcon,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let projection = DesignIconValueCellProjection {
            panel_id: self.id.clone(),
            id_suffix,
            icon,
            editable: self.property_is_editable(property),
            additional_labels: self.preferences.additional_labels,
            value: project_value_cell(
                self,
                DesignValueCellRequest {
                    id_suffix: format!("{id_suffix}-value").into(),
                    prefix: "",
                    value: value.into(),
                    property,
                    fallback: next,
                    left_padding: 0.,
                },
                cx,
            ),
        };
        render_icon_value_cell_projection(
            projection,
            DesignInspectorEventSink::new(cx.entity()),
            cx,
        )
    }

    fn render_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let projection = DesignToggleRowProjection {
            panel_id: self.id.clone(),
            id_suffix: id_suffix.into(),
            label,
            checked,
            property,
            field: crate::molecules::InspectorToggleField::new(
                inspector_boolean_value(self, property, checked),
                inspector_property_presentation(self, property, false, false),
            ),
            variable_button: self.render_property_variable_button(property, cx),
        };
        render_toggle_row_projection(projection, DesignInspectorEventSink::new(cx.entity()), cx)
    }

    fn render_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement {
        div()
            .h(px(16.))
            .flex()
            .items_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .into_any_element()
    }

    fn render_checkbox_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        let projection = DesignCheckboxRowProjection {
            panel_id: self.id.clone(),
            id_suffix: id_suffix.into(),
            label,
            checked,
            property,
            field: crate::molecules::InspectorCheckboxField::new(
                inspector_boolean_value(self, property, checked),
                inspector_property_presentation(self, property, false, false),
            ),
        };
        render_checkbox_row_projection(projection, DesignInspectorEventSink::new(cx.entity()), cx)
    }
}

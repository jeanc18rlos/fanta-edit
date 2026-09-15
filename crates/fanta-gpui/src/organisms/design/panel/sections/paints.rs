use super::super::super::{DesignPaintStyleBinding, DesignSelectionColor, DesignStroke};
use super::super::*;

/// Stable identity shared by the Selection, Fill, and Stroke projections.
///
/// Renderers receive this immutable snapshot instead of reading the panel
/// facade. Direct events return through a typed sink and are validated against
/// the latest host snapshot before they are emitted.
#[derive(Clone)]
pub(in super::super) struct PaintSectionIdentity {
    panel_id: SharedString,
    node_id: SharedString,
    target: DesignPanelTarget,
}

impl PaintSectionIdentity {
    pub(in super::super) fn new(
        panel_id: SharedString,
        node_id: SharedString,
        target: DesignPanelTarget,
    ) -> Self {
        Self {
            panel_id,
            node_id,
            target,
        }
    }
}

/// Host-owned collection data common to Fill and Stroke.
#[derive(Clone)]
pub(in super::super) struct PaintCollectionProjection {
    supported: bool,
    paints: Vec<DesignPaint>,
    style_binding: Option<DesignPaintStyleBinding>,
}

impl PaintCollectionProjection {
    pub(in super::super) fn new(
        supported: bool,
        paints: Vec<DesignPaint>,
        style_binding: Option<DesignPaintStyleBinding>,
    ) -> Self {
        Self {
            supported,
            paints,
            style_binding,
        }
    }
}

/// Fill-owned values layered over the shared collection projection.
#[derive(Clone)]
pub(in super::super) struct FillProjection {
    collection: PaintCollectionProjection,
    shows_in_exports: Option<bool>,
}

impl FillProjection {
    pub(in super::super) fn new(
        collection: PaintCollectionProjection,
        shows_in_exports: Option<bool>,
    ) -> Self {
        Self {
            collection,
            shows_in_exports,
        }
    }
}

/// Availability of Stroke actions that do not pass through a field control.
#[derive(Clone, Copy)]
pub(in super::super) struct StrokeActionAvailability {
    weight_mode: bool,
    swap_endpoints: bool,
}

impl StrokeActionAvailability {
    pub(in super::super) const fn new(weight_mode: bool, swap_endpoints: bool) -> Self {
        Self {
            weight_mode,
            swap_endpoints,
        }
    }
}

/// Stroke-owned geometry and action capabilities.
#[derive(Clone)]
pub(in super::super) struct StrokeProjection {
    identity: PaintSectionIdentity,
    collection: PaintCollectionProjection,
    stroke: Option<DesignStroke>,
    actions: StrokeActionAvailability,
}

impl StrokeProjection {
    pub(in super::super) fn new(
        identity: PaintSectionIdentity,
        collection: PaintCollectionProjection,
        stroke: Option<DesignStroke>,
        actions: StrokeActionAvailability,
    ) -> Self {
        Self {
            identity,
            collection,
            stroke,
            actions,
        }
    }
}

/// Multiple-selection paint aggregation presented by Selection colors.
#[derive(Clone)]
pub(in super::super) struct SelectionColorsProjection {
    identity: PaintSectionIdentity,
    selection_is_multiple: bool,
    colors: Vec<DesignSelectionColor>,
}

impl SelectionColorsProjection {
    pub(in super::super) fn new(
        identity: PaintSectionIdentity,
        selection_is_multiple: bool,
        colors: Vec<DesignSelectionColor>,
    ) -> Self {
        Self {
            identity,
            selection_is_multiple,
            colors,
        }
    }
}

/// Narrow bridge to established inspector chrome and the retained complex
/// paint picker/resource-browser controllers.
pub(in super::super) trait PaintsInspectorChrome {
    fn paints_collection_row(
        &self,
        paint: DesignPaint,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_bound_style_summary(
        &self,
        id_suffix: &'static str,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_checkbox_row(
        &self,
        id_suffix: &'static str,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement;

    fn paints_section(
        &self,
        section: DesignPanelSection,
        collection: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_selection_picker(
        &self,
        index: usize,
        color: &DesignSelectionColor,
        target: AuxiliaryColorPickerTarget,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_selection_style_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_selection_variable_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn paints_selection_occurrences_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl PaintsInspectorChrome for DesignPanel {
    fn paints_collection_row(
        &self,
        paint: DesignPaint,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_paint_row(paint, collection, index, cx)
    }

    fn paints_bound_style_summary(
        &self,
        id_suffix: &'static str,
        name: SharedString,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_bound_style_summary(id_suffix, name, cx)
    }

    fn paints_checkbox_row(
        &self,
        id_suffix: &'static str,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_checkbox_row(id_suffix, label, checked, property, cx)
    }

    fn paints_value_cell(
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

    fn paints_group_label(&self, label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement {
        self.render_group_label(label, cx)
    }

    fn paints_section(
        &self,
        section: DesignPanelSection,
        collection: Option<DesignPanelCollection>,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(section, collection, content, cx)
    }

    fn paints_selection_picker(
        &self,
        index: usize,
        color: &DesignSelectionColor,
        target: AuxiliaryColorPickerTarget,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_selection_paint_picker_control(index, color, target, cx)
    }

    fn paints_selection_style_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_selection_color_paint_style_button(color, cx)
    }

    fn paints_selection_variable_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_selection_color_variable_button(color, cx)
    }

    fn paints_selection_occurrences_button(
        &self,
        color: &DesignSelectionColor,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_selection_color_occurrences_button(color, cx)
    }
}

fn dispatch_direct_action(
    panel: &mut DesignPanel,
    expected_node_id: &SharedString,
    action: DesignPanelAction,
    cx: &mut Context<DesignPanel>,
) {
    if !direct_action_targets_current_node(&panel.host.inspected_node().id, expected_node_id)
        || !panel.can_edit()
        || !panel.node_capability_allows_action(&action)
    {
        return;
    }
    cx.emit_design_panel_action(panel, action);
}

fn direct_action_targets_current_node(
    current_node_id: &SharedString,
    expected_node_id: &SharedString,
) -> bool {
    current_node_id == expected_node_id
}

/// Typed event channel for Fill/Stroke actions that bypass field chrome.
#[derive(Clone)]
pub(in super::super) struct PaintsEventSink {
    panel: Entity<DesignPanel>,
}

impl PaintsEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send_direct(
        &self,
        expected_node_id: &SharedString,
        action: DesignPanelAction,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            dispatch_direct_action(panel, expected_node_id, action, cx);
        });
    }
}

fn render_direct_icon_action(
    identity: &PaintSectionIdentity,
    id_suffix: &'static str,
    icon: LucideIcon,
    action: DesignPanelAction,
    enabled: bool,
    events: &PaintsEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let icon_color = if enabled {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    let mut button = div()
        .id(SharedString::from(format!(
            "{}-{id_suffix}",
            identity.panel_id
        )))
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
        let expected_node_id = identity.node_id.clone();
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            let action = action.clone();
            events.send_direct(&expected_node_id, action, cx);
        });
    }
    button
        .child(render_lucide_icon(icon, icon_color, 16.))
        .into_any_element()
}

fn render_direct_compact_icon_action(
    identity: &PaintSectionIdentity,
    id_suffix: &'static str,
    icon: LucideIcon,
    action: DesignPanelAction,
    enabled: bool,
    events: &PaintsEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let icon_color = if enabled {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    let mut button = div()
        .id(SharedString::from(format!(
            "{}-{id_suffix}",
            identity.panel_id
        )))
        .size(px(ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(cx.theme().transparent)
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
        let expected_node_id = identity.node_id.clone();
        let events = events.clone();
        button = button.on_activate(move |_, _, cx| {
            let action = action.clone();
            events.send_direct(&expected_node_id, action, cx);
        });
    }
    button
        .child(render_lucide_icon(icon, icon_color, 16.))
        .into_any_element()
}

pub(in super::super) fn render_fill(
    projection: &FillProjection,
    chrome: &impl PaintsInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    projection.collection.supported.then(|| {
        let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pt_1().pb_4().gap_2();
        if let Some(binding) = projection.collection.style_binding.as_ref() {
            content =
                content.child(chrome.paints_bound_style_summary("fill", binding.name.clone(), cx));
        } else {
            for (index, paint) in projection.collection.paints.iter().cloned().enumerate() {
                content = content.child(chrome.paints_collection_row(
                    paint,
                    DesignPanelCollection::Fill,
                    index,
                    cx,
                ));
            }
            if projection.collection.paints.is_empty() {
                content = content.child(empty_collection("No fills", cx));
            }
            if let Some(show_in_exports) = projection
                .shows_in_exports
                .filter(|_| !projection.collection.paints.is_empty())
            {
                content = content.child(chrome.paints_checkbox_row(
                    "fill-shows-in-exports",
                    "Show in exports",
                    show_in_exports,
                    DesignPanelProperty::FillShowsInExports,
                    cx,
                ));
            }
        }
        chrome.paints_section(
            DesignPanelSection::Fill,
            Some(DesignPanelCollection::Fill),
            content.into_any_element(),
            cx,
        )
    })
}

pub(in super::super) fn render_selection_colors(
    projection: &SelectionColorsProjection,
    chrome: &impl PaintsInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    if !projection.selection_is_multiple || projection.colors.is_empty() {
        return None;
    }
    let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_1();
    for (index, selection_color) in projection.colors.iter().enumerate() {
        let target = AuxiliaryColorPickerTarget::SelectionColor {
            target: projection.identity.target.clone(),
            selection_color_id: selection_color.id.clone(),
        };
        content = content.child(
            h_flex()
                .h(px(28.))
                .w_full()
                .gap_1()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(chrome.paints_selection_picker(index, selection_color, target, cx)),
                )
                .child(chrome.paints_selection_style_button(selection_color, cx))
                .child(chrome.paints_selection_variable_button(selection_color, cx))
                .child(chrome.paints_selection_occurrences_button(selection_color, cx)),
        );
    }
    Some(chrome.paints_section(
        DesignPanelSection::Selection,
        None,
        content.into_any_element(),
        cx,
    ))
}

pub(in super::super) fn render_stroke(
    projection: &StrokeProjection,
    chrome: &impl PaintsInspectorChrome,
    events: &PaintsEventSink,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    projection.collection.supported.then(|| {
        let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_2();
        let Some(stroke) = projection.stroke.as_ref() else {
            return chrome.paints_section(
                DesignPanelSection::Stroke,
                Some(DesignPanelCollection::Stroke),
                div().into_any_element(),
                cx,
            );
        };

        if let Some(binding) = projection.collection.style_binding.as_ref() {
            content = content.child(chrome.paints_bound_style_summary(
                "stroke",
                binding.name.clone(),
                cx,
            ));
        } else {
            for (index, paint) in projection.collection.paints.iter().cloned().enumerate() {
                content = content.child(chrome.paints_collection_row(
                    paint,
                    DesignPanelCollection::Stroke,
                    index,
                    cx,
                ));
            }
        }
        if projection.collection.paints.is_empty() && projection.collection.style_binding.is_none()
        {
            return chrome.paints_section(
                DesignPanelSection::Stroke,
                Some(DesignPanelCollection::Stroke),
                div().into_any_element(),
                cx,
            );
        }

        let basic = stroke.complex_stroke.is_basic();
        let mut geometry = v_flex().w_full().gap_2();
        let weight_mode_action = DesignPanelAction::PropertyChangeRequested {
            node_id: projection.identity.node_id.clone(),
            property: DesignPanelProperty::StrokeWeightMode,
            value: DesignPanelValue::StrokeWeightMode(
                if stroke.weights.mode == DesignStrokeWeightMode::Custom {
                    DesignStrokeWeightMode::All
                } else {
                    DesignStrokeWeightMode::Custom
                },
            ),
        };
        let weight_control = h_flex()
            .w_full()
            .gap_1()
            .child(div().flex_1().min_w(px(0.)).child(chrome.paints_value_cell(
                "stroke-weight",
                "Weight",
                format_number(stroke.weights.active()),
                DesignPanelProperty::StrokeWeight,
                DesignPanelValue::Number(stroke.weights.active() + 1.),
                cx,
            )))
            .when(stroke.capabilities.individual_weights, |row| {
                row.child(
                    div()
                        .w(px(24.))
                        .flex_none()
                        .child(render_direct_icon_action(
                            &projection.identity,
                            "stroke-weight-mode",
                            LucideIcon::Settings2,
                            weight_mode_action,
                            projection.actions.weight_mode,
                            events,
                            cx,
                        )),
                )
            })
            .into_any_element();

        if stroke.capabilities.position && basic {
            geometry = geometry.child(
                h_flex()
                    .w_full()
                    .items_end()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_1()
                            .child(chrome.paints_group_label("Position", cx))
                            .child(chrome.paints_value_cell(
                                "stroke-align",
                                "Align",
                                stroke.align.label(),
                                DesignPanelProperty::StrokeAlign,
                                DesignPanelValue::StrokeAlign(DesignStrokeAlign::Center),
                                cx,
                            )),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_1()
                            .child(chrome.paints_group_label("Weight", cx))
                            .child(weight_control),
                    ),
            );
        } else {
            geometry = geometry.child(
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(chrome.paints_group_label("Weight", cx))
                    .child(weight_control),
            );
        }

        if stroke.capabilities.individual_weights
            && stroke.weights.mode == DesignStrokeWeightMode::Custom
        {
            geometry = geometry
                .child(
                    h_flex()
                        .gap_2()
                        .child(chrome.paints_value_cell(
                            "stroke-weight-top",
                            "T",
                            format_number(stroke.weights.top),
                            DesignPanelProperty::StrokeWeightTop,
                            DesignPanelValue::Number(stroke.weights.top + 1.),
                            cx,
                        ))
                        .child(chrome.paints_value_cell(
                            "stroke-weight-right",
                            "R",
                            format_number(stroke.weights.right),
                            DesignPanelProperty::StrokeWeightRight,
                            DesignPanelValue::Number(stroke.weights.right + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(chrome.paints_value_cell(
                            "stroke-weight-bottom",
                            "B",
                            format_number(stroke.weights.bottom),
                            DesignPanelProperty::StrokeWeightBottom,
                            DesignPanelValue::Number(stroke.weights.bottom + 1.),
                            cx,
                        ))
                        .child(chrome.paints_value_cell(
                            "stroke-weight-left",
                            "L",
                            format_number(stroke.weights.left),
                            DesignPanelProperty::StrokeWeightLeft,
                            DesignPanelValue::Number(stroke.weights.left + 1.),
                            cx,
                        )),
                );
        }

        if basic {
            geometry = geometry
                .child(chrome.paints_group_label("Dash style", cx))
                .child(chrome.paints_value_cell(
                    "stroke-dash-mode",
                    "⋯",
                    stroke.dashes.mode.label(),
                    DesignPanelProperty::StrokeDashMode,
                    DesignPanelValue::StrokeDashMode(stroke.dashes.mode.next()),
                    cx,
                ));
            if !stroke.dashes.is_solid() {
                geometry = geometry
                    .child(chrome.paints_group_label("Dash pattern", cx))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                chrome.paints_value_cell(
                                    "stroke-dash-pattern",
                                    "⋯",
                                    stroke
                                        .dashes
                                        .pattern
                                        .iter()
                                        .map(|value| format_number(*value))
                                        .collect::<Vec<_>>()
                                        .join(", "),
                                    DesignPanelProperty::StrokeDashPattern,
                                    DesignPanelValue::NumberList(stroke.dashes.pattern.clone()),
                                    cx,
                                ),
                            )
                            .child(chrome.paints_value_cell(
                                "stroke-dash-cap",
                                "Cap",
                                stroke.dash_cap.label(),
                                DesignPanelProperty::StrokeDashCap,
                                DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                cx,
                            )),
                    );
            }

            geometry = match stroke.edit_context.endpoint_control() {
                DesignStrokeEndpointControl::None => geometry,
                DesignStrokeEndpointControl::StartAndEnd => geometry.child(
                    h_flex()
                        .w_full()
                        .items_end()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(chrome.paints_group_label("Start point", cx))
                                .child(chrome.paints_value_cell(
                                    "stroke-start-cap",
                                    "←",
                                    stroke.start_cap.label(),
                                    DesignPanelProperty::StrokeStartCap,
                                    DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                                    cx,
                                )),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(chrome.paints_group_label("End point", cx))
                                .child(chrome.paints_value_cell(
                                    "stroke-end-cap",
                                    "→",
                                    stroke.end_cap.label(),
                                    DesignPanelProperty::StrokeEndCap,
                                    DesignPanelValue::StrokeCap(DesignStrokeCap::LineArrow),
                                    cx,
                                )),
                        )
                        .child(render_direct_compact_icon_action(
                            &projection.identity,
                            "swap-stroke-endpoints",
                            LucideIcon::ArrowLeftRight,
                            DesignPanelAction::SwapStrokeEndpointsRequested {
                                node_id: projection.identity.node_id.clone(),
                            },
                            projection.actions.swap_endpoints,
                            events,
                            cx,
                        )),
                ),
                DesignStrokeEndpointControl::Aggregate => geometry.child(chrome.paints_value_cell(
                    "stroke-endpoint-cap",
                    "Ends",
                    format!("End points · {}", stroke.endpoint_cap.label()),
                    DesignPanelProperty::StrokeEndpointCap,
                    DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                    cx,
                )),
                DesignStrokeEndpointControl::SelectedVertices => {
                    geometry.child(chrome.paints_value_cell(
                        "stroke-selected-endpoint-cap",
                        "Selection",
                        format!("Selection · {}", stroke.endpoint_cap.label()),
                        DesignPanelProperty::StrokeEndpointCap,
                        DesignPanelValue::StrokeCap(DesignStrokeCap::Round),
                        cx,
                    ))
                }
            };

            if stroke.capabilities.joins {
                geometry = geometry.child(
                    h_flex()
                        .gap_2()
                        .child(chrome.paints_value_cell(
                            "stroke-join",
                            "Join",
                            stroke.join.label(),
                            DesignPanelProperty::StrokeJoin,
                            DesignPanelValue::StrokeJoin(DesignStrokeJoin::Round),
                            cx,
                        ))
                        .when(stroke.join == DesignStrokeJoin::Miter, |row| {
                            row.child(chrome.paints_value_cell(
                                "stroke-miter-angle",
                                "°",
                                format_number(stroke.miter_angle),
                                DesignPanelProperty::StrokeMiterAngle,
                                DesignPanelValue::Number((stroke.miter_angle + 1.).min(180.)),
                                cx,
                            ))
                        }),
                );
            }
        }

        if stroke.supports_variable_width() {
            let variable_width_label = stroke
                .variable_width
                .as_ref()
                .map_or("None", DesignVariableWidthStroke::label);
            geometry = geometry
                .child(chrome.paints_group_label("Variable width", cx))
                .child(chrome.paints_value_cell(
                    "stroke-variable-width",
                    "⌇",
                    variable_width_label,
                    DesignPanelProperty::StrokeVariableWidth,
                    DesignPanelValue::StrokeVariableWidth(Some(DesignVariableWidthStroke::Preset(
                        DesignVariableWidthPreset::Taper,
                    ))),
                    cx,
                ));
            if let Some(DesignVariableWidthStroke::Custom { points }) =
                stroke.variable_width.as_ref()
            {
                for (index, point) in points.iter().enumerate() {
                    geometry = geometry
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("Point {}", index + 1)),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(chrome.paints_value_cell(
                                    format!("stroke-width-point-{index}-position"),
                                    "P",
                                    format_number(point.position),
                                    DesignPanelProperty::StrokeVariableWidthPointPosition(index),
                                    DesignPanelValue::Number((point.position + 0.05).min(1.)),
                                    cx,
                                ))
                                .child(chrome.paints_value_cell(
                                    format!("stroke-width-point-{index}-width"),
                                    "W",
                                    format_number(point.width),
                                    DesignPanelProperty::StrokeVariableWidthPointWidth(index),
                                    DesignPanelValue::Number(point.width + 0.1),
                                    cx,
                                )),
                        );
                }
            }
        }

        if stroke.capabilities.complex_stroke {
            let next_type = match stroke.complex_stroke.kind() {
                DesignStrokeType::Basic => DesignStrokeType::StretchBrush,
                DesignStrokeType::StretchBrush => DesignStrokeType::ScatterBrush,
                DesignStrokeType::ScatterBrush => DesignStrokeType::Dynamic,
                DesignStrokeType::Dynamic => DesignStrokeType::Basic,
                DesignStrokeType::Opaque => DesignStrokeType::Opaque,
            };
            geometry = geometry.child(chrome.paints_value_cell(
                "stroke-type",
                "Type",
                stroke.complex_stroke.label(),
                DesignPanelProperty::StrokeType,
                DesignPanelValue::StrokeType(next_type),
                cx,
            ));
            match &stroke.complex_stroke {
                DesignComplexStroke::Basic => {}
                DesignComplexStroke::StretchBrush(stretch) => {
                    geometry = geometry.child(
                        h_flex()
                            .gap_2()
                            .child(chrome.paints_value_cell(
                                "stroke-stretch-brush",
                                "✎",
                                stretch.brush.label(),
                                DesignPanelProperty::StrokeStretchBrush,
                                DesignPanelValue::StrokeStretchBrush(stretch.brush.next()),
                                cx,
                            ))
                            .child(chrome.paints_value_cell(
                                "stroke-brush-direction",
                                "→",
                                stretch.direction.label(),
                                DesignPanelProperty::StrokeBrushDirection,
                                DesignPanelValue::StrokeBrushDirection(match stretch.direction {
                                    DesignStrokeBrushDirection::Forward => {
                                        DesignStrokeBrushDirection::Backward
                                    }
                                    DesignStrokeBrushDirection::Backward => {
                                        DesignStrokeBrushDirection::Forward
                                    }
                                }),
                                cx,
                            )),
                    );
                }
                DesignComplexStroke::ScatterBrush(scatter) => {
                    geometry = geometry
                        .child(chrome.paints_value_cell(
                            "stroke-scatter-brush",
                            "✣",
                            scatter.brush.label(),
                            DesignPanelProperty::StrokeScatterBrush,
                            DesignPanelValue::StrokeScatterBrush(scatter.brush.next()),
                            cx,
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(chrome.paints_value_cell(
                                    "stroke-scatter-gap",
                                    "G",
                                    format_number(scatter.gap),
                                    DesignPanelProperty::StrokeScatterGap,
                                    DesignPanelValue::Number(scatter.gap + 0.25),
                                    cx,
                                ))
                                .child(chrome.paints_value_cell(
                                    "stroke-scatter-wiggle",
                                    "W",
                                    format_number(scatter.wiggle),
                                    DesignPanelProperty::StrokeScatterWiggle,
                                    DesignPanelValue::Number(scatter.wiggle + 0.1),
                                    cx,
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(chrome.paints_value_cell(
                                    "stroke-scatter-size-jitter",
                                    "S",
                                    format_number(scatter.size_jitter),
                                    DesignPanelProperty::StrokeScatterSizeJitter,
                                    DesignPanelValue::Number((scatter.size_jitter + 0.1).min(3.)),
                                    cx,
                                ))
                                .child(chrome.paints_value_cell(
                                    "stroke-scatter-angular-jitter",
                                    "A",
                                    format_number(scatter.angular_jitter),
                                    DesignPanelProperty::StrokeScatterAngularJitter,
                                    DesignPanelValue::Number(
                                        (scatter.angular_jitter + 15.).min(180.),
                                    ),
                                    cx,
                                )),
                        )
                        .child(chrome.paints_value_cell(
                            "stroke-scatter-rotation",
                            "R",
                            format_number(scatter.rotation),
                            DesignPanelProperty::StrokeScatterRotation,
                            DesignPanelValue::Number((scatter.rotation + 15.).min(180.)),
                            cx,
                        ));
                }
                DesignComplexStroke::Dynamic(dynamic) => {
                    geometry = geometry
                        .child(chrome.paints_value_cell(
                            "stroke-dynamic-frequency",
                            "F",
                            format_number(dynamic.frequency),
                            DesignPanelProperty::StrokeDynamicFrequency,
                            DesignPanelValue::Number((dynamic.frequency + 0.25).min(20.)),
                            cx,
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(chrome.paints_value_cell(
                                    "stroke-dynamic-wiggle",
                                    "W",
                                    format_number(dynamic.wiggle),
                                    DesignPanelProperty::StrokeDynamicWiggle,
                                    DesignPanelValue::Number(dynamic.wiggle + 0.1),
                                    cx,
                                ))
                                .child(chrome.paints_value_cell(
                                    "stroke-dynamic-smoothen",
                                    "S",
                                    format_number(dynamic.smoothen),
                                    DesignPanelProperty::StrokeDynamicSmoothen,
                                    DesignPanelValue::Number((dynamic.smoothen + 0.1).min(1.)),
                                    cx,
                                )),
                        );
                }
                DesignComplexStroke::Opaque(opaque) => {
                    geometry = geometry.child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(chrome.paints_group_label("Custom stroke", cx))
                            .child(
                                div()
                                    .w_full()
                                    .min_h(px(ROW_HEIGHT))
                                    .px_2()
                                    .py_1()
                                    .rounded(px(4.))
                                    .bg(cx.theme().secondary)
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{} · {}", opaque.type_name, opaque.raw)),
                            ),
                    );
                }
            }
        }
        content = content.child(geometry);
        chrome.paints_section(
            DesignPanelSection::Stroke,
            Some(DesignPanelCollection::Stroke),
            content.into_any_element(),
            cx,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_action_validation_rejects_a_stale_node() {
        let current = SharedString::from("current");
        let stale = SharedString::from("stale");
        assert!(direct_action_targets_current_node(&current, &current));
        assert!(!direct_action_targets_current_node(&current, &stale));
    }

    #[test]
    fn collection_projection_keeps_support_separate_from_contents() {
        let projection = PaintCollectionProjection::new(false, Vec::new(), None);
        assert!(!projection.supported);
        assert!(projection.paints.is_empty());
        assert!(projection.style_binding.is_none());
    }

    #[test]
    fn section_boundary_uses_projections_and_thin_facade_delegates() {
        let section = include_str!("paints.rs");
        let inherent_impl_marker = ["impl ", "DesignPanel", " {"].concat();
        assert!(!section.contains(&inherent_impl_marker));
        assert!(section.contains("struct PaintsEventSink"));
        assert!(!section.contains("render_stroke(\n    projection: &StrokeProjection,\n    chrome: &impl PaintsInspectorChrome,\n    panel: Entity<DesignPanel>"));

        let facade = include_str!("../paints.rs");
        assert!(facade.contains("sections::paints::render_fill(&self.fill_projection()"));
        assert!(facade.contains("sections::paints::render_stroke(&self.stroke_projection()"));
        assert!(facade.contains("sections::paints::render_selection_colors("));
    }
}

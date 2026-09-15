//! Standalone projections and renderers for legacy shape-related sections.

use super::super::super::{DesignRepeatModifier, DesignSectionProperties};
use super::super::*;

/// Immutable data required by the shape, Section, and Transform renderers.
///
/// Keeping this projection separate from [`DesignPanel`] prevents extracted
/// sections from reaching into the facade's retained host or interaction
/// state. Event handlers return through a narrow sink so every
/// action is revalidated against the latest host snapshot before emission.
#[derive(Clone)]
pub(in super::super) struct ShapeProjection {
    panel_id: SharedString,
    node_id: SharedString,
    can_edit: bool,
    kind: DesignPanelNodeKind,
    shape_geometry: DesignShapeGeometry,
    supports_geometry: bool,
    supports_layer: bool,
    supports_mask: bool,
    supports_section_properties: bool,
    supports_transform: bool,
    effective_mask_type: Option<DesignMaskType>,
    effective_is_mask: bool,
    section: Option<DesignSectionProperties>,
    transform_modifiers: Vec<DesignRepeatModifier>,
}

impl ShapeProjection {
    pub(in super::super) fn from_node(
        panel_id: SharedString,
        can_edit: bool,
        node: &DesignPanelNode,
    ) -> Self {
        Self {
            panel_id,
            node_id: node.id.clone(),
            can_edit,
            kind: node.kind,
            shape_geometry: node.shape_geometry,
            supports_geometry: node.supports_section(DesignPanelSection::Geometry),
            supports_layer: node.supports_section(DesignPanelSection::Layer),
            supports_mask: node.supports_section(DesignPanelSection::Mask),
            supports_section_properties: node.supports_section(DesignPanelSection::Section),
            supports_transform: node.supports_section(DesignPanelSection::Transform),
            effective_mask_type: node.effective_mask_type(),
            effective_is_mask: node.effective_is_mask(),
            section: node.section.clone(),
            transform_modifiers: node.transform_modifiers.clone(),
        }
    }

    fn allows_action(&self, action: &DesignPanelAction) -> bool {
        match action {
            DesignPanelAction::SectionShareRequested { node_id } => {
                *node_id == self.node_id
                    && self.supports_section_properties
                    && self
                        .section
                        .as_ref()
                        .is_some_and(|section| section.capabilities.share)
            }
            DesignPanelAction::SectionResolveChangedStatusRequested { node_id } => {
                *node_id == self.node_id
                    && self.can_edit
                    && self.supports_section_properties
                    && self.section.as_ref().is_some_and(|section| {
                        section.capabilities.resolve_changed_status
                            && section
                                .dev_status
                                .as_ref()
                                .is_some_and(|status| status.changed)
                    })
            }
            DesignPanelAction::TransformModifierAddRequested { node_id, .. } => {
                *node_id == self.node_id
                    && self.can_edit
                    && self.kind == DesignPanelNodeKind::TransformGroup
                    && self.supports_transform
            }
            DesignPanelAction::TransformModifierRemoveRequested {
                node_id,
                modifier_id,
                index,
            } => {
                *node_id == self.node_id
                    && self.can_edit
                    && self.kind == DesignPanelNodeKind::TransformGroup
                    && self.supports_transform
                    && self
                        .transform_modifiers
                        .get(*index)
                        .is_some_and(|modifier| modifier.id == *modifier_id)
            }
            DesignPanelAction::ApplyTransformModifiersRequested { node_id } => {
                *node_id == self.node_id
                    && self.can_edit
                    && self.kind == DesignPanelNodeKind::TransformGroup
                    && self.supports_transform
                    && !self.transform_modifiers.is_empty()
            }
            _ => false,
        }
    }
}

/// Narrow rendering surface supplied by the facade while legacy value fields
/// migrate to shared inspector molecules.
///
/// This deliberately exposes only the four chrome operations this extracted
/// slice needs; it cannot inspect or mutate facade state.
pub(in super::super) trait ShapeInspectorChrome {
    fn shape_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn shape_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn shape_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn shape_appearance_controls(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement>;
}

impl ShapeInspectorChrome for DesignPanel {
    fn shape_value_cell(
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

    fn shape_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: bool,
        property: DesignPanelProperty,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_toggle_row(id_suffix, label, value, property, cx)
    }

    fn shape_section(
        &self,
        section: DesignPanelSection,
        content: AnyElement,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_section(section, None, content, cx)
    }

    fn shape_appearance_controls(&self, cx: &mut Context<DesignPanel>) -> Option<AnyElement> {
        self.render_shape_appearance_controls(cx)
    }
}

#[derive(Clone)]
pub(in super::super) struct ShapeEventSink {
    panel: Entity<DesignPanel>,
}

impl ShapeEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch(&self, action: DesignPanelAction, cx: &mut App) {
        self.panel.update(cx, |panel, cx| {
            if panel.node_capability_allows_action(&action) {
                cx.emit_design_panel_action(panel, action);
            }
        });
    }
}

impl From<&Entity<DesignPanel>> for ShapeEventSink {
    fn from(panel: &Entity<DesignPanel>) -> Self {
        Self::new(panel.clone())
    }
}

pub(in super::super) fn transform_modifier_change_for_property(
    modifiers: &[DesignRepeatModifier],
    property: DesignPanelProperty,
    value: &DesignPanelValue,
) -> Option<(SharedString, usize, DesignTransformModifierChange)> {
    let (index, change) = match (property, value) {
        (
            DesignPanelProperty::TransformRepeatType(index),
            DesignPanelValue::RepeatType(repeat_type),
        ) => {
            let modifier = modifiers.get(index)?;
            let mode = match repeat_type {
                DesignRepeatType::Linear => DesignRepeatMode::Linear(
                    modifier.mode.axis().unwrap_or(DesignRepeatAxis::Horizontal),
                ),
                DesignRepeatType::Radial => DesignRepeatMode::Radial,
            };
            (index, DesignTransformModifierChange::Mode(mode))
        }
        (DesignPanelProperty::TransformRepeatAxis(index), DesignPanelValue::RepeatAxis(axis)) => {
            let modifier = modifiers.get(index)?;
            matches!(modifier.mode, DesignRepeatMode::Linear(_)).then_some(())?;
            (
                index,
                DesignTransformModifierChange::Mode(DesignRepeatMode::Linear(*axis)),
            )
        }
        (DesignPanelProperty::TransformRepeatCount(index), DesignPanelValue::Integer(count)) => (
            index,
            DesignTransformModifierChange::Count(u32::try_from(*count).ok()?.max(1)),
        ),
        (
            DesignPanelProperty::TransformRepeatUnit(index),
            DesignPanelValue::TransformUnit(unit),
        ) => (index, DesignTransformModifierChange::Unit(*unit)),
        (DesignPanelProperty::TransformRepeatOffset(index), DesignPanelValue::Number(offset))
            if offset.is_finite() =>
        {
            (index, DesignTransformModifierChange::Offset(*offset))
        }
        _ => return None,
    };
    let modifier = modifiers.get(index)?;
    Some((modifier.id.clone(), index, change))
}

pub(in super::super) fn transform_modifier_property_with_index(
    property: DesignPanelProperty,
    index: usize,
) -> DesignPanelProperty {
    match property {
        DesignPanelProperty::TransformRepeatType(_) => {
            DesignPanelProperty::TransformRepeatType(index)
        }
        DesignPanelProperty::TransformRepeatAxis(_) => {
            DesignPanelProperty::TransformRepeatAxis(index)
        }
        DesignPanelProperty::TransformRepeatCount(_) => {
            DesignPanelProperty::TransformRepeatCount(index)
        }
        DesignPanelProperty::TransformRepeatUnit(_) => {
            DesignPanelProperty::TransformRepeatUnit(index)
        }
        DesignPanelProperty::TransformRepeatOffset(_) => {
            DesignPanelProperty::TransformRepeatOffset(index)
        }
        property => property,
    }
}

pub(in super::super) fn render_geometry(
    projection: &ShapeProjection,
    chrome: &impl ShapeInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    if !projection.supports_geometry {
        return None;
    }
    let mut rows = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
    let mut has_geometry = false;
    match projection.shape_geometry {
        DesignShapeGeometry::None => {}
        DesignShapeGeometry::Polygon(_)
        | DesignShapeGeometry::Star(_)
        | DesignShapeGeometry::Ellipse(_)
            if !projection.supports_layer =>
        {
            if let Some(shape_controls) =
                render_legacy_shape_geometry_controls(projection, chrome, cx)
            {
                has_geometry = true;
                rows = rows.child(shape_controls);
            }
        }
        DesignShapeGeometry::Polygon(_)
        | DesignShapeGeometry::Star(_)
        | DesignShapeGeometry::Ellipse(_) => {}
        DesignShapeGeometry::Boolean(operation) => {
            has_geometry = true;
            rows = rows.child(chrome.shape_value_cell(
                "boolean-operation",
                "Op",
                operation.label(),
                DesignPanelProperty::BooleanOperation,
                DesignPanelValue::BooleanOperation(DesignBooleanOperation::Subtract),
                cx,
            ));
        }
        DesignShapeGeometry::Table(table) => {
            has_geometry = true;
            rows = rows.child(
                h_flex()
                    .gap_2()
                    .child(chrome.shape_value_cell(
                        "table-rows",
                        "R",
                        format!("{} rows", table.row_count),
                        DesignPanelProperty::TableRows,
                        DesignPanelValue::Integer(i64::from(table.row_count)),
                        cx,
                    ))
                    .child(chrome.shape_value_cell(
                        "table-columns",
                        "C",
                        format!("{} columns", table.column_count),
                        DesignPanelProperty::TableColumns,
                        DesignPanelValue::Integer(i64::from(table.column_count)),
                        cx,
                    )),
            );
        }
    }
    if !projection.supports_mask
        && let Some(mask_type) = projection.effective_mask_type
    {
        has_geometry = true;
        rows = rows
            .child(chrome.shape_toggle_row(
                "is-mask",
                "Use as mask",
                projection.effective_is_mask,
                DesignPanelProperty::IsMask,
                cx,
            ))
            .child(chrome.shape_value_cell(
                "mask-type",
                "Mask",
                mask_type.label(),
                DesignPanelProperty::MaskType,
                DesignPanelValue::MaskType(DesignMaskType::Luminance),
                cx,
            ));
    }
    has_geometry
        .then(|| chrome.shape_section(DesignPanelSection::Geometry, rows.into_any_element(), cx))
}

pub(in super::super) fn render_legacy_shape_geometry_controls(
    projection: &ShapeProjection,
    chrome: &impl ShapeInspectorChrome,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let DesignShapeGeometry::Ellipse(arc) = projection.shape_geometry else {
        return chrome.shape_appearance_controls(cx);
    };
    if !arc.has_inspector_controls() {
        return None;
    }
    Some(
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(chrome.shape_value_cell(
                        "arc-start",
                        "↻",
                        format!("{}°", format_number(arc.starting_degrees())),
                        DesignPanelProperty::ArcStartingAngle,
                        DesignPanelValue::AngleRadians(arc.starting_angle + 15_f32.to_radians()),
                        cx,
                    ))
                    .child(chrome.shape_value_cell(
                        "arc-end",
                        "◔",
                        format!("{}°", format_number(arc.ending_degrees())),
                        DesignPanelProperty::ArcEndingAngle,
                        DesignPanelValue::AngleRadians(arc.ending_angle + 15_f32.to_radians()),
                        cx,
                    )),
            )
            .child(chrome.shape_value_cell(
                "arc-inner",
                "R",
                format!("{}%", format_number(arc.inner_radius * 100.)),
                DesignPanelProperty::ArcInnerRadius,
                DesignPanelValue::Ratio((arc.inner_radius + 0.1).min(1.)),
                cx,
            ))
            .into_any_element(),
    )
}

pub(in super::super) fn render_intent_button(
    projection: &ShapeProjection,
    event_sink: &ShapeEventSink,
    id_suffix: impl Into<SharedString>,
    label: impl Into<SharedString>,
    enabled: bool,
    action: DesignPanelAction,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let id_suffix = id_suffix.into();
    let label = label.into();
    let enabled = enabled && projection.allows_action(&action);
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
        let event_sink = event_sink.clone();
        button = button.on_activate(move |_, _, cx| {
            event_sink.dispatch(action.clone(), cx);
        });
    }
    button.child(label).into_any_element()
}

pub(in super::super) fn render_section_properties(
    projection: &ShapeProjection,
    chrome: &impl ShapeInspectorChrome,
    event_sink: impl Into<ShapeEventSink>,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let event_sink = event_sink.into();
    let section = projection.section.as_ref()?;
    let status_label: SharedString = section.dev_status.as_ref().map_or_else(
        || "No status".into(),
        |status| {
            if status.changed {
                format!("{} · Changed", status.kind.label()).into()
            } else {
                status.kind.label().into()
            }
        },
    );
    let next_status = if section.dev_status.is_some() {
        None
    } else {
        Some(DesignSectionDevStatusKind::ReadyForDev)
    };
    let mut content = v_flex()
        .px(px(PANEL_PADDING))
        .pb_4()
        .gap_2()
        .child(chrome.shape_toggle_row(
            "section-contents-hidden",
            "Hide section contents",
            section.contents_hidden,
            DesignPanelProperty::SectionContentsHidden,
            cx,
        ));
    if section.capabilities.set_dev_status {
        content = content.child(chrome.shape_value_cell(
            "section-dev-status",
            "Status",
            status_label,
            DesignPanelProperty::SectionDevStatus,
            DesignPanelValue::SectionDevStatus(next_status),
            cx,
        ));
    }
    if let Some(description) = section
        .dev_status
        .as_ref()
        .and_then(|status| status.description.clone())
    {
        content = content.child(
            div()
                .px_2()
                .py_1()
                .rounded(px(5.))
                .bg(cx.theme().secondary)
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        );
    }
    let changed = section
        .dev_status
        .as_ref()
        .is_some_and(|status| status.changed);
    content = content.child(
        h_flex()
            .gap_2()
            .when(section.capabilities.share, |row| {
                row.child(render_intent_button(
                    projection,
                    &event_sink,
                    "section-share",
                    "Share",
                    true,
                    DesignPanelAction::SectionShareRequested {
                        node_id: projection.node_id.clone(),
                    },
                    cx,
                ))
            })
            .when(
                changed && section.capabilities.resolve_changed_status,
                |row| {
                    row.child(render_intent_button(
                        projection,
                        &event_sink,
                        "section-resolve-changed",
                        "Resolve changes",
                        projection.can_edit,
                        DesignPanelAction::SectionResolveChangedStatusRequested {
                            node_id: projection.node_id.clone(),
                        },
                        cx,
                    ))
                },
            ),
    );
    Some(chrome.shape_section(DesignPanelSection::Section, content.into_any_element(), cx))
}

pub(in super::super) fn render_transform_modifiers(
    projection: &ShapeProjection,
    chrome: &impl ShapeInspectorChrome,
    event_sink: impl Into<ShapeEventSink>,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let event_sink = event_sink.into();
    if projection.kind != DesignPanelNodeKind::TransformGroup {
        return None;
    }
    let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
    for (index, modifier) in projection.transform_modifiers.iter().enumerate() {
        let repeat_type = modifier.mode.repeat_type();
        let next_type = match repeat_type {
            DesignRepeatType::Linear => DesignRepeatType::Radial,
            DesignRepeatType::Radial => DesignRepeatType::Linear,
        };
        let next_unit = match modifier.unit {
            DesignTransformUnit::Relative => DesignTransformUnit::Pixels,
            DesignTransformUnit::Pixels => DesignTransformUnit::Relative,
        };
        content = content.child(
            v_flex()
                .p_2()
                .gap_2()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .h(px(24.))
                        .justify_between()
                        .child(div().text_xs().font_semibold().child(repeat_type.label()))
                        .child(render_intent_button(
                            projection,
                            &event_sink,
                            format!("transform-remove-{index}"),
                            "Remove",
                            projection.can_edit,
                            DesignPanelAction::TransformModifierRemoveRequested {
                                node_id: projection.node_id.clone(),
                                modifier_id: modifier.id.clone(),
                                index,
                            },
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(chrome.shape_value_cell(
                            format!("transform-type-{index}"),
                            "T",
                            repeat_type.label(),
                            DesignPanelProperty::TransformRepeatType(index),
                            DesignPanelValue::RepeatType(next_type),
                            cx,
                        ))
                        .when_some(modifier.mode.axis(), |row, axis| {
                            row.child(chrome.shape_value_cell(
                                format!("transform-axis-{index}"),
                                "Axis",
                                axis.label(),
                                DesignPanelProperty::TransformRepeatAxis(index),
                                DesignPanelValue::RepeatAxis(match axis {
                                    DesignRepeatAxis::Horizontal => DesignRepeatAxis::Vertical,
                                    DesignRepeatAxis::Vertical => DesignRepeatAxis::Horizontal,
                                }),
                                cx,
                            ))
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(chrome.shape_value_cell(
                            format!("transform-count-{index}"),
                            "#",
                            format!("{} repeats", modifier.count),
                            DesignPanelProperty::TransformRepeatCount(index),
                            DesignPanelValue::Integer(i64::from(modifier.count.saturating_add(1))),
                            cx,
                        ))
                        .child(chrome.shape_value_cell(
                            format!("transform-unit-{index}"),
                            "U",
                            modifier.unit.label(),
                            DesignPanelProperty::TransformRepeatUnit(index),
                            DesignPanelValue::TransformUnit(next_unit),
                            cx,
                        )),
                )
                .child(chrome.shape_value_cell(
                    format!("transform-offset-{index}"),
                    "Offset",
                    format!(
                        "{}{}",
                        format_number(modifier.offset),
                        modifier.unit.suffix()
                    ),
                    DesignPanelProperty::TransformRepeatOffset(index),
                    DesignPanelValue::Number(modifier.offset + 8.),
                    cx,
                )),
        );
    }
    content = content
        .child(
            h_flex()
                .gap_2()
                .child(render_intent_button(
                    projection,
                    &event_sink,
                    "transform-add-linear",
                    "Add linear repeat",
                    projection.can_edit,
                    DesignPanelAction::TransformModifierAddRequested {
                        node_id: projection.node_id.clone(),
                        repeat_type: DesignRepeatType::Linear,
                    },
                    cx,
                ))
                .child(render_intent_button(
                    projection,
                    &event_sink,
                    "transform-add-radial",
                    "Add radial repeat",
                    projection.can_edit,
                    DesignPanelAction::TransformModifierAddRequested {
                        node_id: projection.node_id.clone(),
                        repeat_type: DesignRepeatType::Radial,
                    },
                    cx,
                )),
        )
        .child(render_intent_button(
            projection,
            &event_sink,
            "transform-apply",
            "Apply transforms to selection",
            projection.can_edit && !projection.transform_modifiers.is_empty(),
            DesignPanelAction::ApplyTransformModifiersRequested {
                node_id: projection.node_id.clone(),
            },
            cx,
        ));
    Some(chrome.shape_section(
        DesignPanelSection::Transform,
        content.into_any_element(),
        cx,
    ))
}

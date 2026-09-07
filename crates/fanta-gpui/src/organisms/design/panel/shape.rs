use super::*;

impl DesignPanel {
    pub(super) fn transform_modifier_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<(SharedString, usize, DesignTransformModifierChange)> {
        let (index, change) = match (property, value) {
            (
                DesignPanelProperty::TransformRepeatType(index),
                DesignPanelValue::RepeatType(repeat_type),
            ) => {
                let modifier = self.node.transform_modifiers.get(index)?;
                let mode = match repeat_type {
                    DesignRepeatType::Linear => DesignRepeatMode::Linear(
                        modifier.mode.axis().unwrap_or(DesignRepeatAxis::Horizontal),
                    ),
                    DesignRepeatType::Radial => DesignRepeatMode::Radial,
                };
                (index, DesignTransformModifierChange::Mode(mode))
            }
            (
                DesignPanelProperty::TransformRepeatAxis(index),
                DesignPanelValue::RepeatAxis(axis),
            ) => {
                let modifier = self.node.transform_modifiers.get(index)?;
                matches!(modifier.mode, DesignRepeatMode::Linear(_)).then_some(())?;
                (
                    index,
                    DesignTransformModifierChange::Mode(DesignRepeatMode::Linear(*axis)),
                )
            }
            (
                DesignPanelProperty::TransformRepeatCount(index),
                DesignPanelValue::Integer(count),
            ) => (
                index,
                DesignTransformModifierChange::Count(u32::try_from(*count).ok()?.max(1)),
            ),
            (
                DesignPanelProperty::TransformRepeatUnit(index),
                DesignPanelValue::TransformUnit(unit),
            ) => (index, DesignTransformModifierChange::Unit(*unit)),
            (
                DesignPanelProperty::TransformRepeatOffset(index),
                DesignPanelValue::Number(offset),
            ) if offset.is_finite() => (index, DesignTransformModifierChange::Offset(*offset)),
            _ => return None,
        };
        let modifier = self.node.transform_modifiers.get(index)?;
        Some((modifier.id.clone(), index, change))
    }

    pub(super) fn transform_modifier_property_with_index(
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

    pub(super) fn render_geometry(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.node.supports_section(DesignPanelSection::Geometry) {
            return None;
        }
        let mut rows = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
        let mut has_geometry = false;
        match self.node.shape_geometry {
            DesignShapeGeometry::None => {}
            DesignShapeGeometry::Polygon(_)
            | DesignShapeGeometry::Star(_)
            | DesignShapeGeometry::Ellipse(_)
                if !self.node.supports_section(DesignPanelSection::Layer) =>
            {
                if let Some(shape_controls) = self.render_legacy_shape_geometry_controls(cx) {
                    has_geometry = true;
                    rows = rows.child(shape_controls);
                }
            }
            DesignShapeGeometry::Polygon(_)
            | DesignShapeGeometry::Star(_)
            | DesignShapeGeometry::Ellipse(_) => {}
            DesignShapeGeometry::Boolean(operation) => {
                has_geometry = true;
                rows = rows.child(self.render_value_cell(
                    "boolean-operation",
                    "∩",
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
                        .child(self.render_value_cell(
                            "table-rows",
                            "R",
                            format!("{} rows", table.row_count),
                            DesignPanelProperty::TableRows,
                            DesignPanelValue::Integer(i64::from(table.row_count)),
                            cx,
                        ))
                        .child(self.render_value_cell(
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
        if !self.node.supports_section(DesignPanelSection::Mask)
            && let Some(mask_type) = self.node.effective_mask_type()
        {
            has_geometry = true;
            rows = rows
                .child(self.render_toggle_row(
                    "is-mask",
                    "Use as mask",
                    self.node.effective_is_mask(),
                    DesignPanelProperty::IsMask,
                    cx,
                ))
                .child(self.render_value_cell(
                    "mask-type",
                    "◐",
                    mask_type.label(),
                    DesignPanelProperty::MaskType,
                    DesignPanelValue::MaskType(DesignMaskType::Luminance),
                    cx,
                ));
        }
        has_geometry.then(|| {
            self.render_section(
                DesignPanelSection::Geometry,
                None,
                rows.into_any_element(),
                cx,
            )
        })
    }

    pub(super) fn render_legacy_shape_geometry_controls(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let DesignShapeGeometry::Ellipse(arc) = self.node.shape_geometry else {
            return self.render_shape_appearance_controls(cx);
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
                        .child(self.render_value_cell(
                            "arc-start",
                            "↻",
                            format!("{}°", format_number(arc.starting_degrees())),
                            DesignPanelProperty::ArcStartingAngle,
                            DesignPanelValue::AngleRadians(
                                arc.starting_angle + 15_f32.to_radians(),
                            ),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "arc-end",
                            "◔",
                            format!("{}°", format_number(arc.ending_degrees())),
                            DesignPanelProperty::ArcEndingAngle,
                            DesignPanelValue::AngleRadians(arc.ending_angle + 15_f32.to_radians()),
                            cx,
                        )),
                )
                .child(self.render_value_cell(
                    "arc-inner",
                    "○",
                    format!("{}%", format_number(arc.inner_radius * 100.)),
                    DesignPanelProperty::ArcInnerRadius,
                    DesignPanelValue::Ratio((arc.inner_radius + 0.1).min(1.)),
                    cx,
                ))
                .into_any_element(),
        )
    }

    pub(super) fn render_intent_button(
        &self,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        enabled: bool,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let label = label.into();
        let enabled = enabled && self.node_capability_allows_action(&action);
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
                if this.node_capability_allows_action(&action) {
                    cx.emit_design_panel_action(this, action.clone());
                }
            }));
        }
        button.child(label).into_any_element()
    }

    pub(super) fn render_section_properties(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let section = self.node.section.as_ref()?;
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
        let mut content =
            v_flex()
                .px(px(PANEL_PADDING))
                .pb_4()
                .gap_2()
                .child(self.render_toggle_row(
                    "section-contents-hidden",
                    "Hide section contents",
                    section.contents_hidden,
                    DesignPanelProperty::SectionContentsHidden,
                    cx,
                ));
        if section.capabilities.set_dev_status {
            content = content.child(self.render_value_cell(
                "section-dev-status",
                "✓",
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
                    row.child(self.render_intent_button(
                        "section-share",
                        "Share",
                        true,
                        DesignPanelAction::SectionShareRequested {
                            node_id: self.node.id.clone(),
                        },
                        cx,
                    ))
                })
                .when(
                    changed && section.capabilities.resolve_changed_status,
                    |row| {
                        row.child(self.render_intent_button(
                            "section-resolve-changed",
                            "Resolve changes",
                            self.can_edit(),
                            DesignPanelAction::SectionResolveChangedStatusRequested {
                                node_id: self.node.id.clone(),
                            },
                            cx,
                        ))
                    },
                ),
        );
        Some(self.render_section(
            DesignPanelSection::Section,
            None,
            content.into_any_element(),
            cx,
        ))
    }

    pub(super) fn render_transform_modifiers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.node.kind != DesignPanelNodeKind::TransformGroup {
            return None;
        }
        let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
        for (index, modifier) in self.node.transform_modifiers.iter().enumerate() {
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
                            .child(self.render_intent_button(
                                format!("transform-remove-{index}"),
                                "Remove",
                                self.can_edit(),
                                DesignPanelAction::TransformModifierRemoveRequested {
                                    node_id: self.node.id.clone(),
                                    modifier_id: modifier.id.clone(),
                                    index,
                                },
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.render_value_cell(
                                format!("transform-type-{index}"),
                                "T",
                                repeat_type.label(),
                                DesignPanelProperty::TransformRepeatType(index),
                                DesignPanelValue::RepeatType(next_type),
                                cx,
                            ))
                            .when_some(modifier.mode.axis(), |row, axis| {
                                row.child(self.render_value_cell(
                                    format!("transform-axis-{index}"),
                                    "↔",
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
                            .child(self.render_value_cell(
                                format!("transform-count-{index}"),
                                "#",
                                format!("{} repeats", modifier.count),
                                DesignPanelProperty::TransformRepeatCount(index),
                                DesignPanelValue::Integer(i64::from(
                                    modifier.count.saturating_add(1),
                                )),
                                cx,
                            ))
                            .child(self.render_value_cell(
                                format!("transform-unit-{index}"),
                                "U",
                                modifier.unit.label(),
                                DesignPanelProperty::TransformRepeatUnit(index),
                                DesignPanelValue::TransformUnit(next_unit),
                                cx,
                            )),
                    )
                    .child(self.render_value_cell(
                        format!("transform-offset-{index}"),
                        "↔",
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
                    .child(self.render_intent_button(
                        "transform-add-linear",
                        "Add linear repeat",
                        self.can_edit(),
                        DesignPanelAction::TransformModifierAddRequested {
                            node_id: self.node.id.clone(),
                            repeat_type: DesignRepeatType::Linear,
                        },
                        cx,
                    ))
                    .child(self.render_intent_button(
                        "transform-add-radial",
                        "Add radial repeat",
                        self.can_edit(),
                        DesignPanelAction::TransformModifierAddRequested {
                            node_id: self.node.id.clone(),
                            repeat_type: DesignRepeatType::Radial,
                        },
                        cx,
                    )),
            )
            .child(self.render_intent_button(
                "transform-apply",
                "Apply transforms to selection",
                self.can_edit() && !self.node.transform_modifiers.is_empty(),
                DesignPanelAction::ApplyTransformModifiersRequested {
                    node_id: self.node.id.clone(),
                },
                cx,
            ));
        Some(self.render_section(
            DesignPanelSection::Transform,
            None,
            content.into_any_element(),
            cx,
        ))
    }
}

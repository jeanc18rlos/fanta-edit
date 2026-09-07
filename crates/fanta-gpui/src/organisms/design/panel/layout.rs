use super::*;

impl DesignPanel {
    pub(super) fn is_active_auto_layout_owner(&self) -> bool {
        self.node.supports_auto_layout_container()
            && self
                .node
                .layout
                .as_ref()
                .is_some_and(|layout| layout.mode != DesignLayoutMode::None)
    }

    pub(super) fn add_auto_layout_view_data_for_context(
        &self,
    ) -> Option<&DesignAddAutoLayoutViewData> {
        let view_data = self.add_auto_layout_view_data.as_ref()?;
        (view_data.is_valid()
            && view_data.structurally_eligible
            && view_data.target == self.command_target()
            && !self.inspection_context.selection().is_empty()
            && self.node.supports_add_auto_layout()
            && self.node.supports_section(DesignPanelSection::Layout)
            && !self.is_active_auto_layout_owner())
        .then_some(view_data)
    }

    pub(super) fn emit_add_auto_layout(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.can_edit() {
            return false;
        }
        let target = self
            .add_auto_layout_view_data_for_context()
            .filter(|view_data| view_data.can_request())
            .map(|view_data| view_data.target.clone());
        let Some(target) = target else {
            return false;
        };
        cx.emit_design_panel_action(self, DesignPanelAction::AddAutoLayoutRequested { target });
        true
    }

    pub(super) fn frame_preset_view_data_for_context(&self) -> Option<&DesignFramePresetViewData> {
        let view_data = self.frame_preset_view_data.as_ref()?;
        (self.inspection_context.selection().kind() == DesignPanelSelectionKind::Single
            && self.node.kind == DesignPanelNodeKind::Frame
            && self.node.supports_dimensions()
            && self.node.supports_section(DesignPanelSection::Layout)
            && view_data.target_node_id == self.node.id
            && view_data.is_valid())
        .then_some(view_data)
    }

    pub(super) fn frame_preset_action_is_applicable(
        &self,
        selection: &DesignFramePresetSelection,
        width: f32,
        height: f32,
    ) -> bool {
        if !self.can_edit()
            || !self.property_is_editable(DesignPanelProperty::Width)
            || !self.property_is_editable(DesignPanelProperty::Height)
        {
            return false;
        }
        self.frame_preset_view_data_for_context()
            .filter(|view_data| view_data.can_apply(selection))
            .and_then(|view_data| view_data.preset(selection))
            .is_some_and(|(_, preset)| preset.width == width && preset.height == height)
    }

    pub(super) fn toggle_frame_preset_group(
        &mut self,
        group_id: &SharedString,
        cx: &mut Context<Self>,
    ) {
        let group_exists = self
            .frame_preset_view_data_for_context()
            .is_some_and(|view_data| view_data.groups.iter().any(|group| group.id == *group_id));
        if !group_exists {
            return;
        }
        if !self.collapsed_frame_preset_groups.remove(group_id) {
            self.collapsed_frame_preset_groups.insert(group_id.clone());
        }
        cx.notify();
    }

    pub(super) fn emit_frame_preset_apply(
        &mut self,
        selection: DesignFramePresetSelection,
        cx: &mut Context<Self>,
    ) {
        let dimensions = self
            .frame_preset_view_data_for_context()
            .and_then(|view_data| view_data.preset(&selection))
            .map(|(_, preset)| (preset.width, preset.height));
        let Some((width, height)) = dimensions else {
            return;
        };
        if !self.frame_preset_action_is_applicable(&selection, width, height) {
            return;
        }
        self.frame_preset_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::FramePresetApplyRequested {
                node_id: self.node.id.clone(),
                selection,
                width,
                height,
            },
        );
        cx.notify();
    }

    pub(super) fn layout_property_is_applicable(&self, property: DesignPanelProperty) -> bool {
        let layout = self.node.layout.as_ref();
        let mode = layout.map_or(DesignLayoutMode::None, |layout| layout.mode);
        let owns_layout = self.node.supports_auto_layout_container();
        let parent = self.inspection_context.parent_layout();
        let participates =
            self.node.supports_auto_layout_child() && parent.participates_in_auto_layout();
        let parent_direction = parent.auto_layout_direction();
        match property {
            DesignPanelProperty::LayoutMode => owns_layout,
            DesignPanelProperty::HorizontalSizing | DesignPanelProperty::VerticalSizing => {
                (owns_layout && mode != DesignLayoutMode::None) || participates
            }
            DesignPanelProperty::AutoLayoutAlignment
            | DesignPanelProperty::AlignmentX
            | DesignPanelProperty::AlignmentY => {
                owns_layout
                    && matches!(
                        mode,
                        DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical
                    )
            }
            DesignPanelProperty::Wrap => owns_layout && mode == DesignLayoutMode::Horizontal,
            DesignPanelProperty::Gap => owns_layout && mode != DesignLayoutMode::None,
            DesignPanelProperty::ItemSpacingMode => {
                owns_layout
                    && matches!(
                        mode,
                        DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical
                    )
            }
            DesignPanelProperty::CounterAxisAlignContent => {
                owns_layout
                    && mode == DesignLayoutMode::Horizontal
                    && layout.is_some_and(|layout| layout.wrap)
            }
            DesignPanelProperty::CounterAxisGap => {
                owns_layout
                    && (mode == DesignLayoutMode::Grid
                        || (mode == DesignLayoutMode::Horizontal
                            && layout.is_some_and(|layout| {
                                layout.wrap
                                    && layout.counter_axis_align_content
                                        == DesignCounterAxisAlignContent::Auto
                            })))
            }
            DesignPanelProperty::PaddingVertical
            | DesignPanelProperty::PaddingHorizontal
            | DesignPanelProperty::PaddingShorthand
            | DesignPanelProperty::PaddingTop
            | DesignPanelProperty::PaddingRight
            | DesignPanelProperty::PaddingBottom
            | DesignPanelProperty::PaddingLeft
            | DesignPanelProperty::IncludeStrokes => owns_layout && mode != DesignLayoutMode::None,
            DesignPanelProperty::StackingOrder => {
                owns_layout
                    && matches!(
                        mode,
                        DesignLayoutMode::Horizontal | DesignLayoutMode::Vertical
                    )
            }
            DesignPanelProperty::BaselineAlignment => {
                owns_layout && mode == DesignLayoutMode::Horizontal
            }
            DesignPanelProperty::MinWidth
            | DesignPanelProperty::MaxWidth
            | DesignPanelProperty::MinHeight
            | DesignPanelProperty::MaxHeight => {
                (owns_layout && mode != DesignLayoutMode::None) || participates
            }
            DesignPanelProperty::LayoutPositioning => {
                self.node.supports_auto_layout_child() && parent.is_auto_layout()
            }
            DesignPanelProperty::LayoutAlignSelf | DesignPanelProperty::LayoutGrow => participates,
            DesignPanelProperty::GridAutoTracks
            | DesignPanelProperty::GridItemsPositioning
            | DesignPanelProperty::GridColumnCount
            | DesignPanelProperty::GridRowCount
            | DesignPanelProperty::GridColumnTrack(_)
            | DesignPanelProperty::GridRowTrack(_)
            | DesignPanelProperty::GridColumnTrackValue(_)
            | DesignPanelProperty::GridRowTrackValue(_) => {
                owns_layout
                    && self.node.supports_grid_auto_layout()
                    && mode == DesignLayoutMode::Grid
            }
            DesignPanelProperty::GridRowIndex
            | DesignPanelProperty::GridColumnIndex
            | DesignPanelProperty::GridRowSpan
            | DesignPanelProperty::GridColumnSpan
            | DesignPanelProperty::GridHorizontalAlignment
            | DesignPanelProperty::GridVerticalAlignment => {
                participates && parent_direction == Some(DesignPanelAutoLayoutDirection::Grid)
            }
            DesignPanelProperty::ClipContent => self.node.supports_clip_content(),
            _ => true,
        }
    }

    pub(super) fn layout_property_value_is_applicable(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> bool {
        let mode = self
            .node
            .layout
            .as_ref()
            .map_or(DesignLayoutMode::None, |layout| layout.mode);
        match (property, value) {
            (DesignPanelProperty::FontWeight, DesignPanelValue::Number(weight)) => {
                weight.is_finite() && (1. ..=1000.).contains(weight)
            }
            (DesignPanelProperty::FontWeight, _) => false,
            (DesignPanelProperty::ListSpacing, DesignPanelValue::Number(spacing)) => {
                spacing.is_finite() && *spacing >= 0.
            }
            (
                DesignPanelProperty::TextDecorationStyle,
                DesignPanelValue::TextDecorationStyle(_),
            )
            | (
                DesignPanelProperty::TextDecorationColor,
                DesignPanelValue::TextDecorationColor(_),
            )
            | (DesignPanelProperty::TextDecorationSkipInk, DesignPanelValue::Bool(_)) => true,
            (
                DesignPanelProperty::TextDecorationOffset,
                DesignPanelValue::TextDecorationMetric(metric),
            ) => metric.is_finite(),
            (
                DesignPanelProperty::TextDecorationThickness,
                DesignPanelValue::TextDecorationMetric(metric),
            ) => metric.is_non_negative(),
            (
                DesignPanelProperty::ListSpacing
                | DesignPanelProperty::TextDecorationStyle
                | DesignPanelProperty::TextDecorationOffset
                | DesignPanelProperty::TextDecorationThickness
                | DesignPanelProperty::TextDecorationColor
                | DesignPanelProperty::TextDecorationSkipInk,
                _,
            ) => false,
            (DesignPanelProperty::LayoutMode, DesignPanelValue::LayoutMode(candidate)) => {
                (*candidate == DesignLayoutMode::None || self.node.supports_auto_layout_container())
                    && (*candidate != DesignLayoutMode::Grid
                        || self.node.supports_grid_auto_layout())
            }
            (DesignPanelProperty::Wrap, DesignPanelValue::Bool(true)) => {
                mode == DesignLayoutMode::Horizontal
            }
            (DesignPanelProperty::Gap, DesignPanelValue::Number(gap)) => {
                gap.is_finite() && (mode != DesignLayoutMode::Grid || *gap >= 0.)
            }
            (
                DesignPanelProperty::PaddingVertical
                | DesignPanelProperty::PaddingHorizontal
                | DesignPanelProperty::PaddingTop
                | DesignPanelProperty::PaddingRight
                | DesignPanelProperty::PaddingBottom
                | DesignPanelProperty::PaddingLeft,
                DesignPanelValue::Number(padding),
            ) => padding.is_finite() && *padding >= 0.,
            (DesignPanelProperty::PaddingShorthand, DesignPanelValue::NumberList(padding)) => {
                (1..=4).contains(&padding.len())
                    && padding
                        .iter()
                        .all(|value| value.is_finite() && *value >= 0.)
            }
            (
                DesignPanelProperty::PaddingVertical
                | DesignPanelProperty::PaddingHorizontal
                | DesignPanelProperty::PaddingShorthand
                | DesignPanelProperty::PaddingTop
                | DesignPanelProperty::PaddingRight
                | DesignPanelProperty::PaddingBottom
                | DesignPanelProperty::PaddingLeft,
                _,
            ) => false,
            (
                DesignPanelProperty::CounterAxisAlignContent,
                DesignPanelValue::CounterAxisAlignContent(_),
            ) => {
                mode == DesignLayoutMode::Horizontal
                    && self.node.layout.as_ref().is_some_and(|layout| layout.wrap)
            }
            (DesignPanelProperty::CounterAxisAlignContent, _) => false,
            (DesignPanelProperty::CounterAxisGap, DesignPanelValue::OptionalNumber(spacing)) => {
                self.node.layout.as_ref().is_some_and(|layout| {
                    if layout.mode == DesignLayoutMode::Grid {
                        spacing.is_some_and(|spacing| spacing.is_finite() && spacing >= 0.)
                    } else {
                        layout.mode == DesignLayoutMode::Horizontal
                            && layout.wrap
                            && layout.counter_axis_align_content
                                == DesignCounterAxisAlignContent::Auto
                            && spacing.is_none_or(|spacing| spacing.is_finite() && spacing > 0.)
                    }
                })
            }
            (DesignPanelProperty::CounterAxisGap, _) => false,
            (
                DesignPanelProperty::HorizontalSizing | DesignPanelProperty::VerticalSizing,
                DesignPanelValue::SizingMode(candidate),
            ) => {
                let layout = self.node.layout.as_ref();
                let owns_active_flow =
                    self.node.supports_auto_layout_container() && mode != DesignLayoutMode::None;
                let is_text = matches!(
                    self.node.kind,
                    DesignPanelNodeKind::Text | DesignPanelNodeKind::TextPath
                );
                let participates = self.node.supports_auto_layout_child()
                    && self
                        .inspection_context
                        .parent_layout()
                        .participates_in_auto_layout();
                let requires_fill = layout.is_some_and(|layout| match property {
                    DesignPanelProperty::HorizontalSizing => layout.item.grid_column_span > 1,
                    DesignPanelProperty::VerticalSizing => layout.item.grid_row_span > 1,
                    _ => false,
                }) && self
                    .inspection_context
                    .parent_layout()
                    .auto_layout_direction()
                    == Some(DesignPanelAutoLayoutDirection::Grid);
                let has_fraction_track_on_hug_axis = mode == DesignLayoutMode::Grid
                    && layout.is_some_and(|layout| match property {
                        DesignPanelProperty::HorizontalSizing => layout
                            .grid_columns
                            .iter()
                            .any(|track| track.sizing == DesignGridTrackSizing::Fraction),
                        DesignPanelProperty::VerticalSizing => layout
                            .grid_rows
                            .iter()
                            .any(|track| track.sizing == DesignGridTrackSizing::Fraction),
                        _ => false,
                    });
                match candidate {
                    DesignSizingMode::Fixed => !requires_fill,
                    DesignSizingMode::Hug => {
                        !requires_fill
                            && !has_fraction_track_on_hug_axis
                            && (owns_active_flow || is_text)
                    }
                    DesignSizingMode::Fill => participates,
                }
            }
            (DesignPanelProperty::LayoutGrow, DesignPanelValue::Number(grow)) => {
                grow.is_finite() && (*grow == 0. || *grow == 1.)
            }
            (
                DesignPanelProperty::GridColumnCount | DesignPanelProperty::GridRowCount,
                DesignPanelValue::Integer(count),
            ) => *count >= 1,
            (
                property @ (DesignPanelProperty::GridColumnTrack(_)
                | DesignPanelProperty::GridRowTrack(_)),
                DesignPanelValue::GridTrack(track),
            ) => {
                track.is_valid()
                    && !(track.sizing == DesignGridTrackSizing::Fraction
                        && self
                            .node
                            .layout
                            .as_ref()
                            .is_some_and(|layout| match property {
                                DesignPanelProperty::GridColumnTrack(_) => {
                                    layout.horizontal_sizing == DesignSizingMode::Hug
                                }
                                DesignPanelProperty::GridRowTrack(_) => {
                                    layout.vertical_sizing == DesignSizingMode::Hug
                                }
                                _ => false,
                            }))
            }
            (
                DesignPanelProperty::GridColumnTrackValue(_)
                | DesignPanelProperty::GridRowTrackValue(_),
                DesignPanelValue::Number(value),
            ) => value.is_finite() && *value > 0.,
            (DesignPanelProperty::MinWidth, DesignPanelValue::OptionalNumber(Some(minimum))) => {
                minimum.is_finite()
                    && *minimum > 0.
                    && self
                        .node
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.item.max_width)
                        .is_none_or(|maximum| *minimum <= maximum)
            }
            (DesignPanelProperty::MinHeight, DesignPanelValue::OptionalNumber(Some(minimum))) => {
                minimum.is_finite()
                    && *minimum > 0.
                    && self
                        .node
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.item.max_height)
                        .is_none_or(|maximum| *minimum <= maximum)
            }
            (DesignPanelProperty::MaxWidth, DesignPanelValue::OptionalNumber(Some(maximum))) => {
                maximum.is_finite()
                    && *maximum > 0.
                    && self
                        .node
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.item.min_width)
                        .is_none_or(|minimum| minimum <= *maximum)
            }
            (DesignPanelProperty::MaxHeight, DesignPanelValue::OptionalNumber(Some(maximum))) => {
                maximum.is_finite()
                    && *maximum > 0.
                    && self
                        .node
                        .layout
                        .as_ref()
                        .and_then(|layout| layout.item.min_height)
                        .is_none_or(|minimum| minimum <= *maximum)
            }
            (
                DesignPanelProperty::GridRowSpan | DesignPanelProperty::GridColumnSpan,
                DesignPanelValue::Integer(span),
            ) => {
                let fills_spanned_axis =
                    self.node
                        .layout
                        .as_ref()
                        .is_some_and(|layout| match property {
                            DesignPanelProperty::GridRowSpan => {
                                *span <= 1 || layout.vertical_sizing == DesignSizingMode::Fill
                            }
                            DesignPanelProperty::GridColumnSpan => {
                                *span <= 1 || layout.horizontal_sizing == DesignSizingMode::Fill
                            }
                            _ => false,
                        });
                *span >= 1 && fills_spanned_axis
            }
            (DesignPanelProperty::EffectKind(index), DesignPanelValue::EffectKind(candidate)) => {
                self.node.effects.get(index).is_some_and(|effect| {
                    effect.settings.kind() == *candidate
                        || self.node.can_use_effect_kind(*candidate, Some(index))
                })
            }
            (
                DesignPanelProperty::EffectShaderProperty(index, property_index),
                DesignPanelValue::ShaderProperty(value),
            ) => self
                .node
                .effects
                .get(index)
                .and_then(|effect| {
                    let DesignEffectSettings::Shader(shader) = &effect.settings else {
                        return None;
                    };
                    shader.properties.get(property_index)
                })
                .is_some_and(|property| {
                    !property.read_only
                        && !matches!(
                            &property.value,
                            DesignShaderPropertyValue::VariableAlias { .. }
                                | DesignShaderPropertyValue::Opaque { .. }
                        )
                        && value.is_compatible_with(property.kind)
                }),
            (DesignPanelProperty::StrokeDashMode, DesignPanelValue::StrokeDashMode(_)) => true,
            (DesignPanelProperty::StrokeDashPattern, DesignPanelValue::NumberList(pattern)) => {
                self.node.stroke.as_ref().is_some_and(|stroke| {
                    let mut candidate = stroke.dashes.clone();
                    candidate.set_pattern(pattern.clone())
                })
            }
            (
                DesignPanelProperty::StrokeVariableWidth,
                DesignPanelValue::StrokeVariableWidth(variable_width),
            ) => variable_width
                .as_ref()
                .is_none_or(DesignVariableWidthStroke::is_valid),
            (
                DesignPanelProperty::StrokeVariableWidthPointPosition(_),
                DesignPanelValue::Number(position),
            ) => position.is_finite() && (0. ..=1.).contains(position),
            (
                DesignPanelProperty::StrokeVariableWidthPointWidth(_),
                DesignPanelValue::Number(width),
            ) => width.is_finite() && *width >= 0.,
            (DesignPanelProperty::StrokeType, DesignPanelValue::StrokeType(kind)) => {
                *kind != DesignStrokeType::Opaque
            }
            (DesignPanelProperty::StrokeStretchBrush, DesignPanelValue::StrokeStretchBrush(_))
            | (
                DesignPanelProperty::StrokeBrushDirection,
                DesignPanelValue::StrokeBrushDirection(_),
            )
            | (DesignPanelProperty::StrokeScatterBrush, DesignPanelValue::StrokeScatterBrush(_)) => {
                true
            }
            (DesignPanelProperty::StrokeScatterGap, DesignPanelValue::Number(value)) => {
                value.is_finite() && *value >= 0.25
            }
            (DesignPanelProperty::StrokeScatterWiggle, DesignPanelValue::Number(value)) => {
                value.is_finite() && *value >= 0.
            }
            (DesignPanelProperty::StrokeScatterSizeJitter, DesignPanelValue::Number(value)) => {
                value.is_finite() && (0. ..=3.).contains(value)
            }
            (
                DesignPanelProperty::StrokeScatterAngularJitter
                | DesignPanelProperty::StrokeScatterRotation,
                DesignPanelValue::Number(value),
            ) => value.is_finite() && (-180. ..=180.).contains(value),
            (DesignPanelProperty::StrokeDynamicFrequency, DesignPanelValue::Number(value)) => {
                value.is_finite() && (0.01..=20.).contains(value)
            }
            (DesignPanelProperty::StrokeDynamicWiggle, DesignPanelValue::Number(value)) => {
                value.is_finite() && *value >= 0.
            }
            (DesignPanelProperty::StrokeDynamicSmoothen, DesignPanelValue::Number(value)) => {
                value.is_finite() && (0. ..=1.).contains(value)
            }
            (
                DesignPanelProperty::StrokeDashMode
                | DesignPanelProperty::StrokeDashPattern
                | DesignPanelProperty::StrokeVariableWidth
                | DesignPanelProperty::StrokeVariableWidthPointPosition(_)
                | DesignPanelProperty::StrokeVariableWidthPointWidth(_)
                | DesignPanelProperty::StrokeType
                | DesignPanelProperty::StrokeStretchBrush
                | DesignPanelProperty::StrokeBrushDirection
                | DesignPanelProperty::StrokeScatterBrush
                | DesignPanelProperty::StrokeScatterGap
                | DesignPanelProperty::StrokeScatterWiggle
                | DesignPanelProperty::StrokeScatterSizeJitter
                | DesignPanelProperty::StrokeScatterAngularJitter
                | DesignPanelProperty::StrokeScatterRotation
                | DesignPanelProperty::StrokeDynamicFrequency
                | DesignPanelProperty::StrokeDynamicWiggle
                | DesignPanelProperty::StrokeDynamicSmoothen,
                _,
            ) => false,
            _ => true,
        }
    }

    pub(super) fn render_grid_auto_rows_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let checked = self
            .node
            .layout
            .as_ref()
            .is_some_and(|layout| layout.grid_auto_tracks == DesignGridAutoTracks::Rows);
        let property = DesignPanelProperty::GridAutoTracks;
        let next = if checked {
            DesignGridAutoTracks::None
        } else {
            DesignGridAutoTracks::Rows
        };
        let editable = self.property_is_editable(property);
        let mut row = h_flex()
            .id(SharedString::from(format!("{}-grid-auto-rows", self.id)))
            .h(px(ROW_HEIGHT))
            .w_full()
            .justify_between()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .when(editable, |row| {
                row.key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!editable, |row| row.opacity(0.62));
        if editable {
            row = row.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property(property, DesignPanelValue::GridAutoTracks(next), cx);
            }));
        }
        row.child(div().text_xs().child("Auto rows"))
            .child(
                h_flex()
                    .w(px(30.))
                    .h(px(18.))
                    .p(px(2.))
                    .justify_end()
                    .when(!checked, |toggle| toggle.justify_start())
                    .rounded(px(9.))
                    .bg(if checked {
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

    pub(super) fn render_counter_axis_spacing_controls(
        &self,
        spacing: Option<f32>,
        primary_gap: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(DesignPanelProperty::CounterAxisGap);
        let custom = spacing.is_some();
        let next_spacing = if custom {
            None
        } else {
            Some(primary_gap.abs().max(1.))
        };
        let toggle_value = DesignPanelValue::OptionalNumber(next_spacing);
        let toggle = h_flex()
            .id(SharedString::from(format!(
                "{}-layout-counter-axis-spacing-mode",
                self.id
            )))
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .px_2()
            .gap_1()
            .rounded(px(4.))
            .border_1()
            .border_color(if custom {
                cx.theme().selection
            } else {
                cx.theme().transparent
            })
            .bg(cx.theme().secondary)
            .when(editable, |control| {
                control
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.border_color(cx.theme().muted_foreground))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
                    .on_activate(cx.listener(move |this, _, _, cx| {
                        this.emit_property(
                            DesignPanelProperty::CounterAxisGap,
                            toggle_value.clone(),
                            cx,
                        );
                    }))
            })
            .when(!editable, |control| control.opacity(0.62))
            .child(
                div()
                    .w(px(12.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if custom { "⛓" } else { "↕" }),
            )
            .child(div().flex_1().truncate().text_xs().child(if custom {
                "Custom"
            } else {
                "Linked to gap"
            }));

        h_flex()
            .w_full()
            .gap_2()
            .child(toggle)
            .when_some(spacing, |row, spacing| {
                row.child(self.render_value_cell(
                    "layout-counter-axis-spacing-value",
                    "↕",
                    format_number(spacing),
                    DesignPanelProperty::CounterAxisGap,
                    DesignPanelValue::OptionalNumber(Some(spacing + 2.)),
                    cx,
                ))
            })
            .into_any_element()
    }

    pub(super) fn render_padding_mode_button(
        &self,
        mode: PaddingEditorMode,
        label: &'static str,
        tooltip: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        Button::new(SharedString::from(format!(
            "{}-padding-mode-{}",
            self.id,
            label.to_ascii_lowercase()
        )))
        .label(label)
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .h(px(22.))
        .selected(self.padding_editor_mode == mode)
        .disabled(!self.can_edit())
        .on_activate(move |_, _, cx| {
            panel.update(cx, |this, cx| {
                this.padding_editor_mode = mode;
                cx.notify();
            });
        })
        .into_any_element()
    }

    pub(super) fn open_grid_dimensions_picker(&mut self, cx: &mut Context<Self>) {
        let Some(dimensions) = self
            .node
            .layout
            .as_ref()
            .and_then(DesignLayout::grid_dimensions)
            .filter(|_| {
                self.property_is_editable(DesignPanelProperty::GridColumnCount)
                    || self.property_is_editable(DesignPanelProperty::GridRowCount)
            })
        else {
            self.grid_dimensions_picker = None;
            return;
        };
        self.cancel_property_editor_transaction(cx);
        self.grid_dimensions_picker = Some(GridDimensionsPicker {
            node_id: self.node.id.clone(),
            candidate: dimensions,
        });
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.auxiliary_color_picker = None;
        self.active_effect_settings = None;
        self.paint_style_browser_open = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.layout_grid_style_browser_open = false;
        self.frame_preset_browser_open = false;
        self.typography_style_picker_open = false;
        self.font_browser_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        cx.notify();
    }

    pub(super) fn set_grid_dimensions_candidate(
        &mut self,
        candidate: DesignGridDimensions,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(node_id) = self
            .grid_dimensions_picker
            .as_ref()
            .map(|picker| picker.node_id.clone())
        else {
            return false;
        };
        if node_id != self.node.id || !candidate.is_valid() {
            return false;
        }
        let action = DesignPanelAction::GridDimensionsEditRequested {
            node_id,
            dimensions: candidate,
            phase: DesignPanelEditPhase::Commit,
        };
        if !self.grid_dimensions_action_is_applicable(&action) {
            return false;
        }
        let picker = self
            .grid_dimensions_picker
            .as_mut()
            .expect("validated retained Grid picker remains present");
        if picker.candidate != candidate {
            picker.candidate = candidate;
            cx.notify();
        }
        true
    }

    pub(super) fn commit_grid_dimensions_candidate(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picker) = self.grid_dimensions_picker.clone() else {
            return false;
        };
        let action = DesignPanelAction::GridDimensionsEditRequested {
            node_id: picker.node_id,
            dimensions: picker.candidate,
            phase: DesignPanelEditPhase::Commit,
        };
        if !self.grid_dimensions_action_is_applicable(&action) {
            return false;
        }
        self.grid_dimensions_picker = None;
        cx.emit_design_panel_action(self, action);
        cx.notify();
        true
    }

    pub(super) fn step_grid_dimensions_candidate(
        &mut self,
        key: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = self.grid_dimensions_picker.as_ref() else {
            return false;
        };
        let candidate = picker.candidate;
        let automatic_rows = self
            .node
            .layout
            .as_ref()
            .is_some_and(|layout| layout.grid_auto_tracks == DesignGridAutoTracks::Rows);
        let next = match key {
            "left" => DesignGridDimensions {
                columns: candidate.columns.saturating_sub(1).max(1),
                ..candidate
            },
            "right" => DesignGridDimensions {
                columns: candidate.columns.saturating_add(1),
                ..candidate
            },
            "up" if !automatic_rows => DesignGridDimensions {
                rows: candidate.rows.saturating_sub(1).max(1),
                ..candidate
            },
            "down" if !automatic_rows => DesignGridDimensions {
                rows: candidate.rows.saturating_add(1),
                ..candidate
            },
            "up" | "down" => return false,
            _ => return false,
        };
        self.set_grid_dimensions_candidate(next, cx)
    }

    pub(super) fn handle_grid_dimensions_picker_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Dispatched Enter/Space commit through the bound `ActivateControl`
        // command on the keyboard surface before this handler runs; the arms
        // below stay for the roving arrows, dismissal, and direct invocation.
        let key = event.keystroke.key.as_str();
        let handled = match key {
            "left" | "right" | "up" | "down" => self.step_grid_dimensions_candidate(key, cx),
            "enter" | "space" => self.commit_grid_dimensions_candidate(cx),
            "escape" => {
                let was_open = self.grid_dimensions_picker.take().is_some();
                if was_open {
                    cx.notify();
                }
                was_open
            }
            _ => false,
        };
        if handled {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    pub(super) fn render_grid_dimensions_picker(
        &self,
        layout: &DesignLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const SELECTOR_COLUMNS: usize = 8;
        const SELECTOR_ROWS: usize = 6;

        let Some(current) = layout.grid_dimensions() else {
            return Button::new(SharedString::from(format!(
                "{}-grid-dimensions-incomplete",
                self.id
            )))
            .label(SharedString::from(format!(
                "{} × {}",
                layout.grid_columns.len(),
                layout.grid_rows.len()
            )))
            .tooltip("Waiting for complete positive Grid track vectors from the host")
            .dropdown_caret(true)
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(true)
            .into_any_element();
        };
        let open = self
            .grid_dimensions_picker
            .as_ref()
            .is_some_and(|picker| picker.node_id == self.node.id && picker.candidate.is_valid());
        let candidate = self
            .grid_dimensions_picker
            .as_ref()
            .filter(|picker| picker.node_id == self.node.id)
            .map_or(current, |picker| picker.candidate);
        let automatic_rows = layout.grid_auto_tracks == DesignGridAutoTracks::Rows;
        let columns_editable = self.property_is_editable(DesignPanelProperty::GridColumnCount);
        let rows_editable = self.property_is_editable(DesignPanelProperty::GridRowCount);
        let can_edit = columns_editable || rows_editable;
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let trigger = Button::new(SharedString::from(format!(
            "{}-grid-dimensions-picker",
            self.id
        )))
        .label(SharedString::from(format!(
            "{} × {}",
            current.columns, current.rows
        )))
        .tooltip(if automatic_rows {
            "Choose columns; rows are derived while Auto rows is on"
        } else {
            "Choose Number of columns and Number of rows"
        })
        .dropdown_caret(true)
        .xsmall()
        .compact()
        .ghost()
        .w_full()
        .h(px(ROW_HEIGHT))
        .selected(open)
        .disabled(!can_edit)
        .on_keyboard_activate({
            let panel = panel_for_open.clone();
            move |_, cx| {
                panel.update(cx, |this, cx| {
                    if open {
                        if this.grid_dimensions_picker.is_some() {
                            this.grid_dimensions_picker = None;
                            cx.notify();
                        }
                    } else {
                        this.open_grid_dimensions_picker(cx);
                    }
                });
            }
        });

        Popover::new(SharedString::from(format!(
            "{}-grid-dimensions-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .on_open_change(move |is_open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *is_open {
                    this.open_grid_dimensions_picker(cx);
                } else if this.grid_dimensions_picker.is_some() {
                    this.grid_dimensions_picker = None;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, _, cx| {
            let panel_for_keyboard = panel_for_content.clone();
            let panel_for_commit = panel_for_content.clone();
            let mut selector = v_flex()
                .id("grid-dimensions-keyboard-surface")
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .w_full()
                .gap_1()
                .p_1()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .focus(|style| style.border_color(cx.theme().selection))
                .on_action(move |_: &ActivateControl, _, cx| {
                    panel_for_commit.update(cx, |this, cx| {
                        this.commit_grid_dimensions_candidate(cx);
                    });
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    panel_for_keyboard.update(cx, |this, cx| {
                        this.handle_grid_dimensions_picker_key_down(event, window, cx);
                    });
                });
            let selector_rows = if automatic_rows { 1 } else { SELECTOR_ROWS };
            for visual_row in 1..=selector_rows {
                let mut cells = h_flex().w_full().gap_1();
                for column in 1..=SELECTOR_COLUMNS {
                    let row = if automatic_rows {
                        current.rows
                    } else {
                        visual_row
                    };
                    let dimensions = DesignGridDimensions {
                        columns: column,
                        rows: row,
                    };
                    let cell_is_editable = (column == current.columns || columns_editable)
                        && (row == current.rows || rows_editable);
                    let highlighted =
                        column <= candidate.columns && (automatic_rows || row <= candidate.rows);
                    let panel_for_hover = panel_for_content.clone();
                    let panel_for_click = panel_for_content.clone();
                    let mut cell = div()
                        .id(SharedString::from(format!(
                            "grid-dimensions-{column}-{visual_row}"
                        )))
                        .h(px(18.))
                        .flex_1()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(if highlighted {
                            cx.theme().selection
                        } else {
                            cx.theme().border
                        })
                        .bg(if highlighted {
                            cx.theme().selection.opacity(0.28)
                        } else {
                            cx.theme().secondary
                        })
                        .when(!cell_is_editable, |cell| cell.opacity(0.3));
                    if cell_is_editable {
                        cell = cell
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .on_mouse_move(move |_: &MouseMoveEvent, _, cx| {
                                panel_for_hover.update(cx, |this, cx| {
                                    this.set_grid_dimensions_candidate(dimensions, cx);
                                });
                            })
                            .on_click(move |event: &ClickEvent, _, cx| {
                                if !event.is_keyboard() {
                                    panel_for_click.update(cx, |this, cx| {
                                        if this.set_grid_dimensions_candidate(dimensions, cx) {
                                            this.commit_grid_dimensions_candidate(cx);
                                        }
                                    });
                                }
                            });
                    }
                    cells = cells.child(cell);
                }
                selector = selector.child(cells);
            }

            v_flex()
                .w(px(GRID_DIMENSIONS_POPOVER_WIDTH))
                .gap_2()
                .p_2()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(div().text_xs().font_semibold().child("Grid dimensions"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} × {}", candidate.columns, candidate.rows)),
                        ),
                )
                .child(selector)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if automatic_rows {
                            "Rows are derived while Auto rows is on. Use ←/→, then Enter or Space."
                        } else {
                            "Use arrow keys, then Enter or Space to apply."
                        }),
                )
        })
        .into_any_element()
    }

    pub(super) fn render_grid_track_controls(
        &self,
        axis: DesignGridTrackAxis,
        layout: &DesignLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tracks = layout.grid_tracks(axis);
        let (label, short_label, count_property) = match axis {
            DesignGridTrackAxis::Column => (
                "Number of columns",
                "C",
                DesignPanelProperty::GridColumnCount,
            ),
            DesignGridTrackAxis::Row => ("Number of rows", "R", DesignPanelProperty::GridRowCount),
        };
        let can_change_count = self.can_edit() && layout.grid_track_count_is_editable(axis);
        let mut controls = v_flex().w_full().gap_1().child(
            h_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .w(px(112.))
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(self.render_value_cell(
                    format!("grid-{}-count", label.to_ascii_lowercase()),
                    "#",
                    tracks.len().to_string(),
                    count_property,
                    DesignPanelValue::Integer(
                        i64::try_from(tracks.len().saturating_add(1)).unwrap_or(i64::MAX),
                    ),
                    cx,
                ))
                .child(self.render_grid_track_action_button(
                    format!("grid-{}-add", label.to_ascii_lowercase()),
                    IconName::Plus,
                    DesignPanelAction::GridTrackAddRequested {
                        node_id: self.node.id.clone(),
                        axis,
                        insertion_index: tracks.len(),
                    },
                    can_change_count,
                    cx,
                )),
        );
        if axis == DesignGridTrackAxis::Row && layout.grid_auto_tracks == DesignGridAutoTracks::Rows
        {
            controls = controls.child(
                div()
                    .pl(px(112.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Derived by Auto rows"),
            );
        }

        for (index, track) in tracks.iter().copied().enumerate() {
            let track_property = match axis {
                DesignGridTrackAxis::Column => DesignPanelProperty::GridColumnTrack(index),
                DesignGridTrackAxis::Row => DesignPanelProperty::GridRowTrack(index),
            };
            let value_property = match axis {
                DesignGridTrackAxis::Column => DesignPanelProperty::GridColumnTrackValue(index),
                DesignGridTrackAxis::Row => DesignPanelProperty::GridRowTrackValue(index),
            };
            let value_suffix = match track.sizing {
                DesignGridTrackSizing::Fixed => "px",
                DesignGridTrackSizing::Fraction => "fr",
                DesignGridTrackSizing::Hug => "",
            };
            let up_enabled = self.can_edit() && index > 0;
            let down_enabled = self.can_edit() && index + 1 < tracks.len();
            let delete_enabled = self.can_edit() && layout.can_delete_grid_track(axis);
            controls = controls.child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(self.render_value_cell(
                        format!("grid-{}-track-{index}", label.to_ascii_lowercase()),
                        short_label,
                        format_grid_track(track),
                        track_property,
                        DesignPanelValue::GridTrack(next_grid_track(track)),
                        cx,
                    ))
                    .when(track.sizing != DesignGridTrackSizing::Hug, |row| {
                        row.child(self.render_value_cell(
                            format!("grid-{}-track-value-{index}", label.to_ascii_lowercase()),
                            value_suffix,
                            format_number(track.value),
                            value_property,
                            DesignPanelValue::Number((track.value + 1.).max(0.01)),
                            cx,
                        ))
                    })
                    .child(self.render_grid_track_action_button(
                        format!("grid-{}-track-{index}-up", label.to_ascii_lowercase()),
                        IconName::ChevronUp,
                        DesignPanelAction::GridTracksReorderRequested {
                            node_id: self.node.id.clone(),
                            axis,
                            from_indices: vec![index],
                            insertion_index: index.saturating_sub(1),
                        },
                        up_enabled,
                        cx,
                    ))
                    .child(self.render_grid_track_action_button(
                        format!("grid-{}-track-{index}-down", label.to_ascii_lowercase()),
                        IconName::ChevronDown,
                        DesignPanelAction::GridTracksReorderRequested {
                            node_id: self.node.id.clone(),
                            axis,
                            from_indices: vec![index],
                            insertion_index: index.saturating_add(2).min(tracks.len()),
                        },
                        down_enabled,
                        cx,
                    ))
                    .child(self.render_grid_track_action_button(
                        format!("grid-{}-track-{index}-delete", label.to_ascii_lowercase()),
                        IconName::Minus,
                        DesignPanelAction::GridTrackDeleteRequested {
                            node_id: self.node.id.clone(),
                            axis,
                            index,
                        },
                        delete_enabled,
                        cx,
                    )),
            );
        }

        controls.into_any_element()
    }

    pub(super) fn render_frame_preset_browser(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view_data = self.frame_preset_view_data_for_context()?.clone();
        let open = self.frame_preset_browser_open;
        let selected = view_data.matching_dimensions(self.node.width, self.node.height);
        let trigger_label = selected
            .as_ref()
            .and_then(|selection| {
                view_data
                    .preset(selection)
                    .map(|(group, preset)| format!("{} · {}", group.label, preset.label))
            })
            .unwrap_or_else(|| "Frame presets".to_owned());
        let dimensions_editable = self.property_is_editable(DesignPanelProperty::Width)
            && self.property_is_editable(DesignPanelProperty::Height);
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let target_node_id = view_data.target_node_id.clone();
        let collapsed_groups = self.collapsed_frame_preset_groups.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-frame-preset-browser",
            self.id
        )))
        .label(SharedString::from(trigger_label))
        .tooltip("Browse host-supplied Frame presets")
        .xsmall()
        .compact()
        .ghost()
        .w_full()
        .h(px(ROW_HEIGHT))
        .selected(open)
        .on_keyboard_activate({
            let panel = panel_for_open.clone();
            let target_node_id = view_data.target_node_id.clone();
            move |_, cx| {
                panel.update(cx, |this, cx| {
                    let has_current_catalog = this
                        .frame_preset_view_data_for_context()
                        .is_some_and(|current| current.target_node_id == target_node_id);
                    this.frame_preset_browser_open = !open && has_current_catalog;
                    if this.frame_preset_browser_open {
                        this.cancel_menu_preview(cx);
                        this.cancel_active_paint_edit(cx);
                        this.active_picker = None;
                        this.auxiliary_color_picker = None;
                        this.active_effect_settings = None;
                        this.effect_style_browser_open = false;
                        this.property_variable_picker = None;
                        this.component_property_variable_picker = None;
                        this.component_swap_browser = None;
                        this.typography_style_picker_open = false;
                        this.font_browser_open = false;
                        this.type_settings_open = false;
                        this.selection_header_overlay = None;
                    }
                    cx.notify();
                });
            }
        });

        Some(
            Popover::new(SharedString::from(format!(
                "{}-frame-preset-popover",
                self.id
            )))
            .anchor(Anchor::TopRight)
            .open(open)
            .overlay_closable(true)
            .on_open_change(move |is_open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    let has_current_catalog = this
                        .frame_preset_view_data_for_context()
                        .is_some_and(|current| current.target_node_id == target_node_id);
                    this.frame_preset_browser_open = *is_open && has_current_catalog;
                    if this.frame_preset_browser_open {
                        this.cancel_menu_preview(cx);
                        this.cancel_active_paint_edit(cx);
                        this.active_picker = None;
                        this.auxiliary_color_picker = None;
                        this.active_effect_settings = None;
                        this.effect_style_browser_open = false;
                        this.property_variable_picker = None;
                        this.component_property_variable_picker = None;
                        this.component_swap_browser = None;
                        this.typography_style_picker_open = false;
                        this.font_browser_open = false;
                        this.type_settings_open = false;
                        this.selection_header_overlay = None;
                    }
                    cx.notify();
                });
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let mut content = v_flex()
                    .w(popup_width(window, 300.))
                    .max_h(popup_height(window, 460.))
                    .gap_1()
                    .p_2()
                    .child(div().text_sm().font_semibold().child("Frame presets"));
                if let Some(reason) = view_data.availability.disabled_reason() {
                    content = content.child(
                        div()
                            .px_1()
                            .pb_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(reason.clone()),
                    );
                }
                let mut groups = v_flex()
                    .w_full()
                    .max_h(popup_height(window, 400.))
                    .overflow_y_scrollbar();
                for group in &view_data.groups {
                    let collapsed = collapsed_groups.contains(&group.id);
                    let group_id = group.id.clone();
                    let panel = panel_for_content.clone();
                    let group_tooltip = group
                        .availability
                        .disabled_reason()
                        .cloned()
                        .unwrap_or_else(|| {
                            if collapsed {
                                "Expand preset group".into()
                            } else {
                                "Collapse preset group".into()
                            }
                        });
                    groups = groups.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-frame-preset-group-{}",
                            group.id
                        )))
                        .label(SharedString::from(format!(
                            "{} {}",
                            if collapsed { "›" } else { "⌄" },
                            group.label
                        )))
                        .tooltip(group_tooltip)
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .h(px(ROW_HEIGHT))
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.toggle_frame_preset_group(&group_id, cx);
                            });
                        }),
                    );
                    if let Some(reason) = group.availability.disabled_reason() {
                        groups = groups.child(
                            div()
                                .w_full()
                                .px_2()
                                .pb_1()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(reason.clone()),
                        );
                    }
                    if collapsed {
                        continue;
                    }
                    for preset in &group.presets {
                        let selection = group.selection(preset.id.clone());
                        let unavailable_reason = if !dimensions_editable {
                            Some(SharedString::from("Frame dimensions are read only"))
                        } else {
                            view_data.effective_disabled_reason(&selection).cloned()
                        };
                        let can_apply = unavailable_reason.is_none();
                        let is_selected = selected.as_ref() == Some(&selection);
                        let tooltip = unavailable_reason.unwrap_or_else(|| {
                            SharedString::from(format!(
                                "Resize to {} × {}",
                                format_number(preset.width),
                                format_number(preset.height)
                            ))
                        });
                        let panel = panel_for_content.clone();
                        let selection_for_click = selection.clone();
                        groups = groups.child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-frame-preset-{}-{}",
                                group.id, preset.id
                            )))
                            .label(SharedString::from(format!(
                                "{}  ·  {} × {}",
                                preset.label,
                                format_number(preset.width),
                                format_number(preset.height)
                            )))
                            .tooltip(tooltip)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .h(px(ROW_HEIGHT))
                            .selected(is_selected)
                            .disabled(!can_apply)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_frame_preset_apply(selection_for_click.clone(), cx);
                                });
                            }),
                        );
                    }
                }
                content.child(groups)
            })
            .into_any_element(),
        )
    }

    const fn dimension_properties(
        axis: DesignLayoutDimensionAxis,
    ) -> (
        DesignPanelProperty,
        DesignPanelProperty,
        DesignPanelProperty,
        DesignPanelProperty,
    ) {
        match axis {
            DesignLayoutDimensionAxis::Width => (
                DesignPanelProperty::Width,
                DesignPanelProperty::HorizontalSizing,
                DesignPanelProperty::MinWidth,
                DesignPanelProperty::MaxWidth,
            ),
            DesignLayoutDimensionAxis::Height => (
                DesignPanelProperty::Height,
                DesignPanelProperty::VerticalSizing,
                DesignPanelProperty::MinHeight,
                DesignPanelProperty::MaxHeight,
            ),
        }
    }

    pub(super) fn dimension_limits_for_node(
        node: &DesignPanelNode,
        axis: DesignLayoutDimensionAxis,
    ) -> (Option<f32>, Option<f32>) {
        let Some(layout) = node.layout.as_ref() else {
            return (None, None);
        };
        match axis {
            DesignLayoutDimensionAxis::Width => (layout.item.min_width, layout.item.max_width),
            DesignLayoutDimensionAxis::Height => (layout.item.min_height, layout.item.max_height),
        }
    }

    pub(super) fn dimension_limits_are_applicable_for(
        node: &DesignPanelNode,
        context: &DesignPanelInspectionContext,
    ) -> bool {
        let owns_active_auto_layout = node.supports_auto_layout_container()
            && node
                .layout
                .as_ref()
                .is_some_and(|layout| layout.mode != DesignLayoutMode::None);
        owns_active_auto_layout
            || (node.supports_auto_layout_child()
                && context.parent_layout().participates_in_auto_layout())
    }

    pub(super) fn dimension_limits_are_applicable(&self) -> bool {
        Self::dimension_limits_are_applicable_for(&self.node, &self.inspection_context)
    }

    pub(super) fn dimension_limit_fields_for_axis(
        axis: DesignLayoutDimensionAxis,
    ) -> [DesignPanelProperty; 2] {
        let (_, _, minimum, maximum) = Self::dimension_properties(axis);
        [minimum, maximum]
    }

    pub(super) fn disclose_existing_dimension_limits(
        &mut self,
        axis: DesignLayoutDimensionAxis,
        cx: &mut Context<Self>,
    ) {
        if !self.dimension_limits_are_applicable() {
            return;
        }
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let [minimum_property, maximum_property] = Self::dimension_limit_fields_for_axis(axis);
        if minimum.is_some() {
            self.dimension_limit_fields_disclosed
                .insert(minimum_property);
        }
        if maximum.is_some() {
            self.dimension_limit_fields_disclosed
                .insert(maximum_property);
        }
        cx.notify();
    }

    pub(super) fn reveal_dimension_limit_field(
        &mut self,
        axis: DesignLayoutDimensionAxis,
        property: DesignPanelProperty,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::dimension_limit_fields_for_axis(axis).contains(&property)
            || !self.dimension_limits_are_applicable()
            || !self.property_is_editable(property)
        {
            return;
        }
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let already_exists = match property {
            DesignPanelProperty::MinWidth | DesignPanelProperty::MinHeight => minimum.is_some(),
            DesignPanelProperty::MaxWidth | DesignPanelProperty::MaxHeight => maximum.is_some(),
            _ => true,
        };
        if already_exists {
            return;
        }
        let (_, sizing_property, _, _) = Self::dimension_properties(axis);
        self.cancel_menu_preview_for_property(sizing_property, cx);
        self.dimension_menu_open = None;
        self.dimension_limit_fields_disclosed.insert(property);
        let fallback = match property {
            DesignPanelProperty::MinWidth | DesignPanelProperty::MinHeight => {
                DesignPanelValue::OptionalNumber(Some(1.))
            }
            DesignPanelProperty::MaxWidth => DesignPanelValue::OptionalNumber(Some(
                self.node
                    .width
                    .max(
                        self.node
                            .layout
                            .as_ref()
                            .and_then(|layout| layout.item.min_width)
                            .unwrap_or(1.),
                    )
                    .max(1.),
            )),
            DesignPanelProperty::MaxHeight => DesignPanelValue::OptionalNumber(Some(
                self.node
                    .height
                    .max(
                        self.node
                            .layout
                            .as_ref()
                            .and_then(|layout| layout.item.min_height)
                            .unwrap_or(1.),
                    )
                    .max(1.),
            )),
            _ => return,
        };
        self.activate_property(property, fallback, window, cx);
        cx.notify();
    }

    pub(super) fn remove_dimension_limits(
        &mut self,
        axis: DesignLayoutDimensionAxis,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.dimension_limits_are_applicable() {
            return false;
        }
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let [minimum_property, maximum_property] = Self::dimension_limit_fields_for_axis(axis);
        let mut emitted = false;
        for (property, value) in [(minimum_property, minimum), (maximum_property, maximum)] {
            if value.is_some() && self.property_is_editable(property) {
                self.emit_property(property, DesignPanelValue::OptionalNumber(None), cx);
                emitted = true;
            }
        }
        if emitted {
            self.dimension_menu_open = None;
            self.dimension_limit_fields_disclosed
                .remove(&minimum_property);
            self.dimension_limit_fields_disclosed
                .remove(&maximum_property);
            cx.notify();
        }
        emitted
    }

    pub(super) fn set_dimension_limits_preview(
        &mut self,
        axis: DesignLayoutDimensionAxis,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            let Some(active) = self
                .dimension_limits_preview
                .take()
                .filter(|active| active.axis == axis)
            else {
                return;
            };
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::DimensionLimitsPreviewRequested {
                    node_id: active.node_id,
                    axis: active.axis,
                    minimum: active.minimum,
                    maximum: active.maximum,
                    preview: false,
                },
            );
            return;
        }
        if !self.can_edit()
            || self.inspection_context.selection().kind() != DesignPanelSelectionKind::Single
            || !self.dimension_limits_are_applicable()
        {
            return;
        }
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        if minimum.is_none() && maximum.is_none() {
            return;
        }
        let next = DimensionLimitsPreview {
            node_id: self.node.id.clone(),
            axis,
            minimum,
            maximum,
        };
        if self.dimension_limits_preview.as_ref() == Some(&next) {
            return;
        }
        if let Some(active) = self.dimension_limits_preview.take() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::DimensionLimitsPreviewRequested {
                    node_id: active.node_id,
                    axis: active.axis,
                    minimum: active.minimum,
                    maximum: active.maximum,
                    preview: false,
                },
            );
        }
        self.dimension_limits_preview = Some(next.clone());
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::DimensionLimitsPreviewRequested {
                node_id: next.node_id,
                axis: next.axis,
                minimum: next.minimum,
                maximum: next.maximum,
                preview: true,
            },
        );
    }

    pub(super) fn cancel_dimension_limits_preview(&mut self, cx: &mut Context<Self>) {
        let Some(active) = self.dimension_limits_preview.take() else {
            return;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::DimensionLimitsPreviewRequested {
                node_id: active.node_id,
                axis: active.axis,
                minimum: active.minimum,
                maximum: active.maximum,
                preview: false,
            },
        );
    }

    pub(super) fn toggle_dimension_menu(
        &mut self,
        axis: DesignLayoutDimensionAxis,
        cx: &mut Context<Self>,
    ) {
        if !self.dimension_limits_are_applicable() {
            self.cancel_menu_preview(cx);
            self.dimension_menu_open = None;
            return;
        }
        if self.dimension_menu_open == Some(axis) {
            let (_, sizing_property, _, _) = Self::dimension_properties(axis);
            self.cancel_menu_preview_for_property(sizing_property, cx);
            self.dimension_menu_open = None;
        } else {
            self.cancel_menu_preview(cx);
            self.dimension_menu_open = Some(axis);
            self.disclose_existing_dimension_limits(axis, cx);
        }
        cx.notify();
    }

    pub(super) fn dimension_limits_tooltip(
        &self,
        axis: DesignLayoutDimensionAxis,
        sizing: DesignSizingMode,
    ) -> SharedString {
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let format_limit = |label: &str, value: Option<f32>| {
            value.map(|value| format!("{label} {}", format_number(value)))
        };
        let limits = [format_limit("Min", minimum), format_limit("Max", maximum)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if limits.is_empty() {
            format!("{} · {} · Add min or max", axis.label(), sizing.label()).into()
        } else {
            format!("{} · {} · {limits}", axis.label(), sizing.label()).into()
        }
    }

    pub(super) fn render_dimension_menu_trigger(
        &self,
        axis: DesignLayoutDimensionAxis,
        sizing: DesignSizingMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.dimension_menu_open == Some(axis);
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let has_limits = minimum.is_some() || maximum.is_some();
        let (_, sizing_property, minimum_property, maximum_property) =
            Self::dimension_properties(axis);
        let sizing_options = self.property_options(sizing_property).unwrap_or_default();
        let sizing_editable = self.property_is_editable(sizing_property);
        let minimum_editable = self.property_is_editable(minimum_property);
        let maximum_editable = self.property_is_editable(maximum_property);
        let remove_editable =
            (minimum.is_some() && minimum_editable) || (maximum.is_some() && maximum_editable);
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_hover = panel.clone();
        let panel_for_keyboard = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let axis_name = axis.label().to_ascii_lowercase();
        let trigger_selector =
            SharedString::from(format!("{}-{axis_name}-dimension-icon", self.id));
        let tooltip = self.dimension_limits_tooltip(axis, sizing);
        let tooltip_for_trigger = tooltip.clone();
        let focus_handle = self.dimension_menu_focus[match axis {
            DesignLayoutDimensionAxis::Width => 0,
            DesignLayoutDimensionAxis::Height => 1,
        }]
        .clone();
        let icon = if has_limits {
            match axis {
                DesignLayoutDimensionAxis::Width => "┤W├",
                DesignLayoutDimensionAxis::Height => "┤H├",
            }
        } else {
            match axis {
                DesignLayoutDimensionAxis::Width => "W",
                DesignLayoutDimensionAxis::Height => "H",
            }
        };
        let trigger = DimensionMenuTrigger {
            selected: open,
            base: div()
                .id(trigger_selector.clone())
                .debug_selector(move || trigger_selector.to_string())
                .key_context(CONTROL_KEY_CONTEXT)
                .track_focus(&focus_handle)
                .tab_index(0)
                .w(px(30.))
                .h(px(ROW_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .text_xs()
                .cursor_pointer()
                .hover(|trigger| trigger.bg(cx.theme().accent))
                .focus(|trigger| {
                    trigger
                        .border_1()
                        .border_color(cx.theme().selection)
                        .bg(cx.theme().accent)
                })
                // Keyboard-only convergence: the pointer path is owned by the
                // wrapping `Popover` trigger, so this control binds the shared
                // `ActivateControl` command instead of `on_activate`'s
                // click/action pair.
                .on_action(move |_: &ActivateControl, _, cx| {
                    cx.stop_propagation();
                    panel_for_keyboard.update(cx, |this, cx| {
                        this.toggle_dimension_menu(axis, cx);
                    });
                })
                .on_hover(move |hovered, _, cx| {
                    panel_for_hover.update(cx, |this, cx| {
                        this.set_dimension_limits_preview(axis, *hovered, cx);
                    });
                })
                .tooltip(move |window, cx| {
                    Tooltip::new(tooltip_for_trigger.clone()).build(window, cx)
                })
                .child(icon),
        };

        Popover::new(SharedString::from(format!(
            "{}-{axis_name}-dimension-popover",
            self.id
        )))
        .anchor(Anchor::TopLeft)
        .open(open)
        .overlay_closable(true)
        .on_open_change(move |is_open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *is_open && this.dimension_limits_are_applicable() {
                    this.dimension_menu_open = Some(axis);
                    this.disclose_existing_dimension_limits(axis, cx);
                } else if this.dimension_menu_open == Some(axis) {
                    this.cancel_menu_preview_for_property(sizing_property, cx);
                    this.dimension_menu_open = None;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex().w(popup_width(window, 204.)).gap_0p5().p_1().child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_semibold()
                    .child(format!("{} resizing", axis.label())),
            );
            for option in sizing_options.clone() {
                let DesignPanelValue::SizingMode(mode) = option.value else {
                    continue;
                };
                let panel = panel_for_content.clone();
                let panel_for_hover = panel.clone();
                let panel_for_key = panel.clone();
                let sizing_selector = SharedString::from(format!(
                    "{panel_id}-{axis_name}-sizing-{}",
                    mode.label().to_ascii_lowercase()
                ));
                let sizing_label = match mode {
                    DesignSizingMode::Fixed => {
                        format!("Fixed {}", axis.label().to_ascii_lowercase())
                    }
                    DesignSizingMode::Hug => "Hug contents".to_owned(),
                    DesignSizingMode::Fill => "Fill container".to_owned(),
                };
                content = content.child(
                    Button::new(sizing_selector.clone())
                        .debug_selector(move || sizing_selector.to_string())
                        .label(SharedString::from(sizing_label))
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .selected(mode == sizing)
                        .disabled(!sizing_editable)
                        .on_hover(move |hovered, _, cx| {
                            panel_for_hover.update(cx, |this, cx| {
                                this.set_property_menu_preview(
                                    sizing_property,
                                    DesignPanelValue::SizingMode(mode),
                                    *hovered,
                                    cx,
                                );
                            });
                        })
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            panel_for_key.update(cx, |this, cx| {
                                this.set_property_menu_preview(
                                    sizing_property,
                                    DesignPanelValue::SizingMode(mode),
                                    true,
                                    cx,
                                );
                                match event.keystroke.key.as_str() {
                                    "enter" | "space" => {
                                        this.cancel_menu_preview(cx);
                                        this.dimension_menu_open = None;
                                        this.emit_property(
                                            sizing_property,
                                            DesignPanelValue::SizingMode(mode),
                                            cx,
                                        );
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    }
                                    "escape" => {
                                        this.cancel_menu_preview(cx);
                                        this.dimension_menu_open = None;
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    }
                                    "tab" => this.cancel_menu_preview(cx),
                                    _ => {}
                                }
                            });
                        })
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.cancel_menu_preview(cx);
                                this.dimension_menu_open = None;
                                this.emit_property(
                                    sizing_property,
                                    DesignPanelValue::SizingMode(mode),
                                    cx,
                                );
                                cx.notify();
                            });
                        }),
                );
            }
            let panel = panel_for_content.clone();
            let add_min_selector = SharedString::from(format!("{panel_id}-{axis_name}-add-min"));
            content = content.child(
                Button::new(add_min_selector.clone())
                    .debug_selector(move || add_min_selector.to_string())
                    .label(SharedString::from(format!(
                        "Add min {}",
                        axis.label().to_ascii_lowercase()
                    )))
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(minimum.is_some() || !minimum_editable)
                    .on_activate(move |_, window, cx| {
                        panel.update(cx, |this, cx| {
                            this.reveal_dimension_limit_field(axis, minimum_property, window, cx);
                        });
                    }),
            );
            let panel = panel_for_content.clone();
            let add_max_selector = SharedString::from(format!("{panel_id}-{axis_name}-add-max"));
            content = content.child(
                Button::new(add_max_selector.clone())
                    .debug_selector(move || add_max_selector.to_string())
                    .label(SharedString::from(format!(
                        "Add max {}",
                        axis.label().to_ascii_lowercase()
                    )))
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(maximum.is_some() || !maximum_editable)
                    .on_activate(move |_, window, cx| {
                        panel.update(cx, |this, cx| {
                            this.reveal_dimension_limit_field(axis, maximum_property, window, cx);
                        });
                    }),
            );
            if has_limits {
                let panel = panel_for_content.clone();
                let remove_selector =
                    SharedString::from(format!("{panel_id}-{axis_name}-remove-min-max"));
                content = content.child(div().h(px(1.)).my_1().bg(cx.theme().border));
                content = content.child(
                    Button::new(remove_selector.clone())
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .disabled(!remove_editable)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.remove_dimension_limits(axis, cx);
                            });
                        })
                        .child(
                            div()
                                .debug_selector(move || remove_selector.to_string())
                                .child("Remove min and max"),
                        ),
                );
            }
            content
        })
        .into_any_element()
    }

    pub(super) fn render_dimension_control(
        &self,
        axis: DesignLayoutDimensionAxis,
        layout: &DesignLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (dimension_property, _, _, _) = Self::dimension_properties(axis);
        let (id, value, next, sizing) = match axis {
            DesignLayoutDimensionAxis::Width => (
                "width",
                self.node.width,
                self.node.width + 8.,
                layout.horizontal_sizing,
            ),
            DesignLayoutDimensionAxis::Height => (
                "height",
                self.node.height,
                self.node.height + 8.,
                layout.vertical_sizing,
            ),
        };
        if !self.dimension_limits_are_applicable() {
            let prefix = match axis {
                DesignLayoutDimensionAxis::Width => "W",
                DesignLayoutDimensionAxis::Height => "H",
            };
            return self.render_value_cell(
                id,
                prefix,
                format_number(value),
                dimension_property,
                DesignPanelValue::Number(next),
                cx,
            );
        }
        h_flex()
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .rounded(px(4.))
            .bg(cx.theme().secondary)
            .child(self.render_dimension_menu_trigger(axis, sizing, cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(self.render_value_cell_with_left_padding(
                        id,
                        "",
                        format_number(value),
                        dimension_property,
                        DesignPanelValue::Number(next),
                        0.,
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_dimension_limit_fields(
        &self,
        axis: DesignLayoutDimensionAxis,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.dimension_limits_are_applicable() {
            return None;
        }
        let (minimum, maximum) = Self::dimension_limits_for_node(&self.node, axis);
        let [minimum_property, maximum_property] = Self::dimension_limit_fields_for_axis(axis);
        let show_minimum = self
            .dimension_limit_fields_disclosed
            .contains(&minimum_property);
        let show_maximum = self
            .dimension_limit_fields_disclosed
            .contains(&maximum_property);
        if !show_minimum && !show_maximum {
            return None;
        }
        let (minimum_id, maximum_id, minimum_prefix, maximum_prefix, dimension) = match axis {
            DesignLayoutDimensionAxis::Width => (
                "layout-min-width",
                "layout-max-width",
                "W≥",
                "W≤",
                self.node.width,
            ),
            DesignLayoutDimensionAxis::Height => (
                "layout-min-height",
                "layout-max-height",
                "H≥",
                "H≤",
                self.node.height,
            ),
        };
        let minimum_next = DesignPanelValue::OptionalNumber(Some(1.));
        let maximum_next =
            DesignPanelValue::OptionalNumber(Some(dimension.max(minimum.unwrap_or(1.)).max(1.)));
        Some(
            h_flex()
                .w_full()
                .gap_2()
                .when(show_minimum, |row| {
                    row.child(self.render_value_cell(
                        minimum_id,
                        minimum_prefix,
                        format_optional_number(minimum),
                        minimum_property,
                        minimum_next,
                        cx,
                    ))
                })
                .when(show_maximum, |row| {
                    row.child(self.render_value_cell(
                        maximum_id,
                        maximum_prefix,
                        format_optional_number(maximum),
                        maximum_property,
                        maximum_next,
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }

    pub(super) fn render_add_auto_layout(
        &self,
        view_data: &DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let enabled = self.can_edit() && view_data.can_request();
        let tooltip = view_data.disabled_reason.clone().unwrap_or_else(|| {
            if self.can_edit() {
                "Add auto layout".into()
            } else {
                "Editing permission is required".into()
            }
        });
        let debug_selector = format!("{}-add-auto-layout", self.id);
        Button::new(SharedString::from(debug_selector.clone()))
            .debug_selector(move || debug_selector.clone())
            .label("Add auto layout")
            .icon(IconName::Plus)
            .tooltip(tooltip)
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .disabled(!enabled)
            .on_activate(move |_, _, cx| {
                panel.update(cx, |this, cx| {
                    this.emit_add_auto_layout(cx);
                });
            })
            .into_any_element()
    }

    pub(super) fn render_draw_add_auto_layout(
        &self,
        view_data: &DesignAddAutoLayoutViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let enabled = self.can_edit() && view_data.can_request();
        let tooltip = view_data.disabled_reason.clone().unwrap_or_else(|| {
            if self.can_edit() {
                "Add auto layout".into()
            } else {
                "Editing permission is required".into()
            }
        });
        let debug_selector = format!("{}-draw-add-auto-layout", self.id);
        Button::new(SharedString::from(debug_selector.clone()))
            .debug_selector(move || debug_selector.clone())
            .icon(IconName::Plus)
            .tooltip(tooltip)
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(24.))
            .disabled(!enabled)
            .on_activate(move |_, _, cx| {
                panel.update(cx, |this, cx| {
                    this.emit_add_auto_layout(cx);
                });
            })
            .into_any_element()
    }

    pub(super) fn render_draw_layout_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let section = DesignPanelSection::Layout;
        let id = SharedString::from(format!("{}-section-layout", self.id));
        let mut actions = h_flex()
            .id(SharedString::from(format!(
                "{}-draw-layout-header-actions",
                self.id
            )))
            .gap_1()
            .on_click(|_, _, cx| cx.stop_propagation());
        if self.node.supports_resize_to_fit() {
            actions = actions.child(div().size(px(24.)).flex_none().child(
                self.render_icon_action_button(
                    "draw-resize-to-fit",
                    IconName::Maximize,
                    DesignPanelAction::ResizeToFitRequested {
                        target: self.command_target(),
                    },
                    cx,
                ),
            ));
        }
        if let Some(view_data) = self.add_auto_layout_view_data_for_context() {
            actions = actions.child(self.render_draw_add_auto_layout(view_data, cx));
        }
        h_flex()
            .id(id)
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(40.))
            .w_full()
            .pl(px(PANEL_PADDING))
            .pr_2()
            .gap_1()
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.45)))
            .focus(|style| {
                style
                    .bg(cx.theme().sidebar_accent.opacity(0.45))
                    .border_color(cx.theme().selection)
            })
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.toggle_section(section, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .font_semibold()
                    .child(section.label()),
            )
            .child(actions)
            .into_any_element()
    }

    pub(super) fn render_layout(&self, cx: &mut Context<Self>) -> AnyElement {
        let layout = self.node.layout.clone().unwrap_or_default();
        let owns_auto_layout = self.node.supports_auto_layout_container();
        let parent_layout = self.inspection_context.parent_layout();
        let participates_in_auto_layout =
            self.node.supports_auto_layout_child() && parent_layout.participates_in_auto_layout();
        let ignored_by_auto_layout =
            self.node.supports_auto_layout_child() && parent_layout.is_ignored_by_auto_layout();
        let layout_mode_editable = self.property_is_editable(DesignPanelProperty::LayoutMode);
        let direction_buttons = [
            (DesignLayoutMode::None, IconName::Frame),
            (DesignLayoutMode::Horizontal, IconName::ArrowRight),
            (DesignLayoutMode::Vertical, IconName::ArrowDown),
            (DesignLayoutMode::Grid, IconName::LayoutDashboard),
        ];
        let mut directions = h_flex()
            .h(px(ROW_HEIGHT))
            .w_full()
            .rounded(px(4.))
            .bg(cx.theme().secondary);
        for (mode, icon) in direction_buttons.into_iter().filter(|(mode, _)| {
            *mode != DesignLayoutMode::Grid || self.node.supports_grid_auto_layout()
        }) {
            directions = directions.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-layout-{}",
                        self.id,
                        mode.label().to_lowercase()
                    )))
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .when(!layout_mode_editable, |button| button.opacity(0.62))
                    .when(layout.mode == mode, |button| {
                        button
                            .bg(cx.theme().accent)
                            .border_1()
                            .border_color(cx.theme().selection)
                    })
                    .when(layout_mode_editable, |button| {
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
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.emit_property(
                                    DesignPanelProperty::LayoutMode,
                                    DesignPanelValue::LayoutMode(mode),
                                    cx,
                                );
                            }))
                    })
                    .child(Icon::new(icon).xsmall()),
            );
        }

        let mut content = v_flex().pl(px(PANEL_PADDING)).pr_2().pb_4().gap_1();
        if !self.renders_draw_workspace()
            && let Some(view_data) = self.add_auto_layout_view_data_for_context()
        {
            content = content.child(self.render_add_auto_layout(view_data, cx));
        }
        if owns_auto_layout {
            if self.renders_draw_workspace() {
                let flow_id = SharedString::from(format!("{}-draw-layout-flow-label", self.id));
                let flow_selector = flow_id.to_string();
                content = content.child(
                    div()
                        .id(flow_id)
                        .debug_selector(move || flow_selector.clone())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Flow"),
                );
            }
            content = content.child(directions);
        }
        if let Some(typography) = self.node.typography.as_ref()
            && !owns_auto_layout
        {
            content = content
                .child(self.render_group_label("Resizing", cx))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(self.render_text_resize_control(typography.resize, cx)),
                        )
                        .child(div().size(px(24.)).flex_none()),
                );
        }
        if let Some(frame_presets) = self.render_frame_preset_browser(cx) {
            content = content
                .child(self.render_group_label("Frame", cx))
                .child(frame_presets);
        }
        content = content
            .child(self.render_group_label("Dimensions", cx))
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_2()
                            .child(self.render_dimension_control(
                                DesignLayoutDimensionAxis::Width,
                                &layout,
                                cx,
                            ))
                            .child(self.render_dimension_control(
                                DesignLayoutDimensionAxis::Height,
                                &layout,
                                cx,
                            )),
                    )
                    .when(
                        self.node_capability_allows_property(DesignPanelProperty::LockAspectRatio),
                        |row| {
                            row.child(
                                div()
                                    .w(px(24.))
                                    .flex_none()
                                    .rounded(px(4.))
                                    .when(self.node.lock_aspect_ratio, |button| {
                                        button
                                            .bg(cx.theme().selection.opacity(0.22))
                                            .text_color(cx.theme().selection)
                                    })
                                    .child(self.render_position_action_button_with_enabled(
                                        "lock-aspect-ratio",
                                        PositionActionIcon::LockAspectRatio,
                                        DesignPanelAction::PropertyChangeRequested {
                                            node_id: self.node.id.clone(),
                                            property: DesignPanelProperty::LockAspectRatio,
                                            value: DesignPanelValue::Bool(
                                                !self.node.lock_aspect_ratio,
                                            ),
                                        },
                                        self.property_is_editable(
                                            DesignPanelProperty::LockAspectRatio,
                                        ),
                                        cx,
                                    )),
                            )
                        },
                    )
                    .when(
                        !self.renders_draw_workspace() && self.node.supports_resize_to_fit(),
                        |row| {
                            row.child(div().w(px(24.)).flex_none().child(
                                self.render_icon_action_button(
                                    "resize-to-fit",
                                    IconName::Maximize,
                                    DesignPanelAction::ResizeToFitRequested {
                                        target: self.command_target(),
                                    },
                                    cx,
                                ),
                            ))
                        },
                    ),
            );
        if let Some(width_limits) =
            self.render_dimension_limit_fields(DesignLayoutDimensionAxis::Width, cx)
        {
            content = content.child(width_limits);
        }
        if let Some(height_limits) =
            self.render_dimension_limit_fields(DesignLayoutDimensionAxis::Height, cx)
        {
            content = content.child(height_limits);
        }
        if owns_auto_layout && layout.mode != DesignLayoutMode::None {
            let gap_controls = h_flex()
                .w_full()
                .gap_2()
                .when(layout.mode != DesignLayoutMode::Grid, |controls| {
                    controls.child(self.render_value_cell(
                        "layout-item-spacing-mode",
                        "⇔",
                        layout.item_spacing_mode.label(),
                        DesignPanelProperty::ItemSpacingMode,
                        DesignPanelValue::ItemSpacingMode(match layout.item_spacing_mode {
                            DesignItemSpacingMode::Fixed => DesignItemSpacingMode::Auto,
                            DesignItemSpacingMode::Auto => DesignItemSpacingMode::Fixed,
                        }),
                        cx,
                    ))
                })
                .when(
                    layout.mode == DesignLayoutMode::Grid
                        || layout.item_spacing_mode == DesignItemSpacingMode::Fixed,
                    |controls| {
                        controls.child(self.render_value_cell(
                            "layout-gap",
                            "↔",
                            format_number(layout.gap),
                            DesignPanelProperty::Gap,
                            DesignPanelValue::Number(layout.gap + 2.),
                            cx,
                        ))
                    },
                )
                .when(layout.mode == DesignLayoutMode::Grid, |controls| {
                    controls.child(self.render_value_cell(
                        "layout-counter-axis-gap",
                        "↕",
                        format_optional_number(layout.counter_axis_gap),
                        DesignPanelProperty::CounterAxisGap,
                        DesignPanelValue::OptionalNumber(Some(
                            layout.counter_axis_gap.unwrap_or(0.) + 2.,
                        )),
                        cx,
                    ))
                });
            if layout.mode == DesignLayoutMode::Grid {
                content = content
                    .child(self.render_group_label("Gap", cx))
                    .child(gap_controls);
            } else {
                content = content
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(88.))
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Alignment"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Gap"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(self.render_alignment_grid(cx))
                            .child(v_flex().flex_1().gap_2().child(gap_controls)),
                    );
                if layout.mode == DesignLayoutMode::Horizontal && layout.wrap {
                    let next_alignment = match layout.counter_axis_align_content {
                        DesignCounterAxisAlignContent::Auto => {
                            DesignCounterAxisAlignContent::SpaceBetween
                        }
                        DesignCounterAxisAlignContent::SpaceBetween => {
                            DesignCounterAxisAlignContent::Auto
                        }
                    };
                    content = content
                        .child(self.render_group_label("Wrapped tracks", cx))
                        .child(
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(self.render_value_cell(
                                    "layout-counter-axis-align-content",
                                    "↕",
                                    layout.counter_axis_align_content.label(),
                                    DesignPanelProperty::CounterAxisAlignContent,
                                    DesignPanelValue::CounterAxisAlignContent(next_alignment),
                                    cx,
                                ))
                                .when(
                                    layout.counter_axis_align_content
                                        == DesignCounterAxisAlignContent::Auto,
                                    |controls| {
                                        controls.child(self.render_counter_axis_spacing_controls(
                                            layout.counter_axis_gap,
                                            layout.gap,
                                            cx,
                                        ))
                                    },
                                ),
                        );
                }
            }
            let padding_controls = match self.padding_editor_mode {
                PaddingEditorMode::Axes => h_flex()
                    .w_full()
                    .gap_2()
                    .child(self.render_value_cell(
                        "layout-padding-vertical",
                        "↕",
                        format_number(layout.padding[0]),
                        DesignPanelProperty::PaddingVertical,
                        DesignPanelValue::Number(layout.padding[0] + 4.),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "layout-padding-horizontal",
                        "↔",
                        format_number(layout.padding[3]),
                        DesignPanelProperty::PaddingHorizontal,
                        DesignPanelValue::Number(layout.padding[3] + 4.),
                        cx,
                    ))
                    .into_any_element(),
                PaddingEditorMode::Individual => h_flex()
                    .w_full()
                    .gap_2()
                    .child(self.render_value_cell(
                        "layout-padding-top",
                        "T",
                        format_number(layout.padding[0]),
                        DesignPanelProperty::PaddingTop,
                        DesignPanelValue::Number(layout.padding[0] + 4.),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "layout-padding-right",
                        "R",
                        format_number(layout.padding[1]),
                        DesignPanelProperty::PaddingRight,
                        DesignPanelValue::Number(layout.padding[1] + 4.),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "layout-padding-bottom",
                        "B",
                        format_number(layout.padding[2]),
                        DesignPanelProperty::PaddingBottom,
                        DesignPanelValue::Number(layout.padding[2] + 4.),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        "layout-padding-left",
                        "L",
                        format_number(layout.padding[3]),
                        DesignPanelProperty::PaddingLeft,
                        DesignPanelValue::Number(layout.padding[3] + 4.),
                        cx,
                    ))
                    .into_any_element(),
                PaddingEditorMode::Shorthand => h_flex()
                    .w_full()
                    .gap_2()
                    .child(self.render_value_cell(
                        "layout-padding-shorthand",
                        "CSS",
                        format_padding_shorthand(layout.padding),
                        DesignPanelProperty::PaddingShorthand,
                        DesignPanelValue::NumberList(
                            layout.padding.iter().map(|value| value + 4.).collect(),
                        ),
                        cx,
                    ))
                    .into_any_element(),
            };
            content = content
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(self.render_group_label("Padding", cx))
                        .child(
                            h_flex()
                                .gap_0p5()
                                .child(self.render_padding_mode_button(
                                    PaddingEditorMode::Axes,
                                    "↔↕",
                                    "Horizontal and vertical padding",
                                    cx,
                                ))
                                .child(self.render_padding_mode_button(
                                    PaddingEditorMode::Individual,
                                    "TRBL",
                                    "Individual top, right, bottom, and left padding",
                                    cx,
                                ))
                                .child(self.render_padding_mode_button(
                                    PaddingEditorMode::Shorthand,
                                    "CSS",
                                    "Uniform or CSS shorthand padding",
                                    cx,
                                )),
                        ),
                )
                .child(padding_controls)
                .when(layout.mode == DesignLayoutMode::Horizontal, |content| {
                    content.child(self.render_toggle_row(
                        "wrap",
                        "Wrap",
                        layout.wrap,
                        DesignPanelProperty::Wrap,
                        cx,
                    ))
                })
                .child(self.render_toggle_row(
                    "include-strokes",
                    "Include strokes in layout",
                    layout.include_strokes,
                    DesignPanelProperty::IncludeStrokes,
                    cx,
                ));
            if layout.mode != DesignLayoutMode::Grid {
                content = content.child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "stacking-order",
                            "Z",
                            layout.stacking_order.label(),
                            DesignPanelProperty::StackingOrder,
                            DesignPanelValue::StackingOrder(match layout.stacking_order {
                                DesignStackingOrder::LastOnTop => DesignStackingOrder::FirstOnTop,
                                DesignStackingOrder::FirstOnTop => DesignStackingOrder::LastOnTop,
                            }),
                            cx,
                        ))
                        .when(layout.mode == DesignLayoutMode::Horizontal, |row| {
                            row.child(self.render_value_cell(
                                "baseline-alignment",
                                "A",
                                layout.baseline_alignment.label(),
                                DesignPanelProperty::BaselineAlignment,
                                DesignPanelValue::BaselineAlignment(
                                    match layout.baseline_alignment {
                                        DesignBaselineAlignment::Bounds => {
                                            DesignBaselineAlignment::Baseline
                                        }
                                        DesignBaselineAlignment::Baseline => {
                                            DesignBaselineAlignment::Bounds
                                        }
                                    },
                                ),
                                cx,
                            ))
                        }),
                );
            }
            if layout.mode == DesignLayoutMode::Grid {
                content = content
                    .child(div().pt_1().text_xs().font_semibold().child("Grid"))
                    .child(self.render_grid_dimensions_picker(&layout, cx))
                    .child(self.render_grid_auto_rows_toggle(cx))
                    .child(self.render_group_label("Positioning", cx))
                    .child(self.render_value_cell(
                        "grid-items-positioning",
                        "↳",
                        layout.grid_items_positioning.label(),
                        DesignPanelProperty::GridItemsPositioning,
                        DesignPanelValue::GridItemsPositioning(
                            match layout.grid_items_positioning {
                                DesignGridItemsPositioning::Manual => {
                                    DesignGridItemsPositioning::RowAutoFlow
                                }
                                DesignGridItemsPositioning::RowAutoFlow => {
                                    DesignGridItemsPositioning::Manual
                                }
                            },
                        ),
                        cx,
                    ))
                    .child(self.render_grid_track_controls(
                        DesignGridTrackAxis::Column,
                        &layout,
                        cx,
                    ))
                    .child(self.render_grid_track_controls(DesignGridTrackAxis::Row, &layout, cx));
            }
        }
        if let Some(direction) = parent_layout
            .auto_layout_direction()
            .filter(|_| participates_in_auto_layout)
        {
            content = content
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .font_semibold()
                        .child("Auto-layout child"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "layout-positioning",
                            "P",
                            layout.item.positioning.label(),
                            DesignPanelProperty::LayoutPositioning,
                            DesignPanelValue::LayoutPositioning(match layout.item.positioning {
                                DesignLayoutPositioning::InFlow => {
                                    DesignLayoutPositioning::Absolute
                                }
                                DesignLayoutPositioning::Absolute => {
                                    DesignLayoutPositioning::InFlow
                                }
                            }),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "layout-align-self",
                            "↔",
                            layout.item.align_self.label(),
                            DesignPanelProperty::LayoutAlignSelf,
                            DesignPanelValue::LayoutAlignSelf(match layout.item.align_self {
                                DesignLayoutAlignSelf::Inherit => DesignLayoutAlignSelf::Stretch,
                                DesignLayoutAlignSelf::Stretch => DesignLayoutAlignSelf::Inherit,
                            }),
                            cx,
                        )),
                )
                .child(h_flex().gap_2().child(self.render_value_cell(
                    "layout-grow",
                    "↗",
                    format_number(layout.item.layout_grow),
                    DesignPanelProperty::LayoutGrow,
                    DesignPanelValue::Number(layout.item.layout_grow + 1.),
                    cx,
                )));
            if direction == DesignPanelAutoLayoutDirection::Grid {
                content = content
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                self.render_value_cell(
                                    "grid-row-index",
                                    "R",
                                    (layout.item.grid_row_index + 1).to_string(),
                                    DesignPanelProperty::GridRowIndex,
                                    DesignPanelValue::Integer(
                                        i64::try_from(layout.item.grid_row_index)
                                            .unwrap_or(i64::MAX)
                                            .saturating_add(2),
                                    ),
                                    cx,
                                ),
                            )
                            .child(
                                self.render_value_cell(
                                    "grid-column-index",
                                    "C",
                                    (layout.item.grid_column_index + 1).to_string(),
                                    DesignPanelProperty::GridColumnIndex,
                                    DesignPanelValue::Integer(
                                        i64::try_from(layout.item.grid_column_index)
                                            .unwrap_or(i64::MAX)
                                            .saturating_add(2),
                                    ),
                                    cx,
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(self.render_value_cell(
                                "grid-row-span",
                                "R×",
                                layout.item.grid_row_span.to_string(),
                                DesignPanelProperty::GridRowSpan,
                                DesignPanelValue::Integer(i64::from(
                                    layout.item.grid_row_span.saturating_add(1),
                                )),
                                cx,
                            ))
                            .child(self.render_value_cell(
                                "grid-column-span",
                                "C×",
                                layout.item.grid_column_span.to_string(),
                                DesignPanelProperty::GridColumnSpan,
                                DesignPanelValue::Integer(i64::from(
                                    layout.item.grid_column_span.saturating_add(1),
                                )),
                                cx,
                            )),
                    );
            }
        }
        if ignored_by_auto_layout {
            content = content
                .child(
                    div()
                        .pt_1()
                        .text_xs()
                        .font_semibold()
                        .child("Auto-layout child"),
                )
                .child(self.render_value_cell(
                    "layout-positioning",
                    "P",
                    DesignLayoutPositioning::Absolute.label(),
                    DesignPanelProperty::LayoutPositioning,
                    DesignPanelValue::LayoutPositioning(DesignLayoutPositioning::InFlow),
                    cx,
                ));
        }
        if self.node.supports_clip_content() {
            content = content.child(self.render_checkbox_row(
                "clip-content",
                "Clip content",
                layout.clip_content,
                DesignPanelProperty::ClipContent,
                cx,
            ));
        }
        if self.renders_draw_workspace() {
            let expanded = self.expanded_sections.contains(&DesignPanelSection::Layout);
            v_flex()
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .child(self.render_draw_layout_header(cx))
                .when(expanded, |section| {
                    section.child(content.into_any_element())
                })
                .into_any_element()
        } else {
            self.render_section(
                DesignPanelSection::Layout,
                None,
                content.into_any_element(),
                cx,
            )
        }
    }

    pub(super) fn render_grid_track_action_button(
        &self,
        id_suffix: impl Into<SharedString>,
        icon: IconName,
        action: DesignPanelAction,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = enabled && self.grid_track_action_is_applicable(&action);
        let mut button = div()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
            .size(px(ROW_HEIGHT))
            .flex_none()
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
                button.text_color(cx.theme().muted_foreground).opacity(0.42)
            });
        if enabled {
            button = button.on_activate(cx.listener(move |this, _, _, cx| {
                if this.grid_track_action_is_applicable(&action) {
                    cx.emit_design_panel_action(this, action.clone());
                }
            }));
        }
        button.child(Icon::new(icon).xsmall()).into_any_element()
    }

    pub(super) fn grid_track_action_is_applicable(&self, action: &DesignPanelAction) -> bool {
        if !self.can_edit()
            || !self.node.supports_grid_auto_layout()
            || !self.node.supports_section(DesignPanelSection::Layout)
        {
            return false;
        }
        let Some(layout) = self
            .node
            .layout
            .as_ref()
            .filter(|layout| layout.mode == DesignLayoutMode::Grid)
        else {
            return false;
        };
        match action {
            DesignPanelAction::GridTrackAddRequested {
                node_id,
                axis,
                insertion_index,
            } => {
                node_id == &self.node.id
                    && layout.grid_track_count_is_editable(*axis)
                    && *insertion_index <= layout.grid_track_count(*axis)
            }
            DesignPanelAction::GridTrackDeleteRequested {
                node_id,
                axis,
                index,
            } => {
                node_id == &self.node.id
                    && layout.can_delete_grid_track(*axis)
                    && *index < layout.grid_track_count(*axis)
            }
            DesignPanelAction::GridTracksReorderRequested {
                node_id,
                axis,
                from_indices,
                insertion_index,
            } => {
                let count = layout.grid_track_count(*axis);
                let unique = from_indices.iter().copied().collect::<HashSet<_>>();
                node_id == &self.node.id
                    && !from_indices.is_empty()
                    && unique.len() == from_indices.len()
                    && from_indices.iter().all(|index| *index < count)
                    && *insertion_index <= count
            }
            _ => false,
        }
    }

    pub(super) fn grid_dimensions_action_is_applicable(&self, action: &DesignPanelAction) -> bool {
        let DesignPanelAction::GridDimensionsEditRequested {
            node_id,
            dimensions,
            phase,
        } = action
        else {
            return false;
        };
        if node_id != &self.node.id || !dimensions.is_valid() {
            return false;
        }
        let Some(layout) = self
            .node
            .layout
            .as_ref()
            .filter(|layout| layout.mode == DesignLayoutMode::Grid)
        else {
            return false;
        };
        let cancel_matches_active = *phase == DesignPanelEditPhase::Cancel
            && self
                .active_grid_dimensions_edit
                .as_ref()
                .is_some_and(|edit| &edit.node_id == node_id);
        let dimensions_are_editable = dimensions.columns == layout.grid_columns.len()
            || self.property_is_editable(DesignPanelProperty::GridColumnCount);
        let rows_are_editable = dimensions.rows == layout.grid_rows.len()
            || self.property_is_editable(DesignPanelProperty::GridRowCount);
        cancel_matches_active
            || (self.can_edit()
                && self.node.supports_grid_auto_layout()
                && self.node.supports_section(DesignPanelSection::Layout)
                && dimensions_are_editable
                && rows_are_editable
                && (layout.grid_auto_tracks == DesignGridAutoTracks::None
                    || dimensions.rows == layout.grid_rows.len()))
    }
}

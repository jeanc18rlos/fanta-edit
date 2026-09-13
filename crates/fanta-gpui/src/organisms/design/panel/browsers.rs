use super::*;

impl DesignPanel {
    pub(super) fn property_variable_target(
        &self,
        property: DesignPanelProperty,
    ) -> Option<DesignPropertyVariableTarget> {
        self.node.property_variable_target(property).map(|target| {
            if target.typography_target.is_some() {
                target.with_typography_target(self.typography_target(property))
            } else {
                target
            }
        })
    }

    pub(super) fn property_variable_can_change(&self, property: DesignPanelProperty) -> bool {
        if !self.can_edit()
            || !self.layout_property_is_applicable(property)
            || self.property_variable_target(property).is_none()
        {
            return false;
        }
        let Some(state) = self.property_value_states.get(&property) else {
            return self.property_is_editable(property);
        };
        if state.is_read_only()
            || state
                .binding()
                .is_some_and(|binding| binding.kind() == DesignPanelBindingKind::Style)
        {
            return false;
        }
        state.binding().is_some() || self.property_is_editable(property)
    }

    pub(super) fn emit_property_variable_apply(
        &mut self,
        property: DesignPanelProperty,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.property_variable_target(property) else {
            return;
        };
        let Some(variable) = self
            .property_variable_view_data
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.property_variable_can_change(property)
            || !variable.is_compatible_with(&target)
            || variable.disabled_reason.is_some()
            || variable.import_state == DesignVariableImportState::Available
        {
            return;
        }
        self.property_variable_picker = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyVariableApplyRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_property_variable_import(
        &mut self,
        property: DesignPanelProperty,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.property_variable_target(property) else {
            return;
        };
        let Some(variable) = self
            .property_variable_view_data
            .variable(variable_id.as_ref())
        else {
            return;
        };
        if !self.property_variable_can_change(property)
            || !variable.is_compatible_with(&target)
            || variable.disabled_reason.is_some()
            || variable.import_state != DesignVariableImportState::Available
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyVariableImportRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
    }

    pub(super) fn emit_property_variable_detach(
        &mut self,
        property: DesignPanelProperty,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.property_variable_target(property) else {
            return;
        };
        let Some(binding) = self
            .property_value_states
            .get(&property)
            .and_then(DesignPanelPropertyValueState::binding)
            .filter(|binding| {
                binding.kind() == DesignPanelBindingKind::Variable
                    && binding.id().as_ref() == variable_id.as_ref()
            })
        else {
            return;
        };
        if !self.property_variable_can_change(property) {
            return;
        }
        let variable_id = binding.id().clone();
        self.property_variable_picker = None;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::PropertyVariableDetachRequested {
                node_id: self.node.id.clone(),
                target,
                variable_id,
            },
        );
        cx.notify();
    }

    pub(super) fn open_property_variable_picker(
        &mut self,
        property: DesignPanelProperty,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.property_variable_target(property).is_none() {
            return;
        }
        self.property_variable_picker = Some(property);
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.active_effect_settings = None;
        self.grid_dimensions_picker = None;
        self.effect_style_browser_open = false;
        self.layout_grid_style_browser_open = false;
        self.layout_grid_count_variable_target = None;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        self.property_variable_search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn render_color_styles_icon(&self, cx: &mut Context<Self>) -> AnyElement {
        render_icon_canvas(cx.theme().foreground, 16., |path| {
            for (center_x, center_y) in [(4., 4.), (12., 4.), (4., 12.), (12., 12.)] {
                path.circle(center_x, center_y, 2.5);
            }
        })
    }

    pub(super) fn normalized_style_browser_query(&self, cx: &App) -> String {
        self.style_browser_search
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase()
    }

    pub(super) fn style_browser_row_matches(
        query: &str,
        name: &str,
        summary: &str,
        library_name: Option<&str>,
    ) -> bool {
        query.is_empty()
            || name.to_ascii_lowercase().contains(query)
            || summary.to_ascii_lowercase().contains(query)
            || library_name.is_some_and(|library| library.to_ascii_lowercase().contains(query))
    }

    pub(super) fn set_style_browser_source_filter(
        &mut self,
        filter: StyleBrowserSourceFilter,
        cx: &mut Context<Self>,
    ) {
        if self.style_browser_source_filter != filter {
            self.style_browser_source_filter = filter;
            cx.notify();
        }
    }

    pub(super) fn set_style_browser_view_mode(
        &mut self,
        view_mode: StyleBrowserViewMode,
        cx: &mut Context<Self>,
    ) {
        if self.style_browser_view_mode != view_mode {
            self.style_browser_view_mode = view_mode;
            cx.notify();
        }
    }

    pub(super) fn render_style_browser_toolbar(
        panel: Entity<Self>,
        scope: SharedString,
        search: Entity<InputState>,
        source_filter: StyleBrowserSourceFilter,
        library_sources: Vec<(SharedString, SharedString)>,
        view_mode: StyleBrowserViewMode,
    ) -> AnyElement {
        let mut source_options = StyleBrowserSourceFilter::general().to_vec();
        for (library_id, library_name) in library_sources {
            if source_options.iter().any(|option| {
                matches!(
                    option,
                    StyleBrowserSourceFilter::Library {
                        library_id: existing,
                        ..
                    } if existing == &library_id
                )
            }) {
                continue;
            }
            source_options.push(StyleBrowserSourceFilter::library(library_id, library_name));
        }
        let mut source_controls = h_flex().min_w(px(0.)).gap_0p5().flex_wrap();
        for filter in source_options {
            let panel = panel.clone();
            let label = filter.label();
            let option_id = match &filter {
                StyleBrowserSourceFilter::Library { library_id, .. } => {
                    format!("library-{library_id}")
                }
                _ => label.to_ascii_lowercase().replace(' ', "-"),
            };
            let selected = source_filter == filter;
            source_controls = source_controls.child(
                Button::new(SharedString::from(format!("{scope}-source-{option_id}")))
                    .label(label.clone())
                    .tooltip(label)
                    .xsmall()
                    .compact()
                    .ghost()
                    .max_w(px(120.))
                    .selected(selected)
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.set_style_browser_source_filter(filter.clone(), cx);
                        });
                    }),
            );
        }
        let mut view_controls = h_flex().gap_0p5();
        for candidate in StyleBrowserViewMode::ALL {
            let panel = panel.clone();
            view_controls = view_controls.child(
                Button::new(SharedString::from(format!(
                    "{scope}-{}",
                    candidate.label().to_ascii_lowercase().replace(' ', "-")
                )))
                .tooltip(candidate.label())
                .xsmall()
                .compact()
                .ghost()
                .w(px(24.))
                .h(px(24.))
                .selected(view_mode == candidate)
                .child(Icon::new(candidate.icon()).xsmall())
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.set_style_browser_view_mode(candidate, cx);
                    });
                }),
            );
        }
        v_flex()
            .w_full()
            .gap_1()
            .child(
                Input::new(&search)
                    .small()
                    .prefix(Icon::new(IconName::Search).small()),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_1()
                    .child(source_controls)
                    .child(view_controls),
            )
            .into_any_element()
    }

    pub(super) fn property_variable_button_state(
        &self,
        property: DesignPanelProperty,
    ) -> Option<PropertyVariableButtonState> {
        Some(PropertyVariableButtonState {
            target: self.property_variable_target(property)?,
            panel_id: self.id.clone(),
            search_input: self.property_variable_search.clone(),
            active: self.property_variable_picker == Some(property),
            can_change: self.property_variable_can_change(property),
            can_edit: self.can_edit(),
            state: self.property_value_states.get(&property).cloned(),
            variable_view_data: self.property_variable_view_data.clone(),
        })
    }

    pub(super) fn render_property_variable_button(
        &self,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Self::render_property_variable_button_for(
            cx.entity(),
            property,
            self.property_variable_button_state(property)?,
            cx,
        )
    }

    pub(super) fn render_property_variable_button_for<T: 'static>(
        panel: Entity<DesignPanel>,
        property: DesignPanelProperty,
        button_state: PropertyVariableButtonState,
        cx: &mut Context<T>,
    ) -> Option<AnyElement> {
        let PropertyVariableButtonState {
            target,
            panel_id,
            search_input,
            active,
            can_change,
            can_edit,
            state,
            variable_view_data,
        } = button_state;
        let panel_for_open = panel.clone();
        let panel_for_content = panel.clone();
        let panel_for_trigger = panel;
        let binding = state
            .as_ref()
            .and_then(DesignPanelPropertyValueState::binding)
            .filter(|binding| binding.kind() == DesignPanelBindingKind::Variable)
            .cloned();
        let style_bound = state
            .as_ref()
            .and_then(DesignPanelPropertyValueState::binding)
            .is_some_and(|binding| binding.kind() == DesignPanelBindingKind::Style);
        let read_only_reason = match &state {
            Some(DesignPanelPropertyValueState::ReadOnly(read_only)) => read_only
                .reason()
                .map(|reason| SharedString::from(reason.to_owned()))
                .or_else(|| Some("Property is read only".into())),
            _ if !can_edit => Some("View only".into()),
            _ if style_bound => Some("Detach the style before binding a variable".into()),
            _ => None,
        };
        let query = search_input.read(cx).value();
        let mut groups: Vec<(DesignVariableSource, Vec<DesignVariable>)> = Vec::new();
        for variable in variable_view_data
            .compatible(&target, query.as_ref())
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
        let fields_label = target
            .fields
            .iter()
            .map(|field| field.api_name())
            .collect::<Vec<_>>()
            .join(", ");
        let trigger_tooltip = binding.as_ref().map_or_else(
            || {
                read_only_reason.clone().unwrap_or_else(|| {
                    SharedString::from(format!("Apply variable · {fields_label}"))
                })
            },
            |binding| binding.name().clone(),
        );
        let trigger = Button::new(SharedString::from(format!(
            "{}-property-variable-{property:?}",
            panel_id
        )))
        .label("◇")
        .tooltip(trigger_tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(20.))
        .h(px(ROW_HEIGHT))
        .selected(active || binding.is_some())
        .on_activate(move |_, window, cx| {
            cx.stop_propagation();
            panel_for_trigger.update(cx, |this, cx| {
                this.open_property_variable_picker(property, window, cx);
            });
        });

        Some(
            Popover::new(SharedString::from(format!(
                "{}-property-variable-popover-{property:?}",
                panel_id
            )))
            .anchor(Anchor::TopRight)
            .open(active)
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                panel_for_open.update(cx, |this, cx| {
                    if *open {
                        this.open_property_variable_picker(property, window, cx);
                    } else if this.property_variable_picker == Some(property) {
                        this.property_variable_picker = None;
                        cx.notify();
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
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(fields_label.clone()),
                            ),
                    )
                    .child(
                        Input::new(&search_input)
                            .small()
                            .prefix(Icon::new(IconName::Search).small()),
                    );
                if let Some(binding) = binding.clone() {
                    let panel = panel_for_content.clone();
                    let variable_id = binding.id().clone();
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
                                    .child(binding.name().clone()),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{panel_id}-detach-property-variable-{property:?}"
                                )))
                                .label("Detach")
                                .tooltip(
                                    read_only_reason
                                        .clone()
                                        .unwrap_or_else(|| "Detach variable".into()),
                                )
                                .xsmall()
                                .compact()
                                .ghost()
                                .disabled(!can_change)
                                .on_activate(move |_, _, cx| {
                                    panel.update(cx, |this, cx| {
                                        this.emit_property_variable_detach(
                                            property,
                                            variable_id.clone(),
                                            cx,
                                        );
                                    });
                                }),
                            ),
                    );
                }
                if let Some(reason) = read_only_reason.clone() {
                    content = content.child(
                        div()
                            .px_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(reason),
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
                    let heading = match &source {
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
                        let variable_id = variable.id.clone();
                        let disabled = !can_change || variable.disabled_reason.is_some();
                        let available =
                            variable.import_state == DesignVariableImportState::Available;
                        let label = if available {
                            format!("Import {} / {}", variable.collection_name, variable.name)
                        } else {
                            format!("{} / {}", variable.collection_name, variable.name)
                        };
                        let selected = binding
                            .as_ref()
                            .is_some_and(|binding| binding.id().as_ref() == variable.id.as_ref());
                        let tooltip = variable.disabled_reason.clone().unwrap_or_else(|| {
                            let import = if available {
                                "Available to import"
                            } else {
                                "Apply"
                            };
                            format!(
                                "{import} · {} · {}",
                                variable.resolved_type.label(),
                                variable
                                    .scopes
                                    .iter()
                                    .map(DesignVariableScope::api_name)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                            .into()
                        });
                        rows = rows.child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-property-variable-{property:?}-{}",
                                variable.id
                            )))
                            .label(label)
                            .tooltip(tooltip)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(selected)
                            .disabled(disabled)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    if available {
                                        this.emit_property_variable_import(
                                            property,
                                            variable_id.clone(),
                                            cx,
                                        );
                                    } else {
                                        this.emit_property_variable_apply(
                                            property,
                                            variable_id.clone(),
                                            cx,
                                        );
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

    pub(super) const fn compact_property_label(property: DesignPanelProperty) -> &'static str {
        match property {
            DesignPanelProperty::Visible
            | DesignPanelProperty::PaintVisible { .. }
            | DesignPanelProperty::EffectVisible(_)
            | DesignPanelProperty::LayoutGridVisible(_) => "Visibility",
            DesignPanelProperty::X => "X position",
            DesignPanelProperty::Y => "Y position",
            DesignPanelProperty::VectorVertexX => "Vertex X",
            DesignPanelProperty::VectorVertexY => "Vertex Y",
            DesignPanelProperty::VectorVertexCornerRadius => "Vertex radius",
            DesignPanelProperty::VectorHandleMirroring => "Handle mirror",
            DesignPanelProperty::SmartSelectionHorizontalSpacing => "Horizontal gap",
            DesignPanelProperty::SmartSelectionVerticalSpacing => "Vertical gap",
            DesignPanelProperty::Width => "Width",
            DesignPanelProperty::Height => "Height",
            DesignPanelProperty::Rotation => "Rotation",
            DesignPanelProperty::LockAspectRatio => "Aspect ratio",
            DesignPanelProperty::HorizontalConstraint => "Horizontal",
            DesignPanelProperty::VerticalConstraint => "Vertical",
            DesignPanelProperty::LayoutMode => "Layout",
            DesignPanelProperty::HorizontalSizing => "Width sizing",
            DesignPanelProperty::VerticalSizing => "Height sizing",
            DesignPanelProperty::AutoLayoutAlignment => "Alignment",
            DesignPanelProperty::AlignmentX => "Horizontal align",
            DesignPanelProperty::AlignmentY => "Vertical align",
            DesignPanelProperty::Wrap => "Wrap",
            DesignPanelProperty::Gap => "Gap",
            DesignPanelProperty::ItemSpacingMode => "Spacing mode",
            DesignPanelProperty::CounterAxisAlignContent => "Line alignment",
            DesignPanelProperty::CounterAxisGap => "Row gap",
            DesignPanelProperty::PaddingVertical => "Vertical padding",
            DesignPanelProperty::PaddingHorizontal => "Horizontal padding",
            DesignPanelProperty::PaddingShorthand => "Padding",
            DesignPanelProperty::PaddingTop => "Top padding",
            DesignPanelProperty::PaddingRight => "Right padding",
            DesignPanelProperty::PaddingBottom => "Bottom padding",
            DesignPanelProperty::PaddingLeft => "Left padding",
            DesignPanelProperty::ClipContent => "Clip content",
            DesignPanelProperty::IncludeStrokes => "Include strokes",
            DesignPanelProperty::StackingOrder => "Stacking",
            DesignPanelProperty::BaselineAlignment => "Baseline",
            DesignPanelProperty::MinWidth => "Min width",
            DesignPanelProperty::MaxWidth => "Max width",
            DesignPanelProperty::MinHeight => "Min height",
            DesignPanelProperty::MaxHeight => "Max height",
            DesignPanelProperty::LayoutPositioning => "Positioning",
            DesignPanelProperty::LayoutAlignSelf => "Self alignment",
            DesignPanelProperty::LayoutGrow => "Grow",
            DesignPanelProperty::GridAutoTracks => "Auto tracks",
            DesignPanelProperty::GridItemsPositioning => "Item positioning",
            DesignPanelProperty::GridColumnCount => "Columns",
            DesignPanelProperty::GridRowCount => "Rows",
            DesignPanelProperty::GridColumnTrack(_) => "Column track",
            DesignPanelProperty::GridRowTrack(_) => "Row track",
            DesignPanelProperty::GridColumnTrackValue(_) => "Column size",
            DesignPanelProperty::GridRowTrackValue(_) => "Row size",
            DesignPanelProperty::GridRowIndex => "Row",
            DesignPanelProperty::GridColumnIndex => "Column",
            DesignPanelProperty::GridRowSpan => "Row span",
            DesignPanelProperty::GridColumnSpan => "Column span",
            DesignPanelProperty::GridHorizontalAlignment => "Horizontal align",
            DesignPanelProperty::GridVerticalAlignment => "Vertical align",
            DesignPanelProperty::Opacity | DesignPanelProperty::PaintOpacity { .. } => "Opacity",
            DesignPanelProperty::BlendMode
            | DesignPanelProperty::EffectShadowBlendMode(_)
            | DesignPanelProperty::EffectNoiseBlendMode(_) => "Blend mode",
            DesignPanelProperty::CornerRadius => "Corner radius",
            DesignPanelProperty::CornerRadiusTopLeft => "Top left",
            DesignPanelProperty::CornerRadiusTopRight => "Top right",
            DesignPanelProperty::CornerRadiusBottomRight => "Bottom right",
            DesignPanelProperty::CornerRadiusBottomLeft => "Bottom left",
            DesignPanelProperty::IndependentCorners => "Corner mode",
            DesignPanelProperty::CornerSmoothing => "Smoothing",
            DesignPanelProperty::FillShowsInExports => "Show in exports",
            DesignPanelProperty::TypographyStyle => "Text style",
            DesignPanelProperty::FontFamily => "Font",
            DesignPanelProperty::FontStyle => "Style",
            DesignPanelProperty::FontWeight => "Weight",
            DesignPanelProperty::FontSize => "Size",
            DesignPanelProperty::LineHeight => "Line height",
            DesignPanelProperty::LetterSpacing => "Letter spacing",
            DesignPanelProperty::TextLeadingTrim => "Leading trim",
            DesignPanelProperty::ParagraphSpacing => "Paragraph spacing",
            DesignPanelProperty::ParagraphIndent => "Paragraph indent",
            DesignPanelProperty::ListSpacing => "List spacing",
            DesignPanelProperty::TextHangingPunctuation => "Hanging punctuation",
            DesignPanelProperty::TextHangingLists => "Hanging lists",
            DesignPanelProperty::HorizontalTextAlignment => "Horizontal align",
            DesignPanelProperty::VerticalTextAlignment => "Vertical align",
            DesignPanelProperty::TextResize => "Resizing",
            DesignPanelProperty::TextTruncate => "Truncation",
            DesignPanelProperty::TextMaxLines => "Max lines",
            DesignPanelProperty::TextDecoration => "Decoration",
            DesignPanelProperty::TextDecorationStyle => "Decoration style",
            DesignPanelProperty::TextDecorationOffset => "Decoration offset",
            DesignPanelProperty::TextDecorationThickness => "Decoration weight",
            DesignPanelProperty::TextDecorationColor => "Decoration color",
            DesignPanelProperty::TextDecorationSkipInk => "Skip ink",
            DesignPanelProperty::TextCase => "Letter case",
            DesignPanelProperty::TextList => "List style",
            DesignPanelProperty::TextPathStartSegment => "Start segment",
            DesignPanelProperty::TextPathStartPosition => "Start position",
            DesignPanelProperty::TextPathOffset => "Start offset",
            DesignPanelProperty::ComponentProperty(_) => "Property",
            DesignPanelProperty::SlotStretchChildOnInsert(_) => "Stretch child",
            DesignPanelProperty::SlotDisplayEmpty(_) => "Show empty slot",
            DesignPanelProperty::SlotMinimumInstances(_) => "Minimum instances",
            DesignPanelProperty::SlotMaximumInstances(_) => "Maximum instances",
            DesignPanelProperty::SlotPreferredValuesOnly(_) => "Preferred only",
            DesignPanelProperty::MediaCropMode => "Crop mode",
            DesignPanelProperty::MediaExposure => "Exposure",
            DesignPanelProperty::MediaContrast => "Contrast",
            DesignPanelProperty::MediaSaturation => "Saturation",
            DesignPanelProperty::MediaTemperature => "Temperature",
            DesignPanelProperty::MediaTint => "Tint",
            DesignPanelProperty::MediaHighlights => "Highlights",
            DesignPanelProperty::MediaShadows => "Shadows",
            DesignPanelProperty::PolygonCount => "Count",
            DesignPanelProperty::StarPointCount => "Count",
            DesignPanelProperty::StarInnerRadius => "Ratio",
            DesignPanelProperty::ArcStartingAngle => "Start",
            DesignPanelProperty::ArcSweep => "Sweep",
            DesignPanelProperty::ArcEndingAngle => "End angle",
            DesignPanelProperty::ArcInnerRadius => "Ratio",
            DesignPanelProperty::BooleanOperation => "Operation",
            DesignPanelProperty::IsMask => "Use as mask",
            DesignPanelProperty::MaskType => "Mask type",
            DesignPanelProperty::TableRows => "Rows",
            DesignPanelProperty::TableColumns => "Columns",
            DesignPanelProperty::SectionContentsHidden => "Contents",
            DesignPanelProperty::SectionDevStatus => "Dev status",
            DesignPanelProperty::TransformRepeatType(_) => "Repeat type",
            DesignPanelProperty::TransformRepeatAxis(_) => "Axis",
            DesignPanelProperty::TransformRepeatCount(_) => "Count",
            DesignPanelProperty::TransformRepeatUnit(_) => "Unit",
            DesignPanelProperty::TransformRepeatOffset(_) => "Offset",
            DesignPanelProperty::SelectionColor(_) => "Color",
            DesignPanelProperty::StrokeWeight => "Weight",
            DesignPanelProperty::StrokeWeightMode => "Weight mode",
            DesignPanelProperty::StrokeWeightTop => "Top weight",
            DesignPanelProperty::StrokeWeightRight => "Right weight",
            DesignPanelProperty::StrokeWeightBottom => "Bottom weight",
            DesignPanelProperty::StrokeWeightLeft => "Left weight",
            DesignPanelProperty::StrokeAlign => "Alignment",
            DesignPanelProperty::StrokeStartCap => "Start cap",
            DesignPanelProperty::StrokeEndCap => "End cap",
            DesignPanelProperty::StrokeEndpointCap => "End points",
            DesignPanelProperty::StrokeDashMode => "Dash mode",
            DesignPanelProperty::StrokeDashPattern => "Dash pattern",
            DesignPanelProperty::StrokeDashCap => "Dash cap",
            DesignPanelProperty::StrokeJoin => "Join",
            DesignPanelProperty::StrokeMiterAngle => "Miter angle",
            DesignPanelProperty::StrokeVariableWidth => "Variable width",
            DesignPanelProperty::StrokeVariableWidthPointPosition(_) => "Point position",
            DesignPanelProperty::StrokeVariableWidthPointWidth(_) => "Point width",
            DesignPanelProperty::StrokeType => "Stroke type",
            DesignPanelProperty::StrokeStretchBrush => "Stretch brush",
            DesignPanelProperty::StrokeBrushDirection => "Direction",
            DesignPanelProperty::StrokeScatterBrush => "Scatter brush",
            DesignPanelProperty::StrokeScatterGap => "Gap",
            DesignPanelProperty::StrokeScatterWiggle => "Wiggle",
            DesignPanelProperty::StrokeScatterSizeJitter => "Size jitter",
            DesignPanelProperty::StrokeScatterAngularJitter => "Angle jitter",
            DesignPanelProperty::StrokeScatterRotation => "Rotation",
            DesignPanelProperty::StrokeDynamicFrequency => "Frequency",
            DesignPanelProperty::StrokeDynamicWiggle => "Wiggle",
            DesignPanelProperty::StrokeDynamicSmoothen => "Smoothen",
            DesignPanelProperty::EffectKind(_) => "Effect type",
            DesignPanelProperty::EffectSettings(_) => "Settings",
            DesignPanelProperty::EffectShadowColor(_) => "Shadow color",
            DesignPanelProperty::EffectShadowBlur(_)
            | DesignPanelProperty::EffectBlur(_)
            | DesignPanelProperty::EffectBlurRadius(_) => "Blur",
            DesignPanelProperty::EffectShadowSpread(_) | DesignPanelProperty::EffectSpread(_) => {
                "Spread"
            }
            DesignPanelProperty::EffectShadowOffsetX(_) | DesignPanelProperty::EffectOffsetX(_) => {
                "X offset"
            }
            DesignPanelProperty::EffectShadowOffsetY(_) | DesignPanelProperty::EffectOffsetY(_) => {
                "Y offset"
            }
            DesignPanelProperty::EffectDropShadowShowBehindNode(_) => "Show behind",
            DesignPanelProperty::EffectBlurType(_) => "Blur type",
            DesignPanelProperty::EffectProgressiveBlurStartRadius(_) => "Start radius",
            DesignPanelProperty::EffectProgressiveBlurEndRadius(_) => "End radius",
            DesignPanelProperty::EffectProgressiveBlurStartOffsetX(_) => "Start X",
            DesignPanelProperty::EffectProgressiveBlurStartOffsetY(_) => "Start Y",
            DesignPanelProperty::EffectProgressiveBlurEndOffsetX(_) => "End X",
            DesignPanelProperty::EffectProgressiveBlurEndOffsetY(_) => "End Y",
            DesignPanelProperty::EffectNoiseType(_) => "Noise type",
            DesignPanelProperty::EffectNoisePrimaryColor(_) => "Primary color",
            DesignPanelProperty::EffectNoiseSecondaryColor(_) => "Secondary color",
            DesignPanelProperty::EffectNoiseOpacity(_) => "Noise opacity",
            DesignPanelProperty::EffectNoiseSizeX(_) => "Width",
            DesignPanelProperty::EffectNoiseSizeY(_) => "Height",
            DesignPanelProperty::EffectNoiseDensity(_) => "Density",
            DesignPanelProperty::EffectTextureSizeX(_) => "Texture width",
            DesignPanelProperty::EffectTextureSizeY(_) => "Texture height",
            DesignPanelProperty::EffectTextureRadius(_) => "Texture radius",
            DesignPanelProperty::EffectTextureClipToShape(_) => "Clip to shape",
            DesignPanelProperty::EffectGlassLightIntensity(_) => "Light intensity",
            DesignPanelProperty::EffectGlassLightAngle(_) => "Light angle",
            DesignPanelProperty::EffectGlassRefraction(_) => "Refraction",
            DesignPanelProperty::EffectGlassDepth(_) => "Depth",
            DesignPanelProperty::EffectGlassDispersion(_) => "Dispersion",
            DesignPanelProperty::EffectGlassFrost(_) => "Frost",
            DesignPanelProperty::EffectGlassSplay(_) => "Splay",
            DesignPanelProperty::EffectShaderProperty(_, _) => "Shader property",
            DesignPanelProperty::LayoutGridKind(_) => "Guide type",
            DesignPanelProperty::LayoutGridAlignment(_) => "Alignment",
            DesignPanelProperty::LayoutGridCount(_) => "Count",
            DesignPanelProperty::LayoutGridSize(_) => "Size",
            DesignPanelProperty::LayoutGridOffset(_) => "Offset",
            DesignPanelProperty::LayoutGridGutter(_) => "Gutter",
            DesignPanelProperty::LayoutGridMargin(_) => "Margin",
            DesignPanelProperty::LayoutGridColor(_) => "Color",
            DesignPanelProperty::LayoutGridOpacity(_) => "Opacity",
            DesignPanelProperty::ExportSizing(_) | DesignPanelProperty::ExportScale(_) => "Size",
            DesignPanelProperty::ExportSuffix(_) => "Suffix",
            DesignPanelProperty::ExportFormat(_) => "Format",
            DesignPanelProperty::AlignSelection => "Alignment",
            DesignPanelProperty::DistributeSelection => "Distribution",
        }
    }

    pub(super) fn render_bound_style_summary(
        &self,
        id_suffix: &'static str,
        name: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .id(SharedString::from(format!(
                "{}-{id_suffix}-bound-style",
                self.id
            )))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .rounded(px(4.))
            .bg(cx.theme().secondary)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .child(name),
            )
            .into_any_element()
    }
}

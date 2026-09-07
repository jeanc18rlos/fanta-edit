use super::*;

impl DesignPanel {
    pub(super) fn clear_option_interactions(&mut self) {
        self.option_states.clear();
        self.option_subscriptions.clear();
        self.option_snapshots.clear();
    }

    pub(super) fn option_properties(&self) -> Vec<DesignPanelProperty> {
        let mut properties = vec![
            DesignPanelProperty::HorizontalConstraint,
            DesignPanelProperty::VerticalConstraint,
            DesignPanelProperty::HorizontalSizing,
            DesignPanelProperty::VerticalSizing,
            DesignPanelProperty::ItemSpacingMode,
            DesignPanelProperty::CounterAxisAlignContent,
            DesignPanelProperty::LayoutPositioning,
            DesignPanelProperty::LayoutAlignSelf,
            DesignPanelProperty::StackingOrder,
            DesignPanelProperty::BaselineAlignment,
            DesignPanelProperty::GridItemsPositioning,
            DesignPanelProperty::GridHorizontalAlignment,
            DesignPanelProperty::GridVerticalAlignment,
            DesignPanelProperty::BlendMode,
            DesignPanelProperty::FontStyle,
            DesignPanelProperty::LineHeight,
            DesignPanelProperty::LetterSpacing,
            DesignPanelProperty::HorizontalTextAlignment,
            DesignPanelProperty::VerticalTextAlignment,
            DesignPanelProperty::TextResize,
            DesignPanelProperty::TextDecoration,
            DesignPanelProperty::TextCase,
            DesignPanelProperty::TextList,
            DesignPanelProperty::BooleanOperation,
            DesignPanelProperty::MaskType,
            DesignPanelProperty::SectionDevStatus,
        ];
        if self.active_vector_edit().is_some() {
            properties.push(DesignPanelProperty::VectorHandleMirroring);
        }
        properties.extend(
            (0..self.node.component_properties.len()).map(DesignPanelProperty::ComponentProperty),
        );
        if let Some(layout) = self.node.layout.as_ref() {
            properties
                .extend((0..layout.grid_columns.len()).map(DesignPanelProperty::GridColumnTrack));
            properties.extend((0..layout.grid_rows.len()).map(DesignPanelProperty::GridRowTrack));
        }
        if let Some(stroke) = self.node.stroke.as_ref() {
            if stroke.capabilities.individual_weights {
                properties.push(DesignPanelProperty::StrokeWeightMode);
            }
            if stroke.capabilities.position && stroke.complex_stroke.is_basic() {
                properties.push(DesignPanelProperty::StrokeAlign);
            }
            if stroke.complex_stroke.is_basic() {
                match stroke.edit_context.endpoint_control() {
                    DesignStrokeEndpointControl::None => {}
                    DesignStrokeEndpointControl::StartAndEnd => properties.extend([
                        DesignPanelProperty::StrokeStartCap,
                        DesignPanelProperty::StrokeEndCap,
                    ]),
                    DesignStrokeEndpointControl::Aggregate
                    | DesignStrokeEndpointControl::SelectedVertices => {
                        properties.push(DesignPanelProperty::StrokeEndpointCap);
                    }
                }
                properties.push(DesignPanelProperty::StrokeDashMode);
                if !stroke.dashes.is_solid() {
                    properties.push(DesignPanelProperty::StrokeDashCap);
                }
                if stroke.capabilities.joins {
                    properties.push(DesignPanelProperty::StrokeJoin);
                }
            }
            if stroke.supports_variable_width() {
                properties.push(DesignPanelProperty::StrokeVariableWidth);
            }
            if stroke.capabilities.complex_stroke {
                properties.push(DesignPanelProperty::StrokeType);
                match &stroke.complex_stroke {
                    DesignComplexStroke::StretchBrush(_) => properties.extend([
                        DesignPanelProperty::StrokeStretchBrush,
                        DesignPanelProperty::StrokeBrushDirection,
                    ]),
                    DesignComplexStroke::ScatterBrush(_) => {
                        properties.push(DesignPanelProperty::StrokeScatterBrush);
                    }
                    DesignComplexStroke::Basic
                    | DesignComplexStroke::Dynamic(_)
                    | DesignComplexStroke::Opaque(_) => {}
                }
            }
        }
        for (index, modifier) in self.node.transform_modifiers.iter().enumerate() {
            properties.extend([
                DesignPanelProperty::TransformRepeatType(index),
                DesignPanelProperty::TransformRepeatUnit(index),
            ]);
            if matches!(modifier.mode, DesignRepeatMode::Linear(_)) {
                properties.push(DesignPanelProperty::TransformRepeatAxis(index));
            }
        }
        for (index, effect) in self.node.effects.iter().enumerate() {
            properties.push(DesignPanelProperty::EffectKind(index));
            match &effect.settings {
                DesignEffectSettings::DropShadow(_) | DesignEffectSettings::InnerShadow(_) => {
                    properties.push(DesignPanelProperty::EffectShadowBlendMode(index));
                }
                DesignEffectSettings::LayerBlur(_) | DesignEffectSettings::BackgroundBlur(_) => {
                    properties.push(DesignPanelProperty::EffectBlurType(index));
                }
                DesignEffectSettings::Noise(_) => {
                    properties.extend([
                        DesignPanelProperty::EffectNoiseType(index),
                        DesignPanelProperty::EffectNoiseBlendMode(index),
                    ]);
                }
                DesignEffectSettings::Texture(_)
                | DesignEffectSettings::Glass(_)
                | DesignEffectSettings::Shader(_)
                | DesignEffectSettings::Opaque(_) => {}
            }
        }
        for index in 0..self.node.layout_grids.len() {
            properties.push(DesignPanelProperty::LayoutGridKind(index));
            properties.push(DesignPanelProperty::LayoutGridAlignment(index));
        }
        properties
            .extend((0..self.export_configurations().len()).map(DesignPanelProperty::ExportFormat));
        properties.retain(|property| self.property_options(*property).is_some());
        properties
    }

    pub(super) fn sync_option_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let properties = self.option_properties();
        let desired = properties.iter().copied().collect::<HashSet<_>>();
        self.option_states
            .retain(|property, _| desired.contains(property));
        self.option_subscriptions
            .retain(|property, _| desired.contains(property));
        self.option_snapshots
            .retain(|property, _| desired.contains(property));

        for property in properties {
            let Some(options) = self.property_options(property) else {
                continue;
            };
            let current = self.resolved_property_value(property);
            let snapshot = PropertyOptionSnapshot {
                options: options.clone(),
                selected: current.clone(),
            };
            if self.option_snapshots.get(&property) == Some(&snapshot) {
                continue;
            }
            let selected_index = current
                .as_ref()
                .and_then(|current| options.iter().position(|option| &option.value == current))
                .map(|index| IndexPath::default().row(index));

            if let Some(state) = self.option_states.get(&property).cloned() {
                state.update(cx, |state, cx| {
                    state.set_items(options, window, cx);
                    state.set_selected_index(selected_index, window, cx);
                });
                self.option_snapshots.insert(property, snapshot);
                continue;
            }

            let state = cx.new(|cx| SelectState::new(options, selected_index, window, cx));
            let subscription = cx.subscribe_in(
                &state,
                window,
                move |this, state, event: &SelectEvent<Vec<PropertyOption>>, window, cx| {
                    let SelectEvent::Confirm(Some(value)) = event else {
                        return;
                    };
                    this.emit_property(property, value.clone(), cx);
                    let state = state.clone();
                    cx.defer_in(window, move |this, window, cx| {
                        let controlled = this.resolved_property_value(property);
                        state.update(cx, |state, cx| {
                            if let Some(controlled) = controlled.as_ref() {
                                state.set_selected_value(controlled, window, cx);
                            } else {
                                state.set_selected_index(None, window, cx);
                            }
                        });
                    });
                },
            );
            self.option_states.insert(property, state);
            self.option_subscriptions.insert(property, subscription);
            self.option_snapshots.insert(property, snapshot);
        }
    }

    pub(super) fn render_compact_property_label(
        &self,
        id_suffix: &SharedString,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = SharedString::from(format!("{}-{id_suffix}-additional-label", self.id));
        let debug_selector = selector.to_string();
        div()
            .id(selector)
            .debug_selector(move || debug_selector)
            .min_w(px(0.))
            .max_w(px(64.))
            .overflow_hidden()
            .whitespace_nowrap()
            .truncate()
            .text_size(px(10.))
            .text_color(cx.theme().muted_foreground)
            .child(Self::compact_property_label(property))
            .into_any_element()
    }

    pub(super) fn render_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_value_cell_with_left_padding(id_suffix, prefix, value, property, next, 8., cx)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_value_cell_with_left_padding(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        left_padding: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let value = self.display_property_value(property, value.into());
        let editable = self.property_is_editable(property);
        let copyable = self.generic_property_is_copyable(property);
        let interactive = editable || copyable;
        if editable && let Some(options) = self.property_options(property) {
            return self.render_option_cell(id_suffix, prefix, value, property, options, cx);
        }

        let editing = editable
            && self
                .property_editor
                .as_ref()
                .is_some_and(|editor| editor.property == property);
        let scrubbing = self.numeric_scrub_is_active(property);
        let scrub_enabled = self.numeric_scrub_surface_is_enabled(property);
        let copy_value = value.clone();
        let cell_id = SharedString::from(format!("{}-{}", self.id, id_suffix));
        let debug_selector = cell_id.to_string();
        let retained_focus = self
            .editor_focus_return
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
        let mut cell = h_flex()
            .id(cell_id)
            .debug_selector(move || debug_selector)
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .pl(px(left_padding))
            .pr_2()
            .gap_1()
            .rounded(px(4.))
            .border_1()
            .border_color(if editing && self.property_editor_invalid {
                cx.theme().red
            } else if editing {
                cx.theme().selection
            } else {
                cx.theme().transparent
            })
            .bg(cx.theme().secondary)
            .when(interactive, |cell| {
                cell.key_context(CONTROL_KEY_CONTEXT)
                    .cursor_pointer()
                    .hover(|style| style.border_color(cx.theme().muted_foreground))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!interactive, |cell| {
                cell.text_color(cx.theme().muted_foreground).opacity(0.78)
            });
        if interactive {
            cell = if let Some(focus) = retained_focus {
                cell.track_focus(&focus.tab_index(0).tab_stop(true))
            } else {
                cell.tab_index(0)
            };
        }
        if editable {
            cell = cell.on_activate(cx.listener(move |this, _, window, cx| {
                this.activate_property_from_control(
                    EditorFocusOrigin::ValueCell(property),
                    property,
                    next.clone(),
                    window,
                    cx,
                );
            }));
        } else if copyable {
            cell = cell.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property_copy(property, copy_value.clone(), cx);
            }));
        }
        if editing && !scrubbing {
            if self.additional_labels {
                cell = cell.child(self.render_compact_property_label(&id_suffix, property, cx));
            }
            cell = cell.child(
                Input::new(&self.property_input)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
                    .xsmall()
                    .h(px(ROW_HEIGHT - 2.))
                    .flex_1()
                    .min_w(px(0.)),
            );
        } else {
            let mut readout = h_flex().h_full().flex_1().min_w(px(0.)).gap_1();
            if !prefix.is_empty() {
                readout = readout.child(
                    div()
                        .w(px(12.))
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(prefix),
                );
            }
            if self.additional_labels {
                readout =
                    readout.child(self.render_compact_property_label(&id_suffix, property, cx));
            }
            readout = readout.child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .child(value),
            );
            cell = cell.child(render_numeric_scrub_surface(
                SharedString::from(format!("{}-{id_suffix}-scrub", self.id)),
                cx.entity(),
                property,
                EditorFocusOrigin::ValueCell(property),
                scrub_enabled,
                readout.into_any_element(),
            ));
        }
        if let Some(variable_button) = self.render_property_variable_button(property, cx) {
            cell = cell.child(variable_button);
        }
        cell.into_any_element()
    }

    pub(super) fn render_value_field_icon(
        &self,
        icon: ValueFieldIcon,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = cx.theme().muted_foreground;
        render_icon_canvas(color, 16., move |path| match icon {
            ValueFieldIcon::Opacity => {
                path.rect(2.5, 2.5, 13.5, 13.5);
                for dot_y in [5., 8., 11.] {
                    for dot_x in [5., 8., 11.] {
                        path.line((dot_x - 0.25, dot_y), (dot_x + 0.25, dot_y));
                    }
                }
            }
            ValueFieldIcon::Corners => {
                path.poly([(6., 2.5), (2.5, 2.5), (2.5, 6.)], false);
                path.poly([(10., 2.5), (13.5, 2.5), (13.5, 6.)], false);
                path.poly([(2.5, 10.), (2.5, 13.5), (6., 13.5)], false);
                path.poly([(13.5, 10.), (13.5, 13.5), (10., 13.5)], false);
            }
        })
    }

    pub(super) fn render_icon_value_cell(
        &self,
        id_suffix: &'static str,
        icon: ValueFieldIcon,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(property);
        let labeled_cell_selector =
            SharedString::from(format!("{}-{id_suffix}-additional-label", self.id));
        let labeled_cell_debug_selector = labeled_cell_selector.to_string();
        let prefix = div()
            .w(px(24.))
            .h(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(!editable, |prefix| prefix.opacity(0.62))
            .child(self.render_value_field_icon(icon, cx));

        h_flex()
            .id(SharedString::from(format!(
                "{}-{id_suffix}-icon-value-cell",
                self.id
            )))
            .when(self.additional_labels, |cell| {
                cell.debug_selector(move || labeled_cell_debug_selector)
            })
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .overflow_hidden()
            .rounded(px(4.))
            .bg(cx.theme().secondary)
            .child(prefix)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(self.render_value_cell_with_left_padding(
                        format!("{id_suffix}-value"),
                        "",
                        value,
                        property,
                        next,
                        0.,
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_option_cell(
        &self,
        id_suffix: SharedString,
        prefix: &'static str,
        value: SharedString,
        property: DesignPanelProperty,
        options: Vec<PropertyOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if Self::property_supports_menu_preview(property) {
            return self
                .render_preview_option_cell(id_suffix, prefix, value, property, options, cx);
        }
        let additional_label = self
            .additional_labels
            .then(|| Self::compact_property_label(property));
        let additional_label_selector =
            SharedString::from(format!("{}-{id_suffix}-additional-label", self.id));
        let Some(state) = self.option_states.get(&property) else {
            let mut cell = h_flex()
                .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
                .h(px(ROW_HEIGHT))
                .flex_1()
                .min_w(px(0.))
                .px_2()
                .gap_2()
                .rounded(px(4.))
                .border_1()
                .border_color(cx.theme().transparent)
                .bg(cx.theme().secondary)
                .child(
                    div()
                        .w(px(14.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(prefix),
                );
            if additional_label.is_some() {
                cell = cell.child(self.render_compact_property_label(&id_suffix, property, cx));
            }
            return cell
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .child(value),
                )
                .when_some(
                    self.render_property_variable_button(property, cx),
                    |cell, button| cell.child(button),
                )
                .into_any_element();
        };

        let title_prefix = if let Some(label) = additional_label.as_ref() {
            if prefix.is_empty() {
                format!("{label}  ")
            } else {
                format!("{prefix}  {label}  ")
            }
        } else {
            format!("{prefix}  ")
        };
        let placeholder = if let Some(label) = additional_label.as_ref() {
            if prefix.is_empty() {
                format!("{label}  {value}").into()
            } else {
                format!("{prefix}  {label}  {value}").into()
            }
        } else {
            value
        };
        let additional_debug_selector = additional_label_selector.to_string();
        h_flex()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .when(self.additional_labels, |cell| {
                cell.debug_selector(move || additional_debug_selector)
            })
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .child(
                div().flex_1().min_w(px(0.)).child(
                    Select::new(state)
                        .xsmall()
                        .w_full()
                        .h(px(ROW_HEIGHT))
                        .menu_width(px(168.))
                        .title_prefix(title_prefix)
                        .placeholder(placeholder),
                ),
            )
            .when_some(
                self.render_property_variable_button(property, cx),
                |cell, button| cell.child(button),
            )
            .into_any_element()
    }

    pub(super) fn render_preview_option_cell(
        &self,
        id_suffix: SharedString,
        prefix: &'static str,
        value: SharedString,
        property: DesignPanelProperty,
        options: Vec<PropertyOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.preview_option_menu_open == Some(property);
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let current = self.resolved_property_value(property);
        let label = if self.additional_labels {
            let compact = Self::compact_property_label(property);
            if prefix.is_empty() {
                format!("{compact}  {value}")
            } else {
                format!("{prefix}  {compact}  {value}")
            }
        } else if prefix.is_empty() {
            value.to_string()
        } else {
            format!("{prefix}  {value}")
        };
        let trigger_selector =
            SharedString::from(format!("{}-{id_suffix}-preview-menu-trigger", self.id));
        let trigger_debug_selector = trigger_selector.to_string();
        let cell_id = SharedString::from(format!("{}-{id_suffix}", self.id));
        let trigger = Button::new(trigger_selector)
            .debug_selector(move || trigger_debug_selector.clone())
            .label(SharedString::from(label))
            .dropdown_caret(true)
            .xsmall()
            .compact()
            .outline()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(!self.property_is_editable(property))
            .on_keyboard_activate({
                let panel = panel_for_open.clone();
                move |_, cx| {
                    panel.update(cx, |this, cx| {
                        if !open {
                            this.cancel_menu_preview(cx);
                            this.preview_option_menu_open = Some(property);
                        } else if this.preview_option_menu_open == Some(property) {
                            this.cancel_menu_preview_for_property(property, cx);
                            this.preview_option_menu_open = None;
                        }
                        cx.notify();
                    });
                }
            });

        let popover =
            Popover::new(SharedString::from(format!(
                "{}-{id_suffix}-preview-menu",
                self.id
            )))
            .anchor(Anchor::BottomLeft)
            .open(open)
            .overlay_closable(true)
            .on_open_change(move |is_open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    if *is_open {
                        this.cancel_menu_preview(cx);
                        this.preview_option_menu_open = Some(property);
                    } else if this.preview_option_menu_open == Some(property) {
                        this.cancel_menu_preview_for_property(property, cx);
                        this.preview_option_menu_open = None;
                    }
                    cx.notify();
                });
            })
            .trigger(trigger)
            .content(move |_, window, _| {
                v_flex()
                    .w(popup_width(window, 184.))
                    .max_h(popup_height(window, 336.))
                    .overflow_y_scrollbar()
                    .gap_0p5()
                    .p_1()
                    .children(options.clone().into_iter().enumerate().map(
                        |(option_index, option)| {
                            let panel = panel_for_content.clone();
                            let panel_for_hover = panel.clone();
                            let panel_for_key = panel.clone();
                            let candidate = option.value;
                            let hover_candidate = candidate.clone();
                            let key_candidate = candidate.clone();
                            let option_selector = SharedString::from(format!(
                                "{panel_id}-{id_suffix}-preview-option-{option_index}"
                            ));
                            let option_debug_selector = option_selector.to_string();
                            Button::new(option_selector)
                                .debug_selector(move || option_debug_selector.clone())
                                .label(option.label)
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .selected(current.as_ref() == Some(&candidate))
                                .on_hover(move |hovered, _, cx| {
                                    panel_for_hover.update(cx, |this, cx| {
                                        this.set_property_menu_preview(
                                            property,
                                            hover_candidate.clone(),
                                            *hovered,
                                            cx,
                                        );
                                    });
                                })
                                .on_activate(move |_, _, cx| {
                                    panel.update(cx, |this, cx| {
                                        this.cancel_menu_preview(cx);
                                        this.preview_option_menu_open = None;
                                        this.emit_property(property, candidate.clone(), cx);
                                        cx.notify();
                                    });
                                })
                                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                    panel_for_key.update(cx, |this, cx| {
                                        this.set_property_menu_preview(
                                            property,
                                            key_candidate.clone(),
                                            true,
                                            cx,
                                        );
                                        match event.keystroke.key.as_str() {
                                            "enter" | "space" => {
                                                this.cancel_menu_preview(cx);
                                                this.preview_option_menu_open = None;
                                                this.emit_property(
                                                    property,
                                                    key_candidate.clone(),
                                                    cx,
                                                );
                                                window.prevent_default();
                                                cx.stop_propagation();
                                            }
                                            "escape" => {
                                                this.cancel_menu_preview(cx);
                                                this.preview_option_menu_open = None;
                                                window.prevent_default();
                                                cx.stop_propagation();
                                            }
                                            "tab" => {
                                                this.cancel_menu_preview(cx);
                                            }
                                            _ => {}
                                        }
                                    });
                                })
                        },
                    ))
            });

        h_flex()
            .id(cell_id)
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .child(div().flex_1().min_w(px(0.)).child(popover))
            .when_some(
                self.render_property_variable_button(property, cx),
                |cell, button| cell.child(button),
            )
            .into_any_element()
    }

    pub(super) fn render_toggle_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(property);
        let selector = SharedString::from(format!("{}-{}", self.id, id_suffix.into()));
        let debug_selector = selector.to_string();
        let mut row = h_flex()
            .id(selector)
            .debug_selector(move || debug_selector.clone())
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
            row = row.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property(property, DesignPanelValue::Bool(!checked), cx);
            }));
        }
        row.child(div().flex_1().text_xs().child(label))
            .when_some(
                self.render_property_variable_button(property, cx),
                |row, button| row.child(button),
            )
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

    pub(super) fn render_group_label(
        &self,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .h(px(16.))
            .flex()
            .items_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .into_any_element()
    }

    pub(super) fn render_checkbox_row(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(property);
        let panel = cx.entity();
        let id_suffix = id_suffix.into();
        // Contract (§16): the pointer path stays on the Checkbox's own
        // `on_click`, which carries the next `&bool` payload; this wrapper adds
        // the Enter/Space command path so the row is not a dead tab stop.
        div()
            .id(SharedString::from(format!("{}-{id_suffix}-row", self.id)))
            .h(px(ROW_HEIGHT))
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .when(editable, |row| {
                row.key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .focus(|style| style.border_color(cx.theme().selection))
                    .on_action(cx.listener(move |this, _: &ActivateControl, _, cx| {
                        this.emit_property(property, DesignPanelValue::Bool(!checked), cx);
                    }))
            })
            .child(
                Checkbox::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
                    .xsmall()
                    .label(label)
                    .checked(checked)
                    .disabled(!editable)
                    .tab_stop(false)
                    .on_click(move |next, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_property(property, DesignPanelValue::Bool(*next), cx);
                        });
                    })
                    .h(px(ROW_HEIGHT)),
            )
            .into_any_element()
    }

    pub(super) fn render_remove_button(
        &self,
        id_suffix: impl Into<SharedString>,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_remove = self.collection_is_supported(collection)
            && (!matches!(
                collection,
                DesignPanelCollection::Fill | DesignPanelCollection::Stroke
            ) || self.paint_style_binding(collection).is_none())
            && (collection != DesignPanelCollection::Effect
                || self.node.effect_style_binding.is_none())
            && (collection != DesignPanelCollection::LayoutGrid
                || self.node.layout_grid_style_binding.is_none());
        let can_remove = can_remove
            && if collection == DesignPanelCollection::Export {
                self.can_export()
            } else {
                self.can_edit()
            };
        if !can_remove {
            return div().size(px(24.)).flex_none().into_any_element();
        }
        div()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| {
                style
                    .bg(cx.theme().accent)
                    .border_1()
                    .border_color(cx.theme().selection)
            })
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_remove(collection, index, cx);
            }))
            .child(Icon::new(IconName::Minus).xsmall())
            .into_any_element()
    }

    pub(super) fn menu_preview_effect_index(property: DesignPanelProperty) -> Option<usize> {
        match property {
            DesignPanelProperty::EffectKind(index)
            | DesignPanelProperty::EffectShadowBlendMode(index)
            | DesignPanelProperty::EffectNoiseBlendMode(index)
            | DesignPanelProperty::EffectBlurType(index)
            | DesignPanelProperty::EffectNoiseType(index) => Some(index),
            _ => None,
        }
    }

    pub(super) fn property_supports_menu_preview(property: DesignPanelProperty) -> bool {
        matches!(
            property,
            DesignPanelProperty::BlendMode
                | DesignPanelProperty::HorizontalSizing
                | DesignPanelProperty::VerticalSizing
                | DesignPanelProperty::StrokeAlign
        ) || Self::menu_preview_effect_index(property).is_some()
    }

    pub(super) fn menu_preview_for_property(
        &self,
        property: DesignPanelProperty,
        candidate: DesignPanelValue,
    ) -> Option<DesignMenuPreview> {
        if !Self::property_supports_menu_preview(property)
            || !self.property_is_editable(property)
            || !self.layout_property_value_is_applicable(property, &candidate)
        {
            return None;
        }
        let original = self.resolved_property_value(property)?;
        if original == candidate {
            return None;
        }
        if let Some(index) = Self::menu_preview_effect_index(property) {
            if self.inspection_context.selection().kind() != DesignPanelSelectionKind::Single {
                return None;
            }
            let effect = self.node.effects.get(index)?;
            return Some(DesignMenuPreview::EffectProperty {
                node_id: self.node.id.clone(),
                effect_id: effect.id.clone(),
                index,
                property,
                original,
                candidate,
            });
        }
        let target = self.command_target();
        matches!(target, DesignPanelTarget::Nodes { .. }).then_some(
            DesignMenuPreview::NodeProperty {
                target,
                property,
                original,
                candidate,
            },
        )
    }

    pub(super) fn begin_menu_preview(
        &mut self,
        preview: DesignMenuPreview,
        cx: &mut Context<Self>,
    ) {
        if self.active_menu_preview.as_ref() == Some(&preview) {
            return;
        }
        self.cancel_menu_preview(cx);
        self.active_menu_preview = Some(preview.clone());
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::MenuPreviewRequested {
                preview,
                phase: DesignMenuPreviewPhase::Begin,
            },
        );
    }

    pub(super) fn cancel_menu_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.active_menu_preview.take() else {
            return;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::MenuPreviewRequested {
                preview,
                phase: DesignMenuPreviewPhase::End,
            },
        );
    }

    pub(super) fn menu_preview_matches_property_candidate(
        preview: &DesignMenuPreview,
        property: DesignPanelProperty,
        candidate: &DesignPanelValue,
    ) -> bool {
        match preview {
            DesignMenuPreview::NodeProperty {
                property: active_property,
                candidate: active_candidate,
                ..
            }
            | DesignMenuPreview::EffectProperty {
                property: active_property,
                candidate: active_candidate,
                ..
            } => *active_property == property && active_candidate == candidate,
            DesignMenuPreview::PaintProperty { .. } => false,
        }
    }

    pub(super) fn set_property_menu_preview(
        &mut self,
        property: DesignPanelProperty,
        candidate: DesignPanelValue,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            if self.active_menu_preview.as_ref().is_some_and(|active| {
                Self::menu_preview_matches_property_candidate(active, property, &candidate)
            }) {
                self.cancel_menu_preview(cx);
            }
            return;
        }
        if let Some(preview) = self.menu_preview_for_property(property, candidate) {
            self.begin_menu_preview(preview, cx);
        }
    }

    pub(super) fn cancel_menu_preview_for_property(
        &mut self,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) {
        if self.active_menu_preview.as_ref().is_some_and(|preview| {
            matches!(
                preview,
                DesignMenuPreview::NodeProperty {
                    property: active_property,
                    ..
                } | DesignMenuPreview::EffectProperty {
                    property: active_property,
                    ..
                } if *active_property == property
            )
        }) {
            self.cancel_menu_preview(cx);
        }
    }

    pub(super) fn begin_paint_blend_mode_preview(
        &mut self,
        target: &PickerEventTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        cx: &mut Context<Self>,
    ) {
        let panel_target = PaintPickerTarget {
            collection: target.collection,
            index: target.index,
            paint_id: target.paint_id.clone(),
        };
        let picker_is_exact = self.active_picker.as_ref().is_some_and(|active| {
            active.collection == target.collection
                && active.index == target.index
                && active.paint_id == target.paint_id
        });
        let paint = self
            .paint_collection(target.collection)
            .and_then(|paints| paints.get(target.index));
        let paint_is_exact = paint.is_some_and(|paint| {
            (target.paint_id.is_empty() || paint.id == target.paint_id)
                && paint.blend_mode == original
                && !paint.read_only
        });
        if target.node_id != self.node.id
            || self.inspection_context.selection().kind() != DesignPanelSelectionKind::Single
            || !picker_is_exact
            || !paint_is_exact
            || self.paint_target_index(&panel_target) != Some(target.index)
            || !self.can_edit()
            || !self.collection_is_supported(target.collection)
            || self.paint_style_binding(target.collection).is_some()
            || original == candidate
            || candidate == DesignBlendMode::PassThrough
        {
            return;
        }
        self.begin_menu_preview(
            DesignMenuPreview::PaintProperty {
                node_id: target.node_id.clone(),
                collection: target.collection,
                target: self.paint_target(target.collection),
                paint_id: target.paint_id.clone(),
                index: target.index,
                property: DesignPaintProperty::BlendMode,
                original: DesignPaintValue::BlendMode(original),
                candidate: DesignPaintValue::BlendMode(candidate),
            },
            cx,
        );
    }

    pub(super) fn end_paint_blend_mode_preview(
        &mut self,
        target: &PickerEventTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        cx: &mut Context<Self>,
    ) {
        let matches = self.active_menu_preview.as_ref().is_some_and(|preview| {
            matches!(
                preview,
                DesignMenuPreview::PaintProperty {
                    node_id,
                    collection,
                    paint_id,
                    index,
                    property: DesignPaintProperty::BlendMode,
                    original: DesignPaintValue::BlendMode(active_original),
                    candidate: DesignPaintValue::BlendMode(active_candidate),
                    ..
                } if node_id == &target.node_id
                    && *collection == target.collection
                    && paint_id == &target.paint_id
                    && *index == target.index
                    && *active_original == original
                    && *active_candidate == candidate
            )
        });
        if matches {
            self.cancel_menu_preview(cx);
        }
    }

    pub(super) fn render_action_button(
        &self,
        id_suffix: &'static str,
        label: &'static str,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.can_edit() {
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
        div()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
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
            .on_activate(cx.listener(move |this, _, _, cx| {
                cx.emit_design_panel_action(this, action.clone());
            }))
            .child(label)
            .into_any_element()
    }

    pub(super) fn render_icon_action_button(
        &self,
        id_suffix: &'static str,
        icon: IconName,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_edit() && self.node_capability_allows_action(&action);
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
                if this.node_capability_allows_action(&action) {
                    cx.emit_design_panel_action(this, action.clone());
                }
            }));
        }
        button.child(Icon::new(icon).xsmall()).into_any_element()
    }

    pub(super) fn render_symbol_action_button(
        &self,
        id_suffix: &'static str,
        symbol: &'static str,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_edit() && self.node_capability_allows_action(&action);
        let mut button = div()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
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
            button = button.on_activate(cx.listener(move |this, _, _, cx| {
                if this.node_capability_allows_action(&action) {
                    cx.emit_design_panel_action(this, action.clone());
                }
            }));
        }
        button.child(symbol).into_any_element()
    }
}

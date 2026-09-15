use super::*;

/// Internal option-field controller used by the Design facade and its
/// extracted section renderers.
pub(super) trait DesignOptionsController: Sized {
    fn clear_option_interactions(&mut self);
    fn option_properties(&self) -> Vec<DesignPanelProperty>;
    fn sync_option_states(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn render_option_cell(
        &self,
        id_suffix: SharedString,
        prefix: &'static str,
        value: SharedString,
        property: DesignPanelProperty,
        options: Vec<PropertyOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_preview_option_cell(
        &self,
        id_suffix: SharedString,
        prefix: &'static str,
        value: SharedString,
        property: DesignPanelProperty,
        options: Vec<PropertyOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_remove_button(
        &self,
        id_suffix: impl Into<SharedString>,
        collection: DesignPanelCollection,
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn menu_preview_effect_index(property: DesignPanelProperty) -> Option<usize>;
    fn property_supports_menu_preview(property: DesignPanelProperty) -> bool;
    fn menu_preview_for_property(
        &self,
        property: DesignPanelProperty,
        candidate: DesignPanelValue,
    ) -> Option<DesignMenuPreview>;
    fn begin_menu_preview(&mut self, preview: DesignMenuPreview, cx: &mut Context<Self>);
    fn cancel_menu_preview(&mut self, cx: &mut Context<Self>);
    fn menu_preview_matches_property_candidate(
        preview: &DesignMenuPreview,
        property: DesignPanelProperty,
        candidate: &DesignPanelValue,
    ) -> bool;
    fn set_property_menu_preview(
        &mut self,
        property: DesignPanelProperty,
        candidate: DesignPanelValue,
        preview: bool,
        cx: &mut Context<Self>,
    );
    fn cancel_menu_preview_for_property(
        &mut self,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    );
    fn begin_paint_blend_mode_preview(
        &mut self,
        target: &PickerEventTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        cx: &mut Context<Self>,
    );
    fn end_paint_blend_mode_preview(
        &mut self,
        target: &PickerEventTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        cx: &mut Context<Self>,
    );
    #[allow(dead_code)]
    fn render_action_button(
        &self,
        id_suffix: &'static str,
        label: &'static str,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_icon_action_button(
        &self,
        id_suffix: &'static str,
        icon: IconName,
        action: DesignPanelAction,
        cx: &mut Context<Self>,
    ) -> AnyElement;
}

impl DesignOptionsController for DesignPanel {
    fn clear_option_interactions(&mut self) {
        self.retained.options.states.clear();
        self.retained.options.subscriptions.clear();
        self.retained.options.snapshots.clear();
    }

    fn option_properties(&self) -> Vec<DesignPanelProperty> {
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
            (0..self.host.inspected_node().component_properties.len())
                .map(DesignPanelProperty::ComponentProperty),
        );
        if let Some(layout) = self.host.inspected_node().layout.as_ref() {
            properties
                .extend((0..layout.grid_columns.len()).map(DesignPanelProperty::GridColumnTrack));
            properties.extend((0..layout.grid_rows.len()).map(DesignPanelProperty::GridRowTrack));
        }
        if let Some(stroke) = self.host.inspected_node().stroke.as_ref() {
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
        for (index, modifier) in self
            .host
            .inspected_node()
            .transform_modifiers
            .iter()
            .enumerate()
        {
            properties.extend([
                DesignPanelProperty::TransformRepeatType(index),
                DesignPanelProperty::TransformRepeatUnit(index),
            ]);
            if matches!(modifier.mode, DesignRepeatMode::Linear(_)) {
                properties.push(DesignPanelProperty::TransformRepeatAxis(index));
            }
        }
        for (index, effect) in self.host.inspected_node().effects.iter().enumerate() {
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
        for index in 0..self.host.inspected_node().layout_grids.len() {
            properties.push(DesignPanelProperty::LayoutGridKind(index));
            properties.push(DesignPanelProperty::LayoutGridAlignment(index));
        }
        properties
            .extend((0..self.export_configurations().len()).map(DesignPanelProperty::ExportFormat));
        properties.retain(|property| self.property_options(*property).is_some());
        properties
    }

    fn sync_option_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let properties = self.option_properties();
        let desired = properties.iter().copied().collect::<HashSet<_>>();
        self.retained
            .options
            .states
            .retain(|property, _| desired.contains(property));
        self.retained
            .options
            .subscriptions
            .retain(|property, _| desired.contains(property));
        self.retained
            .options
            .snapshots
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
            if self.retained.options.snapshots.get(&property) == Some(&snapshot) {
                continue;
            }
            let selected_index = current
                .as_ref()
                .and_then(|current| options.iter().position(|option| &option.value == current))
                .map(|index| IndexPath::default().row(index));

            if let Some(state) = self.retained.options.states.get(&property).cloned() {
                state.update(cx, |state, cx| {
                    state.set_items(options, window, cx);
                    state.set_selected_index(selected_index, window, cx);
                });
                self.retained.options.snapshots.insert(property, snapshot);
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
                    let field = crate::molecules::InspectorPickerField::new(
                        inspector_fields::inspector_property_value(this, property),
                        inspector_fields::inspector_property_presentation(
                            this, property, false, false,
                        ),
                    );
                    if field.select(value.clone()).is_none() {
                        return;
                    }
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
            self.retained.options.states.insert(property, state);
            self.retained
                .options
                .subscriptions
                .insert(property, subscription);
            self.retained.options.snapshots.insert(property, snapshot);
        }
    }

    fn render_option_cell(
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
            .preferences
            .additional_labels
            .then(|| Self::compact_property_label(property));
        let additional_label_selector =
            SharedString::from(format!("{}-{id_suffix}-additional-label", self.id));
        let Some(state) = self.retained.options.states.get(&property) else {
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
            .when(self.preferences.additional_labels, |cell| {
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

    fn render_preview_option_cell(
        &self,
        id_suffix: SharedString,
        prefix: &'static str,
        value: SharedString,
        property: DesignPanelProperty,
        options: Vec<PropertyOption>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self.overlays.preview_option_menu_open() == Some(property);
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let current = self.resolved_property_value(property);
        let label = if self.preferences.additional_labels {
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
                            this.overlays
                                .open(DesignOverlayState::PreviewOptionMenu(property));
                        } else if this.overlays.preview_option_menu_open() == Some(property) {
                            this.cancel_menu_preview_for_property(property, cx);
                            this.overlays.discard(DesignOpenOverlay::PreviewOptionMenu);
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
            .on_open_change(move |is_open, window, cx| {
                panel_for_open.update(cx, |this, cx| {
                    if *is_open {
                        this.remember_overlay_focus_return(
                            DesignOpenOverlay::PreviewOptionMenu,
                            window,
                            cx,
                        );
                        this.cancel_menu_preview(cx);
                        this.overlays
                            .open(DesignOverlayState::PreviewOptionMenu(property));
                    } else if this.overlays.preview_option_menu_open() == Some(property) {
                        let _ = this.dismiss_overlay_from_outside_click(
                            DesignOpenOverlay::PreviewOptionMenu,
                            window,
                            cx,
                        );
                    }
                    if *is_open {
                        cx.notify();
                    }
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
                                        this.overlays.discard(DesignOpenOverlay::PreviewOptionMenu);
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
                                                this.overlays
                                                    .discard(DesignOpenOverlay::PreviewOptionMenu);
                                                this.emit_property(
                                                    property,
                                                    key_candidate.clone(),
                                                    cx,
                                                );
                                                window.prevent_default();
                                                cx.stop_propagation();
                                            }
                                            "escape"
                                                if this.dismiss_overlay_from_escape(
                                                    DesignOpenOverlay::PreviewOptionMenu,
                                                    window,
                                                    cx,
                                                ) =>
                                            {
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

    fn render_remove_button(
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
                || self.host.inspected_node().effect_style_binding.is_none())
            && (collection != DesignPanelCollection::LayoutGrid
                || self
                    .host
                    .inspected_node()
                    .layout_grid_style_binding
                    .is_none());
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

    fn menu_preview_effect_index(property: DesignPanelProperty) -> Option<usize> {
        match property {
            DesignPanelProperty::EffectKind(index)
            | DesignPanelProperty::EffectShadowBlendMode(index)
            | DesignPanelProperty::EffectNoiseBlendMode(index)
            | DesignPanelProperty::EffectBlurType(index)
            | DesignPanelProperty::EffectNoiseType(index) => Some(index),
            _ => None,
        }
    }

    fn property_supports_menu_preview(property: DesignPanelProperty) -> bool {
        matches!(
            property,
            DesignPanelProperty::BlendMode
                | DesignPanelProperty::HorizontalSizing
                | DesignPanelProperty::VerticalSizing
                | DesignPanelProperty::StrokeAlign
        ) || Self::menu_preview_effect_index(property).is_some()
    }

    fn menu_preview_for_property(
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
            if self.host.inspection_context.selection().kind() != DesignPanelSelectionKind::Single {
                return None;
            }
            let effect = self.host.inspected_node().effects.get(index)?;
            return Some(DesignMenuPreview::EffectProperty {
                node_id: self.host.inspected_node().id.clone(),
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

    fn begin_menu_preview(&mut self, preview: DesignMenuPreview, cx: &mut Context<Self>) {
        if self.overlays.active_menu_preview().as_ref() == Some(&preview) {
            return;
        }
        self.cancel_menu_preview(cx);
        self.overlays
            .open(DesignOverlayState::MenuPreview(Box::new(preview.clone())));
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::MenuPreviewRequested {
                preview,
                phase: DesignMenuPreviewPhase::Begin,
            },
        );
    }

    fn cancel_menu_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.overlays.take_menu_preview() else {
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

    fn menu_preview_matches_property_candidate(
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

    fn set_property_menu_preview(
        &mut self,
        property: DesignPanelProperty,
        candidate: DesignPanelValue,
        preview: bool,
        cx: &mut Context<Self>,
    ) {
        if !preview {
            if self
                .overlays
                .active_menu_preview()
                .as_ref()
                .is_some_and(|active| {
                    Self::menu_preview_matches_property_candidate(active, property, &candidate)
                })
            {
                self.cancel_menu_preview(cx);
            }
            return;
        }
        if let Some(preview) = self.menu_preview_for_property(property, candidate) {
            self.begin_menu_preview(preview, cx);
        }
    }

    fn cancel_menu_preview_for_property(
        &mut self,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) {
        if self
            .overlays
            .active_menu_preview()
            .as_ref()
            .is_some_and(|preview| {
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
            })
        {
            self.cancel_menu_preview(cx);
        }
    }

    fn begin_paint_blend_mode_preview(
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
        let picker_is_exact = self
            .overlays
            .active_picker()
            .as_ref()
            .is_some_and(|active| {
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
        if target.node_id != self.host.inspected_node().id
            || self.host.inspection_context.selection().kind() != DesignPanelSelectionKind::Single
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

    fn end_paint_blend_mode_preview(
        &mut self,
        target: &PickerEventTarget,
        original: DesignBlendMode,
        candidate: DesignBlendMode,
        cx: &mut Context<Self>,
    ) {
        let matches = self
            .overlays
            .active_menu_preview()
            .as_ref()
            .is_some_and(|preview| {
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

    #[allow(dead_code)]
    fn render_action_button(
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

    fn render_icon_action_button(
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
}

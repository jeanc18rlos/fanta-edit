use super::*;

impl DesignPanel {
    pub(super) fn effect_edit_target(
        &self,
        property: DesignPanelProperty,
    ) -> Option<(usize, SharedString, Option<SharedString>)> {
        let index = property.effect_index()?;
        let effect = self.node.effects.get(index)?;
        let shader_property_id =
            if let DesignPanelProperty::EffectShaderProperty(_, property_index) = property {
                let DesignEffectSettings::Shader(shader) = &effect.settings else {
                    return None;
                };
                Some(shader.properties.get(property_index)?.definition_id.clone())
            } else {
                None
            };
        Some((index, effect.id.clone(), shader_property_id))
    }

    pub(super) fn emit_effect_edit(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) -> bool {
        if property.effect_index().is_none() {
            return false;
        }
        if !self.collection_is_supported(DesignPanelCollection::Effect) {
            return true;
        }
        let Some((index, effect_id, shader_property_id)) = self.effect_edit_target(property) else {
            return true;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectEditRequested {
                node_id: self.node.id.clone(),
                effect_id,
                index,
                property,
                shader_property_id,
                value,
                phase,
            },
        );
        true
    }

    pub(super) fn effect_shader_property(
        &self,
        index: usize,
        property_index: usize,
    ) -> Option<(&DesignEffect, &super::super::DesignShaderProperty)> {
        let effect = self.node.effects.get(index)?;
        let DesignEffectSettings::Shader(shader) = &effect.settings else {
            return None;
        };
        Some((effect, shader.properties.get(property_index)?))
    }

    pub(super) fn request_effect_shader_property_editor(
        &self,
        index: usize,
        property_index: usize,
        target: DesignShaderPropertyEditorTarget,
        editor: DesignShaderPropertyEditorKind,
        cx: &mut Context<Self>,
    ) {
        let Some((effect, property)) = self.effect_shader_property(index, property_index) else {
            return;
        };
        let target_is_valid = match target {
            DesignShaderPropertyEditorTarget::Value => true,
            DesignShaderPropertyEditorTarget::ColorPointColor => {
                matches!(
                    &property.value,
                    DesignShaderPropertyValue::ColorPoint { .. }
                )
            }
            DesignShaderPropertyEditorTarget::GradientStopColor(stop_index) => {
                matches!(
                    &property.value,
                    DesignShaderPropertyValue::Gradient(stops) if stops.get(stop_index).is_some()
                )
            }
        };
        let editor_is_valid = match editor {
            DesignShaderPropertyEditorKind::Resource => {
                target == DesignShaderPropertyEditorTarget::Value
                    && matches!(
                        property.kind,
                        DesignShaderPropertyKind::Image
                            | DesignShaderPropertyKind::InstanceSwap
                            | DesignShaderPropertyKind::Slot
                    )
                    && matches!(&property.value, DesignShaderPropertyValue::AssetId(_))
            }
            DesignShaderPropertyEditorKind::Variable => {
                !matches!(&property.value, DesignShaderPropertyValue::Opaque { .. })
            }
        };
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.node.effect_style_binding.is_some()
            || property.read_only
            || !target_is_valid
            || !editor_is_valid
        {
            return;
        }
        let action = DesignPanelAction::EffectShaderPropertyEditorRequested {
            node_id: self.node.id.clone(),
            effect_id: effect.id.clone(),
            index,
            shader_property_id: property.definition_id.clone(),
            property_index,
            property_kind: property.kind,
            target,
            editor,
            current_value: property.value.clone(),
        };
        if self.node_capability_allows_action(&action) {
            cx.emit_design_panel_action(self, action);
        }
    }

    pub(super) fn request_effect_shader_property_variable_detach(
        &self,
        index: usize,
        property_index: usize,
        target: DesignShaderPropertyEditorTarget,
        expected_variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        let Some((effect, property)) = self.effect_shader_property(index, property_index) else {
            return;
        };
        let current_variable_id = shader_property_variable_id(&property.value, target);
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.node.effect_style_binding.is_some()
            || property.read_only
            || current_variable_id != Some(&expected_variable_id)
        {
            return;
        }
        let action = DesignPanelAction::EffectShaderPropertyVariableDetachRequested {
            node_id: self.node.id.clone(),
            effect_id: effect.id.clone(),
            index,
            shader_property_id: property.definition_id.clone(),
            property_index,
            target,
            variable_id: expected_variable_id,
        };
        if self.node_capability_allows_action(&action) {
            cx.emit_design_panel_action(self, action);
        }
    }

    pub(super) fn emit_effect_reorder(
        &self,
        effect_id: SharedString,
        fallback_from_index: usize,
        to_index: usize,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.node.effect_style_binding.is_some()
        {
            return;
        }
        let from_index = if effect_id.is_empty() {
            fallback_from_index
        } else {
            self.node
                .effect_index_by_id(effect_id.as_ref())
                .unwrap_or(fallback_from_index)
        };
        if from_index == to_index
            || from_index >= self.node.effects.len()
            || to_index >= self.node.effects.len()
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectReorderRequested {
                node_id: self.node.id.clone(),
                effect_id,
                from_index,
                to_index,
            },
        );
    }

    pub(super) fn activate_shader_property_field_from_control(
        &mut self,
        property: DesignPanelProperty,
        field: ShaderPropertyEditorField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.property_editor.as_ref().is_some_and(|editor| {
            editor.property == property
                && matches!(
                    editor.kind,
                    PropertyEditorKind::Shader {
                        field: active_field,
                        ..
                    } if active_field == field
                )
        }) {
            return;
        }
        let return_focus = window.focused(cx).map(|handle| EditorFocusReturn {
            origin: EditorFocusOrigin::ShaderField { property, field },
            handle,
        });
        self.activate_shader_property_field(property, field, window, cx);
        if self.property_editor.as_ref().is_some_and(|editor| {
            editor.property == property
                && matches!(
                    editor.kind,
                    PropertyEditorKind::Shader {
                        field: active_field,
                        ..
                    } if active_field == field
                )
        }) {
            self.editor_focus_return = return_focus;
        }
    }

    pub(super) fn activate_shader_property_field(
        &mut self,
        property: DesignPanelProperty,
        field: ShaderPropertyEditorField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.property_is_editable(property) || self.suppress_next_control_activation {
            self.suppress_next_control_activation = false;
            return;
        }
        if self.property_editor.is_some() {
            return;
        }
        let Some(current) = self
            .resolved_property_value(property)
            .or_else(|| self.current_property_value(property))
        else {
            return;
        };
        let DesignPanelValue::ShaderProperty(value) = &current else {
            return;
        };
        if shader_property_field_is_bound(value, field) {
            return;
        }
        let Some((input, base, draft)) = shader_property_field_draft(value, field) else {
            return;
        };

        if let Some((property_id, _)) = self.component_swap_hovered.take() {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ComponentSwapPreviewRequested {
                    node_id: self.node.id.clone(),
                    property_id,
                    selection: None,
                },
            );
        }
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.type_settings_open = false;
        self.vector_edit_target_ids = None;
        self.editor_focus_return = None;
        self.property_editor = Some(PropertyEditor {
            property,
            layout_grid_target: None,
            export_configuration_id: None,
            original: current.clone(),
            last_preview: None,
            base,
            kind: PropertyEditorKind::Shader { field, input },
        });
        self.property_editor_invalid = false;
        self.emit_property_edit(property, current, DesignPanelEditPhase::Begin, cx);
        self.suppress_property_input_change = true;
        self.property_input.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.suppress_property_input_change = false;
        cx.notify();
    }

    pub(super) fn open_effect_style_browser(&mut self, cx: &mut Context<Self>) {
        if !self.collection_is_supported(DesignPanelCollection::Effect) {
            return;
        }
        self.style_browser_source_filter =
            self.style_browser_source_filter.normalized_for_libraries(
                self.effect_style_view_data
                    .libraries
                    .iter()
                    .map(|library| (&library.id, &library.name)),
            );
        self.effect_style_browser_open = true;
        self.paint_style_browser_open = None;
        self.active_effect_settings = None;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        cx.notify();
    }

    pub(super) fn emit_effect_style_apply(
        &mut self,
        style: DesignEffectStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.effect_style_view_data.style(&style).is_none()
        {
            return;
        }
        self.effect_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectStyleApplyRequested {
                node_id: self.node.id.clone(),
                style,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_effect_style_create(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.node.effects.is_empty()
            || self.node.effect_style_binding.is_some()
        {
            return;
        }
        self.effect_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectStyleCreateRequested {
                node_id: self.node.id.clone(),
                effects: self.node.effects.clone(),
            },
        );
        cx.notify();
    }

    pub(super) fn emit_effect_style_detach(&mut self, cx: &mut Context<Self>) {
        if !self.can_edit() || !self.collection_is_supported(DesignPanelCollection::Effect) {
            return;
        }
        let Some(binding) = self
            .node
            .effect_style_binding
            .as_ref()
            .filter(|binding| binding.can_detach)
        else {
            return;
        };
        self.effect_style_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectStyleDetachRequested {
                node_id: self.node.id.clone(),
                style: binding.selection.clone(),
            },
        );
        cx.notify();
    }

    pub(super) fn render_effect_style_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let active = self.effect_style_browser_open;
        let binding = self.node.effect_style_binding.clone();
        let can_edit = self.can_edit();
        let can_create = can_edit && !self.node.effects.is_empty() && binding.is_none();
        let query = self.normalized_style_browser_query(cx);
        let source_filter = self.style_browser_source_filter.clone();
        let view_mode = self.style_browser_view_mode;
        let catalog_is_empty = self.effect_style_view_data.page_styles.is_empty()
            && self
                .effect_style_view_data
                .libraries
                .iter()
                .all(|library| library.styles.is_empty());
        let page_styles = if source_filter.includes_page() {
            self.effect_style_view_data
                .page_styles
                .iter()
                .map(|style| {
                    let summary = style
                        .effect_kinds
                        .iter()
                        .copied()
                        .map(DesignEffectKind::label)
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        style.name.clone(),
                        DesignEffectStyleSelection::page(style.id.clone()),
                        summary,
                    )
                })
                .filter(|(name, _, summary)| {
                    Self::style_browser_row_matches(&query, name.as_ref(), summary, None)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let libraries = self
            .effect_style_view_data
            .libraries
            .iter()
            .filter(|library| source_filter.includes_library(library.id.as_ref()))
            .map(|library| {
                (
                    library.name.clone(),
                    library
                        .styles
                        .iter()
                        .map(|style| {
                            let summary = style
                                .effect_kinds
                                .iter()
                                .copied()
                                .map(DesignEffectKind::label)
                                .collect::<Vec<_>>()
                                .join(", ");
                            (
                                style.name.clone(),
                                DesignEffectStyleSelection::library(
                                    library.id.clone(),
                                    style.id.clone(),
                                ),
                                summary,
                            )
                        })
                        .filter(|(name, _, summary)| {
                            Self::style_browser_row_matches(
                                &query,
                                name.as_ref(),
                                summary,
                                Some(library.name.as_ref()),
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .filter(|(_, styles)| !styles.is_empty())
            .collect::<Vec<_>>();
        let style_browser_search = self.style_browser_search.clone();
        let style_browser_library_sources = self
            .effect_style_view_data
            .libraries
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect::<Vec<_>>();
        let style_browser_scope = SharedString::from(format!("{panel_id}-effect-style-browser"));
        let tooltip = binding
            .as_ref()
            .map_or_else(|| "Effect styles".into(), |binding| binding.name.clone());
        let trigger = Button::new(SharedString::from(format!("{}-effect-styles", self.id)))
            .tooltip(tooltip)
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(24.))
            .selected(active || binding.is_some())
            .child(self.render_color_styles_icon(cx))
            .on_activate(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.open_effect_style_browser(cx);
            }));

        Popover::new(SharedString::from(format!(
            "{}-effect-style-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                if *open {
                    this.open_effect_style_browser(cx);
                } else if this.effect_style_browser_open {
                    this.effect_style_browser_open = false;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let mut content = v_flex()
                .w(popup_width(window, 260.))
                .max_h(popup_height(window, 520.))
                .overflow_y_scrollbar()
                .gap_1()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .font_semibold()
                        .child("Effect styles"),
                );
            if let Some(binding) = binding.clone() {
                let panel = panel_for_content.clone();
                content = content.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .gap_2()
                        .child(div().flex_1().truncate().text_xs().child(binding.name))
                        .child(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-detach-effect-style"
                            )))
                            .label("Detach")
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!can_edit || !binding.can_detach)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.emit_effect_style_detach(cx);
                                });
                            }),
                        ),
                );
            }
            let panel = panel_for_content.clone();
            content = content.child(
                Button::new(SharedString::from(format!(
                    "{panel_id}-create-effect-style"
                )))
                .label("Create style from selection")
                .xsmall()
                .compact()
                .ghost()
                .w_full()
                .disabled(!can_create)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| this.emit_effect_style_create(cx));
                }),
            );
            content = content.child(div().px_2().child(Self::render_style_browser_toolbar(
                panel_for_content.clone(),
                style_browser_scope.clone(),
                style_browser_search.clone(),
                source_filter.clone(),
                style_browser_library_sources.clone(),
                view_mode,
            )));
            if page_styles.is_empty() && libraries.iter().all(|(_, styles)| styles.is_empty()) {
                content = content.child(
                    div()
                        .px_2()
                        .py_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if catalog_is_empty {
                            "No Effect styles supplied by the host"
                        } else {
                            "No styles match this search and source filter"
                        }),
                );
            }
            if !page_styles.is_empty() {
                content = content.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("This page"),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().px_2().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().px_2().gap_0p5()
                };
                for (row_index, (name, selection, summary)) in
                    page_styles.clone().into_iter().enumerate()
                {
                    let panel = panel_for_content.clone();
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-effect-style-page-{row_index}"
                        )))
                        .label(name)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(118.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.emit_effect_style_apply(selection.clone(), cx);
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            for (library_index, (library_name, styles)) in libraries.clone().into_iter().enumerate()
            {
                content = content.child(
                    div()
                        .px_2()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(library_name),
                );
                let mut rows = if view_mode == StyleBrowserViewMode::Grid {
                    h_flex().w_full().px_2().gap_1().flex_wrap()
                } else {
                    v_flex().w_full().px_2().gap_0p5()
                };
                for (style_index, (name, selection, summary)) in styles.into_iter().enumerate() {
                    let panel = panel_for_content.clone();
                    rows = rows.child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-effect-style-library-{library_index}-{style_index}"
                        )))
                        .label(name)
                        .tooltip(summary)
                        .xsmall()
                        .compact()
                        .ghost()
                        .when(view_mode == StyleBrowserViewMode::Grid, |button| {
                            button.w(px(118.)).h(px(44.))
                        })
                        .when(view_mode == StyleBrowserViewMode::List, |button| {
                            button.w_full()
                        })
                        .disabled(!can_edit)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.emit_effect_style_apply(selection.clone(), cx);
                            });
                        }),
                    );
                }
                content = content.child(rows);
            }
            content
        })
        .into_any_element()
    }

    pub(super) fn render_effect_color_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        color: DesignColor,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(index) = property.effect_index() else {
            return div().into_any_element();
        };
        let Some(effect) = self.node.effects.get(index) else {
            return div().into_any_element();
        };
        self.render_auxiliary_color_picker_control(
            cx.entity(),
            id_suffix,
            format!("{label} · #{}", color.hex()),
            color,
            AuxiliaryColorPickerTarget::Effect {
                node_id: self.node.id.clone(),
                effect_id: effect.id.clone(),
                index,
                property,
            },
            cx,
        )
    }

    pub(super) fn emit_effect_variable_action(
        &self,
        index: usize,
        field: DesignEffectVariableField,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit()
            || !self.collection_is_supported(DesignPanelCollection::Effect)
            || self.node.effect_style_binding.is_some()
        {
            return;
        }
        let Some(effect) = self.node.effects.get(index) else {
            return;
        };
        if let Some(binding) = effect.variable_binding(field) {
            if binding.can_detach {
                cx.emit_design_panel_action(
                    self,
                    DesignPanelAction::EffectVariableDetachRequested {
                        node_id: self.node.id.clone(),
                        effect_id: effect.id.clone(),
                        index,
                        field,
                        variable_id: binding.variable_id.clone(),
                    },
                );
            }
            return;
        }
        let Some(variable) = self.effect_variable_view_data.compatible(field).next() else {
            return;
        };
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::EffectVariableApplyRequested {
                node_id: self.node.id.clone(),
                effect_id: effect.id.clone(),
                index,
                field,
                variable_id: variable.id.clone(),
            },
        );
    }

    pub(super) fn render_effect_variables(
        &self,
        index: usize,
        effect: &DesignEffect,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fields = effect.variable_fields(self.node.effect_capabilities.shadow_spread);
        (!fields.is_empty()).then(|| {
            let mut content = v_flex()
                .w_full()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Variables"),
                );
            for field in fields {
                let binding = effect.variable_binding(*field);
                let candidate = self.effect_variable_view_data.compatible(*field).next();
                let enabled = self.can_edit()
                    && self.node.effect_style_binding.is_none()
                    && (binding.is_some_and(|binding| binding.can_detach)
                        || binding.is_none() && candidate.is_some());
                let value: SharedString = binding.map_or_else(
                    || {
                        candidate.map_or_else(
                            || "No compatible variable".into(),
                            |variable| format!("Apply {}", variable.name).into(),
                        )
                    },
                    |binding| {
                        format!("{} · {}", binding.collection_name, binding.variable_name).into()
                    },
                );
                let field = *field;
                let panel = cx.entity();
                content = content.child(
                    Button::new(SharedString::from(format!(
                        "{}-effect-variable-{index}-{:?}",
                        self.id, field
                    )))
                    .label(format!("{} · {value}", field.label()))
                    .tooltip(if binding.is_some() {
                        "Detach variable"
                    } else {
                        "Apply variable"
                    })
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(!enabled)
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_effect_variable_action(index, field, cx);
                        });
                    }),
                );
            }
            content.into_any_element()
        })
    }

    pub(super) fn render_shader_property_field_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        field: ShaderPropertyEditorField,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let editable = self.property_is_editable(property)
            && self
                .current_property_value(property)
                .and_then(|value| match value {
                    DesignPanelValue::ShaderProperty(value) => Some(value),
                    _ => None,
                })
                .is_some_and(|value| {
                    !shader_property_field_is_bound(&value, field)
                        && shader_property_field_draft(&value, field).is_some()
                });
        let editing = editable
            && self.property_editor.as_ref().is_some_and(|editor| {
                editor.property == property
                    && matches!(
                        editor.kind,
                        PropertyEditorKind::Shader {
                            field: active,
                            ..
                        } if active == field
                    )
            });
        let retained_focus = self
            .editor_focus_return
            .as_ref()
            .filter(|return_focus| {
                matches!(
                    return_focus.origin,
                    EditorFocusOrigin::ShaderField {
                        property: origin_property,
                        field: origin_field,
                    } if origin_property == property && origin_field == field
                )
            })
            .map(|return_focus| return_focus.handle.clone());
        let mut cell = h_flex()
            .id(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .h(px(ROW_HEIGHT))
            .flex_1()
            .min_w(px(0.))
            .pl_2()
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
            .when(editable, |cell| {
                cell.key_context(CONTROL_KEY_CONTEXT)
                    .cursor_pointer()
                    .hover(|style| style.border_color(cx.theme().muted_foreground))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!editable, |cell| {
                cell.text_color(cx.theme().muted_foreground).opacity(0.78)
            });
        if editable {
            cell = if let Some(focus) = retained_focus {
                cell.track_focus(&focus.tab_index(0).tab_stop(true))
            } else {
                cell.tab_index(0)
            };
        }
        if editable {
            cell = cell.on_activate(cx.listener(move |this, _, window, cx| {
                this.activate_shader_property_field_from_control(property, field, window, cx);
            }));
        }
        cell = cell.when(!prefix.is_empty(), |cell| {
            cell.child(
                div()
                    .w(px(12.))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(prefix),
            )
        });
        if editing {
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
            cell = cell.child(div().truncate().text_xs().child(value.into()));
        }
        cell.into_any_element()
    }

    pub(super) fn render_shader_property_toggle(
        &self,
        id_suffix: impl Into<SharedString>,
        checked: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(property);
        let next = DesignPanelValue::ShaderProperty(DesignShaderPropertyValue::Boolean(!checked));
        let mut row = h_flex()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
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
                this.emit_property(property, next.clone(), cx);
            }));
        }
        row.child(div().flex_1().text_xs().child("Enabled"))
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

    pub(super) fn render_shader_property_variable_controls(
        &self,
        index: usize,
        property_index: usize,
        target: DesignShaderPropertyEditorTarget,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((_, property)) = self.effect_shader_property(index, property_index) else {
            return div().into_any_element();
        };
        let binding = shader_property_variable_id(&property.value, target).cloned();
        let enabled = self.can_edit()
            && self.node.effect_style_binding.is_none()
            && !property.read_only
            && !matches!(&property.value, DesignShaderPropertyValue::Opaque { .. });
        let panel = cx.entity();
        let panel_for_change = panel.clone();
        let label: SharedString = binding.as_ref().map_or_else(
            || "No variable".into(),
            |id| format!("Variable · {id}").into(),
        );
        let change_label = if binding.is_some() { "Change" } else { "Bind" };
        let mut row = h_flex()
            .w_full()
            .h(px(ROW_HEIGHT))
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "{}-effect-shader-variable-{index}-{property_index}-{target:?}",
                    self.id
                )))
                .label(change_label)
                .tooltip(if binding.is_some() {
                    "Change variable"
                } else {
                    "Bind variable"
                })
                .xsmall()
                .compact()
                .ghost()
                .disabled(!enabled)
                .on_activate(move |_, _, cx| {
                    panel_for_change.update(cx, |this, cx| {
                        this.request_effect_shader_property_editor(
                            index,
                            property_index,
                            target,
                            DesignShaderPropertyEditorKind::Variable,
                            cx,
                        );
                    });
                }),
            );
        if let Some(variable_id) = binding {
            row = row.child(
                Button::new(SharedString::from(format!(
                    "{}-effect-shader-variable-detach-{index}-{property_index}-{target:?}",
                    self.id
                )))
                .label("Detach")
                .tooltip("Detach variable")
                .xsmall()
                .compact()
                .ghost()
                .disabled(!enabled)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.request_effect_shader_property_variable_detach(
                            index,
                            property_index,
                            target,
                            variable_id.clone(),
                            cx,
                        );
                    });
                }),
            );
        }
        row.into_any_element()
    }

    pub(super) fn render_effect_shader_property(
        &self,
        index: usize,
        property_index: usize,
        property: &super::super::DesignShaderProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel_property = DesignPanelProperty::EffectShaderProperty(index, property_index);
        let field_id =
            |suffix: &str| format!("effect-shader-property-{index}-{property_index}-{suffix}");
        let mut body = v_flex().w_full().gap_1();
        match &property.value {
            DesignShaderPropertyValue::Boolean(value) => {
                body = body.child(self.render_shader_property_toggle(
                    field_id("boolean"),
                    *value,
                    panel_property,
                    cx,
                ));
            }
            DesignShaderPropertyValue::Text(value) => {
                body = body.child(self.render_shader_property_field_cell(
                    field_id("text"),
                    "T",
                    value.clone(),
                    panel_property,
                    ShaderPropertyEditorField::Text,
                    cx,
                ));
            }
            DesignShaderPropertyValue::Number(value) => {
                body = body.child(self.render_shader_property_field_cell(
                    field_id("number"),
                    "#",
                    format_number(*value),
                    panel_property,
                    ShaderPropertyEditorField::Number,
                    cx,
                ));
            }
            DesignShaderPropertyValue::AssetId(asset_id) => {
                let panel = cx.entity();
                let kind = property.kind;
                let enabled = self.property_is_editable(panel_property)
                    && matches!(
                        kind,
                        DesignShaderPropertyKind::Image
                            | DesignShaderPropertyKind::InstanceSwap
                            | DesignShaderPropertyKind::Slot
                    );
                body = body.child(
                    h_flex()
                        .w_full()
                        .h(px(ROW_HEIGHT))
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .text_xs()
                                .child(asset_id.clone()),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "{}-{}",
                                self.id,
                                field_id("resource")
                            )))
                            .label("Choose")
                            .tooltip(format!("Choose {}", kind.label()))
                            .xsmall()
                            .compact()
                            .ghost()
                            .disabled(!enabled)
                            .on_activate(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.request_effect_shader_property_editor(
                                        index,
                                        property_index,
                                        DesignShaderPropertyEditorTarget::Value,
                                        DesignShaderPropertyEditorKind::Resource,
                                        cx,
                                    );
                                });
                            }),
                        ),
                );
            }
            DesignShaderPropertyValue::Color(color) => {
                body = body.child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(
                            div()
                                .size(px(20.))
                                .flex_none()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(color_hsla(*color)),
                        )
                        .child(self.render_shader_property_field_cell(
                            field_id("color"),
                            "#",
                            color.hex(),
                            panel_property,
                            ShaderPropertyEditorField::Color,
                            cx,
                        )),
                );
            }
            DesignShaderPropertyValue::Point(point) => {
                body = body.child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .child(self.render_shader_property_field_cell(
                            field_id("point-x"),
                            "X",
                            format_number(point.x),
                            panel_property,
                            ShaderPropertyEditorField::PointX,
                            cx,
                        ))
                        .child(self.render_shader_property_field_cell(
                            field_id("point-y"),
                            "Y",
                            format_number(point.y),
                            panel_property,
                            ShaderPropertyEditorField::PointY,
                            cx,
                        )),
                );
            }
            DesignShaderPropertyValue::Line { start, end } => {
                body = body
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Start"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("line-start-x"),
                                "X",
                                format_number(start.x),
                                panel_property,
                                ShaderPropertyEditorField::LineStartX,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("line-start-y"),
                                "Y",
                                format_number(start.y),
                                panel_property,
                                ShaderPropertyEditorField::LineStartY,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("End"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("line-end-x"),
                                "X",
                                format_number(end.x),
                                panel_property,
                                ShaderPropertyEditorField::LineEndX,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("line-end-y"),
                                "Y",
                                format_number(end.y),
                                panel_property,
                                ShaderPropertyEditorField::LineEndY,
                                cx,
                            )),
                    );
            }
            DesignShaderPropertyValue::Circle { center, radius } => {
                body = body
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-center-x"),
                                "X",
                                format_number(center.x),
                                panel_property,
                                ShaderPropertyEditorField::CircleCenterX,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-center-y"),
                                "Y",
                                format_number(center.y),
                                panel_property,
                                ShaderPropertyEditorField::CircleCenterY,
                                cx,
                            )),
                    )
                    .child(self.render_shader_property_field_cell(
                        field_id("circle-radius"),
                        "R",
                        format_number(*radius),
                        panel_property,
                        ShaderPropertyEditorField::CircleRadius,
                        cx,
                    ));
            }
            DesignShaderPropertyValue::CirclePoint {
                center,
                radius,
                angle,
            } => {
                body = body
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-point-center-x"),
                                "X",
                                format_number(center.x),
                                panel_property,
                                ShaderPropertyEditorField::CirclePointCenterX,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-point-center-y"),
                                "Y",
                                format_number(center.y),
                                panel_property,
                                ShaderPropertyEditorField::CirclePointCenterY,
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-point-radius"),
                                "R",
                                format_number(*radius),
                                panel_property,
                                ShaderPropertyEditorField::CirclePointRadius,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("circle-point-angle"),
                                "°",
                                format_number(*angle),
                                panel_property,
                                ShaderPropertyEditorField::CirclePointAngle,
                                cx,
                            )),
                    );
            }
            DesignShaderPropertyValue::ColorPoint {
                point,
                color,
                variable_id: _,
            } => {
                body = body
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(self.render_shader_property_field_cell(
                                field_id("color-point-x"),
                                "X",
                                format_number(point.x),
                                panel_property,
                                ShaderPropertyEditorField::ColorPointX,
                                cx,
                            ))
                            .child(self.render_shader_property_field_cell(
                                field_id("color-point-y"),
                                "Y",
                                format_number(point.y),
                                panel_property,
                                ShaderPropertyEditorField::ColorPointY,
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                div()
                                    .size(px(20.))
                                    .flex_none()
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(color_hsla(*color)),
                            )
                            .child(self.render_shader_property_field_cell(
                                field_id("color-point-color"),
                                "#",
                                color.hex(),
                                panel_property,
                                ShaderPropertyEditorField::ColorPointColor,
                                cx,
                            )),
                    )
                    .child(self.render_shader_property_variable_controls(
                        index,
                        property_index,
                        DesignShaderPropertyEditorTarget::ColorPointColor,
                        cx,
                    ));
            }
            DesignShaderPropertyValue::Gradient(stops) => {
                for (stop_index, stop) in stops.iter().enumerate() {
                    let panel = cx.entity();
                    let mut next = stops.clone();
                    next.remove(stop_index);
                    let remove_value =
                        DesignPanelValue::ShaderProperty(DesignShaderPropertyValue::Gradient(next));
                    let can_remove = self.property_is_editable(panel_property) && stops.len() > 2;
                    body = body
                        .child(
                            h_flex()
                                .w_full()
                                .gap_1()
                                .child(
                                    div()
                                        .size(px(20.))
                                        .flex_none()
                                        .rounded(px(4.))
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(color_hsla(stop.color)),
                                )
                                .child(self.render_shader_property_field_cell(
                                    field_id(&format!("gradient-{stop_index}-position")),
                                    "%",
                                    format!("{}%", format_number(stop.position * 100.)),
                                    panel_property,
                                    ShaderPropertyEditorField::GradientStopPosition(stop_index),
                                    cx,
                                ))
                                .child(self.render_shader_property_field_cell(
                                    field_id(&format!("gradient-{stop_index}-color")),
                                    "#",
                                    stop.color.hex(),
                                    panel_property,
                                    ShaderPropertyEditorField::GradientStopColor(stop_index),
                                    cx,
                                ))
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "{}-{}",
                                        self.id,
                                        field_id(&format!("gradient-{stop_index}-remove"))
                                    )))
                                    .icon(IconName::Minus)
                                    .tooltip(if can_remove {
                                        "Remove gradient stop"
                                    } else {
                                        "A gradient needs at least two stops"
                                    })
                                    .xsmall()
                                    .compact()
                                    .ghost()
                                    .disabled(!can_remove)
                                    .on_activate(
                                        move |_, _, cx| {
                                            panel.update(cx, |this, cx| {
                                                this.emit_property(
                                                    panel_property,
                                                    remove_value.clone(),
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                ),
                        )
                        .child(self.render_shader_property_variable_controls(
                            index,
                            property_index,
                            DesignShaderPropertyEditorTarget::GradientStopColor(stop_index),
                            cx,
                        ));
                }
                let panel = cx.entity();
                let add_value = DesignPanelValue::ShaderProperty(
                    DesignShaderPropertyValue::Gradient(shader_gradient_with_added_stop(stops)),
                );
                let can_add = self.property_is_editable(panel_property);
                body = body.child(
                    Button::new(SharedString::from(format!(
                        "{}-{}",
                        self.id,
                        field_id("gradient-add")
                    )))
                    .label("Add stop")
                    .icon(IconName::Plus)
                    .tooltip("Add gradient stop")
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(!can_add)
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_property(panel_property, add_value.clone(), cx);
                        });
                    }),
                );
            }
            DesignShaderPropertyValue::VariableAlias { variable_id } => {
                body = body
                    .child(
                        div()
                            .w_full()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(cx.theme().secondary)
                            .text_xs()
                            .child(format!("Variable alias · {variable_id}")),
                    )
                    .child(self.render_shader_property_variable_controls(
                        index,
                        property_index,
                        DesignShaderPropertyEditorTarget::Value,
                        cx,
                    ));
            }
            DesignShaderPropertyValue::Opaque { type_name, payload } => {
                body = body
                    .child(
                        div()
                            .w_full()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .bg(cx.theme().secondary)
                            .text_xs()
                            .child(format!("Opaque value · {type_name}")),
                    )
                    .child(
                        div()
                            .w_full()
                            .max_h(px(72.))
                            .overflow_y_scrollbar()
                            .px_2()
                            .py_1()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(payload.clone()),
                    );
            }
        }
        if !matches!(
            &property.value,
            DesignShaderPropertyValue::VariableAlias { .. }
                | DesignShaderPropertyValue::Opaque { .. }
        ) {
            body = body.child(self.render_shader_property_variable_controls(
                index,
                property_index,
                DesignShaderPropertyEditorTarget::Value,
                cx,
            ));
        }
        v_flex()
            .w_full()
            .gap_1()
            .p_2()
            .rounded(px(6.))
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(property.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(property.kind.label()),
                    ),
            )
            .when_some(property.description.clone(), |card, description| {
                card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                )
            })
            .child(body)
            .into_any_element()
    }

    pub(super) fn render_effect_settings(
        &self,
        index: usize,
        effect: &DesignEffect,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &effect.settings;
        let content = match settings {
            DesignEffectSettings::DropShadow(settings) => v_flex()
                .w_full()
                .gap_2()
                .child(self.render_effect_color_cell(
                    format!("effect-shadow-color-{index}"),
                    "Color",
                    settings.color,
                    DesignPanelProperty::EffectShadowColor(index),
                    cx,
                ))
                .child(self.render_value_cell(
                    format!("effect-shadow-blend-mode-{index}"),
                    "M",
                    settings.blend_mode.label(),
                    DesignPanelProperty::EffectShadowBlendMode(index),
                    DesignPanelValue::BlendMode(
                        if settings.blend_mode == DesignBlendMode::Normal {
                            DesignBlendMode::Multiply
                        } else {
                            DesignBlendMode::Normal
                        },
                    ),
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-shadow-offset-x-{index}"),
                            "X",
                            format!("Offset X · {}", format_number(settings.offset.x)),
                            DesignPanelProperty::EffectShadowOffsetX(index),
                            DesignPanelValue::Number(settings.offset.x + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-shadow-offset-y-{index}"),
                            "Y",
                            format!("Offset Y · {}", format_number(settings.offset.y)),
                            DesignPanelProperty::EffectShadowOffsetY(index),
                            DesignPanelValue::Number(settings.offset.y + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-shadow-blur-{index}"),
                            "B",
                            format!("Blur · {}", format_number(settings.radius)),
                            DesignPanelProperty::EffectShadowBlur(index),
                            DesignPanelValue::Number(settings.radius + 1.),
                            cx,
                        ))
                        .when(self.node.effect_capabilities.shadow_spread, |row| {
                            row.child(self.render_value_cell(
                                format!("effect-shadow-spread-{index}"),
                                "S",
                                format!("Spread · {}", format_number(settings.spread)),
                                DesignPanelProperty::EffectShadowSpread(index),
                                DesignPanelValue::Number(settings.spread + 1.),
                                cx,
                            ))
                        }),
                )
                .when(
                    self.node
                        .effect_capabilities
                        .show_shadow_behind_transparent_areas,
                    |content| {
                        content.child(self.render_toggle_row(
                            format!("effect-show-behind-node-{index}"),
                            "Show behind transparent areas",
                            settings.show_behind_node,
                            DesignPanelProperty::EffectDropShadowShowBehindNode(index),
                            cx,
                        ))
                    },
                )
                .into_any_element(),
            DesignEffectSettings::InnerShadow(settings) => v_flex()
                .w_full()
                .gap_2()
                .child(self.render_effect_color_cell(
                    format!("effect-shadow-color-{index}"),
                    "Color",
                    settings.color,
                    DesignPanelProperty::EffectShadowColor(index),
                    cx,
                ))
                .child(self.render_value_cell(
                    format!("effect-shadow-blend-mode-{index}"),
                    "M",
                    settings.blend_mode.label(),
                    DesignPanelProperty::EffectShadowBlendMode(index),
                    DesignPanelValue::BlendMode(
                        if settings.blend_mode == DesignBlendMode::Normal {
                            DesignBlendMode::Multiply
                        } else {
                            DesignBlendMode::Normal
                        },
                    ),
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-shadow-offset-x-{index}"),
                            "X",
                            format!("Offset X · {}", format_number(settings.offset.x)),
                            DesignPanelProperty::EffectShadowOffsetX(index),
                            DesignPanelValue::Number(settings.offset.x + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-shadow-offset-y-{index}"),
                            "Y",
                            format!("Offset Y · {}", format_number(settings.offset.y)),
                            DesignPanelProperty::EffectShadowOffsetY(index),
                            DesignPanelValue::Number(settings.offset.y + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-shadow-blur-{index}"),
                            "B",
                            format!("Blur · {}", format_number(settings.radius)),
                            DesignPanelProperty::EffectShadowBlur(index),
                            DesignPanelValue::Number(settings.radius + 1.),
                            cx,
                        ))
                        .when(self.node.effect_capabilities.shadow_spread, |row| {
                            row.child(self.render_value_cell(
                                format!("effect-shadow-spread-{index}"),
                                "S",
                                format!("Spread · {}", format_number(settings.spread)),
                                DesignPanelProperty::EffectShadowSpread(index),
                                DesignPanelValue::Number(settings.spread + 1.),
                                cx,
                            ))
                        }),
                )
                .into_any_element(),
            DesignEffectSettings::LayerBlur(settings)
            | DesignEffectSettings::BackgroundBlur(settings) => {
                let blur_type = settings.blur_type();
                let mut content = v_flex().w_full().gap_2().child(self.render_value_cell(
                    format!("effect-blur-type-{index}"),
                    "T",
                    blur_type.label(),
                    DesignPanelProperty::EffectBlurType(index),
                    DesignPanelValue::EffectBlurType(match blur_type {
                        DesignBlurType::Normal => DesignBlurType::Progressive,
                        DesignBlurType::Progressive => DesignBlurType::Normal,
                    }),
                    cx,
                ));
                match settings {
                    DesignBlurEffect::Normal { radius } => {
                        content = content.child(self.render_value_cell(
                            format!("effect-blur-radius-{index}"),
                            "B",
                            format!("Blur · {}", format_number(*radius)),
                            DesignPanelProperty::EffectBlurRadius(index),
                            DesignPanelValue::Number(*radius + 1.),
                            cx,
                        ));
                    }
                    DesignBlurEffect::Progressive {
                        start_radius,
                        end_radius,
                        start_offset,
                        end_offset,
                    } => {
                        content = content
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-start-radius-{index}"),
                                        "S",
                                        format!("Start blur · {}", format_number(*start_radius)),
                                        DesignPanelProperty::EffectProgressiveBlurStartRadius(
                                            index,
                                        ),
                                        DesignPanelValue::Number(*start_radius + 1.),
                                        cx,
                                    ))
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-end-radius-{index}"),
                                        "E",
                                        format!("End blur · {}", format_number(*end_radius)),
                                        DesignPanelProperty::EffectProgressiveBlurEndRadius(index),
                                        DesignPanelValue::Number(*end_radius + 1.),
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-start-x-{index}"),
                                        "X",
                                        format!("Start X · {}", format_number(start_offset.x)),
                                        DesignPanelProperty::EffectProgressiveBlurStartOffsetX(
                                            index,
                                        ),
                                        DesignPanelValue::Number((start_offset.x + 0.05).min(1.)),
                                        cx,
                                    ))
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-start-y-{index}"),
                                        "Y",
                                        format!("Start Y · {}", format_number(start_offset.y)),
                                        DesignPanelProperty::EffectProgressiveBlurStartOffsetY(
                                            index,
                                        ),
                                        DesignPanelValue::Number((start_offset.y + 0.05).min(1.)),
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-end-x-{index}"),
                                        "X",
                                        format!("End X · {}", format_number(end_offset.x)),
                                        DesignPanelProperty::EffectProgressiveBlurEndOffsetX(index),
                                        DesignPanelValue::Number((end_offset.x + 0.05).min(1.)),
                                        cx,
                                    ))
                                    .child(self.render_value_cell(
                                        format!("effect-progressive-end-y-{index}"),
                                        "Y",
                                        format!("End Y · {}", format_number(end_offset.y)),
                                        DesignPanelProperty::EffectProgressiveBlurEndOffsetY(index),
                                        DesignPanelValue::Number((end_offset.y + 0.05).min(1.)),
                                        cx,
                                    )),
                            );
                    }
                }
                content.into_any_element()
            }
            DesignEffectSettings::Noise(settings) => {
                let noise_type = settings.colors.noise_type();
                let mut content = v_flex().w_full().gap_2().child(self.render_value_cell(
                    format!("effect-noise-type-{index}"),
                    "T",
                    noise_type.label(),
                    DesignPanelProperty::EffectNoiseType(index),
                    DesignPanelValue::EffectNoiseType(match noise_type {
                        DesignNoiseType::Monotone => DesignNoiseType::Duotone,
                        DesignNoiseType::Duotone => DesignNoiseType::Multitone,
                        DesignNoiseType::Multitone => DesignNoiseType::Monotone,
                    }),
                    cx,
                ));
                match &settings.colors {
                    DesignNoiseColors::Monotone { color } => {
                        content = content.child(self.render_effect_color_cell(
                            format!("effect-noise-primary-color-{index}"),
                            "Color",
                            *color,
                            DesignPanelProperty::EffectNoisePrimaryColor(index),
                            cx,
                        ));
                    }
                    DesignNoiseColors::Duotone {
                        color,
                        secondary_color,
                    } => {
                        content = content
                            .child(self.render_effect_color_cell(
                                format!("effect-noise-primary-color-{index}"),
                                "Color 1",
                                *color,
                                DesignPanelProperty::EffectNoisePrimaryColor(index),
                                cx,
                            ))
                            .child(self.render_effect_color_cell(
                                format!("effect-noise-secondary-color-{index}"),
                                "Color 2",
                                *secondary_color,
                                DesignPanelProperty::EffectNoiseSecondaryColor(index),
                                cx,
                            ));
                    }
                    DesignNoiseColors::Multitone { opacity } => {
                        content = content.child(self.render_value_cell(
                            format!("effect-noise-opacity-{index}"),
                            "%",
                            format!("Opacity · {}", format_number(*opacity)),
                            DesignPanelProperty::EffectNoiseOpacity(index),
                            DesignPanelValue::Number((*opacity + 0.05).min(1.)),
                            cx,
                        ));
                    }
                }
                content
                    .child(self.render_value_cell(
                        format!("effect-noise-blend-mode-{index}"),
                        "M",
                        settings.blend_mode.label(),
                        DesignPanelProperty::EffectNoiseBlendMode(index),
                        DesignPanelValue::BlendMode(
                            if settings.blend_mode == DesignBlendMode::Normal {
                                DesignBlendMode::Multiply
                            } else {
                                DesignBlendMode::Normal
                            },
                        ),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .child(self.render_value_cell(
                                format!("effect-noise-size-x-{index}"),
                                "W",
                                format!("Size X · {}", format_number(settings.size.x)),
                                DesignPanelProperty::EffectNoiseSizeX(index),
                                DesignPanelValue::Number(settings.size.x + 1.),
                                cx,
                            ))
                            .child(self.render_value_cell(
                                format!("effect-noise-size-y-{index}"),
                                "H",
                                format!("Size Y · {}", format_number(settings.size.y)),
                                DesignPanelProperty::EffectNoiseSizeY(index),
                                DesignPanelValue::Number(settings.size.y + 1.),
                                cx,
                            )),
                    )
                    .child(self.render_value_cell(
                        format!("effect-noise-density-{index}"),
                        "D",
                        format!("Density · {}", format_number(settings.density)),
                        DesignPanelProperty::EffectNoiseDensity(index),
                        DesignPanelValue::Number((settings.density + 0.05).min(1.)),
                        cx,
                    ))
                    .into_any_element()
            }
            DesignEffectSettings::Texture(settings) => v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-texture-size-x-{index}"),
                            "W",
                            format!("Size X · {}", format_number(settings.size.x)),
                            DesignPanelProperty::EffectTextureSizeX(index),
                            DesignPanelValue::Number(settings.size.x + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-texture-size-y-{index}"),
                            "H",
                            format!("Size Y · {}", format_number(settings.size.y)),
                            DesignPanelProperty::EffectTextureSizeY(index),
                            DesignPanelValue::Number(settings.size.y + 1.),
                            cx,
                        )),
                )
                .child(self.render_value_cell(
                    format!("effect-texture-radius-{index}"),
                    "R",
                    format!("Radius · {}", format_number(settings.radius)),
                    DesignPanelProperty::EffectTextureRadius(index),
                    DesignPanelValue::Number(settings.radius + 1.),
                    cx,
                ))
                .child(self.render_toggle_row(
                    format!("effect-texture-clip-{index}"),
                    "Clip to shape",
                    settings.clip_to_shape,
                    DesignPanelProperty::EffectTextureClipToShape(index),
                    cx,
                ))
                .into_any_element(),
            DesignEffectSettings::Glass(settings) => v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-glass-light-intensity-{index}"),
                            "I",
                            format!("Intensity · {}", format_number(settings.light_intensity)),
                            DesignPanelProperty::EffectGlassLightIntensity(index),
                            DesignPanelValue::Number((settings.light_intensity + 0.05).min(1.)),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-glass-light-angle-{index}"),
                            "°",
                            format!("Angle · {}", format_number(settings.light_angle)),
                            DesignPanelProperty::EffectGlassLightAngle(index),
                            DesignPanelValue::Number(settings.light_angle + 15.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-glass-refraction-{index}"),
                            "R",
                            format!("Refraction · {}", format_number(settings.refraction)),
                            DesignPanelProperty::EffectGlassRefraction(index),
                            DesignPanelValue::Number((settings.refraction + 0.05).min(1.)),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-glass-depth-{index}"),
                            "D",
                            format!("Depth · {}", format_number(settings.depth)),
                            DesignPanelProperty::EffectGlassDepth(index),
                            DesignPanelValue::Number(settings.depth + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            format!("effect-glass-dispersion-{index}"),
                            "C",
                            format!("Dispersion · {}", format_number(settings.dispersion)),
                            DesignPanelProperty::EffectGlassDispersion(index),
                            DesignPanelValue::Number((settings.dispersion + 0.05).min(1.)),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            format!("effect-glass-frost-{index}"),
                            "F",
                            format!("Frost · {}", format_number(settings.frost)),
                            DesignPanelProperty::EffectGlassFrost(index),
                            DesignPanelValue::Number(settings.frost + 1.),
                            cx,
                        )),
                )
                .child(self.render_value_cell(
                    format!("effect-glass-splay-{index}"),
                    "S",
                    format!("Splay · {}", format_number(settings.splay)),
                    DesignPanelProperty::EffectGlassSplay(index),
                    DesignPanelValue::Number((settings.splay + 0.05).min(1.)),
                    cx,
                ))
                .into_any_element(),
            DesignEffectSettings::Shader(shader) => {
                let panel = cx.entity();
                let effect_id = effect.id.clone();
                let node_id = self.node.id.clone();
                let mut content = v_flex().w_full().gap_2().child(
                    Button::new(SharedString::from(format!(
                        "{}-effect-shader-source-{index}",
                        self.id
                    )))
                    .label(if shader.shader_id.is_empty() {
                        "Choose shader".into()
                    } else {
                        shader.name.clone()
                    })
                    .tooltip(if shader.imported {
                        "Change shader"
                    } else {
                        "Import or choose a shader"
                    })
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(
                        !self.can_edit()
                            || !self.collection_is_supported(DesignPanelCollection::Effect)
                            || self.node.effect_style_binding.is_some(),
                    )
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            let action = DesignPanelAction::EffectShaderChooseRequested {
                                node_id: node_id.clone(),
                                effect_id: effect_id.clone(),
                                index,
                            };
                            if this.can_edit()
                                && this.node.effect_style_binding.is_none()
                                && this.node_capability_allows_action(&action)
                            {
                                cx.emit_design_panel_action(this, action);
                            }
                        });
                    }),
                );
                for (property_index, property) in shader.properties.iter().enumerate() {
                    content = content.child(self.render_effect_shader_property(
                        index,
                        property_index,
                        property,
                        cx,
                    ));
                }
                content.into_any_element()
            }
            DesignEffectSettings::Opaque(opaque) => v_flex()
                .w_full()
                .gap_1()
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .child(opaque.summary.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} is preserved as opaque host data",
                            opaque.type_name
                        )),
                )
                .into_any_element(),
        };
        v_flex()
            .w_full()
            .gap_2()
            .child(content)
            .when_some(
                self.render_effect_variables(index, effect, cx),
                |content, variables| content.child(variables),
            )
            .into_any_element()
    }

    pub(super) fn render_effects(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(binding) = self.node.effect_style_binding.as_ref() {
            let content = v_flex()
                .px(px(PANEL_PADDING))
                .pb_4()
                .child(self.render_bound_style_summary("effects", binding.name.clone(), cx));
            return self.render_section(
                DesignPanelSection::Effects,
                Some(DesignPanelCollection::Effect),
                content.into_any_element(),
                cx,
            );
        }
        if self.node.effects.is_empty() {
            return self.render_section(
                DesignPanelSection::Effects,
                Some(DesignPanelCollection::Effect),
                div().into_any_element(),
                cx,
            );
        }
        let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_1();
        for (index, effect) in self.node.effects.iter().enumerate() {
            let can_reorder = self.can_edit()
                && self.collection_is_supported(DesignPanelCollection::Effect)
                && self.node.effect_style_binding.is_none()
                && self.node.effects.len() > 1;
            let active = self
                .active_effect_settings
                .as_ref()
                .is_some_and(|target| target.matches(effect, index));
            let panel = cx.entity();
            let panel_for_open = panel.clone();
            let panel_for_content = panel.clone();
            let effect_for_open = effect.clone();
            let effect_for_content = effect.clone();
            let effect_id = effect.id.clone();
            let panel_for_keyboard = panel.clone();
            let effect_for_keyboard = effect.clone();
            let keyboard_effect_id = effect.id.clone();
            let settings_trigger = Button::new(SharedString::from(format!(
                "{}-effect-settings-{index}",
                self.id
            )))
            .tooltip("Effect settings")
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(24.))
            .selected(active)
            .on_keyboard_activate(move |_, cx| {
                panel_for_keyboard.update(cx, |this, cx| {
                    if !active {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_menu_preview(cx);
                        this.active_effect_settings = Some(EffectSettingsTarget {
                            index,
                            effect_id: keyboard_effect_id.clone(),
                        });
                        this.effect_style_browser_open = false;
                        this.active_picker = None;
                        this.typography_style_picker_open = false;
                    } else if this
                        .active_effect_settings
                        .as_ref()
                        .is_some_and(|target| target.matches(&effect_for_keyboard, index))
                    {
                        this.cancel_menu_preview(cx);
                        this.preview_option_menu_open = None;
                        this.active_effect_settings = None;
                    }
                    cx.notify();
                });
            })
            .icon(IconName::Settings2);
            // Popover sizing applies to its deferred overlay, not this trigger.
            // Keep the 24 px dimensions on the button so the natural-width
            // settings surface can use its TopRight anchor and open inward.
            let settings_popover = Popover::new(SharedString::from(format!(
                "{}-effect-settings-popover-{index}",
                self.id
            )))
            .anchor(Anchor::TopRight)
            .open(active)
            .overlay_closable(true)
            .on_open_change(move |open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    if *open {
                        this.prepare_paint_picker_for_dismissal(cx);
                        this.cancel_menu_preview(cx);
                        this.active_effect_settings = Some(EffectSettingsTarget {
                            index,
                            effect_id: effect_id.clone(),
                        });
                        this.effect_style_browser_open = false;
                        this.active_picker = None;
                        this.typography_style_picker_open = false;
                    } else if this
                        .active_effect_settings
                        .as_ref()
                        .is_some_and(|target| target.matches(&effect_for_open, index))
                    {
                        this.cancel_menu_preview(cx);
                        this.preview_option_menu_open = None;
                        this.active_effect_settings = None;
                    }
                    cx.notify();
                });
            })
            .trigger(settings_trigger)
            .content(move |_, window, cx| {
                let effect = effect_for_content.clone();
                panel_for_content.update(cx, |this, cx| {
                    v_flex()
                        .w(popup_width(window, 292.))
                        .p_3()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .child(effect.settings.kind().label()),
                        )
                        .child(this.render_effect_settings(index, &effect, cx))
                })
            });

            let drop_effect_id = effect.id.clone();
            let can_drop_effect_id = effect.id.clone();
            let drop_background = cx.theme().selection.opacity(0.18);
            let drop_border = cx.theme().selection;
            let drag = EffectDrag {
                effect_id: effect.id.clone(),
                from_index: index,
                label: effect.settings.kind().label().into(),
            };
            let drop_listener = cx.listener(move |this, drag: &EffectDrag, _, cx| {
                this.emit_effect_reorder(drag.effect_id.clone(), drag.from_index, index, cx);
            });
            let row = h_flex()
                .id(SharedString::from(format!(
                    "{}-effect-row-{index}",
                    self.id
                )))
                .w_full()
                .h(px(ROW_HEIGHT))
                .gap_1()
                .rounded(px(4.))
                .border_1()
                .border_color(cx.theme().transparent)
                .when(can_reorder, |row| row.cursor_move())
                .child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .when(can_reorder, |handle| handle.child("⠿")),
                )
                .child(
                    div()
                        .size(px(20.))
                        .flex_none()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_hsla(effect.color)),
                )
                .child(self.render_value_cell(
                    format!("effect-kind-{index}"),
                    "",
                    effect.settings.kind().label(),
                    DesignPanelProperty::EffectKind(index),
                    DesignPanelValue::EffectKind(DesignEffectKind::DropShadow),
                    cx,
                ))
                .child(settings_popover)
                .child(self.render_visibility_button(
                    format!("effect-visible-{index}"),
                    effect.visible,
                    DesignPanelProperty::EffectVisible(index),
                    cx,
                ))
                .child(self.render_remove_button(
                    format!("remove-effect-{index}"),
                    DesignPanelCollection::Effect,
                    index,
                    cx,
                ))
                .when(can_reorder, move |row| {
                    row.on_drag(drag, |drag, _, _, cx| {
                        cx.new(|_| EffectDragPreview { drag: drag.clone() })
                    })
                    .can_drop(move |drag, _, _| {
                        drag.downcast_ref::<EffectDrag>().is_some_and(|drag| {
                            if drag.effect_id.is_empty() || can_drop_effect_id.is_empty() {
                                drag.from_index != index
                            } else {
                                drag.effect_id != can_drop_effect_id
                            }
                        })
                    })
                    .drag_over::<EffectDrag>(move |style, drag, _, _| {
                        let same = if drag.effect_id.is_empty() || drop_effect_id.is_empty() {
                            drag.from_index == index
                        } else {
                            drag.effect_id == drop_effect_id
                        };
                        if same {
                            style
                        } else {
                            style.bg(drop_background).border_color(drop_border)
                        }
                    })
                    .on_drop(drop_listener)
                });
            content = content.child(row);
        }
        self.render_section(
            DesignPanelSection::Effects,
            Some(DesignPanelCollection::Effect),
            content.into_any_element(),
            cx,
        )
    }
}

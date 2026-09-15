//! Shader discovery, binding, and property-editor rendering.

use super::*;

impl PaintPicker {
    pub(super) fn render_shader_row(
        &self,
        shader: &DesignShaderDefinition,
        selection: DesignShaderSelection,
        selected_shader_id: Option<&SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = selected_shader_id.is_some_and(|shader_id| *shader_id == shader.id);
        let selection_for_click = selection.clone();
        h_flex()
            .w_full()
            .min_h(px(34.))
            .gap_2()
            .px_2()
            .rounded(px(6.))
            .when(selected, |row| row.bg(cx.theme().accent.opacity(0.12)))
            .child(
                Icon::new(IconName::Asterisk)
                    .xsmall()
                    .text_color(if selected {
                        cx.theme().accent
                    } else {
                        cx.theme().muted_foreground
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(0.))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(shader.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(if shader.imported {
                                format!(
                                    "Imported · {} properties",
                                    shader.property_definitions.len()
                                )
                            } else {
                                "Available · import required".to_owned()
                            }),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "{}-shader-{}-{}",
                    self.id,
                    if shader.imported { "apply" } else { "import" },
                    shader.id
                )))
                .label(if shader.imported { "Apply" } else { "Import" })
                .xsmall()
                .compact()
                .outline()
                .disabled(self.editing_disabled() || (selected && shader.imported))
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.request_shader_selection(selection_for_click.clone(), cx);
                })),
            )
            .into_any_element()
    }

    pub(super) fn render_shader_browser(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_shader_id = match &paint.payload {
            DesignPaintPayload::Shader(shader) => Some(&shader.shader_id),
            _ => None,
        };
        if self.shader_view_data.page_shaders.is_empty()
            && self.shader_view_data.libraries.is_empty()
        {
            return v_flex()
                .w_full()
                .min_h(px(180.))
                .items_center()
                .justify_center()
                .gap_2()
                .p_4()
                .child(
                    Icon::new(IconName::Asterisk)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().text_sm().font_semibold().child("Shaders"))
                .child(
                    div()
                        .text_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No fill shaders supplied by the host."),
                )
                .into_any_element();
        }

        let mut content = v_flex().w_full().gap_3().p_4();
        if !self.shader_view_data.page_shaders.is_empty() {
            let mut page = v_flex()
                .w_full()
                .gap_1()
                .child(div().text_xs().font_semibold().child("In this file"));
            for shader in &self.shader_view_data.page_shaders {
                page = page.child(self.render_shader_row(
                    shader,
                    DesignShaderSelection::page(shader.id.clone()),
                    selected_shader_id,
                    cx,
                ));
            }
            content = content.child(page);
        }
        for library in &self.shader_view_data.libraries {
            let library_id = library.id.clone();
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .w_full()
                    .h(px(24.))
                    .gap_2()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(library.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(library.shaders.len().to_string()),
                    ),
            );
            for shader in &library.shaders {
                group = group.child(self.render_shader_row(
                    shader,
                    DesignShaderSelection::library(library_id.clone(), shader.id.clone()),
                    selected_shader_id,
                    cx,
                ));
            }
            content = content.child(group);
        }
        content.into_any_element()
    }

    pub(super) fn current_shader_definition(&self) -> Option<&DesignShaderDefinition> {
        let DesignPaintPayload::Shader(shader) = &self.paint.as_ref()?.payload else {
            return None;
        };
        self.shader_view_data.definition(&shader.shader_id)
    }

    pub(super) fn request_shader_selection(
        &self,
        shader: DesignShaderSelection,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(definition)) =
            (self.target.clone(), self.shader_view_data.shader(&shader))
        else {
            return;
        };
        if definition.imported {
            cx.emit(PaintPickerEvent::ShaderApplyRequested { target, shader });
        } else {
            cx.emit(PaintPickerEvent::ShaderImportRequested { target, shader });
        }
    }

    pub(super) fn request_shader_property_bind(
        &self,
        definition_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled()
            || self
                .current_shader_definition()
                .is_none_or(|shader| shader.property(&definition_id).is_none())
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyBindRequested {
            target,
            definition_id,
        });
    }

    pub(super) fn request_shader_property_editor(
        &self,
        definition_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled()
            || self
                .current_shader_definition()
                .is_none_or(|shader| shader.property(&definition_id).is_none())
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyEditorRequested {
            target,
            definition_id,
        });
    }

    pub(super) fn request_shader_property_detach(
        &self,
        definition_id: SharedString,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        let Some(DesignPaintPayload::Shader(shader)) =
            self.paint.as_ref().map(|paint| &paint.payload)
        else {
            return;
        };
        if shader
            .property(&definition_id)
            .and_then(DesignShaderPropertyValue::variable_alias_id)
            .is_none_or(|current| current != &variable_id)
        {
            return;
        }
        let Some(target) = self.target.clone() else {
            return;
        };
        cx.emit(PaintPickerEvent::ShaderPropertyDetachRequested {
            target,
            definition_id,
            variable_id,
        });
    }

    pub(super) fn render_shader_property(
        &self,
        shader: &DesignShaderPaint,
        definition: &DesignShaderPropertyDefinition,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let value = shader
            .property(&definition.id)
            .or(definition.default_value.as_ref());
        let summary = value.map_or_else(|| "No value".into(), DesignShaderPropertyValue::summary);
        let disabled = self.editing_disabled();
        let definition_id = definition.id.clone();
        let variable_id = value
            .and_then(DesignShaderPropertyValue::variable_alias_id)
            .cloned();

        let value_control = match value {
            Some(DesignShaderPropertyValue::Boolean(current)) if variable_id.is_none() => {
                let next = !current;
                Button::new(SharedString::from(format!(
                    "{}-shader-property-{}-boolean",
                    self.id, definition.id
                )))
                .label(if *current { "On" } else { "Off" })
                .xsmall()
                .compact()
                .outline()
                .disabled(disabled)
                .on_activate({
                    let definition_id = definition_id.clone();
                    cx.listener(move |this, _, _, cx| {
                        let _ = this.emit_edit(
                            DesignPaintEdit {
                                property: DesignPaintProperty::ShaderProperty {
                                    definition_id: definition_id.clone(),
                                },
                                value: DesignPaintValue::ShaderProperty(
                                    DesignShaderPropertyValue::Boolean(next),
                                ),
                            },
                            DesignPanelEditPhase::Commit,
                            cx,
                        );
                    })
                })
                .into_any_element()
            }
            Some(DesignShaderPropertyValue::Number(current))
                if variable_id.is_none() && current.is_finite() =>
            {
                let next = *current + 0.25;
                Button::new(SharedString::from(format!(
                    "{}-shader-property-{}-number",
                    self.id, definition.id
                )))
                .label(format_decimal(*current))
                .xsmall()
                .compact()
                .outline()
                .disabled(disabled)
                .on_activate({
                    let definition_id = definition_id.clone();
                    cx.listener(move |this, _, _, cx| {
                        let _ = this.emit_edit(
                            DesignPaintEdit {
                                property: DesignPaintProperty::ShaderProperty {
                                    definition_id: definition_id.clone(),
                                },
                                value: DesignPaintValue::ShaderProperty(
                                    DesignShaderPropertyValue::Number(next),
                                ),
                            },
                            DesignPanelEditPhase::Commit,
                            cx,
                        );
                    })
                })
                .into_any_element()
            }
            _ => Button::new(SharedString::from(format!(
                "{}-shader-property-{}-edit",
                self.id, definition.id
            )))
            .label(summary)
            .xsmall()
            .compact()
            .outline()
            .disabled(disabled || variable_id.is_some())
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_editor(definition_id.clone(), cx);
                })
            })
            .into_any_element(),
        };

        let variable_control = if let Some(variable_id) = variable_id {
            Button::new(SharedString::from(format!(
                "{}-shader-property-{}-detach",
                self.id, definition.id
            )))
            .label("Detach")
            .tooltip(format!("Detach variable {variable_id}"))
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_detach(
                        definition_id.clone(),
                        variable_id.clone(),
                        cx,
                    );
                })
            })
            .into_any_element()
        } else {
            Button::new(SharedString::from(format!(
                "{}-shader-property-{}-bind",
                self.id, definition.id
            )))
            .label("Bind")
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_activate({
                let definition_id = definition_id.clone();
                cx.listener(move |this, _, _, cx| {
                    this.request_shader_property_bind(definition_id.clone(), cx);
                })
            })
            .into_any_element()
        };

        v_flex()
            .w_full()
            .gap_1()
            .py_1()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .font_semibold()
                                    .child(definition.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {}",
                                        definition.kind.label(),
                                        definition.id
                                    )),
                            ),
                    )
                    .child(value_control)
                    .child(variable_control),
            )
            .when_some(definition.description.clone(), |row, description| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_shader_state(
        &self,
        shader: &DesignShaderPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut content = v_flex().w_full().gap_2().child(
            h_flex()
                .w_full()
                .p_2()
                .gap_2()
                .rounded(px(7.))
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(IconName::Asterisk)
                        .small()
                        .text_color(cx.theme().accent),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(shader.name.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(shader.shader_id.clone()),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("{}-choose-shader", self.id)))
                        .label("Choose")
                        .xsmall()
                        .compact()
                        .outline()
                        .disabled(self.editing_disabled())
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.active_tab = PaintPickerTab::Libraries;
                            this.scroll_handle.set_offset(Point::default());
                            cx.notify();
                        })),
                ),
        );

        let Some(definition) = self.shader_view_data.definition(&shader.shader_id) else {
            return content
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Shader metadata is unavailable. The host still preserves every assignment.",
                        ),
                )
                .into_any_element();
        };
        if definition.property_definitions.is_empty() {
            return content
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "Import metadata is pending; no property definitions are available.",
                        ),
                )
                .into_any_element();
        }
        content = content.child(div().text_xs().font_semibold().child("Properties"));
        for property in &definition.property_definitions {
            content = content.child(self.render_shader_property(shader, property, cx));
        }
        content.into_any_element()
    }
}

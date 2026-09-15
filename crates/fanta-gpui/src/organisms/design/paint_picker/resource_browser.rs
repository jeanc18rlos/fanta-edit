//! Variable, style, and creation resource browsers for paint editing.

use super::*;

impl PaintPicker {
    pub(super) fn resource_scopes(&self) -> Vec<PaintResourceScope> {
        let mut scopes = vec![PaintResourceScope::Page];
        for library in &self.color_style_sample_view_data.libraries {
            scopes.push(PaintResourceScope::Library {
                library_id: library.id.clone(),
                library_name: library.name.clone(),
            });
        }
        for variable in &self.paint_variable_view_data.variables {
            let DesignVariableSource::Library {
                library_id,
                library_name,
            } = &variable.source
            else {
                continue;
            };
            if scopes.iter().any(|scope| {
                matches!(
                    scope,
                    PaintResourceScope::Library {
                        library_id: existing,
                        ..
                    } if existing == library_id
                )
            }) {
                continue;
            }
            scopes.push(PaintResourceScope::Library {
                library_id: library_id.clone(),
                library_name: library_name.clone(),
            });
        }
        scopes
    }

    pub(super) fn normalize_resource_scope(&mut self) {
        let scopes = self.resource_scopes();
        let Some(index) = scopes
            .iter()
            .position(|scope| scope.same_identity(&self.resource_scope))
        else {
            self.resource_scope = PaintResourceScope::Page;
            self.resource_scope_menu_index = 0;
            self.forget_nested_overlay(PaintPickerOverlay::ResourceScope);
            return;
        };
        self.resource_scope = scopes[index].clone();
        self.resource_scope_menu_index = index;
    }

    pub(super) fn reset_resource_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.resource_search_input.read(cx).value().is_empty() {
            self.resource_search_input.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
        }
    }

    pub(super) fn open_resource_scope_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.normalize_resource_scope();
        self.open_nested_overlay(PaintPickerOverlay::ResourceScope, window, cx);
        let focus_handle = self.resource_scope_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_resource_scope_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_nested_overlay(PaintPickerOverlay::ResourceScope, window, cx);
    }

    pub(super) fn select_resource_scope(
        &mut self,
        scope: PaintResourceScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .resource_scopes()
            .iter()
            .any(|candidate| candidate.same_identity(&scope))
        {
            self.resource_scope = scope;
            self.normalize_resource_scope();
        }
        self.close_resource_scope_menu(window, cx);
    }

    pub(super) fn handle_resource_scope_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scopes = self.resource_scopes();
        let count = scopes.len();
        if count == 0 {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" => {
                self.resource_scope_menu_index =
                    (self.resource_scope_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.resource_scope_menu_index = (self.resource_scope_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.resource_scope_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.resource_scope_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_resource_scope_menu(window, cx),
            "escape" => {
                if !self.dismiss_nested_overlay(
                    PaintPickerOverlay::ResourceScope,
                    InspectorOverlayDismissCause::Escape,
                    window,
                    cx,
                ) {
                    return;
                }
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted resource scope; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    pub(super) fn commit_resource_scope_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scopes = self.resource_scopes();
        if scopes.is_empty() {
            return;
        }
        let index = self.resource_scope_menu_index.min(scopes.len() - 1);
        self.select_resource_scope(scopes[index].clone(), window, cx);
    }

    /// Replaces the host-filtered Color-variable snapshot supplied by the
    /// surrounding Design-panel host.
    pub(crate) fn set_paint_variable_view_data(
        &mut self,
        view_data: DesignPaintVariableViewData,
        cx: &mut Context<Self>,
    ) {
        if self.paint_variable_view_data != view_data {
            self.paint_variable_view_data = view_data;
            self.normalize_resource_scope();
            cx.notify();
        }
    }

    /// Supplies the exact host-owned contrast projection for the active paint.
    ///
    /// The selected audience and conformance level remain transient picker
    /// state. Ratio, effective background, Auto resolution, and correction
    /// colors never originate in the reusable component.
    pub(crate) fn set_contrast_view_data(
        &mut self,
        view_data: Option<DesignColorContrastPaintViewData>,
        cx: &mut Context<Self>,
    ) {
        let view_data = view_data.filter(DesignColorContrastPaintViewData::is_valid);
        if self.contrast_view_data == view_data {
            return;
        }
        self.contrast_view_data = view_data;
        if self.current_contrast_leaf().is_none() {
            self.contrast_checker_open = false;
        }
        cx.notify();
    }

    /// Replaces the host-filtered sample-only Color-style catalog.
    pub(crate) fn set_color_style_sample_view_data(
        &mut self,
        view_data: DesignColorStyleSampleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.color_style_sample_view_data != view_data {
            self.color_style_sample_view_data = view_data;
            self.normalize_resource_scope();
            cx.notify();
        }
    }

    /// Replaces host-controlled crop, video-preview, and media capability
    /// state without changing the paint document snapshot.
    pub(crate) fn set_media_view_data(
        &mut self,
        view_data: DesignMediaPaintViewData,
        cx: &mut Context<Self>,
    ) {
        if self.media_view_data != view_data {
            self.media_view_data = view_data;
            cx.notify();
        }
    }

    /// Replaces host-controlled fill-shader discovery/import data without
    /// mutating the current paint payload.
    pub(crate) fn set_shader_view_data(
        &mut self,
        view_data: DesignShaderViewData,
        cx: &mut Context<Self>,
    ) {
        if self.shader_view_data != view_data {
            self.shader_view_data = view_data;
            cx.notify();
        }
    }

    pub(crate) fn request_color_variable_apply(
        &self,
        variable_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        let Some(variable) = self.paint_variable_view_data.variable(variable_id.as_ref()) else {
            return;
        };
        match variable.import_state {
            DesignVariableImportState::Available => {
                cx.emit(PaintPickerEvent::ColorVariableImportRequested {
                    target,
                    color_target,
                    variable_id,
                });
            }
            DesignVariableImportState::Local | DesignVariableImportState::Imported => {
                cx.emit(PaintPickerEvent::ColorVariableApplyRequested {
                    target,
                    color_target,
                    variable_id,
                });
            }
        }
    }

    pub(crate) fn request_color_variable_detach(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target), Some(binding)) = (
            self.target.clone(),
            self.selected_color_target(),
            self.paint
                .as_ref()
                .and_then(|paint| selected_color_binding(paint, self.selected_stop)),
        ) else {
            return;
        };
        cx.emit(PaintPickerEvent::ColorVariableDetachRequested {
            target,
            color_target,
            variable_id: binding.variable_id.clone(),
        });
    }

    pub(crate) fn request_color_variable_create(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target), Some(color)) = (
            self.target.clone(),
            self.selected_color_target(),
            self.current_color(),
        ) else {
            return;
        };
        cx.emit(PaintPickerEvent::ColorVariableCreateRequested {
            target,
            color_target,
            color,
        });
    }

    pub(crate) fn request_paint_style_create(&self, cx: &mut Context<Self>) {
        if self.editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        cx.emit(PaintPickerEvent::PaintStyleCreateRequested {
            target,
            color_target,
        });
    }

    pub(super) fn open_creation_menu_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_disabled() || self.selected_color_target().is_none() {
            return;
        }
        self.creation_menu_index = PaintCreationKind::Style as usize;
        self.open_nested_overlay(PaintPickerOverlay::Creation, window, cx);
        let focus_handle = self.creation_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_creation_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_nested_overlay(PaintPickerOverlay::Creation, window, cx);
    }

    pub(super) fn activate_creation_kind(
        &mut self,
        kind: PaintCreationKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            PaintCreationKind::Style => self.request_paint_style_create(cx),
            PaintCreationKind::Variable => self.request_color_variable_create(cx),
        }
        self.close_creation_menu(window, cx);
    }

    pub(super) fn handle_creation_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = PaintCreationKind::ALL.len();
        match event.keystroke.key.as_str() {
            "up" => {
                self.creation_menu_index = (self.creation_menu_index + count - 1) % count;
                cx.notify();
            }
            "down" => {
                self.creation_menu_index = (self.creation_menu_index + 1) % count;
                cx.notify();
            }
            "home" => {
                self.creation_menu_index = 0;
                cx.notify();
            }
            "end" => {
                self.creation_menu_index = count - 1;
                cx.notify();
            }
            "enter" | "space" => self.commit_creation_menu(window, cx),
            "escape" => {
                if !self.dismiss_nested_overlay(
                    PaintPickerOverlay::Creation,
                    InspectorOverlayDismissCause::Escape,
                    window,
                    cx,
                ) {
                    return;
                }
            }
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    /// Commits the highlighted creation kind; bound to the shared
    /// `ActivateControl` command on the roving menu surface.
    pub(super) fn commit_creation_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let count = PaintCreationKind::ALL.len();
        let kind = PaintCreationKind::ALL[self.creation_menu_index.min(count - 1)];
        self.activate_creation_kind(kind, window, cx);
    }

    pub(crate) fn request_color_style_sample(
        &self,
        sample: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) {
        if self.color_editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        if self
            .color_style_sample_view_data
            .sample(&sample)
            .is_none_or(|sample| sample.disabled_reason.is_some())
        {
            return;
        }
        cx.emit(PaintPickerEvent::ColorStyleSampleRequested {
            target,
            color_target,
            sample,
        });
    }

    pub(crate) fn request_eyedropper(&self, cx: &mut Context<Self>) {
        if self.color_editing_disabled() {
            return;
        }
        let (Some(target), Some(color_target)) =
            (self.target.clone(), self.selected_color_target())
        else {
            return;
        };
        cx.emit(PaintPickerEvent::EyedropperRequested {
            target,
            color_target,
        });
    }

    pub(super) fn render_creation_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let highlighted_index = self.creation_menu_index;
        let menu_focus_handle = self.creation_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let disabled = self.editing_disabled() || self.selected_color_target().is_none();
        let trigger = Button::new(SharedString::from(format!("{}-create", self.id)))
            .icon(IconName::Plus)
            .tooltip("Create style or variable")
            .xsmall()
            .compact()
            .ghost()
            .disabled(disabled)
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.nested_overlay_is_open(PaintPickerOverlay::Creation) {
                        this.close_creation_menu(window, cx);
                    } else {
                        this.open_creation_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!("{}-creation-menu", self.id)))
            .anchor(Anchor::BottomRight)
            .open(self.nested_overlay_is_open(PaintPickerOverlay::Creation))
            .track_focus(&menu_focus_handle)
            .overlay_closable(true)
            .on_open_change(move |open, window, cx| {
                picker_for_open.update(cx, |this, cx| {
                    if *open && !this.editing_disabled() && this.selected_color_target().is_some() {
                        this.creation_menu_index = PaintCreationKind::Style as usize;
                        this.open_nested_overlay(PaintPickerOverlay::Creation, window, cx);
                    } else {
                        this.dismiss_nested_overlay(
                            PaintPickerOverlay::Creation,
                            InspectorOverlayDismissCause::OutsideClick,
                            window,
                            cx,
                        );
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, _| {
                v_flex()
                    .id(SharedString::from(format!("{picker_id}-creation-options")))
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                    .on_action({
                        let picker = picker_for_content.clone();
                        move |_: &ActivateControl, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.commit_creation_menu(window, cx);
                            });
                        }
                    })
                    .on_key_down({
                        let picker = picker_for_content.clone();
                        move |event: &KeyDownEvent, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.handle_creation_menu_key(event, window, cx);
                            });
                        }
                    })
                    .w(popup_width(window, 144.))
                    .gap_1()
                    .children(PaintCreationKind::ALL.into_iter().enumerate().map(
                        |(index, kind)| {
                            let picker = picker_for_content.clone();
                            Button::new(SharedString::from(format!(
                                "{}-{}",
                                picker_id,
                                kind.label().to_lowercase().replace(' ', "-")
                            )))
                            .label(kind.label())
                            .tooltip(kind.label())
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .tab_stop(false)
                            .selected(highlighted_index == index)
                            .on_activate(move |_, window, cx| {
                                picker.update(cx, |this, cx| {
                                    this.activate_creation_kind(kind, window, cx);
                                });
                            })
                        },
                    ))
            })
            .into_any_element()
    }

    pub(super) fn render_color_style_sample_swatch(
        &self,
        sample: &DesignColorStyleSample,
        selection: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-page-color-style-sample-{}",
            self.id, sample.id
        )))
        .tooltip(format!("Color style: {}", sample.name))
        .xsmall()
        .compact()
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .disabled(
            self.color_editing_disabled()
                || self.selected_color_target().is_none()
                || sample.disabled_reason.is_some(),
        )
        .child(
            div()
                .relative()
                .size(px(20.))
                .overflow_hidden()
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.18),
                    0.45,
                    0.45,
                ))
                .child(div().absolute().size_full().bg(color_to_hsla(sample.color))),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_style_sample(selection.clone(), cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_color_style_sample_row(
        &self,
        sample: &DesignColorStyleSample,
        selection: DesignColorStyleSampleSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-library-color-style-sample-{}",
            self.id, sample.id
        )))
        .w_full()
        .h(px(28.))
        .px_2()
        .compact()
        .ghost()
        .disabled(
            self.color_editing_disabled()
                || self.selected_color_target().is_none()
                || sample.disabled_reason.is_some(),
        )
        .tooltip(
            sample
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "Sample this Color style without binding it".into()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w(px(0.))
                .gap_2()
                .child(
                    div()
                        .size(px(18.))
                        .flex_none()
                        .rounded(px(9.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_to_hsla(sample.color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_left()
                        .child(sample.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Style"),
                ),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_style_sample(selection.clone(), cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_color_variable_row(
        &self,
        variable: &DesignVariable,
        selected_binding: Option<&DesignPaintBinding>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = paint_variable_matches_binding(variable, selected_binding);
        let disabled = self.editing_disabled()
            || self.selected_color_target().is_none()
            || variable.disabled_reason.is_some();
        let variable_id = variable.id.clone();
        let source_id = match &variable.source {
            DesignVariableSource::Page { page_id, .. } => page_id,
            DesignVariableSource::Library { library_id, .. } => library_id,
        };
        let color = paint_variable_color(variable);
        let label = if variable.import_state == DesignVariableImportState::Available {
            format!("Import {}", variable.name)
        } else {
            variable.name.to_string()
        };
        Button::new(SharedString::from(format!(
            "{}-paint-variable-{}-{}",
            self.id, source_id, variable.id
        )))
        .w_full()
        .h(px(28.))
        .px_2()
        .compact()
        .ghost()
        .selected(selected)
        .disabled(disabled)
        .tooltip(
            variable
                .disabled_reason
                .clone()
                .unwrap_or_else(|| variable.collection_name.clone()),
        )
        .child(
            h_flex()
                .w_full()
                .min_w(px(0.))
                .gap_2()
                .child(
                    div()
                        .size(px(18.))
                        .flex_none()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(color_to_hsla(color)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_left()
                        .child(label),
                )
                .child(
                    div()
                        .max_w(px(74.))
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(variable.collection_name.clone()),
                )
                .when(selected, |row| {
                    row.child(
                        Icon::new(IconName::Check)
                            .xsmall()
                            .text_color(cx.theme().foreground),
                    )
                }),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_variable_apply(variable_id.clone(), cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_color_variable_swatch(
        &self,
        variable: &DesignVariable,
        selected_binding: Option<&DesignPaintBinding>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = paint_variable_matches_binding(variable, selected_binding);
        let disabled = self.editing_disabled()
            || self.selected_color_target().is_none()
            || variable.disabled_reason.is_some();
        let color = paint_variable_color(variable);
        let variable_id = variable.id.clone();
        Button::new(SharedString::from(format!(
            "{}-page-paint-variable-swatch-{}",
            self.id, variable.id
        )))
        .tooltip(format!("{} — {}", variable.name, variable.collection_name))
        .xsmall()
        .compact()
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .selected(selected)
        .disabled(disabled)
        .child(
            div()
                .relative()
                .size(px(20.))
                .overflow_hidden()
                .rounded(px(4.))
                .border_1()
                .border_color(if selected {
                    cx.theme().selection
                } else {
                    cx.theme().border
                })
                .bg(pattern_slash(
                    cx.theme().muted_foreground.opacity(0.18),
                    0.45,
                    0.45,
                ))
                .child(div().absolute().size_full().bg(color_to_hsla(color)))
                .when(selected, |swatch| {
                    swatch.child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::Check)
                                    .xsmall()
                                    .text_color(cx.theme().foreground),
                            ),
                    )
                }),
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            this.request_color_variable_apply(variable_id.clone(), cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_color_variable_detach_button(
        &self,
        scope: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!(
            "{}-detach-{}-paint-variable",
            self.id, scope
        )))
        .label("Detach current variable")
        .xsmall()
        .compact()
        .ghost()
        .w_full()
        .disabled(self.editing_disabled())
        .on_activate(cx.listener(|this, _, _, cx| {
            this.request_color_variable_detach(cx);
        }))
        .into_any_element()
    }

    pub(super) fn render_resource_scope_selector(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker_for_open = cx.entity();
        let picker_for_trigger = picker_for_open.clone();
        let picker_for_content = picker_for_open.clone();
        let picker_id = self.id.clone();
        let scopes = self.resource_scopes();
        let highlighted_index = self
            .resource_scope_menu_index
            .min(scopes.len().saturating_sub(1));
        let menu_focus_handle = self.resource_scope_menu_focus_handle.clone();
        let menu_focus_for_content = menu_focus_handle.clone();
        let trigger = Button::new(SharedString::from(format!("{}-resource-scope", self.id)))
            .label(self.resource_scope.label())
            .dropdown_caret(true)
            .tooltip("Choose a page or library palette")
            .xsmall()
            .compact()
            .outline()
            .w_full()
            .on_keyboard_activate(move |window, cx| {
                picker_for_trigger.update(cx, |this, cx| {
                    if this.nested_overlay_is_open(PaintPickerOverlay::ResourceScope) {
                        this.close_resource_scope_menu(window, cx);
                    } else {
                        this.open_resource_scope_menu_from_keyboard(window, cx);
                    }
                });
            });

        Popover::new(SharedString::from(format!(
            "{}-resource-scope-menu",
            self.id
        )))
        .anchor(Anchor::BottomLeft)
        .open(self.nested_overlay_is_open(PaintPickerOverlay::ResourceScope))
        .track_focus(&menu_focus_handle)
        .overlay_closable(true)
        .on_open_change(move |open, window, cx| {
            picker_for_open.update(cx, |this, cx| {
                if *open {
                    this.normalize_resource_scope();
                    this.open_nested_overlay(PaintPickerOverlay::ResourceScope, window, cx);
                } else {
                    this.dismiss_nested_overlay(
                        PaintPickerOverlay::ResourceScope,
                        InspectorOverlayDismissCause::OutsideClick,
                        window,
                        cx,
                    );
                }
            });
        })
        .trigger(trigger)
        .content(move |_, window, _| {
            v_flex()
                .id(SharedString::from(format!(
                    "{picker_id}-resource-scope-options"
                )))
                .key_context(CONTROL_KEY_CONTEXT)
                .track_focus(&menu_focus_for_content.clone().tab_index(0).tab_stop(true))
                .on_action({
                    let picker = picker_for_content.clone();
                    move |_: &ActivateControl, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.commit_resource_scope_menu(window, cx);
                        });
                    }
                })
                .on_key_down({
                    let picker = picker_for_content.clone();
                    move |event: &KeyDownEvent, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.handle_resource_scope_menu_key(event, window, cx);
                        });
                    }
                })
                .w(popup_width(window, 216.))
                .max_h(popup_height(window, 280.))
                .overflow_y_scroll()
                .gap_1()
                .children(scopes.iter().cloned().enumerate().map(|(index, scope)| {
                    let picker = picker_for_content.clone();
                    let option_id = match &scope {
                        PaintResourceScope::Page => "page".to_owned(),
                        PaintResourceScope::Library { library_id, .. } => {
                            format!("library-{library_id}")
                        }
                    };
                    Button::new(SharedString::from(format!(
                        "{picker_id}-resource-scope-{option_id}"
                    )))
                    .label(scope.label())
                    .tooltip(scope.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .tab_stop(false)
                    .selected(highlighted_index == index)
                    .on_activate(move |_, window, cx| {
                        picker.update(cx, |this, cx| {
                            this.select_resource_scope(scope.clone(), window, cx);
                        });
                    })
                }))
        })
        .into_any_element()
    }

    pub(super) fn render_variable_scope(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_binding = selected_color_binding(paint, self.selected_stop);
        let scoped_style_samples = match &self.resource_scope {
            PaintResourceScope::Page => self
                .color_style_sample_view_data
                .page_samples
                .iter()
                .collect::<Vec<_>>(),
            PaintResourceScope::Library { library_id, .. } => self
                .color_style_sample_view_data
                .libraries
                .iter()
                .find(|library| library.id == *library_id)
                .map_or_else(Vec::new, |library| library.samples.iter().collect()),
        };
        let scoped_variables = self
            .paint_variable_view_data
            .variables
            .iter()
            .filter(|variable| match (&self.resource_scope, &variable.source) {
                (PaintResourceScope::Page, DesignVariableSource::Page { .. }) => true,
                (
                    PaintResourceScope::Library { library_id, .. },
                    DesignVariableSource::Library {
                        library_id: source_id,
                        ..
                    },
                ) => library_id == source_id,
                (PaintResourceScope::Page, DesignVariableSource::Library { .. })
                | (PaintResourceScope::Library { .. }, DesignVariableSource::Page { .. }) => false,
            })
            .collect::<Vec<_>>();
        let mut style_samples = h_flex().w_full().gap_2().flex_wrap();
        if scoped_style_samples.is_empty() {
            style_samples = style_samples.child(
                div()
                    .w_full()
                    .min_h(px(24.))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No Color styles in this source"),
            );
        } else {
            for sample in scoped_style_samples {
                let selection = match &self.resource_scope {
                    PaintResourceScope::Page => {
                        DesignColorStyleSampleSelection::page(sample.id.clone())
                    }
                    PaintResourceScope::Library { library_id, .. } => {
                        DesignColorStyleSampleSelection::library(
                            library_id.clone(),
                            sample.id.clone(),
                        )
                    }
                };
                style_samples = style_samples
                    .child(self.render_color_style_sample_swatch(sample, selection, cx));
            }
        }
        let mut variables = h_flex().w_full().gap_2().flex_wrap();
        if scoped_variables.is_empty() {
            variables = variables.child(
                div()
                    .w_full()
                    .min_h(px(24.))
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No Color variables in this source"),
            );
        } else {
            for variable in scoped_variables {
                variables = variables.child(self.render_color_variable_swatch(
                    variable,
                    selected_binding,
                    cx,
                ));
            }
        }

        v_flex()
            .w_full()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(self.render_resource_scope_selector(cx))
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color styles"),
            )
            .child(style_samples)
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color variables"),
            )
            .child(variables)
            .when(selected_binding.is_some(), |scope| {
                scope.child(self.render_color_variable_detach_button("page", cx))
            })
            .into_any_element()
    }

    pub(super) fn render_variables(
        &self,
        paint: &DesignPaint,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected_binding = selected_color_binding(paint, self.selected_stop);
        let query = self
            .resource_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let style_libraries = self
            .color_style_sample_view_data
            .libraries
            .iter()
            .filter_map(|library| {
                let samples = library
                    .samples
                    .iter()
                    .filter(|sample| {
                        paint_resource_matches(
                            &query,
                            [sample.name.as_ref(), library.name.as_ref()],
                        )
                    })
                    .collect::<Vec<_>>();
                (!samples.is_empty()).then_some((library, samples))
            })
            .collect::<Vec<_>>();
        let mut libraries: Vec<(SharedString, SharedString, Vec<&DesignVariable>)> = Vec::new();
        for variable in &self.paint_variable_view_data.variables {
            let DesignVariableSource::Library {
                library_id,
                library_name,
            } = &variable.source
            else {
                continue;
            };
            if !paint_resource_matches(
                &query,
                [
                    variable.name.as_ref(),
                    variable.collection_name.as_ref(),
                    library_name.as_ref(),
                ],
            ) {
                continue;
            }
            if let Some((_, _, variables)) =
                libraries.iter_mut().find(|(id, _, _)| id == library_id)
            {
                variables.push(variable);
            } else {
                libraries.push((library_id.clone(), library_name.clone(), vec![variable]));
            }
        }
        let mut content = v_flex()
            .w_full()
            .gap_3()
            .p_4()
            .child(Input::new(&self.resource_search_input).xsmall().h(px(28.)));
        if selected_binding.is_some() {
            content = content.child(self.render_color_variable_detach_button("library", cx));
        }
        if libraries.is_empty() && style_libraries.is_empty() {
            let message = if query.is_empty() {
                "No Color styles or variables supplied by the host."
            } else {
                "No styles or variables match this search."
            };
            return content
                .w_full()
                .min_h(px(180.))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::BookOpen)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().text_sm().font_semibold().child("Color libraries"))
                .child(
                    div()
                        .text_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(message),
                )
                .into_any_element();
        }

        if !style_libraries.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color styles"),
            );
        }
        for (library, samples) in style_libraries {
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
                            .child(samples.len().to_string()),
                    ),
            );
            for sample in samples {
                group = group.child(self.render_color_style_sample_row(
                    sample,
                    DesignColorStyleSampleSelection::library(library.id.clone(), sample.id.clone()),
                    cx,
                ));
            }
            content = content.child(group);
        }

        if !libraries.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child("Color variables"),
            );
        }
        for (_, library_name, variables) in libraries {
            let count = variables.len();
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
                            .child(library_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(count.to_string()),
                    ),
            );
            for variable in variables {
                group = group.child(self.render_color_variable_row(variable, selected_binding, cx));
            }
            content = content.child(group);
        }
        content.into_any_element()
    }
}

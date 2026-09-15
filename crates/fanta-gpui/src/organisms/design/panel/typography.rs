use super::*;

/// Project Typography-owned host, catalog, access, and presentation state at
/// the compatibility boundary. The standalone renderer never reads the panel
/// facade directly.
pub(super) fn projection(
    panel: &DesignPanel,
    cx: &App,
) -> Option<sections::typography::TypographyProjection> {
    let typography = panel.host.inspected_node().typography.clone()?;
    let panel_target = panel.command_target();
    let query = panel.retained.inputs.font_search.read(cx).value();
    let font_browser = sections::typography::font_browser::FontBrowserProjection::from_panel(
        panel,
        query.as_ref(),
    );
    let style_picker = sections::typography::style_picker::TypographyStylePickerProjection::new(
        panel.id.clone(),
        panel_target.clone(),
        panel.overlays.typography_style_picker_open(),
        panel.property_is_editable(DesignPanelProperty::TypographyStyle),
        panel.typography_style_picker.clone(),
    );
    let type_settings =
        sections::typography::type_settings::TypeSettingsOverlayProjection::from_panel(
            panel,
            !typography.variable_axes.is_empty(),
        );

    Some(sections::typography::TypographyProjection::new(
        sections::typography::TypographyIdentityProjection::new(
            panel.id.clone(),
            panel_target,
            panel.typography_target(DesignPanelProperty::FontFamily),
        ),
        sections::typography::TypographyValuesProjection::new(
            typography,
            panel.host.inspected_node().text_path,
            panel.host.inspected_node().text_path_start_data,
        ),
        sections::typography::TypographyAccessProjection::new(
            panel.can_edit(),
            panel.property_is_editable(DesignPanelProperty::HorizontalTextAlignment),
            panel.property_is_editable(DesignPanelProperty::VerticalTextAlignment),
            panel.text_path_flip_is_available(),
            panel.text_path_start_debug_controls_are_available(),
        ),
        sections::typography::TypographyResourceProjection::new(
            font_browser,
            panel.retained.inputs.font_search.clone(),
            style_picker,
        ),
        sections::typography::TypographyPresentationProjection::new(
            panel.sections.is_expanded(DesignPanelSection::Typography),
            type_settings,
        ),
    ))
}

/// Internal controller for Typography targeting, transient edits, resources,
/// and compatibility chrome used by the extracted Typography section.
pub(super) trait DesignTypographyController: Sized {
    fn text_path_flip_is_available(&self) -> bool;
    fn text_path_start_debug_controls_are_available(&self) -> bool;
    fn text_max_lines_are_available(&self) -> bool;
    fn typography_target(&self, property: DesignPanelProperty) -> DesignTypographyTarget;
    fn variable_font_axis(&self, tag: &str) -> Option<&DesignFontAxis>;
    fn variable_font_axis_target(&self) -> DesignTypographyTarget;
    fn variable_font_axis_is_editable(&self, tag: &str) -> bool;
    fn variable_font_axis_editor_survives(&self, editor: &VariableFontAxisEditor) -> bool;
    fn emit_variable_font_axis_event(
        &self,
        event: DesignVariableFontAxisEditEvent,
        cx: &mut Context<Self>,
    );
    fn begin_variable_font_axis_edit(&mut self, tag: SharedString, cx: &mut Context<Self>) -> bool;
    fn activate_variable_font_axis_input(
        &mut self,
        tag: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn parsed_variable_font_axis_draft(&self, cx: &App) -> Option<Result<f32, ()>>;
    fn validate_variable_font_axis_draft(&mut self, cx: &mut Context<Self>);
    fn cancel_variable_font_axis_transaction(&mut self, cx: &mut Context<Self>);
    fn finish_variable_font_axis_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn step_variable_font_axis_input(
        &mut self,
        direction: ArrowStep,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool;
    fn apply_variable_font_axis_keyboard_step(
        &mut self,
        tag: SharedString,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool;
    fn handle_variable_font_axis_slider_key_down(
        &mut self,
        tag: SharedString,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    fn start_variable_font_axis_scrub(
        &mut self,
        tag: SharedString,
        position_x: f32,
        position_y: f32,
        cx: &mut Context<Self>,
    ) -> bool;
    fn snapped_variable_font_axis_value(
        editor: &VariableFontAxisEditor,
        value: f32,
        fine: bool,
    ) -> f32;
    fn update_variable_font_axis_scrub(
        &mut self,
        tag: &str,
        position_x: f32,
        position_y: f32,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    );
    fn finish_variable_font_axis_scrub(
        &mut self,
        commit: bool,
        suppress_click: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool;
    fn text_path_start_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<DesignTextPathStartData>;
    fn typography_style_binding(&self) -> Option<DesignTypographyStyleBinding>;
    fn sync_typography_style_picker(&self, cx: &mut Context<Self>);
    fn open_typography_style_picker(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn render_typography_style_button(&self, cx: &mut Context<Self>) -> AnyElement;
    fn open_font_browser(&mut self, window: &mut Window, cx: &mut Context<Self>);
    fn emit_font_apply(&mut self, font: DesignFontSelection, cx: &mut Context<Self>);
    fn emit_font_import(&mut self, font: DesignFontSelection, cx: &mut Context<Self>);
    fn emit_open_type_feature(
        &mut self,
        tag: DesignOpenTypeFeatureTag,
        enabled: bool,
        cx: &mut Context<Self>,
    );
    fn emit_text_path_flip_orientation(&mut self, cx: &mut Context<Self>);
    #[cfg(test)]
    fn render_font_browser(&self, cx: &mut Context<Self>) -> AnyElement;
    fn render_text_resize_icon(&self, icon: TextResizeIcon, cx: &mut Context<Self>) -> AnyElement;
    fn render_text_resize_control(
        &self,
        current: DesignTextResize,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_type_setting_segments(
        panel_id: SharedString,
        id_suffix: &'static str,
        options: Vec<(SharedString, bool, DesignPanelValue)>,
        enabled: bool,
        property: DesignPanelProperty,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement;
    #[allow(clippy::too_many_arguments)]
    fn render_type_setting_number_field(
        panel_id: SharedString,
        id_suffix: &'static str,
        label: SharedString,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        enabled: bool,
        editing: bool,
        invalid: bool,
        input: Entity<InputState>,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement;
    fn render_variable_font_axis_number_field(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement;
    fn render_variable_font_axis_slider(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement;
    fn render_variable_font_axis_row(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement;
    #[cfg(test)]
    fn render_type_settings_popover(
        &self,
        typography: &super::super::DesignTypography,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_type_settings_popover_projected(
        &self,
        typography: &super::super::DesignTypography,
        overlay: sections::typography::type_settings::TypeSettingsOverlayProjection,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_typography(&self, cx: &mut Context<Self>) -> Option<AnyElement>;
}

impl DesignTypographyController for DesignPanel {
    fn text_path_flip_is_available(&self) -> bool {
        self.host.inspected_node().kind == DesignPanelNodeKind::TextPath
            && self
                .host
                .inspected_node()
                .supports_section(DesignPanelSection::Typography)
            && self
                .host
                .inspected_node()
                .text_path
                .as_ref()
                .is_some_and(|text_path| text_path.can_flip_orientation)
    }

    fn text_path_start_debug_controls_are_available(&self) -> bool {
        self.host.inspected_node().kind == DesignPanelNodeKind::TextPath
            && self
                .host
                .inspected_node()
                .supports_section(DesignPanelSection::Typography)
            && self.host.inspected_node().text_path_start_data.is_some()
            && self
                .host
                .inspected_node()
                .text_path
                .as_ref()
                .is_some_and(|text_path| text_path.show_start_data_debug_controls)
    }

    /// Mirrors Figma's contextual Max lines disclosure. The row exists only
    /// for ending-truncated Auto width/Auto height text, and an auto-layout
    /// child additionally has to use vertical Hug sizing.
    fn text_max_lines_are_available(&self) -> bool {
        self.host.inspected_node().text_max_lines_are_available(
            self.host
                .inspection_context
                .parent_layout()
                .is_auto_layout(),
        )
    }

    fn typography_target(&self, property: DesignPanelProperty) -> DesignTypographyTarget {
        if self.host.inspection_context.edit_mode() == DesignPanelEditMode::Text
            && property.supports_selected_text_range()
        {
            self.host.inspection_context.text_range_revision().map_or(
                DesignTypographyTarget::SelectedTextRange,
                DesignTypographyTarget::SelectedTextRangeRevision,
            )
        } else {
            DesignTypographyTarget::WholeLayer
        }
    }

    fn variable_font_axis(&self, tag: &str) -> Option<&DesignFontAxis> {
        let axes = &self
            .host
            .inspected_node()
            .typography
            .as_ref()?
            .variable_axes;
        let mut matching = axes.iter().filter(|axis| axis.tag.as_ref() == tag);
        let axis = matching.next()?;
        matching.next().is_none().then_some(axis)
    }

    fn variable_font_axis_target(&self) -> DesignTypographyTarget {
        self.typography_target(DesignPanelProperty::FontWeight)
    }

    fn variable_font_axis_is_editable(&self, tag: &str) -> bool {
        self.can_edit()
            && self
                .host
                .inspected_node()
                .typography
                .as_ref()
                .is_some_and(|typography| typography.style_binding.is_none())
            && self
                .variable_font_axis(tag)
                .is_some_and(DesignFontAxis::is_editable)
    }

    fn variable_font_axis_editor_survives(&self, editor: &VariableFontAxisEditor) -> bool {
        editor.target == self.variable_font_axis_target()
            && self.variable_font_axis_is_editable(editor.tag.as_ref())
            && self
                .variable_font_axis(editor.tag.as_ref())
                .is_some_and(|axis| {
                    axis.min == editor.min
                        && axis.max == editor.max
                        && axis.default == editor.default
                        && axis.step == editor.step
                })
    }

    fn emit_variable_font_axis_event(
        &self,
        event: DesignVariableFontAxisEditEvent,
        cx: &mut Context<Self>,
    ) {
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyVariableAxisEditRequested {
                node_id: self.host.inspected_node().id.clone(),
                target: event.target,
                tag: event.tag,
                value: event.value,
                phase: event.phase,
            },
        );
    }

    fn begin_variable_font_axis_edit(&mut self, tag: SharedString, cx: &mut Context<Self>) -> bool {
        if self.edit.property.is_some()
            || self.edit.numeric_scrub.is_some()
            || self.edit.variable_font_axis.is_some()
            || !self.variable_font_axis_is_editable(tag.as_ref())
        {
            return false;
        }
        let Some(axis) = self.variable_font_axis(tag.as_ref()).cloned() else {
            return false;
        };
        self.cancel_menu_preview(cx);
        self.overlays.discard(DesignOpenOverlay::PaintPicker);
        self.overlays.discard(DesignOpenOverlay::EffectSettings);
        self.overlays.discard(DesignOpenOverlay::EffectStyle);
        self.overlays.discard(DesignOpenOverlay::PropertyVariable);
        self.overlays
            .discard(DesignOpenOverlay::ComponentPropertyVariable);
        self.overlays.discard(DesignOpenOverlay::ComponentSwap);
        let editor = VariableFontAxisEditor::new(tag, self.variable_font_axis_target(), &axis);
        let displayed = format_number(editor.original);
        let Some(begin) = self.edit.begin_variable_font_axis_edit(editor, &displayed) else {
            return false;
        };
        self.emit_variable_font_axis_event(begin, cx);
        true
    }

    fn activate_variable_font_axis_input(
        &mut self,
        tag: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edit.suppress_next_control_activation {
            self.edit.suppress_next_control_activation = false;
            return;
        }
        if self
            .edit
            .variable_font_axis
            .as_ref()
            .is_some_and(|editor| editor.tag == tag)
        {
            return;
        }
        if !self.begin_variable_font_axis_edit(tag, cx) {
            return;
        }
        let Some(editor) = self.edit.variable_font_axis.as_ref() else {
            return;
        };
        let draft = format_number(editor.original);
        self.edit.suppress_property_input_change = true;
        self.retained.inputs.property.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.edit.suppress_property_input_change = false;
        cx.notify();
    }

    fn parsed_variable_font_axis_draft(&self, cx: &App) -> Option<Result<f32, ()>> {
        let editor = self.edit.variable_font_axis.as_ref()?;
        let clamp =
            NumericClamp::new(Some(f64::from(editor.min)), Some(f64::from(editor.max))).ok();
        let input_draft = self.retained.inputs.property.read(cx).value();
        let draft = editor.field.draft().unwrap_or(input_draft.as_ref());
        Some(
            evaluate_numeric_expression(draft, f64::from(editor.original), clamp)
                .map_err(|_| ())
                .and_then(|value| {
                    (value >= f64::from(f32::MIN) && value <= f64::from(f32::MAX))
                        .then_some(value as f32)
                        .filter(|value| value.is_finite())
                        .ok_or(())
                }),
        )
    }

    fn validate_variable_font_axis_draft(&mut self, cx: &mut Context<Self>) {
        let draft = self.retained.inputs.property.read(cx).value();
        let _ = self.edit.sync_variable_font_axis_draft(draft.as_ref());
        let parsed = self.parsed_variable_font_axis_draft(cx);
        self.edit.variable_font_axis_invalid = parsed.as_ref().is_some_and(Result::is_err);
        if !self.edit.suppress_property_input_change
            && !self.edit.variable_font_axis_invalid
            && let Some(Ok(value)) = parsed
        {
            let editor = self.edit.variable_font_axis.as_ref().cloned();
            if editor
                .as_ref()
                .is_some_and(|editor| self.variable_font_axis_editor_survives(editor))
                && let Some(event) = self.edit.preview_variable_font_axis_edit(
                    editor
                        .as_ref()
                        .expect("axis preview requires an active editor")
                        .tag
                        .as_ref(),
                    value,
                    Some(draft.as_ref()),
                )
            {
                self.emit_variable_font_axis_event(event, cx);
            }
        }
        cx.notify();
    }

    fn cancel_variable_font_axis_transaction(&mut self, cx: &mut Context<Self>) {
        if let Some(event) = self.edit.finish_variable_font_axis_edit(None) {
            self.emit_variable_font_axis_event(event, cx);
        }
    }

    fn finish_variable_font_axis_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .edit
            .variable_font_axis_scrub
            .as_ref()
            .is_some_and(|scrub| scrub.active)
        {
            self.finish_variable_font_axis_scrub(commit, false, window, cx);
            return;
        }
        let Some(editor) = self.edit.variable_font_axis.as_ref().cloned() else {
            return;
        };
        let draft = self.retained.inputs.property.read(cx).value();
        let _ = self.edit.sync_variable_font_axis_draft(draft.as_ref());
        let value = commit
            .then(|| self.parsed_variable_font_axis_draft(cx))
            .flatten()
            .and_then(Result::ok)
            .filter(|_| self.variable_font_axis_editor_survives(&editor));
        if let Some(event) = self.edit.finish_variable_font_axis_edit(value) {
            self.emit_variable_font_axis_event(event, cx);
        }
        self.overlays.type_settings_focus().focus(window, cx);
        cx.notify();
    }

    fn step_variable_font_axis_input(
        &mut self,
        direction: ArrowStep,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // A key event may run before InputState's change notification. Keep
        // the shared numeric field authoritative by synchronizing the visible
        // draft first; otherwise an invalid draft would nudge the last valid
        // controller value instead of being rejected.
        let draft = self.retained.inputs.property.read(cx).value();
        let _ = self.edit.sync_variable_font_axis_draft(draft.as_ref());
        let Some(editor) = self.edit.variable_font_axis.clone() else {
            return false;
        };
        if !self.variable_font_axis_editor_survives(&editor) {
            return false;
        }
        let Some(Ok(current)) = self.parsed_variable_font_axis_draft(cx) else {
            return false;
        };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        let delta = editor.step * self.preferences.nudge_settings.amount(modifiers.shift) * fine;
        let value = match direction {
            ArrowStep::Increase => current + delta,
            ArrowStep::Decrease => current - delta,
        }
        .clamp(editor.min, editor.max);
        let draft = format_nudge_number(value);
        let preview =
            self.edit
                .nudge_variable_font_axis_edit(editor.tag.as_ref(), current, value, &draft);
        self.edit.suppress_property_input_change = true;
        self.retained.inputs.property.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
        });
        self.edit.suppress_property_input_change = false;
        if let Some(preview) = preview {
            self.emit_variable_font_axis_event(preview, cx);
        }
        true
    }

    fn apply_variable_font_axis_keyboard_step(
        &mut self,
        tag: SharedString,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.edit.property.is_some()
            || self.edit.numeric_scrub.is_some()
            || self.edit.variable_font_axis.is_some()
            || self.edit.variable_font_axis_scrub.is_some()
            || modifiers.control
            || modifiers.platform
            || modifiers.function
        {
            return false;
        }
        let Some(axis) = self
            .variable_font_axis(tag.as_ref())
            .filter(|_| self.variable_font_axis_is_editable(tag.as_ref()))
            .cloned()
        else {
            return false;
        };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        let nudge = self.preferences.nudge_settings.amount(modifiers.shift) * fine;
        let candidate = match key {
            "left" | "down" => axis.value - axis.step * nudge,
            "right" | "up" => axis.value + axis.step * nudge,
            "home" => axis.min,
            "end" => axis.max,
            _ => return false,
        };
        let value = candidate.clamp(axis.min, axis.max);
        if value == axis.value {
            return true;
        }
        let editor = VariableFontAxisEditor::new(tag, self.variable_font_axis_target(), &axis);
        let Some(begin) = self
            .edit
            .begin_variable_font_axis_edit(editor, &format_number(axis.value))
        else {
            return false;
        };
        let Some(preview) =
            self.edit
                .preview_variable_font_axis_edit(begin.tag.as_ref(), value, None)
        else {
            let _ = self.edit.finish_variable_font_axis_edit(None);
            return true;
        };
        let Some(commit) = self.edit.finish_variable_font_axis_edit(Some(value)) else {
            return true;
        };
        self.emit_variable_font_axis_event(begin, cx);
        self.emit_variable_font_axis_event(preview, cx);
        self.emit_variable_font_axis_event(commit, cx);
        true
    }

    fn handle_variable_font_axis_slider_key_down(
        &mut self,
        tag: SharedString,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.apply_variable_font_axis_keyboard_step(
            tag,
            event.keystroke.key.as_str(),
            event.keystroke.modifiers,
            cx,
        ) {
            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn start_variable_font_axis_scrub(
        &mut self,
        tag: SharedString,
        position_x: f32,
        position_y: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.edit.property.is_some()
            || self.edit.numeric_scrub.is_some()
            || self.edit.variable_font_axis.is_some()
            || self.edit.variable_font_axis_scrub.is_some()
            || !self.variable_font_axis_is_editable(tag.as_ref())
        {
            return false;
        }
        let Some(axis) = self.variable_font_axis(tag.as_ref()) else {
            return false;
        };
        self.edit.variable_font_axis_scrub = Some(VariableFontAxisScrub {
            tag,
            original: axis.value,
            scalar: axis.value,
            origin_x: position_x,
            origin_y: position_y,
            last_x: position_x,
            last_y: position_y,
            active: false,
        });
        cx.notify();
        true
    }

    fn snapped_variable_font_axis_value(
        editor: &VariableFontAxisEditor,
        value: f32,
        fine: bool,
    ) -> f32 {
        let step = editor.step * if fine { 0.1 } else { 1. };
        let snapped = editor.default + ((value - editor.default) / step).round() * step;
        snapped.clamp(editor.min, editor.max)
    }

    fn update_variable_font_axis_scrub(
        &mut self,
        tag: &str,
        position_x: f32,
        position_y: f32,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(mut scrub) = self.edit.variable_font_axis_scrub.take() else {
            return;
        };
        if scrub.tag.as_ref() != tag {
            self.edit.variable_font_axis_scrub = Some(scrub);
            return;
        }
        let speed = DesignScrubSpeed::from_vertical_displacement(position_y - scrub.origin_y);
        scrub.last_y = position_y;
        let delta = if scrub.active {
            position_x - scrub.last_x
        } else {
            let total = position_x - scrub.origin_x;
            if total.abs() < NUMERIC_SCRUB_THRESHOLD {
                self.edit.variable_font_axis_scrub = Some(scrub);
                return;
            }
            if self
                .variable_font_axis(tag)
                .is_none_or(|axis| axis.value != scrub.original)
                || !self.begin_variable_font_axis_edit(scrub.tag.clone(), cx)
            {
                return;
            }
            scrub.active = true;
            total
        };
        scrub.last_x = position_x;
        let Some(editor) = self.edit.variable_font_axis.clone() else {
            return;
        };
        let coarse = if modifiers.shift { 10. } else { 1. };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        let units_per_pixel = (editor.max - editor.min) / 160.;
        let raw = scrub.scalar + delta * units_per_pixel * coarse * fine * speed.multiplier();
        let value = Self::snapped_variable_font_axis_value(&editor, raw, modifiers.alt);
        scrub.scalar = value;
        let preview = self
            .variable_font_axis_editor_survives(&editor)
            .then(|| {
                self.edit
                    .preview_variable_font_axis_edit(editor.tag.as_ref(), value, None)
            })
            .flatten();
        self.edit.variable_font_axis_scrub = Some(scrub);
        if let Some(preview) = preview {
            self.emit_variable_font_axis_event(preview, cx);
        }
        cx.notify();
    }

    fn finish_variable_font_axis_scrub(
        &mut self,
        commit: bool,
        suppress_click: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scrub) = self.edit.variable_font_axis_scrub.take() else {
            return false;
        };
        if !scrub.active {
            cx.notify();
            return false;
        }
        let Some(editor) = self
            .edit
            .variable_font_axis
            .as_ref()
            .filter(|editor| editor.tag == scrub.tag)
            .cloned()
        else {
            cx.notify();
            return false;
        };
        let valid_commit = commit && self.variable_font_axis_editor_survives(&editor);
        let value = valid_commit.then(|| editor.last_preview.unwrap_or(editor.original));
        if let Some(event) = self.edit.finish_variable_font_axis_edit(value) {
            self.emit_variable_font_axis_event(event, cx);
        }
        if suppress_click {
            self.edit.suppress_next_control_activation = true;
            cx.defer_in(window, |this, _, _| {
                this.edit.suppress_next_control_activation = false;
            });
        }
        self.overlays.type_settings_focus().focus(window, cx);
        cx.notify();
        true
    }

    fn text_path_start_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<DesignTextPathStartData> {
        let current = self.host.inspected_node().text_path_start_data?;
        match (property, value) {
            (DesignPanelProperty::TextPathStartSegment, DesignPanelValue::Integer(segment)) => {
                Some(current.with_segment(u32::try_from(*segment).ok()?))
            }
            (DesignPanelProperty::TextPathStartSegment, DesignPanelValue::Number(segment)) => {
                Some(current.with_segment(
                    u32::try_from(round_to_integer(f64::from(*segment)).ok()? as i64).ok()?,
                ))
            }
            (DesignPanelProperty::TextPathStartPosition, DesignPanelValue::Ratio(position)) => {
                current.with_position(*position)
            }
            _ => None,
        }
    }

    fn typography_style_binding(&self) -> Option<DesignTypographyStyleBinding> {
        self.host
            .inspected_node()
            .typography
            .as_ref()
            .and_then(|typography| typography.style_binding.clone())
    }

    fn sync_typography_style_picker(&self, cx: &mut Context<Self>) {
        let binding = self.typography_style_binding();
        let disabled = !self.property_is_editable(DesignPanelProperty::TypographyStyle);
        self.typography_style_picker.update(cx, |picker, cx| {
            picker.set_current_binding(binding, cx);
            picker.set_disabled(disabled, cx);
        });
    }

    fn open_typography_style_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.inspected_node().typography.is_none()
            || self.host.inspection_context.selection().kind() == DesignPanelSelectionKind::None
        {
            return;
        }
        if !self.overlays.typography_style_picker_open() {
            self.remember_overlay_focus_return(DesignOpenOverlay::TypographyStyle, window, cx);
        }
        self.overlays.open(DesignOverlayState::TypographyStyle);
        self.cancel_menu_preview(cx);
        let view_data = self.resources.typography_styles.clone();
        let binding = self.typography_style_binding();
        let disabled = !self.property_is_editable(DesignPanelProperty::TypographyStyle);
        self.typography_style_picker.update(cx, |picker, cx| {
            picker.set_view_data(view_data, cx);
            picker.set_current_binding(binding, cx);
            picker.set_disabled(disabled, cx);
            picker.prepare_open(window, cx);
        });
        cx.notify();
    }

    fn render_typography_style_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let projection = sections::typography::style_picker::TypographyStylePickerProjection::new(
            self.id.clone(),
            self.command_target(),
            self.overlays.typography_style_picker_open(),
            self.property_is_editable(DesignPanelProperty::TypographyStyle),
            self.typography_style_picker.clone(),
        );
        let events =
            sections::typography::style_picker::TypographyStylePickerEventSink::new(cx.entity());
        sections::typography::style_picker::render(&projection, &events, cx)
    }

    fn open_font_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host.inspected_node().typography.is_none()
            || self.host.inspection_context.selection().kind() == DesignPanelSelectionKind::None
        {
            return;
        }
        if !self.overlays.font_browser_open() {
            self.remember_overlay_focus_return(DesignOpenOverlay::FontBrowser, window, cx);
        }
        self.overlays.open(DesignOverlayState::FontBrowser);
        self.cancel_menu_preview(cx);
        self.retained.inputs.font_search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    fn emit_font_apply(&mut self, font: DesignFontSelection, cx: &mut Context<Self>) {
        let applicable_weight = self
            .resources
            .fonts
            .font(&font)
            .filter(|(_, style)| style.availability.can_apply())
            .map(|(_, style)| style.weight);
        let Some(weight) = applicable_weight else {
            return;
        };
        if !self.can_edit()
            || !self.property_is_editable(DesignPanelProperty::FontFamily)
            || !self.property_is_editable(DesignPanelProperty::FontStyle)
            || (weight.is_some() && !self.property_is_editable(DesignPanelProperty::FontWeight))
        {
            return;
        }
        self.overlays.discard(DesignOpenOverlay::FontBrowser);
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyFontApplyRequested {
                node_id: self.host.inspected_node().id.clone(),
                target: self.typography_target(DesignPanelProperty::FontFamily),
                font,
            },
        );
        cx.notify();
    }

    fn emit_font_import(&mut self, font: DesignFontSelection, cx: &mut Context<Self>) {
        if !self.can_edit()
            || !self
                .resources
                .fonts
                .font(&font)
                .is_some_and(|(_, style)| style.availability.can_import())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyFontImportRequested {
                node_id: self.host.inspected_node().id.clone(),
                target: self.typography_target(DesignPanelProperty::FontFamily),
                font,
            },
        );
    }

    fn emit_open_type_feature(
        &mut self,
        tag: DesignOpenTypeFeatureTag,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let can_change = self
            .host
            .inspected_node()
            .typography
            .as_ref()
            .and_then(|typography| {
                typography
                    .open_type_features
                    .iter()
                    .find(|feature| feature.tag == tag)
            })
            .is_some_and(|feature| {
                feature.availability.is_available() && feature.enabled != enabled
            });
        if !self.can_edit() || !can_change {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyOpenTypeFeatureChangeRequested {
                node_id: self.host.inspected_node().id.clone(),
                target: self.typography_target(DesignPanelProperty::TextCase),
                tag,
                enabled,
            },
        );
    }

    fn emit_text_path_flip_orientation(&mut self, cx: &mut Context<Self>) {
        let action = DesignPanelAction::TextPathFlipOrientationRequested {
            node_id: self.host.inspected_node().id.clone(),
        };
        if !self.can_edit() || !self.node_capability_allows_action(&action) {
            return;
        }
        cx.emit_design_panel_action(self, action);
    }

    #[cfg(test)]
    fn render_font_browser(&self, cx: &mut Context<Self>) -> AnyElement {
        let query = self.retained.inputs.font_search.read(cx).value();
        let Some(projection) =
            sections::typography::font_browser::FontBrowserProjection::from_panel(
                self,
                query.as_ref(),
            )
        else {
            return div().into_any_element();
        };
        let events = sections::typography::font_browser::FontBrowserEventSink::new(cx.entity());
        let browser = sections::typography::font_browser::render(
            projection,
            self.retained.inputs.font_search.clone(),
            &events,
            cx,
        );

        h_flex()
            .w_full()
            .gap_0p5()
            .child(div().flex_1().min_w(px(0.)).child(browser))
            .when_some(
                self.render_property_variable_button(DesignPanelProperty::FontFamily, cx),
                |header, button| header.child(button),
            )
            .when_some(
                self.render_property_variable_button(DesignPanelProperty::FontStyle, cx),
                |header, button| header.child(button),
            )
            .into_any_element()
    }
    fn render_text_resize_icon(&self, icon: TextResizeIcon, cx: &mut Context<Self>) -> AnyElement {
        let color = cx.theme().foreground;
        render_lucide_icon(
            match icon {
                TextResizeIcon::AutoWidth => LucideIcon::MoveHorizontal,
                TextResizeIcon::AutoHeight => LucideIcon::Rows3,
                TextResizeIcon::FixedSize => LucideIcon::SquareDashed,
            },
            color,
            16.,
        )
    }

    fn render_text_resize_control(
        &self,
        current: DesignTextResize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editable = self.property_is_editable(DesignPanelProperty::TextResize);
        let options = [
            (DesignTextResize::AutoWidth, TextResizeIcon::AutoWidth),
            (DesignTextResize::AutoHeight, TextResizeIcon::AutoHeight),
            (DesignTextResize::Fixed, TextResizeIcon::FixedSize),
        ];
        let mut control = h_flex()
            .h(px(ROW_HEIGHT))
            .w_full()
            .overflow_hidden()
            .rounded(px(4.))
            .bg(cx.theme().secondary);
        for (resize, icon) in options {
            let selected = resize == current;
            control = control.child(
                div()
                    .id(SharedString::from(format!(
                        "{}-text-resize-{}",
                        self.id,
                        resize.label().to_lowercase().replace(' ', "-")
                    )))
                    .h_full()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(4.))
                    .when(!editable, |button| button.opacity(0.62))
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().accent)
                            .border_1()
                            .border_color(cx.theme().selection)
                    })
                    .when(editable, |button| {
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
                                    DesignPanelProperty::TextResize,
                                    DesignPanelValue::TextResize(resize),
                                    cx,
                                );
                            }))
                    })
                    .child(self.render_text_resize_icon(icon, cx)),
            );
        }
        control.into_any_element()
    }

    fn render_type_setting_segments(
        panel_id: SharedString,
        id_suffix: &'static str,
        options: Vec<(SharedString, bool, DesignPanelValue)>,
        enabled: bool,
        property: DesignPanelProperty,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let events = sections::typography::type_settings::TypeSettingsEventSink::new(panel);
        sections::typography::type_settings::render_segments(
            panel_id, id_suffix, options, enabled, property, &events, cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_type_setting_number_field(
        panel_id: SharedString,
        id_suffix: &'static str,
        label: SharedString,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        enabled: bool,
        editing: bool,
        invalid: bool,
        input: Entity<InputState>,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let events = sections::typography::type_settings::TypeSettingsEventSink::new(panel);
        sections::typography::type_settings::render_number_field(
            panel_id, id_suffix, label, property, value, enabled, editing, invalid, input, &events,
            cx,
        )
    }
    fn render_variable_font_axis_number_field(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let tag = axis.tag.clone();
        let (
            editable,
            editing,
            scrubbing,
            invalid,
            displayed_value,
            disabled_reason,
            property_input,
        ) = {
            let panel = panel.read(cx);
            let editing = panel
                .edit
                .variable_font_axis
                .as_ref()
                .is_some_and(|editor| editor.tag == tag);
            let scrubbing = panel
                .edit
                .variable_font_axis_scrub
                .as_ref()
                .is_some_and(|scrub| scrub.tag == tag && scrub.active);
            let displayed_value = panel
                .edit
                .variable_font_axis
                .as_ref()
                .filter(|editor| editor.tag == tag)
                .map_or(axis.value, |editor| {
                    editor.last_preview.unwrap_or(editor.original)
                });
            let editable = panel.variable_font_axis_is_editable(tag.as_ref());
            let disabled_reason = if editable {
                None
            } else {
                axis.disabled_reason().or_else(|| {
                    if !panel.can_edit() {
                        Some("View-only access".into())
                    } else {
                        Some("Axis tags must be unique in the host snapshot".into())
                    }
                })
            };
            (
                editable,
                editing,
                scrubbing,
                panel.edit.variable_font_axis_invalid,
                displayed_value,
                disabled_reason,
                panel.retained.inputs.property.clone(),
            )
        };
        let id = SharedString::from(format!(
            "{panel_id}-variable-font-axis-{index}-{}-value",
            axis.tag
        ));
        if editing && !scrubbing {
            return div()
                .id(id)
                .h(px(28.))
                .w(px(76.))
                .rounded(px(4.))
                .border_1()
                .border_color(if invalid {
                    cx.theme().red
                } else {
                    cx.theme().selection
                })
                .bg(cx.theme().secondary)
                .child(
                    Input::new(&property_input)
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false)
                        .xsmall()
                        .h(px(26.))
                        .w_full(),
                )
                .into_any_element();
        }

        let panel_for_activate = panel.clone();
        let activate_tag = tag.clone();
        let tooltip = disabled_reason.unwrap_or_else(|| {
            format!(
                "Edit {} ({}) · {} to {}",
                axis.name,
                axis.tag,
                format_number(axis.min),
                format_number(axis.max)
            )
            .into()
        });
        let button = Button::new(SharedString::from(format!("{id}-activate")))
            .label(format_number(displayed_value))
            .xsmall()
            .compact()
            .w(px(76.))
            .h(px(28.))
            .disabled(!editable)
            .tooltip(tooltip)
            .on_activate(move |_, window, cx| {
                let tag = activate_tag.clone();
                panel_for_activate.update(cx, |this, cx| {
                    this.activate_variable_font_axis_input(tag, window, cx);
                });
            })
            .into_any_element();
        render_variable_font_axis_scrub_surface(
            SharedString::from(format!("{id}-scrub")),
            panel,
            tag,
            editable,
            button,
        )
    }

    fn render_variable_font_axis_slider(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let tag = axis.tag.clone();
        let (editable, displayed_value, disabled_reason) = {
            let panel = panel.read(cx);
            let editable = panel.variable_font_axis_is_editable(tag.as_ref());
            let displayed_value = panel
                .edit
                .variable_font_axis
                .as_ref()
                .filter(|editor| editor.tag == tag)
                .map_or(axis.value, |editor| {
                    editor.last_preview.unwrap_or(editor.original)
                });
            let reason = (!editable).then(|| {
                axis.disabled_reason().unwrap_or_else(|| {
                    if !panel.can_edit() {
                        "View-only access".into()
                    } else {
                        "Axis tags must be unique in the host snapshot".into()
                    }
                })
            });
            (editable, displayed_value, reason)
        };
        let percentage = if axis.has_valid_range() {
            ((displayed_value - axis.min) / (axis.max - axis.min)).clamp(0., 1.)
        } else {
            0.
        };
        let id = SharedString::from(format!(
            "{panel_id}-variable-font-axis-{index}-{}-slider",
            axis.tag
        ));
        let tooltip = disabled_reason.unwrap_or_else(|| {
            format!(
                "{} ({}) slider · {} · range {} to {} · default {}",
                axis.name,
                axis.tag,
                format_number(displayed_value),
                format_number(axis.min),
                format_number(axis.max),
                format_number(axis.default)
            )
            .into()
        });
        let panel_for_key = panel.clone();
        let key_tag = tag.clone();
        let slider = div()
            .id(id.clone())
            .h(px(28.))
            .w_full()
            .flex()
            .items_center()
            .px_1()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .debug_selector(move || tooltip.to_string())
            .when(editable, |slider| {
                slider
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .focus(|style| style.border_color(cx.theme().selection))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let tag = key_tag.clone();
                        panel_for_key.update(cx, |this, cx| {
                            this.handle_variable_font_axis_slider_key_down(tag, event, window, cx);
                        });
                    })
            })
            .when(!editable, |slider| slider.opacity(0.62))
            .child(
                div()
                    .relative()
                    .h(px(4.))
                    .w_full()
                    .rounded_full()
                    .bg(cx.theme().secondary)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .h_full()
                            .w(relative(percentage))
                            .rounded_full()
                            .bg(cx.theme().selection),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(percentage))
                            .top(px(-4.))
                            .ml(px(-6.))
                            .size(px(12.))
                            .rounded_full()
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().foreground),
                    ),
            )
            .into_any_element();
        render_variable_font_axis_scrub_surface(
            SharedString::from(format!("{id}-scrub")),
            panel,
            tag,
            editable,
            slider,
        )
    }

    fn render_variable_font_axis_row(
        panel_id: SharedString,
        index: usize,
        axis: DesignFontAxis,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let binding_summary = axis.binding.as_ref().map(|binding| {
            binding.collection_name.as_ref().map_or_else(
                || format!("Variable · {}", binding.variable_name),
                |collection| format!("{collection} / {}", binding.variable_name),
            )
        });
        let disabled_reason = axis.disabled_reason();
        v_flex()
            .id(SharedString::from(format!(
                "{panel_id}-variable-font-axis-{index}-{}",
                axis.tag
            )))
            .w_full()
            .gap_1()
            .py_1()
            .child(
                h_flex()
                    .h(px(22.))
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .child(axis.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(axis.tag.clone()),
                    ),
            )
            .child(
                h_flex()
                    .h(px(28.))
                    .w_full()
                    .gap_2()
                    .child(div().flex_1().min_w(px(0.)).child(
                        Self::render_variable_font_axis_slider(
                            panel_id.clone(),
                            index,
                            axis.clone(),
                            panel.clone(),
                            cx,
                        ),
                    ))
                    .child(Self::render_variable_font_axis_number_field(
                        panel_id,
                        index,
                        axis.clone(),
                        panel,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .text_xs()
                    .text_color(muted)
                    .child(format_number(axis.min))
                    .child(format!("Default {}", format_number(axis.default)))
                    .child(format_number(axis.max)),
            )
            .when_some(binding_summary, |row, summary| {
                row.child(div().text_xs().text_color(muted).child(summary))
            })
            .when_some(disabled_reason, |row, reason| {
                row.child(div().text_xs().text_color(muted).child(reason))
            })
            .into_any_element()
    }

    #[cfg(test)]
    fn render_type_settings_popover(
        &self,
        typography: &super::super::DesignTypography,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let overlay =
            sections::typography::type_settings::TypeSettingsOverlayProjection::from_panel(
                self,
                !typography.variable_axes.is_empty(),
            );
        self.render_type_settings_popover_projected(typography, overlay, cx)
    }

    fn render_type_settings_popover_projected(
        &self,
        typography: &super::super::DesignTypography,
        overlay: sections::typography::type_settings::TypeSettingsOverlayProjection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let type_settings_events =
            sections::typography::type_settings::TypeSettingsEventSink::new(panel.clone());
        let panel_for_open = panel.clone();
        let axes = typography.variable_axes.clone();
        let has_variable_axes = !axes.is_empty();
        let trigger =
            sections::typography::type_settings::render_trigger(&overlay, &type_settings_events);
        let open = overlay.open;
        let tab = overlay.tab;
        let focus = overlay.focus.clone();
        let focus_for_open = focus.clone();
        let focus_for_content = focus.clone();
        let target_for_open = overlay.target.clone();
        let target_for_content = overlay.target.clone();
        let panel_id = overlay.panel_id.clone();
        let paragraph_spacing = typography.paragraph_spacing;
        let horizontal_alignment = typography.horizontal_alignment;
        let case = typography.case;
        let decoration = typography.decoration;
        let leading_trim = typography.leading_trim;
        let truncate = typography.truncate;
        let max_lines = typography.max_lines;
        let list = typography.list;
        let list_spacing = typography.list_spacing;
        let paragraph_indent = typography.paragraph_indent;
        let hanging_punctuation = typography.hanging_punctuation;
        let hanging_lists = typography.hanging_lists;
        let decoration_details = typography.decoration_details;
        let open_type_features = typography.open_type_features.clone();
        let alignment_editable =
            self.property_is_editable(DesignPanelProperty::HorizontalTextAlignment);
        let case_editable = self.property_is_editable(DesignPanelProperty::TextCase);
        let decoration_editable = self.property_is_editable(DesignPanelProperty::TextDecoration);
        let leading_trim_editable = self.property_is_editable(DesignPanelProperty::TextLeadingTrim);
        let paragraph_spacing_editable =
            self.property_is_editable(DesignPanelProperty::ParagraphSpacing);
        let list_editable = self.property_is_editable(DesignPanelProperty::TextList);
        let list_spacing_editable = self.property_is_editable(DesignPanelProperty::ListSpacing);
        let truncate_editable = self.property_is_editable(DesignPanelProperty::TextTruncate);
        let max_lines_available = self.text_max_lines_are_available();
        let max_lines_editable = self.property_is_editable(DesignPanelProperty::TextMaxLines);
        let paragraph_indent_editable =
            self.property_is_editable(DesignPanelProperty::ParagraphIndent);
        let hanging_punctuation_editable =
            self.property_is_editable(DesignPanelProperty::TextHangingPunctuation);
        let hanging_lists_editable =
            self.property_is_editable(DesignPanelProperty::TextHangingLists);
        let decoration_style_editable =
            self.property_is_editable(DesignPanelProperty::TextDecorationStyle);
        let decoration_offset_editable =
            self.property_is_editable(DesignPanelProperty::TextDecorationOffset);
        let decoration_thickness_editable =
            self.property_is_editable(DesignPanelProperty::TextDecorationThickness);
        let decoration_color_editable =
            self.property_is_editable(DesignPanelProperty::TextDecorationColor);
        let decoration_color_picker_target = AuxiliaryColorPickerTarget::TextDecoration {
            node_id: self.host.inspected_node().id.clone(),
            typography_target: self.typography_target(DesignPanelProperty::TextDecorationColor),
        };
        let decoration_skip_ink_editable =
            self.property_is_editable(DesignPanelProperty::TextDecorationSkipInk);
        let paragraph_indent_label = self.display_property_value(
            DesignPanelProperty::ParagraphIndent,
            format_number(paragraph_indent).into(),
        );
        let paragraph_spacing_label = self.display_property_value(
            DesignPanelProperty::ParagraphSpacing,
            format_number(paragraph_spacing).into(),
        );
        let list_spacing_label = self.display_property_value(
            DesignPanelProperty::ListSpacing,
            format_number(list_spacing).into(),
        );
        let max_lines_label = self.display_property_value(
            DesignPanelProperty::TextMaxLines,
            format_text_max_lines(max_lines).into(),
        );
        let paragraph_indent_editing = self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ParagraphIndent);
        let paragraph_spacing_editing = self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ParagraphSpacing);
        let list_spacing_editing = self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ListSpacing);
        let max_lines_editing = self
            .edit
            .property
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::TextMaxLines);
        let property_input = self.retained.inputs.property.clone();
        let property_editor_invalid = self.edit.property_invalid;
        let can_edit_typography = self.can_edit();
        let paragraph_spacing_variable_button_state =
            self.property_variable_button_state(DesignPanelProperty::ParagraphSpacing);
        let paragraph_indent_variable_button_state =
            self.property_variable_button_state(DesignPanelProperty::ParagraphIndent);

        Popover::new(SharedString::from(format!(
            "{panel_id}-type-settings-popover"
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .track_focus(&focus)
        .on_open_change(move |open, window, cx| {
            let events = sections::typography::type_settings::TypeSettingsEventSink::new(
                panel_for_open.clone(),
            );
            events.send_overlay(
                &target_for_open,
                sections::typography::type_settings::TypeSettingsOverlayEvent::OpenChanged(*open),
                &focus_for_open,
                window,
                cx,
            );
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let muted_foreground = cx.theme().muted_foreground;
            let layout = sections::typography::type_settings::grid_layout();
            let metrics = layout.metrics;
            let tab_button = |candidate: TypographySettingsTab| {
                sections::typography::type_settings::render_tab(
                    &panel_id,
                    tab,
                    candidate,
                    target_for_content.clone(),
                    &type_settings_events,
                    focus_for_content.clone(),
                )
            };
            let control_row = |label: &'static str, control: AnyElement| {
                crate::molecules::inspector_row_with_layout(layout)
                    .h(layout.row_height())
                    .w_full()
                    .child(
                        crate::molecules::inspector_field_label(layout)
                            .text_color(muted_foreground)
                            .child(label),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(control))
            };
            let mut tabs = crate::molecules::inspector_action_group(metrics)
                .flex_1()
                .gap_1()
                .child(tab_button(TypographySettingsTab::Basics))
                .child(tab_button(TypographySettingsTab::Details));
            if has_variable_axes {
                tabs = tabs.child(tab_button(TypographySettingsTab::Variable));
            }

            let mut body = crate::molecules::inspector_field_grid_with_layout(layout).px(px(0.));
            match tab {
                TypographySettingsTab::Basics => {
                    let alignment_options = DesignTextHorizontalAlignment::ALL
                        .into_iter()
                        .zip(["L", "C", "R", "J"])
                        .map(|(alignment, label)| {
                            (
                                SharedString::from(label),
                                alignment == horizontal_alignment,
                                DesignPanelValue::TextHorizontalAlignment(alignment),
                            )
                        })
                        .collect();
                    let decoration_options = DesignTextDecoration::ALL
                        .into_iter()
                        .zip(["None", "Underline", "Strike"])
                        .map(|(candidate, label)| {
                            (
                                SharedString::from(label),
                                candidate == decoration,
                                DesignPanelValue::TextDecoration(candidate),
                            )
                        })
                        .collect();
                    let case_options = [
                        DesignTextCase::Original,
                        DesignTextCase::Uppercase,
                        DesignTextCase::Lowercase,
                        DesignTextCase::TitleCase,
                        DesignTextCase::SmallCaps,
                    ]
                    .into_iter()
                    .zip(["None", "AG", "ag", "Ag", "Aɢ"])
                    .map(|(candidate, label)| {
                        (
                            SharedString::from(label),
                            candidate == case,
                            DesignPanelValue::TextCase(candidate),
                        )
                    })
                    .collect();
                    let list_options = DesignTextList::ALL
                        .into_iter()
                        .zip(["None", "Bullets", "Numbers"])
                        .map(|(candidate, label)| {
                            (
                                SharedString::from(label),
                                candidate == list,
                                DesignPanelValue::TextList(candidate),
                            )
                        })
                        .collect();
                    let leading_trim_options = DesignTextLeadingTrim::ALL
                        .into_iter()
                        .zip(["A̅g̲", "Ag"])
                        .map(|(candidate, label)| {
                            (
                                SharedString::from(label),
                                candidate == leading_trim,
                                DesignPanelValue::TextLeadingTrim(candidate),
                            )
                        })
                        .collect();
                    let truncate_options = [
                        (
                            SharedString::from("None"),
                            !truncate,
                            DesignPanelValue::Bool(false),
                        ),
                        (
                            SharedString::from("A…"),
                            truncate,
                            DesignPanelValue::Bool(true),
                        ),
                    ]
                    .into_iter()
                    .collect();
                    body = body
                        .child(
                            v_flex()
                                .h(px(184.))
                                .w_full()
                                .px_10()
                                .items_start()
                                .justify_center()
                                .gap_2()
                                .rounded(px(5.))
                                .bg(cx.theme().secondary)
                                .child(div().text_lg().child("The quick brown"))
                                .child(div().text_lg().child("fox jumps over"))
                                .child(div().text_lg().child("17 lazy dogs.")),
                        )
                        .child(control_row(
                            "Alignment",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "alignment",
                                alignment_options,
                                alignment_editable,
                                DesignPanelProperty::HorizontalTextAlignment,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(control_row(
                            "Decoration",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "decoration",
                                decoration_options,
                                decoration_editable,
                                DesignPanelProperty::TextDecoration,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(control_row(
                            "Case",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "case",
                                case_options,
                                case_editable,
                                DesignPanelProperty::TextCase,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(div().h(px(1.)).w_full().bg(cx.theme().border))
                        .child(control_row(
                            "Vertical trim",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "vertical-trim",
                                leading_trim_options,
                                leading_trim_editable,
                                DesignPanelProperty::TextLeadingTrim,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(control_row(
                            "List style",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "list",
                                list_options,
                                list_editable,
                                DesignPanelProperty::TextList,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .when(list != DesignTextList::None, |body| {
                            body.child(control_row(
                                "List spacing",
                                Self::render_type_setting_number_field(
                                    panel_id.clone(),
                                    "list-spacing",
                                    list_spacing_label.clone(),
                                    DesignPanelProperty::ListSpacing,
                                    DesignPanelValue::Number(list_spacing),
                                    list_spacing_editable,
                                    list_spacing_editing,
                                    property_editor_invalid,
                                    property_input.clone(),
                                    panel.clone(),
                                    cx,
                                ),
                            ))
                        })
                        .child(control_row(
                            "Paragraph spacing",
                            h_flex()
                                .w_full()
                                .gap_1()
                                .child(div().flex_1().min_w(px(0.)).child(
                                    Self::render_type_setting_number_field(
                                        panel_id.clone(),
                                        "paragraph-spacing",
                                        paragraph_spacing_label.clone(),
                                        DesignPanelProperty::ParagraphSpacing,
                                        DesignPanelValue::Number(paragraph_spacing),
                                        paragraph_spacing_editable,
                                        paragraph_spacing_editing,
                                        property_editor_invalid,
                                        property_input.clone(),
                                        panel.clone(),
                                        cx,
                                    ),
                                ))
                                .when_some(
                                    paragraph_spacing_variable_button_state.clone().and_then(
                                        |button_state| {
                                            Self::render_property_variable_button_for(
                                                panel.clone(),
                                                DesignPanelProperty::ParagraphSpacing,
                                                button_state,
                                                cx,
                                            )
                                        },
                                    ),
                                    |row, button| row.child(button),
                                )
                                .into_any_element(),
                        ))
                        .child(control_row(
                            "Truncate text",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "truncate",
                                truncate_options,
                                truncate_editable,
                                DesignPanelProperty::TextTruncate,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .when(max_lines_available, |body| {
                            body.child(control_row(
                                "Max lines",
                                Self::render_type_setting_number_field(
                                    panel_id.clone(),
                                    "max-lines",
                                    max_lines_label.clone(),
                                    DesignPanelProperty::TextMaxLines,
                                    DesignPanelValue::OptionalNumber(
                                        max_lines.map(|lines| lines as f32),
                                    ),
                                    max_lines_editable,
                                    max_lines_editing,
                                    property_editor_invalid,
                                    property_input.clone(),
                                    panel.clone(),
                                    cx,
                                ),
                            ))
                        });
                }
                TypographySettingsTab::Details => {
                    let bool_options = |enabled: bool| {
                        vec![
                            (
                                SharedString::from("Off"),
                                !enabled,
                                DesignPanelValue::Bool(false),
                            ),
                            (
                                SharedString::from("On"),
                                enabled,
                                DesignPanelValue::Bool(true),
                            ),
                        ]
                    };
                    let case_options = DesignTextCase::ALL
                        .into_iter()
                        .zip(["None", "AG", "ag", "Ag", "Aɢ", "Aɢ⁺"])
                        .map(|(candidate, label)| {
                            (
                                SharedString::from(label),
                                candidate == case,
                                DesignPanelValue::TextCase(candidate),
                            )
                        })
                        .collect();
                    let decoration_style_options = DesignTextDecorationStyle::ALL
                        .into_iter()
                        .map(|candidate| {
                            (
                                SharedString::from(candidate.label()),
                                decoration_details
                                    .is_some_and(|details| details.style == candidate),
                                DesignPanelValue::TextDecorationStyle(candidate),
                            )
                        })
                        .collect();
                    let metric_options =
                        |current: DesignTextDecorationMetric| -> Vec<(
                            SharedString,
                            bool,
                            DesignPanelValue,
                        )> {
                            let pixels = match current {
                                DesignTextDecorationMetric::Pixels(value) => value,
                                _ => 1.,
                            };
                            let percent = match current {
                                DesignTextDecorationMetric::Percent(value) => value,
                                _ => 100.,
                            };
                            [
                                (
                                    SharedString::from("Auto"),
                                    DesignTextDecorationMetric::Auto,
                                ),
                                (
                                    SharedString::from(format!("{}px", format_number(pixels))),
                                    DesignTextDecorationMetric::Pixels(pixels),
                                ),
                                (
                                    SharedString::from(format!("{}%", format_number(percent))),
                                    DesignTextDecorationMetric::Percent(percent),
                                ),
                            ]
                            .into_iter()
                            .map(|(label, candidate)| {
                                (
                                    label,
                                    candidate == current,
                                    DesignPanelValue::TextDecorationMetric(candidate),
                                )
                            })
                            .collect()
                        };
                    let color_options = decoration_details
                        .map(|details| {
                            vec![(
                                SharedString::from("Auto"),
                                details.color == DesignTextDecorationColor::Auto,
                                DesignPanelValue::TextDecorationColor(
                                    DesignTextDecorationColor::Auto,
                                ),
                            )]
                        })
                        .unwrap_or_default();
                    body = body
                        .child(
                            v_flex()
                                .h(px(184.))
                                .w_full()
                                .px_10()
                                .items_start()
                                .justify_center()
                                .gap_2()
                                .rounded(px(5.))
                                .bg(cx.theme().secondary)
                                .child(div().text_lg().child("The quick brown"))
                                .child(div().text_lg().child("fox jumps over"))
                                .child(div().text_lg().child("17 lazy dogs.")),
                        )
                        .child(div().pt_1().text_sm().font_semibold().child("Indentation"))
                        .child(control_row(
                            "Hanging punctuation",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "hanging-punctuation",
                                bool_options(hanging_punctuation),
                                hanging_punctuation_editable,
                                DesignPanelProperty::TextHangingPunctuation,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(control_row(
                            "Hanging lists",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "hanging-lists",
                                bool_options(hanging_lists),
                                hanging_lists_editable,
                                DesignPanelProperty::TextHangingLists,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .when(
                            horizontal_alignment == DesignTextHorizontalAlignment::Left,
                            |body| {
                                body.child(control_row(
                                    "Paragraph indent",
                                    h_flex()
                                        .w_full()
                                        .gap_1()
                                        .child(div().flex_1().min_w(px(0.)).child(
                                            Self::render_type_setting_number_field(
                                                panel_id.clone(),
                                                "paragraph-indent",
                                                paragraph_indent_label.clone(),
                                                DesignPanelProperty::ParagraphIndent,
                                                DesignPanelValue::Number(paragraph_indent),
                                                paragraph_indent_editable,
                                                paragraph_indent_editing,
                                                property_editor_invalid,
                                                property_input.clone(),
                                                panel.clone(),
                                                cx,
                                            ),
                                        ))
                                        .when_some(
                                            paragraph_indent_variable_button_state
                                                .clone()
                                                .and_then(|button_state| {
                                                    Self::render_property_variable_button_for(
                                                        panel.clone(),
                                                        DesignPanelProperty::ParagraphIndent,
                                                        button_state,
                                                        cx,
                                                    )
                                                }),
                                            |row, button| row.child(button),
                                        )
                                        .into_any_element(),
                                ))
                            },
                        )
                        .child(div().pt_1().text_sm().font_semibold().child("Letter case"))
                        .child(control_row(
                            "Case",
                            Self::render_type_setting_segments(
                                panel_id.clone(),
                                "details-case",
                                case_options,
                                case_editable,
                                DesignPanelProperty::TextCase,
                                panel.clone(),
                                cx,
                            ),
                        ))
                        .child(
                            div()
                                .pt_1()
                                .text_sm()
                                .font_semibold()
                                .child("Decoration details"),
                        );
                    if let Some(details) = decoration_details {
                        body = body
                            .child(control_row(
                                "Style",
                                Self::render_type_setting_segments(
                                    panel_id.clone(),
                                    "decoration-style",
                                    decoration_style_options,
                                    decoration_style_editable,
                                    DesignPanelProperty::TextDecorationStyle,
                                    panel.clone(),
                                    cx,
                                ),
                            ))
                            .child(control_row(
                                "Offset",
                                Self::render_type_setting_segments(
                                    panel_id.clone(),
                                    "decoration-offset",
                                    metric_options(details.offset),
                                    decoration_offset_editable,
                                    DesignPanelProperty::TextDecorationOffset,
                                    panel.clone(),
                                    cx,
                                ),
                            ))
                            .child(control_row(
                                "Thickness",
                                Self::render_type_setting_segments(
                                    panel_id.clone(),
                                    "decoration-thickness",
                                    metric_options(details.thickness),
                                    decoration_thickness_editable,
                                    DesignPanelProperty::TextDecorationThickness,
                                    panel.clone(),
                                    cx,
                                ),
                            ))
                            .child(control_row(
                                "Color",
                                h_flex()
                                    .w_full()
                                    .gap_1()
                                    .child(div().flex_1().min_w(px(0.)).child(
                                        Self::render_type_setting_segments(
                                            panel_id.clone(),
                                            "decoration-color-auto",
                                            color_options,
                                            decoration_color_editable,
                                            DesignPanelProperty::TextDecorationColor,
                                            panel.clone(),
                                            cx,
                                        ),
                                    ))
                                    .child(div().flex_1().min_w(px(0.)).child(panel.update(
                                        cx,
                                        |this, cx| {
                                            let color = match details.color {
                                                DesignTextDecorationColor::Auto => {
                                                    DesignColor::BLACK
                                                }
                                                DesignTextDecorationColor::Solid(color) => color,
                                            };
                                            this.render_auxiliary_color_picker_control(
                                                cx.entity(),
                                                "decoration-color-solid",
                                                format!("#{}", color.hex()),
                                                color,
                                                decoration_color_picker_target.clone(),
                                                cx,
                                            )
                                        },
                                    )))
                                    .into_any_element(),
                            ))
                            .child(control_row(
                                "Skip ink",
                                Self::render_type_setting_segments(
                                    panel_id.clone(),
                                    "decoration-skip-ink",
                                    bool_options(details.skip_ink),
                                    decoration_skip_ink_editable,
                                    DesignPanelProperty::TextDecorationSkipInk,
                                    panel.clone(),
                                    cx,
                                ),
                            ));
                    } else {
                        body = body.child(
                            div()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child("Select underline or strikethrough to edit its details."),
                        );
                    }

                    body = body.child(
                        div()
                            .pt_1()
                            .text_sm()
                            .font_semibold()
                            .child("OpenType features"),
                    );
                    if open_type_features.is_empty() {
                        body = body.child(
                            div()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child("The selected font exposes no feature records."),
                        );
                    }
                    for (index, feature) in open_type_features.clone().into_iter().enumerate() {
                        let available = feature.availability.is_available();
                        let detail: SharedString = match &feature.availability {
                            super::super::DesignOpenTypeFeatureAvailability::Available => {
                                let default = if feature.default_enabled {
                                    "default on"
                                } else {
                                    "default off"
                                };
                                feature.preview.as_ref().map_or_else(
                                    || default.into(),
                                    |preview| format!("{default} · {preview}").into(),
                                )
                            }
                            super::super::DesignOpenTypeFeatureAvailability::Unavailable {
                                reason,
                            } => format!("Unavailable · {reason}").into(),
                        };
                        let feature_name: SharedString =
                            format!("{} ({})", feature.name, feature.tag.api_name()).into();
                        let panel_for_off = panel.clone();
                        let panel_for_on = panel.clone();
                        let off_tag = feature.tag.clone();
                        let on_tag = feature.tag.clone();
                        let controls = h_flex()
                            .h(layout.row_height())
                            .w_full()
                            .overflow_hidden()
                            .rounded(px(4.))
                            .bg(cx.theme().secondary)
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{panel_id}-feature-{index}-off"
                                )))
                                .label("Off")
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .selected(!feature.enabled)
                                .disabled(!available || !can_edit_typography)
                                .on_activate(move |_, _, cx| {
                                    let tag = off_tag.clone();
                                    panel_for_off.update(cx, |this, cx| {
                                        this.emit_open_type_feature(tag, false, cx);
                                    });
                                }),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{panel_id}-feature-{index}-on"
                                )))
                                .label("On")
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .selected(feature.enabled)
                                .disabled(!available || !can_edit_typography)
                                .on_activate(move |_, _, cx| {
                                    let tag = on_tag.clone();
                                    panel_for_on.update(cx, |this, cx| {
                                        this.emit_open_type_feature(tag, true, cx);
                                    });
                                }),
                            );
                        body = body.child(
                            v_flex()
                                .w_full()
                                .gap_1()
                                .child(
                                    crate::molecules::inspector_row_with_layout(layout)
                                        .h(layout.row_height())
                                        .w_full()
                                        .child(
                                            crate::molecules::inspector_field_label(layout)
                                                .truncate()
                                                .text_color(muted_foreground)
                                                .child(feature_name),
                                        )
                                        .child(div().flex_1().min_w(px(0.)).child(controls)),
                                )
                                .child(
                                    div()
                                        .pl(px(120.))
                                        .text_xs()
                                        .text_color(muted_foreground)
                                        .child(detail),
                                ),
                        );
                    }
                }
                TypographySettingsTab::Variable => {
                    body = body
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_foreground)
                                .child("Arrow keys adjust by the host step; Shift ×10, Alt ×0.1."),
                        )
                        .child(div().h(px(1.)).w_full().bg(cx.theme().border));
                    for (index, axis) in axes.iter().cloned().enumerate() {
                        body = body.child(Self::render_variable_font_axis_row(
                            panel_id.clone(),
                            index,
                            axis,
                            panel.clone(),
                            cx,
                        ));
                    }
                }
            }

            let close_popover = popover.clone();
            crate::molecules::inspector_popover_surface("type-settings-content", metrics, cx)
                .key_context(DESIGN_PANEL_KEY_CONTEXT)
                .track_focus(&focus_for_content)
                .tab_index(0)
                .w(popup_width(window, 360.))
                .max_h(popup_height(window, 680.))
                .p_3()
                .gap_2()
                .rounded(px(8.))
                .overflow_hidden()
                .child(
                    h_flex().h(layout.row_height()).gap_1().child(tabs).child(
                        Button::new(SharedString::from(format!(
                            "{}-type-settings-close",
                            panel_id
                        )))
                        .xsmall()
                        .compact()
                        .ghost()
                        .w(layout.row_height())
                        .h(layout.row_height())
                        .child(Icon::new(IconName::Close).xsmall())
                        .on_activate(move |_, window, cx| {
                            close_popover.update(cx, |popover, cx| {
                                popover.dismiss(window, cx);
                            });
                        }),
                    ),
                )
                .child(div().h(px(1.)).w_full().bg(cx.theme().border))
                .child(
                    div()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .pr_1()
                        .child(body),
                )
        })
        .into_any_element()
    }

    fn render_typography(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = projection(self, cx)?;
        let controls = sections::typography::TypographySupplementaryControls::new(
            self.render_property_variable_button(DesignPanelProperty::FontFamily, cx),
            self.render_property_variable_button(DesignPanelProperty::FontStyle, cx),
            self.render_applied_component_property_controls(
                DesignComponentPropertyApplicationSurface::Text,
                cx,
            ),
        );
        let events = sections::typography::TypographyEventSink::new(cx.entity());
        Some(sections::typography::render(
            &projection,
            self,
            controls,
            &events,
            cx,
        ))
    }
}

fn render_variable_font_axis_scrub_surface(
    id: SharedString,
    panel: Entity<DesignPanel>,
    tag: SharedString,
    enabled: bool,
    content: AnyElement,
) -> AnyElement {
    let mut surface = div()
        .id(id)
        .h_full()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .child(content);
    if enabled {
        let panel_for_down = panel.clone();
        let panel_for_move = panel.clone();
        let panel_for_up = panel.clone();
        let down_tag = tag.clone();
        let move_tag = tag.clone();
        surface = surface
            .cursor_col_resize()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                let tag = down_tag.clone();
                panel_for_down.update(cx, |this, cx| {
                    this.start_variable_font_axis_scrub(
                        tag,
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        cx,
                    );
                });
            })
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    panel_for_move.update(cx, |this, cx| {
                        this.update_variable_font_axis_scrub(
                            move_tag.as_ref(),
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                            event.modifiers,
                            cx,
                        );
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                panel_for_up.update(cx, |this, cx| {
                    this.finish_variable_font_axis_scrub(true, true, window, cx);
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                panel.update(cx, |this, cx| {
                    this.finish_variable_font_axis_scrub(true, false, window, cx);
                });
            });
    }
    surface.into_any_element()
}

use super::*;

impl DesignPanel {
    pub(super) fn text_path_flip_is_available(&self) -> bool {
        self.node.kind == DesignPanelNodeKind::TextPath
            && self.node.supports_section(DesignPanelSection::Typography)
            && self
                .node
                .text_path
                .as_ref()
                .is_some_and(|text_path| text_path.can_flip_orientation)
    }

    pub(super) fn text_path_start_debug_controls_are_available(&self) -> bool {
        self.node.kind == DesignPanelNodeKind::TextPath
            && self.node.supports_section(DesignPanelSection::Typography)
            && self.node.text_path_start_data.is_some()
            && self
                .node
                .text_path
                .as_ref()
                .is_some_and(|text_path| text_path.show_start_data_debug_controls)
    }

    /// Mirrors Figma's contextual Max lines disclosure. The row exists only
    /// for ending-truncated Auto width/Auto height text, and an auto-layout
    /// child additionally has to use vertical Hug sizing.
    pub(super) fn text_max_lines_are_available(&self) -> bool {
        self.node
            .text_max_lines_are_available(self.inspection_context.parent_layout().is_auto_layout())
    }

    pub(super) fn typography_target(
        &self,
        property: DesignPanelProperty,
    ) -> DesignTypographyTarget {
        if self.inspection_context.edit_mode() == DesignPanelEditMode::Text
            && property.supports_selected_text_range()
        {
            self.inspection_context.text_range_revision().map_or(
                DesignTypographyTarget::SelectedTextRange,
                DesignTypographyTarget::SelectedTextRangeRevision,
            )
        } else {
            DesignTypographyTarget::WholeLayer
        }
    }

    pub(super) fn variable_font_axis(&self, tag: &str) -> Option<&DesignFontAxis> {
        let axes = &self.node.typography.as_ref()?.variable_axes;
        let mut matching = axes.iter().filter(|axis| axis.tag.as_ref() == tag);
        let axis = matching.next()?;
        matching.next().is_none().then_some(axis)
    }

    pub(super) fn variable_font_axis_target(&self) -> DesignTypographyTarget {
        self.typography_target(DesignPanelProperty::FontWeight)
    }

    pub(super) fn variable_font_axis_is_editable(&self, tag: &str) -> bool {
        self.can_edit()
            && self
                .node
                .typography
                .as_ref()
                .is_some_and(|typography| typography.style_binding.is_none())
            && self
                .variable_font_axis(tag)
                .is_some_and(DesignFontAxis::is_editable)
    }

    pub(super) fn variable_font_axis_editor_survives(
        &self,
        editor: &VariableFontAxisEditor,
    ) -> bool {
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

    pub(super) fn emit_variable_font_axis_action(
        &self,
        editor: &VariableFontAxisEditor,
        value: f32,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyVariableAxisEditRequested {
                node_id: self.node.id.clone(),
                target: editor.target,
                tag: editor.tag.clone(),
                value,
                phase,
            },
        );
    }

    pub(super) fn begin_variable_font_axis_edit(
        &mut self,
        tag: SharedString,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.property_editor.is_some()
            || self.numeric_property_scrub.is_some()
            || self.variable_font_axis_editor.is_some()
            || !self.variable_font_axis_is_editable(tag.as_ref())
        {
            return false;
        }
        let Some(axis) = self.variable_font_axis(tag.as_ref()).cloned() else {
            return false;
        };
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.active_effect_settings = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        let editor = VariableFontAxisEditor {
            tag,
            target: self.variable_font_axis_target(),
            original: axis.value,
            last_preview: None,
            min: axis.min,
            max: axis.max,
            default: axis.default,
            step: axis.step,
        };
        self.emit_variable_font_axis_action(
            &editor,
            editor.original,
            DesignPanelEditPhase::Begin,
            cx,
        );
        self.variable_font_axis_editor = Some(editor);
        self.variable_font_axis_editor_invalid = false;
        true
    }

    pub(super) fn activate_variable_font_axis_input(
        &mut self,
        tag: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.suppress_next_control_activation {
            self.suppress_next_control_activation = false;
            return;
        }
        if self
            .variable_font_axis_editor
            .as_ref()
            .is_some_and(|editor| editor.tag == tag)
        {
            return;
        }
        if !self.begin_variable_font_axis_edit(tag, cx) {
            return;
        }
        let Some(editor) = self.variable_font_axis_editor.as_ref() else {
            return;
        };
        let draft = format_number(editor.original);
        self.suppress_property_input_change = true;
        self.property_input.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.suppress_property_input_change = false;
        cx.notify();
    }

    pub(super) fn parsed_variable_font_axis_draft(&self, cx: &App) -> Option<Result<f32, ()>> {
        let editor = self.variable_font_axis_editor.as_ref()?;
        let clamp =
            NumericClamp::new(Some(f64::from(editor.min)), Some(f64::from(editor.max))).ok();
        Some(
            evaluate_numeric_expression(
                self.property_input.read(cx).value().as_ref(),
                f64::from(editor.original),
                clamp,
            )
            .map_err(|_| ())
            .and_then(|value| {
                (value >= f64::from(f32::MIN) && value <= f64::from(f32::MAX))
                    .then_some(value as f32)
                    .filter(|value| value.is_finite())
                    .ok_or(())
            }),
        )
    }

    pub(super) fn validate_variable_font_axis_draft(&mut self, cx: &mut Context<Self>) {
        let parsed = self.parsed_variable_font_axis_draft(cx);
        self.variable_font_axis_editor_invalid = parsed.as_ref().is_some_and(Result::is_err);
        if !self.suppress_property_input_change
            && !self.variable_font_axis_editor_invalid
            && let Some(Ok(value)) = parsed
        {
            let should_preview = self
                .variable_font_axis_editor
                .as_ref()
                .is_some_and(|editor| {
                    self.variable_font_axis_editor_survives(editor)
                        && !(editor.last_preview.is_none() && editor.original == value)
                        && editor.last_preview != Some(value)
                });
            if should_preview {
                let editor = self
                    .variable_font_axis_editor
                    .as_ref()
                    .expect("axis preview requires an active editor")
                    .clone();
                if let Some(active) = self.variable_font_axis_editor.as_mut() {
                    active.last_preview = Some(value);
                }
                self.emit_variable_font_axis_action(
                    &editor,
                    value,
                    DesignPanelEditPhase::Preview,
                    cx,
                );
            }
        }
        cx.notify();
    }

    pub(super) fn cancel_variable_font_axis_transaction(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = self.variable_font_axis_editor.take() {
            self.emit_variable_font_axis_action(
                &editor,
                editor.original,
                DesignPanelEditPhase::Cancel,
                cx,
            );
        }
        self.variable_font_axis_scrub = None;
        self.variable_font_axis_editor_invalid = false;
        self.suppress_property_input_change = false;
    }

    pub(super) fn finish_variable_font_axis_edit(
        &mut self,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .variable_font_axis_scrub
            .as_ref()
            .is_some_and(|scrub| scrub.active)
        {
            self.finish_variable_font_axis_scrub(commit, false, window, cx);
            return;
        }
        let Some(editor) = self.variable_font_axis_editor.clone() else {
            return;
        };
        let value = commit
            .then(|| self.parsed_variable_font_axis_draft(cx))
            .flatten()
            .and_then(Result::ok)
            .filter(|_| self.variable_font_axis_editor_survives(&editor));
        self.variable_font_axis_editor = None;
        self.variable_font_axis_scrub = None;
        self.variable_font_axis_editor_invalid = false;
        self.suppress_property_input_change = false;
        let (phase, value) = value
            .map_or((DesignPanelEditPhase::Cancel, editor.original), |value| {
                (DesignPanelEditPhase::Commit, value)
            });
        self.emit_variable_font_axis_action(&editor, value, phase, cx);
        self.type_settings_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn step_variable_font_axis_input(
        &mut self,
        direction: ArrowStep,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = self.variable_font_axis_editor.clone() else {
            return false;
        };
        if !self.variable_font_axis_editor_survives(&editor) {
            return false;
        }
        let Some(Ok(current)) = self.parsed_variable_font_axis_draft(cx) else {
            return false;
        };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        let delta = editor.step * self.nudge_settings.amount(modifiers.shift) * fine;
        let value = match direction {
            ArrowStep::Increase => current + delta,
            ArrowStep::Decrease => current - delta,
        }
        .clamp(editor.min, editor.max);
        self.property_input.update(cx, |input, cx| {
            input.set_value(format_nudge_number(value), window, cx);
        });
        true
    }

    pub(super) fn apply_variable_font_axis_keyboard_step(
        &mut self,
        tag: SharedString,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.property_editor.is_some()
            || self.numeric_property_scrub.is_some()
            || self.variable_font_axis_editor.is_some()
            || self.variable_font_axis_scrub.is_some()
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
        let nudge = self.nudge_settings.amount(modifiers.shift) * fine;
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
        let editor = VariableFontAxisEditor {
            tag,
            target: self.variable_font_axis_target(),
            original: axis.value,
            last_preview: Some(value),
            min: axis.min,
            max: axis.max,
            default: axis.default,
            step: axis.step,
        };
        self.emit_variable_font_axis_action(
            &editor,
            editor.original,
            DesignPanelEditPhase::Begin,
            cx,
        );
        self.emit_variable_font_axis_action(&editor, value, DesignPanelEditPhase::Preview, cx);
        self.emit_variable_font_axis_action(&editor, value, DesignPanelEditPhase::Commit, cx);
        true
    }

    pub(super) fn handle_variable_font_axis_slider_key_down(
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

    pub(super) fn start_variable_font_axis_scrub(
        &mut self,
        tag: SharedString,
        position_x: f32,
        position_y: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.property_editor.is_some()
            || self.numeric_property_scrub.is_some()
            || self.variable_font_axis_editor.is_some()
            || self.variable_font_axis_scrub.is_some()
            || !self.variable_font_axis_is_editable(tag.as_ref())
        {
            return false;
        }
        let Some(axis) = self.variable_font_axis(tag.as_ref()) else {
            return false;
        };
        self.variable_font_axis_scrub = Some(VariableFontAxisScrub {
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

    pub(super) fn snapped_variable_font_axis_value(
        editor: &VariableFontAxisEditor,
        value: f32,
        fine: bool,
    ) -> f32 {
        let step = editor.step * if fine { 0.1 } else { 1. };
        let snapped = editor.default + ((value - editor.default) / step).round() * step;
        snapped.clamp(editor.min, editor.max)
    }

    pub(super) fn update_variable_font_axis_scrub(
        &mut self,
        tag: &str,
        position_x: f32,
        position_y: f32,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(mut scrub) = self.variable_font_axis_scrub.take() else {
            return;
        };
        if scrub.tag.as_ref() != tag {
            self.variable_font_axis_scrub = Some(scrub);
            return;
        }
        let speed = DesignScrubSpeed::from_vertical_displacement(position_y - scrub.origin_y);
        scrub.last_y = position_y;
        let delta = if scrub.active {
            position_x - scrub.last_x
        } else {
            let total = position_x - scrub.origin_x;
            if total.abs() < NUMERIC_SCRUB_THRESHOLD {
                self.variable_font_axis_scrub = Some(scrub);
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
        let Some(editor) = self.variable_font_axis_editor.clone() else {
            return;
        };
        let coarse = if modifiers.shift { 10. } else { 1. };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        let units_per_pixel = (editor.max - editor.min) / 160.;
        let raw = scrub.scalar + delta * units_per_pixel * coarse * fine * speed.multiplier();
        let value = Self::snapped_variable_font_axis_value(&editor, raw, modifiers.alt);
        scrub.scalar = value;
        let should_preview = self.variable_font_axis_editor_survives(&editor)
            && !(editor.last_preview.is_none() && editor.original == value)
            && editor.last_preview != Some(value);
        if should_preview && let Some(active) = self.variable_font_axis_editor.as_mut() {
            active.last_preview = Some(value);
        }
        self.variable_font_axis_scrub = Some(scrub);
        if should_preview {
            self.emit_variable_font_axis_action(&editor, value, DesignPanelEditPhase::Preview, cx);
        }
        cx.notify();
    }

    pub(super) fn finish_variable_font_axis_scrub(
        &mut self,
        commit: bool,
        suppress_click: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scrub) = self.variable_font_axis_scrub.take() else {
            return false;
        };
        if !scrub.active {
            cx.notify();
            return false;
        }
        let Some(editor) = self
            .variable_font_axis_editor
            .take()
            .filter(|editor| editor.tag == scrub.tag)
        else {
            cx.notify();
            return false;
        };
        self.variable_font_axis_editor_invalid = false;
        self.suppress_property_input_change = false;
        let valid_commit = commit && self.variable_font_axis_editor_survives(&editor);
        let (phase, value) = if valid_commit {
            (
                DesignPanelEditPhase::Commit,
                editor.last_preview.unwrap_or(editor.original),
            )
        } else {
            (DesignPanelEditPhase::Cancel, editor.original)
        };
        self.emit_variable_font_axis_action(&editor, value, phase, cx);
        if suppress_click {
            self.suppress_next_control_activation = true;
            cx.defer_in(window, |this, _, _| {
                this.suppress_next_control_activation = false;
            });
        }
        self.type_settings_focus.focus(window, cx);
        cx.notify();
        true
    }

    pub(super) fn text_path_start_for_property(
        &self,
        property: DesignPanelProperty,
        value: &DesignPanelValue,
    ) -> Option<DesignTextPathStartData> {
        let current = self.node.text_path_start_data?;
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

    pub(super) fn typography_style_binding(&self) -> Option<DesignTypographyStyleBinding> {
        self.node
            .typography
            .as_ref()
            .and_then(|typography| typography.style_binding.clone())
    }

    pub(super) fn sync_typography_style_picker(&self, cx: &mut Context<Self>) {
        let binding = self.typography_style_binding();
        let disabled = !self.property_is_editable(DesignPanelProperty::TypographyStyle);
        self.typography_style_picker.update(cx, |picker, cx| {
            picker.set_current_binding(binding, cx);
            picker.set_disabled(disabled, cx);
        });
    }

    pub(super) fn open_typography_style_picker(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.node.typography.is_none()
            || self.inspection_context.selection().kind() == DesignPanelSelectionKind::None
        {
            return;
        }
        self.typography_style_picker_open = true;
        self.paint_style_browser_open = None;
        self.font_browser_open = false;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.active_effect_settings = None;
        self.effect_style_browser_open = false;
        self.type_settings_open = false;
        self.selection_header_overlay = None;
        let view_data = self.typography_style_view_data.clone();
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

    pub(super) fn render_typography_style_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = cx.entity();
        let picker = self.typography_style_picker.clone();
        let picker_content = picker.clone();
        let picker_focus = picker.focus_handle(cx);
        let active = self.typography_style_picker_open;
        let tooltip = if self.property_is_editable(DesignPanelProperty::TypographyStyle) {
            "Text styles"
        } else {
            "Text styles · View only"
        };
        let trigger = Button::new(SharedString::from(format!("{}-typography-styles", self.id)))
            .tooltip(tooltip)
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(24.))
            .selected(active)
            .child(self.render_color_styles_icon(cx))
            .on_activate(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.open_typography_style_picker(window, cx);
            }));

        Popover::new(SharedString::from(format!(
            "{}-typography-style-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(active)
        .overlay_closable(true)
        .track_focus(&picker_focus)
        .on_open_change(move |open, window, cx| {
            panel.update(cx, |this, cx| {
                if *open {
                    this.open_typography_style_picker(window, cx);
                } else if this.typography_style_picker_open {
                    this.typography_style_picker_open = false;
                    cx.notify();
                }
            });
        })
        .trigger(trigger)
        .content(move |_, _, _| picker_content.clone())
        .into_any_element()
    }

    pub(super) fn open_font_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.node.typography.is_none()
            || self.inspection_context.selection().kind() == DesignPanelSelectionKind::None
        {
            return;
        }
        self.font_browser_open = true;
        self.paint_style_browser_open = None;
        self.typography_style_picker_open = false;
        self.type_settings_open = false;
        self.cancel_menu_preview(cx);
        self.active_picker = None;
        self.active_effect_settings = None;
        self.effect_style_browser_open = false;
        self.selection_header_overlay = None;
        self.font_search.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn emit_font_apply(&mut self, font: DesignFontSelection, cx: &mut Context<Self>) {
        let applicable_weight = self
            .font_view_data
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
        self.font_browser_open = false;
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyFontApplyRequested {
                node_id: self.node.id.clone(),
                target: self.typography_target(DesignPanelProperty::FontFamily),
                font,
            },
        );
        cx.notify();
    }

    pub(super) fn emit_font_import(&mut self, font: DesignFontSelection, cx: &mut Context<Self>) {
        if !self.can_edit()
            || !self
                .font_view_data
                .font(&font)
                .is_some_and(|(_, style)| style.availability.can_import())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::TypographyFontImportRequested {
                node_id: self.node.id.clone(),
                target: self.typography_target(DesignPanelProperty::FontFamily),
                font,
            },
        );
    }

    pub(super) fn emit_open_type_feature(
        &mut self,
        tag: DesignOpenTypeFeatureTag,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        let can_change = self
            .node
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
                node_id: self.node.id.clone(),
                target: self.typography_target(DesignPanelProperty::TextCase),
                tag,
                enabled,
            },
        );
    }

    pub(super) fn emit_text_path_flip_orientation(&mut self, cx: &mut Context<Self>) {
        let action = DesignPanelAction::TextPathFlipOrientationRequested {
            node_id: self.node.id.clone(),
        };
        if !self.can_edit() || !self.node_capability_allows_action(&action) {
            return;
        }
        cx.emit_design_panel_action(self, action);
    }

    pub(super) fn render_font_browser(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(typography) = self.node.typography.as_ref() else {
            return div().into_any_element();
        };
        let open = self.font_browser_open;
        let family_label =
            self.display_property_value(DesignPanelProperty::FontFamily, typography.family.clone());
        let style_label =
            self.display_property_value(DesignPanelProperty::FontStyle, typography.style.clone());
        let trigger_label: SharedString = format!("{family_label} · {style_label}").into();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let search = self.font_search.clone();
        let query = search.read(cx).value();
        let rows = self
            .font_view_data
            .matching(query.as_ref())
            .map(|(family, style)| {
                (
                    family.name.clone(),
                    style.clone(),
                    DesignFontSelection {
                        source: family.source.clone(),
                        family_id: family.id.clone(),
                        style_id: style.id.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let state = self.font_view_data.state.clone();
        let current_family = typography.family.clone();
        let current_style = typography.style.clone();
        let can_edit = self.can_edit();
        let can_edit_family = self.property_is_editable(DesignPanelProperty::FontFamily);
        let can_edit_style = self.property_is_editable(DesignPanelProperty::FontStyle);
        let can_edit_weight = self.property_is_editable(DesignPanelProperty::FontWeight);
        let trigger = Button::new(SharedString::from(format!("{}-font-browser", self.id)))
            .label(trigger_label)
            .tooltip("Browse font family and style")
            .xsmall()
            .compact()
            .w_full()
            .h(px(ROW_HEIGHT))
            .selected(open)
            .on_activate(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.open_font_browser(window, cx);
            }));

        let browser = Popover::new(SharedString::from(format!("{}-font-popover", self.id)))
            .anchor(Anchor::TopRight)
            .open(open)
            .overlay_closable(true)
            .on_open_change(move |is_open, window, cx| {
                panel_for_open.update(cx, |this, cx| {
                    if *is_open {
                        this.open_font_browser(window, cx);
                    } else if this.font_browser_open {
                        this.font_browser_open = false;
                        cx.notify();
                    }
                });
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let mut content = v_flex()
                    .w(popup_width(window, 320.))
                    .max_h(popup_height(window, 460.))
                    .gap_1()
                    .p_2()
                    .child(div().text_sm().font_semibold().child("Fonts"))
                    .child(
                        Input::new(&search)
                            .small()
                            .prefix(Icon::new(IconName::Search).small()),
                    );
                match state.clone() {
                    DesignFontCatalogState::Loading => {
                        content = content.child(
                            div()
                                .px_1()
                                .py_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Loading available fonts…"),
                        );
                    }
                    DesignFontCatalogState::Unavailable { reason } => {
                        content = content.child(
                            v_flex()
                                .px_1()
                                .py_3()
                                .gap_1()
                                .child(div().text_xs().child("Fonts unavailable"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(reason),
                                ),
                        );
                    }
                    DesignFontCatalogState::Ready if rows.is_empty() => {
                        content = content.child(
                            div()
                                .px_1()
                                .py_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("No fonts found"),
                        );
                    }
                    DesignFontCatalogState::Ready => {
                        let mut list = v_flex()
                            .w_full()
                            .max_h(popup_height(window, 360.))
                            .overflow_y_scrollbar();
                        for (family_name, style, selection) in rows.clone() {
                            let selected =
                                family_name == current_family && style.name == current_style;
                            let availability_label: SharedString = match &style.availability {
                                DesignFontAvailability::Imported => "Ready".into(),
                                DesignFontAvailability::Available => "Import".into(),
                                DesignFontAvailability::Missing { reason } => {
                                    format!("Missing · {reason}").into()
                                }
                                DesignFontAvailability::Unavailable { reason } => {
                                    format!("Unavailable · {reason}").into()
                                }
                            };
                            let can_apply = style.availability.can_apply();
                            let can_import = style.availability.can_import();
                            let can_apply_properties = can_edit_family
                                && can_edit_style
                                && (style.weight.is_none() || can_edit_weight);
                            let panel = panel_for_content.clone();
                            let selection_for_click = selection.clone();
                            list = list.child(
                                Button::new(SharedString::from(format!(
                                    "font-{}-{}",
                                    selection.family_id, selection.style_id
                                )))
                                .label(format!(
                                    "{} · {}  —  {}",
                                    family_name, style.name, availability_label
                                ))
                                .tooltip(
                                    style
                                        .preview
                                        .clone()
                                        .unwrap_or_else(|| "Font family and style".into()),
                                )
                                .xsmall()
                                .compact()
                                .ghost()
                                .w_full()
                                .selected(selected)
                                .disabled(
                                    !can_edit
                                        || (!can_import && (!can_apply || !can_apply_properties)),
                                )
                                .on_activate(move |_, _, cx| {
                                    let selection = selection_for_click.clone();
                                    panel.update(cx, |this, cx| {
                                        if can_import {
                                            this.emit_font_import(selection, cx);
                                        } else {
                                            this.emit_font_apply(selection, cx);
                                        }
                                    });
                                }),
                            );
                        }
                        content = content.child(list);
                    }
                }
                content
            })
            .w_full()
            .h(px(ROW_HEIGHT))
            .into_any_element();

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

    pub(super) fn render_text_resize_icon(
        &self,
        icon: TextResizeIcon,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = cx.theme().foreground;
        render_icon_canvas(color, 16., move |path| match icon {
            TextResizeIcon::AutoWidth => {
                path.line((3., 2.5), (3., 13.5));
                path.line((5., 8.), (13., 8.));
                path.poly([(10., 5.), (13., 8.), (10., 11.)], false);
            }
            TextResizeIcon::AutoHeight => {
                path.line((3., 2.5), (3., 13.5));
                path.line((13., 2.5), (13., 13.5));
                for (line_y, width) in [(5., 6.), (8., 8.), (11., 5.)] {
                    path.line((5., line_y), (5. + width, line_y));
                }
            }
            TextResizeIcon::FixedSize => {
                path.rect(2.5, 2.5, 13.5, 13.5);
                path.line((5., 6.), (11., 6.));
                path.line((5., 9.), (9., 9.));
            }
        })
    }

    pub(super) fn render_text_resize_control(
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

    pub(super) fn render_typography_alignment_icon(
        &self,
        icon: TypographyAlignmentIcon,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = cx.theme().foreground;
        render_icon_canvas(color, 16., move |path| match icon {
            TypographyAlignmentIcon::Horizontal(alignment) => {
                let middle_x = match alignment {
                    DesignTextHorizontalAlignment::Left => 2.,
                    DesignTextHorizontalAlignment::Center => 4.,
                    DesignTextHorizontalAlignment::Right => 6.,
                    DesignTextHorizontalAlignment::Justified => 2.,
                };
                let middle_width = if alignment == DesignTextHorizontalAlignment::Justified {
                    12.
                } else {
                    8.
                };
                for (line_y, line_x, width) in
                    [(3., 2., 12.), (7., middle_x, middle_width), (11., 2., 12.)]
                {
                    path.line((line_x, line_y), (line_x + width, line_y));
                }
            }
            TypographyAlignmentIcon::Vertical(alignment) => {
                let start_y = match alignment {
                    DesignTextVerticalAlignment::Top => 1.,
                    DesignTextVerticalAlignment::Center => 4.,
                    DesignTextVerticalAlignment::Bottom => 7.,
                };
                for (line_y, line_x, width) in [
                    (start_y, 2., 12.),
                    (start_y + 3., 4., 8.),
                    (start_y + 6., 2., 12.),
                ] {
                    path.line((line_x, line_y), (line_x + width, line_y));
                }
            }
        })
    }

    pub(super) fn render_typography_alignment_segment(
        &self,
        id_suffix: &'static str,
        icon: TypographyAlignmentIcon,
        selected: bool,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        Button::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .selected(selected)
            .disabled(!self.property_is_editable(property))
            .child(self.render_typography_alignment_icon(icon, cx))
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property(property, value.clone(), cx);
            }))
            .into_any_element()
    }

    pub(super) fn render_type_setting_segments(
        panel_id: SharedString,
        id_suffix: &'static str,
        options: Vec<(SharedString, bool, DesignPanelValue)>,
        enabled: bool,
        property: DesignPanelProperty,
        panel: Entity<Self>,
        cx: &mut App,
    ) -> AnyElement {
        let mut segments = h_flex()
            .h(px(28.))
            .w_full()
            .overflow_hidden()
            .rounded(px(4.))
            .bg(cx.theme().secondary);
        for (index, (label, selected, value)) in options.into_iter().enumerate() {
            let panel = panel.clone();
            segments = segments.child(
                div()
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .when(index > 0, |segment| {
                        segment
                            .border_l_1()
                            .border_color(cx.theme().border.opacity(0.72))
                    })
                    .child(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-type-setting-{id_suffix}-{index}"
                        )))
                        .label(label)
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .h_full()
                        .selected(selected)
                        .disabled(!enabled)
                        .on_activate(move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                this.emit_property(property, value.clone(), cx);
                            });
                        }),
                    ),
            );
        }
        segments.into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_type_setting_number_field(
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
        let id = SharedString::from(format!("{panel_id}-type-setting-{id_suffix}-number"));
        let (scrubbing, scrub_enabled) = {
            let panel = panel.read(cx);
            (
                panel.numeric_scrub_is_active(property),
                panel.numeric_scrub_surface_is_enabled(property),
            )
        };
        let panel_for_activate = panel.clone();
        let button_id = SharedString::from(format!("{id}-activate"));
        let button_debug_selector = button_id.clone();
        let button = Button::new(button_id)
            .debug_selector(move || button_debug_selector.to_string())
            .label(label)
            .xsmall()
            .compact()
            .w_full()
            .h(px(28.))
            .disabled(!enabled)
            .on_activate(move |_, window, cx| {
                panel_for_activate.update(cx, |this, cx| {
                    let type_settings_was_open = this.type_settings_open;
                    this.activate_property_from_control(
                        EditorFocusOrigin::TypeSetting(property),
                        property,
                        value.clone(),
                        window,
                        cx,
                    );
                    this.type_settings_open = type_settings_was_open;
                    cx.notify();
                });
            });
        if editing && !scrubbing {
            return div()
                .id(id)
                .relative()
                .h(px(28.))
                .w_full()
                .rounded(px(4.))
                .border_1()
                .border_color(if invalid {
                    cx.theme().red
                } else {
                    cx.theme().selection
                })
                .bg(cx.theme().secondary)
                .child(button.invisible().tab_stop(false))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top_0()
                        .bottom_0()
                        .child(
                            Input::new(&input)
                                .appearance(false)
                                .bordered(false)
                                .focus_bordered(false)
                                .xsmall()
                                .h(px(26.))
                                .w_full(),
                        ),
                )
                .into_any_element();
        }

        div()
            .id(id)
            .h(px(28.))
            .w_full()
            .child(render_numeric_scrub_surface(
                SharedString::from(format!("{panel_id}-type-setting-{id_suffix}-number-scrub")),
                panel,
                property,
                EditorFocusOrigin::TypeSetting(property),
                scrub_enabled,
                button.into_any_element(),
            ))
            .into_any_element()
    }

    pub(super) fn render_variable_font_axis_number_field(
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
                .variable_font_axis_editor
                .as_ref()
                .is_some_and(|editor| editor.tag == tag);
            let scrubbing = panel
                .variable_font_axis_scrub
                .as_ref()
                .is_some_and(|scrub| scrub.tag == tag && scrub.active);
            let displayed_value = panel
                .variable_font_axis_editor
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
                panel.variable_font_axis_editor_invalid,
                displayed_value,
                disabled_reason,
                panel.property_input.clone(),
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

    pub(super) fn render_variable_font_axis_slider(
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
                .variable_font_axis_editor
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

    pub(super) fn render_variable_font_axis_row(
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

    pub(super) fn render_type_settings_popover(
        &self,
        typography: &super::super::DesignTypography,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let open = self.type_settings_open;
        let requested_tab = self.type_settings_tab;
        let tab = if requested_tab == TypographySettingsTab::Variable
            && typography.variable_axes.is_empty()
        {
            TypographySettingsTab::Details
        } else {
            requested_tab
        };
        let focus = self.type_settings_focus.clone();
        let focus_for_open = focus.clone();
        let focus_for_content = focus.clone();
        let panel_id = self.id.clone();
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
        let axes = typography.variable_axes.clone();
        let has_variable_axes = !axes.is_empty();
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
            node_id: self.node.id.clone(),
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
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ParagraphIndent);
        let paragraph_spacing_editing = self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ParagraphSpacing);
        let list_spacing_editing = self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::ListSpacing);
        let max_lines_editing = self
            .property_editor
            .as_ref()
            .is_some_and(|editor| editor.property == DesignPanelProperty::TextMaxLines);
        let property_input = self.property_input.clone();
        let property_editor_invalid = self.property_editor_invalid;
        let can_edit_typography = self.can_edit();
        let paragraph_spacing_variable_button_state =
            self.property_variable_button_state(DesignPanelProperty::ParagraphSpacing);
        let paragraph_indent_variable_button_state =
            self.property_variable_button_state(DesignPanelProperty::ParagraphIndent);

        let trigger = Button::new(SharedString::from(format!("{}-type-settings", self.id)))
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(ROW_HEIGHT))
            .on_keyboard_activate({
                let panel = panel.clone();
                let focus = focus.clone();
                move |window, cx| {
                    panel.update(cx, |this, cx| {
                        this.type_settings_open = !open;
                        if !open {
                            this.cancel_menu_preview(cx);
                            this.active_picker = None;
                            this.typography_style_picker_open = false;
                            this.font_browser_open = false;
                            focus.focus(window, cx);
                        } else {
                            this.type_settings_tab = TypographySettingsTab::Basics;
                        }
                        cx.notify();
                    });
                }
            })
            .child(Icon::new(IconName::Settings2).xsmall());

        Popover::new(SharedString::from(format!(
            "{}-type-settings-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .track_focus(&focus)
        .on_open_change(move |open, window, cx| {
            panel_for_open.update(cx, |this, cx| {
                this.type_settings_open = *open;
                if *open {
                    this.cancel_menu_preview(cx);
                    this.active_picker = None;
                    this.typography_style_picker_open = false;
                    this.font_browser_open = false;
                    focus_for_open.focus(window, cx);
                } else {
                    this.type_settings_tab = TypographySettingsTab::Basics;
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let muted_foreground = cx.theme().muted_foreground;
            let tab_button = |candidate: TypographySettingsTab, panel: Entity<DesignPanel>| {
                Button::new(SharedString::from(format!(
                    "{}-type-settings-tab-{}",
                    panel_id,
                    candidate.label().to_lowercase()
                )))
                .label(candidate.label())
                .xsmall()
                .compact()
                .ghost()
                .h(px(28.))
                .selected(candidate == tab)
                .on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.type_settings_tab = candidate;
                        cx.notify();
                    });
                })
            };
            let control_row = |label: &'static str, control: AnyElement| {
                h_flex()
                    .h(px(28.))
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .w(px(112.))
                            .flex_none()
                            .text_xs()
                            .text_color(muted_foreground)
                            .child(label),
                    )
                    .child(div().flex_1().min_w(px(0.)).child(control))
            };
            let mut tabs = h_flex()
                .flex_1()
                .gap_1()
                .child(tab_button(TypographySettingsTab::Basics, panel.clone()))
                .child(tab_button(TypographySettingsTab::Details, panel.clone()));
            if has_variable_axes {
                tabs = tabs.child(tab_button(TypographySettingsTab::Variable, panel.clone()));
            }

            let mut body = v_flex().w_full().gap_2();
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
                        .zip(["—", "U̲", "S̶"])
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
                    .zip(["—", "AG", "ag", "Ag", "Aɢ"])
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
                        .zip(["—", "•≡", "1≡"])
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
                            SharedString::from("—"),
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
                                SharedString::from("—"),
                                !enabled,
                                DesignPanelValue::Bool(false),
                            ),
                            (
                                SharedString::from("✓"),
                                enabled,
                                DesignPanelValue::Bool(true),
                            ),
                        ]
                    };
                    let case_options = DesignTextCase::ALL
                        .into_iter()
                        .zip(["—", "AG", "ag", "Ag", "Aɢ", "Aɢ⁺"])
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
                            .h(px(28.))
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
                                    h_flex()
                                        .h(px(28.))
                                        .w_full()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(112.))
                                                .flex_none()
                                                .truncate()
                                                .text_xs()
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
            v_flex()
                .id("type-settings-content")
                .key_context(DESIGN_PANEL_KEY_CONTEXT)
                .track_focus(&focus_for_content)
                .tab_index(0)
                .w(popup_width(window, 360.))
                .max_h(popup_height(window, 680.))
                .p_3()
                .gap_2()
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .shadow_lg()
                .child(
                    h_flex().h(px(28.)).gap_1().child(tabs).child(
                        Button::new(SharedString::from(format!(
                            "{}-type-settings-close",
                            panel_id
                        )))
                        .xsmall()
                        .compact()
                        .ghost()
                        .w(px(28.))
                        .h(px(28.))
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

    pub(super) fn render_text_path_orientation_control(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let text_path = self.node.text_path.as_ref()?;
        let enabled = self.can_edit() && self.text_path_flip_is_available();
        let orientation = text_path.orientation;
        let tooltip: SharedString = if enabled {
            format!("Flip text orientation · currently {}", orientation.label()).into()
        } else if self.can_edit() {
            format!(
                "Flip text orientation unavailable · currently {}",
                orientation.label()
            )
            .into()
        } else {
            format!(
                "Flip text orientation · view only · currently {}",
                orientation.label()
            )
            .into()
        };

        let control_id = SharedString::from(format!("{}-text-path-flip-orientation", self.id));
        let debug_selector = control_id.to_string();
        let mut flip_control = div()
            .id(control_id)
            .debug_selector(move || debug_selector)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().transparent)
            .bg(if orientation == DesignTextPathOrientation::Flipped {
                cx.theme().accent
            } else {
                cx.theme().secondary
            })
            .text_xs()
            .when(!enabled, |control| {
                control
                    .text_color(cx.theme().muted_foreground)
                    .opacity(0.62)
            });
        if enabled {
            flip_control = flip_control
                .key_context(CONTROL_KEY_CONTEXT)
                .tab_index(0)
                .cursor_pointer()
                .hover(|style| style.bg(cx.theme().accent))
                .focus(|style| {
                    style
                        .bg(cx.theme().accent)
                        .border_color(cx.theme().selection)
                })
                .on_activate(cx.listener(|this, _, _, cx| {
                    this.emit_text_path_flip_orientation(cx);
                }));
        }
        Some(
            v_flex()
                .w_full()
                .gap_1()
                .child(flip_control.child("Flip text orientation"))
                .child(
                    div()
                        .px_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tooltip),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_typography(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let typography = self.node.typography.as_ref()?;
        let mut content = if let Some(binding) = typography.style_binding.as_ref() {
            v_flex()
                .px(px(PANEL_PADDING))
                .pb_4()
                .gap_2()
                .child(self.render_bound_style_summary("typography", binding.name.clone(), cx))
        } else {
            v_flex()
                .px(px(PANEL_PADDING))
                .pb_4()
                .gap_2()
                .child(self.render_font_browser(cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(self.render_group_label("Weight", cx))
                                .child(self.render_value_cell(
                                    "font-weight",
                                    "W",
                                    format_number(typography.weight),
                                    DesignPanelProperty::FontWeight,
                                    DesignPanelValue::Number((typography.weight + 100.).min(1000.)),
                                    cx,
                                )),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(self.render_group_label("Size", cx))
                                .child(self.render_value_cell(
                                    "font-size",
                                    "↕",
                                    format_number(typography.size),
                                    DesignPanelProperty::FontSize,
                                    DesignPanelValue::Number((typography.size + 1.).max(1.)),
                                    cx,
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(self.render_group_label("Line height", cx))
                                .child(self.render_value_cell(
                                    "line-height",
                                    "↕",
                                    format_line_height(typography.line_height),
                                    DesignPanelProperty::LineHeight,
                                    DesignPanelValue::LineHeight(next_line_height(
                                        typography.line_height,
                                    )),
                                    cx,
                                )),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w(px(0.))
                                .gap_1()
                                .child(self.render_group_label("Letter spacing", cx))
                                .child(self.render_value_cell(
                                    "letter-spacing",
                                    "↔",
                                    format_letter_spacing(typography.letter_spacing),
                                    DesignPanelProperty::LetterSpacing,
                                    DesignPanelValue::LetterSpacing(next_letter_spacing(
                                        typography.letter_spacing,
                                    )),
                                    cx,
                                )),
                        ),
                )
                .child(self.render_group_label("Alignment", cx))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            h_flex()
                                .h(px(ROW_HEIGHT))
                                .flex_1()
                                .overflow_hidden()
                                .rounded(px(4.))
                                .bg(cx.theme().secondary)
                                .child(self.render_typography_alignment_segment(
                                    "text-align-left",
                                    TypographyAlignmentIcon::Horizontal(
                                        DesignTextHorizontalAlignment::Left,
                                    ),
                                    typography.horizontal_alignment
                                        == DesignTextHorizontalAlignment::Left,
                                    DesignPanelProperty::HorizontalTextAlignment,
                                    DesignPanelValue::TextHorizontalAlignment(
                                        DesignTextHorizontalAlignment::Left,
                                    ),
                                    cx,
                                ))
                                .child(self.render_typography_alignment_segment(
                                    "text-align-center",
                                    TypographyAlignmentIcon::Horizontal(
                                        DesignTextHorizontalAlignment::Center,
                                    ),
                                    typography.horizontal_alignment
                                        == DesignTextHorizontalAlignment::Center,
                                    DesignPanelProperty::HorizontalTextAlignment,
                                    DesignPanelValue::TextHorizontalAlignment(
                                        DesignTextHorizontalAlignment::Center,
                                    ),
                                    cx,
                                ))
                                .child(self.render_typography_alignment_segment(
                                    "text-align-right",
                                    TypographyAlignmentIcon::Horizontal(
                                        DesignTextHorizontalAlignment::Right,
                                    ),
                                    typography.horizontal_alignment
                                        == DesignTextHorizontalAlignment::Right,
                                    DesignPanelProperty::HorizontalTextAlignment,
                                    DesignPanelValue::TextHorizontalAlignment(
                                        DesignTextHorizontalAlignment::Right,
                                    ),
                                    cx,
                                )),
                        )
                        .child(
                            h_flex()
                                .h(px(ROW_HEIGHT))
                                .flex_1()
                                .overflow_hidden()
                                .rounded(px(4.))
                                .bg(cx.theme().secondary)
                                .child(self.render_typography_alignment_segment(
                                    "vertical-text-align-top",
                                    TypographyAlignmentIcon::Vertical(
                                        DesignTextVerticalAlignment::Top,
                                    ),
                                    typography.vertical_alignment
                                        == DesignTextVerticalAlignment::Top,
                                    DesignPanelProperty::VerticalTextAlignment,
                                    DesignPanelValue::TextVerticalAlignment(
                                        DesignTextVerticalAlignment::Top,
                                    ),
                                    cx,
                                ))
                                .child(self.render_typography_alignment_segment(
                                    "vertical-text-align-center",
                                    TypographyAlignmentIcon::Vertical(
                                        DesignTextVerticalAlignment::Center,
                                    ),
                                    typography.vertical_alignment
                                        == DesignTextVerticalAlignment::Center,
                                    DesignPanelProperty::VerticalTextAlignment,
                                    DesignPanelValue::TextVerticalAlignment(
                                        DesignTextVerticalAlignment::Center,
                                    ),
                                    cx,
                                ))
                                .child(self.render_typography_alignment_segment(
                                    "vertical-text-align-bottom",
                                    TypographyAlignmentIcon::Vertical(
                                        DesignTextVerticalAlignment::Bottom,
                                    ),
                                    typography.vertical_alignment
                                        == DesignTextVerticalAlignment::Bottom,
                                    DesignPanelProperty::VerticalTextAlignment,
                                    DesignPanelValue::TextVerticalAlignment(
                                        DesignTextVerticalAlignment::Bottom,
                                    ),
                                    cx,
                                )),
                        )
                        .child(self.render_type_settings_popover(typography, cx)),
                )
        };
        if let Some(applied) = self.render_applied_component_property_controls(
            DesignComponentPropertyApplicationSurface::Text,
            cx,
        ) {
            content = content.child(applied);
        }
        if let Some(orientation_control) = self.render_text_path_orientation_control(cx) {
            content = content
                .child(self.render_group_label("Text on path", cx))
                .child(orientation_control);
        }
        if self.text_path_start_debug_controls_are_available()
            && let Some(start) = self.node.text_path_start_data
        {
            content = content
                .child(self.render_group_label("Start data · API debug", cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w(px(0.)).child(self.render_value_cell(
                            "text-path-start-segment",
                            "#",
                            start.segment.to_string(),
                            DesignPanelProperty::TextPathStartSegment,
                            DesignPanelValue::Integer(i64::from(start.segment.saturating_add(1))),
                            cx,
                        )))
                        .child(div().flex_1().min_w(px(0.)).child(self.render_value_cell(
                            "text-path-start-position",
                            "%",
                            format!("{}%", format_number(start.position * 100.)),
                            DesignPanelProperty::TextPathStartPosition,
                            DesignPanelValue::Ratio((start.position + 0.05).min(1.)),
                            cx,
                        ))),
                );
        }
        Some(self.render_section(
            DesignPanelSection::Typography,
            None,
            content.into_any_element(),
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

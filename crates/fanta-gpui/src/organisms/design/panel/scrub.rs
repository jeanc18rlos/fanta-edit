use super::*;

impl DesignPanel {
    /// Current Figma scrub band while either numeric scrub transaction is
    /// active. Pre-threshold click candidates deliberately return `None`.
    pub fn active_scrub_speed(&self) -> Option<DesignScrubSpeed> {
        self.numeric_property_scrub
            .as_ref()
            .filter(|scrub| scrub.active)
            .map(|scrub| {
                DesignScrubSpeed::from_vertical_displacement(scrub.last_y - scrub.origin_y)
            })
            .or_else(|| {
                self.variable_font_axis_scrub
                    .as_ref()
                    .filter(|scrub| scrub.active)
                    .map(|scrub| {
                        DesignScrubSpeed::from_vertical_displacement(scrub.last_y - scrub.origin_y)
                    })
            })
    }

    pub(super) fn render_scrub_speed_cue(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let speed = self.active_scrub_speed()?;
        let id = SharedString::from(format!("{}-scrub-speed-cue", self.id));
        let selector = id.to_string();
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(12.))
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .id(id)
                        .debug_selector(move || selector.clone())
                        .h(px(28.))
                        .px_3()
                        .gap_2()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().popover.opacity(0.96))
                        .text_color(cx.theme().popover_foreground)
                        .shadow_lg()
                        .child(
                            div()
                                .w(px(speed.cue_width()))
                                .h(px(2.))
                                .rounded_full()
                                .bg(cx.theme().selection),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .child(format!("Scrub {}", speed.label())),
                        ),
                )
                .into_any_element(),
        )
    }

    pub(super) fn numeric_scrub_seed(
        &self,
        property: DesignPanelProperty,
    ) -> Option<(DesignPanelValue, PropertyEditorKind, f64)> {
        if !self.property_is_editable(property)
            || self.vector_property_is_mixed(property)
            || self.smart_selection_property_is_mixed(property)
        {
            return None;
        }
        let current = match self.property_value_states.get(&property) {
            Some(DesignPanelPropertyValueState::Uniform(value)) => value.clone(),
            Some(
                DesignPanelPropertyValueState::Unset
                | DesignPanelPropertyValueState::Mixed
                | DesignPanelPropertyValueState::Bound(_)
                | DesignPanelPropertyValueState::ReadOnly(_),
            ) => return None,
            None => self.current_property_value(property)?,
        };
        let (kind, scalar) = match &current {
            DesignPanelValue::Number(value) => (
                PropertyEditorKind::Number {
                    integer: false,
                    clamp: Self::property_clamp(property),
                },
                f64::from(*value),
            ),
            DesignPanelValue::AngleRadians(value) => (
                PropertyEditorKind::AngleDegrees,
                f64::from(value.to_degrees()),
            ),
            DesignPanelValue::Ratio(value) => (
                PropertyEditorKind::PercentageRatio,
                f64::from(*value * 100.),
            ),
            DesignPanelValue::Integer(value) => (
                PropertyEditorKind::Number {
                    integer: true,
                    clamp: Self::property_clamp(property),
                },
                *value as f64,
            ),
            DesignPanelValue::OptionalNumber(Some(value)) => (
                PropertyEditorKind::OptionalNumber {
                    clamp: Self::property_clamp(property),
                },
                f64::from(*value),
            ),
            DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Number(value)) => {
                (PropertyEditorKind::LayoutGridCount, f64::from(*value))
            }
            DesignPanelValue::ExportSizing(sizing) => {
                (PropertyEditorKind::ExportSizing, f64::from(sizing.value()))
            }
            _ => return None,
        };
        scalar.is_finite().then_some((current, kind, scalar))
    }

    pub(super) fn numeric_scrub_value(
        property: DesignPanelProperty,
        kind: PropertyEditorKind,
        original: &DesignPanelValue,
        scalar: f64,
    ) -> Option<(f64, DesignPanelValue)> {
        if !scalar.is_finite() {
            return None;
        }
        let apply_clamp = |value: f64, clamp: Option<NumericClamp>| {
            clamp.map_or(Some(value), |clamp| clamp.apply(value).ok())
        };
        match kind {
            PropertyEditorKind::Number { integer, clamp } => {
                let mut scalar = apply_clamp(scalar, clamp)?;
                if integer {
                    scalar = round_to_integer(scalar).ok()?;
                    if scalar < i64::MIN as f64 || scalar > i64::MAX as f64 {
                        return None;
                    }
                    Some((scalar, DesignPanelValue::Integer(scalar as i64)))
                } else if scalar < f64::from(f32::MIN) || scalar > f64::from(f32::MAX) {
                    None
                } else {
                    Some((scalar, DesignPanelValue::Number(scalar as f32)))
                }
            }
            PropertyEditorKind::OptionalNumber { clamp } => {
                let mut scalar = apply_clamp(scalar, clamp)?;
                if property == DesignPanelProperty::TextMaxLines {
                    scalar = round_to_integer(scalar).ok()?;
                }
                if scalar < f64::from(f32::MIN) || scalar > f64::from(f32::MAX) {
                    None
                } else {
                    Some((
                        scalar,
                        DesignPanelValue::OptionalNumber(Some(scalar as f32)),
                    ))
                }
            }
            PropertyEditorKind::AngleDegrees => {
                if scalar < f64::from(f32::MIN) || scalar > f64::from(f32::MAX) {
                    None
                } else {
                    Some((
                        scalar,
                        DesignPanelValue::AngleRadians((scalar as f32).to_radians()),
                    ))
                }
            }
            PropertyEditorKind::PercentageRatio => {
                let scalar = NumericClamp::new(Some(0.), Some(100.))
                    .ok()?
                    .apply(scalar)
                    .ok()?;
                Some((scalar, DesignPanelValue::Ratio(scalar as f32 / 100.)))
            }
            PropertyEditorKind::LayoutGridCount => {
                let scalar = NumericClamp::new(Some(1.), Some(f64::from(u16::MAX)))
                    .ok()?
                    .apply(scalar)
                    .and_then(round_to_integer)
                    .ok()?;
                let value = u16::try_from(scalar as i64).ok()?;
                Some((
                    scalar,
                    DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::number(value)),
                ))
            }
            PropertyEditorKind::ExportSizing => {
                let scalar = NumericClamp::new(Some(0.01), None)
                    .ok()?
                    .apply(scalar)
                    .ok()?;
                if scalar > f64::from(f32::MAX) {
                    return None;
                }
                let scalar = scalar as f32;
                let sizing = match original {
                    DesignPanelValue::ExportSizing(DesignExportSizing::Scale(_)) => {
                        DesignExportSizing::Scale(scalar)
                    }
                    DesignPanelValue::ExportSizing(DesignExportSizing::Width(_)) => {
                        DesignExportSizing::Width(scalar)
                    }
                    DesignPanelValue::ExportSizing(DesignExportSizing::Height(_)) => {
                        DesignExportSizing::Height(scalar)
                    }
                    _ => return None,
                };
                Some((f64::from(scalar), DesignPanelValue::ExportSizing(sizing)))
            }
            PropertyEditorKind::Color
            | PropertyEditorKind::Text
            | PropertyEditorKind::NumberList
            | PropertyEditorKind::Shader { .. } => None,
        }
    }

    pub(super) fn numeric_scrub_multiplier(modifiers: Modifiers) -> f64 {
        let coarse = if modifiers.shift { 10. } else { 1. };
        let fine = if modifiers.alt { 0.1 } else { 1. };
        coarse * fine
    }

    pub(super) fn numeric_scrub_is_active(&self, property: DesignPanelProperty) -> bool {
        self.numeric_property_scrub
            .as_ref()
            .is_some_and(|scrub| scrub.property == property && scrub.active)
    }

    pub(super) fn numeric_scrub_surface_is_enabled(&self, property: DesignPanelProperty) -> bool {
        self.numeric_property_scrub
            .as_ref()
            .is_some_and(|scrub| scrub.property == property)
            || (self.property_editor.is_none()
                && self.numeric_property_scrub.is_none()
                && self.numeric_scrub_seed(property).is_some())
    }

    pub(super) fn start_numeric_property_scrub(
        &mut self,
        property: DesignPanelProperty,
        position_x: f32,
        position_y: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.property_editor.is_some() || self.numeric_property_scrub.is_some() {
            return false;
        }
        let Some((original, kind, scalar)) = self.numeric_scrub_seed(property) else {
            return false;
        };
        self.editor_focus_return = None;
        self.numeric_property_scrub = Some(NumericPropertyScrub {
            property,
            original,
            kind,
            scalar,
            origin_x: position_x,
            origin_y: position_y,
            last_x: position_x,
            last_y: position_y,
            active: false,
        });
        cx.notify();
        true
    }

    pub(super) fn begin_numeric_property_scrub_transaction(
        &mut self,
        scrub: &NumericPropertyScrub,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.property_editor.is_some() {
            return false;
        }
        let Some((current, kind, scalar)) = self.numeric_scrub_seed(scrub.property) else {
            return false;
        };
        if current != scrub.original || kind != scrub.kind || scalar != scrub.scalar {
            return false;
        }
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
        self.active_effect_settings = None;
        self.grid_dimensions_picker = None;
        self.effect_style_browser_open = false;
        self.property_variable_picker = None;
        self.component_property_variable_picker = None;
        self.component_swap_browser = None;
        self.vector_edit_target_ids = Self::vector_property_is_contextual(scrub.property)
            .then(|| {
                self.active_vector_edit()
                    .map(DesignVectorEditViewData::selected_vertex_ids)
            })
            .flatten();
        let layout_grid_target = scrub
            .property
            .layout_grid_index()
            .and_then(|index| self.layout_grid_target_for_index(index));
        let export_configuration_id = Self::export_property_index(scrub.property)
            .and_then(|index| self.export_configuration(index))
            .map(|configuration| configuration.id);
        self.property_editor = Some(PropertyEditor {
            property: scrub.property,
            layout_grid_target,
            export_configuration_id,
            original: scrub.original.clone(),
            last_preview: None,
            base: scrub.scalar,
            kind: scrub.kind,
        });
        self.property_editor_invalid = false;
        self.emit_property_edit(
            scrub.property,
            scrub.original.clone(),
            DesignPanelEditPhase::Begin,
            cx,
        );
        true
    }

    pub(super) fn update_numeric_property_scrub(
        &mut self,
        property: DesignPanelProperty,
        position_x: f32,
        position_y: f32,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) {
        let Some(mut scrub) = self.numeric_property_scrub.take() else {
            return;
        };
        if scrub.property != property {
            self.numeric_property_scrub = Some(scrub);
            return;
        }

        let speed = DesignScrubSpeed::from_vertical_displacement(position_y - scrub.origin_y);
        scrub.last_y = position_y;
        let delta = if scrub.active {
            position_x - scrub.last_x
        } else {
            let total = position_x - scrub.origin_x;
            if total.abs() < NUMERIC_SCRUB_THRESHOLD {
                self.numeric_property_scrub = Some(scrub);
                return;
            }
            if !self.begin_numeric_property_scrub_transaction(&scrub, cx) {
                return;
            }
            scrub.active = true;
            total
        };
        scrub.last_x = position_x;
        let scalar = scrub.scalar
            + f64::from(delta)
                * Self::numeric_scrub_multiplier(modifiers)
                * f64::from(speed.multiplier());
        let Some((scalar, value)) =
            Self::numeric_scrub_value(scrub.property, scrub.kind, &scrub.original, scalar)
        else {
            self.numeric_property_scrub = Some(scrub);
            return;
        };
        scrub.scalar = scalar;
        let should_preview = self.property_editor.as_ref().is_some_and(|editor| {
            editor.property == scrub.property
                && !(editor.last_preview.is_none() && editor.original == value)
                && editor.last_preview.as_ref() != Some(&value)
        }) && self.layout_property_value_is_applicable(scrub.property, &value);
        if should_preview && let Some(editor) = self.property_editor.as_mut() {
            editor.last_preview = Some(value.clone());
        }
        self.numeric_property_scrub = Some(scrub);
        if should_preview {
            self.emit_property_edit(property, value, DesignPanelEditPhase::Preview, cx);
        }
        cx.notify();
    }

    pub(super) fn finish_numeric_property_scrub(
        &mut self,
        commit: bool,
        suppress_click: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.finish_numeric_property_scrub_with_focus_restore(
            commit,
            suppress_click,
            true,
            window,
            cx,
        )
    }

    pub(super) fn finish_numeric_property_scrub_with_focus_restore(
        &mut self,
        commit: bool,
        suppress_click: bool,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scrub) = self.numeric_property_scrub.take() else {
            return false;
        };
        if !scrub.active {
            cx.notify();
            return false;
        }
        let Some(editor) = self
            .property_editor
            .take()
            .filter(|editor| editor.property == scrub.property)
        else {
            cx.notify();
            return false;
        };
        self.vector_edit_target_ids = None;
        self.property_editor_invalid = false;
        self.suppress_property_input_change = false;
        let (phase, value) = if commit {
            (
                DesignPanelEditPhase::Commit,
                editor
                    .last_preview
                    .unwrap_or_else(|| editor.original.clone()),
            )
        } else {
            (DesignPanelEditPhase::Cancel, editor.original)
        };
        let return_focus = self.numeric_scrub_return_focus(scrub.property);
        self.emit_property_edit(scrub.property, value, phase, cx);
        if suppress_click {
            self.suppress_next_control_activation = true;
            cx.defer_in(window, |this, _, _| {
                this.suppress_next_control_activation = false;
            });
        }
        let fallback = if self.type_settings_open {
            self.type_settings_focus.clone()
        } else {
            self.focus_handle.clone()
        };
        if restore_focus {
            Self::defer_editor_focus(return_focus.unwrap_or(fallback), window, cx);
        }
        cx.notify();
        true
    }
}

pub(super) fn render_numeric_scrub_surface(
    id: SharedString,
    panel: Entity<DesignPanel>,
    property: DesignPanelProperty,
    origin: EditorFocusOrigin,
    enabled: bool,
    content: AnyElement,
) -> AnyElement {
    let mut surface = div()
        .id(id)
        .h_full()
        .w_full()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .child(content);
    if enabled {
        let panel_for_down = panel.clone();
        let panel_for_move = panel.clone();
        let panel_for_up = panel.clone();
        let origin_for_down = origin.clone();
        surface = surface
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    let origin = origin_for_down.clone();
                    panel_for_down.update(cx, |this, cx| {
                        let started = this.start_numeric_property_scrub(
                            property,
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                            cx,
                        );
                        if started {
                            cx.defer_in(window, move |this, window, cx| {
                                this.capture_numeric_scrub_focus(origin, property, window, cx);
                            });
                        }
                    });
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                if event.dragging() {
                    panel_for_move.update(cx, |this, cx| {
                        this.update_numeric_property_scrub(
                            property,
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
                    this.finish_numeric_property_scrub(true, true, window, cx);
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, window, cx| {
                panel.update(cx, |this, cx| {
                    this.finish_numeric_property_scrub(true, false, window, cx);
                });
            });
    }
    surface.into_any_element()
}

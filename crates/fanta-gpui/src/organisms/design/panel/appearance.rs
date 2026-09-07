use super::*;

impl DesignPanel {
    pub(super) fn rebuild_draw_corner_radius_slider(&mut self, cx: &mut Context<Self>) {
        let range = self
            .draw_appearance_view_data
            .as_ref()
            .map(|view_data| view_data.corner_radius_range)
            .unwrap_or_else(|| {
                DesignDrawSliderRange::new(0., 100., 1.)
                    .expect("the private Draw corner fallback range is valid")
            });
        let state = cx.new(|_| {
            SliderState::new()
                .min(range.min())
                .max(range.max())
                .step(range.step())
                .default_value(range.clamp_and_snap(self.node.corner_radii[0]))
        });
        let subscription = cx.subscribe(&state, |this, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(SliderValue::Single(value)) = event else {
                return;
            };
            this.preview_draw_appearance_slider(DesignPanelProperty::CornerRadius, *value, cx);
        });
        self.draw_corner_radius_slider = state;
        self._draw_corner_radius_slider_subscription = subscription;
    }

    pub(super) fn visibility_control_state(
        &self,
        property: DesignPanelProperty,
        fallback_visible: bool,
    ) -> VisibilityControlState {
        match self.property_value_states.get(&property) {
            Some(state) if state.is_mixed() => VisibilityControlState::Mixed,
            Some(state) => match state.resolved() {
                Some(DesignPanelValue::Bool(visible)) => {
                    VisibilityControlState::from_visible(*visible)
                }
                _ => VisibilityControlState::from_visible(fallback_visible),
            },
            None => VisibilityControlState::from_visible(fallback_visible),
        }
    }

    pub(super) fn draw_appearance_slider_range(
        &self,
        property: DesignPanelProperty,
    ) -> Option<DesignDrawSliderRange> {
        match property {
            DesignPanelProperty::Opacity if self.node.supports_layer_appearance() => {
                DesignDrawSliderRange::new(0., 100., 1.)
            }
            DesignPanelProperty::CornerRadius if self.node.corner_capabilities.uniform_radius => {
                self.draw_appearance_view_data_for_context()
                    .map(|view_data| view_data.corner_radius_range)
            }
            _ => None,
        }
    }

    pub(super) fn preview_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.renders_draw_workspace() {
            return;
        }
        let Some(range) = self.draw_appearance_slider_range(property) else {
            return;
        };
        let value = DesignPanelValue::Number(range.clamp_and_snap(value));
        if self.draw_appearance_slider_property.is_none() {
            if self.property_editor.is_some() || self.numeric_property_scrub.is_some() {
                return;
            }
            let Some((original, kind, base)) = self.numeric_scrub_seed(property) else {
                return;
            };
            self.cancel_menu_preview(cx);
            self.active_picker = None;
            self.active_effect_settings = None;
            self.grid_dimensions_picker = None;
            self.effect_style_browser_open = false;
            self.property_variable_picker = None;
            self.component_property_variable_picker = None;
            self.component_swap_browser = None;
            self.type_settings_open = false;
            self.editor_focus_return = None;
            self.property_editor = Some(PropertyEditor {
                property,
                layout_grid_target: None,
                export_configuration_id: None,
                original: original.clone(),
                last_preview: None,
                base,
                kind,
            });
            self.draw_appearance_slider_property = Some(property);
            self.property_editor_invalid = false;
            self.emit_property_edit(property, original, DesignPanelEditPhase::Begin, cx);
        }
        if self.draw_appearance_slider_property != Some(property) {
            return;
        }
        let should_preview = self.property_editor.as_ref().is_some_and(|editor| {
            editor.property == property
                && !(editor.last_preview.is_none() && editor.original == value)
                && editor.last_preview.as_ref() != Some(&value)
        }) && self.layout_property_value_is_applicable(property, &value);
        if should_preview && let Some(editor) = self.property_editor.as_mut() {
            editor.last_preview = Some(value.clone());
        }
        if should_preview {
            self.emit_property_edit(property, value, DesignPanelEditPhase::Preview, cx);
        }
        cx.notify();
    }

    pub(super) fn finish_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        commit: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.draw_appearance_slider_property != Some(property) {
            return false;
        }
        self.draw_appearance_slider_property = None;
        let Some(editor) = self
            .property_editor
            .take()
            .filter(|editor| editor.property == property)
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
        self.emit_property_edit(property, value, phase, cx);
        cx.notify();
        true
    }

    pub(super) fn step_draw_appearance_slider(
        &mut self,
        property: DesignPanelProperty,
        direction: f32,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(range) = self.draw_appearance_slider_range(property) else {
            return false;
        };
        let Some((_, _, scalar)) = self.numeric_scrub_seed(property) else {
            return false;
        };
        let current = scalar as f32;
        let next = range.clamp_and_snap(current + range.step() * direction.signum());
        if (next - current).abs() <= f32::EPSILON {
            return false;
        }
        self.preview_draw_appearance_slider(property, next, cx);
        self.finish_draw_appearance_slider(property, true, cx)
    }

    pub(super) fn sync_draw_appearance_sliders(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for (property, state) in [
            (
                DesignPanelProperty::Opacity,
                self.draw_opacity_slider.clone(),
            ),
            (
                DesignPanelProperty::CornerRadius,
                self.draw_corner_radius_slider.clone(),
            ),
        ] {
            if self.draw_appearance_slider_property == Some(property)
                || self.draw_appearance_slider_range(property).is_none()
            {
                continue;
            }
            let Some(DesignPanelValue::Number(value)) = self.resolved_property_value(property)
            else {
                continue;
            };
            let Some(range) = self.draw_appearance_slider_range(property) else {
                continue;
            };
            let value = range.clamp_and_snap(value);
            if (state.read(cx).value().end() - value).abs() <= f32::EPSILON {
                continue;
            }
            state.update(cx, |state, cx| {
                state.set_value(value, window, cx);
            });
        }
    }

    pub(super) fn render_appearance_blend_icon(
        &self,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let color = if active {
            cx.theme().selection
        } else {
            cx.theme().foreground
        };
        render_icon_canvas(color, 16., |path| {
            path.move_to(8., 1.5);
            path.cubic_to((14., 10.), (8.5, 3.5), (14., 6.5));
            path.cubic_to((8., 15.5), (14., 13.5), (11.4, 15.5));
            path.cubic_to((2., 10.), (4.6, 15.5), (2., 13.5));
            path.cubic_to((8., 1.5), (2., 6.5), (7.5, 3.5));
            path.close();
        })
    }

    pub(super) fn render_appearance_blend_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.node.blend_mode;
        let supports_pass_through = self.node.supports_pass_through_blend();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_keyboard = panel.clone();
        let panel_for_content = panel;
        let blend_open = self.appearance_blend_mode_open;
        let trigger = Button::new(SharedString::from(format!(
            "{}-appearance-blend-mode",
            self.id
        )))
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .disabled(!self.property_is_editable(DesignPanelProperty::BlendMode))
        .on_keyboard_activate(move |_, cx| {
            panel_for_keyboard.update(cx, |this, cx| {
                this.appearance_blend_mode_open = !blend_open;
                if !blend_open {
                    this.prepare_paint_picker_for_dismissal(cx);
                    this.cancel_menu_preview(cx);
                    this.active_picker = None;
                    this.type_settings_open = false;
                } else {
                    this.cancel_menu_preview_for_property(DesignPanelProperty::BlendMode, cx);
                }
                cx.notify();
            });
        })
        .child(self.render_appearance_blend_icon(
            self.appearance_blend_mode_open
                || !matches!(
                    current,
                    DesignBlendMode::Normal | DesignBlendMode::PassThrough
                ),
            cx,
        ));

        Popover::new(SharedString::from(format!(
            "{}-appearance-blend-mode-popover",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(self.appearance_blend_mode_open)
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                this.appearance_blend_mode_open = *open;
                if *open {
                    this.prepare_paint_picker_for_dismissal(cx);
                    this.cancel_menu_preview(cx);
                    this.active_picker = None;
                    this.type_settings_open = false;
                } else {
                    this.cancel_menu_preview_for_property(DesignPanelProperty::BlendMode, cx);
                }
                cx.notify();
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            v_flex()
                .w(popup_width(window, 176.))
                .max_h(popup_height(window, 336.))
                .overflow_y_scrollbar()
                .gap_1()
                .children(
                    DesignBlendMode::ALL
                        .into_iter()
                        .filter(move |mode| {
                            supports_pass_through || *mode != DesignBlendMode::PassThrough
                        })
                        .map(|mode| {
                            let panel = panel_for_content.clone();
                            let panel_for_hover = panel.clone();
                            let panel_for_key = panel.clone();
                            let popover = popover.clone();
                            Button::new(SharedString::from(format!(
                                "appearance-blend-mode-{}",
                                mode.label().to_lowercase().replace(' ', "-")
                            )))
                            .label(mode.label())
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(mode == current)
                            .on_hover(move |hovered, _, cx| {
                                panel_for_hover.update(cx, |this, cx| {
                                    this.set_property_menu_preview(
                                        DesignPanelProperty::BlendMode,
                                        DesignPanelValue::BlendMode(mode),
                                        *hovered,
                                        cx,
                                    );
                                });
                            })
                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                panel_for_key.update(cx, |this, cx| {
                                    this.set_property_menu_preview(
                                        DesignPanelProperty::BlendMode,
                                        DesignPanelValue::BlendMode(mode),
                                        true,
                                        cx,
                                    );
                                    match event.keystroke.key.as_str() {
                                        "enter" | "space" => {
                                            this.cancel_menu_preview(cx);
                                            this.appearance_blend_mode_open = false;
                                            this.emit_property(
                                                DesignPanelProperty::BlendMode,
                                                DesignPanelValue::BlendMode(mode),
                                                cx,
                                            );
                                            window.prevent_default();
                                            cx.stop_propagation();
                                        }
                                        "escape" => {
                                            this.cancel_menu_preview(cx);
                                            this.appearance_blend_mode_open = false;
                                            window.prevent_default();
                                            cx.stop_propagation();
                                        }
                                        "tab" => this.cancel_menu_preview(cx),
                                        _ => {}
                                    }
                                });
                            })
                            .on_activate(move |_, window, cx| {
                                panel.update(cx, |this, cx| {
                                    this.cancel_menu_preview(cx);
                                    this.emit_property(
                                        DesignPanelProperty::BlendMode,
                                        DesignPanelValue::BlendMode(mode),
                                        cx,
                                    );
                                });
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        }),
                )
        })
        .into_any_element()
    }

    pub(super) fn render_appearance_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let section = DesignPanelSection::Layer;
        let id = SharedString::from(format!("{}-section-appearance", self.id));
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
            .child(
                h_flex()
                    .id(SharedString::from(format!(
                        "{}-appearance-header-actions",
                        self.id
                    )))
                    .gap_1()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .when_some(self.render_variable_mode_popover(cx), |actions, browser| {
                        actions.child(browser)
                    })
                    .when(self.node.supports_visibility(), |actions| {
                        actions.child(self.render_visibility_button(
                            "appearance-visible",
                            self.node.visible,
                            DesignPanelProperty::Visible,
                            cx,
                        ))
                    })
                    .when(self.node.supports_layer_appearance(), |actions| {
                        actions.child(self.render_appearance_blend_popover(cx))
                    }),
            )
            .into_any_element()
    }

    pub(super) fn appearance_corner_details_are_visible(&self) -> bool {
        let first = self.node.corner_radii[0];
        self.appearance_corner_details_open
            || self.node.independent_corners
            || self.node.corner_smoothing.abs() > f32::EPSILON
            || self
                .node
                .corner_radii
                .iter()
                .skip(1)
                .any(|radius| (*radius - first).abs() > f32::EPSILON)
    }

    pub(super) fn render_corner_details_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.can_edit();
        let selected = self.appearance_corner_details_are_visible();
        let mut button = div()
            .id(SharedString::from(format!(
                "{}-appearance-corner-details",
                self.id
            )))
            .size(px(ROW_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .when(selected, |button| {
                button
                    .bg(cx.theme().selection.opacity(0.22))
                    .text_color(cx.theme().selection)
            })
            .when(enabled, |button| {
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
            })
            .when(!enabled, |button| button.opacity(0.38));
        if enabled {
            button = button.on_activate(cx.listener(|this, _, _, cx| {
                this.appearance_corner_details_open = !this.appearance_corner_details_open;
                cx.notify();
            }));
        }
        button
            .child(self.render_value_field_icon(ValueFieldIcon::Corners, cx))
            .into_any_element()
    }

    pub(super) fn render_visibility_button(
        &self,
        id_suffix: impl Into<SharedString>,
        fallback_visible: bool,
        property: DesignPanelProperty,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let visibility = self.visibility_control_state(property, fallback_visible);
        let next_visible = visibility.activation_value();
        let editable = self.property_is_editable(property);
        let icon = match visibility {
            VisibilityControlState::Visible => IconName::Eye,
            VisibilityControlState::Hidden => IconName::EyeOff,
            VisibilityControlState::Mixed => IconName::Minus,
        };
        let tooltip = SharedString::from(visibility.tooltip(editable));
        let selector = format!("{}-{}", self.id, id_suffix);
        let debug_selector = selector.clone();
        let mut button = div()
            .id(SharedString::from(selector))
            .debug_selector(move || debug_selector.clone())
            .size(px(24.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .text_color(match visibility {
                VisibilityControlState::Visible => cx.theme().foreground,
                VisibilityControlState::Hidden => cx.theme().muted_foreground,
                VisibilityControlState::Mixed => cx.theme().selection,
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .when(visibility == VisibilityControlState::Mixed, |button| {
                button.bg(cx.theme().selection.opacity(0.14))
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
            })
            .when(!editable, |button| button.opacity(0.62));
        if editable {
            button = button.on_activate(cx.listener(move |this, _, _, cx| {
                this.emit_property(property, DesignPanelValue::Bool(next_visible), cx);
            }));
        }
        h_flex()
            .h(px(24.))
            .gap_0p5()
            .when_some(
                self.render_property_variable_button(property, cx),
                |controls, variable| controls.child(variable),
            )
            .child(button.child(Icon::new(icon).xsmall()))
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_draw_appearance_slider_row(
        &self,
        id_suffix: &'static str,
        label: &'static str,
        icon: ValueFieldIcon,
        property: DesignPanelProperty,
        displayed_value: SharedString,
        next: DesignPanelValue,
        slider: Entity<SliderState>,
        show_slider: bool,
        trailing: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.renders_draw_workspace()
            && self.draw_appearance_slider_range(property).is_some()
            && self.numeric_scrub_seed(property).is_some()
            && (self.property_editor.is_none()
                || self.draw_appearance_slider_property == Some(property));
        let panel = cx.entity();
        let panel_for_up = panel.clone();
        let panel_for_key = panel.clone();
        let selector = SharedString::from(format!("{}-draw-{id_suffix}-slider", self.id));
        let debug_selector = selector.to_string();
        let slider_element = div()
            .id(selector)
            .debug_selector(move || debug_selector.clone())
            .flex_1()
            .min_w(px(0.))
            .h(px(24.))
            .when(enabled, |track| {
                track
                    .key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        let direction = match event.keystroke.key.as_str() {
                            "left" | "down" => -1.,
                            "right" | "up" => 1.,
                            _ => return,
                        };
                        let handled = panel_for_key.update(cx, |this, cx| {
                            this.step_draw_appearance_slider(property, direction, cx)
                        });
                        if handled {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    })
            })
            .when(!enabled, |track| track.opacity(0.52))
            .on_mouse_up(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                panel_for_up.update(cx, |this, cx| {
                    this.finish_draw_appearance_slider(property, true, cx);
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_: &MouseUpEvent, _, cx| {
                panel.update(cx, |this, cx| {
                    this.finish_draw_appearance_slider(property, true, cx);
                });
            })
            .child(Slider::new(&slider).horizontal().disabled(!enabled));

        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .w(px(96.))
                            .flex_none()
                            .child(self.render_icon_value_cell(
                                id_suffix,
                                icon,
                                displayed_value,
                                property,
                                next,
                                cx,
                            )),
                    )
                    .when(show_slider, |row| row.child(slider_element))
                    .when(!show_slider, |row| row.child(div().flex_1().min_w(px(0.))))
                    .when_some(trailing, |row, action| row.child(action)),
            )
            .into_any_element()
    }

    pub(super) fn render_shape_appearance_controls(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let controls = match self.node.shape_geometry {
            DesignShapeGeometry::Polygon(geometry) => v_flex()
                .w_full()
                .gap_2()
                .child(self.render_value_cell(
                    "polygon-count",
                    "△",
                    geometry.point_count.to_string(),
                    DesignPanelProperty::PolygonCount,
                    DesignPanelValue::Integer(i64::from(
                        geometry.point_count.saturating_add(1).min(60),
                    )),
                    cx,
                ))
                .into_any_element(),
            DesignShapeGeometry::Star(geometry) => h_flex()
                .w_full()
                .gap_2()
                .child(self.render_value_cell(
                    "star-points",
                    "☆",
                    geometry.point_count.to_string(),
                    DesignPanelProperty::StarPointCount,
                    DesignPanelValue::Integer(i64::from(
                        geometry.point_count.saturating_add(1).min(60),
                    )),
                    cx,
                ))
                .child(self.render_value_cell(
                    "star-inner-radius",
                    "%",
                    format!("{}%", format_number(geometry.inner_radius * 100.)),
                    DesignPanelProperty::StarInnerRadius,
                    DesignPanelValue::Ratio((geometry.inner_radius + 0.05).min(1.)),
                    cx,
                ))
                .into_any_element(),
            DesignShapeGeometry::Ellipse(arc) if arc.has_inspector_controls() => v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(self.render_value_cell(
                            "arc-start",
                            "↻",
                            format!("{}°", format_number(arc.starting_degrees())),
                            DesignPanelProperty::ArcStartingAngle,
                            DesignPanelValue::AngleRadians(
                                arc.starting_angle + 15_f32.to_radians(),
                            ),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "arc-sweep",
                            "◔",
                            format!("{}°", format_number(arc.sweep_degrees())),
                            DesignPanelProperty::ArcSweep,
                            DesignPanelValue::AngleRadians(arc.sweep_angle() + 15_f32.to_radians()),
                            cx,
                        )),
                )
                .child(self.render_value_cell(
                    "arc-ratio",
                    "○",
                    format!("{}%", format_number(arc.inner_radius * 100.)),
                    DesignPanelProperty::ArcInnerRadius,
                    DesignPanelValue::Ratio((arc.inner_radius + 0.1).min(1.)),
                    cx,
                ))
                .into_any_element(),
            DesignShapeGeometry::None
            | DesignShapeGeometry::Ellipse(_)
            | DesignShapeGeometry::Boolean(_)
            | DesignShapeGeometry::Table(_) => return None,
        };
        Some(controls)
    }

    pub(super) fn render_draw_layer(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = v_flex()
            .pl(px(PANEL_PADDING))
            .pr_2()
            .pt(px(2.))
            .pb_4()
            .gap_2();
        let corner_capabilities = self.node.corner_capabilities;
        if self.node.supports_visibility() && !self.node.supports_layer_appearance() {
            content = content.child(self.render_toggle_row(
                "draw-appearance-visible",
                "Visible",
                self.node.visible,
                DesignPanelProperty::Visible,
                cx,
            ));
        }
        if self.node.supports_layer_appearance() {
            content = content.child(self.render_draw_appearance_slider_row(
                "opacity",
                "Opacity",
                ValueFieldIcon::Opacity,
                DesignPanelProperty::Opacity,
                format!("{}%", format_number(self.node.opacity)).into(),
                DesignPanelValue::Number(if self.node.opacity <= 20. {
                    100.
                } else {
                    self.node.opacity - 10.
                }),
                self.draw_opacity_slider.clone(),
                true,
                Some(self.render_visibility_button(
                    "draw-appearance-visible",
                    self.node.visible,
                    DesignPanelProperty::Visible,
                    cx,
                )),
                cx,
            ));
        }
        if corner_capabilities.uniform_radius {
            content = content.child(
                self.render_draw_appearance_slider_row(
                    "corner-radius",
                    "Corner radius",
                    ValueFieldIcon::Corners,
                    DesignPanelProperty::CornerRadius,
                    format_number(self.node.corner_radii[0]).into(),
                    DesignPanelValue::Number(self.node.corner_radii[0] + 4.),
                    self.draw_corner_radius_slider.clone(),
                    self.draw_appearance_slider_range(DesignPanelProperty::CornerRadius)
                        .is_some(),
                    corner_capabilities
                        .independent_radii
                        .then(|| self.render_corner_details_button(cx)),
                    cx,
                ),
            );
        }

        let corner_details_visible = self.appearance_corner_details_are_visible();
        if corner_details_visible && corner_capabilities.independent_radii {
            content = content.child(self.render_group_label("Corners", cx)).child(
                self.render_value_cell(
                    "draw-independent-corners",
                    "⁙",
                    if self.node.independent_corners {
                        "Individual"
                    } else {
                        "All corners"
                    },
                    DesignPanelProperty::IndependentCorners,
                    DesignPanelValue::Bool(!self.node.independent_corners),
                    cx,
                ),
            );
        }
        if corner_details_visible && corner_capabilities.smoothing {
            let smoothing = self.node.corner_smoothing.clamp(0., 1.);
            content = content
                .child(self.render_group_label("Corner smoothing", cx))
                .child(self.render_value_cell(
                    "draw-corner-smoothing",
                    "⌁",
                    format!("{}%", format_number(smoothing * 100.)),
                    DesignPanelProperty::CornerSmoothing,
                    DesignPanelValue::Ratio((smoothing + 0.1).min(1.)),
                    cx,
                ));
        }
        if corner_details_visible
            && corner_capabilities.independent_radii
            && self.node.independent_corners
        {
            content = content
                .child(self.render_group_label("Corners", cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "draw-corner-top-left",
                            "⌜",
                            format_number(self.node.corner_radii[0]),
                            DesignPanelProperty::CornerRadiusTopLeft,
                            DesignPanelValue::Number(self.node.corner_radii[0] + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "draw-corner-top-right",
                            "⌝",
                            format_number(self.node.corner_radii[1]),
                            DesignPanelProperty::CornerRadiusTopRight,
                            DesignPanelValue::Number(self.node.corner_radii[1] + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "draw-corner-bottom-left",
                            "⌞",
                            format_number(self.node.corner_radii[3]),
                            DesignPanelProperty::CornerRadiusBottomLeft,
                            DesignPanelValue::Number(self.node.corner_radii[3] + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "draw-corner-bottom-right",
                            "⌟",
                            format_number(self.node.corner_radii[2]),
                            DesignPanelProperty::CornerRadiusBottomRight,
                            DesignPanelValue::Number(self.node.corner_radii[2] + 1.),
                            cx,
                        )),
                );
        }
        if let Some(shape_controls) = self.render_shape_appearance_controls(cx) {
            content = content.child(shape_controls);
        }
        if self.node.supports_layer_appearance() {
            let next_blend = if self.node.blend_mode == DesignBlendMode::Normal {
                DesignBlendMode::Multiply
            } else {
                DesignBlendMode::Normal
            };
            content = content
                .child(self.render_group_label("Blend mode", cx))
                .child(self.render_value_cell(
                    "draw-blend-mode",
                    "◇",
                    self.node.blend_mode.label(),
                    DesignPanelProperty::BlendMode,
                    DesignPanelValue::BlendMode(next_blend),
                    cx,
                ));
        }

        let expanded = self.expanded_sections.contains(&DesignPanelSection::Layer);
        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(self.render_section_header(DesignPanelSection::Layer, None, cx))
            .when(expanded, |section| {
                section.child(content.into_any_element())
            })
            .into_any_element()
    }

    pub(super) fn render_layer(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.renders_draw_workspace() {
            return self.render_draw_layer(cx);
        }
        let mut content = v_flex()
            .pl(px(PANEL_PADDING))
            .pr_2()
            .pt(px(2.))
            .pb_4()
            .gap_2();
        if let Some(applied) = self.render_applied_component_property_controls(
            DesignComponentPropertyApplicationSurface::Appearance,
            cx,
        ) {
            content = content.child(applied);
        }
        let corner_capabilities = self.node.corner_capabilities;
        if self.node.supports_layer_appearance() || corner_capabilities.uniform_radius {
            let mut summary = h_flex().w_full().items_end().gap_2();
            if self.node.supports_layer_appearance() {
                summary = summary.child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap_1()
                        .child(self.render_group_label("Opacity", cx))
                        .child(self.render_icon_value_cell(
                            "opacity",
                            ValueFieldIcon::Opacity,
                            format!("{}%", format_number(self.node.opacity)),
                            DesignPanelProperty::Opacity,
                            DesignPanelValue::Number(if self.node.opacity <= 20. {
                                100.
                            } else {
                                self.node.opacity - 10.
                            }),
                            cx,
                        )),
                );
            }
            if corner_capabilities.uniform_radius {
                summary = summary.child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap_1()
                        .child(self.render_group_label("Corner radius", cx))
                        .child(self.render_icon_value_cell(
                            "corner-radius",
                            ValueFieldIcon::Corners,
                            format_number(self.node.corner_radii[0]),
                            DesignPanelProperty::CornerRadius,
                            DesignPanelValue::Number(self.node.corner_radii[0] + 4.),
                            cx,
                        )),
                );
                summary = if corner_capabilities.independent_radii {
                    summary.child(self.render_corner_details_button(cx))
                } else {
                    summary.child(div().size(px(24.)).flex_none())
                };
            }
            content = content.child(summary);
        }

        let corner_details_visible = self.appearance_corner_details_are_visible();
        if corner_details_visible && corner_capabilities.independent_radii {
            content = content.child(self.render_group_label("Corners", cx)).child(
                self.render_value_cell(
                    "independent-corners",
                    "⁙",
                    if self.node.independent_corners {
                        "Individual"
                    } else {
                        "All corners"
                    },
                    DesignPanelProperty::IndependentCorners,
                    DesignPanelValue::Bool(!self.node.independent_corners),
                    cx,
                ),
            );
        }
        if corner_details_visible && corner_capabilities.smoothing {
            let smoothing = self.node.corner_smoothing.clamp(0., 1.);
            content = content
                .child(self.render_group_label("Corner smoothing", cx))
                .child(self.render_value_cell(
                    "corner-smoothing",
                    "⌁",
                    format!("{}%", format_number(smoothing * 100.)),
                    DesignPanelProperty::CornerSmoothing,
                    DesignPanelValue::Ratio((smoothing + 0.1).min(1.)),
                    cx,
                ));
        }
        if corner_details_visible
            && corner_capabilities.independent_radii
            && self.node.independent_corners
        {
            content = content
                .child(self.render_group_label("Corners", cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "corner-top-left",
                            "⌜",
                            format_number(self.node.corner_radii[0]),
                            DesignPanelProperty::CornerRadiusTopLeft,
                            DesignPanelValue::Number(self.node.corner_radii[0] + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "corner-top-right",
                            "⌝",
                            format_number(self.node.corner_radii[1]),
                            DesignPanelProperty::CornerRadiusTopRight,
                            DesignPanelValue::Number(self.node.corner_radii[1] + 1.),
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(self.render_value_cell(
                            "corner-bottom-left",
                            "⌞",
                            format_number(self.node.corner_radii[3]),
                            DesignPanelProperty::CornerRadiusBottomLeft,
                            DesignPanelValue::Number(self.node.corner_radii[3] + 1.),
                            cx,
                        ))
                        .child(self.render_value_cell(
                            "corner-bottom-right",
                            "⌟",
                            format_number(self.node.corner_radii[2]),
                            DesignPanelProperty::CornerRadiusBottomRight,
                            DesignPanelValue::Number(self.node.corner_radii[2] + 1.),
                            cx,
                        )),
                );
        }
        if let Some(shape_controls) = self.render_shape_appearance_controls(cx) {
            content = content.child(shape_controls);
        }
        let expanded = self.expanded_sections.contains(&DesignPanelSection::Layer);
        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(self.render_appearance_header(cx))
            .when(expanded, |section| {
                section.child(content.into_any_element())
            })
            .into_any_element()
    }

    pub(super) fn render_mask(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.node.supports_section(DesignPanelSection::Mask) {
            return None;
        }
        let mask_type = self.node.effective_mask_type()?;
        let next_mask_type = match mask_type {
            DesignMaskType::Alpha => DesignMaskType::Vector,
            DesignMaskType::Vector => DesignMaskType::Luminance,
            DesignMaskType::Luminance => DesignMaskType::Alpha,
        };
        let content = v_flex()
            .px(px(PANEL_PADDING))
            .pb_4()
            .gap_2()
            .child(self.render_value_cell(
                "mask-type",
                "◐",
                mask_type.label(),
                DesignPanelProperty::MaskType,
                DesignPanelValue::MaskType(next_mask_type),
                cx,
            ));
        Some(self.render_section(
            DesignPanelSection::Mask,
            None,
            content.into_any_element(),
            cx,
        ))
    }

    pub(super) fn draw_appearance_view_data_for_context(
        &self,
    ) -> Option<&DesignDrawAppearanceViewData> {
        let view_data = self.draw_appearance_view_data.as_ref()?;
        (view_data.is_valid()
            && view_data.target == self.command_target()
            && !self.inspection_context.selection().is_empty()
            && self.multiple_selection_has_uniform_draw_geometry()
            && self.node.supports_section(DesignPanelSection::Layer))
        .then_some(view_data)
    }

    pub(super) fn multiple_selection_has_uniform_draw_geometry(&self) -> bool {
        if self.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple {
            return true;
        }

        [DesignPanelProperty::Width, DesignPanelProperty::Height]
            .into_iter()
            .all(|property| {
                self.property_value_states
                    .get(&property)
                    .and_then(DesignPanelPropertyValueState::resolved)
                    .is_some_and(|value| {
                        matches!(value, DesignPanelValue::Number(value) if value.is_finite())
                    })
            })
    }
}

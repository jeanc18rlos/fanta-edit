use super::*;

impl DesignPanel {
    pub(super) fn can_export(&self) -> bool {
        self.inspection_context.permissions().can_export()
    }

    pub(super) fn export_target(&self) -> DesignPanelTarget {
        self.export_view_data
            .as_ref()
            .filter(|view_data| view_data.target == self.command_target())
            .map_or_else(
                || self.command_target(),
                |view_data| view_data.target.clone(),
            )
    }

    pub(super) fn active_export_view_data(&self) -> Option<&DesignExportViewData> {
        self.export_view_data
            .as_ref()
            .filter(|view_data| view_data.target == self.command_target())
    }

    pub(super) fn export_mode(&self) -> DesignExportMode {
        self.active_export_view_data()
            .map_or(DesignExportMode::Static, |view_data| view_data.mode)
    }

    pub(super) fn export_configurations(&self) -> Vec<DesignExportConfiguration> {
        self.active_export_view_data()
            .map(|view_data| view_data.configurations.clone())
            .unwrap_or_else(|| {
                self.node
                    .export_settings
                    .iter()
                    .enumerate()
                    .map(|(index, setting)| {
                        DesignExportConfiguration::from_legacy(
                            SharedString::from(format!("{}-legacy-export-{index}", self.node.id)),
                            setting,
                        )
                    })
                    .collect()
            })
    }

    pub(super) fn export_configuration(&self, index: usize) -> Option<DesignExportConfiguration> {
        self.export_configurations().into_iter().nth(index)
    }

    pub(super) fn export_property_index(property: DesignPanelProperty) -> Option<usize> {
        match property {
            DesignPanelProperty::ExportSizing(index)
            | DesignPanelProperty::ExportScale(index)
            | DesignPanelProperty::ExportSuffix(index)
            | DesignPanelProperty::ExportFormat(index) => Some(index),
            _ => None,
        }
    }

    pub(super) fn export_property_with_index(
        property: DesignPanelProperty,
        index: usize,
    ) -> DesignPanelProperty {
        match property {
            DesignPanelProperty::ExportSizing(_) => DesignPanelProperty::ExportSizing(index),
            DesignPanelProperty::ExportScale(_) => DesignPanelProperty::ExportScale(index),
            DesignPanelProperty::ExportSuffix(_) => DesignPanelProperty::ExportSuffix(index),
            DesignPanelProperty::ExportFormat(_) => DesignPanelProperty::ExportFormat(index),
            property => property,
        }
    }

    pub(super) fn reconcile_export_property_editor(
        &mut self,
        next_view_data: &DesignExportViewData,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.property_editor.as_ref() else {
            return;
        };
        let Some(configuration_id) = editor.export_configuration_id.clone() else {
            return;
        };
        let next_index = (next_view_data.target == self.command_target())
            .then(|| {
                next_view_data
                    .configurations
                    .iter()
                    .position(|configuration| configuration.id == configuration_id)
            })
            .flatten();
        let Some(next_index) = next_index else {
            self.cancel_property_editor_transaction(cx);
            return;
        };
        let next_property = Self::export_property_with_index(editor.property, next_index);
        let previous_view_data = self.export_view_data.replace(next_view_data.clone());
        let next_property_is_editable = self.current_property_value(next_property).is_some()
            && self.property_is_editable(next_property);
        self.export_view_data = previous_view_data;
        if !next_property_is_editable {
            self.cancel_property_editor_transaction(cx);
            return;
        }
        if let Some(editor) = self.property_editor.as_mut() {
            editor.property = next_property;
        }
        if let Some(scrub) = self
            .numeric_property_scrub
            .as_mut()
            .filter(|scrub| scrub.active)
        {
            scrub.property = next_property;
        }
    }

    pub(super) fn export_preview_available(&self) -> bool {
        self.inspection_context.selection().kind() != DesignPanelSelectionKind::Multiple
            && self.active_export_view_data().is_some_and(|view_data| {
                view_data.mode == DesignExportMode::Static && view_data.preview.is_some()
            })
    }

    pub(super) fn export_change_for_property(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
    ) -> Option<(SharedString, DesignExportConfigurationChange)> {
        let (index, change) = match (property, value) {
            (DesignPanelProperty::ExportSizing(index), DesignPanelValue::ExportSizing(sizing)) => {
                (index, DesignExportConfigurationChange::Sizing(sizing))
            }
            (DesignPanelProperty::ExportScale(index), DesignPanelValue::Number(scale)) => (
                index,
                DesignExportConfigurationChange::Sizing(DesignExportSizing::Scale(scale)),
            ),
            (DesignPanelProperty::ExportSuffix(index), DesignPanelValue::Text(suffix)) => {
                (index, DesignExportConfigurationChange::Suffix(suffix))
            }
            (DesignPanelProperty::ExportFormat(index), DesignPanelValue::ExportFormat(format)) => {
                (index, DesignExportConfigurationChange::Format(format))
            }
            _ => return None,
        };
        let configuration_id = self
            .property_editor
            .as_ref()
            .filter(|editor| editor.property == property)
            .and_then(|editor| editor.export_configuration_id.clone())
            .or_else(|| {
                self.export_configuration(index)
                    .map(|configuration| configuration.id)
            })?;
        Some((configuration_id, change))
    }

    pub(super) fn emit_export_configuration_change(
        &mut self,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        phase: DesignPanelEditPhase,
        cx: &mut Context<Self>,
    ) {
        if !self.collection_is_supported(DesignPanelCollection::Export) || !self.can_export() {
            return;
        }
        let Some(configuration) = self
            .export_configurations()
            .into_iter()
            .find(|configuration| configuration.id == configuration_id)
        else {
            return;
        };
        let capabilities = self
            .active_export_view_data()
            .map_or(Default::default(), |view_data| {
                view_data.static_capabilities
            });
        let applicable = match &change {
            DesignExportConfigurationChange::Sizing(_) => {
                configuration.format().supports_custom_sizing()
            }
            DesignExportConfigurationChange::Suffix(_)
            | DesignExportConfigurationChange::Format(_)
            | DesignExportConfigurationChange::ColorProfile(_) => true,
            DesignExportConfigurationChange::IgnoreOverlappingLayers(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Png(_)
                    | DesignExportFormatSettings::Jpg(_)
                    | DesignExportFormatSettings::Svg(_)
            ),
            DesignExportConfigurationChange::IncludeTextBoundingBox(_) => {
                capabilities.can_include_text_bounding_box
                    && matches!(
                        configuration.format_settings,
                        DesignExportFormatSettings::Png(_) | DesignExportFormatSettings::Jpg(_)
                    )
            }
            DesignExportConfigurationChange::Resampling(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Png(_)
                    | DesignExportFormatSettings::Jpg(_)
                    | DesignExportFormatSettings::Pdf(_)
            ),
            DesignExportConfigurationChange::Quality(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Jpg(_) | DesignExportFormatSettings::Pdf(_)
            ),
            DesignExportConfigurationChange::IncludeBounds(_) => {
                capabilities.can_include_svg_bounds
                    && matches!(
                        configuration.format_settings,
                        DesignExportFormatSettings::Svg(_)
                    )
            }
            DesignExportConfigurationChange::IncludeIdAttribute(_)
            | DesignExportConfigurationChange::OutlineText(_)
            | DesignExportConfigurationChange::SimplifyStroke(_) => matches!(
                configuration.format_settings,
                DesignExportFormatSettings::Svg(_)
            ),
        };
        if !self.can_export() || !applicable {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ExportConfigurationChangeRequested {
                target: self.export_target(),
                configuration_id,
                change,
                phase,
            },
        );
    }

    pub(super) fn emit_export_all(&mut self, cx: &mut Context<Self>) {
        if self.can_export()
            && self.collection_is_supported(DesignPanelCollection::Export)
            && !self.export_configurations().is_empty()
        {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ExportAllRequested {
                    target: self.export_target(),
                },
            );
        }
    }

    pub(super) fn emit_export_mode_change(
        &mut self,
        mode: DesignExportMode,
        cx: &mut Context<Self>,
    ) {
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || self
                .active_export_view_data()
                .is_none_or(|view_data| view_data.animated.is_none())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::ExportModeChangeRequested {
                target: self.export_target(),
                mode,
            },
        );
    }

    pub(super) fn emit_animated_export_change(
        &mut self,
        change: DesignAnimatedExportChange,
        cx: &mut Context<Self>,
    ) {
        let Some(view_data) = self.active_export_view_data() else {
            return;
        };
        let Some(animated) = &view_data.animated else {
            return;
        };
        let mut candidate = animated.settings.clone();
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || view_data.mode != DesignExportMode::Animated
        {
            return;
        }
        if let DesignAnimatedExportChange::Format(format) = &change
            && !animated.capability.available_formats.contains(format)
        {
            return;
        }
        if !candidate.apply_change(change.clone()) {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::AnimatedExportChangeRequested {
                target: self.export_target(),
                change,
                phase: DesignPanelEditPhase::Commit,
            },
        );
    }

    pub(super) fn emit_animated_export(&mut self, cx: &mut Context<Self>) {
        let Some(view_data) = self.active_export_view_data() else {
            return;
        };
        let Some(animated) = &view_data.animated else {
            return;
        };
        if !self.can_export()
            || !self.collection_is_supported(DesignPanelCollection::Export)
            || !animated.capability.allows(&animated.settings)
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::AnimatedExportRequested {
                target: self.export_target(),
                settings: animated.settings.clone(),
            },
        );
    }

    pub(super) fn toggle_export_advanced(
        &mut self,
        configuration_id: SharedString,
        cx: &mut Context<Self>,
    ) {
        if !self.expanded_export_settings.remove(&configuration_id) {
            self.expanded_export_settings.insert(configuration_id);
        }
        cx.notify();
    }

    pub(super) fn set_export_choice_overlay(
        &mut self,
        overlay: SharedString,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        self.export_choice_overlay = open.then_some(overlay);
        cx.notify();
    }

    pub(super) fn toggle_export_preview(&mut self, cx: &mut Context<Self>) {
        if !self.export_preview_available() {
            return;
        }
        self.export_preview_expanded = !self.export_preview_expanded;
        if self.export_preview_expanded {
            cx.emit_design_panel_action(
                self,
                DesignPanelAction::ExportPreviewRequested {
                    target: self.export_target(),
                },
            );
        }
        cx.notify();
    }

    pub(super) fn render_export_toggle(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        checked: bool,
        configuration_id: SharedString,
        change: DesignExportConfigurationChange,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_export();
        h_flex()
            .id(SharedString::from(format!(
                "{}-{}",
                self.id,
                id_suffix.into()
            )))
            .h(px(ROW_HEIGHT))
            .w_full()
            .justify_between()
            .rounded(px(5.))
            .when(enabled, |row| {
                row.key_context(CONTROL_KEY_CONTEXT)
                    .tab_index(0)
                    .cursor_pointer()
                    .hover(|style| style.bg(cx.theme().accent.opacity(0.55)))
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_1()
                            .border_color(cx.theme().selection)
                    })
            })
            .when(!enabled, |row| {
                row.text_color(cx.theme().muted_foreground).opacity(0.62)
            })
            .when(enabled, |row| {
                row.on_activate(cx.listener(move |this, _, _, cx| {
                    this.emit_export_configuration_change(
                        configuration_id.clone(),
                        change.clone(),
                        DesignPanelEditPhase::Commit,
                        cx,
                    );
                }))
            })
            .child(div().text_xs().child(label))
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

    pub(super) fn render_export_choice(
        &self,
        id_suffix: impl Into<SharedString>,
        label: &'static str,
        value: impl Into<SharedString>,
        configuration_id: SharedString,
        options: Vec<(SharedString, DesignExportConfigurationChange)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.can_export();
        let id_suffix = id_suffix.into();
        let value = value.into();
        let overlay = SharedString::from(format!("export-choice-{id_suffix}"));
        let overlay_for_open = overlay.clone();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let choice_open = self.export_choice_overlay.as_ref() == Some(&overlay);
        let panel_for_keyboard = panel_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .tooltip(label)
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(!enabled || options.is_empty())
            .on_keyboard_activate(move |_, cx| {
                panel_for_keyboard.update(cx, |this, cx| {
                    this.set_export_choice_overlay(overlay_for_keyboard.clone(), !choice_open, cx);
                });
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().text_xs().child(label))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(value)
                            .child(Icon::new(IconName::ChevronRight).xsmall()),
                    ),
            );
        Popover::new(SharedString::from(format!("{}-{id_suffix}-menu", self.id)))
            .anchor(Anchor::TopRight)
            .open(self.export_choice_overlay.as_ref() == Some(&overlay))
            .overlay_closable(true)
            .on_open_change(move |open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    this.set_export_choice_overlay(overlay_for_open.clone(), *open, cx);
                });
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let popover = cx.entity();
                v_flex().w(popup_width(window, 188.)).gap_1().children(
                    options.clone().into_iter().enumerate().map(
                        |(option_index, (option_label, change))| {
                            let panel = panel_for_content.clone();
                            let popover = popover.clone();
                            let configuration_id = configuration_id.clone();
                            Button::new(SharedString::from(format!(
                                "{panel_id}-{id_suffix}-option-{option_index}"
                            )))
                            .label(option_label)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, window, cx| {
                                panel.update(cx, |this, cx| {
                                    this.export_choice_overlay = None;
                                    this.emit_export_configuration_change(
                                        configuration_id.clone(),
                                        change.clone(),
                                        DesignPanelEditPhase::Commit,
                                        cx,
                                    );
                                    cx.notify();
                                });
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        },
                    ),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_export_advanced(
        &self,
        index: usize,
        configuration: &DesignExportConfiguration,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let configuration_id = configuration.id.clone();
        let capabilities = self
            .active_export_view_data()
            .map_or(Default::default(), |view_data| {
                view_data.static_capabilities
            });
        let mut controls = v_flex().w_full().gap_1().pt_1().child(
            self.render_export_choice(
                format!("export-color-profile-{index}"),
                "Color profile",
                configuration.common.color_profile.label(),
                configuration_id.clone(),
                DesignExportColorProfile::ALL
                    .into_iter()
                    .map(|profile| {
                        (
                            profile.label().into(),
                            DesignExportConfigurationChange::ColorProfile(profile),
                        )
                    })
                    .collect(),
                cx,
            ),
        );

        controls = match configuration.format_settings {
            DesignExportFormatSettings::Png(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_text_bounding_box, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-bounds-{index}"),
                        "Include bounding box",
                        settings.include_text_bounding_box,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeTextBoundingBox(
                            !settings.include_text_bounding_box,
                        ),
                        cx,
                    ))
                })
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id,
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
            DesignExportFormatSettings::Jpg(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_text_bounding_box, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-bounds-{index}"),
                        "Include bounding box",
                        settings.include_text_bounding_box,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeTextBoundingBox(
                            !settings.include_text_bounding_box,
                        ),
                        cx,
                    ))
                })
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id.clone(),
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                )
                .child(
                    self.render_export_choice(
                        format!("export-quality-{index}"),
                        "Image quality",
                        settings.quality.label(),
                        configuration_id,
                        DesignExportImageQuality::ALL
                            .into_iter()
                            .map(|quality| {
                                (
                                    quality.label().into(),
                                    DesignExportConfigurationChange::Quality(quality),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
            DesignExportFormatSettings::Svg(settings) => controls
                .child(self.render_export_toggle(
                    format!("export-ignore-overlap-{index}"),
                    "Ignore overlapping layers",
                    settings.ignore_overlapping_layers,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IgnoreOverlappingLayers(
                        !settings.ignore_overlapping_layers,
                    ),
                    cx,
                ))
                .when(capabilities.can_include_svg_bounds, |controls| {
                    controls.child(self.render_export_toggle(
                        format!("export-include-bounds-{index}"),
                        "Include bounding box",
                        settings.include_bounds,
                        configuration_id.clone(),
                        DesignExportConfigurationChange::IncludeBounds(!settings.include_bounds),
                        cx,
                    ))
                })
                .child(self.render_export_toggle(
                    format!("export-include-id-{index}"),
                    "Include \"id\" attribute",
                    settings.include_id_attribute,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::IncludeIdAttribute(
                        !settings.include_id_attribute,
                    ),
                    cx,
                ))
                .child(self.render_export_toggle(
                    format!("export-outline-text-{index}"),
                    "Outline text",
                    settings.outline_text,
                    configuration_id.clone(),
                    DesignExportConfigurationChange::OutlineText(!settings.outline_text),
                    cx,
                ))
                .child(self.render_export_toggle(
                    format!("export-simplify-stroke-{index}"),
                    "Simplify stroke",
                    settings.simplify_stroke,
                    configuration_id,
                    DesignExportConfigurationChange::SimplifyStroke(!settings.simplify_stroke),
                    cx,
                )),
            DesignExportFormatSettings::Pdf(settings) => controls
                .child(
                    self.render_export_choice(
                        format!("export-resampling-{index}"),
                        "Image resampling",
                        settings.resampling.label(),
                        configuration_id.clone(),
                        DesignExportImageResampling::ALL
                            .into_iter()
                            .map(|resampling| {
                                (
                                    resampling.label().into(),
                                    DesignExportConfigurationChange::Resampling(resampling),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                )
                .child(
                    self.render_export_choice(
                        format!("export-quality-{index}"),
                        "Image quality",
                        settings.quality.label(),
                        configuration_id,
                        DesignExportImageQuality::ALL
                            .into_iter()
                            .map(|quality| {
                                (
                                    quality.label().into(),
                                    DesignExportConfigurationChange::Quality(quality),
                                )
                            })
                            .collect(),
                        cx,
                    ),
                ),
        };
        controls.into_any_element()
    }

    pub(super) fn render_export_mode_switch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view_data = self.active_export_view_data()?;
        view_data.animated.as_ref()?;
        let panel = cx.entity();
        Some(
            h_flex()
                .w_full()
                .p(px(2.))
                .gap_1()
                .rounded(px(6.))
                .bg(cx.theme().secondary)
                .children(DesignExportMode::ALL.into_iter().map(|mode| {
                    let panel = panel.clone();
                    Button::new(SharedString::from(format!(
                        "{}-export-mode-{}",
                        self.id,
                        mode.label().to_ascii_lowercase()
                    )))
                    .label(mode.label())
                    .xsmall()
                    .compact()
                    .ghost()
                    .flex_1()
                    .selected(mode == view_data.mode)
                    .disabled(!self.can_export())
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_export_mode_change(mode, cx);
                        });
                    })
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_animated_export_choice(
        &self,
        id_suffix: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
        options: Vec<(SharedString, DesignAnimatedExportChange)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id_suffix = id_suffix.into();
        let label = label.into();
        let value = value.into();
        let overlay = SharedString::from(format!("animated-export-choice-{id_suffix}"));
        let overlay_for_open = overlay.clone();
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let panel_id = self.id.clone();
        let choice_open = self.export_choice_overlay.as_ref() == Some(&overlay);
        let panel_for_keyboard = panel_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!("{}-{id_suffix}", self.id)))
            .tooltip(label.clone())
            .xsmall()
            .compact()
            .ghost()
            .w_full()
            .h(px(ROW_HEIGHT))
            .disabled(!self.can_export() || options.is_empty())
            .on_keyboard_activate(move |_, cx| {
                panel_for_keyboard.update(cx, |this, cx| {
                    this.set_export_choice_overlay(overlay_for_keyboard.clone(), !choice_open, cx);
                });
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().text_xs().child(label))
                    .child(
                        h_flex()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(value)
                            .child(Icon::new(IconName::ChevronRight).xsmall()),
                    ),
            );
        Popover::new(SharedString::from(format!("{}-{id_suffix}-menu", self.id)))
            .anchor(Anchor::TopRight)
            .open(self.export_choice_overlay.as_ref() == Some(&overlay))
            .overlay_closable(true)
            .on_open_change(move |open, _, cx| {
                panel_for_open.update(cx, |this, cx| {
                    this.set_export_choice_overlay(overlay_for_open.clone(), *open, cx);
                });
            })
            .trigger(trigger)
            .content(move |_, window, cx| {
                let popover = cx.entity();
                v_flex().w(popup_width(window, 188.)).gap_1().children(
                    options.clone().into_iter().enumerate().map(
                        |(option_index, (option_label, change))| {
                            let panel = panel_for_content.clone();
                            let popover = popover.clone();
                            Button::new(SharedString::from(format!(
                                "{panel_id}-{id_suffix}-option-{option_index}"
                            )))
                            .label(option_label)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .on_activate(move |_, window, cx| {
                                panel.update(cx, |this, cx| {
                                    this.export_choice_overlay = None;
                                    this.emit_animated_export_change(change.clone(), cx);
                                    cx.notify();
                                });
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        },
                    ),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_animated_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(view_data) = self.active_export_view_data() else {
            return div().into_any_element();
        };
        let Some(animated) = &view_data.animated else {
            return div().into_any_element();
        };
        let settings = &animated.settings;
        let capability = &animated.capability;
        let mut content = v_flex().w_full().gap_2();
        content = content.child(
            self.render_animated_export_choice(
                "animated-export-format",
                "Format",
                settings.format().label(),
                capability
                    .available_formats
                    .iter()
                    .copied()
                    .map(|format| {
                        (
                            format.label().into(),
                            DesignAnimatedExportChange::Format(format),
                        )
                    })
                    .collect(),
                cx,
            ),
        );

        match settings {
            DesignAnimatedExportSettings::Mp4 {
                sizing,
                fps,
                quality,
            }
            | DesignAnimatedExportSettings::WebM {
                sizing,
                fps,
                quality,
            } => {
                content = content
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-size",
                            "Size",
                            sizing.to_string(),
                            [0.5, 0.75, 1., 1.5, 2., 3., 4.]
                                .into_iter()
                                .map(|scale| {
                                    let sizing = DesignExportSizing::Scale(scale);
                                    (
                                        sizing.to_string().into(),
                                        DesignAnimatedExportChange::Sizing(sizing),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-fps",
                            "Frame rate",
                            fps.label(),
                            DesignVideoExportFps::ALL
                                .into_iter()
                                .map(|fps| (fps.label(), DesignAnimatedExportChange::VideoFps(fps)))
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-quality",
                            "Quality",
                            quality.label(),
                            DesignExportImageQuality::ALL
                                .into_iter()
                                .map(|quality| {
                                    (
                                        quality.label().into(),
                                        DesignAnimatedExportChange::Quality(quality),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    );
            }
            DesignAnimatedExportSettings::Gif {
                sizing,
                fps,
                loop_count,
            } => {
                let current_loop_count = *loop_count;
                content = content
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-size",
                            "Size",
                            sizing.to_string(),
                            [0.5, 0.75, 1., 1.5, 2., 3., 4.]
                                .into_iter()
                                .map(|scale| {
                                    let sizing = DesignExportSizing::Scale(scale);
                                    (
                                        sizing.to_string().into(),
                                        DesignAnimatedExportChange::Sizing(sizing),
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        self.render_animated_export_choice(
                            "animated-export-fps",
                            "Frame rate",
                            fps.label(),
                            DesignGifExportFps::ALL
                                .into_iter()
                                .map(|fps| (fps.label(), DesignAnimatedExportChange::GifFps(fps)))
                                .collect(),
                            cx,
                        ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(ROW_HEIGHT))
                            .justify_between()
                            .child(div().text_xs().child("Loop count"))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child({
                                        let panel = cx.entity();
                                        Button::new(SharedString::from(format!(
                                            "{}-animated-loop-decrement",
                                            self.id
                                        )))
                                        .icon(IconName::Minus)
                                        .xsmall()
                                        .compact()
                                        .ghost()
                                        .disabled(!self.can_export() || current_loop_count == 0)
                                        .on_activate(
                                            move |_, _, cx| {
                                                panel.update(cx, |this, cx| {
                                                    this.emit_animated_export_change(
                                                        DesignAnimatedExportChange::GifLoopCount(
                                                            current_loop_count.saturating_sub(1),
                                                        ),
                                                        cx,
                                                    );
                                                });
                                            },
                                        )
                                    })
                                    .child(
                                        div()
                                            .w(px(56.))
                                            .h(px(ROW_HEIGHT))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(4.))
                                            .bg(cx.theme().secondary)
                                            .text_xs()
                                            .child(if current_loop_count == 0 {
                                                SharedString::from("Forever")
                                            } else {
                                                SharedString::from(current_loop_count.to_string())
                                            }),
                                    )
                                    .child({
                                        let panel = cx.entity();
                                        Button::new(SharedString::from(format!(
                                            "{}-animated-loop-increment",
                                            self.id
                                        )))
                                        .icon(IconName::Plus)
                                        .xsmall()
                                        .compact()
                                        .ghost()
                                        .disabled(!self.can_export() || current_loop_count >= 1000)
                                        .on_activate(
                                            move |_, _, cx| {
                                                panel.update(cx, |this, cx| {
                                                    this.emit_animated_export_change(
                                                        DesignAnimatedExportChange::GifLoopCount(
                                                            current_loop_count.saturating_add(1),
                                                        ),
                                                        cx,
                                                    );
                                                });
                                            },
                                        )
                                    }),
                            ),
                    );
            }
            DesignAnimatedExportSettings::Svg { options } => {
                for option in options {
                    content = content.child(
                        self.render_animated_export_choice(
                            SharedString::from(format!("animated-svg-option-{}", option.id)),
                            option.label.clone(),
                            option.selected.clone(),
                            option
                                .choices
                                .iter()
                                .cloned()
                                .map(|value| {
                                    (
                                        value.clone(),
                                        DesignAnimatedExportChange::SvgOption {
                                            option_id: option.id.clone(),
                                            value,
                                        },
                                    )
                                })
                                .collect(),
                            cx,
                        ),
                    );
                }
                if options.is_empty() {
                    content = content.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Animated SVG settings are supplied by the host."),
                    );
                }
            }
        }

        let reason = capability
            .reason_for(settings)
            .map(|reason| SharedString::from(reason.to_owned()));
        let enabled = self.can_export() && reason.is_none();
        content
            .when_some(reason, |content, reason| {
                content.child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded(px(5.))
                        .bg(cx.theme().secondary)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(reason),
                )
            })
            .child({
                let panel = cx.entity();
                Button::new(SharedString::from(format!("{}-animated-export", self.id)))
                    .label(format!(
                        "Export {} as {}",
                        self.node.name,
                        settings.format().label()
                    ))
                    .xsmall()
                    .compact()
                    .w_full()
                    .disabled(!enabled)
                    .on_activate(move |_, _, cx| {
                        panel.update(cx, |this, cx| {
                            this.emit_animated_export(cx);
                        });
                    })
            })
            .into_any_element()
    }

    pub(super) fn render_export_preview_state(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self
            .active_export_view_data()
            .and_then(|view_data| view_data.preview.as_ref());
        match state {
            Some(DesignExportPreviewState::Idle) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Open Preview to request a host-rendered thumbnail.")
                .into_any_element(),
            Some(DesignExportPreviewState::Loading) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Rendering preview…")
                .into_any_element(),
            Some(DesignExportPreviewState::Ready(preview)) => v_flex()
                .px_2()
                .pb_2()
                .gap_1()
                .child(
                    div()
                        .h(px(96.))
                        .w_full()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().secondary)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(preview.thumbnail_id.clone().unwrap_or_else(|| {
                            SharedString::from("Host-rendered export thumbnail")
                        })),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} × {} px",
                            preview.pixel_width, preview.pixel_height
                        ))
                        .when_some(preview.estimated_output.clone(), |row, output| {
                            row.child(output)
                        }),
                )
                .into_any_element(),
            Some(DesignExportPreviewState::Error { message }) => div()
                .px_2()
                .pb_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(message.clone())
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    pub(super) fn render_export(&self, cx: &mut Context<Self>) -> AnyElement {
        let configurations = self.export_configurations();
        let mode_switch = self.render_export_mode_switch(cx);
        if self.export_mode() == DesignExportMode::Animated {
            let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
            if let Some(mode_switch) = mode_switch {
                content = content.child(mode_switch);
            }
            content = content.child(self.render_animated_export(cx));
            return self.render_section(
                DesignPanelSection::Export,
                None,
                content.into_any_element(),
                cx,
            );
        }
        if configurations.is_empty() {
            let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
            if let Some(mode_switch) = mode_switch {
                content = content.child(mode_switch);
            }
            return self.render_section(
                DesignPanelSection::Export,
                Some(DesignPanelCollection::Export),
                content.into_any_element(),
                cx,
            );
        }
        let mut content = v_flex().px(px(PANEL_PADDING)).pb_4().gap_2();
        if let Some(mode_switch) = mode_switch {
            content = content.child(mode_switch);
        }
        for (index, configuration) in configurations.iter().enumerate() {
            let configuration_id = configuration.id.clone();
            let expanded = self.expanded_export_settings.contains(&configuration.id);
            let suffix = if configuration.common.suffix.is_empty() {
                SharedString::from("No suffix")
            } else {
                configuration.common.suffix.clone()
            };
            let mut row = v_flex().w_full().gap_1().child(
                h_flex()
                    .h(px(ROW_HEIGHT))
                    .gap_1()
                    .child(self.render_value_cell(
                        format!("export-sizing-{index}"),
                        "↔",
                        configuration.sizing.to_string(),
                        DesignPanelProperty::ExportSizing(index),
                        DesignPanelValue::ExportSizing(configuration.sizing),
                        cx,
                    ))
                    .child(self.render_value_cell(
                        format!("export-format-{index}"),
                        "▧",
                        configuration.format().label(),
                        DesignPanelProperty::ExportFormat(index),
                        DesignPanelValue::ExportFormat(configuration.format()),
                        cx,
                    ))
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "{}-export-advanced-{index}",
                                self.id
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
                                this.toggle_export_advanced(configuration_id.clone(), cx);
                            }))
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronUp
                                } else {
                                    IconName::Ellipsis
                                })
                                .xsmall(),
                            ),
                    )
                    .child(self.render_remove_button(
                        format!("remove-export-{index}"),
                        DesignPanelCollection::Export,
                        index,
                        cx,
                    )),
            );
            if expanded {
                row = row
                    .child(self.render_value_cell(
                        format!("export-suffix-{index}"),
                        "S",
                        suffix,
                        DesignPanelProperty::ExportSuffix(index),
                        DesignPanelValue::Text(configuration.common.suffix.clone()),
                        cx,
                    ))
                    .child(self.render_export_advanced(index, configuration, cx));
            }
            content = content.child(row);
        }

        let enabled = self.can_export() && !configurations.is_empty();
        content = content.child(
            div()
                .id(SharedString::from(format!("{}-export-all", self.id)))
                .h(px(ROW_HEIGHT))
                .w_full()
                .px_2()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
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
                })
                .when(enabled, |button| {
                    button.on_activate(cx.listener(move |this, _, _, cx| {
                        this.emit_export_all(cx);
                    }))
                })
                .child(format!("Export {}", self.node.name)),
        );

        if self.export_preview_available() {
            content = content.child(
                v_flex()
                    .w_full()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-export-preview", self.id)))
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .px_2()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| {
                                style
                                    .bg(cx.theme().accent)
                                    .border_color(cx.theme().selection)
                            })
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.toggle_export_preview(cx);
                            }))
                            .child(
                                Icon::new(if self.export_preview_expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall(),
                            )
                            .child(div().text_xs().child("Preview")),
                    )
                    .when(self.export_preview_expanded, |preview| {
                        preview.child(self.render_export_preview_state(cx))
                    }),
            );
        }

        self.render_section(
            DesignPanelSection::Export,
            Some(DesignPanelCollection::Export),
            content.into_any_element(),
            cx,
        )
    }
}

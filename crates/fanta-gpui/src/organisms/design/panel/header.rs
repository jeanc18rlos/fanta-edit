use super::*;

impl DesignPanel {
    pub(super) fn request_surface_change(
        &self,
        requested: DesignPanelSurface,
        cx: &mut Context<Self>,
    ) {
        let current = self.active_surface();
        if requested == current
            || !requested.is_available(self.inspection_context.permissions().can_edit())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SurfaceChangeRequested { current, requested },
        );
    }

    pub(super) fn render_tab(
        &self,
        surface: DesignPanelSurface,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active = self.active_surface() == surface;
        let id = SharedString::from(format!("{}-tab-{}", self.id, surface.slug()));
        let selector = id.to_string();
        let active_selector = format!("{selector}-active");
        let panel = cx.entity();
        let button = Button::new(id)
            .debug_selector(move || selector.clone())
            .label(surface.label())
            .tooltip(SharedString::from(format!("Open {}", surface.label())))
            .xsmall()
            .compact()
            .ghost()
            .selected(active)
            .on_activate(move |_, _, cx| {
                panel.update(cx, |this, cx| {
                    this.request_surface_change(surface, cx);
                });
            });
        div()
            .flex_none()
            .when(active, |wrapper| {
                wrapper.debug_selector(move || active_selector.clone())
            })
            .child(button)
            .into_any_element()
    }

    pub(super) fn render_surface_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self
            .available_surfaces()
            .iter()
            .copied()
            .map(|surface| self.render_tab(surface, cx))
            .collect::<Vec<_>>();
        h_flex()
            .h(px(HEADER_HEIGHT))
            .px(px(PANEL_PADDING))
            .gap_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .children(tabs)
            .into_any_element()
    }

    pub(super) fn selection_header_command_enabled_with_access(
        &self,
        host_enabled: bool,
        access: DesignSelectionHeaderCommandAccess,
    ) -> bool {
        host_enabled && (self.can_edit() || !access.requires_edit())
    }

    pub(super) fn selection_header_tooltip(
        &self,
        label: &SharedString,
        host_enabled: bool,
        disabled_reason: Option<&SharedString>,
        access: Option<DesignSelectionHeaderCommandAccess>,
    ) -> SharedString {
        let reason = if !host_enabled {
            disabled_reason.map(SharedString::as_ref)
        } else if access.is_some_and(DesignSelectionHeaderCommandAccess::requires_edit)
            && !self.can_edit()
        {
            Some("View only")
        } else {
            None
        };
        reason
            .map(|reason| format!("{label} — {reason}").into())
            .unwrap_or_else(|| label.clone())
    }

    pub(super) fn emit_selection_header_command_for_target(
        &self,
        expected_target: DesignPanelTarget,
        command: DesignSelectionHeaderCommand,
        access: DesignSelectionHeaderCommandAccess,
        cx: &mut Context<Self>,
    ) {
        let access = match &command {
            DesignSelectionHeaderCommand::TitleMenuItem { .. }
            | DesignSelectionHeaderCommand::HostDefined { .. } => access,
            _ => command.default_access(),
        };
        if self.current_selection_header_target().as_ref() != Some(&expected_target)
            || (access.requires_edit() && !self.can_edit())
        {
            return;
        }
        cx.emit_design_panel_action(
            self,
            DesignPanelAction::SelectionHeaderCommandRequested {
                target: expected_target,
                command,
            },
        );
    }

    pub(super) fn set_selection_header_overlay(
        &mut self,
        overlay: SelectionHeaderOverlay,
        open: bool,
        cx: &mut Context<Self>,
    ) {
        if open {
            self.prepare_paint_picker_for_dismissal(cx);
            self.cancel_menu_preview(cx);
            self.selection_header_overlay = Some(overlay);
            self.active_picker = None;
            self.active_effect_settings = None;
            self.effect_style_browser_open = false;
            self.type_settings_open = false;
            self.type_settings_tab = TypographySettingsTab::Basics;
            self.appearance_blend_mode_open = false;
            self.appearance_corner_details_open = false;
        } else if self.selection_header_overlay.as_ref() == Some(&overlay) {
            self.selection_header_overlay = None;
        }
        cx.notify();
    }

    pub(super) fn render_selection_header_control_icon(
        &self,
        control: &DesignSelectionHeaderControl,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use DesignSelectionHeaderControlIcon as IconPresentation;

        let kind = control.kind;
        if let IconPresentation::Glyph(glyph) = &control.icon {
            return div()
                .w(px(16.))
                .h(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(glyph.clone())
                .into_any_element();
        }
        let named_icon = match &control.icon {
            IconPresentation::Default if kind == DesignSelectionHeaderControlKind::HostDefined => {
                Some(IconName::Ellipsis)
            }
            IconPresentation::Default => None,
            IconPresentation::Ellipsis => Some(IconName::Ellipsis),
            IconPresentation::Plus => Some(IconName::Plus),
            IconPresentation::Minus => Some(IconName::Minus),
            IconPresentation::Check => Some(IconName::Check),
            IconPresentation::Search => Some(IconName::Search),
            IconPresentation::Settings => Some(IconName::Settings2),
            IconPresentation::File => Some(IconName::File),
            IconPresentation::Inspector => Some(IconName::Inspector),
            IconPresentation::Layout => Some(IconName::LayoutDashboard),
            IconPresentation::Glyph(_) => unreachable!(),
        };
        if let Some(icon) = named_icon {
            return Icon::new(icon).xsmall().into_any_element();
        }
        let color = cx.theme().foreground;
        let icon_width = if kind == DesignSelectionHeaderControlKind::BooleanFlattenMenu {
            21.
        } else {
            16.
        };
        render_wide_icon_canvas(color, 16., icon_width, move |path| match kind {
            DesignSelectionHeaderControlKind::SelectMatchingLayers => {
                for (left, top, horizontal, vertical) in [
                    (1.5, 1.5, 4., 4.),
                    (14.5, 1.5, -4., 4.),
                    (1.5, 14.5, 4., -4.),
                    (14.5, 14.5, -4., -4.),
                ] {
                    path.move_to(left + horizontal, top);
                    path.line_to(left, top);
                    path.line_to(left, top + vertical);
                }
                path.diamond(8., 8., 3.);
            }
            DesignSelectionHeaderControlKind::CreateLink => {
                path.poly([(3., 5.), (6., 2.), (10., 6.), (7., 9.)], true);
                path.poly([(6., 10.), (9., 7.), (13., 11.), (10., 14.)], true);
                path.line((6., 10.), (10., 6.));
            }
            DesignSelectionHeaderControlKind::ApplyTextContentVariable => {
                path.line((1.5, 3.), (8., 3.));
                path.line((4.75, 3.), (4.75, 13.));
                path.diamond(11.5, 10.5, 3.5);
            }
            DesignSelectionHeaderControlKind::CreateComponent => {
                for (center_x, center_y) in [(8., 2.8), (3.5, 8.), (12.5, 8.), (8., 13.2)] {
                    path.diamond(center_x, center_y, 1.9);
                }
            }
            DesignSelectionHeaderControlKind::UseAsMask => {
                for center_x in [5.8, 10.2] {
                    path.circle(center_x, 8., 5.2);
                }
            }
            DesignSelectionHeaderControlKind::BooleanFlattenMenu => {
                path.rect(1., 2., 9., 10.);
                path.rect(5., 6., 13., 14.);
                path.poly([(16., 7.), (18.25, 9.25), (20.5, 7.)], false);
            }
            DesignSelectionHeaderControlKind::EditObject => {
                path.rect(3., 3., 13., 13.);
                for (node_x, node_y) in [(3., 3.), (13., 3.), (13., 13.), (3., 13.)] {
                    path.rect(node_x - 1., node_y - 1., node_x + 1., node_y + 1.);
                }
            }
            DesignSelectionHeaderControlKind::HostDefined => unreachable!(),
        })
    }

    pub(super) fn render_selection_header_title(
        &self,
        data: &DesignSelectionHeaderViewData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(menu) = data.title_menu.clone() else {
            return div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_sm()
                .font_semibold()
                .child(data.title.clone())
                .into_any_element();
        };

        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let overlay = SelectionHeaderOverlay::Title;
        let overlay_for_open = overlay.clone();
        let title = data.title.clone();
        let can_edit = self.can_edit();
        let command_target = self.current_selection_header_target();
        let panel_id = self.id.clone();
        let overlay_open = self.selection_header_overlay.as_ref() == Some(&overlay);
        let panel_for_keyboard = panel_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-selection-header-title",
            self.id
        )))
        .label(title)
        .dropdown_caret(true)
        .tooltip("Change layer type")
        .xsmall()
        .compact()
        .ghost()
        .h(px(24.))
        .max_w(px(144.))
        .disabled(menu.items.is_empty())
        .on_keyboard_activate(move |_, cx| {
            panel_for_keyboard.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_keyboard.clone(), !overlay_open, cx);
            });
        });

        let popover = Popover::new(SharedString::from(format!(
            "{}-selection-header-title-menu",
            self.id
        )))
        .anchor(Anchor::TopLeft)
        .open(self.selection_header_overlay.as_ref() == Some(&overlay))
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_open.clone(), *open, cx);
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            v_flex().w(popup_width(window, 184.)).gap_1().children(
                menu.items.clone().into_iter().map(|item| {
                    let panel = panel_for_content.clone();
                    let popover = popover.clone();
                    let command = item.command.clone();
                    let access = item.effective_access();
                    let expected_target = command_target.clone();
                    let enabled = item.enabled
                        && expected_target.is_some()
                        && (can_edit || !access.requires_edit());
                    let tooltip = if enabled {
                        item.label.clone()
                    } else {
                        let reason = item
                            .disabled_reason
                            .as_ref()
                            .map(SharedString::as_ref)
                            .unwrap_or(if access.requires_edit() && !can_edit {
                                "View only"
                            } else {
                                "Unavailable"
                            });
                        format!("{} — {reason}", item.label).into()
                    };
                    Button::new(SharedString::from(format!(
                        "{panel_id}-selection-header-title-item-{}",
                        item.id,
                    )))
                    .label(item.label)
                    .tooltip(tooltip)
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(!enabled)
                    .when(enabled, |button| {
                        button.on_activate(move |_, window, cx| {
                            panel.update(cx, |this, cx| {
                                this.selection_header_overlay = None;
                                if let Some(expected_target) = expected_target.clone() {
                                    this.emit_selection_header_command_for_target(
                                        expected_target,
                                        command.clone(),
                                        access,
                                        cx,
                                    );
                                }
                                cx.notify();
                            });
                            popover.update(cx, |popover, cx| {
                                popover.dismiss(window, cx);
                            });
                        })
                    })
                }),
            )
        });
        div()
            .flex_1()
            .min_w(px(0.))
            .child(popover)
            .into_any_element()
    }

    pub(super) fn render_selection_header_control(
        &self,
        control: DesignSelectionHeaderControl,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !control.is_menu() {
            let command = control
                .command()
                .expect("every direct selected-node header control has a command");
            let access = control.effective_access();
            let expected_target = self.current_selection_header_target();
            let enabled = expected_target.is_some()
                && self.selection_header_command_enabled_with_access(control.enabled, access);
            let tooltip = self.selection_header_tooltip(
                &control.tooltip,
                control.enabled,
                control.disabled_reason.as_ref(),
                Some(access),
            );
            let debug_selector = tooltip.clone();
            let panel = cx.entity();
            return Button::new(SharedString::from(format!(
                "{}-selection-header-control-{}",
                self.id, control.id
            )))
            .debug_selector(move || debug_selector.to_string())
            .tooltip(tooltip)
            .xsmall()
            .compact()
            .ghost()
            .w(px(24.))
            .h(px(24.))
            .disabled(!enabled)
            .child(self.render_selection_header_control_icon(&control, cx))
            .when(enabled, |button| {
                button.on_activate(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        if let Some(expected_target) = expected_target.clone() {
                            this.emit_selection_header_command_for_target(
                                expected_target,
                                command.clone(),
                                access,
                                cx,
                            );
                        }
                    });
                })
            })
            .into_any_element();
        }

        let overlay = SelectionHeaderOverlay::Control(control.id.clone());
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let overlay_for_open = overlay.clone();
        let can_edit = self.can_edit();
        let command_target = self.current_selection_header_target();
        let panel_id = self.id.clone();
        let control_id = control.id.clone();
        let has_items = !control.menu_items.is_empty();
        let enabled = control.enabled && has_items;
        let tooltip = self.selection_header_tooltip(
            &control.tooltip,
            control.enabled && has_items,
            control.disabled_reason.as_ref(),
            None,
        );
        let overlay_open = self.selection_header_overlay.as_ref() == Some(&overlay);
        let panel_for_keyboard = panel_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-selection-header-control-{}",
            self.id, control.id
        )))
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(26.))
        .h(px(24.))
        .disabled(!enabled)
        .on_keyboard_activate(move |_, cx| {
            panel_for_keyboard.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_keyboard.clone(), !overlay_open, cx);
            });
        })
        .child(self.render_selection_header_control_icon(&control, cx));

        Popover::new(SharedString::from(format!(
            "{}-selection-header-control-menu-{}",
            self.id, control.id
        )))
        .anchor(Anchor::TopRight)
        .open(self.selection_header_overlay.as_ref() == Some(&overlay))
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_open.clone(), *open, cx);
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            v_flex().w(popup_width(window, 208.)).gap_1().children(
                control.menu_items.clone().into_iter().map(|item| {
                    let panel = panel_for_content.clone();
                    let popover = popover.clone();
                    let command = item.command.clone();
                    let access = item.effective_access();
                    let expected_target = command_target.clone();
                    let enabled = item.enabled
                        && expected_target.is_some()
                        && (can_edit || !access.requires_edit());
                    let tooltip = if enabled {
                        item.label.clone()
                    } else {
                        let reason = item
                            .disabled_reason
                            .as_ref()
                            .map(SharedString::as_ref)
                            .unwrap_or(if access.requires_edit() && !can_edit {
                                "View only"
                            } else {
                                "Unavailable"
                            });
                        format!("{} — {reason}", item.label).into()
                    };
                    Button::new(SharedString::from(format!(
                        "{panel_id}-selection-header-control-{control_id}-menu-item-{}",
                        item.id,
                    )))
                    .label(item.label)
                    .tooltip(tooltip)
                    .xsmall()
                    .compact()
                    .ghost()
                    .w_full()
                    .disabled(!enabled)
                    .when(enabled, |button| {
                        button.on_activate(move |_, window, cx| {
                            panel.update(cx, |this, cx| {
                                this.selection_header_overlay = None;
                                if let Some(expected_target) = expected_target.clone() {
                                    this.emit_selection_header_command_for_target(
                                        expected_target,
                                        command.clone(),
                                        access,
                                        cx,
                                    );
                                }
                                cx.notify();
                            });
                            popover.update(cx, |popover, cx| {
                                popover.dismiss(window, cx);
                            });
                        })
                    })
                }),
            )
        })
        .into_any_element()
    }

    pub(super) fn render_selection_header_more(
        &self,
        controls: Vec<DesignSelectionHeaderControl>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let panel = cx.entity();
        let panel_for_open = panel.clone();
        let panel_for_content = panel;
        let overlay = SelectionHeaderOverlay::More;
        let overlay_for_open = overlay.clone();
        let can_edit = self.can_edit();
        let command_target = self.current_selection_header_target();
        let panel_id = self.id.clone();
        let overlay_open = self.selection_header_overlay.as_ref() == Some(&overlay);
        let panel_for_keyboard = panel_for_open.clone();
        let overlay_for_keyboard = overlay.clone();
        let trigger = Button::new(SharedString::from(format!(
            "{}-selection-header-more",
            self.id
        )))
        .tooltip("More")
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .on_keyboard_activate(move |_, cx| {
            panel_for_keyboard.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_keyboard.clone(), !overlay_open, cx);
            });
        })
        .icon(IconName::Ellipsis);

        Popover::new(SharedString::from(format!(
            "{}-selection-header-more-menu",
            self.id
        )))
        .anchor(Anchor::TopRight)
        .open(self.selection_header_overlay.as_ref() == Some(&overlay))
        .overlay_closable(true)
        .on_open_change(move |open, _, cx| {
            panel_for_open.update(cx, |this, cx| {
                this.set_selection_header_overlay(overlay_for_open.clone(), *open, cx);
            });
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let popover = cx.entity();
            let mut rows = Vec::new();
            for control in controls.clone() {
                if control.menu_items.is_empty() {
                    let Some(command) = control.command() else {
                        continue;
                    };
                    let access = control.effective_access();
                    let expected_target = command_target.clone();
                    let enabled = control.enabled
                        && expected_target.is_some()
                        && (can_edit || !access.requires_edit());
                    let reason = if !control.enabled {
                        control
                            .disabled_reason
                            .as_ref()
                            .map(SharedString::as_ref)
                            .unwrap_or("Unavailable")
                    } else if access.requires_edit() && !can_edit {
                        "View only"
                    } else {
                        ""
                    };
                    let tooltip = if reason.is_empty() {
                        control.tooltip.clone()
                    } else {
                        format!("{} — {reason}", control.tooltip).into()
                    };
                    let panel = panel_for_content.clone();
                    let popover = popover.clone();
                    rows.push(
                        Button::new(SharedString::from(format!(
                            "{panel_id}-selection-header-more-control-{}",
                            control.id,
                        )))
                        .label(control.tooltip)
                        .tooltip(tooltip)
                        .xsmall()
                        .compact()
                        .ghost()
                        .w_full()
                        .disabled(!enabled)
                        .when(enabled, |button| {
                            button.on_activate(move |_, window, cx| {
                                panel.update(cx, |this, cx| {
                                    this.selection_header_overlay = None;
                                    if let Some(expected_target) = expected_target.clone() {
                                        this.emit_selection_header_command_for_target(
                                            expected_target,
                                            command.clone(),
                                            access,
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                });
                                popover.update(cx, |popover, cx| {
                                    popover.dismiss(window, cx);
                                });
                            })
                        })
                        .into_any_element(),
                    );
                } else {
                    let control_enabled = control.enabled;
                    let control_disabled_reason = control.disabled_reason.clone();
                    rows.push(
                        div()
                            .h(px(24.))
                            .px_2()
                            .flex()
                            .items_center()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(control.tooltip.clone())
                            .into_any_element(),
                    );
                    for item in control.menu_items {
                        let command = item.command.clone();
                        let access = item.effective_access();
                        let expected_target = command_target.clone();
                        let enabled = control_enabled
                            && item.enabled
                            && expected_target.is_some()
                            && (can_edit || !access.requires_edit());
                        let reason = if !control_enabled {
                            control_disabled_reason
                                .as_ref()
                                .map(SharedString::as_ref)
                                .unwrap_or("Unavailable")
                        } else if !item.enabled {
                            item.disabled_reason
                                .as_ref()
                                .map(SharedString::as_ref)
                                .unwrap_or("Unavailable")
                        } else if access.requires_edit() && !can_edit {
                            "View only"
                        } else {
                            ""
                        };
                        let tooltip = if reason.is_empty() {
                            item.label.clone()
                        } else {
                            format!("{} — {reason}", item.label).into()
                        };
                        let panel = panel_for_content.clone();
                        let popover = popover.clone();
                        rows.push(
                            Button::new(SharedString::from(format!(
                                "{panel_id}-selection-header-more-item-{}-{}",
                                control.id, item.id,
                            )))
                            .label(item.label)
                            .tooltip(tooltip)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .disabled(!enabled)
                            .when(enabled, |button| {
                                button.on_activate(move |_, window, cx| {
                                    panel.update(cx, |this, cx| {
                                        this.selection_header_overlay = None;
                                        if let Some(expected_target) = expected_target.clone() {
                                            this.emit_selection_header_command_for_target(
                                                expected_target,
                                                command.clone(),
                                                access,
                                                cx,
                                            );
                                        }
                                        cx.notify();
                                    });
                                    popover.update(cx, |popover, cx| {
                                        popover.dismiss(window, cx);
                                    });
                                })
                            })
                            .into_any_element(),
                        );
                    }
                }
            }
            v_flex().w(popup_width(window, 216.)).gap_1().children(rows)
        })
        .into_any_element()
    }

    pub(super) fn render_viewer_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let selection_kind = self.inspection_context.selection().kind();
        let selection_count = self.inspection_context.selection().len();
        let title: SharedString = match selection_kind {
            DesignPanelSelectionKind::None => "Page".into(),
            DesignPanelSelectionKind::Single => self.node.name.clone(),
            DesignPanelSelectionKind::Multiple => format!("{selection_count} layers").into(),
        };

        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(self.render_surface_tabs(cx))
            .child(
                h_flex()
                    .h(px(48.))
                    .px(px(PANEL_PADDING))
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .size(px(24.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                Icon::new(if selection_kind == DesignPanelSelectionKind::None {
                                    IconName::File
                                } else {
                                    IconName::Inspector
                                })
                                .xsmall(),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child(title),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_draw_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let selection_kind = self.inspection_context.selection().kind();
        let selection_count = self.inspection_context.selection().len();
        let mut selection_header = self.resolved_selection_header_view_data();
        if selection_kind == DesignPanelSelectionKind::Multiple
            && self.selection_header_view_data_for_context().is_none()
        {
            selection_header.title = format!("{selection_count} layers").into();
        }
        let heading_id = SharedString::from(format!("{}-draw-workspace-heading", self.id));
        let heading_selector = heading_id.to_string();

        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                h_flex()
                    .id(heading_id)
                    .debug_selector(move || heading_selector.clone())
                    .h(px(HEADER_HEIGHT))
                    .px(px(PANEL_PADDING))
                    .items_center()
                    .text_sm()
                    .font_semibold()
                    .child(DesignPanelWorkspaceMode::Draw.label()),
            )
            .child(
                h_flex()
                    .h(px(48.))
                    .pl(px(PANEL_PADDING))
                    .pr_2()
                    .gap_1()
                    .items_center()
                    .when(selection_kind == DesignPanelSelectionKind::None, |header| {
                        header
                            .child(
                                div()
                                    .size(px(24.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(Icon::new(IconName::File).xsmall()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_sm()
                                    .font_semibold()
                                    .child("Page"),
                            )
                    })
                    .when(selection_kind != DesignPanelSelectionKind::None, |header| {
                        header
                            .child(self.render_selection_header_title(&selection_header, cx))
                            .children(
                                selection_header.primary_controls.clone().into_iter().map(
                                    |control| self.render_selection_header_control(control, cx),
                                ),
                            )
                            .when(!selection_header.overflow_controls.is_empty(), |header| {
                                header.child(self.render_selection_header_more(
                                    selection_header.overflow_controls.clone(),
                                    cx,
                                ))
                            })
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.inspection_context.permissions().can_edit() {
            return self.render_viewer_header(cx);
        }
        if self.renders_draw_workspace() {
            return self.render_draw_header(cx);
        }
        let selection_kind = self.inspection_context.selection().kind();
        let selection_count = self.inspection_context.selection().len();
        let mut selection_header = self.resolved_selection_header_view_data();
        if selection_kind == DesignPanelSelectionKind::Multiple
            && self.selection_header_view_data_for_context().is_none()
        {
            selection_header.title = format!("{selection_count} layers").into();
        }

        v_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .child(self.render_surface_tabs(cx))
            .child(
                h_flex()
                    .h(px(48.))
                    .pl(px(PANEL_PADDING))
                    .pr_2()
                    .gap_1()
                    .items_center()
                    .when(selection_kind == DesignPanelSelectionKind::None, |header| {
                        header
                            .child(
                                div()
                                    .size(px(24.))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(Icon::new(IconName::File).xsmall()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_sm()
                                    .font_semibold()
                                    .child("Page"),
                            )
                    })
                    .when(selection_kind != DesignPanelSelectionKind::None, |header| {
                        header
                            .child(self.render_selection_header_title(&selection_header, cx))
                            .children(
                                selection_header.primary_controls.clone().into_iter().map(
                                    |control| self.render_selection_header_control(control, cx),
                                ),
                            )
                            .when(!selection_header.overflow_controls.is_empty(), |header| {
                                header.child(self.render_selection_header_more(
                                    selection_header.overflow_controls.clone(),
                                    cx,
                                ))
                            })
                    }),
            )
            .into_any_element()
    }
}

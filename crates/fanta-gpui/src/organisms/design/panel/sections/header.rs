//! Standalone projection, live event validation, and rendering for the Design
//! inspector header.

use super::super::*;

/// Immutable data consumed by the editor, viewer, and Draw header renderers.
///
/// The renderer cannot reach into the retained panel facade. Interactions
/// return through a narrow event sink and are checked against the live
/// host snapshot by [`dispatch`].
#[derive(Clone)]
pub(in super::super) struct HeaderProjection {
    panel_id: SharedString,
    available_surfaces: Vec<DesignPanelSurface>,
    active_surface: DesignPanelSurface,
    mode: HeaderMode,
    can_edit: bool,
    selection_kind: DesignPanelSelectionKind,
    viewer_title: SharedString,
    selection_header: DesignSelectionHeaderViewData,
    command_target: Option<DesignPanelTarget>,
    selection_header_overlay: Option<SelectionHeaderOverlay>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderMode {
    Editor,
    Viewer,
    Draw,
}

#[derive(Clone)]
pub(in super::super) struct HeaderNavigationProjection {
    available_surfaces: Vec<DesignPanelSurface>,
    active_surface: DesignPanelSurface,
    workspace_mode: DesignPanelWorkspaceMode,
    can_edit: bool,
}

impl HeaderNavigationProjection {
    pub(in super::super) fn new(
        available_surfaces: impl IntoIterator<Item = DesignPanelSurface>,
        active_surface: DesignPanelSurface,
        workspace_mode: DesignPanelWorkspaceMode,
        can_edit: bool,
    ) -> Self {
        Self {
            available_surfaces: available_surfaces.into_iter().collect(),
            active_surface,
            workspace_mode,
            can_edit,
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct HeaderSelectionProjection {
    kind: DesignPanelSelectionKind,
    count: usize,
    selected_node_name: SharedString,
    header: DesignSelectionHeaderViewData,
    command_target: Option<DesignPanelTarget>,
    overlay: Option<SelectionHeaderOverlay>,
}

impl HeaderSelectionProjection {
    pub(in super::super) fn new(
        kind: DesignPanelSelectionKind,
        count: usize,
        selected_node_name: SharedString,
        header: DesignSelectionHeaderViewData,
        command_target: Option<DesignPanelTarget>,
        overlay: Option<SelectionHeaderOverlay>,
    ) -> Self {
        Self {
            kind,
            count,
            selected_node_name,
            header,
            command_target,
            overlay,
        }
    }
}

impl HeaderProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        navigation: HeaderNavigationProjection,
        selection: HeaderSelectionProjection,
    ) -> Self {
        let mode = if !navigation.can_edit {
            HeaderMode::Viewer
        } else if navigation.workspace_mode == DesignPanelWorkspaceMode::Draw {
            HeaderMode::Draw
        } else {
            HeaderMode::Editor
        };
        let viewer_title = match selection.kind {
            DesignPanelSelectionKind::None => "Page".into(),
            DesignPanelSelectionKind::Single => selection.selected_node_name.clone(),
            DesignPanelSelectionKind::Multiple => format!("{} layers", selection.count).into(),
        };
        Self {
            panel_id,
            available_surfaces: navigation.available_surfaces,
            active_surface: navigation.active_surface,
            mode,
            can_edit: navigation.can_edit,
            selection_kind: selection.kind,
            viewer_title,
            selection_header: selection.header,
            command_target: selection.command_target,
            selection_header_overlay: selection.overlay,
        }
    }
}

#[derive(Clone)]
enum HeaderEvent {
    SurfaceChange(DesignPanelSurface),
    SelectionCommand {
        expected_target: DesignPanelTarget,
        command: DesignSelectionHeaderCommand,
        access: DesignSelectionHeaderCommandAccess,
        dismiss_overlay: bool,
    },
    SetSelectionOverlay {
        overlay: SelectionHeaderOverlay,
        open: bool,
    },
    ToggleSelectionOverlay(SelectionHeaderOverlay),
}

fn dispatch(panel: &mut DesignPanel, event: HeaderEvent, cx: &mut Context<DesignPanel>) {
    match event {
        HeaderEvent::SurfaceChange(requested) => request_surface_change(panel, requested, cx),
        HeaderEvent::SelectionCommand {
            expected_target,
            command,
            access,
            dismiss_overlay,
        } => {
            if dismiss_overlay {
                panel.overlays.discard(DesignOpenOverlay::SelectionHeader);
            }
            emit_selection_header_command_for_target(panel, expected_target, command, access, cx);
            if dismiss_overlay {
                cx.notify();
            }
        }
        HeaderEvent::SetSelectionOverlay { overlay, open } => {
            set_selection_header_overlay(panel, overlay, open, cx)
        }
        HeaderEvent::ToggleSelectionOverlay(overlay) => {
            let open = panel.overlays.selection_header_overlay().as_ref() != Some(&overlay);
            set_selection_header_overlay(panel, overlay, open, cx);
        }
    }
}

#[derive(Clone)]
pub(in super::super) struct HeaderEventSink {
    panel: Entity<DesignPanel>,
}

impl HeaderEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn dispatch(&self, event: HeaderEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, event, cx));
    }

    fn update_selection_overlay_from_popover(
        &self,
        overlay: SelectionHeaderOverlay,
        open: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            if open {
                panel.remember_overlay_focus_return(DesignOpenOverlay::SelectionHeader, window, cx);
                dispatch(
                    panel,
                    HeaderEvent::SetSelectionOverlay {
                        overlay,
                        open: true,
                    },
                    cx,
                );
            } else if panel.overlays.selection_header_overlay().as_ref() == Some(&overlay) {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::SelectionHeader,
                    window,
                    cx,
                );
            }
        });
    }
}

pub(in super::super) fn request_surface_change(
    panel: &DesignPanel,
    requested: DesignPanelSurface,
    cx: &mut Context<DesignPanel>,
) {
    let current = panel.active_surface();
    if requested == current || !requested.is_available(panel.can_edit()) {
        return;
    }
    cx.emit_design_panel_action(
        panel,
        DesignPanelAction::SurfaceChangeRequested { current, requested },
    );
}

pub(in super::super) fn render_tab(
    projection: &HeaderProjection,
    surface: DesignPanelSurface,
    event_sink: HeaderEventSink,
) -> AnyElement {
    let active = projection.active_surface == surface;
    let id = SharedString::from(format!("{}-tab-{}", projection.panel_id, surface.slug()));
    let selector = id.to_string();
    let active_selector = format!("{selector}-active");
    let button = Button::new(id)
        .debug_selector(move || selector.clone())
        .label(surface.label())
        .tooltip(SharedString::from(format!("Open {}", surface.label())))
        .xsmall()
        .compact()
        .ghost()
        .selected(active)
        .on_activate(move |_, _, cx| {
            event_sink.dispatch(HeaderEvent::SurfaceChange(surface), cx);
        });
    div()
        .flex_none()
        .when(active, |wrapper| {
            wrapper.debug_selector(move || active_selector.clone())
        })
        .child(button)
        .into_any_element()
}

pub(in super::super) fn render_surface_tabs(
    projection: &HeaderProjection,
    event_sink: &HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let tabs = projection
        .available_surfaces
        .iter()
        .copied()
        .map(|surface| render_tab(projection, surface, event_sink.clone()))
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

pub(in super::super) fn selection_header_command_enabled_with_access(
    can_edit: bool,
    host_enabled: bool,
    access: DesignSelectionHeaderCommandAccess,
) -> bool {
    host_enabled && (can_edit || !access.requires_edit())
}

pub(in super::super) fn selection_header_tooltip(
    can_edit: bool,
    label: &SharedString,
    host_enabled: bool,
    disabled_reason: Option<&SharedString>,
    access: Option<DesignSelectionHeaderCommandAccess>,
) -> SharedString {
    let reason = if !host_enabled {
        disabled_reason.map(SharedString::as_ref)
    } else if access.is_some_and(DesignSelectionHeaderCommandAccess::requires_edit) && !can_edit {
        Some("View only")
    } else {
        None
    };
    reason
        .map(|reason| format!("{label} — {reason}").into())
        .unwrap_or_else(|| label.clone())
}

pub(in super::super) fn emit_selection_header_command_for_target(
    panel: &DesignPanel,
    expected_target: DesignPanelTarget,
    command: DesignSelectionHeaderCommand,
    access: DesignSelectionHeaderCommandAccess,
    cx: &mut Context<DesignPanel>,
) {
    let access = match &command {
        DesignSelectionHeaderCommand::TitleMenuItem { .. }
        | DesignSelectionHeaderCommand::HostDefined { .. } => access,
        _ => command.default_access(),
    };
    if panel.current_selection_header_target().as_ref() != Some(&expected_target)
        || (access.requires_edit() && !panel.can_edit())
    {
        return;
    }
    cx.emit_design_panel_action(
        panel,
        DesignPanelAction::SelectionHeaderCommandRequested {
            target: expected_target,
            command,
        },
    );
}

pub(in super::super) fn set_selection_header_overlay(
    panel: &mut DesignPanel,
    overlay: SelectionHeaderOverlay,
    open: bool,
    cx: &mut Context<DesignPanel>,
) {
    if open {
        panel.prepare_paint_picker_for_dismissal(cx);
        panel.cancel_menu_preview(cx);
        panel
            .overlays
            .open(DesignOverlayState::SelectionHeader(overlay));
        panel.features.typography.settings_tab = TypographySettingsTab::Basics;
        panel.sections.collapse_appearance_details();
    } else if panel.overlays.selection_header_overlay().as_ref() == Some(&overlay) {
        panel.overlays.discard(DesignOpenOverlay::SelectionHeader);
    }
    cx.notify();
}

pub(in super::super) fn render_selection_header_control_icon(
    control: &DesignSelectionHeaderControl,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    use DesignSelectionHeaderControlIcon as IconPresentation;

    let kind = control.kind;
    if let IconPresentation::Lucide(icon) = &control.icon {
        return render_lucide_icon(*icon, cx.theme().foreground, 16.);
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
        IconPresentation::Lucide(_) => unreachable!(),
    };
    if let Some(icon) = named_icon {
        return Icon::new(icon).xsmall().into_any_element();
    }
    let color = cx.theme().foreground;
    let icon = match kind {
        DesignSelectionHeaderControlKind::SelectMatchingLayers => LucideIcon::ScanSearch,
        DesignSelectionHeaderControlKind::CreateLink => LucideIcon::Link,
        DesignSelectionHeaderControlKind::ApplyTextContentVariable => LucideIcon::TypeIcon,
        DesignSelectionHeaderControlKind::CreateComponent => LucideIcon::Component,
        DesignSelectionHeaderControlKind::UseAsMask => LucideIcon::Blend,
        DesignSelectionHeaderControlKind::BooleanFlattenMenu => LucideIcon::Combine,
        DesignSelectionHeaderControlKind::EditObject => LucideIcon::SquareDashedMousePointer,
        DesignSelectionHeaderControlKind::HostDefined => unreachable!(),
    };
    render_lucide_icon(icon, color, 16.)
}

pub(in super::super) fn render_selection_header_title(
    projection: &HeaderProjection,
    data: &DesignSelectionHeaderViewData,
    event_sink: HeaderEventSink,
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

    let sink_for_open = event_sink.clone();
    let sink_for_content = event_sink;
    let overlay = SelectionHeaderOverlay::Title;
    let overlay_for_open = overlay.clone();
    let title = data.title.clone();
    let can_edit = projection.can_edit;
    let command_target = projection.command_target.clone();
    let panel_id = projection.panel_id.clone();
    let sink_for_keyboard = sink_for_open.clone();
    let overlay_for_keyboard = overlay.clone();
    let trigger = Button::new(SharedString::from(format!(
        "{}-selection-header-title",
        projection.panel_id
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
        sink_for_keyboard.dispatch(
            HeaderEvent::ToggleSelectionOverlay(overlay_for_keyboard.clone()),
            cx,
        );
    });

    let popover = Popover::new(SharedString::from(format!(
        "{}-selection-header-title-menu",
        projection.panel_id
    )))
    .anchor(Anchor::TopLeft)
    .open(projection.selection_header_overlay.as_ref() == Some(&overlay))
    .overlay_closable(true)
    .on_open_change(move |open, window, cx| {
        sink_for_open.update_selection_overlay_from_popover(
            overlay_for_open.clone(),
            *open,
            window,
            cx,
        );
    })
    .trigger(trigger)
    .content(move |_, window, cx| {
        let popover = cx.entity();
        v_flex()
            .w(popup_width(window, 184.))
            .gap_1()
            .children(menu.items.clone().into_iter().map(|item| {
                let event_sink = sink_for_content.clone();
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
                        if let Some(expected_target) = expected_target.clone() {
                            event_sink.dispatch(
                                HeaderEvent::SelectionCommand {
                                    expected_target,
                                    command: command.clone(),
                                    access,
                                    dismiss_overlay: true,
                                },
                                cx,
                            );
                        }
                        popover.update(cx, |popover, cx| {
                            popover.dismiss(window, cx);
                        });
                    })
                })
            }))
    });
    div()
        .flex_1()
        .min_w(px(0.))
        .child(popover)
        .into_any_element()
}

pub(in super::super) fn render_selection_header_control(
    projection: &HeaderProjection,
    control: DesignSelectionHeaderControl,
    event_sink: HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    if !control.is_menu() {
        let command = control
            .command()
            .expect("every direct selected-node header control has a command");
        let access = control.effective_access();
        let expected_target = projection.command_target.clone();
        let enabled = expected_target.is_some()
            && selection_header_command_enabled_with_access(
                projection.can_edit,
                control.enabled,
                access,
            );
        let tooltip = selection_header_tooltip(
            projection.can_edit,
            &control.tooltip,
            control.enabled,
            control.disabled_reason.as_ref(),
            Some(access),
        );
        let debug_selector = tooltip.clone();
        return Button::new(SharedString::from(format!(
            "{}-selection-header-control-{}",
            projection.panel_id, control.id
        )))
        .debug_selector(move || debug_selector.to_string())
        .tooltip(tooltip)
        .xsmall()
        .compact()
        .ghost()
        .w(px(24.))
        .h(px(24.))
        .disabled(!enabled)
        .child(render_selection_header_control_icon(&control, cx))
        .when(enabled, |button| {
            button.on_activate(move |_, _, cx| {
                if let Some(expected_target) = expected_target.clone() {
                    event_sink.dispatch(
                        HeaderEvent::SelectionCommand {
                            expected_target,
                            command: command.clone(),
                            access,
                            dismiss_overlay: false,
                        },
                        cx,
                    );
                }
            })
        })
        .into_any_element();
    }

    let overlay = SelectionHeaderOverlay::Control(control.id.clone());
    let sink_for_open = event_sink.clone();
    let sink_for_content = event_sink;
    let overlay_for_open = overlay.clone();
    let can_edit = projection.can_edit;
    let command_target = projection.command_target.clone();
    let panel_id = projection.panel_id.clone();
    let control_id = control.id.clone();
    let has_items = !control.menu_items.is_empty();
    let enabled = control.enabled && has_items;
    let tooltip = selection_header_tooltip(
        projection.can_edit,
        &control.tooltip,
        control.enabled && has_items,
        control.disabled_reason.as_ref(),
        None,
    );
    let sink_for_keyboard = sink_for_open.clone();
    let overlay_for_keyboard = overlay.clone();
    let trigger = Button::new(SharedString::from(format!(
        "{}-selection-header-control-{}",
        projection.panel_id, control.id
    )))
    .tooltip(tooltip)
    .xsmall()
    .compact()
    .ghost()
    .w(px(26.))
    .h(px(24.))
    .disabled(!enabled)
    .on_keyboard_activate(move |_, cx| {
        sink_for_keyboard.dispatch(
            HeaderEvent::ToggleSelectionOverlay(overlay_for_keyboard.clone()),
            cx,
        );
    })
    .child(render_selection_header_control_icon(&control, cx));

    Popover::new(SharedString::from(format!(
        "{}-selection-header-control-menu-{}",
        projection.panel_id, control.id
    )))
    .anchor(Anchor::TopRight)
    .open(projection.selection_header_overlay.as_ref() == Some(&overlay))
    .overlay_closable(true)
    .on_open_change(move |open, window, cx| {
        sink_for_open.update_selection_overlay_from_popover(
            overlay_for_open.clone(),
            *open,
            window,
            cx,
        );
    })
    .trigger(trigger)
    .content(move |_, window, cx| {
        let popover = cx.entity();
        v_flex().w(popup_width(window, 208.)).gap_1().children(
            control.menu_items.clone().into_iter().map(|item| {
                let event_sink = sink_for_content.clone();
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
                        if let Some(expected_target) = expected_target.clone() {
                            event_sink.dispatch(
                                HeaderEvent::SelectionCommand {
                                    expected_target,
                                    command: command.clone(),
                                    access,
                                    dismiss_overlay: true,
                                },
                                cx,
                            );
                        }
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

pub(in super::super) fn render_selection_header_more(
    projection: &HeaderProjection,
    controls: Vec<DesignSelectionHeaderControl>,
    event_sink: HeaderEventSink,
) -> AnyElement {
    let sink_for_open = event_sink.clone();
    let sink_for_content = event_sink;
    let overlay = SelectionHeaderOverlay::More;
    let overlay_for_open = overlay.clone();
    let can_edit = projection.can_edit;
    let command_target = projection.command_target.clone();
    let panel_id = projection.panel_id.clone();
    let sink_for_keyboard = sink_for_open.clone();
    let overlay_for_keyboard = overlay.clone();
    let trigger = Button::new(SharedString::from(format!(
        "{}-selection-header-more",
        projection.panel_id
    )))
    .tooltip("More")
    .xsmall()
    .compact()
    .ghost()
    .w(px(24.))
    .h(px(24.))
    .on_keyboard_activate(move |_, cx| {
        sink_for_keyboard.dispatch(
            HeaderEvent::ToggleSelectionOverlay(overlay_for_keyboard.clone()),
            cx,
        );
    })
    .icon(IconName::Ellipsis);

    Popover::new(SharedString::from(format!(
        "{}-selection-header-more-menu",
        projection.panel_id
    )))
    .anchor(Anchor::TopRight)
    .open(projection.selection_header_overlay.as_ref() == Some(&overlay))
    .overlay_closable(true)
    .on_open_change(move |open, window, cx| {
        sink_for_open.update_selection_overlay_from_popover(
            overlay_for_open.clone(),
            *open,
            window,
            cx,
        );
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
                let event_sink = sink_for_content.clone();
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
                            if let Some(expected_target) = expected_target.clone() {
                                event_sink.dispatch(
                                    HeaderEvent::SelectionCommand {
                                        expected_target,
                                        command: command.clone(),
                                        access,
                                        dismiss_overlay: true,
                                    },
                                    cx,
                                );
                            }
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
                    let event_sink = sink_for_content.clone();
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
                                if let Some(expected_target) = expected_target.clone() {
                                    event_sink.dispatch(
                                        HeaderEvent::SelectionCommand {
                                            expected_target,
                                            command: command.clone(),
                                            access,
                                            dismiss_overlay: true,
                                        },
                                        cx,
                                    );
                                }
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

pub(in super::super) fn render_viewer_header(
    projection: &HeaderProjection,
    event_sink: &HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    v_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(render_surface_tabs(projection, event_sink, cx))
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
                            Icon::new(
                                if projection.selection_kind == DesignPanelSelectionKind::None {
                                    IconName::File
                                } else {
                                    IconName::Inspector
                                },
                            )
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
                        .child(projection.viewer_title.clone()),
                ),
        )
        .into_any_element()
}

fn render_editable_selection_row(
    projection: &HeaderProjection,
    event_sink: &HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let selection_header = &projection.selection_header;
    h_flex()
        .h(px(48.))
        .pl(px(PANEL_PADDING))
        .pr_2()
        .gap_1()
        .items_center()
        .when(
            projection.selection_kind == DesignPanelSelectionKind::None,
            |header| {
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
            },
        )
        .when(
            projection.selection_kind != DesignPanelSelectionKind::None,
            |header| {
                header
                    .child(render_selection_header_title(
                        projection,
                        selection_header,
                        event_sink.clone(),
                    ))
                    .children(selection_header.primary_controls.clone().into_iter().map(
                        |control| {
                            render_selection_header_control(
                                projection,
                                control,
                                event_sink.clone(),
                                cx,
                            )
                        },
                    ))
                    .when(!selection_header.overflow_controls.is_empty(), |header| {
                        header.child(render_selection_header_more(
                            projection,
                            selection_header.overflow_controls.clone(),
                            event_sink.clone(),
                        ))
                    })
            },
        )
        .into_any_element()
}

pub(in super::super) fn render_draw_header(
    projection: &HeaderProjection,
    event_sink: &HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let heading_id = SharedString::from(format!("{}-draw-workspace-heading", projection.panel_id));
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
        .child(render_editable_selection_row(projection, event_sink, cx))
        .into_any_element()
}

pub(in super::super) fn render_editor_header(
    projection: &HeaderProjection,
    event_sink: &HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    v_flex()
        .w_full()
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
        .child(render_surface_tabs(projection, event_sink, cx))
        .child(render_editable_selection_row(projection, event_sink, cx))
        .into_any_element()
}

pub(in super::super) fn render(
    projection: &HeaderProjection,
    event_sink: HeaderEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    match projection.mode {
        HeaderMode::Viewer => render_viewer_header(projection, &event_sink, cx),
        HeaderMode::Draw => render_draw_header(projection, &event_sink, cx),
        HeaderMode::Editor => render_editor_header(projection, &event_sink, cx),
    }
}

/// Thin compatibility adapter for the facade renderer and the existing
/// in-crate behavioral tests. Every renderer still consumes
/// [`HeaderProjection`].
pub(in super::super) trait HeaderPanelCompat: Sized {
    #[cfg(test)]
    fn selection_header_command_enabled_with_access(
        &self,
        host_enabled: bool,
        access: DesignSelectionHeaderCommandAccess,
    ) -> bool;
    #[cfg(test)]
    fn selection_header_tooltip(
        &self,
        label: &SharedString,
        host_enabled: bool,
        disabled_reason: Option<&SharedString>,
        access: Option<DesignSelectionHeaderCommandAccess>,
    ) -> SharedString;
    #[cfg(test)]
    fn emit_selection_header_command_for_target(
        &self,
        expected_target: DesignPanelTarget,
        command: DesignSelectionHeaderCommand,
        access: DesignSelectionHeaderCommandAccess,
        cx: &mut Context<Self>,
    );
    #[cfg(test)]
    fn render_selection_header_control(
        &self,
        control: DesignSelectionHeaderControl,
        cx: &mut Context<Self>,
    ) -> AnyElement;
    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement;
}

impl HeaderPanelCompat for DesignPanel {
    #[cfg(test)]
    fn selection_header_command_enabled_with_access(
        &self,
        host_enabled: bool,
        access: DesignSelectionHeaderCommandAccess,
    ) -> bool {
        selection_header_command_enabled_with_access(self.can_edit(), host_enabled, access)
    }

    #[cfg(test)]
    fn selection_header_tooltip(
        &self,
        label: &SharedString,
        host_enabled: bool,
        disabled_reason: Option<&SharedString>,
        access: Option<DesignSelectionHeaderCommandAccess>,
    ) -> SharedString {
        selection_header_tooltip(
            self.can_edit(),
            label,
            host_enabled,
            disabled_reason,
            access,
        )
    }

    #[cfg(test)]
    fn emit_selection_header_command_for_target(
        &self,
        expected_target: DesignPanelTarget,
        command: DesignSelectionHeaderCommand,
        access: DesignSelectionHeaderCommandAccess,
        cx: &mut Context<Self>,
    ) {
        emit_selection_header_command_for_target(self, expected_target, command, access, cx);
    }

    #[cfg(test)]
    fn render_selection_header_control(
        &self,
        control: DesignSelectionHeaderControl,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_selection_header_control(
            &self.header_projection(),
            control,
            HeaderEventSink::new(cx.entity()),
            cx,
        )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        render(
            &self.header_projection(),
            HeaderEventSink::new(cx.entity()),
            cx,
        )
    }
}

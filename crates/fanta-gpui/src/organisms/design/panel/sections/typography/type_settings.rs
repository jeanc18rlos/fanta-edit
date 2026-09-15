//! Type-settings overlay projection and interaction controller.

use super::super::super::*;

/// Type-settings keeps its existing, slightly roomier 28px controls while
/// deriving every repeated dimension from the shared inspector metrics.
pub(in super::super::super) fn grid_layout() -> crate::molecules::InspectorGridLayout {
    let mut metrics = InspectorMetrics::current();
    metrics.row_height = px(28.);
    metrics.row_gap = px(8.);
    metrics.label_width = px(112.);
    crate::molecules::InspectorGridLayout::compact(metrics)
}

/// Narrow presentation state shared by the legacy type-settings renderer and
/// its standalone overlay controller.
#[derive(Clone)]
pub(in super::super::super) struct TypeSettingsOverlayProjection {
    pub(in super::super::super) panel_id: SharedString,
    pub(in super::super::super) target: DesignPanelTarget,
    pub(in super::super::super) open: bool,
    pub(in super::super::super) tab: TypographySettingsTab,
    pub(in super::super::super) focus: FocusHandle,
}

impl TypeSettingsOverlayProjection {
    pub(in super::super::super) fn from_panel(
        panel: &DesignPanel,
        has_variable_axes: bool,
    ) -> Self {
        Self {
            panel_id: panel.id.clone(),
            target: panel.command_target(),
            open: panel.overlays.type_settings_open(),
            tab: effective_tab(panel.features.typography.settings_tab, has_variable_axes),
            focus: panel.overlays.type_settings_focus().clone(),
        }
    }
}

/// Typed callback boundary retained by type-settings controls.
#[derive(Clone)]
pub(in super::super::super) struct TypeSettingsEventSink {
    panel: Entity<DesignPanel>,
}

impl TypeSettingsEventSink {
    pub(in super::super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    pub(in super::super::super) fn send_overlay(
        &self,
        target: &DesignPanelTarget,
        event: TypeSettingsOverlayEvent,
        focus: &FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            dispatch(panel, target, event, focus, window, cx);
        });
    }

    fn emit_property(&self, property: DesignPanelProperty, value: DesignPanelValue, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| panel.emit_property(property, value, cx));
    }

    fn numeric_scrub_state(&self, property: DesignPanelProperty, cx: &App) -> (bool, bool) {
        let panel = self.panel.read(cx);
        (
            panel.numeric_scrub_is_active(property),
            panel.numeric_scrub_surface_is_enabled(property),
        )
    }

    fn activate_number_field(
        &self,
        property: DesignPanelProperty,
        value: DesignPanelValue,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            let type_settings_was_open = panel.overlays.type_settings_open();
            panel.activate_property_from_control(
                EditorFocusOrigin::TypeSetting(property),
                property,
                value,
                window,
                cx,
            );
            panel
                .overlays
                .set_open(DesignOverlayState::TypeSettings, type_settings_was_open);
            cx.notify();
        });
    }

    fn numeric_scrub_surface(
        &self,
        id: SharedString,
        property: DesignPanelProperty,
        enabled: bool,
        content: AnyElement,
    ) -> AnyElement {
        render_numeric_scrub_surface(
            id,
            self.panel.clone(),
            property,
            EditorFocusOrigin::TypeSetting(property),
            enabled,
            content,
        )
    }
}

pub(in super::super::super) fn render_trigger(
    projection: &TypeSettingsOverlayProjection,
    events: &TypeSettingsEventSink,
) -> Button {
    let metrics = InspectorMetrics::current();
    let focus = projection.focus.clone();
    let target = projection.target.clone();
    let events = events.clone();
    Button::new(SharedString::from(format!(
        "{}-type-settings",
        projection.panel_id
    )))
    .xsmall()
    .compact()
    .ghost()
    .w(metrics.row_height)
    .h(metrics.row_height)
    .on_keyboard_activate(move |window, cx| {
        events.send_overlay(
            &target,
            TypeSettingsOverlayEvent::Toggle,
            &focus,
            window,
            cx,
        );
    })
    .child(Icon::new(IconName::Settings2).xsmall())
}

pub(in super::super::super) fn render_tab(
    panel_id: &SharedString,
    selected_tab: TypographySettingsTab,
    candidate: TypographySettingsTab,
    target: DesignPanelTarget,
    events: &TypeSettingsEventSink,
    focus: FocusHandle,
) -> Button {
    let layout = grid_layout();
    let events = events.clone();
    Button::new(SharedString::from(format!(
        "{}-type-settings-tab-{}",
        panel_id,
        candidate.label().to_lowercase()
    )))
    .label(candidate.label())
    .xsmall()
    .compact()
    .ghost()
    .h(layout.row_height())
    .selected(candidate == selected_tab)
    .on_activate(move |_, window, cx| {
        events.send_overlay(
            &target,
            TypeSettingsOverlayEvent::SelectTab(candidate),
            &focus,
            window,
            cx,
        );
    })
}

pub(in super::super::super) fn render_segments(
    panel_id: SharedString,
    id_suffix: &'static str,
    options: Vec<(SharedString, bool, DesignPanelValue)>,
    enabled: bool,
    property: DesignPanelProperty,
    events: &TypeSettingsEventSink,
    cx: &mut App,
) -> AnyElement {
    let metrics = grid_layout().metrics;
    let access = if enabled {
        InspectorFieldAccess::Editable
    } else {
        InspectorFieldAccess::disabled(None)
    };
    let mut segments = crate::molecules::inspector_segmented_control(metrics, cx)
        .border_0()
        .w_full()
        .bg(cx.theme().secondary);
    for (index, (label, selected, value)) in options.into_iter().enumerate() {
        let events = events.clone();
        let mut segment = crate::molecules::inspector_segment(
            SharedString::from(format!("{panel_id}-type-setting-{id_suffix}-{index}")),
            selected,
            &access,
            metrics,
            cx,
        )
        .when(index > 0, |segment| {
            segment
                .border_l_1()
                .border_color(cx.theme().border.opacity(0.72))
        })
        .child(label);
        if enabled {
            segment = segment.on_activate(move |_, _, cx| {
                events.emit_property(property, value.clone(), cx);
            });
        }
        segments = segments.child(segment);
    }
    segments.into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(in super::super::super) fn render_number_field(
    panel_id: SharedString,
    id_suffix: &'static str,
    label: SharedString,
    property: DesignPanelProperty,
    value: DesignPanelValue,
    enabled: bool,
    editing: bool,
    invalid: bool,
    input: Entity<InputState>,
    events: &TypeSettingsEventSink,
    cx: &mut App,
) -> AnyElement {
    let layout = grid_layout();
    let id = SharedString::from(format!("{panel_id}-type-setting-{id_suffix}-number"));
    let (scrubbing, scrub_enabled) = events.numeric_scrub_state(property, cx);
    let events_for_activate = events.clone();
    let button_id = SharedString::from(format!("{id}-activate"));
    let button_debug_selector = button_id.clone();
    let button = Button::new(button_id)
        .debug_selector(move || button_debug_selector.to_string())
        .label(label)
        .xsmall()
        .compact()
        .w_full()
        .h(layout.row_height())
        .disabled(!enabled)
        .on_activate(move |_, window, cx| {
            events_for_activate.activate_number_field(property, value.clone(), window, cx);
        });
    if editing && !scrubbing {
        return div()
            .id(id)
            .relative()
            .h(layout.row_height())
            .w_full()
            .rounded(layout.metrics.radius)
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
                            .h(layout.row_height() - px(2.))
                            .w_full(),
                    ),
            )
            .into_any_element();
    }

    div()
        .id(id)
        .h(layout.row_height())
        .w_full()
        .child(events.numeric_scrub_surface(
            SharedString::from(format!("{panel_id}-type-setting-{id_suffix}-number-scrub")),
            property,
            scrub_enabled,
            button.into_any_element(),
        ))
        .into_any_element()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in super::super::super) enum TypeSettingsOverlayEvent {
    Toggle,
    OpenChanged(bool),
    SelectTab(TypographySettingsTab),
}

pub(in super::super::super) fn effective_tab(
    requested: TypographySettingsTab,
    has_variable_axes: bool,
) -> TypographySettingsTab {
    if requested == TypographySettingsTab::Variable && !has_variable_axes {
        TypographySettingsTab::Details
    } else {
        requested
    }
}

/// Explicit event sink for the retained overlay/tab presentation state.
pub(in super::super::super) fn dispatch(
    panel: &mut DesignPanel,
    expected_target: &DesignPanelTarget,
    event: TypeSettingsOverlayEvent,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<DesignPanel>,
) {
    let live_target_matches = panel.command_target() == *expected_target
        && panel.host.inspected_node().typography.is_some()
        && panel
            .host
            .inspected_node()
            .supports_section(DesignPanelSection::Typography);
    match event {
        TypeSettingsOverlayEvent::Toggle if live_target_matches => {
            if !panel.overlays.type_settings_open() {
                panel.remember_overlay_focus_return(DesignOpenOverlay::TypeSettings, window, cx);
            }
            if panel.overlays.toggle(DesignOverlayState::TypeSettings) {
                prepare_open(panel, focus, window, cx);
            } else {
                panel.features.typography.settings_tab = TypographySettingsTab::Basics;
            }
            cx.notify();
        }
        TypeSettingsOverlayEvent::OpenChanged(false) => {
            if panel.overlays.type_settings_open() {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::TypeSettings,
                    window,
                    cx,
                );
            }
        }
        TypeSettingsOverlayEvent::OpenChanged(true) if live_target_matches => {
            if !panel.overlays.type_settings_open() {
                panel.remember_overlay_focus_return(DesignOpenOverlay::TypeSettings, window, cx);
            }
            panel.overlays.open(DesignOverlayState::TypeSettings);
            prepare_open(panel, focus, window, cx);
            cx.notify();
        }
        TypeSettingsOverlayEvent::SelectTab(tab) if live_target_matches => {
            panel.features.typography.settings_tab = tab;
            cx.notify();
        }
        TypeSettingsOverlayEvent::Toggle
        | TypeSettingsOverlayEvent::OpenChanged(true)
        | TypeSettingsOverlayEvent::SelectTab(_) => {}
    }
}

fn prepare_open(
    panel: &mut DesignPanel,
    focus: &FocusHandle,
    window: &mut Window,
    cx: &mut Context<DesignPanel>,
) {
    panel.cancel_menu_preview(cx);
    focus.focus(window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_tab_falls_back_only_without_axes() {
        assert_eq!(
            effective_tab(TypographySettingsTab::Variable, false),
            TypographySettingsTab::Details
        );
        assert_eq!(
            effective_tab(TypographySettingsTab::Variable, true),
            TypographySettingsTab::Variable
        );
        assert_eq!(
            effective_tab(TypographySettingsTab::Basics, false),
            TypographySettingsTab::Basics
        );
    }
}

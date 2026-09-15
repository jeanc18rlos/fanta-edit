//! Controlled font-browser projection, renderer, and event adapter.

use super::super::super::*;

/// One immutable row in the font browser.
///
/// Presentation decisions are made while projecting the current host snapshot.
/// Activations still flow through [`dispatch`], which revalidates the selection
/// against the live catalog before emitting an existing Design-panel action.
#[derive(Clone, Debug, PartialEq)]
struct FontBrowserRow {
    selection: DesignFontSelection,
    label: SharedString,
    tooltip: SharedString,
    selected: bool,
    enabled: bool,
    activation: FontBrowserActivation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FontBrowserActivation {
    Apply,
    Import,
}

impl FontBrowserActivation {
    fn for_availability(availability: &DesignFontAvailability) -> Self {
        if availability.can_import() {
            Self::Import
        } else {
            Self::Apply
        }
    }
}

/// Narrow immutable read model for the font-browser child.
#[derive(Clone, Debug, PartialEq)]
pub(in super::super::super) struct FontBrowserProjection {
    panel_id: SharedString,
    panel_target: DesignPanelTarget,
    typography_target: DesignTypographyTarget,
    trigger_label: SharedString,
    open: bool,
    state: DesignFontCatalogState,
    rows: Vec<FontBrowserRow>,
}

impl FontBrowserProjection {
    pub(in super::super::super) fn from_panel(panel: &DesignPanel, query: &str) -> Option<Self> {
        let typography = panel.host.inspected_node().typography.as_ref()?;
        let family_label = panel
            .display_property_value(DesignPanelProperty::FontFamily, typography.family.clone());
        let style_label =
            panel.display_property_value(DesignPanelProperty::FontStyle, typography.style.clone());
        let current_family = typography.family.clone();
        let current_style = typography.style.clone();
        let can_edit = panel.can_edit();
        let can_edit_family = panel.property_is_editable(DesignPanelProperty::FontFamily);
        let can_edit_style = panel.property_is_editable(DesignPanelProperty::FontStyle);
        let can_edit_weight = panel.property_is_editable(DesignPanelProperty::FontWeight);
        let rows = panel
            .resources
            .fonts
            .matching(query)
            .map(|(family, style)| {
                let selection = DesignFontSelection {
                    source: family.source.clone(),
                    family_id: family.id.clone(),
                    style_id: style.id.clone(),
                };
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
                let can_apply_properties = can_edit_family
                    && can_edit_style
                    && (style.weight.is_none() || can_edit_weight);
                let enabled = can_edit
                    && (style.availability.can_import()
                        || (style.availability.can_apply() && can_apply_properties));
                FontBrowserRow {
                    selected: family.name == current_family && style.name == current_style,
                    label: format!(
                        "{} · {}  —  {}",
                        family.name, style.name, availability_label
                    )
                    .into(),
                    tooltip: style
                        .preview
                        .clone()
                        .unwrap_or_else(|| "Font family and style".into()),
                    activation: FontBrowserActivation::for_availability(&style.availability),
                    selection,
                    enabled,
                }
            })
            .collect();

        Some(Self {
            panel_id: panel.id.clone(),
            panel_target: panel.command_target(),
            typography_target: panel.typography_target(DesignPanelProperty::FontFamily),
            trigger_label: format!("{family_label} · {style_label}").into(),
            open: panel.overlays.font_browser_open(),
            state: panel.resources.fonts.state.clone(),
            rows,
        })
    }
}

/// Events owned by the font-browser child rather than the panel facade.
#[derive(Clone, Debug, PartialEq)]
enum FontBrowserEvent {
    OpenChanged(bool),
    Activate {
        selection: DesignFontSelection,
        activation: FontBrowserActivation,
    },
}

/// Typed callback boundary retained by the font-browser renderer.
#[derive(Clone)]
pub(in super::super::super) struct FontBrowserEventSink {
    panel: Entity<DesignPanel>,
}

impl FontBrowserEventSink {
    pub(in super::super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(
        &self,
        expected_panel_target: &DesignPanelTarget,
        expected_typography_target: DesignTypographyTarget,
        event: FontBrowserEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.panel.update(cx, |panel, cx| {
            dispatch(
                panel,
                expected_panel_target,
                expected_typography_target,
                event,
                window,
                cx,
            );
        });
    }
}

/// Explicit event sink back into the stable Design-panel facade.
fn dispatch(
    panel: &mut DesignPanel,
    expected_panel_target: &DesignPanelTarget,
    expected_typography_target: DesignTypographyTarget,
    event: FontBrowserEvent,
    window: &mut Window,
    cx: &mut Context<DesignPanel>,
) {
    match event {
        FontBrowserEvent::OpenChanged(true)
            if panel.command_target() == *expected_panel_target
                && panel.host.inspected_node().typography.is_some() =>
        {
            panel.open_font_browser(window, cx);
        }
        FontBrowserEvent::OpenChanged(true) => {}
        FontBrowserEvent::OpenChanged(false) => {
            if panel.overlays.font_browser_open() {
                let _ = panel.dismiss_overlay_from_outside_click(
                    DesignOpenOverlay::FontBrowser,
                    window,
                    cx,
                );
            }
        }
        FontBrowserEvent::Activate {
            selection,
            activation: FontBrowserActivation::Apply,
        } if panel.command_target() == *expected_panel_target
            && panel.typography_target(DesignPanelProperty::FontFamily)
                == expected_typography_target =>
        {
            panel.emit_font_apply(selection, cx);
        }
        FontBrowserEvent::Activate {
            selection,
            activation: FontBrowserActivation::Import,
        } if panel.command_target() == *expected_panel_target
            && panel.typography_target(DesignPanelProperty::FontFamily)
                == expected_typography_target =>
        {
            panel.emit_font_import(selection, cx);
        }
        FontBrowserEvent::Activate { .. } => {}
    }
}

/// Render the font trigger and its controlled browser popover.
pub(in super::super::super) fn render(
    projection: FontBrowserProjection,
    search: Entity<InputState>,
    events: &FontBrowserEventSink,
    _cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let events_for_trigger = events.clone();
    let events_for_open = events.clone();
    let events_for_content = events.clone();
    let open = projection.open;
    let panel_id = projection.panel_id.clone();
    let trigger_panel_target = projection.panel_target.clone();
    let trigger_typography_target = projection.typography_target;
    let open_panel_target = projection.panel_target.clone();
    let open_typography_target = projection.typography_target;
    let content_panel_target = projection.panel_target.clone();
    let content_typography_target = projection.typography_target;
    let state = projection.state.clone();
    let rows = projection.rows.clone();
    let trigger = Button::new(SharedString::from(format!("{panel_id}-font-browser")))
        .label(projection.trigger_label)
        .tooltip("Browse font family and style")
        .xsmall()
        .compact()
        .w_full()
        .h(px(ROW_HEIGHT))
        .selected(open)
        .on_activate(move |_, window, cx| {
            cx.stop_propagation();
            events_for_trigger.send(
                &trigger_panel_target,
                trigger_typography_target,
                FontBrowserEvent::OpenChanged(true),
                window,
                cx,
            );
        });

    Popover::new(SharedString::from(format!("{panel_id}-font-popover")))
        .anchor(Anchor::TopRight)
        .open(open)
        .overlay_closable(true)
        .on_open_change(move |is_open, window, cx| {
            events_for_open.send(
                &open_panel_target,
                open_typography_target,
                FontBrowserEvent::OpenChanged(*is_open),
                window,
                cx,
            );
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
                    for row in rows.clone() {
                        let events = events_for_content.clone();
                        let panel_target = content_panel_target.clone();
                        let selection = row.selection.clone();
                        let activation = row.activation;
                        list = list.child(
                            Button::new(SharedString::from(format!(
                                "font-{}-{}",
                                row.selection.family_id, row.selection.style_id
                            )))
                            .label(row.label)
                            .tooltip(row.tooltip)
                            .xsmall()
                            .compact()
                            .ghost()
                            .w_full()
                            .selected(row.selected)
                            .disabled(!row.enabled)
                            .on_activate(move |_, window, cx| {
                                events.send(
                                    &panel_target,
                                    content_typography_target,
                                    FontBrowserEvent::Activate {
                                        selection: selection.clone(),
                                        activation,
                                    },
                                    window,
                                    cx,
                                );
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
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_follows_catalog_availability() {
        assert_eq!(
            FontBrowserActivation::for_availability(&DesignFontAvailability::Imported),
            FontBrowserActivation::Apply
        );
        assert_eq!(
            FontBrowserActivation::for_availability(&DesignFontAvailability::Available),
            FontBrowserActivation::Import
        );
        assert_eq!(
            FontBrowserActivation::for_availability(&DesignFontAvailability::Missing {
                reason: "Not installed".into(),
            }),
            FontBrowserActivation::Apply
        );
    }
}

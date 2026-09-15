//! Standalone Typography section projection, controller, and renderer.
//!
//! Stateful number editing and the retained type-settings surface still live
//! behind [`TypographyInspectorChrome`]. The section itself cannot inspect the
//! `DesignPanel` facade: it renders an immutable snapshot and sends commands
//! through a typed event sink, where the live inspection target and property
//! access are checked again.

use super::super::super::{DesignTextPathViewData, DesignTypography};
use super::super::*;

pub(in super::super) mod font_browser;
pub(in super::super) mod style_picker;
pub(in super::super) mod type_settings;

/// Stable identity and target scope for one Typography render pass.
#[derive(Clone)]
pub(in super::super) struct TypographyIdentityProjection {
    panel_id: SharedString,
    panel_target: DesignPanelTarget,
    selected_text_target: DesignTypographyTarget,
}

impl TypographyIdentityProjection {
    pub(in super::super) fn new(
        panel_id: SharedString,
        panel_target: DesignPanelTarget,
        selected_text_target: DesignTypographyTarget,
    ) -> Self {
        Self {
            panel_id,
            panel_target,
            selected_text_target,
        }
    }

    fn typography_target(&self, property: DesignPanelProperty) -> DesignTypographyTarget {
        if property.supports_selected_text_range() {
            self.selected_text_target
        } else {
            DesignTypographyTarget::WholeLayer
        }
    }
}

/// Host-controlled typography values rendered by the section.
#[derive(Clone)]
pub(in super::super) struct TypographyValuesProjection {
    typography: DesignTypography,
    text_path: Option<DesignTextPathViewData>,
    text_path_start: Option<DesignTextPathStartData>,
}

impl TypographyValuesProjection {
    pub(in super::super) fn new(
        typography: DesignTypography,
        text_path: Option<DesignTextPathViewData>,
        text_path_start: Option<DesignTextPathStartData>,
    ) -> Self {
        Self {
            typography,
            text_path,
            text_path_start,
        }
    }
}

/// Live-host capabilities projected for deterministic presentation.
#[derive(Clone, Copy)]
pub(in super::super) struct TypographyAccessProjection {
    can_edit: bool,
    horizontal_alignment_editable: bool,
    vertical_alignment_editable: bool,
    text_path_flip_available: bool,
    text_path_start_debug_available: bool,
}

impl TypographyAccessProjection {
    pub(in super::super) const fn new(
        can_edit: bool,
        horizontal_alignment_editable: bool,
        vertical_alignment_editable: bool,
        text_path_flip_available: bool,
        text_path_start_debug_available: bool,
    ) -> Self {
        Self {
            can_edit,
            horizontal_alignment_editable,
            vertical_alignment_editable,
            text_path_flip_available,
            text_path_start_debug_available,
        }
    }
}

/// Catalog projections and retained search/picker entities.
#[derive(Clone)]
pub(in super::super) struct TypographyResourceProjection {
    font_browser: Option<font_browser::FontBrowserProjection>,
    font_search: Entity<InputState>,
    style_picker: style_picker::TypographyStylePickerProjection,
}

impl TypographyResourceProjection {
    pub(in super::super) fn new(
        font_browser: Option<font_browser::FontBrowserProjection>,
        font_search: Entity<InputState>,
        style_picker: style_picker::TypographyStylePickerProjection,
    ) -> Self {
        Self {
            font_browser,
            font_search,
            style_picker,
        }
    }
}

/// Disclosure and overlay state owned by inspector presentation.
#[derive(Clone)]
pub(in super::super) struct TypographyPresentationProjection {
    expanded: bool,
    type_settings: type_settings::TypeSettingsOverlayProjection,
}

impl TypographyPresentationProjection {
    pub(in super::super) fn new(
        expanded: bool,
        type_settings: type_settings::TypeSettingsOverlayProjection,
    ) -> Self {
        Self {
            expanded,
            type_settings,
        }
    }
}

/// Canonical immutable snapshot consumed by the Typography renderer.
#[derive(Clone)]
pub(in super::super) struct TypographyProjection {
    identity: TypographyIdentityProjection,
    values: TypographyValuesProjection,
    access: TypographyAccessProjection,
    resources: TypographyResourceProjection,
    presentation: TypographyPresentationProjection,
}

impl TypographyProjection {
    pub(in super::super) fn new(
        identity: TypographyIdentityProjection,
        values: TypographyValuesProjection,
        access: TypographyAccessProjection,
        resources: TypographyResourceProjection,
        presentation: TypographyPresentationProjection,
    ) -> Self {
        Self {
            identity,
            values,
            access,
            resources,
            presentation,
        }
    }
}

/// Elements prepared by adjacent feature controllers at the facade boundary.
pub(in super::super) struct TypographySupplementaryControls {
    family_variable: Option<AnyElement>,
    style_variable: Option<AnyElement>,
    applied_component_properties: Option<AnyElement>,
}

impl TypographySupplementaryControls {
    pub(in super::super) fn new(
        family_variable: Option<AnyElement>,
        style_variable: Option<AnyElement>,
        applied_component_properties: Option<AnyElement>,
    ) -> Self {
        Self {
            family_variable,
            style_variable,
            applied_component_properties,
        }
    }
}

/// Narrow continuity adapter for stateful field and type-settings children.
pub(in super::super) trait TypographyInspectorChrome {
    fn typography_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;

    fn typography_type_settings(
        &self,
        typography: &DesignTypography,
        overlay: type_settings::TypeSettingsOverlayProjection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement;
}

impl TypographyInspectorChrome for DesignPanel {
    fn typography_value_cell(
        &self,
        id_suffix: impl Into<SharedString>,
        prefix: &'static str,
        value: impl Into<SharedString>,
        property: DesignPanelProperty,
        next: DesignPanelValue,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_value_cell(id_suffix, prefix, value, property, next, cx)
    }

    fn typography_type_settings(
        &self,
        typography: &DesignTypography,
        overlay: type_settings::TypeSettingsOverlayProjection,
        cx: &mut Context<DesignPanel>,
    ) -> AnyElement {
        self.render_type_settings_popover_projected(typography, overlay, cx)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TypographyEvent {
    Property(DesignPanelProperty, DesignPanelValue),
    FlipTextPath,
    ToggleSection,
}

/// Typed event channel retained by the Typography renderer.
#[derive(Clone)]
pub(in super::super) struct TypographyEventSink {
    panel: Entity<DesignPanel>,
}

impl TypographyEventSink {
    pub(in super::super) fn new(panel: Entity<DesignPanel>) -> Self {
        Self { panel }
    }

    fn send(&self, identity: &TypographyIdentityProjection, event: TypographyEvent, cx: &mut App) {
        self.panel
            .update(cx, |panel, cx| dispatch(panel, identity, event, cx));
    }

    fn font_browser(&self) -> font_browser::FontBrowserEventSink {
        font_browser::FontBrowserEventSink::new(self.panel.clone())
    }

    fn style_picker(&self) -> style_picker::TypographyStylePickerEventSink {
        style_picker::TypographyStylePickerEventSink::new(self.panel.clone())
    }
}

/// Revalidate snapshot-sensitive commands against the current host echo.
fn dispatch(
    panel: &mut DesignPanel,
    identity: &TypographyIdentityProjection,
    event: TypographyEvent,
    cx: &mut Context<DesignPanel>,
) {
    if panel.command_target() != identity.panel_target
        || panel.host.inspected_node().typography.is_none()
        || !panel
            .host
            .inspected_node()
            .supports_section(DesignPanelSection::Typography)
    {
        return;
    }
    match event {
        TypographyEvent::Property(property, value)
            if panel.typography_target(property) == identity.typography_target(property)
                && panel.property_is_editable(property) =>
        {
            panel.emit_property(property, value, cx);
        }
        TypographyEvent::Property(_, _) => {}
        TypographyEvent::FlipTextPath => panel.emit_text_path_flip_orientation(cx),
        TypographyEvent::ToggleSection => panel.toggle_section(DesignPanelSection::Typography, cx),
    }
}

fn group_label(label: &'static str, cx: &mut Context<DesignPanel>) -> AnyElement {
    div()
        .h(px(16.))
        .flex()
        .items_center()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

fn bound_style_summary(
    panel_id: &SharedString,
    name: SharedString,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    h_flex()
        .id(SharedString::from(format!(
            "{panel_id}-typography-bound-style"
        )))
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_2()
        .gap_2()
        .rounded(px(4.))
        .bg(cx.theme().secondary)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .child(name),
        )
        .into_any_element()
}

fn alignment_icon(
    horizontal: Option<DesignTextHorizontalAlignment>,
    vertical: Option<DesignTextVerticalAlignment>,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let icon = match (horizontal, vertical) {
        (Some(DesignTextHorizontalAlignment::Left), _) => LucideIcon::TextAlignStart,
        (Some(DesignTextHorizontalAlignment::Center), _) => LucideIcon::TextAlignCenter,
        (Some(DesignTextHorizontalAlignment::Right), _) => LucideIcon::TextAlignEnd,
        (Some(DesignTextHorizontalAlignment::Justified), _) => LucideIcon::TextAlignJustify,
        (_, Some(DesignTextVerticalAlignment::Top)) => LucideIcon::AlignVerticalJustifyStart,
        (_, Some(DesignTextVerticalAlignment::Center)) => LucideIcon::AlignVerticalJustifyCenter,
        (_, Some(DesignTextVerticalAlignment::Bottom)) => LucideIcon::AlignVerticalJustifyEnd,
        (None, None) => LucideIcon::TextAlignStart,
    };
    render_lucide_icon(icon, cx.theme().foreground, 16.)
}

struct TypographyAlignmentOption {
    id_suffix: &'static str,
    horizontal: Option<DesignTextHorizontalAlignment>,
    vertical: Option<DesignTextVerticalAlignment>,
    selected: bool,
    editable: bool,
    property: DesignPanelProperty,
    value: DesignPanelValue,
}

impl TypographyAlignmentOption {
    fn horizontal(
        id_suffix: &'static str,
        value: DesignTextHorizontalAlignment,
        current: DesignTextHorizontalAlignment,
        editable: bool,
    ) -> Self {
        Self {
            id_suffix,
            horizontal: Some(value),
            vertical: None,
            selected: current == value,
            editable,
            property: DesignPanelProperty::HorizontalTextAlignment,
            value: DesignPanelValue::TextHorizontalAlignment(value),
        }
    }

    fn vertical(
        id_suffix: &'static str,
        value: DesignTextVerticalAlignment,
        current: DesignTextVerticalAlignment,
        editable: bool,
    ) -> Self {
        Self {
            id_suffix,
            horizontal: None,
            vertical: Some(value),
            selected: current == value,
            editable,
            property: DesignPanelProperty::VerticalTextAlignment,
            value: DesignPanelValue::TextVerticalAlignment(value),
        }
    }
}

fn alignment_segment(
    panel_id: &SharedString,
    option: TypographyAlignmentOption,
    identity: TypographyIdentityProjection,
    events: &TypographyEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let events = events.clone();
    Button::new(SharedString::from(format!(
        "{panel_id}-{}",
        option.id_suffix
    )))
    .xsmall()
    .compact()
    .ghost()
    .w_full()
    .h(px(ROW_HEIGHT))
    .selected(option.selected)
    .disabled(!option.editable)
    .child(alignment_icon(option.horizontal, option.vertical, cx))
    .on_activate(move |_, _, cx| {
        events.send(
            &identity,
            TypographyEvent::Property(option.property, option.value.clone()),
            cx,
        );
    })
    .into_any_element()
}

fn font_browser_row(
    projection: &TypographyProjection,
    controls: &mut TypographySupplementaryControls,
    events: &TypographyEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let browser = projection.resources.font_browser.clone().map_or_else(
        || div().into_any_element(),
        |browser| {
            let browser_events = events.font_browser();
            font_browser::render(
                browser,
                projection.resources.font_search.clone(),
                &browser_events,
                cx,
            )
        },
    );
    h_flex()
        .w_full()
        .gap_0p5()
        .child(div().flex_1().min_w(px(0.)).child(browser))
        .when_some(controls.family_variable.take(), |row, button| {
            row.child(button)
        })
        .when_some(controls.style_variable.take(), |row, button| {
            row.child(button)
        })
        .into_any_element()
}

fn text_path_orientation(
    projection: &TypographyProjection,
    events: &TypographyEventSink,
    cx: &mut Context<DesignPanel>,
) -> Option<AnyElement> {
    let text_path = projection.values.text_path?;
    let enabled = projection.access.can_edit && projection.access.text_path_flip_available;
    let orientation = text_path.orientation;
    let tooltip: SharedString = if enabled {
        format!("Flip text orientation · currently {}", orientation.label()).into()
    } else if projection.access.can_edit {
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
    let identity = projection.identity.clone();
    let control_id = SharedString::from(format!(
        "{}-text-path-flip-orientation",
        projection.identity.panel_id
    ));
    let debug_selector = control_id.to_string();
    let mut control = div()
        .id(control_id)
        .debug_selector(move || debug_selector.clone())
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
        let events = events.clone();
        control = control
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| {
                style
                    .bg(cx.theme().accent)
                    .border_color(cx.theme().selection)
            })
            .on_activate(move |_, _, cx| {
                events.send(&identity, TypographyEvent::FlipTextPath, cx);
            });
    }
    Some(
        v_flex()
            .w_full()
            .gap_1()
            .child(control.child("Flip text orientation"))
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

fn render_content(
    projection: &TypographyProjection,
    chrome: &impl TypographyInspectorChrome,
    mut controls: TypographySupplementaryControls,
    events: &TypographyEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let typography = &projection.values.typography;
    let mut content = if let Some(binding) = typography.style_binding.as_ref() {
        v_flex()
            .px(px(PANEL_PADDING))
            .pb_4()
            .gap_2()
            .child(bound_style_summary(
                &projection.identity.panel_id,
                binding.name.clone(),
                cx,
            ))
    } else {
        let horizontal = typography.horizontal_alignment;
        let vertical = typography.vertical_alignment;
        v_flex()
            .px(px(PANEL_PADDING))
            .pb_4()
            .gap_2()
            .child(font_browser_row(projection, &mut controls, events, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_1()
                            .child(group_label("Weight", cx))
                            .child(chrome.typography_value_cell(
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
                            .child(group_label("Size", cx))
                            .child(chrome.typography_value_cell(
                                "font-size",
                                "Size",
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
                            .child(group_label("Line height", cx))
                            .child(chrome.typography_value_cell(
                                "line-height",
                                "Line",
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
                            .child(group_label("Letter spacing", cx))
                            .child(chrome.typography_value_cell(
                                "letter-spacing",
                                "Track",
                                format_letter_spacing(typography.letter_spacing),
                                DesignPanelProperty::LetterSpacing,
                                DesignPanelValue::LetterSpacing(next_letter_spacing(
                                    typography.letter_spacing,
                                )),
                                cx,
                            )),
                    ),
            )
            .child(group_label("Alignment", cx))
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
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::horizontal(
                                    "text-align-left",
                                    DesignTextHorizontalAlignment::Left,
                                    horizontal,
                                    projection.access.horizontal_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
                                cx,
                            ))
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::horizontal(
                                    "text-align-center",
                                    DesignTextHorizontalAlignment::Center,
                                    horizontal,
                                    projection.access.horizontal_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
                                cx,
                            ))
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::horizontal(
                                    "text-align-right",
                                    DesignTextHorizontalAlignment::Right,
                                    horizontal,
                                    projection.access.horizontal_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
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
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::vertical(
                                    "vertical-text-align-top",
                                    DesignTextVerticalAlignment::Top,
                                    vertical,
                                    projection.access.vertical_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
                                cx,
                            ))
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::vertical(
                                    "vertical-text-align-center",
                                    DesignTextVerticalAlignment::Center,
                                    vertical,
                                    projection.access.vertical_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
                                cx,
                            ))
                            .child(alignment_segment(
                                &projection.identity.panel_id,
                                TypographyAlignmentOption::vertical(
                                    "vertical-text-align-bottom",
                                    DesignTextVerticalAlignment::Bottom,
                                    vertical,
                                    projection.access.vertical_alignment_editable,
                                ),
                                projection.identity.clone(),
                                events,
                                cx,
                            )),
                    )
                    .child(chrome.typography_type_settings(
                        typography,
                        projection.presentation.type_settings.clone(),
                        cx,
                    )),
            )
    };
    if let Some(applied) = controls.applied_component_properties.take() {
        content = content.child(applied);
    }
    if let Some(orientation) = text_path_orientation(projection, events, cx) {
        content = content
            .child(group_label("Text on path", cx))
            .child(orientation);
    }
    if projection.access.text_path_start_debug_available
        && let Some(start) = projection.values.text_path_start
    {
        content = content
            .child(group_label("Start data · API debug", cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(chrome.typography_value_cell(
                                "text-path-start-segment",
                                "#",
                                start.segment.to_string(),
                                DesignPanelProperty::TextPathStartSegment,
                                DesignPanelValue::Integer(i64::from(
                                    start.segment.saturating_add(1),
                                )),
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(chrome.typography_value_cell(
                                "text-path-start-position",
                                "%",
                                format!("{}%", format_number(start.position * 100.)),
                                DesignPanelProperty::TextPathStartPosition,
                                DesignPanelValue::Ratio((start.position + 0.05).min(1.)),
                                cx,
                            )),
                    ),
            );
    }
    content.into_any_element()
}

/// Render the complete Typography section from a stable read model.
pub(in super::super) fn render(
    projection: &TypographyProjection,
    chrome: &impl TypographyInspectorChrome,
    controls: TypographySupplementaryControls,
    events: &TypographyEventSink,
    cx: &mut Context<DesignPanel>,
) -> AnyElement {
    let identity = projection.identity.clone();
    let header_events = events.clone();
    let header = crate::molecules::inspector_section_header(
        SharedString::from(format!(
            "{}-section-typography",
            projection.identity.panel_id
        )),
        InspectorMetrics::default(),
        cx,
    )
    .pl(px(PANEL_PADDING))
    .pr_2()
    .gap_1()
    .on_activate(move |_, _, cx| {
        header_events.send(&identity, TypographyEvent::ToggleSection, cx);
    })
    .child(
        div()
            .flex_1()
            .text_sm()
            .font_semibold()
            .child(DesignPanelSection::Typography.label()),
    )
    .child({
        let style_events = events.style_picker();
        style_picker::render(&projection.resources.style_picker, &style_events, cx)
    });
    crate::molecules::inspector_section(cx)
        .child(header)
        .when(projection.presentation.expanded, |section| {
            section.child(render_content(projection, chrome, controls, events, cx))
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_layer_properties_ignore_a_projected_text_range() {
        let identity = TypographyIdentityProjection::new(
            "design".into(),
            DesignPanelTarget::Nodes {
                node_ids: vec!["text".into()],
            },
            DesignTypographyTarget::SelectedTextRangeRevision(7),
        );
        assert_eq!(
            identity.typography_target(DesignPanelProperty::FontSize),
            DesignTypographyTarget::SelectedTextRangeRevision(7)
        );
        assert_eq!(
            identity.typography_target(DesignPanelProperty::VerticalTextAlignment),
            DesignTypographyTarget::WholeLayer
        );
        assert_eq!(
            identity.typography_target(DesignPanelProperty::TextResize),
            DesignTypographyTarget::WholeLayer
        );
    }
}

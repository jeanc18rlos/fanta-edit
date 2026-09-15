//! Action, choice, and collection control recipes for inspectors.

use gpui::{
    App, Div, ElementId, InteractiveElement as _, Stateful, StatefulInteractiveElement as _,
    Styled as _,
};
use gpui_component::{ActiveTheme as _, h_flex};

use crate::atoms::CONTROL_KEY_CONTEXT;

use super::{InspectorFieldAccess, InspectorMetrics};

/// Horizontal group for related, directly visible actions.
pub fn inspector_action_group(metrics: InspectorMetrics) -> Div {
    h_flex()
        .min_w(gpui::px(0.))
        .items_center()
        .gap(metrics.row_gap)
}

/// Compact action-button baseline. Callers supply the icon/label and the one
/// domain-specific activation mapping.
pub fn inspector_action_button(
    id: impl Into<ElementId>,
    access: &InspectorFieldAccess,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    let mut control = h_flex()
        .id(id)
        .h(metrics.row_height)
        .min_w(metrics.row_height)
        .items_center()
        .justify_center()
        .gap(metrics.row_gap)
        .px(metrics.row_gap)
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .text_xs();
    if access.is_interactive() {
        control = control
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .active(|style| style.bg(cx.theme().secondary_active));
    }
    if access.is_editable() {
        control = control.focus(|style| style.border_color(cx.theme().selection));
    } else {
        control = control
            .text_color(cx.theme().muted_foreground)
            .opacity(0.62);
    }
    control
}

/// One shared border owner for a mutually exclusive compact choice group.
pub fn inspector_segmented_control(metrics: InspectorMetrics, cx: &App) -> Div {
    h_flex()
        .min_w(gpui::px(0.))
        .h(metrics.row_height)
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
}

/// Segment baseline. Individual segments own no border, preventing doubled
/// seams inside the group.
pub fn inspector_segment(
    id: impl Into<ElementId>,
    selected: bool,
    access: &InspectorFieldAccess,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    let mut segment = h_flex()
        .id(id)
        .h_full()
        .min_w(metrics.row_height)
        .flex_1()
        .items_center()
        .justify_center()
        .px(metrics.row_gap)
        .text_xs();
    if selected {
        segment = segment
            .bg(cx.theme().accent)
            .text_color(cx.theme().selection);
    }
    if access.is_interactive() {
        segment = segment
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.bg(cx.theme().accent));
    } else {
        segment = segment
            .text_color(cx.theme().muted_foreground)
            .opacity(0.62);
    }
    segment
}

/// Row-oriented collection baseline for paints, effects, exports, and other
/// inspector collections whose row is the interaction unit.
pub fn inspector_collection_row(
    id: impl Into<ElementId>,
    selected: bool,
    access: &InspectorFieldAccess,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    let mut row = h_flex()
        .id(id)
        .relative()
        .h(metrics.row_height)
        .w_full()
        .min_w(gpui::px(0.))
        .flex_none()
        .items_center()
        .gap(metrics.control_gap)
        .px(metrics.row_gap)
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .text_xs();
    if selected {
        row = row.bg(cx.theme().sidebar_accent);
    }
    if access.is_interactive() {
        row = row
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.55)))
            .focus(|style| style.bg(cx.theme().sidebar_accent));
    } else {
        row = row.text_color(cx.theme().muted_foreground).opacity(0.62);
    }
    row
}

//! Structural recipes for compact inspector sections and fields.

use gpui::{
    App, Div, ElementId, InteractiveElement as _, Stateful, Styled as _,
    prelude::FluentBuilder as _,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::atoms::CONTROL_KEY_CONTEXT;

use super::{
    InspectorFieldAccess, InspectorFieldPresentation, InspectorGridLayout, InspectorLabelPlacement,
    InspectorMetrics,
};

/// Section surface. The section owns the separator; its header and body do
/// not add competing borders.
pub fn inspector_section(cx: &App) -> Div {
    v_flex()
        .w_full()
        .min_w(gpui::px(0.))
        .flex_none()
        .border_b_1()
        .border_color(cx.theme().sidebar_border)
}

/// Groups adjacent inspector sections without adding another visual border.
/// Individual sections remain the sole owners of their separators.
pub fn inspector_section_group() -> Div {
    v_flex().w_full().min_w(gpui::px(0.)).flex_none()
}

/// Disclosure-header baseline for an inspector section.
pub fn inspector_section_header(
    id: impl Into<ElementId>,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    h_flex()
        .id(id)
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .h(metrics.section_header_height)
        .w_full()
        .min_w(gpui::px(0.))
        .flex_none()
        .items_center()
        .px(metrics.horizontal_padding)
        .gap(metrics.control_gap)
        .cursor_pointer()
        .border_1()
        .border_color(cx.theme().transparent)
        .hover(|style| style.bg(cx.theme().sidebar_accent.opacity(0.45)))
        .focus(|style| {
            style
                .bg(cx.theme().sidebar_accent.opacity(0.45))
                .border_color(cx.theme().selection)
        })
}

/// Vertical property-grid baseline. Label/control alignment is chosen once by
/// this container rather than hard-coded in every field.
pub fn inspector_field_grid(metrics: InspectorMetrics) -> Div {
    inspector_field_grid_with_layout(InspectorGridLayout::compact(metrics))
}

/// Property grid with one inherited density and label-placement policy.
pub fn inspector_field_grid_with_layout(layout: InspectorGridLayout) -> Div {
    v_flex()
        .w_full()
        .min_w(gpui::px(0.))
        .gap(layout.grid_gap())
        .px(layout.metrics.horizontal_padding)
}

/// Groups sibling fields that share one outline (for example X/Y or W/H).
/// Child field frames should remove their outer radius and border when
/// mounted here so this container remains the only seam owner.
pub fn inspector_field_group(metrics: InspectorMetrics, cx: &App) -> Div {
    h_flex()
        .w_full()
        .min_w(gpui::px(0.))
        .h(metrics.row_height)
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
}

/// One horizontally aligned property row.
pub fn inspector_row(metrics: InspectorMetrics) -> Div {
    inspector_row_with_layout(InspectorGridLayout::compact(metrics))
}

/// Row whose axis and height are inherited from its surrounding grid.
pub fn inspector_row_with_layout(layout: InspectorGridLayout) -> Div {
    match layout.label_placement {
        InspectorLabelPlacement::Leading => h_flex(),
        InspectorLabelPlacement::Stacked => v_flex(),
    }
    .w_full()
    .min_w(gpui::px(0.))
    .min_h(layout.row_height())
    .items_center()
    .gap(layout.metrics.control_gap)
}

/// Label recipe that follows the grid's placement policy.
pub fn inspector_field_label(layout: InspectorGridLayout) -> Div {
    gpui::div()
        .min_w(gpui::px(0.))
        .text_xs()
        .when(
            layout.label_placement == InspectorLabelPlacement::Leading,
            |label| label.w(layout.metrics.label_width).flex_none().truncate(),
        )
        .when(
            layout.label_placement == InspectorLabelPlacement::Stacked,
            |label| label.w_full(),
        )
}

/// Framing baseline shared by exact-entry fields and picker triggers.
pub fn inspector_field_frame(
    id: impl Into<ElementId>,
    access: &InspectorFieldAccess,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    let mut field = h_flex()
        .id(id)
        .h(metrics.row_height)
        .min_w(gpui::px(0.))
        .items_center()
        .gap(metrics.control_gap)
        .px(metrics.control_gap)
        .rounded(metrics.radius)
        .border_1()
        .border_color(cx.theme().transparent)
        .bg(cx.theme().secondary)
        .text_xs();

    if access.is_interactive() {
        field = field
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.border_color(cx.theme().muted_foreground))
            .focus(|style| {
                style
                    .bg(cx.theme().accent)
                    .border_color(cx.theme().selection)
            });
    }
    if !access.is_editable() {
        field = field.text_color(cx.theme().muted_foreground);
    }
    if !access.is_interactive() {
        field = field.opacity(0.62);
    }
    field
}

/// Field frame that consumes the complete presentation contract. Bound state
/// remains available to the caller for its binding affordance; invalid state
/// owns the frame border so validation never changes layout geometry.
pub fn inspector_field_frame_with_presentation(
    id: impl Into<ElementId>,
    presentation: &InspectorFieldPresentation,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    inspector_field_frame(id, &presentation.access, metrics, cx)
        .when(presentation.invalid, |field| {
            field.border_color(cx.theme().red)
        })
}

/// Field frame for a child mounted inside [`inspector_field_group`].
///
/// The group owns the only outline and clips its children to the shared
/// radius. Individual fields therefore use background changes for hover,
/// focus, and validation instead of adding doubled internal seams.
pub fn inspector_grouped_field_frame(
    id: impl Into<ElementId>,
    presentation: &InspectorFieldPresentation,
    metrics: InspectorMetrics,
    cx: &App,
) -> Stateful<Div> {
    let mut field = h_flex()
        .id(id)
        .h(metrics.row_height)
        .min_w(gpui::px(0.))
        .items_center()
        .gap(metrics.control_gap)
        .px(metrics.control_gap)
        .bg(cx.theme().secondary)
        .text_xs();

    if presentation.access.is_interactive() {
        field = field
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().secondary_hover))
            .focus(|style| style.bg(cx.theme().accent));
    }
    if !presentation.access.is_editable() {
        field = field.text_color(cx.theme().muted_foreground);
    }
    if !presentation.access.is_interactive() {
        field = field.opacity(0.62);
    }
    if presentation.invalid {
        field = field.bg(cx.theme().red.opacity(0.12));
    }
    field
}

//! Shared anchored-popup molecule for trigger-attached transient surfaces.
//!
//! The editor toolbar's tool flyouts, Actions palette, Agent composer, and
//! zoom menu each copy-adapted the same recipe: a viewport-clamped popover
//! surface rendered in a deferred layer that snaps inside the window with an
//! 8 px edge margin (ARCHITECTURE.md §12). This module owns that recipe once.
//! Callers keep `on_mouse_down_out` dismissal, sizing, padding, and content as
//! chained styles on [`popup_surface`], then wrap it in [`anchored_popup`].

use gpui::{
    Anchor, AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Point, Stateful, StatefulInteractiveElement as _, Styled as _,
    Window, anchored, deferred, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, v_flex};

/// Window-snap margin for anchored popups. Borders are painted half a pixel
/// beyond layout bounds, so snapping uses an extra half-pixel to keep the
/// rendered edge inside the 8 px inset (§12).
pub const POPUP_SAFE_MARGIN: f32 = 8.5;

/// Preferred popup width, clamped so both 8 px layout margins stay clear.
pub fn popup_width(window: &Window, preferred: f32) -> Pixels {
    px((window.viewport_size().width.as_f32() - 16.).clamp(1., preferred))
}

/// Tallest a popup can grow while both snap margins stay clear.
pub fn popup_max_height(window: &Window) -> Pixels {
    px((window.viewport_size().height.as_f32() - 2. * POPUP_SAFE_MARGIN).max(1.))
}

/// Preferred popup height, clamped by [`popup_max_height`].
pub fn popup_height(window: &Window, preferred: f32) -> Pixels {
    popup_max_height(window).min(px(preferred))
}

/// Popover chrome for a trigger-anchored transient surface.
///
/// Owns mouse blocking, the rounded border/popover fill/shadow treatment, and
/// vertical scrolling. Callers chain sizing (usually from [`popup_width`] and
/// [`popup_height`]), padding, dismissal, and children, then position the
/// result with [`anchored_popup`].
pub fn popup_surface(id: impl Into<ElementId>, radius: Pixels, cx: &App) -> Stateful<Div> {
    v_flex()
        .id(id)
        .block_mouse_except_scroll()
        .rounded(radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().popover_foreground)
        .when(cx.theme().shadow, |surface| surface.shadow_lg())
        .overflow_y_scroll()
}

/// Renders a popup surface in a deferred layer anchored to its trigger,
/// snapped inside the window on both axes (§12).
pub fn anchored_popup(
    anchor: Anchor,
    offset: Point<Pixels>,
    priority: usize,
    surface: impl IntoElement,
) -> AnyElement {
    deferred(
        anchored()
            .snap_to_window_with_margin(px(POPUP_SAFE_MARGIN))
            .anchor(anchor)
            .offset(offset)
            .child(surface),
    )
    .with_priority(priority)
    .into_any_element()
}

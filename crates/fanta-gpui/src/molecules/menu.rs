//! Shared context/popover menu molecule.
//!
//! Layers and Pages each grew a hand-rolled absolutely-positioned menu with
//! its own clamping and chrome. This module owns the shared pieces once:
//! [`clamp_menu_origin`] keeps a menu inside its container on both axes,
//! [`menu_surface`] is the popover chrome, and [`menu_item`] is the row
//! baseline. Callers keep ownership of dismissal, focus continuity, children,
//! and deferral: attach `on_mouse_down_out`, `track_scroll`, first-item
//! `track_focus`, and wrap the surface in `deferred(...).with_priority(...)`
//! at the call site.

use gpui::{
    App, Div, ElementId, InteractiveElement as _, Pixels, Point, Size, Stateful,
    StatefulInteractiveElement as _, Styled as _, point, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::atoms::CONTROL_KEY_CONTEXT;

/// Clamps `anchor` so a menu of size `menu` stays inside `container`.
///
/// Both axes clamp independently and floor at zero, so an oversized menu pins
/// to the container's top-left edge instead of escaping it.
pub fn clamp_menu_origin(
    anchor: Point<Pixels>,
    container: Size<Pixels>,
    menu: Size<Pixels>,
) -> Point<Pixels> {
    point(
        anchor
            .x
            .clamp(px(0.), (container.width - menu.width).max(px(0.))),
        anchor
            .y
            .clamp(px(0.), (container.height - menu.height).max(px(0.))),
    )
}

/// Shared popover-menu chrome positioned at a pre-clamped `origin`.
///
/// Returns the styled surface so the caller attaches a scroll handle,
/// `on_mouse_down_out` dismissal, and children, then wraps the result in
/// `deferred(...).with_priority(...)`.
pub fn menu_surface(
    id: impl Into<ElementId>,
    origin: Point<Pixels>,
    width: Pixels,
    max_height: Pixels,
    radius: Pixels,
    cx: &App,
) -> Stateful<Div> {
    v_flex()
        .id(id)
        .absolute()
        .left(origin.x)
        .top(origin.y)
        .block_mouse_except_scroll()
        .w(width)
        .max_h(max_height)
        .overflow_y_scroll()
        .py_2()
        .rounded(radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .when(cx.theme().shadow, |surface| surface.shadow_sm())
}

/// Menu-row baseline.
///
/// Menus keep the accent-fill focus treatment (no focus ring) so keyboard
/// traversal reads as the row highlight. First-item `track_focus` stays at the
/// call site, which owns menu focus continuity. The caller adds one
/// [`ControlExt::on_activate`] handler and children.
///
/// [`ControlExt::on_activate`]: crate::atoms::ControlExt::on_activate
pub fn menu_item(id: impl Into<ElementId>, height: Pixels, cx: &App) -> Stateful<Div> {
    h_flex()
        .id(id)
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .h(height)
        .flex_none()
        .mx_2()
        .px_2()
        .gap_2()
        .rounded(px(4.))
        .text_xs()
        .cursor_pointer()
        .hover(|style| style.bg(cx.theme().accent))
        .focus(|style| style.bg(cx.theme().accent))
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size};

    use super::clamp_menu_origin;

    #[test]
    fn interior_anchor_is_unchanged() {
        assert_eq!(
            clamp_menu_origin(
                point(px(50.), px(80.)),
                size(px(400.), px(600.)),
                size(px(224.), px(300.)),
            ),
            point(px(50.), px(80.))
        );
    }

    #[test]
    fn right_edge_clamps_x() {
        assert_eq!(
            clamp_menu_origin(
                point(px(350.), px(80.)),
                size(px(400.), px(600.)),
                size(px(224.), px(300.)),
            ),
            point(px(176.), px(80.))
        );
    }

    #[test]
    fn bottom_edge_clamps_y() {
        assert_eq!(
            clamp_menu_origin(
                point(px(50.), px(500.)),
                size(px(400.), px(600.)),
                size(px(224.), px(300.)),
            ),
            point(px(50.), px(300.))
        );
    }

    #[test]
    fn corner_clamps_both_axes() {
        assert_eq!(
            clamp_menu_origin(
                point(px(390.), px(590.)),
                size(px(400.), px(600.)),
                size(px(224.), px(300.)),
            ),
            point(px(176.), px(300.))
        );
    }

    #[test]
    fn oversized_menu_pins_to_container_origin() {
        assert_eq!(
            clamp_menu_origin(
                point(px(120.), px(40.)),
                size(px(400.), px(600.)),
                size(px(500.), px(700.)),
            ),
            point(px(0.), px(0.))
        );
    }

    #[test]
    fn negative_anchor_floors_at_zero() {
        assert_eq!(
            clamp_menu_origin(
                point(px(-20.), px(-10.)),
                size(px(400.), px(600.)),
                size(px(224.), px(300.)),
            ),
            point(px(0.), px(0.))
        );
    }
}

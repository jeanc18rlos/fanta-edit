//! Selectable list-row baseline shared by panel row surfaces.

use gpui::{App, Div, ElementId, InteractiveElement as _, Pixels, Stateful, Styled as _, px};
use gpui_component::{ActiveTheme as _, h_flex};

use crate::atoms::CONTROL_KEY_CONTEXT;

/// Baseline for a focusable, selectable list row.
///
/// Owns the §9 recipe the panel rows share: key context, tab stop, pointer
/// cursor, and a non-shifting focus ring (the border width is reserved while
/// unfocused so keyboard focus never moves layout). Hover and selected
/// treatments differ per panel and stay at the call site as chained styles,
/// as do activation handlers and children. The row is `relative()` so callers
/// can layer a bounds-tracking canvas inside it.
pub fn list_row(id: impl Into<ElementId>, height: Pixels, cx: &App) -> Stateful<Div> {
    h_flex()
        .id(id)
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .relative()
        .h(height)
        .w_full()
        .flex_none()
        .rounded(px(4.))
        .cursor_pointer()
        .border_1()
        .border_color(cx.theme().transparent)
        .focus(|style| style.border_color(cx.theme().selection))
}

use gpui::{
    App, Div, ElementId, InteractiveElement as _, Pixels, Stateful,
    StatefulInteractiveElement as _, Styled as _, div,
};
use gpui_component::ActiveTheme as _;

use super::CONTROL_KEY_CONTEXT;

/// Complete baseline for compact square icon controls.
///
/// Owns the full §9 control recipe: key context, tab stop, pointer cursor,
/// hover fill, a pressed fill (`secondary_active`, the same token the
/// gpui-component Button uses, so holding the pointer down visibly deepens
/// the control past its hover state), a non-shifting focus ring (the border
/// width is reserved while unfocused so keyboard focus never moves layout),
/// and flex centering of the child in both axes. Consumers add selected
/// styling, children, and one [`ControlExt::on_activate`] handler.
///
/// [`ControlExt::on_activate`]: super::ControlExt::on_activate
pub fn icon_button(
    id: impl Into<ElementId>,
    size: Pixels,
    radius: Pixels,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .key_context(CONTROL_KEY_CONTEXT)
        .tab_index(0)
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius)
        .cursor_pointer()
        .border_1()
        .border_color(cx.theme().transparent)
        .hover(|style| style.bg(cx.theme().accent))
        .active(|style| style.bg(cx.theme().secondary_active))
        .focus(|style| style.border_color(cx.theme().selection))
}

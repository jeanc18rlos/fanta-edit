//! Shared composition for component-authoring dialogs.
//!
//! Callers provide already-wired header, content, and footer elements. This
//! module owns only the dialog geometry and chrome, keeping entity callbacks
//! and Design actions in the panel façade.

use gpui::{
    AnyElement, App, Div, IntoElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use gpui_component::{ActiveTheme as _, dialog::Dialog, v_flex};

use crate::molecules::popup_width;

pub(super) enum ComponentAuthoringDialogSpacing {
    Compact,
    Relaxed,
}

pub(super) struct ComponentAuthoringDialogView {
    pub(super) header: AnyElement,
    pub(super) content: AnyElement,
    pub(super) footer: AnyElement,
    pub(super) spacing: ComponentAuthoringDialogSpacing,
}

pub(super) fn configure_host(dialog: Dialog, window: &Window) -> Dialog {
    dialog
        .w(popup_width(window, 440.))
        .max_w(popup_width(window, 560.))
        .overlay_closable(false)
}

pub(super) fn render(view: ComponentAuthoringDialogView, cx: &App) -> AnyElement {
    render_container(
        v_flex()
            .child(view.header)
            .child(view.content)
            .child(view.footer),
        view.spacing,
        cx,
    )
}

pub(super) fn render_container(
    shell: Div,
    spacing: ComponentAuthoringDialogSpacing,
    cx: &App,
) -> AnyElement {
    let shell = shell
        .w_full()
        .p_3()
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .shadow_lg();
    let shell = match spacing {
        ComponentAuthoringDialogSpacing::Compact => shell.gap_2(),
        ComponentAuthoringDialogSpacing::Relaxed => shell.gap_3(),
    };
    shell.into_any_element()
}

pub(super) fn unavailable_editor(cx: &App) -> AnyElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child("This component-property editor is no longer available.")
        .into_any_element()
}

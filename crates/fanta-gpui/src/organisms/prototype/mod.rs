//! Host-controlled Prototype inspector matching Figma's empty-selection state.

use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rgba,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex,
};

use crate::{
    atoms::{
        CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon, icon_button, render_control_icon,
        truncating_label,
    },
    color::parse_hex_rgba,
};

/// Narrowest width the panel lays out without clipping its header tabs and
/// settings rows: the device name truncates and the hint copy wraps below
/// the 473px reference design (ARCHITECTURE.md §5).
pub const PROTOTYPE_PANEL_MIN_WIDTH: f32 = 300.;
/// Shortest height the panel stays useful at; hint cards wrap their copy.
pub const PROTOTYPE_PANEL_MIN_HEIGHT: f32 = 400.;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrototypePanelSurface {
    Design,
    #[default]
    Prototype,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrototypeHint {
    CreatingConnection,
    RunningPrototype,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrototypeViewData {
    pub surface: PrototypePanelSurface,
    pub zoom_percent: u16,
    pub device_name: SharedString,
    pub background_hex: SharedString,
}

impl Default for PrototypeViewData {
    fn default() -> Self {
        Self {
            surface: PrototypePanelSurface::Prototype,
            zoom_percent: 12,
            device_name: "No device".into(),
            background_hex: "000000".into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrototypePanelAction {
    SurfaceChangeRequested { surface: PrototypePanelSurface },
    ZoomMenuRequested,
    DeviceMenuRequested,
    BackgroundEditRequested,
    HintDismissed { hint: PrototypeHint },
}

pub struct PrototypePanel {
    id: SharedString,
    focus_handle: FocusHandle,
    view_data: PrototypeViewData,
    show_connection_hint: bool,
    show_running_hint: bool,
}

impl EventEmitter<PrototypePanelAction> for PrototypePanel {}

impl PrototypePanel {
    pub fn new(
        id: impl Into<SharedString>,
        view_data: PrototypeViewData,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            view_data,
            show_connection_hint: true,
            show_running_hint: true,
        }
    }

    pub fn set_view_data(&mut self, view_data: PrototypeViewData, cx: &mut Context<Self>) {
        self.view_data = view_data;
        cx.notify();
    }

    pub fn restore_hints(&mut self, cx: &mut Context<Self>) {
        self.show_connection_hint = true;
        self.show_running_hint = true;
        cx.notify();
    }

    fn background_rgba(hex: &str) -> u32 {
        parse_hex_rgba(hex).unwrap_or(0x000000ff)
    }

    fn render_surface_tab(
        &self,
        surface: PrototypePanelSurface,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.view_data.surface == surface;
        div()
            .id(SharedString::from(format!(
                "{}-surface-{}",
                self.id,
                label.to_lowercase()
            )))
            .debug_selector(move || format!("prototype-surface-{}", label.to_lowercase()))
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(24.))
            .px(px(7.))
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(if selected {
                cx.theme().tab_active_foreground
            } else {
                cx.theme().muted_foreground
            })
            .when(selected, |tab| {
                tab.bg(cx.theme().tab_active).font_semibold()
            })
            .border_1()
            .border_color(cx.theme().transparent)
            .hover(|style| style.bg(cx.theme().accent))
            .focus(|style| style.border_color(cx.theme().selection))
            .on_activate(cx.listener(move |_, _, _, cx| {
                cx.emit(PrototypePanelAction::SurfaceChangeRequested { surface });
            }))
            .child(div().relative().top(px(3.)).child(label))
            .into_any_element()
    }

    fn render_hint(
        &self,
        hint: PrototypeHint,
        title: &'static str,
        body: &'static str,
        icon: ControlIcon,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dismiss_selector = match hint {
            PrototypeHint::CreatingConnection => "prototype-dismiss-connection",
            PrototypeHint::RunningPrototype => "prototype-dismiss-running",
        };
        v_flex()
            .gap(px(15.5))
            .child(
                h_flex()
                    .w_full()
                    .child(div().font_semibold().text_size(px(11.)).child(title))
                    .child(div().flex_1())
                    .child(
                        icon_button(
                            SharedString::from(format!("{}-dismiss-{hint:?}", self.id)),
                            px(24.),
                            px(4.),
                            cx,
                        )
                        .debug_selector(move || dismiss_selector.to_owned())
                        .relative()
                        .left(px(8.))
                        .top(px(4.5))
                        .on_activate(cx.listener(move |this, _, _, cx| {
                            match hint {
                                PrototypeHint::CreatingConnection => {
                                    this.show_connection_hint = false;
                                }
                                PrototypeHint::RunningPrototype => {
                                    this.show_running_hint = false;
                                }
                            }
                            cx.emit(PrototypePanelAction::HintDismissed { hint });
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::Close).with_size(px(17.5))),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(0.))
                    .items_start()
                    .gap(px(16.))
                    .child(div().w(px(16.)).flex_none().child(render_control_icon(
                        icon,
                        cx.theme().foreground,
                        16.,
                    )))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .child(body),
                    ),
            )
            .into_any_element()
    }
}

impl Focusable for PrototypePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PrototypePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .size_full()
            .min_h(px(0.))
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(px(34.))
                    .items_start()
                    .pt(px(1.))
                    .px(px(10.))
                    .gap(px(4.))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(self.render_surface_tab(
                        PrototypePanelSurface::Design,
                        "Design",
                        cx,
                    ))
                    .child(self.render_surface_tab(
                        PrototypePanelSurface::Prototype,
                        "Prototype",
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-zoom", self.id)))
                            .debug_selector(|| "prototype-zoom".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .relative()
                            .left(px(-1.5))
                            .top(px(-1.))
                            .h(px(26.))
                            .gap_1()
                            .rounded(px(5.))
                            .cursor_pointer()
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(PrototypePanelAction::ZoomMenuRequested);
                            }))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .child(format!("{}%", self.view_data.zoom_percent)),
                            )
                            .child(Icon::new(IconName::ChevronDown).xsmall()),
                    ),
            )
            .child(
                v_flex()
                    .px(px(16.))
                    .pt(px(11.5))
                    .pb(px(16.))
                    .gap(px(8.))
                    .child(
                        div()
                            .font_semibold()
                            .text_size(px(11.))
                            .child("Prototype settings"),
                    )
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-device", self.id)))
                            .debug_selector(|| "prototype-device".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .mt(px(6.5))
                            .h(px(24.))
                            .w_full()
                            .px(px(8.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .text_size(px(11.))
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(PrototypePanelAction::DeviceMenuRequested);
                            }))
                            // The device name is host data of any length;
                            // it truncates instead of widening the row.
                            .child(truncating_label(self.view_data.device_name.clone()))
                            .child(div().flex_none().child(Icon::new(IconName::ChevronDown).xsmall())),
                    )
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("{}-background", self.id)))
                            .debug_selector(|| "prototype-background".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(24.))
                            .w_full()
                            .px(px(5.))
                            .gap(px(4.5))
                            .rounded(px(6.))
                            .bg(cx.theme().secondary)
                            .cursor_pointer()
                            .border_1()
                            .border_color(cx.theme().transparent)
                            .hover(|style| style.bg(cx.theme().secondary_hover))
                            .focus(|style| style.border_color(cx.theme().selection))
                            .on_activate(cx.listener(|_, _, _, cx| {
                                cx.emit(PrototypePanelAction::BackgroundEditRequested);
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .rounded(px(3.))
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .bg(rgba(Self::background_rgba(
                                        &self.view_data.background_hex,
                                    ))),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .child(self.view_data.background_hex.clone()),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(0.))
                    .px(px(16.))
                    .pt(px(7.5))
                    .gap(px(25.))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .when(self.show_connection_hint, |content| {
                        content.child(self.render_hint(
                            PrototypeHint::CreatingConnection,
                            "Creating a connection",
                            "Select a frame or object in a frame and use the circular node to drag a connection to another frame.",
                            ControlIcon::JumpArrow,
                            cx,
                        ))
                    })
                    .when(self.show_running_hint, |content| {
                        content.child(self.render_hint(
                            PrototypeHint::RunningPrototype,
                            "Running your prototype",
                            "Use the play button in the toolbar to play your prototype. If there are no connections, the play button can be used to play a presentation of your frames.",
                            ControlIcon::Play,
                            cx,
                        ))
                    }),
            )
    }
}

#[cfg(test)]
mod interaction_tests;

#[cfg(test)]
mod tests {
    use super::PrototypePanel;

    #[test]
    fn prototype_background_accepts_rgb_and_rgba_hex() {
        assert_eq!(PrototypePanel::background_rgba("336699"), 0x336699ff);
        assert_eq!(PrototypePanel::background_rgba("#33669980"), 0x33669980);
        assert_eq!(PrototypePanel::background_rgba("AbC"), 0xaabbccff);
        assert_eq!(PrototypePanel::background_rgba("#abcd"), 0xaabbccdd);
    }

    #[test]
    fn prototype_background_uses_black_for_invalid_host_values() {
        assert_eq!(PrototypePanel::background_rgba(""), 0x000000ff);
        assert_eq!(PrototypePanel::background_rgba("not-a-color"), 0x000000ff);
        assert_eq!(PrototypePanel::background_rgba("#12345"), 0x000000ff);
    }
}

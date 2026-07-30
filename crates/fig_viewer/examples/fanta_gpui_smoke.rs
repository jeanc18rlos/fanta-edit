//! Interactive smoke check for the fanta-gpui platform unification.
//!
//! Run: `cargo run -p fig_viewer --example fanta_gpui_smoke --features fanta-gpui-ui`
//!
//! One window proving, in a single glance:
//! - Button hover/active (Anchor-based corner styling)
//! - Input with blinking cursor (smol Timer port) and editing
//!   (tree-sitter 0.26 InputEdit path)
//! - A non-colliding icon (lucide `inspector`) and a colliding one
//!   (`check`, deliberately resolving to the app's glyph)
//! - A Popover (Anchor positioning + deferred layering without Root)
//! - The gpui_component dark palette

use gpui::{
    App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
    WindowOptions, div, px, size,
};
use gpui_component::input::{Input, InputState};
use gpui_component::popover::Popover;
use gpui_component::theme::{Theme, ThemeMode};
use gpui_component::{ActiveTheme as _, Icon, IconName, button::Button, h_flex, v_flex};

struct Smoke {
    input: gpui::Entity<InputState>,
}

impl Smoke {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type here — cursor must blink"));
        Self { input }
    }
}

impl Render for Smoke {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child("fanta-gpui smoke: hover the button, type in the input, open the popover")
            .child(
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(Button::new("smoke-button").label("Hover me"))
                    .child(Icon::new(IconName::Inspector))
                    .child(Icon::new(IconName::Check))
                    .child(
                        Popover::new("smoke-popover")
                            .trigger(Button::new("smoke-popover-trigger").label("Popover"))
                            .content(|_, _, _| div().p_2().child("Anchored without Root")),
                    ),
            )
            .child(div().w(px(360.)).child(Input::new(&self.input)))
    }
}

fn main() {
    let app = gpui_platform::application();
    app.run(|cx: &mut App| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(720.), px(420.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Smoke::new(window, cx)),
        )
        .expect("open smoke window");
        cx.activate(true);
    });
}

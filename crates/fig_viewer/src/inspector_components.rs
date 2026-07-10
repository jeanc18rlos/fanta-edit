use gpui::{AnyElement, App, IntoElement, RenderOnce, SharedString, Window, px};
use ui::prelude::*;

const SECTION_HEADER_HEIGHT: f32 = 28.0;
const PROPERTY_LABEL_WIDTH: f32 = 76.0;

#[derive(IntoElement)]
pub struct InspectorSectionHeader {
    title: SharedString,
    action: Option<AnyElement>,
}

impl InspectorSectionHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for InspectorSectionHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .px_4()
            .h(px(SECTION_HEADER_HEIGHT))
            .items_center()
            .justify_between()
            .child(
                Label::new(self.title)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .children(self.action)
    }
}

#[derive(IntoElement)]
pub struct InspectorPropertyRow {
    label: SharedString,
    content: AnyElement,
    inset: bool,
    label_width: f32,
}

impl InspectorPropertyRow {
    pub fn new(label: impl Into<SharedString>, content: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            content: content.into_any_element(),
            inset: true,
            label_width: PROPERTY_LABEL_WIDTH,
        }
    }

    pub fn inset(mut self, inset: bool) -> Self {
        self.inset = inset;
        self
    }

    pub fn label_width(mut self, width: f32) -> Self {
        self.label_width = width;
        self
    }
}

impl RenderOnce for InspectorPropertyRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .gap_2()
            .items_center()
            .when(self.inset, |row| row.px_4())
            .child(
                div().w(px(self.label_width)).flex_none().child(
                    Label::new(self.label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .single_line(),
                ),
            )
            .child(div().flex_1().min_w_0().child(self.content))
    }
}

#[derive(IntoElement)]
pub struct InspectorMessage {
    message: SharedString,
}

impl InspectorMessage {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl RenderOnce for InspectorMessage {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .px_4()
            .child(Label::new(self.message).color(Color::Muted))
    }
}

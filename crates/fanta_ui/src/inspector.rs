use gpui::{
    AnyElement, App, ElementId, FontWeight, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div,
};
use ui::prelude::*;

#[derive(IntoElement)]
pub struct InspectorSection {
    id: ElementId,
    title: SharedString,
    action: Option<AnyElement>,
    children: Vec<AnyElement>,
    separated: bool,
}

impl InspectorSection {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            action: None,
            children: Vec::new(),
            separated: true,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.push(child.into_any_element());
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.children
            .extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }

    pub fn separated(mut self, separated: bool) -> Self {
        self.separated = separated;
        self
    }
}

impl RenderOnce for InspectorSection {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let header = h_flex()
            .h_8()
            .px_4()
            .justify_between()
            .child(
                Label::new(self.title)
                    .size(LabelSize::Small)
                    .weight(FontWeight::SEMIBOLD)
                    .line_height_style(LineHeightStyle::UiLabel),
            )
            .children(self.action);
        div()
            .id(self.id)
            .w_full()
            .when(self.separated, |section| {
                section
                    .border_t_1()
                    .border_color(cx.theme().colors().border_variant)
            })
            .child(header)
            .children(self.children)
    }
}

#[derive(IntoElement)]
pub struct InspectorFieldRow {
    label: SharedString,
    control: AnyElement,
}

impl InspectorFieldRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            control: control.into_any_element(),
        }
    }
}

impl RenderOnce for InspectorFieldRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        h_flex()
            .min_h_8()
            .px_4()
            .gap_2()
            .items_center()
            .child(
                div().w(px(72.)).flex_none().child(
                    Label::new(self.label)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .single_line(),
                ),
            )
            .child(div().flex_1().min_w_0().child(self.control))
    }
}

#[derive(IntoElement)]
pub struct InspectorEmptyState {
    title: SharedString,
    message: SharedString,
}

impl InspectorEmptyState {
    pub fn new(title: impl Into<SharedString>, message: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }
}

impl RenderOnce for InspectorEmptyState {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        v_flex()
            .px_4()
            .py_3()
            .gap_1()
            .child(Label::new(self.title).size(LabelSize::Small))
            .child(
                Label::new(self.message)
                    .size(LabelSize::XSmall)
                    .color(Color::Muted)
                    .line_clamp(3),
            )
    }
}

use std::{rc::Rc, time::Duration};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, IntoElement, RenderOnce, SharedString, Window,
    ease_out_quint, px,
};
use ui::prelude::*;

const SECTION_HEADER_HEIGHT: f32 = 28.0;
const PROPERTY_LABEL_WIDTH: f32 = 76.0;

type TabHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct CollapsibleIconTab {
    scope: &'static str,
    index: usize,
    icon: IconName,
    label: SharedString,
    selected: bool,
    on_click: TabHandler,
}

impl CollapsibleIconTab {
    pub fn new(
        scope: &'static str,
        index: usize,
        icon: IconName,
        label: impl Into<SharedString>,
        selected: bool,
        on_click: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            scope,
            index,
            icon,
            label: label.into(),
            selected,
            on_click: Rc::new(on_click),
        }
    }
}

impl RenderOnce for CollapsibleIconTab {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selected = self.selected;
        let collapsed_width = 32.0;
        let expanded_width = 36.0 + self.label.chars().count() as f32 * 7.0;
        let on_click = self.on_click;
        let label = div().child(Label::new(self.label).size(LabelSize::Small).single_line());
        #[cfg(test)]
        let label = label.debug_selector(|| {
            format!("fanta-collapsible-tab-label-{}-{}", self.scope, self.index)
        });
        let label = div()
            .min_w_0()
            .overflow_hidden()
            .when(selected, |container| container.child(label));
        let tab = h_flex()
            .id((self.scope, self.index))
            .h(px(28.0))
            .flex_none()
            .items_center()
            .gap_1()
            .px_2()
            .rounded_md()
            .overflow_hidden()
            .cursor_pointer()
            .when(selected, |tab| {
                tab.bg(cx.theme().colors().element_selected)
                    .text_color(cx.theme().colors().text)
            })
            .when(!selected, |tab| {
                tab.text_color(cx.theme().colors().text_muted)
                    .hover(|tab| tab.bg(cx.theme().colors().element_hover))
            })
            .on_click(move |_, window, cx| on_click(window, cx))
            .child(Icon::new(self.icon).size(IconSize::Small))
            .child(label);
        let animation_state = if selected { "expand" } else { "collapsed" };

        tab.with_animation(
            (
                gpui::ElementId::from((self.scope, self.index)),
                animation_state,
            ),
            Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
            move |tab, delta| {
                let width = if selected {
                    collapsed_width + (expanded_width - collapsed_width) * delta
                } else {
                    expanded_width + (collapsed_width - expanded_width) * delta
                };
                tab.w(px(width))
            },
        )
    }
}

#[cfg(test)]
mod tab_tests {
    use gpui::{Context, Render, TestAppContext};

    use super::*;

    struct TabHarness;

    impl Render for TabHarness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .child(CollapsibleIconTab::new(
                    "collapsed-test",
                    0,
                    IconName::ToolFrame,
                    "Canvas",
                    false,
                    |_, _| {},
                ))
                .child(CollapsibleIconTab::new(
                    "selected-test",
                    0,
                    IconName::DatabaseZap,
                    "Variables",
                    true,
                    |_, _| {},
                ))
        }
    }

    #[gpui::test]
    fn collapsed_tab_omits_its_label_from_the_element_tree(cx: &mut TestAppContext) {
        cx.update(|cx| {
            assets::Assets.load_test_fonts(cx);
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
        });

        let window = cx.add_window(|_, _| TabHarness);
        let mut visual_context = gpui::VisualTestContext::from_window(window.into(), cx);
        visual_context.update(|window, cx| window.draw(cx).clear());

        assert!(
            visual_context
                .debug_bounds("fanta-collapsible-tab-label-collapsed-test-0")
                .is_none()
        );
        assert!(
            visual_context
                .debug_bounds("fanta-collapsible-tab-label-selected-test-0")
                .is_some()
        );
    }
}

#[derive(IntoElement)]
pub struct CollapsibleIconTabBar {
    id: &'static str,
    tabs: Vec<AnyElement>,
}

impl CollapsibleIconTabBar {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            tabs: Vec::new(),
        }
    }

    pub fn tab(mut self, tab: CollapsibleIconTab) -> Self {
        self.tabs.push(tab.into_any_element());
        self
    }
}

impl RenderOnce for CollapsibleIconTabBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .id(self.id)
            .h(px(34.0))
            .p_0p5()
            .gap_0p5()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .shadow_sm()
            .children(self.tabs)
    }
}

#[derive(IntoElement)]
pub struct InspectorSectionHeader {
    title: SharedString,
    action: Option<AnyElement>,
    top_border: bool,
}

impl InspectorSectionHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            action: None,
            top_border: true,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    /// The owning panel supplies the separator. This prevents two adjacent
    /// hairlines when section layout and the reusable header are composed.
    pub fn without_top_border(mut self) -> Self {
        self.top_border = false;
        self
    }
}

impl RenderOnce for InspectorSectionHeader {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .px_4()
            .h(px(SECTION_HEADER_HEIGHT))
            .items_center()
            .justify_between()
            .when(self.top_border, |header| {
                header.border_t_1().border_color(cx.theme().colors().border)
            })
            .child(
                Label::new(self.title)
                    .size(LabelSize::Small)
                    .weight(gpui::FontWeight::BOLD),
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

#[cfg(test)]
mod tests {
    use super::InspectorSectionHeader;

    #[test]
    fn section_owner_can_disable_the_headers_builtin_separator() {
        assert!(InspectorSectionHeader::new("Standalone").top_border);
        assert!(
            !InspectorSectionHeader::new("Stacked")
                .without_top_border()
                .top_border
        );
    }
}

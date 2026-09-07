//! Retained, searchable text-style picker used by the Typography section.
//!
//! The picker owns only transient search, focus, and keyboard-highlight state.
//! Page/library data and the current style binding remain host-controlled, and
//! every apply or detach operation is emitted as a typed candidate event.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Point, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{Scrollbar, ScrollbarAxis},
    v_flex,
};

use crate::atoms::ButtonControlExt as _;
use crate::molecules::{popup_height, popup_width};

use super::{
    CancelDesignInteraction, DESIGN_PANEL_KEY_CONTEXT, DesignTypographyStyle,
    DesignTypographyStyleBinding, DesignTypographyStyleSelection, DesignTypographyStyleSource,
    DesignTypographyStyleViewData,
};

const PICKER_WIDTH: f32 = 304.;
const PICKER_MAX_HEIGHT: f32 = 520.;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TypographyStylePickerEvent {
    ApplyRequested {
        style: DesignTypographyStyleSelection,
    },
    DetachRequested {
        style: DesignTypographyStyleSelection,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct FilteredStyle {
    selection: DesignTypographyStyleSelection,
}

/// Retained presentation state for the Typography style browser.
pub(crate) struct TypographyStylePicker {
    id: SharedString,
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    scroll_handle: ScrollHandle,
    view_data: DesignTypographyStyleViewData,
    current_binding: Option<DesignTypographyStyleBinding>,
    disabled: bool,
    highlighted_index: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TypographyStylePickerEvent> for TypographyStylePicker {}

impl Focusable for TypographyStylePicker {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TypographyStylePicker {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search text styles"));
        let search_subscription = cx.subscribe_in(
            &search_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.highlighted_index = 0;
                    this.scroll_handle.scroll_to_item(0);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.request_highlighted(cx);
                    window.prevent_default();
                    cx.stop_propagation();
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        );

        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            search_input,
            scroll_handle: ScrollHandle::new(),
            view_data: DesignTypographyStyleViewData::default(),
            current_binding: None,
            disabled: false,
            highlighted_index: 0,
            _subscriptions: vec![search_subscription],
        }
    }

    pub(crate) fn set_view_data(
        &mut self,
        view_data: DesignTypographyStyleViewData,
        cx: &mut Context<Self>,
    ) {
        if self.view_data != view_data {
            self.view_data = view_data;
            self.clamp_highlighted_index(cx);
            cx.notify();
        }
    }

    pub(crate) fn set_current_binding(
        &mut self,
        binding: Option<DesignTypographyStyleBinding>,
        cx: &mut Context<Self>,
    ) {
        if self.current_binding != binding {
            self.current_binding = binding;
            cx.notify();
        }
    }

    pub(crate) fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        if self.disabled != disabled {
            self.disabled = disabled;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) fn prepare_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.highlighted_index = 0;
        self.scroll_handle.set_offset(Point::default());
        self.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn request_apply(
        &self,
        selection: DesignTypographyStyleSelection,
        cx: &mut Context<Self>,
    ) {
        if self.disabled || self.view_data.style(&selection).is_none() {
            return;
        }
        cx.emit(TypographyStylePickerEvent::ApplyRequested { style: selection });
    }

    pub(crate) fn request_detach(&self, cx: &mut Context<Self>) {
        let Some(binding) = self
            .current_binding
            .as_ref()
            .filter(|binding| binding.can_detach && !self.disabled)
        else {
            return;
        };
        cx.emit(TypographyStylePickerEvent::DetachRequested {
            style: binding.selection.clone(),
        });
    }

    fn normalized_query(&self, cx: &App) -> String {
        self.search_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase()
    }

    fn filtered_styles(&self, cx: &App) -> Vec<FilteredStyle> {
        let query = self.normalized_query(cx);
        let mut rows = self
            .view_data
            .page_styles
            .iter()
            .filter(|style| style_matches_query(style, None, &query))
            .map(|style| FilteredStyle {
                selection: DesignTypographyStyleSelection::page(style.id.clone()),
            })
            .collect::<Vec<_>>();
        for library in &self.view_data.libraries {
            rows.extend(
                library
                    .styles
                    .iter()
                    .filter(|style| style_matches_query(style, Some(&library.name), &query))
                    .map(|style| FilteredStyle {
                        selection: DesignTypographyStyleSelection::library(
                            library.id.clone(),
                            style.id.clone(),
                        ),
                    }),
            );
        }
        rows
    }

    fn clamp_highlighted_index(&mut self, cx: &App) {
        self.highlighted_index = self
            .highlighted_index
            .min(self.filtered_styles(cx).len().saturating_sub(1));
    }

    fn request_highlighted(&self, cx: &mut Context<Self>) {
        if let Some(row) = self.filtered_styles(cx).get(self.highlighted_index) {
            self.request_apply(row.selection.clone(), cx);
        }
    }

    fn move_highlight(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.filtered_styles(cx).len();
        if count == 0 {
            self.highlighted_index = 0;
            return;
        }
        self.highlighted_index = self
            .highlighted_index
            .saturating_add_signed(delta)
            .min(count - 1);
        self.scroll_handle.scroll_to_item(self.highlighted_index);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "up" => self.move_highlight(-1, cx),
            "down" => self.move_highlight(1, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }

    fn render_current_binding(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let binding = self.current_binding.as_ref()?;
        let can_detach = binding.can_detach && !self.disabled;
        let reason = if self.disabled {
            "Typography is read only"
        } else if !binding.can_detach {
            "This style binding cannot be detached"
        } else {
            "Detach style"
        };

        Some(
            h_flex()
                .w_full()
                .min_h(px(36.))
                .px_2()
                .gap_2()
                .rounded(px(5.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .child(format!("Applied · {}", binding.name)),
                )
                .child(
                    Button::new(SharedString::from(format!("{}-detach", self.id)))
                        .label("Detach")
                        .tooltip(reason)
                        .xsmall()
                        .compact()
                        .ghost()
                        .disabled(!can_detach)
                        .on_activate(cx.listener(|this, _, _, cx| {
                            this.request_detach(cx);
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_style_row(
        &self,
        style: &DesignTypographyStyle,
        selection: DesignTypographyStyleSelection,
        flat_index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self
            .current_binding
            .as_ref()
            .is_some_and(|binding| binding.selection == selection);
        let highlighted = flat_index == self.highlighted_index;
        let metadata = format!(
            "{} {} · {}",
            style.family,
            style.font_style,
            format_style_size(style.size)
        );
        let selection_for_click = selection.clone();
        let row_id = match &selection.source {
            DesignTypographyStyleSource::Page => {
                format!("{}-page-style-{}", self.id, style.id)
            }
            DesignTypographyStyleSource::Library { library_id } => {
                format!("{}-library-{library_id}-style-{}", self.id, style.id)
            }
        };

        Button::new(SharedString::from(row_id))
            .tooltip(style.name.clone())
            .w_full()
            .h(px(48.))
            .px_2()
            .compact()
            .ghost()
            .selected(selected)
            .disabled(self.disabled)
            .when(highlighted && !selected, |button| {
                button.bg(cx.theme().accent.opacity(0.72))
            })
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(0.))
                    .gap_2()
                    .child(
                        div()
                            .size(px(28.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .text_sm()
                            .font_semibold()
                            .child("Aa"),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(0.))
                            .gap_0p5()
                            .child(
                                div()
                                    .truncate()
                                    .text_left()
                                    .text_xs()
                                    .font_semibold()
                                    .child(style.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_left()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(metadata),
                            ),
                    )
                    .when(selected, |row| {
                        row.child(Icon::new(IconName::Check).xsmall())
                    }),
            )
            .on_activate(cx.listener(move |this, _, _, cx| {
                this.request_apply(selection_for_click.clone(), cx);
            }))
            .into_any_element()
    }

    fn render_empty_state(
        &self,
        title: &'static str,
        detail: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .w_full()
            .min_h(px(180.))
            .items_center()
            .justify_center()
            .gap_2()
            .p_4()
            .child(
                Icon::new(IconName::BookOpen)
                    .small()
                    .text_color(cx.theme().muted_foreground),
            )
            .child(div().text_sm().font_semibold().child(title))
            .child(
                div()
                    .max_w(px(224.))
                    .text_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_style_groups(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.view_data.is_empty() {
            return self.render_empty_state(
                "Text styles",
                "No page or library text styles supplied by the host.".into(),
                cx,
            );
        }

        let query = self.normalized_query(cx);
        let filtered = self.filtered_styles(cx);
        if filtered.is_empty() && !query.is_empty() {
            return self.render_empty_state(
                "No matching styles",
                format!(
                    "No text styles match “{}”.",
                    self.search_input.read(cx).value()
                )
                .into(),
                cx,
            );
        }

        let mut flat_index = 0;
        let mut content = v_flex().w_full().gap_3().p_3();
        let page_styles = self
            .view_data
            .page_styles
            .iter()
            .filter(|style| style_matches_query(style, None, &query))
            .collect::<Vec<_>>();
        if query.is_empty() || !page_styles.is_empty() {
            let mut page_group = v_flex().w_full().gap_1().child(
                h_flex()
                    .h(px(24.))
                    .w_full()
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .font_semibold()
                            .child("On this page"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(page_styles.len().to_string()),
                    ),
            );
            if page_styles.is_empty() {
                page_group = page_group.child(
                    div()
                        .min_h(px(28.))
                        .flex()
                        .items_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No page text styles supplied by the host"),
                );
            } else {
                for style in page_styles {
                    page_group = page_group.child(self.render_style_row(
                        style,
                        DesignTypographyStyleSelection::page(style.id.clone()),
                        flat_index,
                        cx,
                    ));
                    flat_index += 1;
                }
            }
            content = content.child(page_group);
        }

        for library in &self.view_data.libraries {
            let styles = library
                .styles
                .iter()
                .filter(|style| style_matches_query(style, Some(&library.name), &query))
                .collect::<Vec<_>>();
            if !query.is_empty() && styles.is_empty() {
                continue;
            }
            let mut group = v_flex().w_full().gap_1().child(
                h_flex()
                    .h(px(24.))
                    .w_full()
                    .gap_2()
                    .child(
                        Icon::new(IconName::BookOpen)
                            .xsmall()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_xs()
                            .font_semibold()
                            .child(library.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(styles.len().to_string()),
                    ),
            );
            if styles.is_empty() {
                group = group.child(
                    div()
                        .pl_6()
                        .min_h(px(28.))
                        .flex()
                        .items_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No available text styles"),
                );
            } else {
                for style in styles {
                    group = group.child(self.render_style_row(
                        style,
                        DesignTypographyStyleSelection::library(
                            library.id.clone(),
                            style.id.clone(),
                        ),
                        flat_index,
                        cx,
                    ));
                    flat_index += 1;
                }
            }
            content = content.child(group);
        }
        content.into_any_element()
    }
}

impl Render for TypographyStylePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scroll_handle = self.scroll_handle.clone();
        v_flex()
            .id(self.id.clone())
            .key_context(DESIGN_PANEL_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .w(popup_width(window, PICKER_WIDTH))
            .max_h(popup_height(window, PICKER_MAX_HEIGHT))
            .overflow_hidden()
            .rounded(px(10.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .shadow_lg()
            .child(
                h_flex()
                    .h(px(40.))
                    .w_full()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .child("Text styles"),
                    )
                    .child(
                        Button::new(SharedString::from(format!("{}-close", self.id)))
                            .icon(IconName::Close)
                            .tooltip("Close")
                            .xsmall()
                            .compact()
                            .ghost()
                            .on_activate(|_, window, cx| {
                                window.dispatch_action(Box::new(CancelDesignInteraction), cx);
                            }),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Input::new(&self.search_input)
                            .small()
                            .prefix(Icon::new(IconName::Search).small()),
                    )
                    .children(self.render_current_binding(cx))
                    .when(self.disabled, |header| {
                        header.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    "View only · Search is available, style changes are disabled",
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-styles-scroll", self.id)))
                            .max_h(popup_height(window, PICKER_MAX_HEIGHT) - px(112.))
                            .overflow_y_scroll()
                            .track_scroll(&scroll_handle)
                            .child(self.render_style_groups(cx)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .h_full()
                            .child(Scrollbar::new(&scroll_handle).axis(ScrollbarAxis::Vertical)),
                    ),
            )
    }
}

fn style_matches_query(
    style: &DesignTypographyStyle,
    library_name: Option<&SharedString>,
    normalized_query: &str,
) -> bool {
    normalized_query.is_empty()
        || [
            style.name.as_ref(),
            style.family.as_ref(),
            style.font_style.as_ref(),
            library_name.map_or("", SharedString::as_ref),
        ]
        .iter()
        .any(|candidate| candidate.to_ascii_lowercase().contains(normalized_query))
}

fn format_style_size(size: f32) -> String {
    super::format::format_compact_number(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_style_metadata_and_library_names() {
        let style = DesignTypographyStyle::new(
            "display",
            "Display / Large",
            "Source Serif",
            "Semi Bold",
            48.,
        );
        assert!(style_matches_query(&style, None, "display"));
        assert!(style_matches_query(&style, None, "source serif"));
        assert!(style_matches_query(&style, None, "semi"));
        assert!(style_matches_query(
            &style,
            Some(&SharedString::from("Editorial")),
            "editorial"
        ));
        assert!(!style_matches_query(&style, None, "caption"));
    }

    #[test]
    fn style_size_formatting_is_compact_and_stable() {
        assert_eq!(format_style_size(16.), "16");
        assert_eq!(format_style_size(12.5), "12.5");
        assert_eq!(format_style_size(12.25), "12.25");
        assert_eq!(format_style_size(12.75), "12.75");
    }
}

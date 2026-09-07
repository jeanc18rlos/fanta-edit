use super::*;

impl PagesPanel {
    fn collapsed_title(&self) -> SharedString {
        self.selected_page
            .as_ref()
            .and_then(|selected| self.pages.iter().find(|page| &page.id == selected))
            .map(|page| page.title.clone())
            .unwrap_or_else(|| "Pages".into())
    }

    pub(super) fn render_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let expanded = self.expanded;
        let title = if expanded {
            SharedString::from("Pages")
        } else {
            self.collapsed_title()
        };

        h_flex()
            .id(SharedString::from(format!("{}-header", self.id)))
            .debug_selector(|| "pages-header".to_owned())
            .key_context(CONTROL_KEY_CONTEXT)
            .tab_index(0)
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().transparent)
            .cursor_pointer()
            .occlude()
            .when(self.header_hovered, |header| {
                header.bg(cx.theme().sidebar_accent.opacity(0.55))
            })
            .focus(|style| {
                style
                    .bg(cx.theme().sidebar_accent.opacity(0.55))
                    .border_color(cx.theme().selection)
            })
            .when(expanded, |header| {
                header.border_b_1().border_color(cx.theme().border)
            })
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if this.header_hovered != *hovered {
                    this.header_hovered = *hovered;
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.dismiss_page_menu(cx);
                }),
            )
            .on_activate(cx.listener(|this, _, _, cx| this.toggle_expanded(cx)))
            .child(
                h_flex()
                    .id(SharedString::from(format!("{}-toggle", self.id)))
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .gap_1()
                    .px_2()
                    .child(
                        Icon::new(if expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall(),
                    )
                    .child(
                        truncating_label(title).debug_selector(|| "pages-header-title".to_owned()),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{}-find-control", self.id)))
                    .relative()
                    .flex_none()
                    .debug_selector(|| "pages-search-trigger".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&self.find_focus_handle.clone().tab_index(0).tab_stop(true))
                    .focus(|style| style.bg(cx.theme().sidebar_accent).rounded(px(5.)))
                    .occlude()
                    .on_activate(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_search(window, cx);
                    }))
                    .child(
                        Button::new(SharedString::from(format!("{}-search", self.id)))
                            .ghost()
                            .xsmall()
                            .compact()
                            .tab_stop(false)
                            .icon(IconName::Search)
                            .tooltip_with_action(
                                "Find",
                                &FindInPages,
                                Some(PAGES_PANEL_KEY_CONTEXT),
                            ),
                    )
                    .child(self.control_bounds_tracker(FocusTooltipKind::Find, cx)),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{}-add-control", self.id)))
                    .relative()
                    .flex_none()
                    .debug_selector(|| "pages-add-trigger".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(
                        &self
                            .add_page_focus_handle
                            .clone()
                            .tab_index(0)
                            .tab_stop(true),
                    )
                    .focus(|style| style.bg(cx.theme().sidebar_accent).rounded(px(5.)))
                    .occlude()
                    .on_activate(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.begin_new_page(window, cx);
                    }))
                    .child(
                        Button::new(SharedString::from(format!("{}-add", self.id)))
                            .ghost()
                            .xsmall()
                            .compact()
                            .tab_stop(false)
                            .icon(IconName::Plus)
                            .tooltip_with_action(
                                "Add new page",
                                &AddPage,
                                Some(PAGES_PANEL_KEY_CONTEXT),
                            ),
                    )
                    .child(self.control_bounds_tracker(FocusTooltipKind::AddPage, cx)),
            )
            .child(div().w(px(4.)))
            .into_any_element()
    }
}

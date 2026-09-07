use super::*;

impl PagesPanel {
    fn page_reveal_height(&self) -> f32 {
        let count = self.pages.len() + usize::from(self.editing_is_new());
        (PAGE_PADDING * 2.
            + count as f32 * PAGE_ROW_HEIGHT
            + count.saturating_sub(1) as f32 * PAGE_ROW_GAP)
            .min(MAX_PAGE_LIST_HEIGHT)
    }

    fn editing_is_new(&self) -> bool {
        matches!(self.editing, Some(PageEditorTarget::New))
    }

    pub(super) fn render_pages(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let selected_page = self.selected_page.clone();
        let editing = self.editing.clone();
        let rows = self
            .pages
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let is_selected = selected_page.as_ref() == Some(&page.id);
                let is_editing = matches!(
                    editing.as_ref(),
                    Some(PageEditorTarget::Existing { page_id, .. }) if page_id == &page.id
                );
                if is_editing {
                    self.render_page_editor(format!("{}-edit-{index}", self.id))
                } else {
                    self.render_page_row(index, page, is_selected, cx)
                }
            })
            .collect::<Vec<_>>();

        let scroll_viewport = v_flex()
            .id(SharedString::from(format!("{}-pages-scroll", self.id)))
            .debug_selector(|| "pages-scroll-viewport".to_owned())
            .relative()
            .size_full()
            .overflow_scroll()
            .track_scroll(&self.pages_scroll_handle)
            .p_2()
            .gap_1()
            .children(rows)
            .when(self.editing_is_new(), |content| {
                content.child(self.render_page_editor(format!("{}-new-page", self.id)))
            });

        let expanded = self.expanded;
        let height = self.page_reveal_height();
        let reveal = div()
            .relative()
            .w_full()
            .h(px(if expanded { height } else { 0. }))
            .min_h(px(0.))
            .overflow_hidden()
            .child(scroll_viewport)
            .child(
                div()
                    .debug_selector(|| "pages-scrollbar-layer".to_owned())
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(
                        Scrollbar::new(&self.pages_scroll_handle)
                            .id(SharedString::from(format!("{}-pages-scrollbar", self.id)))
                            .axis(ScrollbarAxis::Both),
                    ),
            );
        #[cfg(test)]
        {
            reveal.into_any_element()
        }
        #[cfg(not(test))]
        {
            reveal
                .with_animation(
                    ElementId::NamedInteger(
                        SharedString::from(format!("{}-reveal", self.id)),
                        expanded as u64,
                    ),
                    Animation::new(Duration::from_secs_f64(REVEAL_DURATION))
                        .with_easing(cubic_bezier(0.4, 0., 0.2, 1.)),
                    move |element, delta| {
                        let progress = if expanded { delta } else { 1. - delta };
                        element.h(px(height * progress)).opacity(progress)
                    },
                )
                .into_any_element()
        }
    }

    fn render_page_editor(&self, id: impl Into<SharedString>) -> AnyElement {
        h_flex()
            .id(id.into())
            .debug_selector(|| "pages-page-editor".to_owned())
            .h(px(PAGE_ROW_HEIGHT))
            .w_full()
            .flex_none()
            .child(
                div()
                    .debug_selector(|| "pages-page-editor-input-slot".to_owned())
                    .key_context(PAGES_TEXT_ENTRY_KEY_CONTEXT)
                    .h_full()
                    .flex_1()
                    .min_w(px(0.))
                    .child(Input::new(&self.rename_input).xsmall()),
            )
            .into_any_element()
    }

    fn render_page_row(
        &mut self,
        index: usize,
        page: PagesPanelItem,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page_id = page.id.clone();
        let page_title = page.title.clone();
        let menu_page_id = page.id.clone();
        let menu_page_title = page.title.clone();
        let row_selector = format!("pages-row-{}", page.id);
        let row_focus_handle = self
            .page_focus_handles
            .entry(page.id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let is_hovered = self.hovered_page.as_ref() == Some(&page.id);
        let row = list_row(
            SharedString::from(format!("{}-page-{index}", self.id)),
            px(PAGE_ROW_HEIGHT),
            cx,
        )
        .debug_selector(move || row_selector)
        .track_focus(&row_focus_handle.tab_index(0).tab_stop(true))
        .px_2()
        .text_sm()
        .focus(|style| {
            style
                .bg(cx.theme().sidebar_accent.opacity(0.65))
                .border_color(cx.theme().selection)
        })
        .when(is_hovered && !is_selected, |row| {
            row.bg(cx.theme().sidebar_accent.opacity(0.65))
        })
        .when(is_selected, |row| {
            row.bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
                .font_semibold()
        })
        .on_hover(cx.listener({
            let hovered_page_id = page.id.clone();
            move |this, hovered, _, cx| {
                let changed = if *hovered {
                    if this.hovered_page.as_ref() == Some(&hovered_page_id) {
                        false
                    } else {
                        this.hovered_page = Some(hovered_page_id.clone());
                        true
                    }
                } else if this.hovered_page.as_ref() == Some(&hovered_page_id) {
                    this.hovered_page = None;
                    true
                } else {
                    false
                };
                if changed {
                    cx.notify();
                }
            }
        }));
        row.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.last_keyboard_page_activation = None;
                this.dismiss_page_menu(cx);
                this.cancel_page_edit(cx);
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener({
                let menu_page_id = menu_page_id.clone();
                let menu_page_title = menu_page_title.clone();
                move |this, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_page_menu(
                        menu_page_id.clone(),
                        menu_page_title.clone(),
                        event.position,
                        window,
                        cx,
                    );
                }
            }),
        )
        .on_action(cx.listener({
            let menu_page_id = menu_page_id.clone();
            let menu_page_title = menu_page_title.clone();
            move |this, _: &OpenPageContextMenu, window, cx| {
                cx.stop_propagation();
                this.open_page_menu_from_keyboard(
                    menu_page_id.clone(),
                    menu_page_title.clone(),
                    window,
                    cx,
                );
            }
        }))
        // The keyboard and pointer paths stay separate instead of sharing one
        // `on_activate` handler: pointer activation needs `ClickEvent`'s click
        // count for double-click rename, while the keyboard path owns the
        // equivalent double-Enter detection.
        .on_action(cx.listener({
            let keyboard_page_id = page.id.clone();
            let keyboard_page_title = page.title.clone();
            move |this, _: &ActivateControl, window, cx| {
                this.activate_page_from_keyboard(
                    keyboard_page_id.clone(),
                    keyboard_page_title.clone(),
                    window,
                    cx,
                );
            }
        }))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            if event.is_keyboard() {
                return;
            }
            this.last_keyboard_page_activation = None;
            this.page_menu = None;
            if event.click_count() >= 2 {
                this.begin_rename(page_id.clone(), page_title.clone(), window, cx);
            } else {
                cx.emit(PagesPanelAction::SelectRequested {
                    page_id: page_id.clone(),
                });
            }
        }))
        .child(truncating_label(page.title))
        .child(track_bounds(cx.entity(), {
            let row_bounds_page_id = page.id.clone();
            move |this, bounds| {
                this.page_row_bounds
                    .insert(row_bounds_page_id.clone(), bounds);
            }
        }))
        .into_any_element()
    }
}

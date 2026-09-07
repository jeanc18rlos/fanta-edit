use super::*;

impl PagesPanel {
    pub(super) fn toggle_expanded(&mut self, cx: &mut Context<Self>) {
        self.page_menu = None;
        self.expanded = !self.expanded;
        cx.emit(PagesPanelAction::ExpansionChanged {
            expanded: self.expanded,
        });
        cx.notify();
    }

    pub(super) fn toggle_search_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == PanelMode::Pages {
            return;
        }
        self.filter_menu_open = !self.filter_menu_open;
        self.scope_menu_open = false;
        if self.filter_menu_open {
            self.filter_menu_scroll_handle.set_offset(Point::default());
            let focus_handle = self.filter_menu_focus_handle.clone();
            window.defer(cx, move |window, cx| {
                focus_handle.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn open_search_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == PanelMode::Pages {
            return;
        }
        self.filter_menu_open = true;
        self.scope_menu_open = false;
        self.filter_menu_scroll_handle.set_offset(Point::default());
        let focus_handle = self.filter_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn toggle_search_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == PanelMode::Pages {
            return;
        }
        self.scope_menu_open = !self.scope_menu_open;
        self.filter_menu_open = false;
        if self.scope_menu_open {
            let focus_handle = self.scope_menu_focus_handle.clone();
            window.defer(cx, move |window, cx| {
                focus_handle.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn open_search_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == PanelMode::Pages {
            return;
        }
        self.scope_menu_open = true;
        self.filter_menu_open = false;
        let focus_handle = self.scope_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn on_toggle_panel(
        &mut self,
        _: &TogglePagesPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.mode == PanelMode::Pages {
            self.toggle_expanded(cx);
        }
    }

    pub(super) fn on_find_in_pages(
        &mut self,
        _: &FindInPages,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_search(window, cx);
    }

    pub(super) fn on_add_page(&mut self, _: &AddPage, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_new_page(window, cx);
    }

    pub(super) fn on_toggle_search_settings(
        &mut self,
        _: &ToggleSearchSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_search_settings(window, cx);
    }

    pub(super) fn on_close_search(
        &mut self,
        _: &ClosePagesSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cancel_active_draft(window, cx) || self.dismiss_open_popover(window, cx) {
            cx.stop_propagation();
            return;
        }
        if self.mode != PanelMode::Pages {
            cx.stop_propagation();
            self.close_search(window, cx);
        }
    }

    fn cancel_active_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.editing.take().is_none() {
            return false;
        }
        self.focus_panel_after_action(window, cx);
        cx.notify();
        true
    }

    fn dismiss_open_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(menu) = self.page_menu.take() {
            menu.return_focus
                .unwrap_or_else(|| self.focus_handle.clone())
                .focus(window, cx);
        } else if self.filter_menu_open {
            self.filter_menu_open = false;
            self.settings_focus_handle.focus(window, cx);
        } else if self.scope_menu_open {
            self.scope_menu_open = false;
            self.scope_trigger_focus_handle.focus(window, cx);
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub(super) fn on_previous_search_result(
        &mut self,
        _: &PreviousSearchResult,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_results(PagesPanelResultDirection::Previous, cx);
    }

    pub(super) fn on_next_search_result(
        &mut self,
        _: &NextSearchResult,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_results(PagesPanelResultDirection::Next, cx);
    }

    pub(super) fn on_replace_current_result(
        &mut self,
        _: &ReplaceCurrentResult,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_replace(false, cx);
    }

    pub(super) fn on_replace_all_results(
        &mut self,
        _: &ReplaceAllResults,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_replace(true, cx);
    }

    pub(super) fn on_focus_previous_page(
        &mut self,
        _: &FocusPreviousPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.focused_page_index(window) {
            self.focus_page_row(index.saturating_sub(1), window, cx);
        }
    }

    pub(super) fn on_focus_next_page(
        &mut self,
        _: &FocusNextPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self.focused_page_index(window) {
            self.focus_page_row(
                (index + 1).min(self.pages.len().saturating_sub(1)),
                window,
                cx,
            );
        }
    }

    pub(super) fn on_focus_first_page(
        &mut self,
        _: &FocusFirstPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focused_page_index(window).is_some() {
            self.focus_page_row(0, window, cx);
        }
    }

    pub(super) fn on_focus_last_page(
        &mut self,
        _: &FocusLastPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focused_page_index(window).is_some() {
            self.focus_page_row(self.pages.len().saturating_sub(1), window, cx);
        }
    }

    fn focused_page_index(&self, window: &Window) -> Option<usize> {
        if self.mode != PanelMode::Pages {
            return None;
        }
        self.pages.iter().position(|page| {
            self.page_focus_handles
                .get(&page.id)
                .is_some_and(|handle| handle.is_focused(window))
        })
    }

    fn focus_page_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.pages.get(index) else {
            return;
        };
        let Some(handle) = self.page_focus_handles.get(&page.id) else {
            return;
        };
        handle.focus(window, cx);
        self.pages_scroll_handle.scroll_to_item(index);
        cx.notify();
    }

    pub(super) fn begin_new_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.expanded {
            self.expanded = true;
            cx.emit(PagesPanelAction::ExpansionChanged { expanded: true });
        }
        self.mode = PanelMode::Pages;
        self.page_menu = None;

        let title = next_page_title(&self.pages);
        self.editing = Some(PageEditorTarget::New);
        let offset = self.pages_scroll_handle.offset();
        self.pages_scroll_handle.set_offset(point(px(0.), offset.y));
        self.pages_scroll_handle.scroll_to_bottom();
        self.focus_page_editor(title, window, cx);
    }

    pub(super) fn begin_rename(
        &mut self,
        page_id: SharedString,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.page_menu = None;
        self.editing = Some(PageEditorTarget::Existing {
            page_id,
            original_title: title.clone(),
        });
        let offset = self.pages_scroll_handle.offset();
        self.pages_scroll_handle.set_offset(point(px(0.), offset.y));
        self.focus_page_editor(title, window, cx);
    }

    fn focus_page_editor(
        &mut self,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rename_input.update(cx, |input, cx| {
            input.set_value(title, window, cx);
            input.focus(window, cx);
        });
        self.select_page_editor_after_render = true;
        cx.notify();
    }

    /// Commits the active draft. Matching the Layers rename contract, an
    /// intent is emitted only for non-empty text that actually changes the
    /// page: an unchanged rename and a cleared draft commit as no-ops.
    pub(super) fn commit_page_name(
        &mut self,
        restore_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.editing.take() else {
            return;
        };
        let entered = self.rename_input.read(cx).value();
        let entered = entered.trim();

        match target {
            PageEditorTarget::Existing {
                page_id,
                original_title,
            } => {
                if !entered.is_empty() && entered != original_title.as_ref() {
                    cx.emit(PagesPanelAction::RenameRequested {
                        page_id,
                        title: entered.to_owned().into(),
                    });
                }
            }
            PageEditorTarget::New => {
                if !entered.is_empty() {
                    cx.emit(PagesPanelAction::CreateRequested {
                        title: entered.to_owned().into(),
                    });
                }
            }
        }
        if restore_focus {
            let focus_handle = self.focus_handle.clone();
            window.defer(cx, move |window, cx| {
                focus_handle.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn cancel_page_edit(&mut self, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn activate_page_from_keyboard(
        &mut self,
        page_id: SharedString,
        page_title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = Instant::now();
        let is_double_enter = self.last_keyboard_page_activation.as_ref().is_some_and(
            |(last_page_id, activated_at)| {
                *last_page_id == page_id
                    && now.saturating_duration_since(*activated_at) <= DOUBLE_ENTER_INTERVAL
            },
        );

        if is_double_enter {
            self.last_keyboard_page_activation = None;
            self.begin_rename(page_id, page_title, window, cx);
        } else {
            self.last_keyboard_page_activation = Some((page_id.clone(), now));
            self.cancel_page_edit(cx);
            cx.emit(PagesPanelAction::SelectRequested { page_id });
        }
    }

    pub(super) fn open_page_menu(
        &mut self,
        page_id: SharedString,
        page_title: SharedString,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_page_edit(cx);
        let return_focus = window.focused(cx);
        self.page_menu = Some(PageMenuState {
            page_id,
            page_title,
            anchor,
            return_focus,
        });
        let focus_handle = self.page_menu_focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn open_page_menu_from_keyboard(
        &mut self,
        page_id: SharedString,
        page_title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.page_row_bounds.get(&page_id).copied() else {
            return;
        };
        self.open_page_menu(
            page_id,
            page_title,
            bounds.bottom_left() + point(px(0.), px(4.)),
            window,
            cx,
        );
    }

    pub(super) fn dismiss_page_menu(&mut self, cx: &mut Context<Self>) {
        if self.page_menu.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn activate_page_menu_item(
        &mut self,
        action: PageMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.page_menu.take() else {
            return;
        };
        match action {
            PageMenuAction::CopyLink => {
                cx.emit(PagesPanelAction::CopyLinkRequested {
                    page_id: menu.page_id,
                });
                self.focus_panel_after_action(window, cx);
            }
            PageMenuAction::Rename => {
                self.begin_rename(menu.page_id, menu.page_title, window, cx);
            }
            PageMenuAction::Duplicate => {
                cx.emit(PagesPanelAction::DuplicateRequested {
                    page_id: menu.page_id,
                });
                self.focus_panel_after_action(window, cx);
            }
            PageMenuAction::Delete => {
                cx.emit(PagesPanelAction::DeleteRequested {
                    page_id: menu.page_id,
                });
                self.focus_panel_after_action(window, cx);
            }
        }
        cx.notify();
    }

    fn focus_panel_after_action(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus_handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
    }

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_page_name(false, window, cx);
        self.mode = PanelMode::Find;
        self.filter_menu_open = false;
        self.scope_menu_open = false;
        self.page_menu = None;
        self.search_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = PanelMode::Pages;
        self.filter_menu_open = false;
        self.scope_menu_open = false;
        cx.emit(PagesPanelAction::SearchClosed);
        let focus_handle = self.focus_handle.clone();
        window.defer(cx, move |window, cx| {
            focus_handle.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn build_search_request(&self, cx: &App) -> PagesPanelSearchRequest {
        PagesPanelSearchRequest {
            query: self.search_input.read(cx).value(),
            scope: self.search_scope,
            element_kinds: self.active_filters.clone(),
            match_case: self.match_case,
            whole_words: self.whole_words,
        }
    }

    pub(super) fn emit_search_request(&mut self, cx: &mut Context<Self>) {
        self.active_result = None;
        cx.emit(PagesPanelAction::SearchRequested(
            self.build_search_request(cx),
        ));
        cx.notify();
    }

    pub(super) fn toggle_filter(&mut self, kind: PagesPanelElementKind, cx: &mut Context<Self>) {
        if kind == PagesPanelElementKind::All {
            self.active_filters.clear();
        } else if let Some(index) = self.active_filters.iter().position(|item| *item == kind) {
            self.active_filters.remove(index);
        } else {
            self.active_filters.push(kind);
            self.active_filters.sort_unstable();
        }
        self.emit_search_request(cx);
    }

    pub(super) fn toggle_match_case(&mut self, cx: &mut Context<Self>) {
        self.match_case = !self.match_case;
        self.emit_search_request(cx);
    }

    pub(super) fn toggle_whole_words(&mut self, cx: &mut Context<Self>) {
        self.whole_words = !self.whole_words;
        self.emit_search_request(cx);
    }

    pub(super) fn set_panel_mode(
        &mut self,
        mode: PanelMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = mode;
        self.filter_menu_open = false;
        if mode == PanelMode::Replace {
            self.replace_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        } else {
            self.search_input.update(cx, |input, cx| {
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(super) fn set_scope(
        &mut self,
        scope: PagesPanelSearchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_scope = scope;
        self.scope_menu_open = false;
        self.emit_search_request(cx);
        self.search_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }

    pub(super) fn select_result(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(result) = self.results.items.get(index) else {
            return;
        };
        self.active_result = Some(index);
        cx.emit(PagesPanelAction::SearchResultSelected {
            result_id: result.id.clone(),
        });
        cx.notify();
    }

    pub(super) fn navigate_results(
        &mut self,
        direction: PagesPanelResultDirection,
        cx: &mut Context<Self>,
    ) {
        let len = self.results.items.len();
        if len == 0 {
            return;
        }
        let next = Some(match (self.active_result, direction) {
            (None, _) => 0,
            (Some(0), PagesPanelResultDirection::Previous) => len - 1,
            (Some(index), PagesPanelResultDirection::Previous) => index - 1,
            (Some(index), PagesPanelResultDirection::Next) => (index + 1) % len,
        });
        self.active_result = next;
        let result_id = next.map(|index| self.results.items[index].id.clone());
        cx.emit(PagesPanelAction::NavigateResults {
            direction,
            result_id,
        });
        cx.notify();
    }

    pub(super) fn request_replace(&mut self, all: bool, cx: &mut Context<Self>) {
        if (all && self.results.total == 0) || (!all && self.results.items.is_empty()) {
            return;
        }
        let request = self.build_search_request(cx);
        let replacement = self.replace_input.read(cx).value();
        if all {
            cx.emit(PagesPanelAction::ReplaceAllRequested {
                request,
                replacement,
            });
        } else {
            let result_id = self
                .active_result
                .and_then(|index| self.results.items.get(index))
                .map(|result| result.id.clone());
            cx.emit(PagesPanelAction::ReplaceRequested {
                request,
                result_id,
                replacement,
            });
        }
    }
}

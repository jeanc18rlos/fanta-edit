use super::*;

impl PagesPanel {
    pub(super) fn control_bounds_tracker(
        &self,
        kind: FocusTooltipKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        track_bounds(cx.entity(), move |this, bounds| {
            this.tooltip_anchor_bounds.insert(kind, bounds);
        })
        .into_any_element()
    }

    fn render_page_menu_item(
        &mut self,
        label: &'static str,
        selector: &'static str,
        action: PageMenuAction,
        first: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        menu_item(
            SharedString::from(format!("{}-{selector}", self.id)),
            px(36.),
            cx,
        )
        .debug_selector(move || selector.to_owned())
        .when(first, |item| {
            item.track_focus(
                &self
                    .page_menu_focus_handle
                    .clone()
                    .tab_index(0)
                    .tab_stop(true),
            )
        })
        .on_activate(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            this.activate_page_menu_item(action, window, cx);
        }))
        .child(label)
        .into_any_element()
    }

    pub(super) fn render_page_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(panel_bounds), Some(menu)) = (self.panel_bounds, self.page_menu.as_ref()) else {
            return div().into_any_element();
        };
        // Menus clamp against the window like other Fanta popups (§12), not
        // against the anchoring panel: the deferred surface may extend past
        // the panel but must stay fully on screen.
        let window_size = window.viewport_size();
        let clamped = clamp_menu_origin(
            menu.anchor,
            window_size,
            size(
                px(PAGE_MENU_WIDTH),
                px(PAGE_MENU_HEIGHT).min(window_size.height),
            ),
        );
        let max_height = window_size.height - clamped.y;
        let origin = clamped - panel_bounds.origin;

        deferred(
            menu_surface(
                SharedString::from(format!("{}-page-menu", self.id)),
                origin,
                px(PAGE_MENU_WIDTH),
                max_height,
                px(MENU_RADIUS),
                cx,
            )
            .debug_selector(|| "pages-page-menu".to_owned())
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.dismiss_page_menu(cx);
            }))
            .child(self.render_page_menu_item(
                "Copy link to page",
                "pages-page-menu-copy-link",
                PageMenuAction::CopyLink,
                true,
                cx,
            ))
            .child(menu_separator(cx))
            .child(self.render_page_menu_item(
                "Rename page",
                "pages-page-menu-rename",
                PageMenuAction::Rename,
                false,
                cx,
            ))
            .child(self.render_page_menu_item(
                "Duplicate page",
                "pages-page-menu-duplicate",
                PageMenuAction::Duplicate,
                false,
                cx,
            ))
            .child(menu_separator(cx))
            .child(self.render_page_menu_item(
                "Delete page",
                "pages-page-menu-delete",
                PageMenuAction::Delete,
                false,
                cx,
            )),
        )
        .with_priority(3)
        .into_any_element()
    }

    pub(super) fn render_focused_tooltip(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let kind = [
            (FocusTooltipKind::Find, &self.find_focus_handle),
            (FocusTooltipKind::AddPage, &self.add_page_focus_handle),
            (FocusTooltipKind::Settings, &self.settings_focus_handle),
            (
                FocusTooltipKind::CloseSearch,
                &self.close_search_focus_handle,
            ),
            (
                FocusTooltipKind::ReplaceCurrent,
                &self.replace_current_focus_handle,
            ),
            (FocusTooltipKind::ReplaceAll, &self.replace_all_focus_handle),
            (
                FocusTooltipKind::PreviousResult,
                &self.previous_result_focus_handle,
            ),
            (FocusTooltipKind::NextResult, &self.next_result_focus_handle),
        ]
        .into_iter()
        .find_map(|(kind, handle)| handle.is_focused(window).then_some(kind))?;
        let panel_bounds = self.panel_bounds?;
        let anchor_bounds = *self.tooltip_anchor_bounds.get(&kind)?;
        let tooltip = match kind {
            FocusTooltipKind::Find => {
                Tooltip::new("Find").action(&FindInPages, Some(PAGES_PANEL_KEY_CONTEXT))
            }
            FocusTooltipKind::AddPage => {
                Tooltip::new("Add new page").action(&AddPage, Some(PAGES_PANEL_KEY_CONTEXT))
            }
            FocusTooltipKind::Settings => Tooltip::new("Settings")
                .action(&ToggleSearchSettings, Some(PAGES_PANEL_KEY_CONTEXT)),
            FocusTooltipKind::CloseSearch => Tooltip::new("Close search")
                .action(&ClosePagesSearch, Some(PAGES_PANEL_KEY_CONTEXT)),
            FocusTooltipKind::ReplaceCurrent => Tooltip::new("Replace current result")
                .action(&ReplaceCurrentResult, Some(PAGES_PANEL_KEY_CONTEXT)),
            FocusTooltipKind::ReplaceAll => Tooltip::new("Replace all results")
                .action(&ReplaceAllResults, Some(PAGES_PANEL_KEY_CONTEXT)),
            FocusTooltipKind::PreviousResult => Tooltip::new("Previous result")
                .action(&PreviousSearchResult, Some(PAGES_PANEL_KEY_CONTEXT)),
            FocusTooltipKind::NextResult => {
                Tooltip::new("Next result").action(&NextSearchResult, Some(PAGES_PANEL_KEY_CONTEXT))
            }
        }
        .build(window, cx);
        let right = panel_bounds.right() - anchor_bounds.right();
        let top = anchor_bounds.bottom() + px(2.) - panel_bounds.top();

        Some(
            deferred(
                div()
                    .absolute()
                    .right(right)
                    .top(top)
                    .debug_selector(|| "pages-focus-tooltip".to_owned())
                    .occlude()
                    .child(tooltip),
            )
            .with_priority(4)
            .into_any_element(),
        )
    }

    pub(super) fn render_filter_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(panel_bounds), Some(anchor_bounds)) =
            (self.search_panel_bounds, self.filter_anchor_bounds)
        else {
            return div().into_any_element();
        };
        let counts = self.results.element_counts.clone();
        let all_active = self.active_filters.is_empty();
        let mode = self.mode;
        let estimated_height =
            px((PagesPanelElementKind::FILTER_ORDER.len() as f32 + 4.) * 32. + 50.);
        let window_size = window.viewport_size();
        let clamped = clamp_menu_origin(
            anchor_bounds.bottom_left() + point(px(0.), px(4.)),
            window_size,
            size(
                px(FILTER_MENU_WIDTH),
                estimated_height.min(window_size.height),
            ),
        );
        let max_height = window_size.height - clamped.y;
        let origin = clamped - panel_bounds.origin;

        deferred(
            menu_surface(
                SharedString::from(format!("{}-filter-menu", self.id)),
                origin,
                px(FILTER_MENU_WIDTH),
                max_height,
                px(MENU_RADIUS),
                cx,
            )
            .debug_selector(|| "pages-filter-menu".to_owned())
            .capture_key_up(prevent_keyboard_activation_click)
            .track_scroll(&self.filter_menu_scroll_handle)
            .child(self.render_mode_item(PanelMode::Find, "Find", mode, cx))
            .child(self.render_mode_item(PanelMode::Replace, "Replace", mode, cx))
            .child(menu_separator(cx))
            .children(PagesPanelElementKind::FILTER_ORDER.into_iter().map(|kind| {
                let active = if kind == PagesPanelElementKind::All {
                    all_active
                } else {
                    self.active_filters.contains(&kind)
                };
                let count = counts
                    .iter()
                    .find(|item| item.kind == kind)
                    .map(|item| item.count);
                menu_item(
                    SharedString::from(format!("{}-filter-{}", self.id, kind.label())),
                    px(32.),
                    cx,
                )
                .on_activate(cx.listener(move |this, _, _, cx| {
                    this.toggle_filter(kind, cx);
                }))
                .child(div().w(px(14.)).when(active, |slot| {
                    slot.child(Icon::new(IconName::Check).xsmall())
                }))
                .child(Self::element_icon(kind, cx))
                .child(div().flex_1().text_sm().child(kind.label()))
                .when_some(count, |row, count| {
                    row.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(count.to_string()),
                    )
                })
            }))
            .child(menu_separator(cx))
            .child(self.render_option_item("Match case", self.match_case, true, cx))
            .child(self.render_option_item("Whole words", self.whole_words, false, cx))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .debug_selector(|| "pages-filter-menu-scrollbar".to_owned())
                    .child(Scrollbar::vertical(&self.filter_menu_scroll_handle).id(
                        SharedString::from(format!("{}-filter-menu-scrollbar", self.id)),
                    )),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }

    fn render_mode_item(
        &mut self,
        item_mode: PanelMode,
        label: &'static str,
        active_mode: PanelMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = format!("pages-mode-{}", label.to_lowercase());
        let menu_focus_handle = self.filter_menu_focus_handle.clone();
        menu_item(
            SharedString::from(format!("{}-mode-{label}", self.id)),
            px(32.),
            cx,
        )
        .debug_selector(move || selector)
        .when(item_mode == PanelMode::Find, |row| {
            row.track_focus(&menu_focus_handle)
        })
        .when(item_mode == active_mode, |row| row.bg(cx.theme().accent))
        .on_activate(cx.listener(move |this, _, window, cx| {
            this.set_panel_mode(item_mode, window, cx);
        }))
        .child(div().w(px(14.)).when(item_mode == active_mode, |slot| {
            slot.child(Icon::new(IconName::Check).xsmall())
        }))
        .child(label)
        .into_any_element()
    }

    fn render_option_item(
        &mut self,
        label: &'static str,
        active: bool,
        match_case: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        menu_item(
            SharedString::from(format!("{}-option-{label}", self.id)),
            px(32.),
            cx,
        )
        .on_activate(cx.listener(move |this, _, _, cx| {
            if match_case {
                this.toggle_match_case(cx);
            } else {
                this.toggle_whole_words(cx);
            }
        }))
        .child(div().w(px(14.)).when(active, |slot| {
            slot.child(Icon::new(IconName::Check).xsmall())
        }))
        .child(label)
        .into_any_element()
    }

    pub(super) fn render_scope_menu(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(panel_bounds), Some(anchor_bounds)) =
            (self.search_panel_bounds, self.scope_anchor_bounds)
        else {
            return div().into_any_element();
        };
        let window_size = window.viewport_size();
        let clamped = clamp_menu_origin(
            anchor_bounds.bottom_left() + point(px(0.), px(4.)),
            window_size,
            size(
                px(SCOPE_MENU_WIDTH),
                px(SCOPE_MENU_HEIGHT).min(window_size.height),
            ),
        );
        let max_height = window_size.height - clamped.y;
        let origin = clamped - panel_bounds.origin;
        let menu_focus_handle = self.scope_menu_focus_handle.clone();

        deferred(
            menu_surface(
                SharedString::from(format!("{}-scope-menu", self.id)),
                origin,
                px(SCOPE_MENU_WIDTH),
                max_height,
                px(MENU_RADIUS),
                cx,
            )
            .debug_selector(|| "pages-scope-menu".to_owned())
            .capture_key_up(prevent_keyboard_activation_click)
            .children(
                [
                    PagesPanelSearchScope::CurrentPage,
                    PagesPanelSearchScope::AllPages,
                ]
                .into_iter()
                .map(|scope| {
                    let menu_focus_handle = menu_focus_handle.clone();
                    let selector = match scope {
                        PagesPanelSearchScope::CurrentPage => "pages-scope-current-page",
                        PagesPanelSearchScope::AllPages => "pages-scope-all-pages",
                    };
                    menu_item(
                        SharedString::from(format!("{}-scope-{}", self.id, scope.label())),
                        px(36.),
                        cx,
                    )
                    .debug_selector(move || selector.to_owned())
                    .when(scope == PagesPanelSearchScope::CurrentPage, |row| {
                        row.track_focus(&menu_focus_handle)
                    })
                    .on_activate(cx.listener(move |this, _, window, cx| {
                        this.set_scope(scope, window, cx);
                    }))
                    .child(div().w(px(14.)).when(scope == self.search_scope, |slot| {
                        slot.child(Icon::new(IconName::Check).xsmall())
                    }))
                    .child(scope.label())
                }),
            ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

fn menu_separator(cx: &App) -> gpui::Div {
    div()
        .h(px(1.))
        .w_full()
        .my_2()
        .flex_none()
        .bg(cx.theme().border)
}

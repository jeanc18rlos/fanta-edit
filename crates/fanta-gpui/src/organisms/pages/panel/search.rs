use super::*;

impl PagesPanel {
    /// Node-kind icon shared with the Layers iconography: code-native vector
    /// icons for kinds a node draws, the toolbar's Rectangle drawing for
    /// generic shapes, and canonical icon assets for the UI-only categories.
    pub(super) fn element_icon(kind: PagesPanelElementKind, cx: &App) -> AnyElement {
        let color = match kind {
            PagesPanelElementKind::Component | PagesPanelElementKind::Instance => {
                cx.theme().selection
            }
            _ => cx.theme().muted_foreground,
        };
        let icon = match kind {
            PagesPanelElementKind::All => {
                return themed_icon_asset(IconName::GalleryVerticalEnd, color);
            }
            PagesPanelElementKind::Other => {
                return themed_icon_asset(IconName::Ellipsis, color);
            }
            PagesPanelElementKind::Text => ControlIcon::Text,
            PagesPanelElementKind::FrameGroup => ControlIcon::Frame,
            PagesPanelElementKind::Component => ControlIcon::Component,
            PagesPanelElementKind::Instance => ControlIcon::Instance,
            PagesPanelElementKind::Image => ControlIcon::Image,
            PagesPanelElementKind::Shape => {
                return render_tool_icon(ToolbarTool::Rectangle, color, ELEMENT_ICON_SIZE);
            }
        };
        render_control_icon(icon, color, ELEMENT_ICON_SIZE)
    }

    fn render_search_toolbar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let replace = self.mode == PanelMode::Replace;
        let can_replace_one = !self.results.items.is_empty();
        let can_replace_all = self.results.total > 0;
        let settings_focus_handle = self
            .settings_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        let close_search_focus_handle = self
            .close_search_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        let replace_current_focus_handle = self
            .replace_current_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        let replace_all_focus_handle = self
            .replace_all_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            // The query input yields width to the fixed
                            // settings/close controls instead of forcing the
                            // toolbar wider than a narrow panel.
                            .min_w(px(0.))
                            .key_context(PAGES_TEXT_ENTRY_KEY_CONTEXT)
                            .child(
                                Input::new(&self.search_input)
                                    .small()
                                    .cleanable(true)
                                    .prefix(Icon::new(IconName::Search).small()),
                            ),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-filters-control", self.id)))
                            .relative()
                            .flex_none()
                            .debug_selector(|| "pages-filter-trigger".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .track_focus(&settings_focus_handle)
                            .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                            .on_activate(cx.listener(|this, event: &ActivateEvent, window, cx| {
                                cx.stop_propagation();
                                if event.keyboard {
                                    this.open_search_settings(window, cx);
                                } else {
                                    this.toggle_search_settings(window, cx);
                                }
                            }))
                            .child(
                                Button::new(SharedString::from(format!("{}-filters", self.id)))
                                    .ghost()
                                    .small()
                                    .compact()
                                    .tab_stop(false)
                                    .icon(IconName::Settings2)
                                    .tooltip_with_action(
                                        "Settings",
                                        &ToggleSearchSettings,
                                        Some(PAGES_PANEL_KEY_CONTEXT),
                                    ),
                            )
                            .child(self.control_bounds_tracker(FocusTooltipKind::Settings, cx))
                            .child(track_bounds(cx.entity(), |this, bounds| {
                                this.filter_anchor_bounds = Some(bounds);
                            })),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "{}-close-search-control",
                                self.id
                            )))
                            .relative()
                            .flex_none()
                            .debug_selector(|| "pages-close-search".to_owned())
                            .key_context(CONTROL_KEY_CONTEXT)
                            .track_focus(&close_search_focus_handle)
                            .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                            .on_activate(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_search(window, cx);
                            }))
                            .child(
                                Button::new(SharedString::from(format!(
                                    "{}-close-search",
                                    self.id
                                )))
                                .ghost()
                                .small()
                                .compact()
                                .tab_stop(false)
                                .icon(IconName::Close)
                                .tooltip_with_action(
                                    "Close search",
                                    &ClosePagesSearch,
                                    Some(PAGES_PANEL_KEY_CONTEXT),
                                ),
                            )
                            .child(self.control_bounds_tracker(FocusTooltipKind::CloseSearch, cx)),
                    ),
            )
            .when(replace, |toolbar| {
                toolbar
                    .child(
                        div()
                            .w_full()
                            .key_context(PAGES_TEXT_ENTRY_KEY_CONTEXT)
                            .child(
                                Input::new(&self.replace_input)
                                    .small()
                                    .cleanable(true)
                                    .prefix(Icon::new(IconName::ArrowRight).small()),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "{}-replace-one-control",
                                        self.id
                                    )))
                                    .relative()
                                    .flex_none()
                                    .debug_selector(|| "pages-replace-one".to_owned())
                                    .key_context(CONTROL_KEY_CONTEXT)
                                    .when(can_replace_one, |control| {
                                        control.track_focus(&replace_current_focus_handle)
                                    })
                                    .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                                    .on_activate(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if can_replace_one {
                                            this.request_replace(false, cx);
                                        }
                                    }))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-replace-one",
                                            self.id
                                        )))
                                        .label("Replace")
                                        .small()
                                        .tab_stop(false)
                                        .disabled(!can_replace_one)
                                        .tooltip_with_action(
                                            "Replace current result",
                                            &ReplaceCurrentResult,
                                            Some(PAGES_PANEL_KEY_CONTEXT),
                                        ),
                                    )
                                    .child(self.control_bounds_tracker(
                                        FocusTooltipKind::ReplaceCurrent,
                                        cx,
                                    )),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "{}-replace-all-control",
                                        self.id
                                    )))
                                    .relative()
                                    .flex_none()
                                    .debug_selector(|| "pages-replace-all".to_owned())
                                    .key_context(CONTROL_KEY_CONTEXT)
                                    .when(can_replace_all, |control| {
                                        control.track_focus(&replace_all_focus_handle)
                                    })
                                    .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                                    .on_activate(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        if can_replace_all {
                                            this.request_replace(true, cx);
                                        }
                                    }))
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "{}-replace-all",
                                            self.id
                                        )))
                                        .label("Replace all")
                                        .small()
                                        .tab_stop(false)
                                        .disabled(!can_replace_all)
                                        .tooltip_with_action(
                                            "Replace all results",
                                            &ReplaceAllResults,
                                            Some(PAGES_PANEL_KEY_CONTEXT),
                                        ),
                                    )
                                    .child(
                                        self.control_bounds_tracker(
                                            FocusTooltipKind::ReplaceAll,
                                            cx,
                                        ),
                                    ),
                            ),
                    )
            })
            .when(!self.active_filters.is_empty(), |toolbar| {
                toolbar.child(h_flex().w_full().gap_1().flex_wrap().children(
                    self.active_filters.clone().into_iter().map(|kind| {
                        h_flex()
                            .id(SharedString::from(format!(
                                "{}-filter-pill-{}",
                                self.id,
                                kind.label()
                            )))
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .h(px(28.))
                            .gap_1()
                            .px_2()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_pointer()
                            .hover(|style| style.bg(cx.theme().accent))
                            .focus(|style| {
                                style
                                    .bg(cx.theme().accent)
                                    .border_color(cx.theme().selection)
                            })
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.toggle_filter(kind, cx);
                            }))
                            .child(kind.label())
                            .child(Icon::new(IconName::Close).xsmall())
                    }),
                ))
            })
            .into_any_element()
    }

    fn render_results_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let result_label = result_count_label(self.results.total);
        let can_navigate = !self.results.items.is_empty();
        let previous_result_focus_handle = self
            .previous_result_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        let next_result_focus_handle = self
            .next_result_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        let scope_trigger_focus_handle = self
            .scope_trigger_focus_handle
            .clone()
            .tab_index(0)
            .tab_stop(true);
        h_flex()
            .debug_selector(|| "pages-results-header".to_owned())
            .h(px(48.))
            .w_full()
            .min_w(px(0.))
            .px_3()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .debug_selector(|| "pages-result-count".to_owned())
                    .flex_none()
                    .whitespace_nowrap()
                    .text_sm()
                    .child(result_label),
            )
            .child(div().flex_none().text_sm().child("·"))
            .child(
                h_flex()
                    .id(SharedString::from(format!("{}-scope", self.id)))
                    .relative()
                    // The scope trigger is the header's one compressible
                    // run: on narrow panels its label truncates so the
                    // count and navigation controls keep their hit areas.
                    .min_w(px(0.))
                    .debug_selector(|| "pages-scope-trigger".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .track_focus(&scope_trigger_focus_handle)
                    .gap_1()
                    .text_sm()
                    .cursor_pointer()
                    .focus(|style| {
                        style
                            .bg(cx.theme().accent)
                            .border_color(cx.theme().selection)
                    })
                    .on_activate(cx.listener(|this, event: &ActivateEvent, window, cx| {
                        cx.stop_propagation();
                        if event.keyboard {
                            this.open_search_scope(window, cx);
                        } else {
                            this.toggle_search_scope(window, cx);
                        }
                    }))
                    .child(truncating_label(self.search_scope.label()))
                    .child(
                        div()
                            .flex_none()
                            .child(Icon::new(IconName::ChevronDown).xsmall()),
                    )
                    .child(track_bounds(cx.entity(), |this, bounds| {
                        this.scope_anchor_bounds = Some(bounds);
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id(SharedString::from(format!("{}-previous-control", self.id)))
                    .relative()
                    .flex_none()
                    .debug_selector(|| "pages-previous-result".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .when(can_navigate, |control| {
                        control.track_focus(&previous_result_focus_handle)
                    })
                    .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                    .on_activate(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if can_navigate {
                            this.navigate_results(PagesPanelResultDirection::Previous, cx);
                        }
                    }))
                    .child(
                        Button::new(SharedString::from(format!("{}-previous", self.id)))
                            .ghost()
                            .xsmall()
                            .compact()
                            .tab_stop(false)
                            .icon(IconName::ChevronUp)
                            .disabled(!can_navigate)
                            .tooltip_with_action(
                                "Previous result",
                                &PreviousSearchResult,
                                Some(PAGES_PANEL_KEY_CONTEXT),
                            ),
                    )
                    .child(self.control_bounds_tracker(FocusTooltipKind::PreviousResult, cx)),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{}-next-control", self.id)))
                    .relative()
                    .flex_none()
                    .debug_selector(|| "pages-next-result".to_owned())
                    .key_context(CONTROL_KEY_CONTEXT)
                    .when(can_navigate, |control| {
                        control.track_focus(&next_result_focus_handle)
                    })
                    .focus(|style| style.bg(cx.theme().accent).rounded(px(5.)))
                    .on_activate(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if can_navigate {
                            this.navigate_results(PagesPanelResultDirection::Next, cx);
                        }
                    }))
                    .child(
                        Button::new(SharedString::from(format!("{}-next", self.id)))
                            .ghost()
                            .xsmall()
                            .compact()
                            .tab_stop(false)
                            .icon(IconName::ChevronDown)
                            .disabled(!can_navigate)
                            .tooltip_with_action(
                                "Next result",
                                &NextSearchResult,
                                Some(PAGES_PANEL_KEY_CONTEXT),
                            ),
                    )
                    .child(self.control_bounds_tracker(FocusTooltipKind::NextResult, cx)),
            )
            .into_any_element()
    }

    fn render_results(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.active_result;
        let hovered = self.hovered_result;
        let replacement = self.replace_input.read(cx).value();
        let replace_mode = self.mode == PanelMode::Replace;
        let empty = self.results.items.is_empty();

        v_flex()
            .debug_selector(|| "pages-results-list".to_owned())
            .w_full()
            .when(empty, |results| {
                results.child(
                    div()
                        .debug_selector(|| "pages-results-empty".to_owned())
                        .w_full()
                        .p_4()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(empty_results_label(self.search_scope)),
                )
            })
            .children(
                self.results
                    .items
                    .clone()
                    .into_iter()
                    .enumerate()
                    .map(|(index, result)| {
                        let is_active = active == Some(index);
                        let is_hovered = hovered == Some(index);
                        h_flex()
                            .id(SharedString::from(format!("{}-result-{index}", self.id)))
                            .debug_selector(move || format!("pages-result-{index}"))
                            .key_context(CONTROL_KEY_CONTEXT)
                            .tab_index(0)
                            .min_h(px(52.))
                            .flex_none()
                            .w_full()
                            .gap_2()
                            .px_4()
                            .py_2()
                            .cursor_pointer()
                            .when(is_hovered && !is_active, |row| row.bg(cx.theme().accent))
                            .focus(|style| {
                                style
                                    .bg(cx.theme().accent)
                                    .border_l_2()
                                    .border_color(cx.theme().selection)
                            })
                            .when(is_active, |row| {
                                row.bg(cx.theme().list_active)
                                    .border_l_2()
                                    .border_color(cx.theme().list_active_border)
                            })
                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                let changed = if *hovered {
                                    if this.hovered_result == Some(index) {
                                        false
                                    } else {
                                        this.hovered_result = Some(index);
                                        true
                                    }
                                } else if this.hovered_result == Some(index) {
                                    this.hovered_result = None;
                                    true
                                } else {
                                    false
                                };
                                if changed {
                                    cx.notify();
                                }
                            }))
                            .on_activate(cx.listener(move |this, _, _, cx| {
                                this.select_result(index, cx);
                            }))
                            .child(Self::element_icon(result.kind, cx))
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .when(
                                                        replace_mode && !replacement.is_empty(),
                                                        |text| {
                                                            text.text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .line_through()
                                                        },
                                                    )
                                                    .child(result.title),
                                            )
                                            .when(
                                                replace_mode && !replacement.is_empty(),
                                                |line| line.child(replacement.clone()),
                                            ),
                                    )
                                    .when_some(result.parent, |column, parent| {
                                        column.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(parent),
                                        )
                                    }),
                            )
                    }),
            )
            .into_any_element()
    }

    pub(super) fn render_search(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .debug_selector(|| "pages-search-surface".to_owned())
            .relative()
            .size_full()
            .min_h(px(280.))
            .bg(cx.theme().sidebar)
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.search_panel_bounds = Some(bounds);
            }))
            .child(self.render_search_toolbar(cx))
            .child(self.render_results_header(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .child(
                        div()
                            .id(SharedString::from(format!("{}-results-scroll", self.id)))
                            .debug_selector(|| "pages-results-viewport".to_owned())
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .overflow_scroll()
                            .track_scroll(&self.results_scroll_handle)
                            .child(self.render_results(cx)),
                    )
                    .child(
                        div()
                            .debug_selector(|| "pages-results-scrollbar-layer".to_owned())
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .child(
                                Scrollbar::vertical(&self.results_scroll_handle).id(
                                    SharedString::from(format!("{}-results-scrollbar", self.id)),
                                ),
                            ),
                    ),
            )
            .when(self.filter_menu_open, |panel| {
                panel.child(self.render_filter_menu(window, cx))
            })
            .when(self.scope_menu_open, |panel| {
                panel.child(self.render_scope_menu(window, cx))
            })
            .into_any_element()
    }
}

fn themed_icon_asset(icon: IconName, color: gpui::Hsla) -> AnyElement {
    div()
        .text_color(color)
        .child(Icon::new(icon).small())
        .into_any_element()
}

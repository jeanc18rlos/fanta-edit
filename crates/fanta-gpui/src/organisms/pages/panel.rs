use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[cfg(not(test))]
use gpui::{Animation, AnimationExt as _, ElementId};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyUpEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Point, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, deferred, div, point,
    prelude::FluentBuilder as _, px, size,
};
#[cfg(not(test))]
use gpui_component::animation::cubic_bezier;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, SelectAll},
    scroll::{Scrollbar, ScrollbarAxis},
    tooltip::Tooltip,
    v_flex,
};

use super::{
    AddPage, ClosePagesSearch, ConfirmPagesTextEntry, FindInPages, FocusFirstPage, FocusLastPage,
    FocusNextPage, FocusPreviousPage, NextSearchResult, OpenPageContextMenu,
    PAGES_PANEL_KEY_CONTEXT, PAGES_TEXT_ENTRY_KEY_CONTEXT, PagesPanelAction, PagesPanelElementKind,
    PagesPanelItem, PagesPanelResultDirection, PagesPanelSearchRequest, PagesPanelSearchResults,
    PagesPanelSearchScope, PreviousSearchResult, ReplaceAllResults, ReplaceCurrentResult,
    TogglePagesPanel, ToggleSearchSettings,
};
use crate::atoms::{
    ActivateControl, ActivateEvent, CONTROL_KEY_CONTEXT, ControlExt as _, ControlIcon,
    render_control_icon, track_bounds, truncating_label,
};
use crate::molecules::{clamp_menu_origin, list_row, menu_item, menu_surface};
use crate::toolbar::{ToolbarTool, render_tool_icon};

mod events;
mod header;
mod overlays;
mod page_list;
mod search;
#[cfg(test)]
mod tests;

/// Narrowest width the panel lays out without clipping: Figma's narrow
/// left rail. The header title and page rows truncate and the search
/// chrome compresses down to this floor (ARCHITECTURE.md §5).
pub const PAGES_PANEL_MIN_WIDTH: f32 = 240.;
/// Shortest height the panel stays fully operable at: the header plus a
/// useful slice of the page list or search results; both lists scroll.
pub const PAGES_PANEL_MIN_HEIGHT: f32 = 400.;

const HEADER_HEIGHT: f32 = 40.;
const PAGE_ROW_HEIGHT: f32 = 32.;
const PAGE_ROW_GAP: f32 = 4.;
const PAGE_PADDING: f32 = 8.;
const MAX_PAGE_LIST_HEIGHT: f32 = 320.;
const DOUBLE_ENTER_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(not(test))]
const REVEAL_DURATION: f64 = 0.18;
const ELEMENT_ICON_SIZE: f32 = 14.;
const PAGE_MENU_WIDTH: f32 = 224.;
/// Four 36px rows, two separators, and the surface padding/border.
const PAGE_MENU_HEIGHT: f32 = 196.;
const FILTER_MENU_WIDTH: f32 = 224.;
const SCOPE_MENU_WIDTH: f32 = 168.;
/// Two 36px rows plus the surface padding/border.
const SCOPE_MENU_HEIGHT: f32 = 90.;
const MENU_RADIUS: f32 = 12.;

#[derive(Clone, Debug, Eq, PartialEq)]
enum PageEditorTarget {
    Existing {
        page_id: SharedString,
        original_title: SharedString,
    },
    New,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum PanelMode {
    #[default]
    Pages,
    Find,
    Replace,
}

#[derive(Clone, Debug)]
struct PageMenuState {
    page_id: SharedString,
    page_title: SharedString,
    anchor: Point<Pixels>,
    return_focus: Option<FocusHandle>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum FocusTooltipKind {
    Find,
    AddPage,
    Settings,
    CloseSearch,
    ReplaceCurrent,
    ReplaceAll,
    PreviousResult,
    NextResult,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PageMenuAction {
    CopyLink,
    Rename,
    Duplicate,
    Delete,
}

/// Stateful, headless Pages panel.
///
/// Create this as a GPUI entity, subscribe to [`PagesPanelAction`], and render
/// the entity directly:
///
/// ```ignore
/// let panel = cx.new(|cx| PagesPanel::new("pages", pages, window, cx));
/// cx.subscribe(&panel, |host, _, event, cx| host.handle_pages(event, cx));
/// ```
///
/// The entity owns only presentation state: open/closed animation, focused
/// inputs, draft text, menus, filters, and active search result. Page data,
/// selection, search results, and all document mutations stay with the host.
pub struct PagesPanel {
    id: SharedString,
    focus_handle: FocusHandle,
    filter_menu_focus_handle: FocusHandle,
    scope_menu_focus_handle: FocusHandle,
    page_menu_focus_handle: FocusHandle,
    find_focus_handle: FocusHandle,
    add_page_focus_handle: FocusHandle,
    settings_focus_handle: FocusHandle,
    scope_trigger_focus_handle: FocusHandle,
    close_search_focus_handle: FocusHandle,
    replace_current_focus_handle: FocusHandle,
    replace_all_focus_handle: FocusHandle,
    previous_result_focus_handle: FocusHandle,
    next_result_focus_handle: FocusHandle,
    pages: Vec<PagesPanelItem>,
    expanded: bool,
    selected_page: Option<SharedString>,
    mode: PanelMode,
    editing: Option<PageEditorTarget>,
    select_page_editor_after_render: bool,
    rename_input: Entity<InputState>,
    search_input: Entity<InputState>,
    replace_input: Entity<InputState>,
    active_filters: Vec<PagesPanelElementKind>,
    search_scope: PagesPanelSearchScope,
    match_case: bool,
    whole_words: bool,
    filter_menu_open: bool,
    scope_menu_open: bool,
    page_menu: Option<PageMenuState>,
    panel_bounds: Option<Bounds<Pixels>>,
    search_panel_bounds: Option<Bounds<Pixels>>,
    filter_anchor_bounds: Option<Bounds<Pixels>>,
    scope_anchor_bounds: Option<Bounds<Pixels>>,
    tooltip_anchor_bounds: HashMap<FocusTooltipKind, Bounds<Pixels>>,
    page_row_bounds: HashMap<SharedString, Bounds<Pixels>>,
    page_focus_handles: HashMap<SharedString, FocusHandle>,
    pages_scroll_handle: ScrollHandle,
    results_scroll_handle: ScrollHandle,
    filter_menu_scroll_handle: ScrollHandle,
    results: PagesPanelSearchResults,
    active_result: Option<usize>,
    last_keyboard_page_activation: Option<(SharedString, Instant)>,
    hovered_result: Option<usize>,
    header_hovered: bool,
    hovered_page: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PagesPanelAction> for PagesPanel {}

impl PagesPanel {
    /// Creates an expanded Pages panel.
    pub fn new(
        id: impl Into<SharedString>,
        pages: Vec<PagesPanelItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("Page name"));
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find…"));
        let replace_input = cx.new(|cx| InputState::new(window, cx).placeholder("Replace with…"));

        let subscriptions = vec![
            cx.subscribe_in(
                &rename_input,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        this.commit_page_name(true, window, cx);
                    }
                    InputEvent::Blur => this.commit_page_name(false, window, cx),
                    InputEvent::Change => cx.notify(),
                    InputEvent::Focus => {}
                },
            ),
            cx.subscribe(
                &search_input,
                |this, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.emit_search_request(cx),
                    InputEvent::PressEnter { secondary } => {
                        let direction = if *secondary {
                            PagesPanelResultDirection::Previous
                        } else {
                            PagesPanelResultDirection::Next
                        };
                        this.navigate_results(direction, cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ),
            cx.subscribe(&replace_input, |_this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];

        Self {
            id: id.into(),
            focus_handle: cx.focus_handle(),
            filter_menu_focus_handle: cx.focus_handle(),
            scope_menu_focus_handle: cx.focus_handle(),
            page_menu_focus_handle: cx.focus_handle(),
            find_focus_handle: cx.focus_handle(),
            add_page_focus_handle: cx.focus_handle(),
            settings_focus_handle: cx.focus_handle(),
            scope_trigger_focus_handle: cx.focus_handle(),
            close_search_focus_handle: cx.focus_handle(),
            replace_current_focus_handle: cx.focus_handle(),
            replace_all_focus_handle: cx.focus_handle(),
            previous_result_focus_handle: cx.focus_handle(),
            next_result_focus_handle: cx.focus_handle(),
            pages,
            expanded: true,
            selected_page: None,
            mode: PanelMode::Pages,
            editing: None,
            select_page_editor_after_render: false,
            rename_input,
            search_input,
            replace_input,
            active_filters: Vec::new(),
            search_scope: PagesPanelSearchScope::default(),
            match_case: false,
            whole_words: false,
            filter_menu_open: false,
            scope_menu_open: false,
            page_menu: None,
            panel_bounds: None,
            search_panel_bounds: None,
            filter_anchor_bounds: None,
            scope_anchor_bounds: None,
            tooltip_anchor_bounds: HashMap::new(),
            page_row_bounds: HashMap::new(),
            page_focus_handles: HashMap::new(),
            pages_scroll_handle: ScrollHandle::new(),
            results_scroll_handle: ScrollHandle::new(),
            filter_menu_scroll_handle: ScrollHandle::new(),
            results: PagesPanelSearchResults::default(),
            active_result: None,
            last_keyboard_page_activation: None,
            hovered_result: None,
            header_hovered: false,
            hovered_page: None,
            _subscriptions: subscriptions,
        }
    }

    /// Replaces the host-controlled page read model.
    pub fn set_pages(&mut self, pages: Vec<PagesPanelItem>, cx: &mut Context<Self>) {
        self.pages = pages;
        let pages = &self.pages;
        self.page_row_bounds
            .retain(|page_id, _| pages.iter().any(|page| page.id == *page_id));
        self.page_focus_handles
            .retain(|page_id, _| pages.iter().any(|page| page.id == *page_id));
        if self
            .hovered_page
            .as_ref()
            .is_some_and(|page_id| !self.pages.iter().any(|page| page.id == *page_id))
        {
            self.hovered_page = None;
        }
        cx.notify();
    }

    /// Replaces the host-controlled selected page and reveals it when it is
    /// scrolled out of view.
    pub fn set_selected_page(
        &mut self,
        selected_page: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = selected_page
            .as_ref()
            .and_then(|selected| self.pages.iter().position(|page| page.id == *selected))
        {
            self.pages_scroll_handle.scroll_to_item(index);
        }
        self.selected_page = selected_page;
        cx.notify();
    }

    /// Replaces the host-controlled expanded state.
    pub fn set_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.expanded = expanded;
        cx.notify();
    }

    /// Replaces the host-controlled search result read model.
    pub fn set_search_results(&mut self, results: PagesPanelSearchResults, cx: &mut Context<Self>) {
        self.results = results;
        self.active_result = if self.results.items.is_empty() {
            None
        } else {
            Some(
                self.active_result
                    .unwrap_or(0)
                    .min(self.results.items.len() - 1),
            )
        };
        if self
            .hovered_result
            .is_some_and(|index| index >= self.results.items.len())
        {
            self.hovered_result = None;
        }
        cx.notify();
    }

    /// Returns the current search request, useful when initially populating a
    /// host result model.
    pub fn search_request(&self, cx: &App) -> PagesPanelSearchRequest {
        self.build_search_request(cx)
    }

    /// The host-controlled selected page id, for hosts verifying their echo.
    pub fn selected_page(&self) -> Option<&SharedString> {
        self.selected_page.as_ref()
    }

    /// The host-controlled search results the panel currently renders.
    pub fn search_results(&self) -> &PagesPanelSearchResults {
        &self.results
    }
}

impl Focusable for PagesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PagesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.select_page_editor_after_render {
            self.select_page_editor_after_render = false;
            window.on_next_frame(|window, cx| {
                window.dispatch_action(Box::new(SelectAll), cx);
            });
        }
        let id = self.id.clone();
        let root = v_flex()
            .id(id)
            .key_context(PAGES_PANEL_KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .capture_key_up(prevent_keyboard_activation_click)
            .on_action(cx.listener(Self::on_toggle_panel))
            .on_action(cx.listener(Self::on_find_in_pages))
            .on_action(cx.listener(Self::on_add_page))
            .on_action(cx.listener(Self::on_toggle_search_settings))
            .on_action(cx.listener(Self::on_close_search))
            // The focused single-line Input already emitted PressEnter (which
            // navigates results or commits the page draft) before propagating
            // Enter; consuming the propagated action here keeps the
            // keystroke's "\n" key_char out of the single-line fields.
            .on_action(cx.listener(|_, _: &ConfirmPagesTextEntry, _, _| {}))
            .on_action(cx.listener(Self::on_previous_search_result))
            .on_action(cx.listener(Self::on_next_search_result))
            .on_action(cx.listener(Self::on_replace_current_result))
            .on_action(cx.listener(Self::on_replace_all_results))
            .on_action(cx.listener(Self::on_focus_previous_page))
            .on_action(cx.listener(Self::on_focus_next_page))
            .on_action(cx.listener(Self::on_focus_first_page))
            .on_action(cx.listener(Self::on_focus_last_page))
            .relative()
            .w_full()
            .max_h_full()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(track_bounds(cx.entity(), |this, bounds| {
                this.panel_bounds = Some(bounds);
            }));
        let root = if self.mode == PanelMode::Pages {
            root.child(self.render_header(cx))
                .child(self.render_pages(cx))
        } else {
            root.debug_selector(|| "pages-search-panel".to_owned())
                .size_full()
                .child(self.render_search(window, cx))
        };
        let tooltip = self.render_focused_tooltip(window, cx);
        root.when(self.page_menu.is_some(), |root| {
            root.child(self.render_page_menu(window, cx))
        })
        .when_some(tooltip, |root, tooltip| root.child(tooltip))
    }
}

fn result_count_label(total: usize) -> String {
    if total == 1 {
        "1 result".to_owned()
    } else {
        format!("{total} results")
    }
}

fn prevent_keyboard_activation_click(event: &KeyUpEvent, window: &mut Window, _: &mut App) {
    if !event.keystroke.modifiers.modified()
        && matches!(event.keystroke.key.as_str(), "enter" | "space")
    {
        window.prevent_default();
    }
}

fn empty_results_label(scope: PagesPanelSearchScope) -> String {
    format!("No results on {}", scope.label().to_lowercase())
}

fn next_page_title(pages: &[PagesPanelItem]) -> SharedString {
    let next = pages
        .iter()
        .filter_map(|page| {
            page.title
                .strip_prefix("Page ")
                .and_then(|suffix| suffix.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    format!("Page {next}").into()
}

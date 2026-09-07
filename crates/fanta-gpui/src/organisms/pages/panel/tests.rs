use std::{cell::RefCell, rc::Rc};

use gpui::{
    AppContext as _, Bounds, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement as _, Pixels, Render, ScrollDelta, ScrollWheelEvent, SharedString,
    Styled as _, Subscription, TestAppContext, VisualTestContext, Window, actions, div, point, px,
    size,
};
use gpui_component::Root;

use super::{
    PAGE_ROW_HEIGHT, PAGES_PANEL_MIN_HEIGHT, PAGES_PANEL_MIN_WIDTH, PageEditorTarget, PagesPanel,
    PanelMode, empty_results_label, next_page_title, result_count_label,
};
use crate::pages::{
    AddPage, FindInPages, PagesPanelAction, PagesPanelElementKind, PagesPanelItem,
    PagesPanelResultDirection, PagesPanelSearchResult, PagesPanelSearchResults,
    PagesPanelSearchScope,
};

actions!(pages_input_regression, [CompetingDeleteSelection]);

struct TestHost {
    panel: Entity<PagesPanel>,
    actions: Rc<RefCell<Vec<PagesPanelAction>>>,
    competing_delete_count: usize,
    _subscription: Subscription,
}

impl TestHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panel = cx.new(|cx| {
            PagesPanel::new(
                "test-pages",
                vec![
                    PagesPanelItem::new("page-1", "Page 1"),
                    PagesPanelItem::new("page-2", "Page 2"),
                    PagesPanelItem::new("page-3", "Page 3"),
                ],
                window,
                cx,
            )
        });
        let actions = Rc::new(RefCell::new(Vec::new()));
        let captured_actions = actions.clone();
        let subscription = cx.subscribe(&panel, move |_, _, action: &PagesPanelAction, _| {
            captured_actions.borrow_mut().push(action.clone());
        });

        Self {
            panel,
            actions,
            competing_delete_count: 0,
            _subscription: subscription,
        }
    }
}

impl Render for TestHost {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .h_full()
            .max_w(px(320.))
            .max_h(px(220.))
            .key_context("TestCanvas")
            .on_action(cx.listener(|this, _: &CompetingDeleteSelection, _, _| {
                this.competing_delete_count += 1;
            }))
            .child(self.panel.clone())
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<TestHost>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::init(cx);
    });
    let host_slot = Rc::new(RefCell::new(None));
    let captured_host = host_slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let host = cx.new(|cx| TestHost::new(window, cx));
        *captured_host.borrow_mut() = Some(host.clone());
        Root::new(host, window, cx)
    });
    let host = host_slot
        .borrow_mut()
        .take()
        .expect("test host should be installed in the component root");
    (host, cx)
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing rendered selector: {selector}"))
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    cx.simulate_click(position, Modifiers::none());
}

fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    double_click_at(cx, position);
}

fn double_click_at(cx: &mut VisualTestContext, position: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.simulate_click(position, Modifiers::none());
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::none(),
        click_count: 2,
    });
}

fn key_down(cx: &mut VisualTestContext, key: &str) {
    cx.simulate_event(KeyDownEvent {
        keystroke: Keystroke::parse(key).expect("valid test keystroke"),
        is_held: false,
        prefer_character_input: false,
    });
    cx.run_until_parked();
}

fn key_up(cx: &mut VisualTestContext, key: &str) {
    cx.simulate_event(KeyUpEvent {
        keystroke: Keystroke::parse(key).expect("valid test keystroke"),
    });
    cx.run_until_parked();
}

fn secondary_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    secondary_click_at(cx, position);
}

fn secondary_click_at(cx: &mut VisualTestContext, position: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.simulate_event(MouseDownEvent {
        position,
        button: MouseButton::Right,
        modifiers: Modifiers::none(),
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        button: MouseButton::Right,
        modifiers: Modifiers::none(),
        click_count: 1,
    });
    cx.run_until_parked();
}

fn panel(host: &Entity<TestHost>, cx: &VisualTestContext) -> Entity<PagesPanel> {
    cx.read(|app| host.read(app).panel.clone())
}

fn actions(host: &Entity<TestHost>, cx: &VisualTestContext) -> Rc<RefCell<Vec<PagesPanelAction>>> {
    cx.read(|app| host.read(app).actions.clone())
}

fn read_panel<R>(
    panel: &Entity<PagesPanel>,
    cx: &VisualTestContext,
    read: impl FnOnce(&PagesPanel) -> R,
) -> R {
    cx.read(|app| read(panel.read(app)))
}

fn set_one_search_result(panel: &Entity<PagesPanel>, cx: &mut VisualTestContext) {
    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_search_results(
                PagesPanelSearchResults {
                    total: 1,
                    items: vec![
                        PagesPanelSearchResult::new(
                            "result-1",
                            "Long page heading",
                            PagesPanelElementKind::Text,
                        )
                        .parent("Hero"),
                    ],
                    element_counts: Vec::new(),
                },
                cx,
            );
        });
    });
    cx.run_until_parked();
}

fn focus_panel(panel: &Entity<PagesPanel>, cx: &mut VisualTestContext) {
    cx.update(|window, app| {
        panel.focus_handle(app).focus(window, app);
    });
    cx.run_until_parked();
}

#[test]
fn next_page_title_uses_the_highest_page_number_not_the_count() {
    let pages = vec![
        PagesPanelItem::new("a", "Page 1"),
        PagesPanelItem::new("b", "Page 3"),
        PagesPanelItem::new("c", "Marketing"),
    ];

    assert_eq!(next_page_title(&pages).as_ref(), "Page 4");
}

#[test]
fn next_page_title_starts_at_one() {
    let pages = vec![PagesPanelItem::new("a", "Untitled")];

    assert_eq!(next_page_title(&pages).as_ref(), "Page 1");
}

#[test]
fn empty_results_use_a_count_header_and_a_scoped_body_message() {
    assert_eq!(result_count_label(0), "0 results");
    assert_eq!(result_count_label(1), "1 result");
    assert_eq!(result_count_label(12), "12 results");
    assert_eq!(
        empty_results_label(PagesPanelSearchScope::CurrentPage),
        "No results on this page"
    );
    assert_eq!(
        empty_results_label(PagesPanelSearchScope::AllPages),
        "No results on all pages"
    );
}

#[gpui::test]
fn empty_results_render_in_the_scrollable_body_without_header_overflow(cx: &mut TestAppContext) {
    let (_host, cx) = setup(cx);

    click(cx, "pages-search-trigger");
    click(cx, "pages-scope-trigger");
    click(cx, "pages-scope-all-pages");

    let panel = bounds(cx, "pages-search-panel");
    let header = bounds(cx, "pages-results-header");
    let count = bounds(cx, "pages-result-count");
    let previous = bounds(cx, "pages-previous-result");
    let next = bounds(cx, "pages-next-result");
    let viewport = bounds(cx, "pages-results-viewport");
    let empty = bounds(cx, "pages-results-empty");

    assert!(header.right() <= panel.right());
    assert!(count.left() >= header.left() && count.right() <= header.right());
    assert!(previous.left() >= header.left());
    assert!(next.right() <= header.right());
    assert!(empty.top() >= viewport.top());
    assert!(empty.right() <= viewport.right());
}

#[gpui::test]
fn zero_results_prevent_replace_and_navigation_actions(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let actions = actions(&host, cx);

    click(cx, "pages-search-trigger");
    click(cx, "pages-filter-trigger");
    click(cx, "pages-mode-replace");

    actions.borrow_mut().clear();
    click(cx, "pages-previous-result");
    click(cx, "pages-next-result");
    click(cx, "pages-replace-one");
    click(cx, "pages-replace-all");

    assert!(
        actions.borrow().is_empty(),
        "disabled zero-result controls must not emit host intents"
    );
}

#[gpui::test]
fn registered_commands_drive_the_same_headless_component_state(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.dispatch_action(FindInPages);
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);

    cx.dispatch_action(AddPage);
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Pages);
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::New)
    ));
}

#[gpui::test]
fn search_query_accepts_backward_and_forward_deletion(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.dispatch_action(FindInPages);
    cx.run_until_parked();

    cx.update(|_, app| {
        app.clear_key_bindings();
        app.bind_keys([
            KeyBinding::new("backspace", CompetingDeleteSelection, Some("TestCanvas")),
            KeyBinding::new("delete", CompetingDeleteSelection, Some("TestCanvas")),
        ]);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("a b c backspace");
    assert_eq!(
        cx.read(|app| panel.read(app).search_input.read(app).value()),
        "ab"
    );

    cx.simulate_keystrokes("left delete");
    assert_eq!(
        cx.read(|app| panel.read(app).search_input.read(app).value()),
        "a"
    );

    cx.simulate_keystrokes("b c shift-left shift-left backspace");
    assert_eq!(
        cx.read(|app| panel.read(app).search_input.read(app).value()),
        "a"
    );

    cx.simulate_keystrokes("secondary-a delete");
    assert!(cx.read(|app| panel.read(app).search_input.read(app).value().is_empty()));
    assert_eq!(
        cx.read(|app| host.read(app).competing_delete_count),
        0,
        "focused Pages search must suppress a competing canvas delete action"
    );
}

#[gpui::test]
fn enter_confirms_search_entries_without_typing_a_newline(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.dispatch_action(FindInPages);
    cx.run_until_parked();

    cx.simulate_keystrokes("q u e r y");
    set_one_search_result(&panel, cx);

    cx.simulate_keystrokes("enter enter");
    assert_eq!(
        cx.read(|app| panel.read(app).search_input.read(app).value()),
        "query",
        "Enter must navigate results, not type into the single-line query"
    );
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.active_result),
        Some(0),
        "the consumed Enter keystroke must not reset the active result"
    );

    click(cx, "pages-filter-trigger");
    click(cx, "pages-mode-replace");
    cx.simulate_keystrokes("n e w enter");
    assert_eq!(
        cx.read(|app| panel.read(app).replace_input.read(app).value()),
        "new",
        "Enter must not type into the single-line replacement field"
    );
}

#[gpui::test]
fn shortcuts_and_tab_activation_cover_pages_and_search_controls(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab enter");
    assert!(
        !read_panel(&panel, cx, |panel| panel.expanded),
        "Enter should activate the header reached with Tab"
    );

    cx.simulate_keystrokes("tab enter");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.mode),
        PanelMode::Find,
        "the next tab stop should be the Find button"
    );

    cx.simulate_keystrokes("tab enter");
    assert!(
        read_panel(&panel, cx, |panel| panel.filter_menu_open),
        "Tab should leave the search input and Enter should open Settings"
    );

    cx.simulate_keystrokes("tab enter");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.mode),
        PanelMode::Replace,
        "the open Settings menu should move focus to Find, then Tab to Replace"
    );

    cx.simulate_keystrokes("tab enter");
    assert!(
        read_panel(&panel, cx, |panel| panel.scope_menu_open),
        "Tab should leave the replace input for the search-scope control"
    );

    cx.simulate_keystrokes("tab enter");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.search_scope),
        PagesPanelSearchScope::AllPages,
        "the scope menu should focus This page, then Tab to All pages"
    );
}

#[gpui::test]
fn key_up_does_not_reverse_a_header_disclosure_activation(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab");
    key_down(cx, "enter");
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));

    key_up(cx, "enter");
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::ExpansionChanged { expanded: false }]
    );
}

#[gpui::test]
fn closing_search_from_the_keyboard_restores_tab_focus_and_expands_the_header(
    cx: &mut TestAppContext,
) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    click(cx, "pages-header");
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("secondary-f");
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);

    cx.simulate_keystrokes("tab tab enter");
    cx.run_until_parked();
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Pages);

    cx.simulate_keystrokes("tab enter");
    assert!(
        read_panel(&panel, cx, |panel| panel.expanded),
        "the restored panel focus should make the collapsed header the next tab stop"
    );
}

#[gpui::test]
fn complete_keyboard_lifecycle_keeps_filter_and_scope_menus_open(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("secondary-f tab");
    key_down(cx, "enter");
    assert!(read_panel(&panel, cx, |panel| panel.filter_menu_open));
    assert!(bounds(cx, "pages-filter-menu").size.height > px(0.));
    key_up(cx, "enter");
    assert!(read_panel(&panel, cx, |panel| panel.filter_menu_open));
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);

    click(cx, "pages-filter-trigger");
    assert!(!read_panel(&panel, cx, |panel| panel.filter_menu_open));

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab tab");
    key_down(cx, "enter");
    assert!(read_panel(&panel, cx, |panel| panel.scope_menu_open));
    assert!(bounds(cx, "pages-scope-menu").size.height > px(0.));
    key_up(cx, "enter");
    assert!(read_panel(&panel, cx, |panel| panel.scope_menu_open));
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.search_scope),
        PagesPanelSearchScope::CurrentPage
    );

    click(cx, "pages-scope-trigger");
    click(cx, "pages-filter-trigger");
    cx.run_until_parked();
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("tab tab tab");
    key_down(cx, "enter");
    key_up(cx, "enter");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.active_filters.clone()),
        vec![PagesPanelElementKind::Text],
        "one Enter key lifecycle must toggle a filter exactly once"
    );
    assert_eq!(
        actions
            .borrow()
            .iter()
            .filter(|action| matches!(action, PagesPanelAction::SearchRequested(_)))
            .count(),
        1
    );
}

#[gpui::test]
fn escape_dismisses_search_popovers_before_closing_search(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("secondary-f tab enter");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.filter_menu_open));

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    assert!(!read_panel(&panel, cx, |panel| panel.filter_menu_open));
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);
    assert!(actions.borrow().is_empty());
    assert!(cx.update(|window, app| { panel.read(app).settings_focus_handle.is_focused(window) }));

    cx.simulate_keystrokes("escape");
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Pages);
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SearchClosed]
    );

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("secondary-f tab tab tab enter");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.scope_menu_open));

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    assert!(!read_panel(&panel, cx, |panel| panel.scope_menu_open));
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);
    assert!(actions.borrow().is_empty());
    assert!(cx.update(|window, app| {
        panel
            .read(app)
            .scope_trigger_focus_handle
            .is_focused(window)
    }));

    cx.simulate_keystrokes("escape");
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Pages);
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SearchClosed]
    );
}

#[gpui::test]
fn tab_focus_opens_the_same_command_tooltips_as_pointer_hover(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab");
    cx.run_until_parked();
    let find_tooltip = bounds(cx, "pages-focus-tooltip");
    assert!(find_tooltip.size.width > px(0.) && find_tooltip.size.height > px(0.));

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    let add_tooltip = bounds(cx, "pages-focus-tooltip");
    assert!(add_tooltip.size.width > px(0.) && add_tooltip.size.height > px(0.));
    assert_ne!(
        add_tooltip.origin, find_tooltip.origin,
        "the tooltip should follow the newly focused Add page control"
    );
}

#[gpui::test]
fn tab_order_reaches_page_and_result_rows(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("tab tab tab tab enter");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-1".into(),
        }],
        "header, Find, Add, then the first page row should form the page tab order"
    );

    focus_panel(&panel, cx);
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-tab enter");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-3".into(),
        }],
        "Shift-Tab should traverse the page tab order in reverse"
    );

    set_one_search_result(&panel, cx);
    focus_panel(&panel, cx);
    cx.simulate_keystrokes("secondary-f");
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("tab tab tab tab tab tab enter");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SearchResultSelected {
            result_id: "result-1".into(),
        }],
        "search input, Settings, Close, scope, navigation, then result rows should be tabbable"
    );
}

#[gpui::test]
fn adding_a_page_scrolls_and_focuses_the_editor_then_restores_tab_navigation(
    cx: &mut TestAppContext,
) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);
    let pages = (1..=18)
        .map(|index| PagesPanelItem::new(format!("page-{index}"), format!("Page {index}")))
        .collect();
    cx.update(|_, app| {
        panel.update(app, |panel, cx| panel.set_pages(pages, cx));
    });
    cx.run_until_parked();

    click(cx, "pages-add-trigger");
    cx.run_until_parked();
    let viewport = bounds(cx, "pages-scroll-viewport");
    let editor = bounds(cx, "pages-page-editor");
    let scroll_handle = read_panel(&panel, cx, |panel| panel.pages_scroll_handle.clone());
    assert!(editor.top() >= viewport.top());
    assert!(editor.bottom() <= viewport.bottom());
    assert_eq!(scroll_handle.offset().y, -scroll_handle.max_offset().y);

    cx.simulate_keystrokes("x");
    let editor_value = cx.read(|app| panel.read(app).rename_input.read(app).value());
    assert!(
        editor_value.ends_with('x'),
        "typing should go directly to the newly revealed editor"
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::CreateRequested {
            title: editor_value,
        }]
    );
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));

    cx.simulate_keystrokes("tab enter");
    assert!(
        !read_panel(&panel, cx, |panel| panel.expanded),
        "committing the new page should restore the panel as a tab-navigation owner"
    );
}

#[gpui::test]
fn double_enter_renames_the_focused_page_row(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("tab tab tab tab enter enter");
    cx.run_until_parked();

    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-1".into(),
        }]
    );
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::Existing { ref page_id, .. })
            if page_id.as_ref() == "page-1"
    ));
    assert!(bounds(cx, "pages-page-editor").size.height > px(0.));
}

#[gpui::test]
fn result_navigation_shortcuts_emit_typed_intents(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);
    set_one_search_result(&panel, cx);
    focus_panel(&panel, cx);

    cx.simulate_keystrokes("secondary-f");
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-secondary-f shift-secondary-d");
    assert_eq!(
        actions.borrow().as_slice(),
        &[
            PagesPanelAction::NavigateResults {
                direction: PagesPanelResultDirection::Next,
                result_id: Some("result-1".into()),
            },
            PagesPanelAction::NavigateResults {
                direction: PagesPanelResultDirection::Previous,
                result_id: Some("result-1".into()),
            },
        ]
    );
}

#[gpui::test]
fn clicking_another_page_cancels_rename_and_new_page_edits(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    let first_page_bounds = bounds(cx, "pages-row-page-1");
    let viewport_bounds = bounds(cx, "pages-scroll-viewport");
    let first_page_center = first_page_bounds.center();
    cx.simulate_mouse_move(first_page_center, None, Modifiers::none());
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.hovered_page.clone()),
        Some("page-1".into()),
        "row bounds: {first_page_bounds:?}; viewport bounds: {viewport_bounds:?}"
    );
    double_click(cx, "pages-row-page-1");
    let editing = read_panel(&panel, cx, |panel| panel.editing.clone());
    assert!(
        editing.is_some(),
        "double click did not start editing; actions: {:?}",
        actions.borrow().as_slice()
    );
    assert!(bounds(cx, "pages-page-editor").size.height > px(0.));
    assert!(matches!(
        editing,
        Some(PageEditorTarget::Existing { ref page_id, .. }) if page_id.as_ref() == "page-1"
    ));

    actions.borrow_mut().clear();
    click(cx, "pages-row-page-2");
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-2".into(),
        }]
    );

    click(cx, "pages-add-trigger");
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::New)
    ));

    actions.borrow_mut().clear();
    click(cx, "pages-row-page-3");
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-3".into(),
        }]
    );
    assert!(!actions.borrow().iter().any(|action| matches!(
        action,
        PagesPanelAction::CreateRequested { .. } | PagesPanelAction::RenameRequested { .. }
    )));
}

#[gpui::test]
fn page_context_menu_emits_headless_intents_and_starts_inline_rename(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    secondary_click(cx, "pages-row-page-2");
    assert!(bounds(cx, "pages-page-menu-copy-link").size.width > px(0.));
    actions.borrow_mut().clear();
    click(cx, "pages-page-menu-copy-link");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::CopyLinkRequested {
            page_id: "page-2".into(),
        }]
    );

    secondary_click(cx, "pages-row-page-2");
    actions.borrow_mut().clear();
    click(cx, "pages-page-menu-duplicate");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::DuplicateRequested {
            page_id: "page-2".into(),
        }]
    );

    secondary_click(cx, "pages-row-page-2");
    actions.borrow_mut().clear();
    click(cx, "pages-page-menu-delete");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::DeleteRequested {
            page_id: "page-2".into(),
        }]
    );

    secondary_click(cx, "pages-row-page-1");
    actions.borrow_mut().clear();
    click(cx, "pages-page-menu-rename");
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::Existing { ref page_id, .. })
            if page_id.as_ref() == "page-1"
    ));
    assert!(
        actions.borrow().is_empty(),
        "rename is emitted only after the inline editor commits"
    );
    assert!(bounds(cx, "pages-page-editor").size.height > px(0.));
}

#[gpui::test]
fn page_menu_is_row_scoped_and_ctrl_enter_opens_it_for_keyboard_users(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    secondary_click(cx, "pages-header");
    assert!(cx.debug_bounds("pages-page-menu").is_none());

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab tab ctrl-enter");
    cx.run_until_parked();
    assert!(bounds(cx, "pages-page-menu").size.height > px(0.));
    assert!(bounds(cx, "pages-page-menu-copy-link").size.height > px(0.));

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::CopyLinkRequested {
            page_id: "page-1".into(),
        }]
    );

    secondary_click(cx, "pages-row-page-2");
    assert!(bounds(cx, "pages-page-menu").size.height > px(0.));
    click(cx, "pages-header");
    cx.run_until_parked();
    assert!(
        read_panel(&panel, cx, |panel| panel.page_menu.is_none()),
        "a header click must dismiss the panel-owned page menu state"
    );
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));

    secondary_click(cx, "pages-header");
    assert!(
        read_panel(&panel, cx, |panel| panel.page_menu.is_none()),
        "clipped rows must never receive a secondary click through the collapsed header"
    );
}

#[gpui::test]
fn escape_closes_the_page_menu_and_restores_the_page_row_focus(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab tab ctrl-enter");
    cx.run_until_parked();
    let return_focus = read_panel(&panel, cx, |panel| {
        panel
            .page_menu
            .as_ref()
            .and_then(|menu| menu.return_focus.clone())
            .expect("keyboard-opened menu should remember its page row")
    });
    assert!(bounds(cx, "pages-page-menu").size.height > px(0.));

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    assert!(read_panel(&panel, cx, |panel| panel.page_menu.is_none()));
    assert!(cx.update(|window, _| return_focus.is_focused(window)));
    assert!(actions.borrow().is_empty());

    cx.simulate_keystrokes("escape");
    assert!(actions.borrow().is_empty());
}

#[gpui::test]
fn search_result_rows_do_not_open_the_page_context_menu(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);
    set_one_search_result(&panel, cx);
    click(cx, "pages-search-trigger");

    actions.borrow_mut().clear();
    secondary_click(cx, "pages-result-0");

    assert!(cx.debug_bounds("pages-page-menu-copy-link").is_none());
    assert!(actions.borrow().is_empty());
}

#[gpui::test]
fn popovers_anchor_top_left_and_clamp_inside_the_window(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);
    set_one_search_result(&panel, cx);

    click(cx, "pages-search-trigger");
    let find_trigger = bounds(cx, "pages-filter-trigger");
    click(cx, "pages-filter-trigger");
    let find_menu = bounds(cx, "pages-filter-menu");
    assert_eq!(
        find_menu.origin.x, find_trigger.origin.x,
        "an interior anchor is not displaced by clamping"
    );
    assert_eq!(find_menu.origin.y, find_trigger.bottom() + px(4.));

    click(cx, "pages-mode-replace");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.mode),
        PanelMode::Replace
    );
    let replace_trigger = bounds(cx, "pages-filter-trigger");
    assert_eq!(replace_trigger.origin, find_trigger.origin);

    click(cx, "pages-filter-trigger");
    let replace_menu = bounds(cx, "pages-filter-menu");
    assert_eq!(replace_menu.origin, find_menu.origin);
    assert_eq!(replace_menu.origin.x, replace_trigger.origin.x);
    assert_eq!(replace_menu.origin.y, replace_trigger.bottom() + px(4.));

    click(cx, "pages-filter-trigger");
    let scope_trigger = bounds(cx, "pages-scope-trigger");
    click(cx, "pages-scope-trigger");
    let scope_menu = bounds(cx, "pages-scope-menu");
    assert_eq!(scope_menu.origin.x, scope_trigger.origin.x);
    assert_eq!(scope_menu.origin.y, scope_trigger.bottom() + px(4.));

    actions.borrow_mut().clear();
    click(cx, "pages-scope-all-pages");
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.search_scope),
        PagesPanelSearchScope::AllPages
    );
    assert!(matches!(
        actions.borrow().last(),
        Some(PagesPanelAction::SearchRequested(request))
            if request.scope == PagesPanelSearchScope::AllPages
    ));

    cx.simulate_resize(size(px(320.), px(220.)));
    cx.run_until_parked();
    click(cx, "pages-filter-trigger");
    let clamped_menu = bounds(cx, "pages-filter-menu");
    assert!(
        clamped_menu.right() <= px(320.),
        "a small window clamps the fixed-width menu on the x axis: {clamped_menu:?}"
    );
    assert!(
        clamped_menu.bottom() <= px(220.),
        "a menu taller than the window pins to the top edge and scrolls internally"
    );
    assert_eq!(clamped_menu.top(), px(0.));
}

#[gpui::test]
fn page_context_menu_clamps_inside_the_window(cx: &mut TestAppContext) {
    let (_host, cx) = setup(cx);

    cx.simulate_resize(size(px(320.), px(220.)));
    cx.run_until_parked();
    let row = bounds(cx, "pages-row-page-3");
    secondary_click_at(cx, point(row.right() - px(2.), row.center().y));

    let menu = bounds(cx, "pages-page-menu");
    assert!(
        menu.right() <= px(320.),
        "a near-edge anchor must not push the menu off the window: {menu:?}"
    );
    assert!(menu.left() >= px(0.));
    assert!(
        menu.bottom() <= px(220.),
        "the bottom edge clamps instead of rendering off screen: {menu:?}"
    );
    assert!(menu.top() >= px(0.));
    assert!(
        menu.top() < row.center().y,
        "the clamped menu shifts up from its pointer anchor"
    );
    assert!(bounds(cx, "pages-page-menu-delete").size.height > px(0.));
}

#[gpui::test]
fn popup_occlusion_prevents_hover_from_leaking_to_results(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    set_one_search_result(&panel, cx);
    click(cx, "pages-search-trigger");
    click(cx, "pages-filter-trigger");

    let overlap = bounds(cx, "pages-filter-menu").intersect(&bounds(cx, "pages-result-0"));
    assert!(overlap.size.width > px(0.) && overlap.size.height > px(0.));
    let overlap_point = overlap.center();

    click(cx, "pages-filter-trigger");
    cx.simulate_mouse_move(overlap_point, None, Modifiers::none());
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.hovered_result),
        Some(0)
    );

    click(cx, "pages-filter-trigger");
    cx.simulate_mouse_move(overlap_point, None, Modifiers::none());
    assert_eq!(read_panel(&panel, cx, |panel| panel.hovered_result), None);
}

#[gpui::test]
fn collapsed_header_hover_excludes_its_action_buttons(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    click(cx, "pages-header");
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));

    let header = bounds(cx, "pages-header");
    cx.simulate_mouse_move(
        point(header.left() + px(12.), header.center().y),
        None,
        Modifiers::none(),
    );
    assert!(read_panel(&panel, cx, |panel| panel.header_hovered));

    let search_button_center = bounds(cx, "pages-search-trigger").center();
    cx.simulate_mouse_move(search_button_center, None, Modifiers::none());
    assert!(!read_panel(&panel, cx, |panel| panel.header_hovered));

    actions.borrow_mut().clear();
    click(cx, "pages-search-trigger");
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Find);
    assert!(!read_panel(&panel, cx, |panel| panel.expanded));
    assert!(actions.borrow().is_empty());
}

#[gpui::test]
fn collapsed_long_page_titles_truncate_before_the_header_actions(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let long_id: SharedString = "long-selected-page".into();
    let long_title =
        "This selected page title is intentionally much wider than the collapsed Pages panel";
    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_pages(
                vec![
                    PagesPanelItem::new(long_id.clone(), long_title),
                    PagesPanelItem::new("page-2", "Page 2"),
                ],
                cx,
            );
            panel.set_selected_page(Some(long_id.clone()), cx);
        });
    });
    cx.run_until_parked();

    click(cx, "pages-header");
    let assert_constrained = |cx: &mut VisualTestContext| {
        let header = bounds(cx, "pages-header");
        let title = bounds(cx, "pages-header-title");
        let search = bounds(cx, "pages-search-trigger");
        let add = bounds(cx, "pages-add-trigger");
        assert!(title.right() <= search.left());
        assert!(search.right() <= add.left());
        assert!(add.right() <= header.right());
    };
    assert_constrained(cx);

    cx.simulate_resize(size(px(220.), px(180.)));
    cx.run_until_parked();
    assert_constrained(cx);
}

#[gpui::test]
fn search_surface_compresses_to_the_panel_floor_width(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    cx.simulate_resize(size(px(PAGES_PANEL_MIN_WIDTH), px(PAGES_PANEL_MIN_HEIGHT)));
    cx.run_until_parked();

    focus_panel(&panel, cx);
    cx.dispatch_action(FindInPages);
    cx.run_until_parked();
    set_one_search_result(&panel, cx);

    // Every toolbar and results-header control keeps its hit area inside
    // the floor: the query input and the scope label are the compressible
    // runs, so the fixed controls never spill past the panel edge.
    let close = bounds(cx, "pages-close-search");
    let filters = bounds(cx, "pages-filter-trigger");
    assert!(
        close.right() <= px(PAGES_PANEL_MIN_WIDTH + 0.5),
        "close-search must stay inside the floor, got {:?}",
        close.right()
    );
    assert!(filters.right() <= close.left());
    let count = bounds(cx, "pages-result-count");
    let scope = bounds(cx, "pages-scope-trigger");
    let previous = bounds(cx, "pages-previous-result");
    let next = bounds(cx, "pages-next-result");
    assert!(count.right() <= scope.left());
    assert!(
        scope.right() <= previous.left(),
        "the scope trigger must compress ahead of the navigation controls, got scope {:?} vs previous {:?}",
        scope.right(),
        previous.left()
    );
    assert!(previous.right() <= next.left());
    assert!(
        next.right() <= px(PAGES_PANEL_MIN_WIDTH + 0.5),
        "next-result must stay inside the floor, got {:?}",
        next.right()
    );

    // Activation still flows at the floor width: closing search emits the
    // typed intent and returns to the Pages surface.
    actions.borrow_mut().clear();
    click(cx, "pages-close-search");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SearchClosed]
    );
    assert_eq!(read_panel(&panel, cx, |panel| panel.mode), PanelMode::Pages);
}

#[gpui::test]
fn long_page_names_truncate_and_many_pages_scroll_vertically(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let pages = (1..=14)
        .map(|index| {
            let title = if index == 1 {
                "This page name is intentionally far wider than the Pages panel viewport".to_owned()
            } else {
                format!("Page {index}")
            };
            PagesPanelItem::new(format!("long-page-{index}"), title)
        })
        .collect();
    cx.update(|_, app| {
        panel.update(app, |panel, cx| panel.set_pages(pages, cx));
    });
    cx.run_until_parked();

    let viewport = bounds(cx, "pages-scroll-viewport");
    let scrollbar_layer = bounds(cx, "pages-scrollbar-layer");
    let long_row = bounds(cx, "pages-row-long-page-1");
    let short_row = bounds(cx, "pages-row-long-page-2");
    let scroll_handle = read_panel(&panel, cx, |panel| panel.pages_scroll_handle.clone());
    assert_eq!(scrollbar_layer, viewport);
    assert_eq!(scroll_handle.bounds(), viewport);
    assert!(
        long_row.size.width <= viewport.size.width,
        "a long page name truncates instead of widening its row past the viewport"
    );
    assert_eq!(
        long_row.size.width, short_row.size.width,
        "every page row shares the viewport width"
    );
    assert_eq!(
        scroll_handle.max_offset().x,
        px(0.),
        "truncated rows must not create horizontal scroll range"
    );
    assert!(scroll_handle.max_offset().y > px(0.));

    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
        ..Default::default()
    });
    assert!(scroll_handle.offset().y < px(0.));
    assert_eq!(bounds(cx, "pages-scrollbar-layer"), viewport);

    cx.simulate_resize(size(px(260.), px(180.)));
    cx.run_until_parked();
    let resized_viewport = bounds(cx, "pages-scroll-viewport");
    assert_ne!(resized_viewport.size, viewport.size);
    assert_eq!(bounds(cx, "pages-scrollbar-layer"), resized_viewport);
    assert_eq!(scroll_handle.bounds(), resized_viewport);
    assert!(bounds(cx, "pages-row-long-page-1").size.width <= resized_viewport.size.width);
}

#[gpui::test]
fn page_editor_fills_the_row_width_regardless_of_its_text(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let long_title =
        "This sibling is intentionally wider than the viewport but should not widen Page 2";
    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_pages(
                vec![
                    PagesPanelItem::new("long-page", long_title),
                    PagesPanelItem::new("short-page", "Page 2"),
                ],
                cx,
            );
        });
    });
    cx.run_until_parked();

    let viewport = bounds(cx, "pages-scroll-viewport");
    let long_row = bounds(cx, "pages-row-long-page");
    let short_row = bounds(cx, "pages-row-short-page");
    assert!(long_row.size.width <= viewport.size.width);
    assert_eq!(long_row.size.width, short_row.size.width);

    double_click(cx, "pages-row-short-page");
    cx.run_until_parked();
    let editor = bounds(cx, "pages-page-editor");
    let input_slot = bounds(cx, "pages-page-editor-input-slot");
    assert_eq!(editor.size.height, px(PAGE_ROW_HEIGHT));
    assert_eq!(editor.size.width, short_row.size.width);
    assert_eq!(input_slot.size.width, editor.size.width);

    cx.simulate_input(&"x".repeat(160));
    let scroll_handle = read_panel(&panel, cx, |panel| panel.pages_scroll_handle.clone());
    assert_eq!(
        bounds(cx, "pages-page-editor").size.width,
        editor.size.width,
        "long draft text scrolls inside the input instead of widening the editor"
    );
    assert_eq!(scroll_handle.max_offset().x, px(0.));
    let editor_value = cx.read(|app| panel.read(app).rename_input.read(app).value());
    assert!(
        editor_value.ends_with('x'),
        "typing must continue to reach the editor"
    );
}

#[gpui::test]
fn escape_cancels_the_active_draft_without_emitting_or_closing(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    double_click(cx, "pages-row-page-1");
    cx.run_until_parked();
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::Existing { .. })
    ));
    cx.simulate_input("Draft change");
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(
        read_panel(&panel, cx, |panel| panel.editing.is_none()),
        "Escape must cancel the rename draft before any other ladder step"
    );
    assert!(
        actions.borrow().is_empty(),
        "a cancelled draft must not emit a rename"
    );

    click(cx, "pages-add-trigger");
    cx.run_until_parked();
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::New)
    ));
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert!(
        actions.borrow().is_empty(),
        "a cancelled new-page draft must not emit a create"
    );
}

#[gpui::test]
fn rename_commits_emit_only_nonempty_changed_titles(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    double_click(cx, "pages-row-page-1");
    cx.run_until_parked();
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert!(
        actions.borrow().is_empty(),
        "committing an unchanged title must not emit a rename"
    );

    double_click(cx, "pages-row-page-1");
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("Renamed");
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::RenameRequested {
            page_id: "page-1".into(),
            title: "Renamed".into(),
        }]
    );

    // Blur commits are driven by window focus-path events, which only fire
    // while the window is active.
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    double_click(cx, "pages-row-page-2");
    cx.run_until_parked();
    assert!(matches!(
        read_panel(&panel, cx, |panel| panel.editing.clone()),
        Some(PageEditorTarget::Existing { ref page_id, .. }) if page_id.as_ref() == "page-2"
    ));
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("Blurred");
    actions.borrow_mut().clear();
    focus_panel(&panel, cx);
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::RenameRequested {
            page_id: "page-2".into(),
            title: "Blurred".into(),
        }],
        "moving focus away commits the draft"
    );
}

fn focused_page_id(panel: &Entity<PagesPanel>, cx: &mut VisualTestContext) -> Option<SharedString> {
    cx.update(|window, app| {
        let panel = panel.read(app);
        panel
            .pages
            .iter()
            .find(|page| {
                panel
                    .page_focus_handles
                    .get(&page.id)
                    .is_some_and(|handle| handle.is_focused(window))
            })
            .map(|page| page.id.clone())
    })
}

#[gpui::test]
fn arrow_keys_traverse_page_rows_with_home_and_end(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let actions = actions(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab tab");
    assert_eq!(focused_page_id(&panel, cx), Some("page-1".into()));

    cx.simulate_keystrokes("down");
    assert_eq!(focused_page_id(&panel, cx), Some("page-2".into()));

    cx.simulate_keystrokes("down");
    assert_eq!(focused_page_id(&panel, cx), Some("page-3".into()));

    cx.simulate_keystrokes("down");
    assert_eq!(
        focused_page_id(&panel, cx),
        Some("page-3".into()),
        "Down clamps at the last page row"
    );

    cx.simulate_keystrokes("up");
    assert_eq!(focused_page_id(&panel, cx), Some("page-2".into()));

    cx.simulate_keystrokes("home");
    assert_eq!(focused_page_id(&panel, cx), Some("page-1".into()));

    cx.simulate_keystrokes("up");
    assert_eq!(
        focused_page_id(&panel, cx),
        Some("page-1".into()),
        "Up clamps at the first page row"
    );

    cx.simulate_keystrokes("end");
    assert_eq!(focused_page_id(&panel, cx), Some("page-3".into()));

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    assert_eq!(
        actions.borrow().as_slice(),
        &[PagesPanelAction::SelectRequested {
            page_id: "page-3".into(),
        }],
        "the row reached with arrow keys activates like any other control"
    );
}

#[gpui::test]
fn set_selected_page_scrolls_any_offscreen_page_into_view(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let pages = (1..=18)
        .map(|index| PagesPanelItem::new(format!("page-{index}"), format!("Page {index}")))
        .collect();
    cx.update(|_, app| {
        panel.update(app, |panel, cx| panel.set_pages(pages, cx));
    });
    cx.run_until_parked();
    let scroll_handle = read_panel(&panel, cx, |panel| panel.pages_scroll_handle.clone());
    assert_eq!(scroll_handle.offset().y, px(0.));

    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_selected_page(Some("page-12".into()), cx);
        });
    });
    cx.run_until_parked();
    let viewport = bounds(cx, "pages-scroll-viewport");
    let row = bounds(cx, "pages-row-page-12");
    assert!(
        scroll_handle.offset().y < px(0.),
        "selecting a page below the fold must scroll the list"
    );
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());

    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_selected_page(Some("page-2".into()), cx);
        });
    });
    cx.run_until_parked();
    let viewport = bounds(cx, "pages-scroll-viewport");
    let row = bounds(cx, "pages-row-page-2");
    assert!(
        row.top() >= viewport.top() && row.bottom() <= viewport.bottom(),
        "selecting a page above the fold must scroll back up"
    );
}

#[gpui::test]
fn set_pages_prunes_stale_row_bounds_and_focus_handles(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);

    let removed = SharedString::from("page-3");
    let kept = SharedString::from("page-1");
    assert!(read_panel(&panel, cx, |panel| {
        panel.page_row_bounds.contains_key(&removed)
            && panel.page_focus_handles.contains_key(&removed)
    }));

    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_pages(
                vec![
                    PagesPanelItem::new("page-1", "Page 1"),
                    PagesPanelItem::new("page-2", "Page 2"),
                ],
                cx,
            );
        });
    });
    cx.run_until_parked();

    assert!(read_panel(&panel, cx, |panel| {
        !panel.page_row_bounds.contains_key(&removed)
            && !panel.page_focus_handles.contains_key(&removed)
    }));
    assert!(read_panel(&panel, cx, |panel| {
        panel.page_row_bounds.contains_key(&kept) && panel.page_focus_handles.contains_key(&kept)
    }));
}

#[gpui::test]
fn search_results_have_a_bounded_scrollable_viewport(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let panel = panel(&host, cx);
    let items = (1..=24)
        .map(|index| {
            PagesPanelSearchResult::new(
                format!("result-{index}"),
                format!("Result {index}"),
                PagesPanelElementKind::Text,
            )
            .parent("Frame")
        })
        .collect::<Vec<_>>();
    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            panel.set_search_results(
                PagesPanelSearchResults {
                    total: items.len(),
                    items,
                    element_counts: Vec::new(),
                },
                cx,
            );
        });
    });
    cx.run_until_parked();
    click(cx, "pages-search-trigger");

    let viewport = bounds(cx, "pages-results-viewport");
    let scrollbar_layer = bounds(cx, "pages-results-scrollbar-layer");
    let scroll_handle = read_panel(&panel, cx, |panel| panel.results_scroll_handle.clone());
    assert_eq!(scrollbar_layer, viewport);
    assert_eq!(scroll_handle.bounds(), viewport);
    assert!(scroll_handle.max_offset().y > px(0.));

    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-160.))),
        ..Default::default()
    });
    assert!(scroll_handle.offset().y < px(0.));
    assert_eq!(bounds(cx, "pages-results-scrollbar-layer"), viewport);

    cx.simulate_resize(size(px(260.), px(180.)));
    cx.run_until_parked();
    let resized_viewport = bounds(cx, "pages-results-viewport");
    assert_ne!(resized_viewport.size, viewport.size);
    assert_eq!(
        bounds(cx, "pages-results-scrollbar-layer"),
        resized_viewport
    );
    assert_eq!(scroll_handle.bounds(), resized_viewport);
}

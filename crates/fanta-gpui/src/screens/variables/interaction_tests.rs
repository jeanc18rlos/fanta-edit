use gpui::{
    Focusable as _, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, point,
    px, size,
};

use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;

fn fixture() -> VariablesViewData {
    VariablesViewData {
        document_name: "Test document".into(),
        collections: vec![
            VariablesCollection::new("collection", "Primitives", 2),
            VariablesCollection::new("tokens", "Tokens", 0),
        ],
        selected_collection_id: "collection".into(),
        groups: vec![
            VariablesGroup::new("all", "All variables", 2).aggregate(),
            VariablesGroup::new("brand", "Brand", 1),
        ],
        selected_group_id: "all".into(),
        modes: vec![VariablesMode::new("light", "Light")],
        variables: vec![
            VariableRow::new(
                "color",
                "Accent",
                "brand",
                VariableKind::Color,
                [VariableModeValue::new("light", "#336699").color("336699")],
            ),
            VariableRow::new(
                "radius",
                "Radius",
                "all",
                VariableKind::Number,
                [VariableModeValue::new("light", "8")],
            ),
        ],
    }
}

fn mount(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, VariablesScreen, VariablesAction> {
    mount_component(cx, |window, cx| {
        VariablesScreen::new("test-variables", fixture(), window, cx)
    })
}

fn double_click(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let position = cx.debug_bounds(selector).unwrap().center();
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
    cx.run_until_parked();
}

#[gpui::test]
fn sidebar_and_name_column_compress_on_narrow_pages(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);

    // Wide: both hold their reference design widths.
    cx.simulate_resize(size(px(1440.), px(900.)));
    cx.run_until_parked();
    cx.run_until_parked();
    let sidebar = cx
        .debug_bounds("variables-sidebar")
        .expect("sidebar should render");
    assert_eq!(f32::from(sidebar.size.width), SIDEBAR_WIDTH);
    let header = cx
        .debug_bounds("variables-name-header")
        .expect("name header should render");
    assert_eq!(f32::from(header.size.width), NAME_COLUMN_WIDTH);

    // Narrow: the sidebar floors, and the header and body name cells share
    // one compressed width so the column border stays aligned.
    cx.simulate_resize(size(px(560.), px(600.)));
    cx.run_until_parked();
    cx.run_until_parked();
    let sidebar = cx
        .debug_bounds("variables-sidebar")
        .expect("sidebar should render when narrow");
    assert_eq!(f32::from(sidebar.size.width), SIDEBAR_MIN_WIDTH);
    let header = cx
        .debug_bounds("variables-name-header")
        .expect("name header should render when narrow");
    let cell = cx
        .debug_bounds("variables-name-cell-radius")
        .expect("a body name cell should render when narrow");
    assert_eq!(f32::from(header.size.width), NAME_COLUMN_MIN_WIDTH);
    assert_eq!(
        f32::from(header.size.width),
        f32::from(cell.size.width),
        "header and body must share one name-column width"
    );
}

#[gpui::test]
fn the_page_reflows_to_its_published_minimum_size(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    cx.simulate_resize(size(
        px(VARIABLES_SCREEN_MIN_WIDTH),
        px(VARIABLES_SCREEN_MIN_HEIGHT),
    ));
    cx.run_until_parked();
    cx.run_until_parked();

    let sidebar = cx
        .debug_bounds("variables-sidebar")
        .expect("sidebar should render at the published minimum");
    assert_eq!(f32::from(sidebar.size.width), SIDEBAR_MIN_WIDTH);
    let header = cx
        .debug_bounds("variables-name-header")
        .expect("name header should render at the published minimum");
    assert_eq!(f32::from(header.size.width), NAME_COLUMN_MIN_WIDTH);
    let add_mode = cx
        .debug_bounds("variables-add-mode")
        .expect("the add-mode header should render at the published minimum");
    assert!(
        f32::from(add_mode.right()) <= VARIABLES_SCREEN_MIN_WIDTH + 1.,
        "the actions header must stay pinned inside the page"
    );
    let create_variable = cx
        .debug_bounds("variables-create-variable")
        .expect("the fixed create-variable row should render");
    assert!(f32::from(create_variable.bottom()) <= VARIABLES_SCREEN_MIN_HEIGHT + 1.);

    // The pinned header cell keeps its full activation contract at the floor.
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-add-mode",
        &actions,
        VariablesAction::AddModeRequested,
    );
}

#[gpui::test]
fn collection_rows_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-collection-collection",
        &actions,
        VariablesAction::CollectionSelected {
            collection_id: "collection".into(),
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-collection-tokens",
        &actions,
        VariablesAction::CollectionSelected {
            collection_id: "tokens".into(),
        },
    );
}

#[gpui::test]
fn collection_double_click_commits_a_typed_rename(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);
    double_click(cx, "variables-collection-collection");
    actions.borrow_mut().clear();

    let rename_input = cx.read(|app| {
        host.read(app)
            .component
            .read(app)
            .collection_name_input
            .clone()
    });
    cx.update(|window, app| {
        rename_input.update(app, |input, cx| input.set_value("Typography", window, cx));
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[VariablesAction::CollectionRenameRequested {
            collection_id: "collection".into(),
            name: "Typography".into(),
        }]
    );
}

#[gpui::test]
fn empty_and_no_match_states_offer_recovery_actions(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);
    let component = cx.read(|app| host.read(app).component.clone());
    component.update(cx, |page, cx| {
        let mut data = page.view_data.clone();
        data.variables.clear();
        page.set_view_data(data, cx);
    });
    cx.run_until_parked();
    let create = cx.debug_bounds("variables-empty-create").unwrap();
    let import = cx.debug_bounds("variables-empty-import").unwrap();
    assert_eq!(create.size.height, px(24.));
    assert_eq!(import.size.height, px(24.));
    actions.borrow_mut().clear();
    cx.simulate_click(import.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[VariablesAction::ImportVariablesRequested]
    );

    component.update(cx, |page, cx| {
        page.set_view_data(fixture(), cx);
    });
    cx.run_until_parked();
    let search_input = cx.read(|app| component.read(app).search_input.clone());
    cx.update(|window, app| {
        search_input.read(app).focus_handle(app).focus(window, app);
    });
    cx.simulate_keystrokes("z z z");
    cx.run_until_parked();
    let description = cx.debug_bounds("variables-empty-description").unwrap();
    let clear = cx.debug_bounds("variables-clear-empty-search").unwrap();
    assert_eq!(clear.size.height, px(24.));
    assert!(clear.top() >= description.bottom() + px(20.));
    cx.simulate_click(clear.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-value-color-light").is_some());
}

#[gpui::test]
fn empty_states_keep_content_inset_on_narrow_pages(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount(cx);
    cx.simulate_resize(size(
        px(VARIABLES_SCREEN_MIN_WIDTH),
        px(VARIABLES_SCREEN_MIN_HEIGHT),
    ));
    let component = cx.read(|app| host.read(app).component.clone());
    component.update(cx, |page, cx| {
        let mut data = page.view_data.clone();
        data.variables.clear();
        page.set_view_data(data, cx);
    });
    cx.run_until_parked();
    cx.run_until_parked();

    let state = cx.debug_bounds("variables-empty-state").unwrap();
    let title = cx.debug_bounds("variables-empty-title").unwrap();
    let description = cx.debug_bounds("variables-empty-description").unwrap();
    for content in [title, description] {
        assert!(content.left() >= state.left() + px(16.));
        assert!(content.right() <= state.right() - px(16.));
    }
}

#[gpui::test]
fn type_filter_menu_filters_without_mutating_host_data(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);
    let trigger = cx.debug_bounds("variables-search-options").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    let colors = cx.debug_bounds("variables-filter-kind-0").unwrap();
    cx.simulate_click(colors.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-value-color-light").is_none());
    assert!(cx.debug_bounds("variables-value-radius-light").is_some());
}

#[gpui::test]
fn clicking_outside_dismisses_the_type_filter_menu(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);
    let trigger = cx.debug_bounds("variables-search-options").unwrap();
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-filter-menu").is_some());

    let name_header = cx.debug_bounds("variables-name-header").unwrap();
    cx.simulate_click(name_header.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-filter-menu").is_none());
}

#[gpui::test]
fn group_rows_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-group-all",
        &actions,
        VariablesAction::GroupSelected {
            group_id: "all".into(),
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-group-brand",
        &actions,
        VariablesAction::GroupSelected {
            group_id: "brand".into(),
        },
    );
}

#[gpui::test]
fn creation_controls_emit_collection_variable_and_mode_intents(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-create-collection",
        &actions,
        VariablesAction::CreateCollectionRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-create-variable",
        &actions,
        VariablesAction::CreateVariableRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-add-mode",
        &actions,
        VariablesAction::AddModeRequested,
    );
}

#[gpui::test]
fn variable_cells_keep_a_uniform_fixed_height(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);

    let name = cx.debug_bounds("variables-name-cell-color").unwrap();
    let value = cx.debug_bounds("variables-value-color-light").unwrap();
    let action = cx
        .debug_bounds("variables-variable-settings-color")
        .unwrap();
    assert_eq!(name.size.height, value.size.height);
    assert_eq!(name.size.height, action.size.height);
}

#[gpui::test]
fn one_horizontal_offset_moves_headers_and_values_while_edge_columns_stay_fixed(
    cx: &mut TestAppContext,
) {
    let (host, _actions, cx) = mount(cx);
    cx.simulate_resize(size(px(560.), px(600.)));
    let component = cx.read(|app| host.read(app).component.clone());
    component.update(cx, |page, cx| {
        let mut data = page.view_data.clone();
        data.modes.extend([
            VariablesMode::new("dark", "Dark"),
            VariablesMode::new("contrast", "High contrast"),
        ]);
        page.set_view_data(data, cx);
    });
    cx.run_until_parked();
    cx.run_until_parked();

    let header_before = cx.debug_bounds("variables-mode-header-light").unwrap();
    let value_before = cx.debug_bounds("variables-value-color-light").unwrap();
    let name_before = cx.debug_bounds("variables-name-cell-color").unwrap();
    let action_before = cx
        .debug_bounds("variables-variable-settings-color")
        .unwrap();

    component.update(cx, |page, cx| {
        page.table_horizontal_scroll_handle
            .set_offset(point(px(-80.), px(0.)));
        cx.notify();
    });
    cx.run_until_parked();

    let header_after = cx.debug_bounds("variables-mode-header-light").unwrap();
    let value_after = cx.debug_bounds("variables-value-color-light").unwrap();
    let name_after = cx.debug_bounds("variables-name-cell-color").unwrap();
    let action_after = cx
        .debug_bounds("variables-variable-settings-color")
        .unwrap();
    assert_eq!(header_after.left() - header_before.left(), px(-80.));
    assert_eq!(value_after.left() - value_before.left(), px(-80.));
    assert_eq!(name_after.left(), name_before.left());
    assert_eq!(action_after.left(), action_before.left());
}

#[gpui::test]
fn vertical_scrolling_moves_all_body_columns_but_keeps_the_header_fixed(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount(cx);
    cx.simulate_resize(size(px(560.), px(160.)));
    cx.run_until_parked();
    cx.run_until_parked();

    let component = cx.read(|app| host.read(app).component.clone());
    let header_before = cx.debug_bounds("variables-name-header").unwrap();
    let mode_header_before = cx.debug_bounds("variables-mode-header-light").unwrap();
    let name_before = cx.debug_bounds("variables-name-cell-color").unwrap();
    let value_before = cx.debug_bounds("variables-value-color-light").unwrap();

    component.update(cx, |page, cx| {
        page.table_scroll_handle.set_offset(point(px(0.), px(-20.)));
        cx.notify();
    });
    cx.run_until_parked();

    let header_after = cx.debug_bounds("variables-name-header").unwrap();
    let mode_header_after = cx.debug_bounds("variables-mode-header-light").unwrap();
    let name_after = cx.debug_bounds("variables-name-cell-color").unwrap();
    let value_after = cx.debug_bounds("variables-value-color-light").unwrap();
    assert_eq!(header_after.top(), header_before.top());
    assert_eq!(mode_header_after.top(), mode_header_before.top());
    assert_eq!(name_after.top() - name_before.top(), px(-20.));
    assert_eq!(value_after.top() - value_before.top(), px(-20.));
}

#[gpui::test]
fn value_cells_request_edits_for_their_variable_and_mode(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-value-color-light",
        &actions,
        VariablesAction::ValueEditRequested {
            variable_id: "color".into(),
            mode_id: "light".into(),
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-value-radius-light",
        &actions,
        VariablesAction::ValueEditRequested {
            variable_id: "radius".into(),
            mode_id: "light".into(),
        },
    );
}

#[gpui::test]
fn variable_settings_controls_emit_their_intents(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-variable-settings-color",
        &actions,
        VariablesAction::VariableSettingsRequested {
            variable_id: "color".into(),
        },
    );
    assert!(cx.debug_bounds("variables-help").is_none());
}

#[gpui::test]
fn sidebar_toggle_hides_and_restores_the_sidebar(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);

    let toggle = cx
        .debug_bounds("variables-toggle-sidebar")
        .expect("expanded sidebar toggle should render");
    cx.simulate_click(toggle.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-sidebar").is_none());

    let restore = cx
        .debug_bounds("variables-toggle-sidebar-collapsed")
        .expect("collapsed toggle should render before the collection name");
    cx.simulate_click(restore.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("variables-sidebar").is_some());
}

#[gpui::test]
fn search_options_control_emits_its_intent(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-search-options",
        &actions,
        VariablesAction::SearchOptionsRequested,
    );
}

#[gpui::test]
fn search_typing_emits_query_changes_and_filters_rows(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    let search_input = cx.read(|app| host.read(app).component.read(app).search_input.clone());
    cx.update(|window, app| {
        search_input.read(app).focus_handle(app).focus(window, app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("r a");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[
            VariablesAction::SearchQueryChanged { query: "r".into() },
            VariablesAction::SearchQueryChanged { query: "ra".into() },
        ]
    );
    assert!(
        cx.debug_bounds("variables-value-radius-light").is_some(),
        "matching row should stay visible"
    );
    assert!(
        cx.debug_bounds("variables-value-color-light").is_none(),
        "non-matching row should be filtered out"
    );
}

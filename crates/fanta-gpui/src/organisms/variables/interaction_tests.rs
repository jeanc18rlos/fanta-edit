use gpui::{Focusable as _, TestAppContext, px, size};

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
) -> crate::test_support::Mounted<'_, VariablesPage, VariablesAction> {
    mount_component(cx, |window, cx| {
        VariablesPage::new("test-variables", fixture(), window, cx)
    })
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
        px(VARIABLES_PAGE_MIN_WIDTH),
        px(VARIABLES_PAGE_MIN_HEIGHT),
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
        .expect("the add-mode cell should render at the published minimum");
    assert!(
        f32::from(add_mode.right()) <= VARIABLES_PAGE_MIN_WIDTH + 1.,
        "the add-mode cell must stay pinned inside the page while the mode \
         headers clip (right edge {:?})",
        add_mode.right()
    );
    let create_variable = cx
        .debug_bounds("variables-create-variable")
        .expect("the create-variable row should render at the published minimum");
    assert!(
        f32::from(create_variable.bottom()) <= VARIABLES_PAGE_MIN_HEIGHT + 1.,
        "the create-variable row must stay inside the minimum-height page"
    );

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
fn share_and_help_controls_emit_their_intents(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "variables-share",
        &actions,
        VariablesAction::ShareRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "variables-help",
        &actions,
        VariablesAction::HelpRequested,
    );
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

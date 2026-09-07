use gpui::{TestAppContext, size};

use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;

fn mount(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, AssetsPanel, AssetsPanelAction> {
    mount_component(cx, |window, cx| {
        AssetsPanel::new(
            "test-assets",
            AssetsViewData::new([
                AssetsLibrary::new(
                    "local",
                    "Local components",
                    12,
                    AssetsLibraryKind::CurrentFile,
                    0x336699,
                ),
                AssetsLibrary::new(
                    "kit",
                    "Fanta UI Kit",
                    48,
                    AssetsLibraryKind::UiKit,
                    0x996633,
                ),
            ]),
            window,
            cx,
        )
    })
}

#[gpui::test]
fn rail_items_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    for (item, selector) in [
        (AssetsRailItem::File, "assets-rail-file"),
        (AssetsRailItem::Agents, "assets-rail-agents"),
        (AssetsRailItem::Assets, "assets-rail-assets"),
        (AssetsRailItem::Tools, "assets-rail-tools"),
        (AssetsRailItem::Variables, "assets-rail-variables"),
    ] {
        assert_pointer_and_keyboard_parity(
            cx,
            selector,
            &actions,
            AssetsPanelAction::RailItemSelected { item },
        );
    }
}

#[gpui::test]
fn library_rows_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "assets-library-local",
        &actions,
        AssetsPanelAction::LibrarySelected {
            library_id: "local".into(),
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "assets-library-kit",
        &actions,
        AssetsPanelAction::LibrarySelected {
            library_id: "kit".into(),
        },
    );
}

#[gpui::test]
fn filters_and_add_libraries_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "assets-filters",
        &actions,
        AssetsPanelAction::FiltersRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "assets-add-libraries",
        &actions,
        AssetsPanelAction::AddLibrariesRequested,
    );
}

#[gpui::test]
fn panel_stays_laid_out_and_operable_at_its_floor_size(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    cx.simulate_resize(size(
        px(ASSETS_PANEL_MIN_WIDTH),
        px(ASSETS_PANEL_MIN_HEIGHT),
    ));
    cx.run_until_parked();

    // Every control keeps its hit area inside the floored panel: the rail
    // holds its fixed chrome width and the content column compresses.
    for selector in [
        "assets-rail-file",
        "assets-rail-variables",
        "assets-filters",
        "assets-library-local",
        "assets-library-kit",
        "assets-add-libraries",
    ] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` should render at the floor size"));
        assert!(
            f32::from(bounds.right()) <= ASSETS_PANEL_MIN_WIDTH + 0.5,
            "`{selector}` must stay inside the {ASSETS_PANEL_MIN_WIDTH}px floor, got {:?}",
            bounds.right()
        );
    }

    // Activation still flows at the floor width.
    assert_pointer_and_keyboard_parity(
        cx,
        "assets-library-local",
        &actions,
        AssetsPanelAction::LibrarySelected {
            library_id: "local".into(),
        },
    );
}

#[gpui::test]
fn search_query_emits_intents_and_filters_library_rows(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    let search_input = cx.read(|app| host.read(app).component.read(app).search_input.clone());
    cx.update(|window, app| {
        search_input.update(app, |input, cx| input.focus(window, cx));
    });
    cx.simulate_keystrokes("k i t");
    cx.run_until_parked();

    assert_eq!(
        actions.borrow().as_slice(),
        &[
            AssetsPanelAction::SearchQueryChanged { query: "k".into() },
            AssetsPanelAction::SearchQueryChanged { query: "ki".into() },
            AssetsPanelAction::SearchQueryChanged {
                query: "kit".into()
            },
        ]
    );
    assert!(
        cx.debug_bounds("assets-library-kit").is_some(),
        "matching library row should stay visible"
    );
    assert!(
        cx.debug_bounds("assets-library-local").is_none(),
        "non-matching library row should be filtered out"
    );
}

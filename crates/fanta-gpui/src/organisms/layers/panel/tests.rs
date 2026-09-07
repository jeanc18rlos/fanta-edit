use std::collections::HashSet;

use gpui::{
    Bounds, Entity, Focusable as _, KeyBinding, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, Pixels, Point, SharedString, TestAppContext, VisualTestContext, actions, px,
    size,
};
use gpui_component::Root;

use crate::test_support::{
    Mounted, ProbeHost, assert_pointer_and_keyboard_parity, mount_component,
};

use super::{events::layer_drop_position, menu::menu_sections_for, *};

actions!(layers_input_regression, [CompetingDeleteSelection]);

type Host = Entity<ProbeHost<LayersPanel, LayersPanelAction>>;

fn fixture_nodes() -> Vec<LayersPanelItem> {
    use LayersPanelNodeKind as Kind;

    vec![
        LayersPanelItem::new("frame", "Checkout", Kind::Frame).children(vec![
            LayersPanelItem::new("group", "Header", Kind::Group)
                .locked(true)
                .children(vec![
                    LayersPanelItem::new("text", "Title", Kind::Text),
                    LayersPanelItem::new("image", "Logo", Kind::Image).visible(false),
                ]),
            LayersPanelItem::new("component", "Primary button", Kind::Component).children(vec![
                LayersPanelItem::new("instance", "Icon instance", Kind::Instance),
            ]),
            LayersPanelItem::new("rectangle", "Background", Kind::Rectangle),
        ]),
        LayersPanelItem::new("section", "Variants", Kind::Section),
        LayersPanelItem::new("slice", "Export slice", Kind::Slice),
    ]
}

fn setup(cx: &mut TestAppContext) -> Mounted<'_, LayersPanel, LayersPanelAction> {
    mount_component(cx, |window, cx| {
        let mut panel = LayersPanel::new("test-layers", fixture_nodes(), window, cx);
        panel.set_selected_node_ids(vec!["text".into()], cx);
        panel.set_expanded_node_ids(vec!["frame".into(), "group".into(), "component".into()], cx);
        panel
    })
}

fn panel(host: &Host, cx: &VisualTestContext) -> Entity<LayersPanel> {
    cx.read(|app| host.read(app).component.clone())
}

fn read_panel<R>(
    panel: &Entity<LayersPanel>,
    cx: &VisualTestContext,
    read: impl FnOnce(&LayersPanel) -> R,
) -> R {
    cx.read(|app| read(panel.read(app)))
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing rendered selector: {selector}"))
}

fn click(cx: &mut VisualTestContext, selector: &'static str, modifiers: Modifiers) {
    let position = bounds(cx, selector).center();
    cx.simulate_click(position, modifiers);
    cx.run_until_parked();
}

fn secondary_click_at(cx: &mut VisualTestContext, position: Point<Pixels>) {
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

fn secondary_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
    secondary_click_at(cx, position);
}

fn double_click(cx: &mut VisualTestContext, selector: &'static str) {
    let position = bounds(cx, selector).center();
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
    cx.run_until_parked();
}

fn focus_panel(panel: &Entity<LayersPanel>, cx: &mut VisualTestContext) {
    cx.update(|window, app| {
        panel.focus_handle(app).focus(window, app);
    });
    cx.run_until_parked();
}

fn focused_row(panel: &Entity<LayersPanel>, cx: &mut VisualTestContext) -> Option<SharedString> {
    cx.update(|window, app| {
        panel
            .read(app)
            .row_focus_handles
            .iter()
            .find_map(|(node_id, handle)| handle.is_focused(window).then(|| node_id.clone()))
    })
}

fn actions_for(kind: LayersPanelNodeKind) -> HashSet<LayersPanelContextAction> {
    menu_sections_for(kind)
        .into_iter()
        .flatten()
        .map(|entry| entry.action)
        .collect()
}

#[test]
fn visible_tree_respects_expansion_and_depth() {
    let arena = LayerArena::from_items(&fixture_nodes());
    let expanded: HashSet<SharedString> = ["frame".into(), "group".into()].into_iter().collect();
    let visible = arena.visible_indices(&expanded);

    let identity_and_depth = visible
        .iter()
        .map(|index| {
            let node = arena.get(*index).expect("visible index is in the arena");
            (
                node.id.to_string(),
                node.depth,
                arena
                    .ancestor_ids(*index)
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let expected = vec![
        ("frame", 0, vec![]),
        ("group", 1, vec!["frame"]),
        ("text", 2, vec!["frame", "group"]),
        ("image", 2, vec!["frame", "group"]),
        ("component", 1, vec!["frame"]),
        ("rectangle", 1, vec!["frame"]),
        ("section", 0, vec![]),
        ("slice", 0, vec![]),
    ]
    .into_iter()
    .map(|(id, depth, ancestors): (&str, usize, Vec<&str>)| {
        (
            id.to_owned(),
            depth,
            ancestors.into_iter().map(str::to_owned).collect::<Vec<_>>(),
        )
    })
    .collect::<Vec<_>>();
    assert_eq!(identity_and_depth, expected);
}

#[test]
fn arena_answers_subtree_membership_from_preorder_indices() {
    let arena = LayerArena::from_items(&fixture_nodes());
    let index = |id: &str| arena.index_of(&SharedString::from(id.to_owned())).unwrap();

    assert!(arena.is_within(index("text"), index("frame")));
    assert!(arena.is_within(index("text"), index("group")));
    assert!(arena.is_within(index("group"), index("group")));
    assert!(!arena.is_within(index("group"), index("text")));
    assert!(!arena.is_within(index("section"), index("frame")));
    assert!(!arena.is_within(index("rectangle"), index("group")));
    assert!(arena.has_children(index("frame")));
    assert!(!arena.has_children(index("slice")));
    assert_eq!(arena.len(), 9);
}

#[test]
fn every_supported_node_kind_has_safe_common_context_actions() {
    use LayersPanelNodeKind as Kind;
    let kinds = [
        Kind::Frame,
        Kind::Group,
        Kind::Section,
        Kind::Component,
        Kind::ComponentSet,
        Kind::Instance,
        Kind::Text,
        Kind::Image,
        Kind::Video,
        Kind::Rectangle,
        Kind::Ellipse,
        Kind::Polygon,
        Kind::Star,
        Kind::Line,
        Kind::Arrow,
        Kind::Vector,
        Kind::BooleanOperation,
        Kind::Slice,
        Kind::Mask,
        Kind::Pen,
        Kind::Pencil,
        Kind::Other,
    ];

    for kind in kinds {
        let actions = actions_for(kind);
        assert!(actions.contains(&LayersPanelContextAction::Copy));
        assert!(actions.contains(&LayersPanelContextAction::Rename));
        assert!(actions.contains(&LayersPanelContextAction::ShowHide));
        assert!(actions.contains(&LayersPanelContextAction::LockUnlock));
        assert!(actions.contains(&LayersPanelContextAction::Plugins));
    }
}

#[test]
fn context_menu_capabilities_vary_by_node_type() {
    use LayersPanelContextAction as Action;
    use LayersPanelNodeKind as Kind;

    let text = actions_for(Kind::Text);
    assert!(text.contains(&Action::EditText));
    assert!(text.contains(&Action::OutlineStroke));
    assert!(!text.contains(&Action::CropImage));

    let image = actions_for(Kind::Image);
    assert!(image.contains(&Action::CropImage));
    assert!(image.contains(&Action::ReplaceMedia));
    assert!(!image.contains(&Action::EditText));

    let instance = actions_for(Kind::Instance);
    assert!(instance.contains(&Action::GoToMainComponent));
    assert!(instance.contains(&Action::DetachInstance));
    assert!(!instance.contains(&Action::CreateComponent));

    let frame = actions_for(Kind::Frame);
    assert!(frame.contains(&Action::ConvertToSection));
    assert!(frame.contains(&Action::RemoveFrame));
    assert!(frame.contains(&Action::MoreLayoutOptions));

    let section = actions_for(Kind::Section);
    assert!(section.contains(&Action::ConvertToFrame));
    assert!(!section.contains(&Action::FlipHorizontal));
    assert!(!section.contains(&Action::CreateComponent));
}

#[gpui::test]
fn tree_renders_nested_types_selection_and_host_states(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    assert!(bounds(cx, "layers-row-frame").size.height > px(0.));
    assert!(bounds(cx, "layers-row-text").size.height > px(0.));
    assert!(bounds(cx, "layers-row-instance").size.height > px(0.));
    assert!(bounds(cx, "layers-lock-group").size.width > px(0.));
    assert!(bounds(cx, "layers-visibility-image").size.width > px(0.));
    assert_eq!(
        read_panel(&panel, cx, |panel| panel
            .selected_node_ids
            .iter()
            .cloned()
            .collect::<Vec<_>>()),
        vec![SharedString::from("text")]
    );
}

#[gpui::test]
fn hovering_a_nested_row_highlights_its_nearest_visible_group(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    let position = bounds(cx, "layers-row-text").center();
    cx.simulate_mouse_move(position, None, Modifiers::none());
    cx.run_until_parked();

    assert_eq!(
        read_panel(&panel, cx, |panel| panel.hovered_group_id()),
        Some("group".into())
    );
}

#[gpui::test]
fn selecting_a_group_highlights_its_visible_subtree(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);
    panel.update(cx, |panel, cx| {
        panel.set_selected_node_ids(vec!["group".into()], cx);
    });
    cx.run_until_parked();

    let highlighted = read_panel(&panel, cx, |panel| panel.rows_within_selected_groups());
    assert_eq!(
        highlighted,
        vec![
            SharedString::from("group"),
            SharedString::from("text"),
            SharedString::from("image"),
        ]
    );
}

#[gpui::test]
fn layers_header_collapses_and_expands_the_whole_panel(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    click(cx, "layers-header", Modifiers::none());
    assert!(!read_panel(&panel, cx, |panel| panel.panel_expanded));
    assert_eq!(bounds(cx, "layers-panel").size.height, px(HEADER_HEIGHT));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::PanelExpansionChanged { expanded: false })
    );

    click(cx, "layers-header", Modifiers::none());
    assert!(read_panel(&panel, cx, |panel| panel.panel_expanded));
    assert!(bounds(cx, "layers-tree-viewport").size.height > px(0.));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::PanelExpansionChanged { expanded: true })
    );
}

#[gpui::test]
fn row_clicks_emit_replace_toggle_and_range_selection(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);

    click(cx, "layers-row-rectangle", Modifiers::none());

    let mut toggle = Modifiers::none();
    #[cfg(target_os = "macos")]
    {
        toggle.platform = true;
    }
    #[cfg(not(target_os = "macos"))]
    {
        toggle.control = true;
    }
    click(cx, "layers-row-text", toggle);

    let mut range = Modifiers::none();
    range.shift = true;
    click(cx, "layers-row-image", range);

    assert_eq!(
        actions.borrow().as_slice(),
        &[
            LayersPanelAction::SelectRequested {
                node_id: "rectangle".into(),
                mode: LayersPanelSelectionMode::Replace,
            },
            LayersPanelAction::SelectRequested {
                node_id: "text".into(),
                mode: LayersPanelSelectionMode::Toggle,
            },
            LayersPanelAction::SelectRequested {
                node_id: "image".into(),
                mode: LayersPanelSelectionMode::Range,
            },
        ]
    );
}

#[gpui::test]
fn row_and_collapse_all_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "layers-row-rectangle",
        &actions,
        LayersPanelAction::SelectRequested {
            node_id: "rectangle".into(),
            mode: LayersPanelSelectionMode::Replace,
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "layers-collapse-all",
        &actions,
        LayersPanelAction::CollapseAllRequested,
    );
}

#[gpui::test]
fn dragging_a_row_emits_a_typed_move_intent(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);
    let source = bounds(cx, "layers-row-rectangle").center();
    let target_bounds = bounds(cx, "layers-row-section");
    let target = gpui::point(target_bounds.center().x, target_bounds.top() + px(2.));

    cx.simulate_mouse_move(source, None, Modifiers::none());
    cx.simulate_mouse_down(source, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(
        source + gpui::point(px(8.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::MoveRequested {
            node_id: "rectangle".into(),
            target_node_id: "section".into(),
            position: LayersPanelDropPosition::Before,
        })
    );
}

#[gpui::test]
fn expansion_and_collapse_all_update_presentation_and_emit_intents(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    click(cx, "layers-expand-group", Modifiers::none());
    assert!(!read_panel(&panel, cx, |panel| panel
        .expanded_node_ids
        .contains(&SharedString::from("group"))));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::ExpansionChanged {
            node_id: "group".into(),
            expanded: false,
        })
    );

    click(cx, "layers-collapse-all", Modifiers::none());
    assert!(read_panel(&panel, cx, |panel| panel
        .expanded_node_ids
        .is_empty()));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::CollapseAllRequested)
    );
}

#[gpui::test]
fn arrow_keys_move_focus_between_visible_rows(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("frame".into()));

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));

    cx.simulate_keystrokes("down");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("text".into()));

    cx.simulate_keystrokes("up");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));

    cx.simulate_keystrokes("up up");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("frame".into()));
}

#[gpui::test]
fn left_and_right_arrows_collapse_expand_and_traverse_the_hierarchy(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab down down");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("text".into()));

    // Left on a leaf focuses its parent without emitting an expansion intent.
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));
    assert!(actions.borrow().is_empty());

    // Left on an expanded container collapses it and keeps focus.
    cx.simulate_keystrokes("left");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::ExpansionChanged {
            node_id: "group".into(),
            expanded: false,
        })
    );

    // Right on a collapsed container expands it via the same intent.
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::ExpansionChanged {
            node_id: "group".into(),
            expanded: true,
        })
    );

    // Right on an expanded container focuses its first child.
    cx.simulate_keystrokes("right");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("text".into()));
}

#[gpui::test]
fn rows_are_single_tab_stops_with_satellite_controls_skipped(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("frame".into()));

    // The frame row hosts expand, lock, and visibility satellites; Tab must
    // skip all of them and land on the next row.
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("group".into()));

    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("text".into()));
}

#[gpui::test]
fn row_scoped_shortcuts_toggle_visibility_and_lock(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);

    click(cx, "layers-row-image", Modifiers::none());
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-secondary-h");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::VisibilityChanged {
            node_id: "image".into(),
            visible: true,
        }]
    );

    click(cx, "layers-row-group", Modifiers::none());
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-secondary-l");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::LockChanged {
            node_id: "group".into(),
            locked: false,
        }]
    );
}

#[gpui::test]
fn row_controls_emit_visibility_and_lock_changes_without_selecting(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);

    click(cx, "layers-lock-group", Modifiers::none());
    click(cx, "layers-visibility-image", Modifiers::none());

    assert_eq!(
        actions.borrow().as_slice(),
        &[
            LayersPanelAction::LockChanged {
                node_id: "group".into(),
                locked: false,
            },
            LayersPanelAction::VisibilityChanged {
                node_id: "image".into(),
                visible: true,
            },
        ]
    );
}

#[gpui::test]
fn contextual_menu_renders_node_specific_actions_and_emits_typed_intent(cx: &mut TestAppContext) {
    let (_host, actions, cx) = setup(cx);

    secondary_click(cx, "layers-row-instance");
    assert!(bounds(cx, "layers-context-menu").size.width > px(0.));
    assert!(bounds(cx, "layers-menu-detach-instance").size.height > px(0.));
    assert!(cx.debug_bounds("layers-menu-create-component").is_none());

    actions.borrow_mut().clear();
    click(cx, "layers-menu-detach-instance", Modifiers::none());
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::ContextActionRequested {
            node_id: "instance".into(),
            action: LayersPanelContextAction::DetachInstance,
        }]
    );
}

#[gpui::test]
fn menu_visibility_and_lock_items_use_dedicated_intents(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    secondary_click(cx, "layers-row-image");
    actions.borrow_mut().clear();
    cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_menu_action(LayersPanelContextAction::ShowHide, window, cx);
        });
    });
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::VisibilityChanged {
            node_id: "image".into(),
            visible: true,
        }]
    );

    secondary_click(cx, "layers-row-group");
    actions.borrow_mut().clear();
    cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_menu_action(LayersPanelContextAction::LockUnlock, window, cx);
        });
    });
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::LockChanged {
            node_id: "group".into(),
            locked: false,
        }]
    );
}

#[gpui::test]
fn long_context_menu_repositions_to_stay_inside_the_panel_height(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = setup(cx);

    secondary_click(cx, "layers-row-slice");
    let panel = bounds(cx, "layers-panel");
    let menu = bounds(cx, "layers-context-menu");

    assert!(menu.top() >= panel.top());
    assert!(menu.bottom() <= panel.bottom());
    assert_eq!(bounds(cx, "layers-menu-copy").size.height, px(28.));
    assert!(bounds(cx, "layers-context-menu-scrollbar").size.height > px(0.));
}

#[gpui::test]
fn context_menu_clamps_inside_the_panel_right_edge(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = setup(cx);

    let row = bounds(cx, "layers-row-section");
    secondary_click_at(cx, gpui::point(row.right() - px(6.), row.center().y));

    let panel = bounds(cx, "layers-panel");
    let menu = bounds(cx, "layers-context-menu");
    assert!(menu.size.width > px(0.));
    assert!(menu.left() >= panel.left());
    assert!(menu.right() <= panel.right());
}

#[gpui::test]
fn long_titles_truncate_instead_of_forcing_horizontal_scroll(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    panel.update(cx, |panel, cx| {
        panel.set_nodes(
            vec![LayersPanelItem::new(
                "long",
                "An extremely long layer title that previously forced the tree wider ".repeat(8),
                LayersPanelNodeKind::Text,
            )],
            cx,
        );
    });
    cx.run_until_parked();

    let viewport = bounds(cx, "layers-tree-viewport");
    let row = bounds(cx, "layers-row-long");
    assert!(row.size.width <= viewport.size.width);
}

#[gpui::test]
fn set_nodes_prunes_stale_row_caches(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    assert!(read_panel(&panel, cx, |panel| {
        panel.node_row_bounds.contains_key("slice") && panel.row_focus_handles.contains_key("slice")
    }));

    panel.update(cx, |panel, cx| {
        panel.set_nodes(
            vec![LayersPanelItem::new(
                "frame",
                "Checkout",
                LayersPanelNodeKind::Frame,
            )],
            cx,
        );
    });

    assert!(read_panel(&panel, cx, |panel| {
        !panel.node_row_bounds.contains_key("slice")
            && !panel.row_focus_handles.contains_key("slice")
    }));
    assert!(read_panel(&panel, cx, |panel| {
        panel.node_row_bounds.contains_key("frame")
    }));
}

#[gpui::test]
fn double_click_rename_commits_only_a_changed_nonempty_title(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    double_click(cx, "layers-row-text");
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_some()));
    assert!(bounds(cx, "layers-row-editor").size.height > px(0.));

    cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.rename_input.update(cx, |input, cx| {
                input.set_value("Renamed title", window, cx);
            });
        });
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::RenameRequested {
            node_id: "text".into(),
            title: "Renamed title".into(),
        })
    );
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert_eq!(
        cx.read(|app| panel.read(app).rename_input.read(app).value()),
        "Renamed title",
        "the consumed Enter keystroke must not type into the rename draft"
    );
}

#[gpui::test]
fn rename_draft_editing_survives_a_hostile_global_keymap(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    double_click(cx, "layers-row-text");
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_some()));

    cx.update(|_, app| {
        app.clear_key_bindings();
        app.bind_keys([KeyBinding::new("backspace", CompetingDeleteSelection, None)]);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("secondary-a backspace");
    assert!(
        cx.read(|app| panel.read(app).rename_input.read(app).value().is_empty()),
        "select-all and deletion must reach the focused rename input"
    );

    cx.simulate_keystrokes("a b backspace");
    assert_eq!(
        cx.read(|app| panel.read(app).rename_input.read(app).value()),
        "a",
        "the focused rename input must keep its editing keys under a hostile keymap"
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.editing.is_none()));
    assert!(
        actions.borrow().is_empty(),
        "cancelling a rename draft must not emit a host intent"
    );
    cx.update(|window, app| {
        assert!(
            panel.read(app).focus_handle.is_focused(window),
            "escape must cancel the rename and restore panel focus"
        );
    });
}

struct NarrowRailHost {
    panel: Entity<LayersPanel>,
}

impl Render for NarrowRailHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .justify_end()
            .child(div().w(px(160.)).h_full().child(self.panel.clone()))
    }
}

#[gpui::test]
fn context_menu_clamps_against_the_window_at_narrow_rail_widths(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::init(cx);
    });
    let (_, cx) = cx.add_window_view(|window, cx| {
        let panel = cx.new(|cx| LayersPanel::new("narrow-layers", fixture_nodes(), window, cx));
        let host = cx.new(|_| NarrowRailHost { panel });
        Root::new(host, window, cx)
    });

    let row = bounds(cx, "layers-row-section");
    secondary_click_at(cx, gpui::point(row.right() - px(6.), row.center().y));

    let viewport = cx.update(|window, _| window.viewport_size());
    let menu = bounds(cx, "layers-context-menu");
    assert!(menu.size.width > px(0.));
    assert!(
        menu.left() >= px(0.) && menu.right() <= viewport.width,
        "the pointer-anchored menu must clamp against the window, not the rail"
    );
    assert!(menu.top() >= px(0.) && menu.bottom() <= viewport.height);
    assert!(
        menu.right() > row.right() - px(6.),
        "clamping must keep the menu under the pointer instead of pinning it to the rail edge"
    );
}

#[gpui::test]
fn panel_stays_laid_out_and_operable_at_its_floor_size(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    // A root row with an unbounded host title must truncate ahead of its
    // lock/visibility satellites instead of widening the floored panel.
    cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            let mut nodes = fixture_nodes();
            nodes.push(LayersPanelItem::new(
                "wide",
                "An intentionally very long layer title that cannot fit the narrow rail",
                LayersPanelNodeKind::Text,
            ));
            panel.set_nodes(nodes, cx);
        });
    });
    cx.simulate_resize(size(
        px(LAYERS_PANEL_MIN_WIDTH),
        px(LAYERS_PANEL_MIN_HEIGHT),
    ));
    cx.run_until_parked();

    for selector in [
        "layers-header",
        "layers-collapse-all",
        "layers-row-text",
        "layers-row-wide",
        "layers-lock-group",
    ] {
        let control = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` should render at the floor size"));
        assert!(
            f32::from(control.right()) <= LAYERS_PANEL_MIN_WIDTH + 0.5,
            "`{selector}` must stay inside the {LAYERS_PANEL_MIN_WIDTH}px floor, got {:?}",
            control.right()
        );
    }

    // Activation still flows at the floor width.
    actions.borrow_mut().clear();
    click(cx, "layers-row-wide", Modifiers::none());
    assert_eq!(
        actions.borrow().as_slice(),
        &[LayersPanelAction::SelectRequested {
            node_id: "wide".into(),
            mode: LayersPanelSelectionMode::Replace,
        }]
    );
}

#[gpui::test]
fn keyboard_opens_and_escape_closes_a_row_menu_with_focus_restored(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab ctrl-enter");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.menu.is_some()));
    assert!(bounds(cx, "layers-context-menu").size.width > px(0.));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(read_panel(&panel, cx, |panel| panel.menu.is_none()));
}

fn wide_root_layers(count: usize) -> Vec<LayersPanelItem> {
    (0..count)
        .map(|index| {
            LayersPanelItem::new(
                format!("leaf-{index}"),
                format!("Leaf {index}"),
                LayersPanelNodeKind::Rectangle,
            )
        })
        .collect()
}

#[gpui::test]
fn the_tree_is_virtualized_and_reveal_scrolls_a_far_row_into_view(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount_component(cx, |window, cx| {
        LayersPanel::new("virtual-layers", wide_root_layers(5_000), window, cx)
    });
    let panel = panel(&host, cx);

    // Only the viewport's worth of rows is built: the first row exists, a
    // row thousands deep does not, and the panel holds focus handles for the
    // rendered rows only.
    assert!(bounds(cx, "layers-row-leaf-0").size.height > px(0.));
    assert!(cx.debug_bounds("layers-row-leaf-4000").is_none());
    let rendered = read_panel(&panel, cx, |panel| panel.row_focus_handles.len());
    assert!(
        rendered < 200,
        "a 5,000-row tree must build only the visible rows, built {rendered}"
    );

    let revealed = panel.update(cx, |panel, cx| {
        panel.reveal_node(&SharedString::from("leaf-4000"), cx)
    });
    assert!(revealed);
    cx.run_until_parked();
    let far = bounds(cx, "layers-row-leaf-4000");
    let viewport = bounds(cx, "layers-tree-viewport");
    assert!(
        far.top() >= viewport.top() && far.bottom() <= viewport.bottom(),
        "the revealed row must sit inside the viewport: {far:?} in {viewport:?}"
    );
    assert!(cx.debug_bounds("layers-row-leaf-0").is_none());
}

#[gpui::test]
fn reveal_refuses_rows_hidden_under_a_collapsed_ancestor(cx: &mut TestAppContext) {
    let (host, _actions, cx) = setup(cx);
    let panel = panel(&host, cx);

    panel.update(cx, |panel, cx| {
        panel.set_expanded_node_ids(vec!["frame".into()], cx);
    });
    cx.run_until_parked();
    let revealed = panel.update(cx, |panel, cx| {
        panel.reveal_node(&SharedString::from("text"), cx)
    });
    assert!(!revealed, "a row under the collapsed `group` is not shown");
    assert!(cx.debug_bounds("layers-row-text").is_none());

    panel.update(cx, |panel, cx| {
        panel.set_expanded_node_ids(vec!["frame".into(), "group".into()], cx);
    });
    cx.run_until_parked();
    let revealed = panel.update(cx, |panel, cx| {
        panel.reveal_node(&SharedString::from("text"), cx)
    });
    assert!(revealed);
    cx.run_until_parked();
    assert!(bounds(cx, "layers-row-text").size.height > px(0.));
    assert_eq!(
        read_panel(&panel, cx, |panel| panel.visible_row_ids()),
        vec![
            SharedString::from("frame"),
            SharedString::from("group"),
            SharedString::from("text"),
            SharedString::from("image"),
            SharedString::from("component"),
            SharedString::from("rectangle"),
            SharedString::from("section"),
            SharedString::from("slice"),
        ]
    );
}

#[gpui::test]
fn arrow_keys_scroll_focus_to_rows_outside_the_viewport(cx: &mut TestAppContext) {
    let (host, _actions, cx) = mount_component(cx, |window, cx| {
        LayersPanel::new("virtual-focus-layers", wide_root_layers(400), window, cx)
    });
    let panel = panel(&host, cx);

    focus_panel(&panel, cx);
    cx.simulate_keystrokes("tab tab tab");
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("leaf-0".into()));

    // Walk well past the first viewport: each Down must land on the next
    // row even though it had no element when the key was pressed.
    for _ in 0..60 {
        cx.simulate_keystrokes("down");
    }
    cx.run_until_parked();
    assert_eq!(focused_row(&panel, cx), Some("leaf-60".into()));
    let row = bounds(cx, "layers-row-leaf-60");
    let viewport = bounds(cx, "layers-tree-viewport");
    assert!(row.top() >= viewport.top() && row.bottom() <= viewport.bottom());
}

#[gpui::test]
fn a_refusing_drop_validator_suppresses_the_move_intent(cx: &mut TestAppContext) {
    let (host, actions, cx) = setup(cx);
    let panel = panel(&host, cx);
    panel.update(cx, |panel, cx| {
        panel.set_drop_validator(
            Some(Rc::new(|dragged, target, _, _| {
                !(dragged.as_ref() == "rectangle" && target.as_ref() == "section")
            })),
            cx,
        );
    });
    cx.run_until_parked();

    let drag_to = |cx: &mut VisualTestContext, source: &'static str, target: &'static str| {
        let source = bounds(cx, source).center();
        let target_bounds = bounds(cx, target);
        let target = gpui::point(target_bounds.center().x, target_bounds.top() + px(2.));
        cx.simulate_mouse_move(source, None, Modifiers::none());
        cx.simulate_mouse_down(source, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_move(
            source + gpui::point(px(8.), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        cx.simulate_mouse_move(target, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
        cx.run_until_parked();
    };

    actions.borrow_mut().clear();
    drag_to(cx, "layers-row-rectangle", "layers-row-section");
    assert!(
        !actions
            .borrow()
            .iter()
            .any(|action| matches!(action, LayersPanelAction::MoveRequested { .. })),
        "a drop the host refuses must not emit MoveRequested: {:?}",
        actions.borrow()
    );

    actions.borrow_mut().clear();
    drag_to(cx, "layers-row-rectangle", "layers-row-slice");
    assert_eq!(
        actions.borrow().last(),
        Some(&LayersPanelAction::MoveRequested {
            node_id: "rectangle".into(),
            target_node_id: "slice".into(),
            position: LayersPanelDropPosition::Before,
        })
    );
}

#[test]
fn drop_position_never_nests_inside_an_instance() {
    let bounds = Bounds::new(gpui::point(px(0.), px(0.)), size(px(200.), px(28.)));
    let center = bounds.center();
    assert_eq!(
        layer_drop_position(LayersPanelNodeKind::Frame, bounds, center),
        LayersPanelDropPosition::Inside
    );
    assert_ne!(
        layer_drop_position(LayersPanelNodeKind::Instance, bounds, center),
        LayersPanelDropPosition::Inside
    );
    assert_ne!(
        layer_drop_position(LayersPanelNodeKind::Rectangle, bounds, center),
        LayersPanelDropPosition::Inside
    );
}

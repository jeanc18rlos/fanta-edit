use super::super::{
    DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD, DESIGN_SCRUB_HALF_SPEED_Y_THRESHOLD,
    DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD, DesignArcData, DesignComponentResetState,
    DesignLayoutGrid, DesignPaintCollectionEditMode, DesignPanelNodeCapabilities,
    DesignPanelPropertyBinding, DesignRepeatModifier, DesignSectionDevStatus,
    DesignShaderDefinition, DesignShaderEffect, DesignShaderProperty,
    DesignShaderPropertyDefinition, DesignStroke,
};
use super::*;
use std::{cell::RefCell, rc::Rc};

use gpui::{
    Entity, IntoElement, Modifiers, Render, Subscription, TestAppContext, VisualTestContext,
    Window, div, px,
};
use gpui_component::Root;

#[test]
fn style_browser_search_matches_names_previews_and_library_sources() {
    assert!(DesignPanel::style_browser_row_matches(
        "raised",
        "Raised surface",
        "Drop shadow",
        None,
    ));
    assert!(DesignPanel::style_browser_row_matches(
        "gradient",
        "Brand",
        "2 ordered paints · Linear gradient, Solid",
        None,
    ));
    assert!(DesignPanel::style_browser_row_matches(
        "marketing",
        "Hero",
        "Solid",
        Some("Marketing library"),
    ));
    assert!(!DesignPanel::style_browser_row_matches(
        "missing",
        "Hero",
        "Solid",
        Some("Marketing library"),
    ));
    assert!(StyleBrowserSourceFilter::ThisPage.includes_page());
    assert!(!StyleBrowserSourceFilter::ThisPage.includes_library("marketing"));
    assert!(StyleBrowserSourceFilter::Libraries.includes_library("marketing"));
    assert!(!StyleBrowserSourceFilter::Libraries.includes_page());
    let marketing = StyleBrowserSourceFilter::library("marketing", "Marketing library");
    assert!(marketing.includes_library("marketing"));
    assert!(!marketing.includes_library("product"));
    assert!(!marketing.includes_page());
    assert_eq!(marketing.label().as_ref(), "Marketing library");
    let current_libraries = [
        ("marketing".into(), "Marketing system".into()),
        ("product".into(), "Product system".into()),
    ];
    assert_eq!(
        marketing.normalized_for_libraries(
            current_libraries
                .iter()
                .map(|(library_id, library_name)| (library_id, library_name)),
        ),
        StyleBrowserSourceFilter::library("marketing", "Marketing system"),
        "a host rename updates the retained source label by stable ID"
    );
    assert_eq!(
        marketing.normalized_for_libraries(
            [("product".into(), "Product system".into())]
                .iter()
                .map(|(library_id, library_name)| (library_id, library_name)),
        ),
        StyleBrowserSourceFilter::All,
        "a removed host library cannot leave a dead selected filter"
    );
}

#[gpui::test]
fn style_browser_controls_are_transient_and_leave_host_catalogs_untouched(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("styled", "Styled", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let catalog = DesignPaintStyleViewData::new(
        [super::super::DesignPaintStyle::new(
            "surface",
            "Surface",
            [DesignPaint::solid(DesignColor::WHITE)],
        )],
        [super::super::DesignPaintStyleLibrary::new(
            "marketing",
            "Marketing library",
            [super::super::DesignPaintStyle::new(
                "hero",
                "Hero gradient",
                [DesignPaint::solid(DesignColor::BLACK)],
            )],
        )],
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_paint_style_view_data(catalog.clone(), cx);
        panel.set_style_browser_source_filter(StyleBrowserSourceFilter::Libraries, cx);
        panel.set_style_browser_view_mode(StyleBrowserViewMode::Grid, cx);
        assert_eq!(
            panel.style_browser_source_filter,
            StyleBrowserSourceFilter::Libraries
        );
        assert_eq!(panel.style_browser_view_mode, StyleBrowserViewMode::Grid);
        assert_eq!(panel.paint_style_view_data(), &catalog);
    });

    let search = visual_cx.read(|app| panel.read(app).style_browser_search.clone());
    visual_cx.update(|window, app| {
        search.update(app, |search, cx| {
            search.set_value("  MARKETING  ", window, cx);
        });
    });
    visual_cx.run_until_parked();

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(panel.normalized_style_browser_query(cx), "marketing");
        assert_eq!(panel.paint_style_view_data(), &catalog);
        drop(panel.render_paint_style_button(DesignPanelCollection::Fill, cx));
    });
}

#[gpui::test]
fn active_specific_library_filter_tracks_host_rename_and_removal(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("styled", "Styled", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let catalog = |library_name: &str| {
        DesignPaintStyleViewData::new(
            [],
            [super::super::DesignPaintStyleLibrary::new(
                "marketing",
                library_name.to_owned(),
                [super::super::DesignPaintStyle::new(
                    "hero",
                    "Hero",
                    [DesignPaint::solid(DesignColor::BLACK)],
                )],
            )],
        )
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_paint_style_view_data(catalog("Marketing library"), cx);
        panel.set_style_browser_source_filter(
            StyleBrowserSourceFilter::library("marketing", "Marketing library"),
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_paint_style_browser(DesignPanelCollection::Fill, window, cx);
        });
    });
    panel.update(visual_cx, |panel, cx| {
        panel.set_paint_style_view_data(catalog("Marketing system"), cx);
        assert_eq!(
            panel.style_browser_source_filter,
            StyleBrowserSourceFilter::library("marketing", "Marketing system")
        );
        panel.set_paint_style_view_data(DesignPaintStyleViewData::default(), cx);
        assert_eq!(
            panel.style_browser_source_filter,
            StyleBrowserSourceFilter::All
        );
    });
}

#[test]
fn alignment_candidates_follow_fixed_and_auto_spacing_geometry() {
    let mut layout = DesignLayout {
        mode: DesignLayoutMode::Horizontal,
        alignment_x: 2,
        alignment_y: 1,
        ..DesignLayout::default()
    };

    let (fixed, width, height) = alignment_grid_candidates(&layout);
    assert_eq!(fixed.len(), 3);
    assert!(fixed.iter().all(|row| row.len() == 3));
    assert_eq!((width, height), (88., 72.));

    layout.item_spacing_mode = DesignItemSpacingMode::Auto;
    let (horizontal_auto, width, height) = alignment_grid_candidates(&layout);
    assert_eq!(
        horizontal_auto,
        vec![vec![(2, 0)], vec![(2, 1)], vec![(2, 2)]]
    );
    assert_eq!((width, height), (28., 72.));

    layout.mode = DesignLayoutMode::Vertical;
    let (vertical_auto, width, height) = alignment_grid_candidates(&layout);
    assert_eq!(vertical_auto, vec![vec![(0, 1), (1, 1), (2, 1)]]);
    assert_eq!((width, height), (88., 28.));

    layout.mode = DesignLayoutMode::Grid;
    assert!(alignment_grid_candidates(&layout).0.is_empty());
}

#[test]
fn alignment_shortcuts_follow_visible_axes_and_ignore_modifiers() {
    let shortcut = |key: &str| {
        AlignmentGridShortcut::from_key_down(&KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).expect("valid shortcut"),
            is_held: false,
            prefer_character_input: false,
        })
    };
    assert_eq!(shortcut("up"), Some(AlignmentGridShortcut::Up));
    assert_eq!(shortcut("w"), Some(AlignmentGridShortcut::SetTopEdge));
    assert_eq!(shortcut("right"), Some(AlignmentGridShortcut::Right));
    assert_eq!(shortcut("d"), Some(AlignmentGridShortcut::SetRightEdge));
    assert_eq!(shortcut("down"), Some(AlignmentGridShortcut::Down));
    assert_eq!(shortcut("s"), Some(AlignmentGridShortcut::SetBottomEdge));
    assert_eq!(shortcut("left"), Some(AlignmentGridShortcut::Left));
    assert_eq!(shortcut("a"), Some(AlignmentGridShortcut::SetLeftEdge));
    assert_eq!(shortcut("x"), Some(AlignmentGridShortcut::ToggleSpacing));
    assert_eq!(shortcut("b"), Some(AlignmentGridShortcut::ToggleBaseline));
    assert_eq!(shortcut("shift-w"), None);
    assert_eq!(shortcut("cmd-x"), None);

    let mut layout = DesignLayout {
        mode: DesignLayoutMode::Horizontal,
        alignment_x: 1,
        alignment_y: 1,
        ..DesignLayout::default()
    };
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Up),
        Some(DesignAutoLayoutAlignment::new(1, 0))
    );
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Right),
        Some(DesignAutoLayoutAlignment::new(2, 1))
    );
    layout.alignment_x = 2;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Right),
        None,
        "movement clamps at the edge without emitting a duplicate value"
    );
    layout.alignment_y = 2;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::SetTopEdge),
        Some(DesignAutoLayoutAlignment::new(2, 0)),
        "W sets the edge directly instead of stepping through center"
    );

    layout.item_spacing_mode = DesignItemSpacingMode::Auto;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Left),
        None,
        "space-between horizontal flow has no editable primary-axis target"
    );
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::SetLeftEdge),
        None,
        "A cannot set an edge on the distributed horizontal axis"
    );
    layout.alignment_y = 1;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Down),
        Some(DesignAutoLayoutAlignment::new(2, 2))
    );

    layout.mode = DesignLayoutMode::Vertical;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Up),
        None,
        "space-between vertical flow has no editable primary-axis target"
    );
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Left),
        Some(DesignAutoLayoutAlignment::new(1, 1))
    );
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::SetTopEdge),
        None,
        "W cannot set an edge on the distributed vertical axis"
    );

    layout.mode = DesignLayoutMode::Grid;
    assert_eq!(
        alignment_for_shortcut(&layout, AlignmentGridShortcut::Left),
        None
    );
}

#[gpui::test]
fn alignment_grid_keyboard_emits_atomic_host_intents_and_gates_each_leaf(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("keyboard", "Keyboard", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let layout = node.layout.as_mut().expect("horizontal layout");
    layout.alignment_x = 1;
    layout.alignment_y = 1;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    for key in ["right", "x", "b", "shift-w"] {
        let event = KeyDownEvent {
            keystroke: gpui::Keystroke::parse(key).expect("valid shortcut"),
            is_held: false,
            prefer_character_input: false,
        };
        visual_cx.update(|window, app| {
            panel.update(app, |panel, cx| {
                panel.handle_alignment_grid_key_down(&event, window, cx);
            });
        });
    }
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::PropertyChangeRequested {
                node_id: "keyboard".into(),
                property: DesignPanelProperty::AutoLayoutAlignment,
                value: DesignPanelValue::AutoLayoutAlignment(DesignAutoLayoutAlignment::new(2, 1)),
            },
            DesignPanelAction::PropertyChangeRequested {
                node_id: "keyboard".into(),
                property: DesignPanelProperty::ItemSpacingMode,
                value: DesignPanelValue::ItemSpacingMode(DesignItemSpacingMode::Auto),
            },
            DesignPanelAction::PropertyChangeRequested {
                node_id: "keyboard".into(),
                property: DesignPanelProperty::BaselineAlignment,
                value: DesignPanelValue::BaselineAlignment(DesignBaselineAlignment::Baseline),
            },
        ]
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .layout
                .as_ref()
                .expect("controlled layout")
                .clone()
        }),
        *node.layout.as_ref().expect("source layout"),
        "keyboard intents must not mutate the controlled layout snapshot"
    );

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::AutoLayoutAlignment,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::AutoLayoutAlignment(
                        DesignAutoLayoutAlignment::new(1, 1),
                    ))
                    .read_only(),
                ),
                (
                    DesignPanelProperty::BaselineAlignment,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::BaselineAlignment(
                        DesignBaselineAlignment::Bounds,
                    ))
                    .read_only(),
                ),
            ],
            cx,
        );
        assert!(!panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::Up, cx));
        assert!(panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::ToggleSpacing, cx));
        assert!(!panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::ToggleBaseline, cx));
        drop(panel.render_alignment_grid(cx));
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PropertyChangeRequested {
            node_id: "keyboard".into(),
            property: DesignPanelProperty::ItemSpacingMode,
            value: DesignPanelValue::ItemSpacingMode(DesignItemSpacingMode::Auto),
        }]
    );

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states([], cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::Down, cx));
        assert!(!panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::ToggleSpacing, cx));
        assert!(!panel.apply_alignment_grid_shortcut(AlignmentGridShortcut::ToggleBaseline, cx));
        drop(panel.render_alignment_grid(cx));
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn grid_controls_validate_counts_tracks_and_host_track_intents(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("grid", "Grid", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Grid);
    let layout = node.layout.as_mut().expect("Grid layout");
    layout.grid_auto_tracks = DesignGridAutoTracks::None;
    layout.grid_columns = vec![DesignGridTrack::hug(), DesignGridTrack::fixed(120.)];
    layout.grid_rows = vec![DesignGridTrack::hug(), DesignGridTrack::fraction(1.)];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::GridColumnCount),
            Some(DesignPanelValue::Integer(2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::GridRowCount),
            Some(DesignPanelValue::Integer(2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::GridColumnTrackValue(1)),
            Some(DesignPanelValue::Number(120.))
        );
        assert!(
            panel
                .property_options(DesignPanelProperty::GridColumnTrack(0))
                .expect("Grid track options")
                .iter()
                .all(|option| !matches!(
                    &option.value,
                    DesignPanelValue::GridTrack(track)
                        if track.sizing == DesignGridTrackSizing::Fraction
                ))
        );
        assert!(panel.property_is_editable(DesignPanelProperty::GridRowCount));
        assert!(
            panel.grid_track_action_is_applicable(&DesignPanelAction::GridTrackAddRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Row,
                insertion_index: 2,
            })
        );
        assert!(panel.grid_track_action_is_applicable(
            &DesignPanelAction::GridTrackDeleteRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Column,
                index: 0,
            }
        ));
        assert!(panel.grid_track_action_is_applicable(
            &DesignPanelAction::GridTracksReorderRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Column,
                from_indices: vec![0],
                insertion_index: 2,
            }
        ));
        assert!(!panel.grid_track_action_is_applicable(
            &DesignPanelAction::GridTracksReorderRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Column,
                from_indices: vec![0, 0],
                insertion_index: 2,
            }
        ));

        panel.emit_property(
            DesignPanelProperty::GridColumnCount,
            DesignPanelValue::Integer(0),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::GridColumnTrack(1),
            DesignPanelValue::GridTrack(DesignGridTrack::fixed(0.)),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::GridColumnTrack(1),
            DesignPanelValue::GridTrack(DesignGridTrack::fraction(1.)),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::GridColumnTrack(1),
            DesignPanelValue::GridTrack(DesignGridTrack::fixed(144.)),
            cx,
        );

        let mut fixed_axis = panel.node.clone();
        fixed_axis
            .layout
            .as_mut()
            .expect("Grid layout")
            .horizontal_sizing = DesignSizingMode::Fixed;
        panel.set_node(fixed_axis, cx);
        assert!(
            panel
                .property_options(DesignPanelProperty::GridColumnTrack(0))
                .expect("Grid track options")
                .iter()
                .any(|option| matches!(
                    &option.value,
                    DesignPanelValue::GridTrack(track)
                        if track.sizing == DesignGridTrackSizing::Fraction
                ))
        );

        let mut auto_rows = panel.node.clone();
        auto_rows
            .layout
            .as_mut()
            .expect("Grid layout")
            .grid_auto_tracks = DesignGridAutoTracks::Rows;
        panel.set_node(auto_rows, cx);
        assert!(!panel.property_is_editable(DesignPanelProperty::GridRowCount));
        assert!(!panel.grid_track_action_is_applicable(
            &DesignPanelAction::GridTrackAddRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Row,
                insertion_index: 2,
            }
        ));
        assert!(!panel.grid_track_action_is_applicable(
            &DesignPanelAction::GridTrackDeleteRequested {
                node_id: "grid".into(),
                axis: DesignGridTrackAxis::Row,
                index: 0,
            }
        ));
    });

    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PropertyChangeRequested {
            node_id: "grid".into(),
            property: DesignPanelProperty::GridColumnTrack(1),
            value: DesignPanelValue::GridTrack(DesignGridTrack::fixed(144.)),
        }]
    );
}

#[gpui::test]
fn grid_dimensions_picker_is_atomic_keyboard_accessible_and_stale_safe(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("grid", "Grid", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Grid);
    let layout = node.layout.as_mut().expect("Grid layout");
    layout.grid_auto_tracks = DesignGridAutoTracks::None;
    layout.grid_columns = vec![DesignGridTrack::hug(), DesignGridTrack::fixed(120.)];
    layout.grid_rows = vec![DesignGridTrack::hug(), DesignGridTrack::fixed(96.)];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.open_grid_dimensions_picker(cx);
        assert!(panel.set_grid_dimensions_candidate(
            DesignGridDimensions {
                columns: 4,
                rows: 3,
            },
            cx,
        ));
        assert!(panel.commit_grid_dimensions_candidate(cx));
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::GridDimensionsEditRequested {
            node_id: "grid".into(),
            dimensions: DesignGridDimensions {
                columns: 4,
                rows: 3,
            },
            phase: DesignPanelEditPhase::Commit,
        }]
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .layout
                .as_ref()
                .and_then(DesignLayout::grid_dimensions)
        }),
        DesignGridDimensions::new(2, 2),
        "pointer selection emits an intent and keeps the controlled tracks unchanged"
    );

    captured.borrow_mut().clear();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_grid_dimensions_picker(cx);
            for key in ["right", "down", "space"] {
                panel.handle_grid_dimensions_picker_key_down(
                    &KeyDownEvent {
                        keystroke: gpui::Keystroke::parse(key).expect("valid Grid picker shortcut"),
                        is_held: false,
                        prefer_character_input: false,
                    },
                    window,
                    cx,
                );
            }
            assert!(!panel.grid_dimensions_action_is_applicable(
                &DesignPanelAction::GridDimensionsEditRequested {
                    node_id: "stale-grid".into(),
                    dimensions: DesignGridDimensions {
                        columns: 3,
                        rows: 3,
                    },
                    phase: DesignPanelEditPhase::Commit,
                }
            ));
        });
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::GridDimensionsEditRequested {
            node_id: "grid".into(),
            dimensions: DesignGridDimensions {
                columns: 3,
                rows: 3,
            },
            phase: DesignPanelEditPhase::Commit,
        }]
    );

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.emit_property_edit(
            DesignPanelProperty::GridColumnCount,
            DesignPanelValue::Integer(2),
            DesignPanelEditPhase::Begin,
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::GridColumnCount,
            DesignPanelValue::Integer(5),
            DesignPanelEditPhase::Preview,
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::GridColumnCount,
            DesignPanelValue::Integer(2),
            DesignPanelEditPhase::Cancel,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::GridDimensionsEditRequested {
                node_id: "grid".into(),
                dimensions: DesignGridDimensions {
                    columns: 2,
                    rows: 2,
                },
                phase: DesignPanelEditPhase::Begin,
            },
            DesignPanelAction::GridDimensionsEditRequested {
                node_id: "grid".into(),
                dimensions: DesignGridDimensions {
                    columns: 5,
                    rows: 2,
                },
                phase: DesignPanelEditPhase::Preview,
            },
            DesignPanelAction::GridDimensionsEditRequested {
                node_id: "grid".into(),
                dimensions: DesignGridDimensions {
                    columns: 2,
                    rows: 2,
                },
                phase: DesignPanelEditPhase::Cancel,
            },
        ]
    );

    captured.borrow_mut().clear();
    let mut automatic_rows = node.clone();
    automatic_rows
        .layout
        .as_mut()
        .expect("Grid layout")
        .grid_auto_tracks = DesignGridAutoTracks::Rows;
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(automatic_rows.clone(), cx);
        panel.open_grid_dimensions_picker(cx);
        assert!(!panel.step_grid_dimensions_candidate("down", cx));
        assert!(!panel.set_grid_dimensions_candidate(
            DesignGridDimensions {
                columns: 3,
                rows: 4,
            },
            cx,
        ));
        assert!(panel.step_grid_dimensions_candidate("right", cx));
        assert!(panel.commit_grid_dimensions_candidate(cx));
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::GridDimensionsEditRequested {
            node_id: "grid".into(),
            dimensions: DesignGridDimensions {
                columns: 3,
                rows: 2,
            },
            phase: DesignPanelEditPhase::Commit,
        }],
        "automatic rows preserve the exact host-derived row count"
    );

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [(
                DesignPanelProperty::GridColumnCount,
                DesignPanelPropertyValueState::Uniform(DesignPanelValue::Integer(2)).read_only(),
            )],
            cx,
        );
        panel.open_grid_dimensions_picker(cx);
        assert!(panel.grid_dimensions_picker.is_none());
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                automatic_rows,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.open_grid_dimensions_picker(cx);
        assert!(panel.grid_dimensions_picker.is_none());
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[test]
fn grid_dimensions_popover_fits_every_supported_panel_width() {
    for panel_width in [320., 400., 472.] {
        assert!(
            GRID_DIMENSIONS_POPOVER_WIDTH <= panel_width - PANEL_PADDING * 2.,
            "{GRID_DIMENSIONS_POPOVER_WIDTH}px inward-anchored picker must fit {panel_width}px"
        );
    }
}

#[gpui::test]
fn grid_child_alignment_is_a_position_property(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("grid-child", "Grid child", DesignPanelNodeKind::Rectangle);
    let layout = node.layout.as_mut().expect("Grid item data");
    layout.item.grid_horizontal_alignment = DesignGridItemAlignment::Center;
    layout.item.grid_vertical_alignment = DesignGridItemAlignment::End;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Grid,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(panel.property_is_editable(DesignPanelProperty::GridHorizontalAlignment));
        assert!(panel.property_is_editable(DesignPanelProperty::GridVerticalAlignment));
        drop(panel.render_position(cx));
        drop(panel.render_layout(cx));
    });
}

#[gpui::test]
fn grouped_frame_presets_emit_exact_available_identity_and_stay_controlled(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let phone = super::super::DesignFramePresetGroup::new(
        "phone",
        "Phone",
        [
            super::super::DesignFramePreset::new("pro", "Pro phone", 393., 852.),
            super::super::DesignFramePreset::new("legacy", "Legacy phone", 375., 667.)
                .disabled("Archived by the host"),
        ],
    );
    let desktop = super::super::DesignFramePresetGroup::new(
        "desktop",
        "Desktop",
        [super::super::DesignFramePreset::new(
            "wide", "Desktop", 1440., 1024.,
        )],
    );
    let pro = phone.selection("pro");
    let legacy = phone.selection("legacy");
    let catalog = DesignFramePresetViewData::new("frame", [phone, desktop]);

    panel.update(visual_cx, |panel, cx| {
        panel.set_frame_preset_view_data(catalog.clone(), cx);
        assert!(panel.frame_preset_view_data_for_context().is_some());
        let phone_group_id: SharedString = "phone".into();
        panel.toggle_frame_preset_group(&phone_group_id, cx);
        assert!(
            panel
                .collapsed_frame_preset_groups
                .contains(&phone_group_id)
        );
        panel.set_frame_preset_view_data(catalog.clone(), cx);
        assert!(
            panel
                .collapsed_frame_preset_groups
                .contains(&phone_group_id),
            "same-target echoes preserve disclosure state by stable group id"
        );
        panel.toggle_frame_preset_group(&phone_group_id, cx);
        assert!(
            !panel
                .collapsed_frame_preset_groups
                .contains(&phone_group_id)
        );
        assert!(panel.frame_preset_action_is_applicable(&pro, 393., 852.));
        assert!(!panel.frame_preset_action_is_applicable(&pro, 393., 851.));
        assert!(!panel.frame_preset_action_is_applicable(&legacy, 375., 667.));
        drop(panel.render_layout(cx));

        panel.emit_frame_preset_apply(pro.clone(), cx);
        panel.emit_frame_preset_apply(legacy.clone(), cx);
        assert_eq!((panel.node.width, panel.node.height), (320., 180.));

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_frame_preset_apply(pro.clone(), cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::FramePresetApplyRequested {
            node_id: "frame".into(),
            selection: pro,
            width: 393.,
            height: 852.,
        }]
    );
}

#[gpui::test]
fn frame_preset_catalog_is_target_bound_and_hidden_for_non_frames(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let group = super::super::DesignFramePresetGroup::new(
        "desktop",
        "Desktop",
        [super::super::DesignFramePreset::new(
            "desktop", "Desktop", 1440., 1024.,
        )],
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_frame_preset_view_data(
            DesignFramePresetViewData::new("stale-frame", [group.clone()]),
            cx,
        );
        assert!(panel.frame_preset_view_data_for_context().is_none());

        panel.set_frame_preset_view_data(DesignFramePresetViewData::new("frame", [group]), cx);
        assert!(panel.frame_preset_view_data_for_context().is_some());

        panel.frame_preset_browser_open = true;
        panel.set_node(
            DesignPanelNode::new("frame", "Converted group", DesignPanelNodeKind::Group),
            cx,
        );
        assert!(panel.frame_preset_view_data_for_context().is_none());
        assert!(!panel.frame_preset_browser_open);
    });
}

#[gpui::test]
fn color_contrast_catalog_is_bound_to_exact_paint_and_selection_targets(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("shape", "Shape", DesignPanelNodeKind::Rectangle);
    node.fills = vec![DesignPaint::solid(DesignColor::rgb(0xa0, 0xae, 0xc0)).with_id("fill")];
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let leaf = super::super::DesignColorContrastLeafViewData::new(
        DesignPaintColorTarget::Solid,
        DesignColor::WHITE,
        2.19,
        super::super::DesignColorContrastCategory::Graphics,
    );
    let exact = DesignColorContrastPaintViewData::new(
        DesignColorContrastPaintTarget::paint("shape", DesignPanelCollection::Fill, "fill", 0),
        [leaf.clone()],
    );
    let stale = DesignColorContrastPaintViewData::new(
        DesignColorContrastPaintTarget::paint("stale", DesignPanelCollection::Fill, "fill", 0),
        [leaf.clone()],
    );

    panel.update(visual_cx, |panel, cx| {
        panel.active_picker = Some(PaintPickerTarget {
            collection: DesignPanelCollection::Fill,
            index: 0,
            paint_id: "fill".into(),
        });
        panel.set_color_contrast_view_data(DesignColorContrastViewData::new([stale]), cx);
        assert!(panel.active_color_contrast_view_data().is_none());
        panel.set_color_contrast_view_data(DesignColorContrastViewData::new([exact]), cx);
        assert!(panel.active_color_contrast_view_data().is_some());

        let selection = DesignPanelMultipleSelection::new(node.clone(), second.clone());
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                selection,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        let current_target = DesignPanelTarget::Nodes {
            node_ids: vec!["shape".into(), "second".into()],
        };
        let stale_target = DesignPanelTarget::Nodes {
            node_ids: vec!["second".into(), "shape".into()],
        };
        panel.auxiliary_color_picker = Some(AuxiliaryColorPickerTarget::SelectionColor {
            target: stale_target.clone(),
            selection_color_id: "aggregate".into(),
        });
        panel.set_color_contrast_view_data(
            DesignColorContrastViewData::new([DesignColorContrastPaintViewData::new(
                DesignColorContrastPaintTarget::selection_color(stale_target, "aggregate"),
                [leaf.clone()],
            )]),
            cx,
        );
        assert!(panel.active_color_contrast_view_data().is_none());

        panel.auxiliary_color_picker = Some(AuxiliaryColorPickerTarget::SelectionColor {
            target: current_target.clone(),
            selection_color_id: "aggregate".into(),
        });
        panel.set_color_contrast_view_data(
            DesignColorContrastViewData::new([DesignColorContrastPaintViewData::new(
                DesignColorContrastPaintTarget::selection_color(current_target, "aggregate"),
                [leaf],
            )]),
            cx,
        );
        assert!(panel.active_color_contrast_view_data().is_some());
    });
}

struct TestHost {
    panel: Entity<DesignPanel>,
    panel_width: f32,
    render_type_setting_probe: bool,
    actions: Rc<RefCell<Vec<DesignPanelAction>>>,
    key_downs: Rc<RefCell<Vec<SharedString>>>,
    _subscription: Subscription,
}

impl TestHost {
    fn new(node: DesignPanelNode, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let panel = cx.new(|cx| DesignPanel::new("design", node, window, cx));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let captured_actions = actions.clone();
        let key_downs = Rc::new(RefCell::new(Vec::new()));
        let subscription = cx.subscribe(&panel, move |_, _, action: &DesignPanelAction, _| {
            captured_actions.borrow_mut().push(action.clone());
        });
        Self {
            panel,
            panel_width: 320.,
            render_type_setting_probe: false,
            actions,
            key_downs,
            _subscription: subscription,
        }
    }
}

impl Render for TestHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let key_downs = self.key_downs.clone();
        // The dialog layer must live in the window's real element tree for its
        // key bindings (escape -> Cancel) to dispatch, exactly as a production
        // host renders it; an ephemeral `visual_cx.draw` frame does not keep
        // dispatch state past the next refresh.
        let mut content = div()
            .relative()
            .w(px(self.panel_width))
            .h(px(720.))
            .on_key_down(move |event: &KeyDownEvent, _, _| {
                key_downs
                    .borrow_mut()
                    .push(event.keystroke.key.clone().into());
            })
            .child(self.panel.clone())
            .children(Root::render_dialog_layer(window, cx));
        if self.render_type_setting_probe {
            let (editing, invalid, input) = {
                let panel = self.panel.read(cx);
                (
                    panel.property_editor.as_ref().is_some_and(|editor| {
                        editor.property == DesignPanelProperty::ParagraphSpacing
                    }),
                    panel.property_editor_invalid,
                    panel.property_input.clone(),
                )
            };
            let probe = DesignPanel::render_type_setting_number_field(
                "focus-probe".into(),
                "paragraph-spacing",
                "0".into(),
                DesignPanelProperty::ParagraphSpacing,
                DesignPanelValue::Number(0.),
                true,
                editing,
                invalid,
                input,
                self.panel.clone(),
                cx,
            );
            content = content.child(div().absolute().top_0().left_0().w(px(180.)).child(probe));
        }
        content
    }
}

struct TestDialogLayer;

impl Render for TestDialogLayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .children(Root::render_dialog_layer(window, cx))
    }
}

fn setup(
    node: DesignPanelNode,
    cx: &mut TestAppContext,
) -> (Entity<TestHost>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::init(cx);
    });
    let host_slot = Rc::new(RefCell::new(None));
    let captured_host = host_slot.clone();
    let (_, visual_cx) = cx.add_window_view(move |window, cx| {
        let host = cx.new(|cx| TestHost::new(node, window, cx));
        *captured_host.borrow_mut() = Some(host.clone());
        Root::new(host, window, cx)
    });
    let host = host_slot
        .borrow_mut()
        .take()
        .expect("Design test host should be installed");
    visual_cx.run_until_parked();
    (host, visual_cx)
}

fn panel(host: &Entity<TestHost>, cx: &VisualTestContext) -> Entity<DesignPanel> {
    cx.read(|app| host.read(app).panel.clone())
}

fn actions(host: &Entity<TestHost>, cx: &VisualTestContext) -> Rc<RefCell<Vec<DesignPanelAction>>> {
    cx.read(|app| host.read(app).actions.clone())
}

fn key_downs(host: &Entity<TestHost>, cx: &VisualTestContext) -> Rc<RefCell<Vec<SharedString>>> {
    cx.read(|app| host.read(app).key_downs.clone())
}

fn rendered_tab_stop_count(panel: &Entity<DesignPanel>, cx: &mut VisualTestContext) -> usize {
    cx.update(|window, app| {
        panel.read(app).focus_handle.clone().focus(window, app);
        let mut visited = Vec::new();
        for _ in 0..512 {
            window.focus_next(app);
            let focused = window
                .focused(app)
                .expect("the rendered Design panel should expose at least one tab stop");
            let identity = format!("{focused:?}");
            if visited.contains(&identity) {
                return visited.len();
            }
            visited.push(identity);
        }
        panic!("the Design panel tab order should wrap within the bounded control count");
    })
}

#[test]
fn right_sidebar_surface_sets_are_permission_specific() {
    assert_eq!(
        DesignPanelSurface::available(true),
        &[DesignPanelSurface::Design, DesignPanelSurface::Prototype]
    );
    assert_eq!(
        DesignPanelSurface::available(false),
        &[DesignPanelSurface::Comment, DesignPanelSurface::Properties]
    );
    for surface in DesignPanelSurface::EDITOR {
        assert!(surface.is_available(true));
        assert!(!surface.is_available(false));
    }
    for surface in DesignPanelSurface::VIEWER {
        assert!(surface.is_available(false));
        assert!(!surface.is_available(true));
    }
}

#[gpui::test]
fn read_only_custom_property_controls_are_removed_from_the_tab_order(cx: &mut TestAppContext) {
    let frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let x = frame.x;
    let visible = frame.visible;
    let include_strokes = frame
        .layout
        .as_ref()
        .expect("horizontal frame layout")
        .include_strokes;
    let (host, visual_cx) = setup(frame, cx);
    let panel = panel(&host, visual_cx);
    let editable_count = rendered_tab_stop_count(&panel, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [(
                DesignPanelProperty::IncludeStrokes,
                DesignPanelPropertyValueState::Uniform(DesignPanelValue::Bool(include_strokes))
                    .read_only(),
            )],
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        rendered_tab_stop_count(&panel, visual_cx),
        editable_count - 1,
        "a read-only custom toggle must not remain as an inert tab stop",
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::IncludeStrokes,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Bool(include_strokes))
                        .read_only(),
                ),
                (
                    DesignPanelProperty::X,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(x)).read_only(),
                ),
                (
                    DesignPanelProperty::Visible,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Bool(visible))
                        .read_only(),
                ),
            ],
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        rendered_tab_stop_count(&panel, visual_cx),
        editable_count - 3,
        "read-only value, toggle, and visibility divs must each leave the tab order",
    );
}

#[gpui::test]
fn read_only_grid_segments_and_auto_rows_toggle_are_removed_from_the_tab_order(
    cx: &mut TestAppContext,
) {
    let frame = DesignPanelNode::new("grid", "Grid", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Grid);
    let auto_tracks = frame
        .layout
        .as_ref()
        .expect("grid frame layout")
        .grid_auto_tracks;
    let (host, visual_cx) = setup(frame, cx);
    let panel = panel(&host, visual_cx);
    let editable_count = rendered_tab_stop_count(&panel, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::LayoutMode,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::LayoutMode(
                        DesignLayoutMode::Grid,
                    ))
                    .read_only(),
                ),
                (
                    DesignPanelProperty::GridAutoTracks,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::GridAutoTracks(
                        auto_tracks,
                    ))
                    .read_only(),
                ),
            ],
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        rendered_tab_stop_count(&panel, visual_cx),
        editable_count - 5,
        "four read-only layout-mode segments and Auto rows must leave the tab order",
    );
}

#[gpui::test]
fn surface_tabs_request_host_echo_from_pointer_and_keyboard(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    assert_eq!(
        visual_cx.read(|app| panel.read(app).available_surfaces()),
        DesignPanelSurface::EDITOR.as_slice()
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Design
    );
    assert!(visual_cx.debug_bounds("design-tab-design-active").is_some());

    let prototype_tab = visual_cx
        .debug_bounds("design-tab-prototype")
        .expect("Prototype Button")
        .center();
    visual_cx.simulate_click(prototype_tab, Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::SurfaceChangeRequested {
            current: DesignPanelSurface::Design,
            requested: DesignPanelSurface::Prototype,
        }]
    );
    captured_actions.borrow_mut().clear();

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.focus_handle.focus(window, cx);
        });
    });
    visual_cx.simulate_keystrokes("tab tab enter");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::SurfaceChangeRequested {
            current: DesignPanelSurface::Design,
            requested: DesignPanelSurface::Prototype,
        }]
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Design,
        "a navigation request must not optimistically change controlled state"
    );

    captured_actions.borrow_mut().clear();
    visual_cx.simulate_keystrokes("space");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::SurfaceChangeRequested {
            current: DesignPanelSurface::Design,
            requested: DesignPanelSurface::Prototype,
        }],
        "Space on the focused Button must use the same request path"
    );

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.set_active_surface(DesignPanelSurface::Prototype, cx));
        assert!(!panel.set_active_surface(DesignPanelSurface::Properties, cx));
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Prototype
    );
    assert!(
        visual_cx
            .debug_bounds("design-tab-prototype-active")
            .is_some()
    );
    assert!(
        visual_cx
            .debug_bounds("design-surface-prototype-host-owned")
            .is_some(),
        "a non-inspector surface must render the explicit host-owned projection"
    );

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).available_surfaces()),
        DesignPanelSurface::VIEWER.as_slice()
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Properties,
        "viewer permissions use their independently controlled default"
    );

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.focus_handle.focus(window, cx);
        });
    });
    visual_cx.simulate_keystrokes("tab enter");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::SurfaceChangeRequested {
            current: DesignPanelSurface::Properties,
            requested: DesignPanelSurface::Comment,
        }]
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Properties
    );

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.set_active_surface(DesignPanelSurface::Comment, cx));
        assert!(!panel.set_active_surface(DesignPanelSurface::Design, cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-surface-comment-host-owned")
            .is_some()
    );
}

#[gpui::test]
fn surface_changes_balance_active_edits_and_dismiss_design_overlays(cx: &mut TestAppContext) {
    let rectangle = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(10.),
                window,
                cx,
            );
            panel.selection_header_overlay = Some(SelectionHeaderOverlay::More);
            panel.appearance_blend_mode_open = true;
            panel.type_settings_open = true;

            assert!(
                !panel.set_active_surface(DesignPanelSurface::Properties, cx),
                "an unavailable surface must not disturb the active Design interaction",
            );
            assert!(panel.property_editor.is_some());
            assert!(panel.set_active_surface(DesignPanelSurface::Prototype, cx));
        });
    });
    visual_cx.run_until_parked();

    let phases = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::X,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(panel.active_surface(), DesignPanelSurface::Prototype);
        assert!(panel.property_editor.is_none());
        assert!(panel.selection_header_overlay.is_none());
        assert!(!panel.appearance_blend_mode_open);
        assert!(!panel.type_settings_open);
    });
}

#[gpui::test]
fn draw_workspace_preserves_the_saved_editor_surface_and_mounts_its_projection(
    cx: &mut TestAppContext,
) {
    let frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(frame, cx);
    let panel = panel(&host, visual_cx);
    assert_eq!(
        visual_cx.read(|app| panel.read(app).workspace_mode()),
        DesignPanelWorkspaceMode::Design
    );
    panel.update(visual_cx, |panel, cx| {
        assert!(panel.set_active_surface(DesignPanelSurface::Prototype, cx));
        assert!(panel.set_draw_appearance_view_data(
            DesignDrawAppearanceViewData::new(
                DesignPanelTarget::Nodes {
                    node_ids: vec!["frame".into()],
                },
                DesignDrawSliderRange::new(0., 320., 1.).expect("valid test corner range"),
            ),
            cx,
        ));
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
                node_ids: vec!["frame".into()],
            }),
            cx,
        );
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(
        visual_cx.read(|app| panel.read(app).workspace_mode()),
        DesignPanelWorkspaceMode::Draw
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Prototype,
        "Draw must preserve the host's accepted editor surface"
    );
    for selector in [
        "design-draw-workspace-heading",
        "design-draw-position-content",
        "design-draw-layout-flow-label",
        "design-draw-add-auto-layout",
        "design-draw-opacity-slider",
        "design-draw-corner-radius-slider",
    ] {
        assert!(
            visual_cx.debug_bounds(selector).is_some(),
            "Draw should mount {selector}"
        );
    }
    assert!(
        visual_cx
            .debug_bounds("design-surface-prototype-host-owned")
            .is_none(),
        "the saved Prototype surface must not replace Draw's inspector projection"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Design, cx);
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).active_surface()),
        DesignPanelSurface::Prototype
    );
    assert!(
        visual_cx
            .debug_bounds("design-surface-prototype-host-owned")
            .is_some(),
        "returning to Design must reveal the still-controlled Prototype projection"
    );
}

#[gpui::test]
fn workspace_changes_balance_an_active_property_transaction(cx: &mut TestAppContext) {
    let rectangle = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(10.),
                window,
                cx,
            );
            panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
        });
    });
    visual_cx.run_until_parked();

    let phases = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::X,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
    assert!(
        visual_cx.read(|app| panel.read(app).property_editor.is_none()),
        "the replaced Design editor must not retain a hidden draft"
    );
}

#[gpui::test]
fn mixed_visibility_resolves_to_show_all_with_the_exact_target_on_draw_surface(
    cx: &mut TestAppContext,
) {
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.visible = true;
    let mut second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    second.visible = false;
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["one".into(), "two".into()],
    };
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        panel.expanded_sections.clear();
        panel.expanded_sections.insert(DesignPanelSection::Layer);
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-draw-appearance-visible")
            .is_some(),
        "Draw must mount the shared visibility control",
    );
    assert_eq!(
        visual_cx.read(|app| panel
            .read(app)
            .visibility_control_state(DesignPanelProperty::Visible, panel.read(app).node.visible,)),
        VisibilityControlState::Mixed,
    );

    let visibility = visual_cx
        .debug_bounds("design-draw-appearance-visible")
        .expect("Draw visibility control")
        .center();
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    {
        let captured = captured.borrow();
        assert_eq!(captured.len(), 1);
        let (action_target, action) = captured[0]
            .targeted_node_action()
            .expect("Draw visibility must preserve the multiple-selection target");
        assert_eq!(action_target, &target);
        assert!(matches!(
            action,
            DesignPanelAction::PropertyChangeRequested {
                node_id,
                property: DesignPanelProperty::Visible,
                value: DesignPanelValue::Bool(true),
            } if node_id.as_ref() == "one"
        ));
    }

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Mixed.read_only_with_reason("Locked by the host"),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel
            .read(app)
            .visibility_control_state(DesignPanelProperty::Visible, panel.read(app).node.visible,)),
        VisibilityControlState::Mixed,
    );
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    assert!(
        captured.borrow().is_empty(),
        "Draw must preserve read-only mixed visibility",
    );
}

#[gpui::test]
fn draw_opacity_slider_uses_balanced_phases_and_strict_keyboard_propagation(
    cx: &mut TestAppContext,
) {
    let rectangle = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(rectangle.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let captured_keys = key_downs(&host, visual_cx);
    panel.update(visual_cx, |panel, cx| {
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
        assert!(panel.set_draw_appearance_view_data(
            DesignDrawAppearanceViewData::new(
                DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                DesignDrawSliderRange::new(0., 240., 1.).expect("valid test corner range"),
            ),
            cx,
        ));
    });
    visual_cx.run_until_parked();

    let bounds = visual_cx
        .debug_bounds("design-draw-opacity-slider")
        .expect("Draw opacity slider");
    let quarter = gpui::point(bounds.left() + bounds.size.width * 0.25, bounds.center().y);
    visual_cx.simulate_mouse_down(quarter, MouseButton::Left, Modifiers::none());
    visual_cx.simulate_mouse_up(quarter, MouseButton::Left, Modifiers::none());
    visual_cx.run_until_parked();
    let phases = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Opacity,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
        ]
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.opacity),
        rectangle.opacity,
        "the slider must wait for a host echo"
    );

    captured_actions.borrow_mut().clear();
    captured_keys.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Opacity,
            DesignPanelPropertyValueState::bound(DesignPanelPropertyBinding::new(
                "opacity-variable",
                "Opacity",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(rectangle.opacity),
            )),
            cx,
        );
    });
    visual_cx.run_until_parked();
    visual_cx.simulate_click(bounds.center(), Modifiers::none());
    visual_cx.simulate_keystrokes("right");
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a bound Draw slider must not begin an edit"
    );
    assert_eq!(
        captured_keys.borrow().as_slice(),
        [SharedString::from("right")],
        "a disabled slider must leave its arrow key available to the host"
    );

    captured_keys.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                rectangle,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| panel.focus_handle.focus(window, cx));
    });
    visual_cx.simulate_keystrokes("right");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_keys.borrow().as_slice(),
        [SharedString::from("right")],
        "viewer mode must not consume Draw slider keys"
    );
}

#[gpui::test]
fn draw_corner_slider_requires_current_target_bound_host_data(cx: &mut TestAppContext) {
    let rectangle = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);
    panel.update(visual_cx, |panel, cx| {
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
        assert!(!panel.set_draw_appearance_view_data(
            DesignDrawAppearanceViewData::new(
                DesignPanelTarget::Page {
                    page_id: "page".into(),
                },
                DesignDrawSliderRange::new(0., 100., 1.).expect("valid range"),
            ),
            cx,
        ));
        assert!(panel.set_draw_appearance_view_data(
            DesignDrawAppearanceViewData::new(
                DesignPanelTarget::Nodes {
                    node_ids: vec!["stale-node".into()],
                },
                DesignDrawSliderRange::new(0., 100., 1.).expect("valid range"),
            ),
            cx,
        ));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-draw-opacity-slider")
            .is_some(),
        "opacity has a fixed public range"
    );
    assert!(
        visual_cx
            .debug_bounds("design-draw-corner-radius-slider")
            .is_none(),
        "stale corner metadata must not render a usable slider"
    );
}

#[gpui::test]
fn multi_draw_corner_range_requires_uniform_geometry_and_exact_order(cx: &mut TestAppContext) {
    let mut first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    first.width = 120.;
    first.height = 80.;
    let mut second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Rectangle);
    second.width = 240.;
    second.height = 80.;
    let exact_target = DesignPanelTarget::Nodes {
        node_ids: vec!["first".into(), "second".into()],
    };
    let view_data = DesignDrawAppearanceViewData::new(
        exact_target,
        DesignDrawSliderRange::new(0., 240., 1.).expect("valid aggregate range"),
    );
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::Width,
                    DesignPanelPropertyValueState::Mixed,
                ),
                (
                    DesignPanelProperty::Height,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(80.)),
                ),
            ],
            cx,
        );
        assert!(panel.set_draw_appearance_view_data(view_data, cx));
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
        assert!(
            panel.draw_appearance_view_data_for_context().is_none(),
            "an exact target cannot make a mixed geometry range authoritative",
        );
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-draw-corner-radius-slider")
            .is_none(),
        "mixed geometry must omit the corner slider",
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::Width,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(120.)),
                ),
                (
                    DesignPanelProperty::Height,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(80.)),
                ),
            ],
            cx,
        );
        assert!(
            panel.draw_appearance_view_data_for_context().is_some(),
            "a host-resolved uniform aggregate may share one range",
        );
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-draw-corner-radius-slider")
            .is_some(),
        "uniform geometry should render its exact aggregate range",
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(second, first),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(
            panel.draw_appearance_view_data_for_context().is_none(),
            "reordering the selection must stale the ordered range",
        );
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-draw-corner-radius-slider")
            .is_none(),
    );
}

#[test]
fn additional_labels_use_short_native_property_names() {
    assert_eq!(
        DesignPanel::compact_property_label(DesignPanelProperty::X),
        "X position"
    );
    assert_eq!(
        DesignPanel::compact_property_label(DesignPanelProperty::HorizontalSizing),
        "Width sizing"
    );
    assert_eq!(
        DesignPanel::compact_property_label(DesignPanelProperty::Opacity),
        "Opacity"
    );
    assert_eq!(
        DesignPanel::compact_property_label(DesignPanelProperty::EffectShadowBlendMode(0)),
        "Blend mode"
    );
    assert_eq!(
        DesignPanel::compact_property_label(DesignPanelProperty::ExportFormat(0)),
        "Format"
    );
}

#[gpui::test]
fn additional_labels_are_transient_width_safe_and_do_not_emit_document_intents(
    cx: &mut TestAppContext,
) {
    let rectangle = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    assert!(!visual_cx.read(|app| panel.read(app).additional_labels()));
    assert!(
        visual_cx
            .debug_bounds("design-x-additional-label")
            .is_none()
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_additional_labels(true, cx);
        assert!(panel.additional_labels());
    });
    visual_cx.run_until_parked();
    assert!(
        captured.borrow().is_empty(),
        "changing a file preference must not look like a document edit"
    );

    for width in [320., 400., 472.] {
        host.update(visual_cx, |host, cx| {
            host.panel_width = width;
            cx.notify();
        });
        visual_cx.run_until_parked();

        let cell = visual_cx
            .debug_bounds("design-x")
            .unwrap_or_else(|| panic!("X cell at {width}px"));
        let label = visual_cx
            .debug_bounds("design-x-additional-label")
            .unwrap_or_else(|| panic!("X label at {width}px"));
        assert!(label.origin.x >= cell.origin.x);
        assert!(
            label.origin.x + label.size.width <= cell.origin.x + cell.size.width,
            "the label must truncate inside its compact cell at {width}px"
        );
    }

    assert!(
        visual_cx
            .debug_bounds("design-opacity-value-additional-label")
            .is_some(),
        "icon-led Appearance controls also receive a textual label"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
                .with_layout_mode(DesignLayoutMode::Horizontal),
            cx,
        );
        assert!(
            panel.additional_labels(),
            "the cross-file preference survives node selection changes"
        );
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-layout-item-spacing-mode-additional-label")
            .is_some(),
        "select-backed compact controls expose the same preference"
    );
    assert!(captured.borrow().is_empty());

    panel.update(visual_cx, |panel, cx| {
        panel.set_additional_labels(false, cx);
        assert!(!panel.additional_labels());
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn numeric_pointer_scrub_emits_one_typed_lifecycle_with_modifier_precision(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.x = 10.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let mut coarse = Modifiers::none();
    coarse.shift = true;
    let mut fine = Modifiers::none();
    fine.alt = true;

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                1.,
                0.,
                Modifiers::none(),
                cx,
            );
            assert!(
                captured.borrow().is_empty(),
                "sub-threshold movement remains a click candidate and emits no transaction"
            );
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                3.,
                0.,
                Modifiers::none(),
                cx,
            );
            panel.update_numeric_property_scrub(DesignPanelProperty::X, 5., 0., coarse, cx);
            panel.update_numeric_property_scrub(DesignPanelProperty::X, 6., 0., fine, cx);
            assert!(panel.finish_numeric_property_scrub(true, false, window, cx));
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::X,
                value: DesignPanelValue::Number(value),
                phase,
                ..
            } => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (10., DesignPanelEditPhase::Begin),
            (13., DesignPanelEditPhase::Preview),
            (33., DesignPanelEditPhase::Preview),
            (33.1, DesignPanelEditPhase::Preview),
            (33.1, DesignPanelEditPhase::Commit),
        ]
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.x),
        10.,
        "pointer previews remain host controlled"
    );
    assert!(visual_cx.read(|app| {
        let panel = panel.read(app);
        panel.property_editor.is_none() && panel.numeric_property_scrub.is_none()
    }));
}

#[gpui::test]
fn numeric_scrub_focus_is_captured_after_mouse_down_and_rebases_with_its_guide(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        DesignLayoutGrid::uniform(8., DesignColor::BLUE).with_id("guide-a"),
        DesignLayoutGrid::uniform(12., DesignColor::PURPLE).with_id("guide-b"),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_numeric_property_scrub(
                DesignPanelProperty::LayoutGridSize(1),
                0.,
                0.,
                cx,
            ));
            cx.defer_in(window, |panel, window, cx| {
                panel.capture_numeric_scrub_focus(
                    EditorFocusOrigin::ValueCell(DesignPanelProperty::LayoutGridSize(1)),
                    DesignPanelProperty::LayoutGridSize(1),
                    window,
                    cx,
                );
            });
        });
    });
    visual_cx.run_until_parked();

    let return_handle = visual_cx.read(|app| {
        let panel = panel.read(app);
        let return_focus = panel
            .editor_focus_return
            .as_ref()
            .expect("the deferred mouse-down phase should retain the row");
        assert_eq!(
            return_focus.origin,
            EditorFocusOrigin::ValueCell(DesignPanelProperty::LayoutGridSize(1))
        );
        return_focus.handle.clone()
    });

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            let (origin_x, origin_y) = panel
                .numeric_property_scrub
                .as_ref()
                .map(|scrub| (scrub.origin_x, scrub.origin_y))
                .expect("mousedown should seed a scrub");
            panel.update_numeric_property_scrub(
                DesignPanelProperty::LayoutGridSize(1),
                origin_x + NUMERIC_SCRUB_THRESHOLD + 1.,
                origin_y,
                Modifiers::none(),
                cx,
            );
            assert!(panel.numeric_scrub_is_active(DesignPanelProperty::LayoutGridSize(1)));

            node.layout_grids.swap(0, 1);
            panel.set_node(node.clone(), cx);
            assert_eq!(
                panel
                    .numeric_property_scrub
                    .as_ref()
                    .map(|scrub| scrub.property),
                Some(DesignPanelProperty::LayoutGridSize(0))
            );
            assert_eq!(
                panel
                    .editor_focus_return
                    .as_ref()
                    .map(|return_focus| return_focus.origin.clone()),
                Some(EditorFocusOrigin::ValueCell(
                    DesignPanelProperty::LayoutGridSize(0)
                ))
            );
            assert!(panel.finish_numeric_property_scrub(true, false, window, cx));
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));

    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .property_editor
                .as_ref()
                .map(|editor| editor.property)
        }),
        Some(DesignPanelProperty::LayoutGridSize(0)),
        "Enter should reopen the rebased logical value row"
    );
}

#[gpui::test]
fn numeric_scrub_uses_all_four_vertical_speed_bands_and_transient_cue(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.x = 10.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                3.,
                0.,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Normal));
        });
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-scrub-speed-cue").is_some(),
        "an active scrub renders its speed and cursor-width cue"
    );

    let mut coarse = Modifiers::none();
    coarse.shift = true;
    let mut fine = Modifiers::none();
    fine.alt = true;
    let mut coarse_fine = coarse;
    coarse_fine.alt = true;
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                4.,
                DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Double));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                5.,
                DESIGN_SCRUB_HALF_SPEED_Y_THRESHOLD,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Half));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                6.,
                DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Quarter));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                7.,
                DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD,
                coarse,
                cx,
            );
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                8.,
                DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD,
                fine,
                cx,
            );
            panel.update_numeric_property_scrub(
                DesignPanelProperty::X,
                9.,
                DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD,
                coarse_fine,
                cx,
            );
            assert!(panel.finish_numeric_property_scrub(false, false, window, cx));
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-scrub-speed-cue").is_none());

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::X,
                value: DesignPanelValue::Number(value),
                phase,
                ..
            } => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = [
        (10., DesignPanelEditPhase::Begin),
        (13., DesignPanelEditPhase::Preview),
        (15., DesignPanelEditPhase::Preview),
        (15.5, DesignPanelEditPhase::Preview),
        (15.75, DesignPanelEditPhase::Preview),
        (18.25, DesignPanelEditPhase::Preview),
        (18.275, DesignPanelEditPhase::Preview),
        (18.525, DesignPanelEditPhase::Preview),
        (10., DesignPanelEditPhase::Cancel),
    ];
    assert_eq!(edits.len(), expected.len());
    for ((actual, actual_phase), (expected, expected_phase)) in edits.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.001);
        assert_eq!(actual_phase, expected_phase);
    }
}

#[gpui::test]
fn host_nudge_preferences_drive_numeric_lifecycle_without_mutating_the_node(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.x = 10.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let custom = DesignNudgeSettings::new(0.5, 8.).expect("valid nudge settings");

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert_eq!(panel.nudge_settings(), DesignNudgeSettings::default());
            panel.set_nudge_settings(custom, cx);
            assert_eq!(panel.nudge_settings(), custom);
            assert!(captured.borrow().is_empty());
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(10.),
                window,
                cx,
            );
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.step_property_editor(ArrowStep::Increase, false, window, cx));
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.step_property_editor(ArrowStep::Increase, true, window, cx));
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::X,
                value: DesignPanelValue::Number(value),
                phase,
                ..
            } => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (10., DesignPanelEditPhase::Begin),
            (10.5, DesignPanelEditPhase::Preview),
            (18.5, DesignPanelEditPhase::Preview),
            (18.5, DesignPanelEditPhase::Commit),
        ]
    );
    assert_eq!(visual_cx.read(|app| panel.read(app).node.x), 10.);
}

#[gpui::test]
fn arrow_nudge_rejects_invalid_auto_none_and_nonnumeric_drafts(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.x = 4.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(4.),
                window,
                cx,
            );
            panel.property_input.update(cx, |input, cx| {
                input.set_value("not-a-number", window, cx);
            });
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            assert_eq!(
                panel.property_input.read(cx).value().as_ref(),
                "not-a-number"
            );
            panel.finish_property_edit(false, window, cx);

            panel.set_property_value_state(
                DesignPanelProperty::X,
                DesignPanelPropertyValueState::Mixed,
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(4.),
                window,
                cx,
            );
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "");
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            panel.finish_property_edit(false, window, cx);
            panel.set_property_value_states([], cx);

            panel.set_property_value_state(
                DesignPanelProperty::X,
                DesignPanelPropertyValueState::Unset,
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(4.),
                window,
                cx,
            );
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "");
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            panel.finish_property_edit(false, window, cx);
            panel.set_property_value_states([], cx);

            let frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
                .with_layout_mode(DesignLayoutMode::Horizontal);
            panel.set_node(frame, cx);
            panel.activate_property(
                DesignPanelProperty::MinWidth,
                DesignPanelValue::OptionalNumber(None),
                window,
                cx,
            );
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "");
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            panel.finish_property_edit(false, window, cx);

            let mut grid = DesignPanelNode::new("grid", "Grid", DesignPanelNodeKind::Frame);
            grid.layout_grids = vec![
                DesignLayoutGrid::columns(
                    super::super::DesignColumnLayoutGrid {
                        count: DesignLayoutGridCount::Auto,
                        ..Default::default()
                    },
                    DesignColor::PURPLE,
                )
                .with_id("auto-guide"),
            ];
            panel.set_node(grid, cx);
            panel.activate_property(
                DesignPanelProperty::LayoutGridCount(0),
                DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto),
                window,
                cx,
            );
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "Auto");
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            panel.finish_property_edit(false, window, cx);

            let text = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
            panel.set_node(text, cx);
            panel.activate_property(
                DesignPanelProperty::FontFamily,
                DesignPanelValue::Text("Inter".into()),
                window,
                cx,
            );
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            panel.finish_property_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();

    assert!(
        captured.borrow().iter().all(|action| {
            !matches!(
                action,
                DesignPanelAction::PropertyEditRequested {
                    phase: DesignPanelEditPhase::Preview,
                    ..
                } | DesignPanelAction::LayoutGridPropertyEditRequested {
                    phase: DesignPanelEditPhase::Preview,
                    ..
                }
            )
        }),
        "rejected arrow keys neither replace the draft nor emit a numeric preview"
    );
}

#[gpui::test]
fn export_nudge_preserves_discriminator_stable_id_and_clamp(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("slice", "Slice", DesignPanelNodeKind::Slice);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["slice".into()],
    };
    let mut configuration =
        DesignExportConfiguration::new("export-stable", DesignExportFormat::Png);
    configuration.sizing = DesignExportSizing::Width(320.);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_export_view_data(
                DesignExportViewData {
                    target,
                    configurations: vec![configuration],
                    mode: DesignExportMode::Static,
                    static_capabilities: Default::default(),
                    preview: None,
                    animated: None,
                },
                cx,
            );
            panel.set_nudge_settings(
                DesignNudgeSettings::new(0.5, 8.).expect("valid nudge settings"),
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::ExportSizing(0),
                DesignPanelValue::ExportSizing(DesignExportSizing::Width(320.)),
                window,
                cx,
            );
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.property_input.update(cx, |input, cx| {
                input.set_value("2x", window, cx);
            });
            assert!(!panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "2x");
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.property_input.update(cx, |input, cx| {
                input.set_value("320w", window, cx);
            });
            assert!(panel.step_property_editor(ArrowStep::Increase, false, window, cx));
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "320.5w");
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.property_input.update(cx, |input, cx| {
                input.set_value("0.01w", window, cx);
            });
            assert!(panel.step_property_editor(ArrowStep::Decrease, true, window, cx));
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "0.01w");
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();

    assert!(captured.borrow().iter().all(|action| {
        !matches!(
            action,
            DesignPanelAction::ExportConfigurationChangeRequested {
                configuration_id,
                ..
            } if configuration_id.as_ref() != "export-stable"
        )
    }));
    assert!(captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::ExportConfigurationChangeRequested {
            configuration_id,
            change:
                DesignExportConfigurationChange::Sizing(DesignExportSizing::Width(value)),
            phase: DesignPanelEditPhase::Preview,
            ..
        } if configuration_id.as_ref() == "export-stable"
            && (*value - 320.5).abs() < f32::EPSILON
    )));
}

#[gpui::test]
fn numeric_scrub_is_gated_for_mixed_bound_read_only_and_viewer_values(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::X,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        assert!(!panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));

        panel.set_property_value_state(
            DesignPanelProperty::X,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "x-variable",
                "Position / X",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(0.),
            )),
            cx,
        );
        assert!(!panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));

        panel.set_property_value_state(
            DesignPanelProperty::X,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(0.))
                .read_only_with_reason("Locked"),
            cx,
        );
        assert!(!panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.start_numeric_property_scrub(DesignPanelProperty::X, 0., 0., cx));
        assert!(panel.numeric_property_scrub.is_none());
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn numeric_scrub_state_downgrade_cancels_once_before_replacement(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.width = 100.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_numeric_property_scrub(DesignPanelProperty::Width, 0., 0., cx));
            panel.update_numeric_property_scrub(
                DesignPanelProperty::Width,
                4.,
                0.,
                Modifiers::none(),
                cx,
            );
            panel.set_property_value_state(
                DesignPanelProperty::Width,
                DesignPanelPropertyValueState::bound(
                    super::super::DesignPanelPropertyBinding::new(
                        "width-variable",
                        "Dimensions / Width",
                        DesignPanelBindingKind::Variable,
                        DesignPanelValue::Number(104.),
                    ),
                ),
                cx,
            );
            assert!(!panel.finish_numeric_property_scrub(true, false, window, cx));
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Width,
                value: DesignPanelValue::Number(value),
                phase,
                ..
            } => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (100., DesignPanelEditPhase::Begin),
            (104., DesignPanelEditPhase::Preview),
            (100., DesignPanelEditPhase::Cancel),
        ]
    );
    assert!(visual_cx.read(|app| {
        let panel = panel.read(app);
        panel.property_editor.is_none() && panel.numeric_property_scrub.is_none()
    }));
}

#[gpui::test]
fn variable_font_axis_pointer_scrub_emits_tag_keyed_phases_with_precision_modifiers(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes = vec![DesignFontAxis::new(
        "wght", "Weight", 520., 100., 900., 400.,
    )];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let mut coarse = Modifiers::none();
    coarse.shift = true;
    let mut fine = Modifiers::none();
    fine.alt = true;

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_variable_font_axis_scrub("wght".into(), 0., 0., cx));
            panel.update_variable_font_axis_scrub("wght", 1., 0., Modifiers::none(), cx);
            assert!(
                captured.borrow().is_empty(),
                "sub-threshold motion is still a click candidate"
            );
            panel.update_variable_font_axis_scrub("wght", 4., 0., Modifiers::none(), cx);
            panel.update_variable_font_axis_scrub("wght", 5., 0., coarse, cx);
            panel.update_variable_font_axis_scrub("wght", 6., 0., fine, cx);
            assert!(panel.finish_variable_font_axis_scrub(true, false, window, cx));
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyVariableAxisEditRequested {
                target,
                tag,
                value,
                phase,
                ..
            } => Some((*target, tag.clone(), *value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (
                DesignTypographyTarget::WholeLayer,
                "wght".into(),
                520.,
                DesignPanelEditPhase::Begin,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                "wght".into(),
                540.,
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                "wght".into(),
                590.,
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                "wght".into(),
                590.5,
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                "wght".into(),
                590.5,
                DesignPanelEditPhase::Commit,
            ),
        ]
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .variable_font_axis("wght")
                .expect("controlled axis")
                .value
        }),
        520.,
        "the reusable panel emits intents and never mutates the host axis"
    );
}

#[gpui::test]
fn variable_font_axis_scrub_uses_vertical_speed_bands(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes = vec![DesignFontAxis::new(
        "wght", "Weight", 520., 100., 900., 400.,
    )];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_variable_font_axis_scrub("wght".into(), 0., 0., cx));
            panel.update_variable_font_axis_scrub(
                "wght",
                4.,
                DESIGN_SCRUB_DOUBLE_SPEED_Y_THRESHOLD,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Double));
            panel.update_variable_font_axis_scrub(
                "wght",
                5.,
                DESIGN_SCRUB_QUARTER_SPEED_Y_THRESHOLD,
                Modifiers::none(),
                cx,
            );
            assert_eq!(panel.active_scrub_speed(), Some(DesignScrubSpeed::Quarter));
            assert!(panel.finish_variable_font_axis_scrub(false, false, window, cx));
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag, value, phase, ..
            } if tag.as_ref() == "wght" => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (520., DesignPanelEditPhase::Begin),
            (560., DesignPanelEditPhase::Preview),
            (561., DesignPanelEditPhase::Preview),
            (520., DesignPanelEditPhase::Cancel),
        ]
    );
}

#[gpui::test]
fn variable_font_axes_use_custom_nudges_and_reject_invalid_drafts(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes =
        vec![DesignFontAxis::new("wdth", "Width", 96.5, 75., 125., 100.).with_step(0.1)];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_nudge_settings(
                DesignNudgeSettings::new(0.5, 8.).expect("valid nudge settings"),
                cx,
            );
            panel.activate_variable_font_axis_input("wdth".into(), window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.step_variable_font_axis_input(
                ArrowStep::Increase,
                Modifiers::none(),
                window,
                cx
            ));
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            let mut coarse = Modifiers::none();
            coarse.shift = true;
            assert!(panel.step_variable_font_axis_input(ArrowStep::Increase, coarse, window, cx));
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.property_input.update(cx, |input, cx| {
                input.set_value("invalid", window, cx);
            });
            assert!(!panel.step_variable_font_axis_input(
                ArrowStep::Increase,
                Modifiers::none(),
                window,
                cx
            ));
            assert_eq!(panel.property_input.read(cx).value().as_ref(), "invalid");
            panel.finish_variable_font_axis_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|_, app| {
        panel.update(app, |panel, cx| {
            let mut fine = Modifiers::none();
            fine.alt = true;
            assert!(
                panel.apply_variable_font_axis_keyboard_step("wdth".into(), "right", fine, cx,)
            );
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag, value, phase, ..
            } if tag.as_ref() == "wdth" => Some((*value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = [
        (96.5, DesignPanelEditPhase::Begin),
        (96.55, DesignPanelEditPhase::Preview),
        (97.35, DesignPanelEditPhase::Preview),
        (96.5, DesignPanelEditPhase::Cancel),
        (96.5, DesignPanelEditPhase::Begin),
        (96.505, DesignPanelEditPhase::Preview),
        (96.505, DesignPanelEditPhase::Commit),
    ];
    assert_eq!(edits.len(), expected.len());
    for ((actual, actual_phase), (expected, expected_phase)) in edits.into_iter().zip(expected) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {expected} for {expected_phase:?}, got {actual}"
        );
        assert_eq!(actual_phase, expected_phase);
    }
}

#[gpui::test]
fn variable_font_axis_input_and_slider_keyboard_share_phased_intents(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes =
        vec![DesignFontAxis::new("wdth", "Width", 96.5, 75., 125., 100.).with_step(0.1)];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_variable_font_axis_input("wdth".into(), window, cx);
            panel.property_input.update(cx, |input, cx| {
                input.set_value("102.25", window, cx);
            });
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_variable_font_axis_edit(true, window, cx);
            let mut fine = Modifiers::none();
            fine.alt = true;
            assert!(
                panel.apply_variable_font_axis_keyboard_step("wdth".into(), "right", fine, cx,)
            );
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag, value, phase, ..
            } => Some((tag.clone(), *value, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            ("wdth".into(), 96.5, DesignPanelEditPhase::Begin),
            ("wdth".into(), 102.25, DesignPanelEditPhase::Preview),
            ("wdth".into(), 102.25, DesignPanelEditPhase::Commit),
            ("wdth".into(), 96.5, DesignPanelEditPhase::Begin),
            ("wdth".into(), 96.51, DesignPanelEditPhase::Preview),
            ("wdth".into(), 96.51, DesignPanelEditPhase::Commit),
        ]
    );
}

#[gpui::test]
fn variable_font_axis_transaction_survives_same_tag_reorder_and_echo(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes = vec![
        DesignFontAxis::new("wght", "Weight", 520., 100., 900., 400.),
        DesignFontAxis::new("opsz", "Optical size", 18., 9., 144., 14.).with_step(0.1),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.start_variable_font_axis_scrub("wght".into(), 0., 0., cx));
            panel.update_variable_font_axis_scrub("wght", 4., 0., Modifiers::none(), cx);

            let axes = &mut node
                .typography
                .as_mut()
                .expect("text typography")
                .variable_axes;
            axes.iter_mut()
                .find(|axis| axis.tag.as_ref() == "wght")
                .expect("weight axis")
                .value = 540.;
            axes.swap(0, 1);
            panel.set_node(node.clone(), cx);
            assert_eq!(
                panel
                    .variable_font_axis_editor
                    .as_ref()
                    .map(|editor| editor.tag.as_ref()),
                Some("wght")
            );
            assert!(
                panel
                    .variable_font_axis_scrub
                    .as_ref()
                    .is_some_and(|scrub| scrub.active)
            );
            assert!(panel.finish_variable_font_axis_scrub(true, false, window, cx));
        });
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag,
                phase: DesignPanelEditPhase::Begin,
                ..
            },
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag: preview_tag,
                phase: DesignPanelEditPhase::Preview,
                ..
            },
            DesignPanelAction::TypographyVariableAxisEditRequested {
                tag: commit_tag,
                phase: DesignPanelEditPhase::Commit,
                ..
            },
        ] if tag.as_ref() == "wght"
            && preview_tag.as_ref() == "wght"
            && commit_tag.as_ref() == "wght"
    ));
}

#[gpui::test]
fn variable_font_axis_range_revision_and_permission_changes_cancel_captured_target(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes = vec![DesignFontAxis::new(
        "wght", "Weight", 520., 100., 900., 400.,
    )];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let first_range = DesignPanelInspectionContext::single(
            node.clone(),
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        )
        .with_edit_mode(DesignPanelEditMode::Text)
        .expect("Text mode")
        .with_text_range_revision(7)
        .expect("exact range");
        panel.set_inspection_context(first_range, cx);
        assert!(panel.begin_variable_font_axis_edit("wght".into(), cx));

        let second_range = DesignPanelInspectionContext::single(
            node.clone(),
            DesignPanelParentLayout::Freeform,
            DesignPanelPermissions::editor(),
        )
        .with_edit_mode(DesignPanelEditMode::Text)
        .expect("Text mode")
        .with_text_range_revision(8)
        .expect("next exact range");
        panel.set_inspection_context(second_range, cx);
        assert!(panel.variable_font_axis_editor.is_none());

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.begin_variable_font_axis_edit("wght".into(), cx));
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::TypographyVariableAxisEditRequested {
                target: DesignTypographyTarget::SelectedTextRangeRevision(7),
                tag,
                phase: DesignPanelEditPhase::Begin,
                ..
            },
            DesignPanelAction::TypographyVariableAxisEditRequested {
                target: DesignTypographyTarget::SelectedTextRangeRevision(7),
                tag: cancel_tag,
                phase: DesignPanelEditPhase::Cancel,
                ..
            },
        ] if tag.as_ref() == "wght" && cancel_tag.as_ref() == "wght"
    ));
}

#[gpui::test]
fn variable_font_axis_gates_read_only_unavailable_bound_invalid_and_duplicate_tags(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .variable_axes = vec![
        DesignFontAxis::new("wght", "Weight", 520., 100., 900., 400.).read_only(),
        DesignFontAxis::new("opsz", "Optical size", 18., 9., 144., 14.)
            .unavailable("Font data is still loading"),
        DesignFontAxis::new("wdth", "Width", 100., 75., 125., 100.).with_binding(
            super::super::DesignFontAxisBinding::new("width-variable", "Width"),
        ),
        DesignFontAxis::new("bad", "Bad tag", 0., -1., 1., 0.),
        DesignFontAxis::new("GRAD", "Grade", 0., -100., 150., 0.),
        DesignFontAxis::new("GRAD", "Duplicate grade", 0., -100., 150., 0.),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        for tag in ["wght", "opsz", "wdth", "bad", "GRAD"] {
            assert!(!panel.begin_variable_font_axis_edit(tag.into(), cx));
            assert!(!panel.start_variable_font_axis_scrub(tag.into(), 0., 0., cx));
        }
        let typography = panel.node.typography.as_ref().expect("typography");
        drop(panel.render_type_settings_popover(typography, cx));
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn component_instance_child_hides_and_rejects_aspect_ratio_lock(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("nested", "Nested layer", DesignPanelNodeKind::Rectangle);
    node.is_component_instance_child = true;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.node_capability_allows_property(DesignPanelProperty::LockAspectRatio));
        assert!(!panel.property_is_editable(DesignPanelProperty::LockAspectRatio));
        panel.emit_property(
            DesignPanelProperty::LockAspectRatio,
            DesignPanelValue::Bool(true),
            cx,
        );
        drop(panel.render_layout(cx));
    });
    visual_cx.run_until_parked();

    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn authoritative_other_capabilities_gate_sections_properties_and_intents(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("plugin", "Plugin node", DesignPanelNodeKind::Other)
        .with_capabilities(
            DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Other)
                .with_sections([
                    DesignPanelSection::Position,
                    DesignPanelSection::Layout,
                    DesignPanelSection::Layer,
                    DesignPanelSection::Stroke,
                    DesignPanelSection::LayoutGrid,
                    DesignPanelSection::Export,
                ])
                .with_fill(false)
                .with_effects(false)
                .with_constraints(true)
                .with_layout_guides(true),
        );
    node.effects.push(DesignEffect::drop_shadow(
        DesignColor::BLACK,
        8.,
        0.,
        0.,
        2.,
    ));
    node.layout_grids.push(DesignLayoutGrid::uniform(
        8.,
        DesignColor::rgba(0xff, 0x00, 0x00, 0x20),
    ));
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.can_show_constraints());
        assert!(panel.render_fill(cx).is_none());
        assert!(panel.render_layout_grids(cx).is_some());
        assert!(
            !panel.property_is_editable(DesignPanelProperty::PaintVisible {
                collection: DesignPanelCollection::Fill,
                index: 0,
            })
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::EffectVisible(0)));
        assert!(panel.property_is_editable(DesignPanelProperty::HorizontalConstraint));
        assert!(panel.property_is_editable(DesignPanelProperty::LayoutGridColor(0)));

        panel.emit_add(DesignPanelCollection::Fill, cx);
        panel.emit_add(DesignPanelCollection::Effect, cx);
        panel.emit_property(
            DesignPanelProperty::PaintVisible {
                collection: DesignPanelCollection::Fill,
                index: 0,
            },
            DesignPanelValue::Bool(false),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::EffectVisible(0),
            DesignPanelValue::Bool(false),
            cx,
        );
        assert!(captured.borrow().is_empty());

        panel.emit_property(
            DesignPanelProperty::HorizontalConstraint,
            DesignPanelValue::Constraint(DesignConstraint::Center),
            cx,
        );
        panel.emit_add(DesignPanelCollection::LayoutGrid, cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::PropertyChangeRequested {
                node_id: "plugin".into(),
                property: DesignPanelProperty::HorizontalConstraint,
                value: DesignPanelValue::Constraint(DesignConstraint::Center),
            },
            DesignPanelAction::CollectionItemAddRequested {
                node_id: "plugin".into(),
                collection: DesignPanelCollection::LayoutGrid,
                target: DesignPaintTarget::WholeLayer,
            },
        ]
    );
}

#[gpui::test]
fn widget_profile_exposes_only_opaque_scene_node_controls(cx: &mut TestAppContext) {
    let mut widget = DesignPanelNode::new(
        "reference-widget",
        "Widget · Opaque object",
        DesignPanelNodeKind::Widget,
    )
    .with_capabilities(DesignPanelNodeCapabilities::for_node_kind(
        DesignPanelNodeKind::Widget,
    ));
    widget.width = 320.;
    widget.height = 180.;
    let (host, visual_cx) = setup(widget.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                widget,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Grid,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::Width,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(320.))
                        .read_only_with_reason("Widget dimensions are read-only in the Plugin API"),
                ),
                (
                    DesignPanelProperty::Height,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(180.))
                        .read_only_with_reason("Widget dimensions are read-only in the Plugin API"),
                ),
            ],
            cx,
        );
        let add_auto_layout = DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
            node_ids: vec!["reference-widget".into()],
        });
        panel.set_add_auto_layout_view_data(add_auto_layout, cx);

        assert!(panel.property_is_editable(DesignPanelProperty::X));
        assert!(panel.property_is_editable(DesignPanelProperty::Y));
        assert!(panel.property_is_editable(DesignPanelProperty::Visible));
        assert!(!panel.property_is_editable(DesignPanelProperty::Width));
        assert!(!panel.property_is_editable(DesignPanelProperty::Height));
        assert!(!panel.property_is_editable(DesignPanelProperty::Rotation));
        assert!(!panel.property_is_editable(DesignPanelProperty::LockAspectRatio));
        assert!(!panel.property_is_editable(DesignPanelProperty::Opacity));
        assert!(!panel.property_is_editable(DesignPanelProperty::BlendMode));
        assert!(!panel.property_is_editable(DesignPanelProperty::LayoutPositioning));
        assert!(!panel.property_is_editable(DesignPanelProperty::GridRowIndex));
        assert!(!panel.dimension_limits_are_applicable());
        assert!(panel.add_auto_layout_view_data_for_context().is_none());
        assert!(!panel.emit_add_auto_layout(cx));
        assert!(
            !panel.node_capability_allows_action(&DesignPanelAction::AddAutoLayoutRequested {
                target: panel.command_target(),
            })
        );
        assert!(
            !panel.node_capability_allows_action(&DesignPanelAction::ArrangeRequested {
                target: panel.command_target(),
                operation: DesignArrangeOperation::AlignLeft,
            })
        );
        assert!(
            !panel.node_capability_allows_action(&DesignPanelAction::TransformRequested {
                target: panel.command_target(),
                operation: DesignTransformOperation::FlipHorizontal,
            })
        );

        panel.emit_property(DesignPanelProperty::X, DesignPanelValue::Number(24.), cx);
        panel.emit_property(
            DesignPanelProperty::Visible,
            DesignPanelValue::Bool(false),
            cx,
        );
        for (property, value) in [
            (DesignPanelProperty::Width, DesignPanelValue::Number(640.)),
            (DesignPanelProperty::Height, DesignPanelValue::Number(360.)),
            (DesignPanelProperty::Rotation, DesignPanelValue::Number(15.)),
            (
                DesignPanelProperty::LockAspectRatio,
                DesignPanelValue::Bool(true),
            ),
            (DesignPanelProperty::Opacity, DesignPanelValue::Number(50.)),
            (
                DesignPanelProperty::LayoutPositioning,
                DesignPanelValue::LayoutPositioning(DesignLayoutPositioning::Absolute),
            ),
        ] {
            panel.emit_property(property, value, cx);
        }
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::PropertyChangeRequested {
                node_id: "reference-widget".into(),
                property: DesignPanelProperty::X,
                value: DesignPanelValue::Number(24.),
            },
            DesignPanelAction::PropertyChangeRequested {
                node_id: "reference-widget".into(),
                property: DesignPanelProperty::Visible,
                value: DesignPanelValue::Bool(false),
            },
        ]
    );
    assert!(visual_cx.debug_bounds("design-x").is_some());
    assert!(visual_cx.debug_bounds("design-y").is_some());
    assert!(visual_cx.debug_bounds("design-width").is_some());
    assert!(visual_cx.debug_bounds("design-height").is_some());
    assert!(
        visual_cx
            .debug_bounds("design-appearance-visible")
            .is_some()
    );
    for omitted in [
        "design-align-left",
        "design-rotation",
        "design-rotate-clockwise-90",
        "design-flip-horizontal",
        "design-flip-vertical",
        "design-lock-aspect-ratio",
        "design-opacity",
        "design-layout-positioning",
        "design-grid-row-index",
        "design-add-auto-layout",
    ] {
        assert!(
            visual_cx.debug_bounds(omitted).is_none(),
            "Widget must omit unsupported control {omitted}",
        );
    }
}

#[gpui::test]
fn page_context_emits_controlled_resource_intents_and_keeps_viewers_read_only(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("storybook-page", "Page", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let local_group = super::super::DesignLocalResourceGroup::new(
        "local-styles",
        "Local styles",
        super::super::DesignLocalResourceSource::Local,
        [super::super::DesignLocalResource::local(
            "local-paint",
            "Local paint",
            DesignLocalResourceKind::PaintStyle,
        )],
    );
    let library_group = super::super::DesignLocalResourceGroup::new(
        "brand-library",
        "Brand",
        super::super::DesignLocalResourceSource::library("brand", "Brand"),
        [super::super::DesignLocalResource::available(
            "library-paint",
            "Library paint",
            DesignLocalResourceKind::PaintStyle,
        )],
    );
    let local_selection = local_group.selection("local-paint");
    let library_selection = library_group.selection("library-paint");
    let page = DesignPageViewData::new(
        "storybook-page",
        super::super::DesignPageBackground::new(DesignColor::WHITE),
        super::super::DesignLocalResourceViewData::new([local_group, library_group]),
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(page.clone(), cx);
        panel.emit_page_background_edit(DesignColor::BLACK, DesignPanelEditPhase::Commit, cx);
        panel.emit_local_resource_browse(DesignLocalResourceCategory::Styles, cx);
        panel.emit_local_resource_open(local_selection.clone(), cx);
        panel.emit_local_resource_import(library_selection.clone(), cx);
        panel.emit_local_resource_create(DesignLocalResourceKind::PaintStyle, cx);

        assert_eq!(
            panel
                .page_view_data()
                .expect("controlled Page data")
                .background
                .color,
            DesignColor::WHITE
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::viewer()),
            cx,
        );
        panel.emit_page_background_edit(DesignColor::PURPLE, DesignPanelEditPhase::Commit, cx);
        panel.emit_local_resource_import(library_selection.clone(), cx);
        panel.emit_local_resource_create(DesignLocalResourceKind::TextStyle, cx);
        panel.emit_local_resource_browse(DesignLocalResourceCategory::Styles, cx);
        panel.emit_local_resource_open(local_selection, cx);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 7);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::PageBackgroundEditRequested {
            page_id,
            color,
            phase: DesignPanelEditPhase::Commit,
        } if page_id.as_ref() == "storybook-page" && *color == DesignColor::BLACK
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::LocalResourceBrowseRequested {
            category: DesignLocalResourceCategory::Styles,
            ..
        }
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::LocalResourceOpenRequested { resource, .. }
            if resource.group_id.as_ref() == "local-styles"
                && resource.resource_id.as_ref() == "local-paint"
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::LocalResourceImportRequested { resource, .. }
            if resource.group_id.as_ref() == "brand-library"
                && resource.resource_id.as_ref() == "library-paint"
    ));
    assert!(matches!(
        &captured[4],
        DesignPanelAction::LocalResourceCreateRequested {
            kind: DesignLocalResourceKind::PaintStyle,
            ..
        }
    ));
    assert!(matches!(
        &captured[5],
        DesignPanelAction::LocalResourceBrowseRequested { .. }
    ));
    assert!(matches!(
        &captured[6],
        DesignPanelAction::LocalResourceOpenRequested { .. }
    ));
}

#[gpui::test]
fn page_local_styles_emit_exact_commands_and_gate_editor_viewer_permissions(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("storybook-page", "Page", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let style_a = DesignLocalStyleTarget::new(
        "storybook-page",
        DesignLocalStyleKind::Color,
        "color-a",
        None,
        0,
    );
    let disabled_child = DesignLocalStyleTarget::new(
        "storybook-page",
        DesignLocalStyleKind::Text,
        "text-disabled",
        Some("folder-disabled".into()),
        0,
    );
    let destination = DesignLocalStyleInsertion::new(
        DesignLocalStyleKind::Color,
        None,
        2,
        Some("color-b".into()),
        None,
    );
    let local_styles = DesignPageLocalStylesViewData::for_page(
        "storybook-page",
        [
            super::super::DesignLocalStyleSection::new(
                DesignLocalStyleKind::Color,
                [
                    DesignLocalStyleEntry::style(super::super::DesignLocalStyleItem::new(
                        "color-a",
                        "A",
                        DesignLocalStylePreview::Color(vec![DesignPaint::solid(DesignColor::BLUE)]),
                    )),
                    DesignLocalStyleEntry::style(super::super::DesignLocalStyleItem::new(
                        "color-b",
                        "B",
                        DesignLocalStylePreview::Color(vec![DesignPaint::solid(
                            DesignColor::PURPLE,
                        )]),
                    )),
                ],
            ),
            super::super::DesignLocalStyleSection::new(
                DesignLocalStyleKind::Text,
                [DesignLocalStyleEntry::folder(
                    "folder-disabled",
                    "Managed",
                    [DesignLocalStyleEntry::style(
                        super::super::DesignLocalStyleItem::new(
                            "text-disabled",
                            "Managed body",
                            DesignLocalStylePreview::Text(
                                super::super::DesignTypographyStyle::new(
                                    "text-disabled",
                                    "Managed body",
                                    "Inter",
                                    "Regular",
                                    16.,
                                ),
                            ),
                        ),
                    )],
                )
                .disabled("Managed by a library")],
            ),
        ],
    );
    let page = DesignPageViewData::new(
        "storybook-page",
        super::super::DesignPageBackground::new(DesignColor::WHITE),
        super::super::DesignLocalResourceViewData::default(),
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(page, cx);
        panel.set_page_local_styles_view_data(local_styles, cx);

        for command in [
            DesignLocalStyleCommand::Edit,
            DesignLocalStyleCommand::GoToDefinition,
            DesignLocalStyleCommand::Copy,
            DesignLocalStyleCommand::Duplicate,
        ] {
            panel.emit_local_style_command(style_a.clone(), command, cx);
        }
        panel.emit_local_style_create(DesignLocalStyleKind::LayoutGuide, None, cx);
        panel.emit_local_style_folder_create(DesignLocalStyleKind::Effect, Vec::new(), cx);
        panel.emit_local_styles_delete(vec![style_a.clone()], cx);
        panel.emit_local_styles_move(vec![style_a.clone()], destination.clone(), cx);

        panel.emit_local_style_command(
            disabled_child.clone(),
            DesignLocalStyleCommand::GoToDefinition,
            cx,
        );
        panel.emit_local_styles_delete(vec![disabled_child], cx);
        panel.emit_local_style_command(
            DesignLocalStyleTarget {
                expected_index: 1,
                ..style_a.clone()
            },
            DesignLocalStyleCommand::Edit,
            cx,
        );
        panel.emit_local_style_command(
            DesignLocalStyleTarget {
                page_id: "stale-page".into(),
                ..style_a.clone()
            },
            DesignLocalStyleCommand::Edit,
            cx,
        );
        panel.emit_local_style_command(
            DesignLocalStyleTarget {
                kind: DesignLocalStyleKind::Text,
                ..style_a.clone()
            },
            DesignLocalStyleCommand::Edit,
            cx,
        );
        panel.emit_local_styles_move(
            vec![style_a.clone()],
            DesignLocalStyleInsertion {
                expected_before_id: Some("color-a".into()),
                ..destination.clone()
            },
            cx,
        );
        panel.emit_local_styles_delete(vec![style_a.clone(), style_a.clone()], cx);
        panel.emit_local_styles_move(
            vec![style_a.clone(), style_a.clone()],
            destination.clone(),
            cx,
        );

        panel.emit_variables_view_open(cx);
        panel.set_variables_entry_point(
            DesignVariablesEntryPoint::LegacyRightSidebar {
                disabled_reason: Some("Variables are managed".into()),
            },
            cx,
        );
        panel.emit_variables_view_open(cx);
        panel.set_variables_entry_point(
            DesignVariablesEntryPoint::LegacyRightSidebar {
                disabled_reason: None,
            },
            cx,
        );
        panel.emit_variables_view_open(cx);

        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::viewer()),
            cx,
        );
        panel.emit_local_style_command(style_a.clone(), DesignLocalStyleCommand::Edit, cx);
        panel.emit_local_style_command(style_a.clone(), DesignLocalStyleCommand::Duplicate, cx);
        panel.emit_local_style_create(DesignLocalStyleKind::Color, None, cx);
        panel.emit_local_styles_delete(vec![style_a.clone()], cx);
        panel.emit_local_styles_move(vec![style_a.clone()], destination, cx);
        panel.emit_local_style_command(
            style_a.clone(),
            DesignLocalStyleCommand::GoToDefinition,
            cx,
        );
        panel.emit_local_style_command(style_a.clone(), DesignLocalStyleCommand::Copy, cx);
        panel.emit_variables_view_open(cx);

        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::restricted_viewer()),
            cx,
        );
        panel.emit_local_style_command(style_a.clone(), DesignLocalStyleCommand::Copy, cx);
        panel.emit_local_style_command(style_a, DesignLocalStyleCommand::GoToDefinition, cx);
        panel.emit_variables_view_open(cx);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::LocalStyleCommandRequested { .. }))
            .count(),
        7,
        "editor gets four commands, viewer gets Go-to/Copy, restricted gets Go-to"
    );
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::LocalStyleCreateRequested { .. }))
            .count(),
        1
    );
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::LocalStyleFolderCreateRequested { .. }
            ))
            .count(),
        1
    );
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::LocalStylesDeleteRequested { .. }))
            .count(),
        1
    );
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::LocalStylesMoveRequested { .. }))
            .count(),
        1
    );
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::VariablesViewOpenRequested { .. }))
            .count(),
        3,
        "explicit compatibility navigation is permission-independent"
    );
}

#[gpui::test]
fn page_local_style_folders_collapse_and_prune_stale_presentation_state(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("storybook-page", "Page", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let page = DesignPageViewData::new(
        "storybook-page",
        super::super::DesignPageBackground::new(DesignColor::WHITE),
        super::super::DesignLocalResourceViewData::default(),
    );
    let nested = DesignPageLocalStylesViewData::for_page(
        "storybook-page",
        [super::super::DesignLocalStyleSection::new(
            DesignLocalStyleKind::Text,
            [DesignLocalStyleEntry::folder(
                "folder-type",
                "Typography",
                [DesignLocalStyleEntry::style(
                    super::super::DesignLocalStyleItem::new(
                        "text-body",
                        "Body",
                        DesignLocalStylePreview::Text(super::super::DesignTypographyStyle::new(
                            "text-body",
                            "Body",
                            "Inter",
                            "Regular",
                            16.,
                        )),
                    ),
                )],
            )],
        )],
    );
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::viewer()),
            cx,
        );
        panel.set_page_view_data(page, cx);
        panel.set_page_local_styles_view_data(nested, cx);
    });
    visual_cx.run_until_parked();

    assert!(
        visual_cx
            .debug_bounds("design-local-style-text-body")
            .is_some()
    );
    let toggle = visual_cx
        .debug_bounds("design-local-style-folder-folder-type-toggle")
        .expect("folder disclosure");
    let color_before = visual_cx
        .debug_bounds("design-local-style-Color-root-drop")
        .expect("following canonical family header");
    visual_cx.simulate_click(toggle.center(), Modifiers::none());
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert!(
            panel.collapsed_local_style_folders.contains("folder-type"),
            "folder disclosure button must update transient state"
        );
    });
    visual_cx.update(|window, _| window.refresh());
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-local-style-Color-root-drop")
            .expect("following family remains rendered")
            .top()
            < color_before.top(),
        "collapsing the folder must remove its recursive child from layout"
    );
    panel.update(visual_cx, |panel, cx| {
        panel.set_page_local_styles_view_data(
            DesignPageLocalStylesViewData::for_page("storybook-page", []),
            cx,
        );
        assert!(panel.collapsed_local_style_folders.is_empty());
    });
}

#[gpui::test]
fn page_local_style_drag_between_folders_emits_exact_neighbor_checked_move(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("storybook-page", "Page", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let page = DesignPageViewData::new(
        "storybook-page",
        super::super::DesignPageBackground::new(DesignColor::WHITE),
        super::super::DesignLocalResourceViewData::default(),
    );
    let local_styles = DesignPageLocalStylesViewData::for_page(
        "storybook-page",
        [super::super::DesignLocalStyleSection::new(
            DesignLocalStyleKind::Text,
            [
                DesignLocalStyleEntry::folder(
                    "folder-source",
                    "Source",
                    [DesignLocalStyleEntry::style(
                        super::super::DesignLocalStyleItem::new(
                            "text-body",
                            "Body",
                            DesignLocalStylePreview::Text(
                                super::super::DesignTypographyStyle::new(
                                    "text-body",
                                    "Body",
                                    "Inter",
                                    "Regular",
                                    16.,
                                ),
                            ),
                        ),
                    )],
                ),
                DesignLocalStyleEntry::folder("folder-destination", "Destination", []),
            ],
        )],
    );
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(page, cx);
        panel.set_page_local_styles_view_data(local_styles, cx);
    });
    visual_cx.run_until_parked();

    let source = visual_cx
        .debug_bounds("design-local-style-text-body")
        .expect("source style row");
    let destination = visual_cx
        .debug_bounds("design-local-style-folder-folder-destination")
        .expect("destination folder row");
    let source_point = gpui::point(source.left() + px(8.), source.top() + px(8.));
    let destination_point = destination.center();
    visual_cx.simulate_mouse_move(source_point, None, Modifiers::none());
    visual_cx.simulate_mouse_down(source_point, MouseButton::Left, Modifiers::none());
    visual_cx.simulate_mouse_move(
        source_point + gpui::point(px(8.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    visual_cx.simulate_mouse_move(destination_point, MouseButton::Left, Modifiers::none());
    visual_cx.simulate_mouse_up(destination_point, MouseButton::Left, Modifiers::none());
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().last(),
        Some(&DesignPanelAction::LocalStylesMoveRequested {
            targets: vec![DesignLocalStyleTarget::new(
                "storybook-page",
                DesignLocalStyleKind::Text,
                "text-body",
                Some("folder-source".into()),
                0,
            )],
            destination: DesignLocalStyleInsertion::new(
                DesignLocalStyleKind::Text,
                Some("folder-destination".into()),
                0,
                None,
                None,
            ),
        })
    );
}

#[gpui::test]
fn explicit_variable_modes_validate_page_and_scene_targets_and_viewer_gates(
    cx: &mut TestAppContext,
) {
    fn collection() -> super::super::DesignVariableModeCollection {
        super::super::DesignVariableModeCollection::new(
            "theme",
            "Theme",
            super::super::DesignLocalResourceSource::Local,
            [
                super::super::DesignVariableMode::new("light", "Light"),
                super::super::DesignVariableMode::new("dark", "Dark"),
            ],
            "light",
            "dark",
        )
        .explicit("dark")
    }

    let node = DesignPanelNode::new("scene-node", "Scene node", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let page_target = DesignPanelTarget::Page {
        page_id: "storybook-page".into(),
    };
    let scene_target = DesignPanelTarget::Nodes {
        node_ids: vec!["scene-node".into()],
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(
            DesignPageViewData::new(
                "storybook-page",
                super::super::DesignPageBackground::new(DesignColor::WHITE),
                super::super::DesignLocalResourceViewData::default(),
            ),
            cx,
        );
        panel.set_variable_mode_view_data(
            DesignVariableModeViewData::new(page_target.clone(), [collection()]),
            cx,
        );
        panel.emit_variable_mode_apply("theme".into(), "light".into(), cx);
        panel.emit_variable_mode_clear("theme".into(), "dark".into(), cx);

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_variable_mode_view_data(
            DesignVariableModeViewData::new(scene_target.clone(), [collection()]),
            cx,
        );
        panel.emit_variable_mode_apply("theme".into(), "light".into(), cx);
        panel.emit_variable_mode_clear("theme".into(), "dark".into(), cx);
        panel.emit_variable_mode_apply("theme".into(), "missing".into(), cx);

        panel.set_variable_mode_view_data(
            DesignVariableModeViewData::new(page_target.clone(), [collection()]),
            cx,
        );
        panel.emit_variable_mode_apply("theme".into(), "light".into(), cx);

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_variable_mode_view_data(
            DesignVariableModeViewData::new(scene_target.clone(), [collection()]),
            cx,
        );
        panel.emit_variable_mode_apply("theme".into(), "light".into(), cx);
        panel.emit_variable_mode_clear("theme".into(), "dark".into(), cx);
        assert_eq!(
            panel
                .variable_mode_view_data()
                .and_then(|view_data| view_data.collection("theme"))
                .and_then(|collection| collection.explicit_mode_id.as_ref())
                .map(|mode_id| mode_id.as_ref()),
            Some("dark")
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 4);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::VariableModeApplyRequested { target, mode_id, .. }
            if target == &page_target && mode_id.as_ref() == "light"
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::VariableModeClearRequested {
            target,
            explicit_mode_id,
            ..
        } if target == &page_target && explicit_mode_id.as_ref() == "dark"
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::VariableModeApplyRequested { target, .. }
            if target == &scene_target
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::VariableModeClearRequested { target, .. }
            if target == &scene_target
    ));
}

#[gpui::test]
fn host_owned_selection_header_is_authoritative_and_emits_typed_leaf_commands(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let view_data = DesignSelectionHeaderViewData::new(
        "Host rectangle",
        [DesignSelectionHeaderControl::new(
            "plugin-command",
            DesignSelectionHeaderControlKind::HostDefined,
        )
        .with_tooltip("Run host command")],
    )
    .with_title_menu(super::super::DesignSelectionHeaderMenu::new(
        "host-title-menu",
        [super::super::DesignSelectionHeaderMenuItem::new(
            "convert-special",
            "Convert to special",
            DesignSelectionHeaderCommand::TitleMenuItem {
                item_id: "convert-special".into(),
            },
        )],
    ));

    panel.update(visual_cx, |panel, cx| {
        panel.set_selection_header_view_data(view_data.clone(), cx);
        assert_eq!(panel.selection_header_view_data(), Some(&view_data));
        assert_eq!(panel.resolved_selection_header_view_data(), view_data);
        drop(panel.render_header(cx));
        let target = panel
            .current_selection_header_target()
            .expect("single selection target");
        panel.emit_selection_header_command_for_target(
            target.clone(),
            DesignSelectionHeaderCommand::HostDefined {
                command_id: "plugin-command".into(),
            },
            DesignSelectionHeaderCommandAccess::EditRequired,
            cx,
        );
        panel.emit_selection_header_command_for_target(
            target,
            DesignSelectionHeaderCommand::TitleMenuItem {
                item_id: "convert-special".into(),
            },
            DesignSelectionHeaderCommandAccess::EditRequired,
            cx,
        );
    });

    assert_eq!(
        captured_actions.borrow().as_slice(),
        [
            DesignPanelAction::SelectionHeaderCommandRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                command: DesignSelectionHeaderCommand::HostDefined {
                    command_id: "plugin-command".into(),
                },
            },
            DesignPanelAction::SelectionHeaderCommandRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                command: DesignSelectionHeaderCommand::TitleMenuItem {
                    item_id: "convert-special".into(),
                },
            },
        ]
    );
}

#[gpui::test]
fn direct_selection_header_tooltips_preserve_labels_and_disabled_reasons(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let label = SharedString::from("Run host command");
    let disabled_reason = SharedString::from("Unavailable for this layer");

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.selection_header_tooltip(
                &label,
                true,
                None,
                Some(DesignSelectionHeaderCommandAccess::EditRequired),
            ),
            label,
        );
        assert_eq!(
            panel.selection_header_tooltip(
                &label,
                false,
                Some(&disabled_reason),
                Some(DesignSelectionHeaderCommandAccess::EditRequired),
            ),
            "Run host command — Unavailable for this layer",
        );
        drop(
            panel.render_selection_header_control(
                DesignSelectionHeaderControl::new(
                    "plugin-command",
                    DesignSelectionHeaderControlKind::HostDefined,
                )
                .with_tooltip(label.clone()),
                cx,
            ),
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert_eq!(
            panel.selection_header_tooltip(
                &label,
                true,
                None,
                Some(DesignSelectionHeaderCommandAccess::EditRequired),
            ),
            "Run host command — View only",
        );
    });
}

#[gpui::test]
fn mixed_selection_header_fallback_is_kind_neutral_and_order_independent(cx: &mut TestAppContext) {
    use super::super::DesignSelectionHeaderControlKind as Kind;

    let text = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let ellipse = DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(text.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(text.clone(), ellipse.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        let header = panel.resolved_selection_header_view_data();
        assert_eq!(header.title, "2 layers");
        assert!(header.title_menu.is_none());
        assert_eq!(
            header
                .primary_controls
                .iter()
                .map(|control| control.kind)
                .collect::<Vec<_>>(),
            vec![Kind::CreateComponent],
        );
        assert!(header.overflow_controls.is_empty());

        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(ellipse, text),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            panel.resolved_selection_header_view_data(),
            header,
            "reordering a mixed selection must not swap in a first-kind preset",
        );
    });
}

#[gpui::test]
fn selection_header_permissions_gate_edits_but_keep_viewer_selection_commands(
    cx: &mut TestAppContext,
) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.selection_header_command_enabled_with_access(
            true,
            DesignSelectionHeaderCommand::CreateComponent.default_access(),
        ));
        assert!(panel.selection_header_command_enabled_with_access(
            true,
            DesignSelectionHeaderCommand::SelectMatchingLayers.default_access(),
        ));
        let target = panel
            .current_selection_header_target()
            .expect("multiple selection target");
        panel.emit_selection_header_command_for_target(
            target.clone(),
            DesignSelectionHeaderCommand::CreateComponent,
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            cx,
        );
        panel.emit_selection_header_command_for_target(
            target,
            DesignSelectionHeaderCommand::SelectMatchingLayers,
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            cx,
        );
    });

    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::SelectionHeaderCommandRequested {
            target: DesignPanelTarget::Nodes {
                node_ids: vec!["first".into(), "second".into()],
            },
            command: DesignSelectionHeaderCommand::SelectMatchingLayers,
        }]
    );
}

#[gpui::test]
fn selection_header_host_access_allows_viewer_safe_custom_commands(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let command = DesignSelectionHeaderCommand::HostDefined {
        command_id: "inspect-plugin-data".into(),
    };
    let title_command = DesignSelectionHeaderCommand::TitleMenuItem {
        item_id: "inspect-layer-type".into(),
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(
            !panel.selection_header_command_enabled_with_access(true, command.default_access(),)
        );
        assert!(panel.selection_header_command_enabled_with_access(
            true,
            DesignSelectionHeaderCommandAccess::ViewerSafe,
        ));
        let target = panel
            .current_selection_header_target()
            .expect("single selection target");
        panel.emit_selection_header_command_for_target(
            target.clone(),
            command.clone(),
            command.default_access(),
            cx,
        );
        panel.emit_selection_header_command_for_target(
            target.clone(),
            command.clone(),
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            cx,
        );
        panel.emit_selection_header_command_for_target(
            target,
            title_command.clone(),
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            cx,
        );
    });

    assert_eq!(
        captured_actions.borrow().as_slice(),
        [
            DesignPanelAction::SelectionHeaderCommandRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                command,
            },
            DesignPanelAction::SelectionHeaderCommandRequested {
                target: DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                command: title_command,
            },
        ]
    );
}

#[gpui::test]
fn selection_header_data_and_events_are_bound_to_the_exact_selection_target(
    cx: &mut TestAppContext,
) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
    let third = DesignPanelNode::new("third", "Third", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let stale_target = DesignPanelTarget::Nodes {
        node_ids: vec!["first".into(), "second".into()],
    };
    let view_data = DesignSelectionHeaderViewData::new(
        "Host AB",
        [DesignSelectionHeaderControl::new(
            "inspect",
            DesignSelectionHeaderControlKind::HostDefined,
        )
        .viewer_safe()],
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_selection_header_view_data(view_data.clone(), cx);
        assert_eq!(panel.selection_header_view_data(), Some(&view_data));

        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), third.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(panel.selection_header_view_data().is_none());
        assert_ne!(panel.resolved_selection_header_view_data().title, "Host AB");
        panel.set_selection_header_view_data_for_target(
            stale_target.clone(),
            view_data.clone(),
            cx,
        );
        assert!(panel.selection_header_view_data().is_none());
        panel.emit_selection_header_command_for_target(
            stale_target.clone(),
            DesignSelectionHeaderCommand::HostDefined {
                command_id: "inspect".into(),
            },
            DesignSelectionHeaderCommandAccess::ViewerSafe,
            cx,
        );

        panel.set_selection_header_view_data(view_data.clone(), cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::with_remaining(
                    first.clone(),
                    third.clone(),
                    [second],
                ),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(panel.selection_header_view_data().is_none());

        panel.set_selection_header_view_data_for_target(
            DesignPanelTarget::Nodes {
                node_ids: vec!["first".into(), "third".into()],
            },
            view_data.clone(),
            cx,
        );
        assert!(panel.selection_header_view_data().is_none());
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, third),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert_eq!(panel.selection_header_view_data(), Some(&view_data));
    });

    assert!(captured_actions.borrow().is_empty());
}

#[gpui::test]
fn host_defined_primary_controls_with_items_use_the_menu_path(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("other", "Other", DesignPanelNodeKind::Other);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let control = DesignSelectionHeaderControl::new(
        "plugin-menu",
        DesignSelectionHeaderControlKind::HostDefined,
    )
    .with_menu_items([super::super::DesignSelectionHeaderMenuItem::new(
        "inspect",
        "Inspect metadata",
        DesignSelectionHeaderCommand::HostDefined {
            command_id: "inspect".into(),
        },
    )
    .viewer_safe()]);

    panel.update(visual_cx, |panel, cx| {
        assert!(control.is_menu());
        assert!(control.command().is_none());
        drop(panel.render_selection_header_control(control.clone(), cx));
    });
}

#[gpui::test]
fn selection_change_and_escape_close_transient_header_menus(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_selection_header_view_data(
                DesignSelectionHeaderViewData::for_node_kind(DesignPanelNodeKind::Rectangle),
                cx,
            );
            panel.selection_header_overlay = Some(SelectionHeaderOverlay::More);
            panel.focus_handle.focus(window, cx);
            cx.notify();
        });
    });
    visual_cx.run_until_parked();
    visual_cx.simulate_keystrokes("escape");
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).selection_header_overlay.clone()),
        None
    );

    panel.update(visual_cx, |panel, cx| {
        panel.selection_header_overlay = Some(SelectionHeaderOverlay::Title);
        panel.set_node(
            DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse),
            cx,
        );
        assert!(panel.selection_header_view_data().is_none());
        assert!(panel.selection_header_overlay.is_none());
    });
}

#[test]
fn rotation_display_is_normalized_to_figmas_signed_range() {
    assert_eq!(normalize_rotation(0.), 0.);
    assert_eq!(normalize_rotation(190.), -170.);
    assert_eq!(normalize_rotation(-190.), 170.);
    assert_eq!(normalize_rotation(540.), -180.);
}

#[test]
fn layout_guide_count_and_color_inputs_are_typed() {
    assert_eq!(
        parse_design_color("#F24"),
        Some(DesignColor::rgb(0xff, 0x22, 0x44))
    );
    assert_eq!(
        parse_design_color("4C86F780"),
        Some(DesignColor::rgba(0x4c, 0x86, 0xf7, 0x80))
    );
    assert_eq!(parse_design_color("invalid"), None);
    assert_eq!(
        DesignLayoutGridCount::number(0),
        DesignLayoutGridCount::Number(1)
    );
    assert_eq!(DesignLayoutGridCount::Auto.label().as_ref(), "Auto");
}

#[test]
fn font_size_cell_agrees_with_the_style_picker_formatter() {
    // The typography section's font-size cell and the typography style picker
    // display the same host value through the shared compact formatter.
    assert_eq!(format_number(12.75), "12.75");
    assert_eq!(format_number(12.), "12");
}

#[gpui::test]
fn native_shape_controls_use_discriminated_values_and_read_only_table_counts(
    cx: &mut TestAppContext,
) {
    let mut ellipse = DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse);
    ellipse.shape_geometry = DesignShapeGeometry::Ellipse(DesignArcData::new(
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        0.25,
    ));
    let (host, visual_cx) = setup(ellipse, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::ArcStartingAngle),
            Some(DesignPanelValue::AngleRadians(std::f32::consts::FRAC_PI_2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::ArcSweep),
            Some(DesignPanelValue::AngleRadians(std::f32::consts::FRAC_PI_2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::ArcEndingAngle),
            Some(DesignPanelValue::AngleRadians(std::f32::consts::PI))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::ArcInnerRadius),
            Some(DesignPanelValue::Ratio(0.25))
        );

        let boolean =
            DesignPanelNode::new("boolean", "Union", DesignPanelNodeKind::BooleanOperation);
        panel.set_node(boolean, cx);
        assert!(
            !panel.property_is_editable(DesignPanelProperty::BooleanOperation),
            "canonical Boolean operations live in the selected-node header"
        );
        let options = panel
            .property_options(DesignPanelProperty::BooleanOperation)
            .expect("Boolean options");
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.clone())
                .collect::<Vec<_>>(),
            DesignBooleanOperation::ALL
                .into_iter()
                .map(DesignPanelValue::BooleanOperation)
                .collect::<Vec<_>>()
        );

        let table = DesignPanelNode::new("table", "Table", DesignPanelNodeKind::Table);
        panel.set_node(table, cx);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TableRows),
            Some(DesignPanelValue::Integer(4))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TableColumns),
            Some(DesignPanelValue::Integer(3))
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::TableRows));
        assert!(!panel.property_is_editable(DesignPanelProperty::TableColumns));
    });
}

#[gpui::test]
fn native_shape_rows_render_inside_design_and_draw_appearance(cx: &mut TestAppContext) {
    let polygon = DesignPanelNode::new("polygon", "Polygon", DesignPanelNodeKind::Polygon);
    let (host, visual_cx) = setup(polygon, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.expanded_sections.clear();
        panel.expanded_sections.insert(DesignPanelSection::Layer);
        cx.notify();
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-polygon-count").is_some());

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("star", "Star", DesignPanelNodeKind::Star),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-star-points").is_some());
    assert!(visual_cx.debug_bounds("design-star-inner-radius").is_some());

    let mut partial_arc =
        DesignPanelNode::new("partial-arc", "Partial arc", DesignPanelNodeKind::Ellipse);
    partial_arc.shape_geometry = DesignShapeGeometry::Ellipse(DesignArcData::new(
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        0.25,
    ));
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(partial_arc.clone(), cx);
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-arc-start").is_some());
    assert!(visual_cx.debug_bounds("design-arc-sweep").is_some());
    assert!(visual_cx.debug_bounds("design-arc-ratio").is_some());
    assert!(
        visual_cx.debug_bounds("design-arc-end").is_none(),
        "absolute ending angle is compatibility-only"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-arc-start").is_none());
    assert!(visual_cx.debug_bounds("design-arc-sweep").is_none());
    assert!(visual_cx.debug_bounds("design-arc-ratio").is_none());

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(partial_arc, cx);
        panel.set_workspace_mode(DesignPanelWorkspaceMode::Draw, cx);
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-arc-start").is_some());
    assert!(visual_cx.debug_bounds("design-arc-sweep").is_some());
    assert!(visual_cx.debug_bounds("design-arc-ratio").is_some());
}

#[gpui::test]
fn boolean_property_is_legacy_geometry_only(cx: &mut TestAppContext) {
    let canonical = DesignPanelNode::new("boolean", "Union", DesignPanelNodeKind::BooleanOperation);
    let (host, visual_cx) = setup(canonical, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.node.supports_section(DesignPanelSection::Geometry));
        assert!(!panel.property_is_editable(DesignPanelProperty::BooleanOperation));
        assert!(panel.render_geometry(cx).is_none());

        let legacy =
            DesignPanelNode::new("boolean", "Union", DesignPanelNodeKind::BooleanOperation)
                .with_capabilities(
                    DesignPanelNodeCapabilities::for_node_kind(
                        DesignPanelNodeKind::BooleanOperation,
                    )
                    .with_sections([
                        DesignPanelSection::Position,
                        DesignPanelSection::Geometry,
                        DesignPanelSection::Export,
                    ]),
                );
        panel.set_node(legacy, cx);
        assert!(panel.property_is_editable(DesignPanelProperty::BooleanOperation));
        assert!(panel.render_geometry(cx).is_some());
    });
}

#[gpui::test]
fn canonical_mask_section_omits_use_as_mask_but_legacy_geometry_retains_it(
    cx: &mut TestAppContext,
) {
    let canonical = DesignPanelNode::new("mask", "Mask", DesignPanelNodeKind::Mask);
    let (host, visual_cx) = setup(canonical, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.expanded_sections.clear();
        panel.expanded_sections.insert(DesignPanelSection::Mask);
        panel.set_additional_labels(true, cx);
        assert!(panel.node.supports_section(DesignPanelSection::Mask));
        assert!(!panel.node.supports_section(DesignPanelSection::Geometry));
        assert!(panel.render_mask(cx).is_some());
        assert!(panel.render_geometry(cx).is_none());
        cx.notify();
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-mask-type-additional-label")
            .is_some()
    );
    assert!(
        visual_cx.debug_bounds("design-is-mask").is_none(),
        "Use as mask is a selected-node header action canonically"
    );

    panel.update(visual_cx, |panel, cx| {
        let legacy = DesignPanelNode::new("mask", "Mask", DesignPanelNodeKind::Mask)
            .with_capabilities(
                DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Mask)
                    .with_sections([
                        DesignPanelSection::Position,
                        DesignPanelSection::Geometry,
                        DesignPanelSection::Export,
                    ]),
            );
        panel.set_node(legacy, cx);
        panel.expanded_sections.clear();
        panel.expanded_sections.insert(DesignPanelSection::Geometry);
        panel.set_additional_labels(true, cx);
        assert!(!panel.node.supports_section(DesignPanelSection::Mask));
        assert!(panel.render_geometry(cx).is_some());
        cx.notify();
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.debug_bounds("design-is-mask").is_some());
    assert!(
        visual_cx
            .debug_bounds("design-mask-type-additional-label")
            .is_some()
    );
}

#[gpui::test]
fn ellipse_editors_display_degrees_and_percent_but_emit_canonical_units(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse);
    node.shape_geometry = DesignShapeGeometry::Ellipse(DesignArcData::new(
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
        0.25,
    ));
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::ArcStartingAngle,
                DesignPanelValue::AngleRadians(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("180", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::ArcSweep,
                DesignPanelValue::AngleRadians(0.),
                window,
                cx,
            );
        });
    });
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("270", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::ArcInnerRadius,
                DesignPanelValue::Ratio(0.),
                window,
                cx,
            );
        });
    });
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("75", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });

    let captured_actions = captured_actions.borrow();
    assert!(captured_actions.iter().any(|action| match action {
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::ArcStartingAngle,
            value: DesignPanelValue::AngleRadians(value),
            phase: DesignPanelEditPhase::Commit,
            ..
        } => (*value - std::f32::consts::PI).abs() < f32::EPSILON,
        _ => false,
    }));
    assert!(captured_actions.iter().any(|action| match action {
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::ArcSweep,
            value: DesignPanelValue::AngleRadians(value),
            phase: DesignPanelEditPhase::Commit,
            ..
        } => (*value - 3. * std::f32::consts::FRAC_PI_2).abs() < f32::EPSILON,
        _ => false,
    }));
    assert!(captured_actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::ArcInnerRadius,
            value: DesignPanelValue::Ratio(0.75),
            phase: DesignPanelEditPhase::Commit,
            ..
        }
    )));
}

#[gpui::test]
fn section_visibility_is_independent_from_rotation_and_layer_appearance(cx: &mut TestAppContext) {
    let section = DesignPanelNode::new("section", "Checkout", DesignPanelNodeKind::Section);
    let (host, visual_cx) = setup(section, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let hostile_parent = panel.node.clone();
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                hostile_parent,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Horizontal,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
                node_ids: vec!["section".into()],
            }),
            cx,
        );
        assert!(panel.node.supports_visibility());
        assert!(!panel.node.supports_transforms());
        assert!(!panel.node.supports_auto_layout_child());
        assert!(!panel.node.supports_add_auto_layout());
        assert!(panel.property_is_editable(DesignPanelProperty::Visible));
        assert!(!panel.property_is_editable(DesignPanelProperty::Rotation));
        assert!(!panel.property_is_editable(DesignPanelProperty::Opacity));
        assert!(!panel.property_is_editable(DesignPanelProperty::BlendMode));
        assert!(!panel.property_is_editable(DesignPanelProperty::LayoutPositioning));
        assert!(!panel.dimension_limits_are_applicable());
        assert!(panel.add_auto_layout_view_data_for_context().is_none());
        assert!(!panel.emit_add_auto_layout(cx));
        panel.emit_property(
            DesignPanelProperty::Visible,
            DesignPanelValue::Bool(false),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::Rotation,
            DesignPanelValue::Number(15.),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(
        visual_cx
            .debug_bounds("design-appearance-visible")
            .is_some(),
        "Section keeps Figma's eye control without inheriting opacity/blend"
    );
    for unsupported in [
        "design-rotation",
        "design-rotate-clockwise-90",
        "design-flip-horizontal",
        "design-flip-vertical",
        "design-opacity",
        "design-layout-positioning",
        "design-add-auto-layout",
    ] {
        assert!(
            visual_cx.debug_bounds(unsupported).is_none(),
            "Section must omit unsupported control {unsupported}",
        );
    }
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PropertyChangeRequested {
            node_id: "section".into(),
            property: DesignPanelProperty::Visible,
            value: DesignPanelValue::Bool(false),
        }]
    );
}

#[gpui::test]
fn section_and_transform_intent_guards_reject_stale_host_echoes(cx: &mut TestAppContext) {
    let mut section = DesignPanelNode::new("section", "Checkout", DesignPanelNodeKind::Section);
    section
        .section
        .as_mut()
        .expect("Section properties")
        .dev_status =
        Some(DesignSectionDevStatus::new(DesignSectionDevStatusKind::ReadyForDev).changed(true));
    let (host, visual_cx) = setup(section, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let share = DesignPanelAction::SectionShareRequested {
            node_id: "section".into(),
        };
        let resolve = DesignPanelAction::SectionResolveChangedStatusRequested {
            node_id: "section".into(),
        };
        assert!(panel.node_capability_allows_action(&share));
        assert!(panel.node_capability_allows_action(&resolve));
        assert!(
            !panel.node_capability_allows_action(&DesignPanelAction::SectionShareRequested {
                node_id: "stale-section".into(),
            })
        );

        {
            let section = panel.node.section.as_mut().expect("Section properties");
            section.capabilities.share = false;
            section.capabilities.resolve_changed_status = false;
        }
        assert!(!panel.node_capability_allows_action(&share));
        assert!(!panel.node_capability_allows_action(&resolve));
        {
            let section = panel.node.section.as_mut().expect("Section properties");
            section.capabilities.share = true;
            section.capabilities.resolve_changed_status = true;
            section.dev_status.as_mut().expect("status").changed = false;
        }
        assert!(!panel.node_capability_allows_action(&resolve));

        let mut transform =
            DesignPanelNode::new("transform", "Pattern", DesignPanelNodeKind::TransformGroup);
        transform.transform_modifiers = vec![DesignRepeatModifier::linear(
            "repeat-stable",
            DesignRepeatAxis::Horizontal,
        )];
        panel.set_node(transform, cx);
        let add = DesignPanelAction::TransformModifierAddRequested {
            node_id: "transform".into(),
            repeat_type: DesignRepeatType::Radial,
        };
        let remove = DesignPanelAction::TransformModifierRemoveRequested {
            node_id: "transform".into(),
            modifier_id: "repeat-stable".into(),
            index: 0,
        };
        let change = DesignPanelAction::TransformModifierChangeRequested {
            node_id: "transform".into(),
            modifier_id: "repeat-stable".into(),
            index: 0,
            change: DesignTransformModifierChange::Count(8),
            phase: DesignPanelEditPhase::Commit,
        };
        let apply = DesignPanelAction::ApplyTransformModifiersRequested {
            node_id: "transform".into(),
        };
        for action in [&add, &remove, &change, &apply] {
            assert!(panel.node_capability_allows_action(action));
        }
        assert!(!panel.node_capability_allows_action(
            &DesignPanelAction::TransformModifierRemoveRequested {
                node_id: "transform".into(),
                modifier_id: "wrong".into(),
                index: 0,
            }
        ));
        assert!(!panel.node_capability_allows_action(
            &DesignPanelAction::TransformModifierChangeRequested {
                node_id: "transform".into(),
                modifier_id: "repeat-stable".into(),
                index: 0,
                change: DesignTransformModifierChange::Count(0),
                phase: DesignPanelEditPhase::Commit,
            }
        ));
        assert!(!panel.node_capability_allows_action(
            &DesignPanelAction::TransformModifierChangeRequested {
                node_id: "transform".into(),
                modifier_id: "repeat-stable".into(),
                index: 0,
                change: DesignTransformModifierChange::Offset(f32::NAN),
                phase: DesignPanelEditPhase::Preview,
            }
        ));

        panel
            .node
            .transform_modifiers
            .insert(0, DesignRepeatModifier::radial("host-inserted-before"));
        assert!(
            !panel.node_capability_allows_action(&remove),
            "a retained index hint is rejected after host reorder"
        );
        assert!(panel.node_capability_allows_action(
            &DesignPanelAction::TransformModifierRemoveRequested {
                node_id: "transform".into(),
                modifier_id: "repeat-stable".into(),
                index: 1,
            }
        ));

        let viewer_node = panel.node.clone();
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                viewer_node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.node_capability_allows_action(&add));
        assert!(!panel.node_capability_allows_action(&apply));
    });
}

#[gpui::test]
fn section_and_transform_controls_follow_capabilities_and_emit_stable_intents(
    cx: &mut TestAppContext,
) {
    let mut section = DesignPanelNode::new("section", "Checkout", DesignPanelNodeKind::Section);
    let section_properties = section.section.as_mut().expect("Section properties");
    section_properties.capabilities.completed_status = false;
    section_properties.dev_status = Some(
        DesignSectionDevStatus::new(DesignSectionDevStatusKind::ReadyForDev)
            .with_description("Implement")
            .changed(true),
    );
    let (host, visual_cx) = setup(section, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.property_is_editable(DesignPanelProperty::Opacity));
        assert!(!panel.property_is_editable(DesignPanelProperty::BlendMode));
        assert!(panel.property_is_editable(DesignPanelProperty::CornerRadius));
        assert!(panel.property_is_editable(DesignPanelProperty::CornerSmoothing));
        assert!(panel.render_section_properties(cx).is_some());
        assert!(
            panel
                .property_options(DesignPanelProperty::SectionDevStatus)
                .expect("Section status options")
                .iter()
                .all(|option| {
                    option.value
                        != DesignPanelValue::SectionDevStatus(Some(
                            DesignSectionDevStatusKind::Completed,
                        ))
                })
        );

        let mut transform =
            DesignPanelNode::new("transform", "Pattern", DesignPanelNodeKind::TransformGroup);
        transform.transform_modifiers = vec![DesignRepeatModifier::linear(
            "repeat-stable",
            DesignRepeatAxis::Horizontal,
        )];
        panel.set_node(transform, cx);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TransformRepeatType(0)),
            Some(DesignPanelValue::RepeatType(DesignRepeatType::Linear))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TransformRepeatAxis(0)),
            Some(DesignPanelValue::RepeatAxis(DesignRepeatAxis::Horizontal))
        );
        panel.emit_property(
            DesignPanelProperty::TransformRepeatAxis(0),
            DesignPanelValue::RepeatAxis(DesignRepeatAxis::Vertical),
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::TransformRepeatCount(0),
            DesignPanelValue::Integer(12),
            DesignPanelEditPhase::Preview,
            cx,
        );

        let mut radial = panel.node.clone();
        radial.transform_modifiers[0] = DesignRepeatModifier::radial("radial-stable");
        panel.set_node(radial, cx);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TransformRepeatAxis(0)),
            None
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::TransformRepeatAxis(0)));
    });

    let captured_actions = captured_actions.borrow();
    assert!(captured_actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TransformModifierChangeRequested {
            node_id,
            modifier_id,
            index: 0,
            change: DesignTransformModifierChange::Mode(DesignRepeatMode::Linear(
                DesignRepeatAxis::Vertical
            )),
            phase: DesignPanelEditPhase::Commit,
        } if node_id.as_ref() == "transform" && modifier_id.as_ref() == "repeat-stable"
    )));
    assert!(captured_actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TransformModifierChangeRequested {
            modifier_id,
            index: 0,
            change: DesignTransformModifierChange::Count(12),
            phase: DesignPanelEditPhase::Preview,
            ..
        } if modifier_id.as_ref() == "repeat-stable"
    )));
}

#[gpui::test]
fn layout_guide_fields_follow_kind_alignment_and_bindings(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        super::super::DesignLayoutGrid::columns(
            super::super::DesignColumnLayoutGrid {
                alignment: DesignColumnGridAlignment::Left,
                count: DesignLayoutGridCount::number(12),
                size: 64.,
                offset: 32.,
                gutter: 16.,
                margin: 0.,
                ..super::super::DesignColumnLayoutGrid::default()
            },
            DesignColor::PURPLE,
        )
        .with_variable_binding(
            super::super::DesignLayoutGridVariableField::Count,
            super::super::DesignLayoutGridVariableBinding::new("count-variable", "Desktop columns"),
        )
        .with_variable_binding(
            super::super::DesignLayoutGridVariableField::Offset,
            super::super::DesignLayoutGridVariableBinding::new("offset-variable", "Desktop edge"),
        ),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridAlignment(0)),
            Some(DesignPanelValue::ColumnGridAlignment(
                DesignColumnGridAlignment::Left
            ))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridSize(0)),
            Some(DesignPanelValue::Number(64.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridOffset(0)),
            Some(DesignPanelValue::Number(32.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridMargin(0)),
            None
        );
        assert!(
            !panel.property_is_editable(DesignPanelProperty::LayoutGridCount(0)),
            "a variable-bound count must detach before direct editing"
        );
        assert!(
            !panel.property_is_editable(DesignPanelProperty::LayoutGridOffset(0)),
            "the exact offset leaf must also detach before direct editing"
        );
        assert!(panel.property_is_editable(DesignPanelProperty::LayoutGridSize(0)));
        assert!(panel.property_is_editable(DesignPanelProperty::LayoutGridGutter(0)));

        let mut rows = node.clone();
        rows.layout_grids = vec![super::super::DesignLayoutGrid::rows(
            super::super::DesignRowLayoutGrid {
                alignment: DesignRowGridAlignment::Stretch,
                ..super::super::DesignRowLayoutGrid::default()
            },
            DesignColor::BLUE,
        )];
        panel.set_node(rows, cx);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridSize(0)),
            None,
            "stretch guides display an implicit Auto size"
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutGridOffset(0)),
            None
        );
        assert!(
            panel
                .current_property_value(DesignPanelProperty::LayoutGridMargin(0))
                .is_some()
        );

        let mut styled = node.clone();
        styled.layout_grid_style_binding = Some(super::super::DesignLayoutGridStyleBinding::new(
            "grid-style",
            "Desktop grid",
        ));
        panel.set_node(styled, cx);
        assert!(!panel.property_is_editable(DesignPanelProperty::LayoutGridColor(0)));

        let mut slot = DesignPanelNode::new("slot", "Slot", DesignPanelNodeKind::Slot);
        slot.layout_grids = vec![super::super::DesignLayoutGrid::uniform(
            8.,
            DesignColor::BLUE,
        )];
        panel.set_node(slot, cx);
        assert!(
            panel.render_layout_grids(cx).is_some(),
            "the renderer must honor the Slot capability promised by the resolver"
        );
    });
}

#[gpui::test]
fn layout_guide_detach_intents_retain_bound_identity(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        super::super::DesignLayoutGrid::columns(
            super::super::DesignColumnLayoutGrid::default(),
            DesignColor::PURPLE,
        )
        .with_id("guide-columns")
        .with_variable_binding(
            super::super::DesignLayoutGridVariableField::GutterSize,
            super::super::DesignLayoutGridVariableBinding::new("gutter-variable", "Desktop gutter"),
        ),
    ];
    node.layout_grid_style_binding = Some(super::super::DesignLayoutGridStyleBinding::new(
        "grid-style",
        "Desktop grid",
    ));
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_layout_grid_style_detach(cx);
        let mut detached_style = panel.node.clone();
        detached_style.layout_grid_style_binding = None;
        panel.set_node(detached_style, cx);
        let (target, _) = panel
            .layout_grid_variable_target(0, DesignPanelProperty::LayoutGridGutter(0))
            .expect("column guide gutter target");
        panel.emit_layout_grid_variable_detach(target, cx);
    });

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridStyleDetachRequested {
            node_id,
            style,
        } if node_id.as_ref() == "frame" && style.style_id.as_ref() == "grid-style"
    )));
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridVariableDetachRequested {
            node_id,
            target,
            variable_id,
        } if node_id.as_ref() == "frame"
            && target.guide_id.as_ref() == "guide-columns"
            && target.index == 0
            && target.field == super::super::DesignLayoutGridVariableField::GutterSize
            && target.property == DesignPanelProperty::LayoutGridGutter(0)
            && variable_id.as_ref() == "gutter-variable"
    )));
}

#[gpui::test]
fn layout_guide_resource_browsers_emit_atomic_styles_and_stable_variable_targets(
    cx: &mut TestAppContext,
) {
    let columns = super::super::DesignLayoutGrid::columns(
        super::super::DesignColumnLayoutGrid {
            alignment: DesignColumnGridAlignment::Left,
            count: DesignLayoutGridCount::number(12),
            ..super::super::DesignColumnLayoutGrid::default()
        },
        DesignColor::PURPLE,
    )
    .with_id("guide-columns");
    let rows = super::super::DesignLayoutGrid::rows(
        super::super::DesignRowLayoutGrid {
            count: DesignLayoutGridCount::Auto,
            ..super::super::DesignRowLayoutGrid::default()
        },
        DesignColor::BLUE,
    )
    .with_id("guide-rows");
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![columns.clone(), rows.clone()];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_layout_grid_style_view_data(
            super::super::DesignLayoutGridStyleViewData::new(
                [super::super::DesignLayoutGridStyle::new(
                    "page-grid",
                    "Page grid",
                    [columns.clone(), rows.clone()],
                )],
                [super::super::DesignLayoutGridStyleLibrary::new(
                    "acme-grids",
                    "Acme grids",
                    [super::super::DesignLayoutGridStyle::new(
                        "responsive-grid",
                        "Responsive grid",
                        [columns.clone()],
                    )
                    .available()],
                )],
            ),
            cx,
        );
        panel.set_layout_grid_variable_view_data(
            super::super::DesignLayoutGridVariableViewData::new([
                DesignVariable::page(
                    "columns-12",
                    "Columns / 12",
                    "breakpoints",
                    "Breakpoints",
                    super::super::DesignVariableResolvedType::Float,
                )
                .with_resolved_value(super::super::DesignVariableResolvedValue::Float(12.)),
                DesignVariable::page(
                    "columns-auto",
                    "Columns / Auto",
                    "breakpoints",
                    "Breakpoints",
                    super::super::DesignVariableResolvedType::Float,
                )
                .with_resolved_value(
                    super::super::DesignVariableResolvedValue::Float(f32::INFINITY),
                ),
                DesignVariable::page(
                    "section-64",
                    "Section / 64",
                    "spacing",
                    "Spacing",
                    super::super::DesignVariableResolvedType::Float,
                )
                .with_source(DesignVariableSource::library(
                    "layout-library",
                    "Layout library",
                ))
                .with_import_state(DesignVariableImportState::Available)
                .with_resolved_value(super::super::DesignVariableResolvedValue::Float(64.)),
            ])
            .with_create_state(super::super::DesignLayoutGridVariableCreateState::Enabled),
            cx,
        );

        panel.emit_layout_grid_style_apply(DesignLayoutGridStyleSelection::page("page-grid"), cx);
        panel.emit_layout_grid_style_import(
            DesignLayoutGridStyleSelection::library("acme-grids", "responsive-grid"),
            cx,
        );
        panel.emit_layout_grid_style_create(cx);

        let (columns_target, _) = panel
            .layout_grid_variable_target(0, DesignPanelProperty::LayoutGridCount(0))
            .expect("stable columns count target");
        panel.emit_layout_grid_variable_apply(columns_target.clone(), "columns-auto".into(), cx);

        let mut reordered = panel.node.clone();
        reordered.layout_grids.swap(0, 1);
        panel.set_node(reordered, cx);
        panel.emit_layout_grid_variable_apply(columns_target, "columns-12".into(), cx);
        let (size_target, _) = panel
            .layout_grid_variable_target(1, DesignPanelProperty::LayoutGridSize(1))
            .expect("stable columns section-size target");
        panel.emit_layout_grid_variable_import(size_target, "section-64".into(), cx);
        let (gutter_target, _) = panel
            .layout_grid_variable_target(1, DesignPanelProperty::LayoutGridGutter(1))
            .expect("stable columns gutter target");
        panel.emit_layout_grid_variable_create(gutter_target, cx);
    });

    let captured = captured_actions.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridStyleApplyRequested { node_id, style }
            if node_id.as_ref() == "frame"
                && style == &DesignLayoutGridStyleSelection::page("page-grid")
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridStyleImportRequested { node_id, style }
            if node_id.as_ref() == "frame"
                && style == &DesignLayoutGridStyleSelection::library(
                    "acme-grids",
                    "responsive-grid"
                )
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridStyleCreateRequested {
            node_id,
            layout_grids,
        } if node_id.as_ref() == "frame"
            && layout_grids.iter().map(|guide| guide.id.as_ref()).collect::<Vec<_>>()
                == vec!["guide-columns", "guide-rows"]
    )));
    assert_eq!(
        captured
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::LayoutGridVariableApplyRequested { .. }
            ))
            .count(),
        1,
        "the Auto candidate must be rejected for a numeric guide"
    );
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridVariableApplyRequested {
            node_id,
            target,
            variable_id,
        } if node_id.as_ref() == "frame"
            && target.guide_id.as_ref() == "guide-columns"
            && target.index == 1
            && target.field == super::super::DesignLayoutGridVariableField::Count
            && target.property == DesignPanelProperty::LayoutGridCount(1)
            && variable_id.as_ref() == "columns-12"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridVariableImportRequested {
            target,
            variable_id,
            ..
        } if target.guide_id.as_ref() == "guide-columns"
            && target.index == 1
            && target.field == super::super::DesignLayoutGridVariableField::SectionSize
            && target.property == DesignPanelProperty::LayoutGridSize(1)
            && variable_id.as_ref() == "section-64"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridVariableCreateRequested {
            target,
            value: super::super::DesignLayoutGridVariableValue::Number(24.),
            ..
        } if target.guide_id.as_ref() == "guide-columns"
            && target.field == super::super::DesignLayoutGridVariableField::GutterSize
            && target.property == DesignPanelProperty::LayoutGridGutter(1)
    )));
}

#[gpui::test]
fn layout_guide_resource_actions_are_read_only_gated(cx: &mut TestAppContext) {
    let guide = super::super::DesignLayoutGrid::columns(
        super::super::DesignColumnLayoutGrid::default(),
        DesignColor::PURPLE,
    )
    .with_id("guide-columns");
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![guide.clone()];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_layout_grid_style_view_data(
            super::super::DesignLayoutGridStyleViewData::new(
                [super::super::DesignLayoutGridStyle::new(
                    "page-grid",
                    "Page grid",
                    [guide],
                )],
                [],
            ),
            cx,
        );
        panel.set_layout_grid_variable_view_data(
            super::super::DesignLayoutGridVariableViewData::new([DesignVariable::page(
                "columns-12",
                "Columns / 12",
                "breakpoints",
                "Breakpoints",
                super::super::DesignVariableResolvedType::Float,
            )
            .with_resolved_value(super::super::DesignVariableResolvedValue::Float(12.))])
            .with_create_state(super::super::DesignLayoutGridVariableCreateState::Enabled),
            cx,
        );
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_layout_grid_style_apply(DesignLayoutGridStyleSelection::page("page-grid"), cx);
        panel.emit_layout_grid_style_create(cx);
        let (target, _) = panel
            .layout_grid_variable_target(0, DesignPanelProperty::LayoutGridCount(0))
            .expect("columns target");
        panel.emit_layout_grid_variable_apply(target.clone(), "columns-12".into(), cx);
        panel.emit_layout_grid_variable_create(target, cx);
    });

    assert!(captured_actions.borrow().is_empty());
}

#[gpui::test]
fn layout_guide_count_editor_emits_auto_as_a_typed_value(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        super::super::DesignLayoutGrid::columns(
            super::super::DesignColumnLayoutGrid::default(),
            DesignColor::PURPLE,
        )
        .with_id("guide-columns"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::LayoutGridCount(0),
                DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto),
                window,
                cx,
            );
            panel.property_input.update(cx, |input, cx| {
                input.set_value("Auto", window, cx);
            });
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridPropertyEditRequested {
            node_id,
            guide_id,
            index: 0,
            property: DesignPanelProperty::LayoutGridCount(0),
            value: DesignPanelValue::LayoutGridCount(DesignLayoutGridCount::Auto),
            phase: DesignPanelEditPhase::Commit,
        } if node_id.as_ref() == "frame" && guide_id.as_ref() == "guide-columns"
    )));
}

#[gpui::test]
fn layout_guide_edit_and_remove_intents_follow_stable_ids_across_host_reorder(
    cx: &mut TestAppContext,
) {
    let first =
        super::super::DesignLayoutGrid::uniform(8., DesignColor::BLUE).with_id("guide-first");
    let second =
        super::super::DesignLayoutGrid::uniform(12., DesignColor::PURPLE).with_id("guide-second");
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![first, second];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::LayoutGridSize(0),
                DesignPanelValue::Number(8.),
                window,
                cx,
            );

            let mut reordered = panel.node.clone();
            reordered.layout_grids.swap(0, 1);
            panel.set_node(reordered, cx);
            assert_eq!(
                panel.property_editor.as_ref().map(|editor| editor.property),
                Some(DesignPanelProperty::LayoutGridSize(1))
            );
            panel.emit_property_edit(
                DesignPanelProperty::LayoutGridSize(1),
                DesignPanelValue::Number(10.),
                DesignPanelEditPhase::Preview,
                cx,
            );
            panel.finish_property_edit(false, window, cx);
            panel.emit_remove(DesignPanelCollection::LayoutGrid, 1, cx);
        });
    });

    let captured = captured_actions.borrow();
    let edits = captured
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::LayoutGridPropertyEditRequested {
                guide_id,
                index,
                property,
                phase,
                ..
            } => Some((guide_id.as_ref(), *index, *property, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (
                "guide-first",
                0,
                DesignPanelProperty::LayoutGridSize(0),
                DesignPanelEditPhase::Begin,
            ),
            (
                "guide-first",
                1,
                DesignPanelProperty::LayoutGridSize(1),
                DesignPanelEditPhase::Preview,
            ),
            (
                "guide-first",
                1,
                DesignPanelProperty::LayoutGridSize(1),
                DesignPanelEditPhase::Cancel,
            ),
        ]
    );
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::LayoutGridRemoveRequested {
            node_id,
            guide_id,
            index: 1,
        } if node_id.as_ref() == "frame" && guide_id.as_ref() == "guide-first"
    )));
    assert!(!captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::LayoutGridSize(_),
            ..
        } | DesignPanelAction::CollectionItemRemoveRequested {
            collection: DesignPanelCollection::LayoutGrid,
            ..
        }
    )));
}

#[gpui::test]
fn auxiliary_layout_guide_color_edits_resolve_stable_identity_after_reorder(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        DesignLayoutGrid::uniform(8., DesignColor::rgba(0x11, 0x22, 0x33, 0x44))
            .with_id("guide-a")
            .with_opacity(16.),
        DesignLayoutGrid::uniform(12., DesignColor::BLUE).with_id("guide-b"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.auxiliary_color_picker = Some(AuxiliaryColorPickerTarget::LayoutGrid {
            node_id: "frame".into(),
            guide_id: "guide-a".into(),
            index: 0,
        });
        panel.node.layout_grids.swap(0, 1);
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(37.5),
            },
            DesignPanelEditPhase::Preview,
            cx,
        );
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(DesignColor::rgb(0xab, 0xcd, 0xef)),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert!(matches!(
        &captured[0],
        DesignPanelAction::LayoutGridPropertyEditRequested {
            guide_id,
            index: 1,
            property: DesignPanelProperty::LayoutGridOpacity(1),
            value: DesignPanelValue::Number(opacity),
            phase: DesignPanelEditPhase::Preview,
            ..
        } if guide_id.as_ref() == "guide-a" && (*opacity - 37.5).abs() < f32::EPSILON
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::LayoutGridPropertyEditRequested {
            guide_id,
            index: 1,
            property: DesignPanelProperty::LayoutGridColor(1),
            value: DesignPanelValue::Color(color),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if guide_id.as_ref() == "guide-a"
            && *color == DesignColor::rgba(0xab, 0xcd, 0xef, 0x44)
    ));
}

#[gpui::test]
fn layout_guide_picker_keeps_color_and_opacity_permissions_independent(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.layout_grids = vec![
        DesignLayoutGrid::uniform(8., DesignColor::rgba(0x11, 0x22, 0x33, 0x44))
            .with_id("guide-a")
            .with_opacity(16.),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = AuxiliaryColorPickerTarget::LayoutGrid {
        node_id: "frame".into(),
        guide_id: "guide-a".into(),
        index: 0,
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_auxiliary_color_picker(target.clone(), window, cx);
            let picker_target = PickerEventTarget {
                node_id: AuxiliaryColorPickerTarget::PICKER_NODE_ID.into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: target.picker_paint_id(),
            };
            let opacity_edit = DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(37.5),
            };
            assert!(panel.track_paint_edit(
                &picker_target,
                &opacity_edit,
                DesignPanelEditPhase::Begin
            ));
            panel.emit_auxiliary_color_edit(&opacity_edit, DesignPanelEditPhase::Begin, cx);
            panel.set_property_value_state(
                DesignPanelProperty::LayoutGridColor(0),
                DesignPanelPropertyValueState::Uniform(DesignPanelValue::Color(DesignColor::rgb(
                    0x11, 0x22, 0x33,
                )))
                .read_only_with_reason("Color is bound"),
                cx,
            );
            assert!(panel.active_paint_edit.is_some());
            panel.sync_paint_picker(window, cx);
            assert_eq!(
                panel.paint_picker.read(cx).color_only_editability(),
                (false, true)
            );

            panel.emit_auxiliary_color_edit(
                &DesignPaintEdit {
                    property: DesignPaintProperty::Color,
                    value: DesignPaintValue::Color(DesignColor::BLUE),
                },
                DesignPanelEditPhase::Commit,
                cx,
            );
            panel.emit_auxiliary_color_edit(
                &DesignPaintEdit {
                    property: DesignPaintProperty::Opacity,
                    value: DesignPaintValue::Number(37.5),
                },
                DesignPanelEditPhase::Preview,
                cx,
            );

            panel.set_property_value_state(
                DesignPanelProperty::LayoutGridOpacity(0),
                DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(16.))
                    .read_only_with_reason("Opacity is bound"),
                cx,
            );
            assert!(panel.active_paint_edit.is_none());
            assert_eq!(panel.auxiliary_color_picker.as_ref(), Some(&target));
            panel.sync_paint_picker(window, cx);
            assert_eq!(
                panel.paint_picker.read(cx).color_only_editability(),
                (false, false)
            );
        });
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::LayoutGridPropertyEditRequested {
                property: DesignPanelProperty::LayoutGridOpacity(0),
                phase: DesignPanelEditPhase::Begin,
                ..
            },
            DesignPanelAction::LayoutGridPropertyEditRequested {
                property: DesignPanelProperty::LayoutGridOpacity(0),
                phase: DesignPanelEditPhase::Preview,
                ..
            },
            DesignPanelAction::LayoutGridPropertyEditRequested {
                property: DesignPanelProperty::LayoutGridOpacity(0),
                phase: DesignPanelEditPhase::Cancel,
                ..
            }
        ]
    ));
}

#[gpui::test]
fn auxiliary_effect_color_edits_resolve_kind_and_stable_identity_after_reorder(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("shape", "Shape", DesignPanelNodeKind::Rectangle);
    node.effects = vec![
        DesignEffect::from_settings(
            true,
            DesignEffectSettings::Noise(super::super::DesignNoiseEffect {
                colors: DesignNoiseColors::Duotone {
                    color: DesignColor::BLACK,
                    secondary_color: DesignColor::rgba(0x10, 0x20, 0x30, 0x40),
                },
                ..super::super::DesignNoiseEffect::default()
            }),
        )
        .with_id("noise-a"),
        DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow-b"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.auxiliary_color_picker = Some(AuxiliaryColorPickerTarget::Effect {
            node_id: "shape".into(),
            effect_id: "noise-a".into(),
            index: 0,
            property: DesignPanelProperty::EffectNoiseSecondaryColor(0),
        });
        panel.node.effects.swap(0, 1);
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(DesignColor::rgb(0xa1, 0xb2, 0xc3)),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::EffectEditRequested {
            effect_id,
            index: 1,
            property: DesignPanelProperty::EffectNoiseSecondaryColor(1),
            value: DesignPanelValue::Color(color),
            phase: DesignPanelEditPhase::Commit,
            ..
        }] if effect_id.as_ref() == "noise-a"
            && *color == DesignColor::rgba(0xa1, 0xb2, 0xc3, 0x40)
    ));
}

#[gpui::test]
fn replacing_selection_closes_the_picker(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("a", "Frame", DesignPanelNodeKind::Frame),
        cx,
    );
    let panel = panel(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "a-fill-0".into(),
    };
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(target.clone());
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    assert!(visual_cx.read(|app| picker.read(app).target().is_some()));

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("b", "Text", DesignPanelNodeKind::Text),
            cx,
        );
        assert!(panel.active_picker.is_none());
    });
    visual_cx.run_until_parked();

    assert!(
        visual_cx.read(|app| picker.read(app).target().is_none()),
        "the retained picker must clear its controlled snapshot after dismissal"
    );
    assert!(visual_cx.read(|app| picker.read(app).paint().is_none()));
}

#[gpui::test]
fn removing_the_active_paint_clears_picker_state(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("a", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: node.fills[0].id.clone(),
    };
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(target.clone());
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());

    let mut without_fills = node;
    without_fills.fills.clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(without_fills, cx);
        assert!(panel.active_picker.is_none());
    });
    visual_cx.run_until_parked();

    assert!(visual_cx.read(|app| picker.read(app).target().is_none()));
    assert!(visual_cx.read(|app| picker.read(app).paint().is_none()));
}

#[gpui::test]
fn numeric_edits_emit_one_controlled_transaction(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.width = 100.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Width,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+10", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    let edits = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property,
                value,
                phase,
                ..
            } => Some((*property, value.clone(), *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (
                DesignPanelProperty::Width,
                DesignPanelValue::Number(100.),
                DesignPanelEditPhase::Begin,
            ),
            (
                DesignPanelProperty::Width,
                DesignPanelValue::Number(110.),
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignPanelProperty::Width,
                DesignPanelValue::Number(110.),
                DesignPanelEditPhase::Commit,
            ),
        ]
    );
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.width),
        100.,
        "the controlled panel must not mutate host view data"
    );
}

#[gpui::test]
fn value_cell_focus_round_trips_and_blur_transfer_is_never_stolen(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let x = visual_cx
        .debug_bounds("design-x")
        .expect("X value cell")
        .center();
    let y = visual_cx
        .debug_bounds("design-y")
        .expect("Y value cell")
        .center();

    visual_cx.simulate_click(x, Modifiers::none());
    visual_cx.run_until_parked();
    let x_return_handle = visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.property_editor.as_ref().map(|editor| editor.property),
            Some(DesignPanelProperty::X)
        );
        let return_focus = panel
            .editor_focus_return
            .as_ref()
            .expect("control activation should retain X");
        assert_eq!(
            return_focus.origin,
            EditorFocusOrigin::ValueCell(DesignPanelProperty::X)
        );
        return_focus.handle.clone()
    });

    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| x_return_handle.is_focused(window)));

    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .property_editor
                .as_ref()
                .map(|editor| editor.property)
        }),
        Some(DesignPanelProperty::X),
        "Enter on the restored row should reopen that row"
    );

    visual_cx.simulate_click(y, Modifiers::none());
    visual_cx.run_until_parked();
    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.property_editor.as_ref().map(|editor| editor.property),
            Some(DesignPanelProperty::Y),
            "the Input blur must allow the newly clicked row to take over"
        );
        assert!(matches!(
            panel
                .editor_focus_return
                .as_ref()
                .map(|return_focus| &return_focus.origin),
            Some(EditorFocusOrigin::ValueCell(DesignPanelProperty::Y))
        ));
    });
    assert!(
        !visual_cx.update(|window, _| x_return_handle.is_focused(window)),
        "Input blur must not schedule restoration that steals focus back to X"
    );
}

#[gpui::test]
fn cancel_restores_the_original_controlled_value(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.width = 100.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Width,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+25", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(false, window, cx);
        });
    });

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::Width,
            value: DesignPanelValue::Number(100.),
            phase: DesignPanelEditPhase::Cancel,
            ..
        }
    )));
}

#[gpui::test]
fn permission_downgrade_terminates_active_numeric_edit_before_gating(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.width = 100.;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Width,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node.clone(),
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::viewer(),
                ),
                cx,
            );
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Width,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn dimension_menu_opens_from_keyboard_and_emits_sizing_selection(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let width_menu_focus = visual_cx.read(|app| {
        panel
            .read(app)
            .dimension_menu_focus
            .first()
            .expect("Width menu focus")
            .clone()
    });

    visual_cx.update(|window, cx| {
        width_menu_focus.focus(window, cx);
    });
    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).dimension_menu_open),
        Some(DesignLayoutDimensionAxis::Width)
    );
    assert!(
        visual_cx.debug_bounds("design-horizontal-sizing").is_none(),
        "the duplicate lower Resizing control must stay removed"
    );
    let fixed = visual_cx
        .debug_bounds("design-width-sizing-fixed")
        .expect("Fixed width menu row")
        .center();
    visual_cx.simulate_click(fixed, Modifiers::none());
    visual_cx.run_until_parked();

    assert!(
        captured_actions.borrow().iter().any(|action| matches!(
            action,
            DesignPanelAction::PropertyChangeRequested {
                property: DesignPanelProperty::HorizontalSizing,
                value: DesignPanelValue::SizingMode(DesignSizingMode::Fixed),
                ..
            }
        )),
        "unexpected actions: {:?}",
        captured_actions.borrow()
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .layout
                .as_ref()
                .expect("frame layout")
                .horizontal_sizing
        }),
        DesignSizingMode::Hug,
        "select confirmation must remain host controlled"
    );
}

#[gpui::test]
fn sizing_options_follow_owner_child_text_and_grid_span_context(cx: &mut TestAppContext) {
    let owner = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let (host, visual_cx) = setup(owner, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let modes = |panel: &DesignPanel, property| {
            panel
                .property_options(property)
                .expect("sizing options")
                .into_iter()
                .filter_map(|option| match option.value {
                    DesignPanelValue::SizingMode(mode) => Some(mode),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };

        assert_eq!(
            modes(panel, DesignPanelProperty::HorizontalSizing),
            vec![DesignSizingMode::Fixed, DesignSizingMode::Hug],
            "a top-level auto-layout owner cannot fill a nonexistent parent"
        );

        let rectangle = DesignPanelNode::new("child", "Child", DesignPanelNodeKind::Rectangle);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                rectangle,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Horizontal,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            modes(panel, DesignPanelProperty::HorizontalSizing),
            vec![DesignSizingMode::Fixed, DesignSizingMode::Fill],
            "a non-text child can fill but cannot hug"
        );

        let text = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                text,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Horizontal,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            modes(panel, DesignPanelProperty::HorizontalSizing),
            vec![
                DesignSizingMode::Fixed,
                DesignSizingMode::Hug,
                DesignSizingMode::Fill,
            ]
        );

        let mut grid_child =
            DesignPanelNode::new("grid-child", "Grid child", DesignPanelNodeKind::Rectangle);
        let layout = grid_child.layout.as_mut().expect("grid child layout data");
        layout.item.grid_column_span = 2;
        layout.item.grid_row_span = 3;
        layout.horizontal_sizing = DesignSizingMode::Fill;
        layout.vertical_sizing = DesignSizingMode::Fill;
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                grid_child,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Grid,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            modes(panel, DesignPanelProperty::HorizontalSizing),
            vec![DesignSizingMode::Fill]
        );
        assert_eq!(
            modes(panel, DesignPanelProperty::VerticalSizing),
            vec![DesignSizingMode::Fill]
        );
    });
}

#[gpui::test]
fn layout_value_guards_reject_invalid_fill_and_limits_but_allow_fixed_negative_gap(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    node.layout.as_mut().expect("frame layout").item.max_width = Some(200.);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_property(
            DesignPanelProperty::HorizontalSizing,
            DesignPanelValue::SizingMode(DesignSizingMode::Fill),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::MinWidth,
            DesignPanelValue::OptionalNumber(Some(220.)),
            cx,
        );
        panel.emit_property(DesignPanelProperty::Gap, DesignPanelValue::Number(-8.), cx);
        panel.emit_property(
            DesignPanelProperty::MinWidth,
            DesignPanelValue::OptionalNumber(Some(120.)),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured_actions = captured_actions.borrow();
    assert_eq!(
        captured_actions
            .iter()
            .filter(|action| matches!(action, DesignPanelAction::PropertyChangeRequested { .. }))
            .count(),
        2,
        "only the valid negative fixed gap and ordered minimum should emit"
    );
    assert!(captured_actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::Gap,
            value: DesignPanelValue::Number(value),
            ..
        } if *value == -8.
    )));
    assert!(captured_actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::MinWidth,
            value: DesignPanelValue::OptionalNumber(Some(value)),
            ..
        } if *value == 120.
    )));
}

#[gpui::test]
fn dimension_limit_disclosure_is_scoped_to_owners_and_participating_children(
    cx: &mut TestAppContext,
) {
    let mut owner = DesignPanelNode::new("owner", "Owner", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let owner_layout = owner.layout.as_mut().expect("owner layout");
    owner_layout.item.min_width = Some(120.);
    owner_layout.item.max_width = Some(640.);
    let (host, visual_cx) = setup(owner.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.dimension_limits_are_applicable());
        assert!(
            panel
                .render_dimension_limit_fields(DesignLayoutDimensionAxis::Width, cx)
                .is_none(),
            "existing host limits start collapsed after selection"
        );

        let mut child = DesignPanelNode::new("child", "Child", DesignPanelNodeKind::Rectangle);
        child.layout.as_mut().expect("child layout").item.min_height = Some(44.);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                child.clone(),
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Vertical,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(panel.dimension_limits_are_applicable());

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                child.clone(),
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Vertical,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::Ignored,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(!panel.dimension_limits_are_applicable());
        assert!(
            panel
                .render_dimension_limit_fields(DesignLayoutDimensionAxis::Height, cx)
                .is_none(),
            "Ignore auto layout must not leak stale min/max item data"
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                child,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(!panel.dimension_limits_are_applicable());

        let inactive_owner =
            DesignPanelNode::new("inactive", "Inactive", DesignPanelNodeKind::Frame);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                inactive_owner,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(
            !panel.dimension_limits_are_applicable(),
            "a Frame without active auto layout is not a valid min/max owner"
        );
    });
}

#[gpui::test]
fn freeform_nodes_omit_dimension_limit_menus_even_with_stale_host_values(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("freeform", "Freeform", DesignPanelNodeKind::Rectangle);
    let item = &mut node.layout.as_mut().expect("layout snapshot").item;
    item.min_width = Some(120.);
    item.max_width = Some(640.);
    item.min_height = Some(44.);
    item.max_height = Some(240.);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    assert!(
        visual_cx.debug_bounds("design-width").is_some(),
        "ordinary freeform dimensions remain available"
    );
    assert!(
        visual_cx
            .debug_bounds("design-width-dimension-icon")
            .is_none()
            && visual_cx
                .debug_bounds("design-height-dimension-icon")
                .is_none()
            && visual_cx.debug_bounds("design-layout-min-width").is_none()
            && visual_cx.debug_bounds("design-layout-max-height").is_none(),
        "stale item limits cannot leak the auto-layout-only disclosure"
    );
    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.dimension_limits_are_applicable());
        assert!(!panel.remove_dimension_limits(DesignLayoutDimensionAxis::Width, cx));
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn dimension_limit_add_remove_stays_controlled_and_gates_each_leaf(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("limits", "Limits", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Vertical);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    let width_menu = visual_cx
        .debug_bounds("design-width-dimension-icon")
        .expect("Width menu trigger")
        .center();
    visual_cx.simulate_click(width_menu, Modifiers::none());
    visual_cx.run_until_parked();
    let add_min_width = visual_cx
        .debug_bounds("design-width-add-min")
        .expect("Add min width row")
        .center();
    visual_cx.simulate_click(add_min_width, Modifiers::none());
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(
                panel
                    .dimension_limit_fields_disclosed
                    .contains(&DesignPanelProperty::MinWidth)
            );
            assert!(panel.property_editor.as_ref().is_some_and(|editor| {
                editor.property == DesignPanelProperty::MinWidth
                    && editor.original == DesignPanelValue::OptionalNumber(None)
            }));
            assert_eq!(
                panel
                    .node
                    .layout
                    .as_ref()
                    .expect("controlled layout")
                    .item
                    .min_width,
                None,
                "adding the field cannot invent a document value"
            );
            panel.finish_property_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::PropertyEditRequested {
                node_id: "limits".into(),
                property: DesignPanelProperty::MinWidth,
                value: DesignPanelValue::OptionalNumber(None),
                phase: DesignPanelEditPhase::Begin,
            },
            DesignPanelAction::PropertyEditRequested {
                node_id: "limits".into(),
                property: DesignPanelProperty::MinWidth,
                value: DesignPanelValue::OptionalNumber(None),
                phase: DesignPanelEditPhase::Cancel,
            },
        ]
    );

    captured.borrow_mut().clear();
    let mut limited = node.clone();
    let layout = limited.layout.as_mut().expect("limited layout");
    layout.item.min_width = Some(120.);
    layout.item.max_width = Some(640.);
    layout.item.min_height = Some(44.);
    layout.item.max_height = Some(240.);
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(limited, cx);
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::MaxWidth,
                    DesignPanelPropertyValueState::bound(
                        super::super::DesignPanelPropertyBinding::new(
                            "max-width-variable",
                            "Dimensions / Max width",
                            DesignPanelBindingKind::Variable,
                            DesignPanelValue::OptionalNumber(Some(640.)),
                        ),
                    ),
                ),
                (
                    DesignPanelProperty::MinHeight,
                    DesignPanelPropertyValueState::Uniform(DesignPanelValue::OptionalNumber(Some(
                        44.,
                    )))
                    .read_only_with_reason("Owned by the component"),
                ),
            ],
            cx,
        );
        assert!(panel.remove_dimension_limits(DesignLayoutDimensionAxis::Width, cx));
        assert!(panel.remove_dimension_limits(DesignLayoutDimensionAxis::Height, cx));
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::PropertyChangeRequested {
                node_id: "limits".into(),
                property: DesignPanelProperty::MinWidth,
                value: DesignPanelValue::OptionalNumber(None),
            },
            DesignPanelAction::PropertyChangeRequested {
                node_id: "limits".into(),
                property: DesignPanelProperty::MaxHeight,
                value: DesignPanelValue::OptionalNumber(None),
            },
        ],
        "Remove min and max must clear only independently editable leaves"
    );
    assert_eq!(
        visual_cx.read(|app| {
            let layout = panel
                .read(app)
                .node
                .layout
                .as_ref()
                .expect("controlled layout");
            (
                layout.item.min_width,
                layout.item.max_width,
                layout.item.min_height,
                layout.item.max_height,
            )
        }),
        (Some(120.), Some(640.), Some(44.), Some(240.)),
        "remove intents cannot mutate the host snapshot before its echo"
    );

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(
            !panel.remove_dimension_limits(DesignLayoutDimensionAxis::Width, cx),
            "viewer and invalid freeform contexts remain inert"
        );
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn dimension_limit_icon_supports_pointer_keyboard_hover_and_selection_reset(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("limits", "Limits", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let layout = node.layout.as_mut().expect("limits layout");
    layout.item.min_width = Some(120.);
    layout.item.max_width = Some(640.);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    assert!(
        visual_cx
            .debug_bounds("design-width-dimension-icon")
            .is_some()
    );
    assert!(
        visual_cx.debug_bounds("design-layout-min-width").is_none(),
        "existing values are hidden after initial selection"
    );

    let icon = visual_cx
        .debug_bounds("design-width-dimension-icon")
        .expect("width dimension icon")
        .center();
    visual_cx.simulate_mouse_move(icon, None, Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::DimensionLimitsPreviewRequested {
            node_id: "limits".into(),
            axis: DesignLayoutDimensionAxis::Width,
            minimum: Some(120.),
            maximum: Some(640.),
            preview: true,
        }]
    );
    visual_cx.simulate_mouse_move(gpui::point(px(1.), px(1.)), None, Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().last(),
        Some(&DesignPanelAction::DimensionLimitsPreviewRequested {
            node_id: "limits".into(),
            axis: DesignLayoutDimensionAxis::Width,
            minimum: Some(120.),
            maximum: Some(640.),
            preview: false,
        })
    );

    captured.borrow_mut().clear();
    visual_cx.simulate_click(icon, Modifiers::none());
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-layout-min-width").is_some()
            && visual_cx.debug_bounds("design-layout-max-width").is_some(),
        "pointer activation discloses exact existing fields"
    );
    assert!(
        visual_cx
            .debug_bounds("design-width-remove-min-max")
            .is_some(),
        "the open Width menu exposes the native combined remove row"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(panel.dimension_limit_fields_disclosed.is_empty());
        assert_eq!(panel.dimension_menu_open, None);
    });
    visual_cx.run_until_parked();
    let width_menu_focus = visual_cx.read(|app| {
        panel
            .read(app)
            .dimension_menu_focus
            .first()
            .expect("Width menu focus")
            .clone()
    });
    visual_cx.update(|window, cx| width_menu_focus.focus(window, cx));
    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert!(
        visual_cx.read(|app| {
            let panel = panel.read(app);
            panel
                .dimension_limit_fields_disclosed
                .contains(&DesignPanelProperty::MinWidth)
                && panel
                    .dimension_limit_fields_disclosed
                    .contains(&DesignPanelProperty::MaxWidth)
                && panel.dimension_menu_open == Some(DesignLayoutDimensionAxis::Width)
        }),
        "Enter on the focused dimension trigger follows the pointer disclosure path"
    );
}

#[gpui::test]
fn padding_supports_axis_individual_and_css_shorthand_modes(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("padding", "Padding", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Vertical);
    node.layout.as_mut().expect("layout").padding = [8., 16., 8., 16.];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(panel.padding_editor_mode, PaddingEditorMode::Axes);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::PaddingVertical),
            Some(DesignPanelValue::Number(8.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::PaddingHorizontal),
            Some(DesignPanelValue::Number(16.))
        );
        panel.emit_property(
            DesignPanelProperty::PaddingVertical,
            DesignPanelValue::Number(12.),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::PaddingHorizontal,
            DesignPanelValue::Number(20.),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::PaddingShorthand,
            DesignPanelValue::NumberList(vec![1., 2., 3., 4.]),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::PaddingShorthand,
            DesignPanelValue::NumberList(vec![1., 2., 3., 4., 5.]),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::PaddingTop,
            DesignPanelValue::Number(-1.),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let actions = captured_actions.borrow();
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::PropertyChangeRequested {
                    property: DesignPanelProperty::PaddingVertical
                        | DesignPanelProperty::PaddingHorizontal
                        | DesignPanelProperty::PaddingShorthand,
                    ..
                }
            ))
            .count(),
        3
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .layout
                .as_ref()
                .expect("controlled layout")
                .padding
        }),
        [8., 16., 8., 16.],
        "padding candidates must remain host controlled"
    );
    assert_eq!(format_padding_shorthand([8.; 4]), "8");
    assert_eq!(format_padding_shorthand([8., 16., 8., 16.]), "8, 16");
    assert_eq!(format_padding_shorthand([8., 16., 24., 16.]), "8, 16, 24");
    assert_eq!(
        format_padding_shorthand([8., 12., 16., 20.]),
        "8, 12, 16, 20"
    );
}

#[gpui::test]
fn wrap_counter_axis_exposes_exact_controlled_values_and_options(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("wrap", "Wrap", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    node.layout.as_mut().expect("frame layout").set_wrap(true);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::CounterAxisAlignContent),
            Some(DesignPanelValue::CounterAxisAlignContent(
                DesignCounterAxisAlignContent::Auto
            ))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::CounterAxisGap),
            Some(DesignPanelValue::OptionalNumber(None))
        );
        assert!(panel.property_is_editable(DesignPanelProperty::CounterAxisAlignContent));
        assert!(panel.property_is_editable(DesignPanelProperty::CounterAxisGap));
        let options = panel
            .property_options(DesignPanelProperty::CounterAxisAlignContent)
            .expect("wrapped-track distribution options");
        assert_eq!(
            options
                .iter()
                .map(|option| option.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["Auto", "Space between"]
        );
        drop(panel.render_layout(cx));
    });
}

#[gpui::test]
fn wrap_counter_axis_guards_values_allows_cancel_and_never_emits_for_viewers(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("wrap", "Wrap", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    node.layout.as_mut().expect("frame layout").set_wrap(true);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_property(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(None),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(Some(4.)),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(Some(0.)),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::Number(4.),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::CounterAxisAlignContent,
            DesignPanelValue::CounterAxisAlignContent(DesignCounterAxisAlignContent::SpaceBetween),
            cx,
        );

        let layout = panel.node.layout.as_mut().expect("frame layout");
        layout.counter_axis_align_content = DesignCounterAxisAlignContent::SpaceBetween;
        layout.counter_axis_gap = None;
        assert!(!panel.property_is_editable(DesignPanelProperty::CounterAxisGap));
        panel.emit_property(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(Some(6.)),
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(Some(4.)),
            DesignPanelEditPhase::Cancel,
            cx,
        );
    });
    visual_cx.run_until_parked();

    let actions = captured_actions.borrow();
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::PropertyChangeRequested {
                    property: DesignPanelProperty::CounterAxisGap,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::CounterAxisAlignContent,
            value: DesignPanelValue::CounterAxisAlignContent(
                DesignCounterAxisAlignContent::SpaceBetween
            ),
            ..
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyEditRequested {
            property: DesignPanelProperty::CounterAxisGap,
            value: DesignPanelValue::OptionalNumber(Some(value)),
            phase: DesignPanelEditPhase::Cancel,
            ..
        } if *value == 4.
    )));
    drop(actions);

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::CounterAxisAlignContent,
            DesignPanelValue::CounterAxisAlignContent(DesignCounterAxisAlignContent::SpaceBetween),
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::CounterAxisGap,
            DesignPanelValue::OptionalNumber(None),
            DesignPanelEditPhase::Cancel,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().is_empty());
}

#[gpui::test]
fn viewer_and_page_contexts_never_emit_edit_intents(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::Width,
            DesignPanelValue::Number(400.),
            cx,
        );
        panel.emit_add(DesignPanelCollection::Fill, cx);
        panel.emit_paint_edit(
            PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "frame-fill-0".into(),
            },
            DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(DesignColor::BLACK),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::Width,
            DesignPanelValue::Number(500.),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().is_empty());
    assert_eq!(
        visual_cx.read(|app| { panel.read(app).inspection_context.selection().kind() }),
        DesignPanelSelectionKind::None
    );
}

#[gpui::test]
fn viewer_generic_value_cells_emit_exact_copy_intents_from_pointer_and_keyboard(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.x = 123.5;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_viewer_properties_view_data(
            DesignViewerPropertiesViewData::new(
                DesignPanelTarget::Nodes {
                    node_ids: vec!["rectangle".into()],
                },
                [super::super::DesignViewerPropertySection::new(
                    "layout",
                    "Layout",
                    [
                        super::super::DesignViewerPropertyRow::new("x", "X", "123.5  ◇")
                            .with_property(DesignPanelProperty::X),
                    ],
                )],
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::X,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "viewer-x-variable",
                "Position / X",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(123.5),
            )),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let value_cell = visual_cx
        .debug_bounds("design-viewer-row-layout-x")
        .expect("viewer X value cell")
        .center();
    visual_cx.simulate_click(value_cell, Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PropertyCopyRequested {
            target: DesignPanelTarget::Nodes {
                node_ids: vec!["rectangle".into()],
            },
            property: DesignPanelProperty::X,
            displayed_value: "123.5  ◇".into(),
        }]
    );

    captured_actions.borrow_mut().clear();
    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PropertyCopyRequested {
            target: DesignPanelTarget::Nodes {
                node_ids: vec!["rectangle".into()],
            },
            property: DesignPanelProperty::X,
            displayed_value: "123.5  ◇".into(),
        }],
        "Enter on the focused viewer readout must use the same copy path"
    );

    captured_actions.borrow_mut().clear();
    visual_cx.simulate_keystrokes("space");
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PropertyCopyRequested {
            target: DesignPanelTarget::Nodes {
                node_ids: vec!["rectangle".into()],
            },
            property: DesignPanelProperty::X,
            displayed_value: "123.5  ◇".into(),
        }],
        "Space on the focused viewer readout must use the same copy path"
    );

    captured_actions.borrow_mut().clear();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_property_value_states([], cx);
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node.clone(),
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                ),
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::X,
                DesignPanelValue::Number(131.5),
                window,
                cx,
            );
            assert!(
                panel
                    .property_editor
                    .as_ref()
                    .is_some_and(|editor| editor.property == DesignPanelProperty::X),
                "editor mode must retain its existing value-editor activation"
            );
            panel.emit_property_copy(DesignPanelProperty::X, "123.5".into(), cx);
            panel.finish_property_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert!(
        !captured_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, DesignPanelAction::PropertyCopyRequested { .. })),
        "editor activation must never be replaced by viewer copy"
    );

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::restricted_viewer(),
            ),
            cx,
        );
    });
    visual_cx.run_until_parked();
    visual_cx.simulate_click(value_cell, Modifiers::none());
    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a viewer without copy permission must remain inert"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::viewer()),
            cx,
        );
        panel.emit_property_copy(DesignPanelProperty::X, "123.5".into(), cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "page/no-selection inspection has no generic node target to copy"
    );
}

#[gpui::test]
fn viewer_ui3_projection_uses_exact_section_copy_and_representation_intents(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Hero title", DesignPanelNodeKind::Text);
    node.stroke = Some(DesignStroke::for_node(
        DesignPanelNodeKind::Text,
        DesignPaint::solid(DesignColor::BLACK),
        2.,
        DesignStrokeAlign::Inside,
    ));
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["text".into()],
    };
    let typography_copy: SharedString = "font-family: \"Inter\";\nfont-size: 16px;".into();
    let borders_copy: SharedString = "border: 2px solid rgb(30, 30, 30);".into();
    let view_data = DesignViewerPropertiesViewData::new(
        target.clone(),
        [
            super::super::DesignViewerPropertySection::text_content(
                "content",
                "Design with confidence",
            ),
            super::super::DesignViewerPropertySection::new(
                "typography",
                "Typography",
                [
                    super::super::DesignViewerPropertyRow::new("font-size", "Size", "16px")
                        .with_property(DesignPanelProperty::FontSize),
                ],
            )
            .with_copy_value(typography_copy.clone())
            .with_copy_all(),
            super::super::DesignViewerPropertySection::new(
                "borders",
                "Borders",
                [
                    super::super::DesignViewerPropertyRow::new("color", "Color", "rgb(30, 30, 30)"),
                    super::super::DesignViewerPropertyRow::new("weight", "Weight", "2px")
                        .with_property(DesignPanelProperty::StrokeWeight),
                ],
            )
            .with_summary(borders_copy.clone())
            .with_copy_value(borders_copy.clone())
            .with_color_representation(DesignViewerColorRepresentation::Css),
        ],
    );
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_viewer_properties_view_data(view_data.clone(), cx);
    });
    visual_cx.run_until_parked();

    assert!(visual_cx.debug_bounds("design-tab-comment").is_some());
    assert!(visual_cx.debug_bounds("design-tab-properties").is_some());
    assert!(
        visual_cx
            .debug_bounds("design-viewer-row-typography-font-size")
            .is_some()
    );
    assert!(
        visual_cx.debug_bounds("design-x").is_none(),
        "viewer mode must not project disabled editor controls"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.emit_viewer_section_copy("typography".into(), typography_copy.clone(), cx);
        panel.emit_viewer_section_representation_change(
            "borders".into(),
            DesignViewerColorRepresentation::Hex,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured_actions.borrow().as_slice(),
        [
            DesignPanelAction::ViewerSectionCopyRequested {
                target: target.clone(),
                section_id: "typography".into(),
                copy_value: typography_copy,
            },
            DesignPanelAction::ViewerSectionRepresentationChangeRequested {
                target: target.clone(),
                section_id: "borders".into(),
                representation: DesignViewerColorRepresentation::Hex,
            },
        ]
    );

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_viewer_properties_view_data(
            DesignViewerPropertiesViewData::new(
                DesignPanelTarget::Nodes {
                    node_ids: vec!["stale-text".into()],
                },
                view_data.sections.clone(),
            ),
            cx,
        );
        panel.emit_viewer_section_copy("borders".into(), borders_copy.clone(), cx);
        panel.emit_viewer_section_representation_change(
            "borders".into(),
            DesignViewerColorRepresentation::Rgb,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a stale viewer projection cannot emit against a new selection"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::restricted_viewer(),
            ),
            cx,
        );
        panel.set_viewer_properties_view_data(view_data, cx);
        panel.emit_viewer_section_copy("borders".into(), borders_copy, cx);
        panel.emit_viewer_section_representation_change(
            "borders".into(),
            DesignViewerColorRepresentation::Hsl,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "restricted viewers remain non-copyable and cannot change copy representation"
    );
}

#[gpui::test]
fn canvas_parent_omits_constraints_without_hiding_position_controls(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.can_show_constraints());
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Canvas,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(!panel.can_show_constraints());
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::X),
            Some(DesignPanelValue::Number(120.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::Y),
            Some(DesignPanelValue::Number(96.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::Rotation),
            Some(DesignPanelValue::Number(0.))
        );
        drop(panel.render_position(cx));
    });
}

#[gpui::test]
fn ignored_auto_layout_child_restores_constraints_and_keeps_a_flow_escape(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("ignored", "Ignored child", DesignPanelNodeKind::Rectangle);
    node.layout
        .as_mut()
        .expect("rectangle layout")
        .item
        .positioning = DesignLayoutPositioning::Absolute;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Horizontal,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::Ignored,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(panel.can_show_constraints());
        assert!(
            panel
                .inspection_context
                .parent_layout()
                .is_ignored_by_auto_layout()
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::LayoutPositioning),
            Some(DesignPanelValue::LayoutPositioning(
                DesignLayoutPositioning::Absolute
            ))
        );
        drop(panel.render_position(cx));
        drop(panel.render_layout(cx));
    });
}

#[gpui::test]
fn command_targets_preserve_every_selected_node_and_page_context(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            panel.command_target(),
            DesignPanelTarget::Nodes {
                node_ids: vec!["one".into(), "two".into()]
            }
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        assert_eq!(
            panel.command_target(),
            DesignPanelTarget::Page {
                page_id: "one".into()
            }
        );
    });
}

#[gpui::test]
fn ordinary_multiple_selection_intents_use_the_exact_target_and_keep_leaf_states_controlled(
    cx: &mut TestAppContext,
) {
    let first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let bound_opacity =
        DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
            "opacity-variable",
            "Opacity",
            DesignPanelBindingKind::Variable,
            DesignPanelValue::Number(first.opacity),
        ));

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_states(
            [
                (
                    DesignPanelProperty::Width,
                    DesignPanelPropertyValueState::Mixed,
                ),
                (
                    DesignPanelProperty::Height,
                    DesignPanelPropertyValueState::Unset,
                ),
                (DesignPanelProperty::Opacity, bound_opacity.clone()),
            ],
            cx,
        );
        let controlled_states = panel.property_value_states.clone();

        panel.emit_property(
            DesignPanelProperty::Width,
            DesignPanelValue::Number(640.),
            cx,
        );
        panel.emit_add(DesignPanelCollection::Fill, cx);

        assert_eq!(
            panel.property_value_states, controlled_states,
            "emitting candidates must not synthesize uniform, unset, or binding state"
        );
    });
    visual_cx.run_until_parked();

    let actions = captured.borrow();
    assert_eq!(actions.len(), 2);
    for action in actions.iter() {
        let Some((target, _)) = action.targeted_node_action() else {
            panic!("ordinary multi-selection actions must carry an exact target");
        };
        assert_eq!(
            target,
            &DesignPanelTarget::Nodes {
                node_ids: vec!["one".into(), "two".into()],
            }
        );
    }
    assert!(matches!(
        actions[0].targeted_node_action().map(|(_, action)| action),
        Some(DesignPanelAction::PropertyChangeRequested {
            node_id,
            property: DesignPanelProperty::Width,
            value: DesignPanelValue::Number(640.),
        }) if node_id.as_ref() == "one"
    ));
    assert!(matches!(
        actions[1].targeted_node_action().map(|(_, action)| action),
        Some(DesignPanelAction::CollectionItemAddRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
        }) if node_id.as_ref() == "one"
    ));
}

#[gpui::test]
fn selection_color_edits_preserve_aggregate_and_occurrence_identity(cx: &mut TestAppContext) {
    let first_reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "one-fill",
        0,
    );
    let second_reference = super::super::DesignSelectionPaintReference::paint(
        "two",
        super::super::DesignSelectionPaintCollection::Stroke,
        "two-stroke",
        0,
    );
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate =
        super::super::DesignSelectionColors::new([super::super::DesignSelectionColor::new(
            "aggregate-purple",
            DesignColor::PURPLE,
            [first_reference.clone(), second_reference.clone()],
        )]);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::SelectionColor(0),
            DesignPanelValue::Color(DesignColor::BLACK),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::SelectionColorEditRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            selection_color_id,
            color: DesignColor::BLACK,
            paint_references,
            phase: DesignPanelEditPhase::Commit,
        } if node_ids == &vec![SharedString::from("one"), SharedString::from("two")]
            && selection_color_id.as_ref() == "aggregate-purple"
            && paint_references == &vec![first_reference.clone(), second_reference.clone()]
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.resolved_selection_colors()[0].color),
        DesignColor::PURPLE,
        "selection-color edits remain host controlled"
    );
}

#[gpui::test]
fn selection_paint_edits_and_occurrence_selection_preserve_exact_targeting(
    cx: &mut TestAppContext,
) {
    let first_reference = super::super::DesignSelectionPaintReference::paint(
        "two",
        super::super::DesignSelectionPaintCollection::Fill,
        "two-fill",
        0,
    );
    let second_reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Stroke,
        "one-stroke",
        1,
    );
    let representative = DesignPaint::solid(DesignColor::BLUE).with_id("two-fill");
    let row = super::super::DesignSelectionColor::new(
        "aggregate-blue",
        DesignColor::BLUE,
        [first_reference.clone(), second_reference.clone()],
    )
    .with_paint(representative.clone());
    let mut first = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate = super::super::DesignSelectionColors::new([row.clone()]);
    let second = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = AuxiliaryColorPickerTarget::SelectionColor {
        target: DesignPanelTarget::Nodes {
            node_ids: vec!["two".into(), "one".into()],
        },
        selection_color_id: row.id.clone(),
    };
    let gradient_payload = DesignPaint::gradient(
        super::super::DesignPaintKind::LinearGradient,
        vec![
            super::super::DesignGradientStop::new(0., DesignColor::BLUE).with_id("start"),
            super::super::DesignGradientStop::new(1., DesignColor::PURPLE).with_id("end"),
        ],
    )
    .payload;

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.auxiliary_color_picker = Some(target);
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(42.),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::BlendMode,
                value: DesignPaintValue::BlendMode(DesignBlendMode::Multiply),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                value: DesignPaintValue::Payload(gradient_payload),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_selection_color_occurrences_select(row.id.clone(), cx);
    });
    visual_cx.run_until_parked();

    let expected_target = DesignPanelTarget::Nodes {
        node_ids: vec!["two".into(), "one".into()],
    };
    let expected_references = vec![first_reference, second_reference];
    let captured = captured.borrow();
    assert_eq!(captured.len(), 4);
    for action in captured.iter() {
        match action {
            DesignPanelAction::SelectionColorPaintEditRequested {
                target,
                selection_color_id,
                paint_references,
                ..
            }
            | DesignPanelAction::SelectionColorOccurrencesSelectRequested {
                target,
                selection_color_id,
                paint_references,
            } => {
                assert_eq!(target, &expected_target);
                assert_eq!(selection_color_id, &row.id);
                assert_eq!(paint_references, &expected_references);
            }
            action => panic!("unexpected Selection colors intent: {action:?}"),
        }
    }
    assert!(matches!(
        &captured[0],
        DesignPanelAction::SelectionColorPaintEditRequested {
            edit: DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(42.),
            },
            ..
        }
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::SelectionColorPaintEditRequested {
            edit: DesignPaintEdit {
                property: DesignPaintProperty::BlendMode,
                value: DesignPaintValue::BlendMode(DesignBlendMode::Multiply),
            },
            ..
        }
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::SelectionColorPaintEditRequested {
            edit: DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                ..
            },
            ..
        }
    ));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.resolved_selection_colors()[0]
            .paint
            .clone()),
        representative,
        "the reusable panel keeps the host paint snapshot controlled"
    );
}

#[gpui::test]
fn selection_paint_picker_rebases_references_on_same_target_host_echo(cx: &mut TestAppContext) {
    let old_reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "stable-fill",
        4,
    );
    let new_reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "stable-fill",
        1,
    );
    let row =
        super::super::DesignSelectionColor::new("stable-row", DesignColor::BLUE, [old_reference])
            .with_paint(DesignPaint::solid(DesignColor::BLUE).with_id("stable-fill"));
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate = super::super::DesignSelectionColors::new([row.clone()]);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = AuxiliaryColorPickerTarget::SelectionColor {
        target: DesignPanelTarget::Nodes {
            node_ids: vec!["one".into(), "two".into()],
        },
        selection_color_id: row.id.clone(),
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.auxiliary_color_picker = Some(target.clone());

        let echoed_row = super::super::DesignSelectionColor::new(
            row.id.clone(),
            DesignColor::BLUE,
            [new_reference.clone()],
        )
        .with_paint(DesignPaint::solid(DesignColor::BLUE).with_id("stable-fill"));
        first.selection_color_aggregate = super::super::DesignSelectionColors::new([echoed_row]);
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(panel.auxiliary_color_picker.as_ref(), Some(&target));
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(64.),
            },
            DesignPanelEditPhase::Preview,
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::SelectionColorPaintEditRequested {
            paint_references,
            edit: DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(64.),
            },
            phase: DesignPanelEditPhase::Preview,
            ..
        }] if paint_references == &vec![new_reference]
    ));
}

#[gpui::test]
fn selection_occurrence_target_remains_viewer_safe_while_paint_edits_are_gated(
    cx: &mut TestAppContext,
) {
    let reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "one-fill",
        0,
    );
    let row = super::super::DesignSelectionColor::new(
        "stable-row",
        DesignColor::BLUE,
        [reference.clone()],
    )
    .with_paint(DesignPaint::solid(DesignColor::BLUE).with_id("one-fill"));
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate = super::super::DesignSelectionColors::new([row.clone()]);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.auxiliary_color_picker = Some(AuxiliaryColorPickerTarget::SelectionColor {
            target: DesignPanelTarget::Nodes {
                node_ids: vec!["one".into(), "two".into()],
            },
            selection_color_id: row.id.clone(),
        });
        panel.emit_auxiliary_color_edit(
            &DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(30.),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_selection_color_occurrences_select(row.id.clone(), cx);
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::SelectionColorOccurrencesSelectRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            selection_color_id,
            paint_references,
        }] if node_ids == &vec![SharedString::from("one"), SharedString::from("two")]
            && selection_color_id == &row.id
            && paint_references == &vec![reference]
    ));
}

#[gpui::test]
fn selection_color_resources_emit_exact_targeted_style_and_variable_intents(
    cx: &mut TestAppContext,
) {
    let first_reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "one-fill",
        0,
    );
    let second_reference = super::super::DesignSelectionPaintReference::paint(
        "two",
        super::super::DesignSelectionPaintCollection::Stroke,
        "two-stroke",
        0,
    );
    let row = super::super::DesignSelectionColor::new(
        "aggregate-blue",
        DesignColor::BLUE,
        [first_reference.clone(), second_reference.clone()],
    )
    .with_paint(DesignPaint::solid(DesignColor::BLUE).with_id("one-fill"))
    .with_style_context(
        [
            DesignPaint::solid(DesignColor::BLUE).with_id("one-fill"),
            DesignPaint::solid(DesignColor::WHITE).with_id("one-highlight"),
        ],
        None,
    );
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate = super::super::DesignSelectionColors::new([row.clone()]);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let local_style = super::super::DesignPaintStyleSelection::page("local-style");
    let remote_style = super::super::DesignPaintStyleSelection::library("library", "remote-style");
    let paint_styles = super::super::DesignPaintStyleViewData::new(
        [super::super::DesignPaintStyle::new(
            "local-style",
            "Local",
            [DesignPaint::solid(DesignColor::PURPLE)],
        )],
        [super::super::DesignPaintStyleLibrary::new(
            "library",
            "Library",
            [super::super::DesignPaintStyle::new(
                "remote-style",
                "Remote",
                [DesignPaint::solid(DesignColor::BLACK)],
            )
            .with_import_state(super::super::DesignPaintStyleImportState::Available)],
        )],
    );
    let paint_variables = super::super::DesignPaintVariableViewData::new([
        super::super::DesignVariable::page(
            "local-variable",
            "Local variable",
            "colors",
            "Colors",
            super::super::DesignVariableResolvedType::Color,
        )
        .with_resolved_value(super::super::DesignVariableResolvedValue::Color(
            DesignColor::PURPLE,
        )),
        super::super::DesignVariable::page(
            "remote-variable",
            "Remote variable",
            "colors",
            "Colors",
            super::super::DesignVariableResolvedType::Color,
        )
        .with_source(super::super::DesignVariableSource::library(
            "library", "Library",
        ))
        .with_import_state(super::super::DesignVariableImportState::Available)
        .with_resolved_value(super::super::DesignVariableResolvedValue::Color(
            DesignColor::BLACK,
        )),
    ]);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_paint_style_view_data(paint_styles, cx);
        panel.set_paint_variable_view_data(paint_variables, cx);
        panel.emit_selection_color_paint_style_apply(row.id.clone(), local_style.clone(), cx);
        panel.emit_selection_color_paint_style_import(row.id.clone(), remote_style.clone(), cx);
        panel.emit_selection_color_paint_style_create(row.id.clone(), cx);
        panel.emit_selection_color_variable_apply(row.id.clone(), "local-variable".into(), cx);
        panel.emit_selection_color_variable_import(row.id.clone(), "remote-variable".into(), cx);
        panel.emit_selection_color_variable_create(row.id.clone(), cx);
    });
    visual_cx.run_until_parked();

    let expected_target = DesignPanelTarget::Nodes {
        node_ids: vec!["one".into(), "two".into()],
    };
    let expected_references = vec![first_reference.clone(), second_reference.clone()];
    let captured = captured.borrow();
    assert_eq!(captured.len(), 6);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::SelectionColorPaintStyleApplyRequested {
            target,
            selection_color_id,
            paint_references,
            style,
        } if target == &expected_target
            && selection_color_id == &row.id
            && paint_references == &expected_references
            && style == &local_style
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::SelectionColorPaintStyleImportRequested {
            target,
            paint_references,
            style,
            ..
        } if target == &expected_target
            && paint_references == &expected_references
            && style == &remote_style
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::SelectionColorPaintStyleCreateRequested {
            target,
            paint_references,
            paints,
            ..
        } if target == &expected_target
            && paint_references == &expected_references
            && paints.iter().map(|paint| paint.id.as_ref()).collect::<Vec<_>>()
                == vec!["one-fill", "one-highlight"]
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::SelectionColorVariableApplyRequested {
            target,
            paint_references,
            variable_id,
            ..
        } if target == &expected_target
            && paint_references == &expected_references
            && variable_id.as_ref() == "local-variable"
    ));
    assert!(matches!(
        &captured[4],
        DesignPanelAction::SelectionColorVariableImportRequested {
            target,
            variable_id,
            ..
        } if target == &expected_target && variable_id.as_ref() == "remote-variable"
    ));
    assert!(matches!(
        &captured[5],
        DesignPanelAction::SelectionColorVariableCreateRequested {
            target,
            color: DesignColor::BLUE,
            ..
        } if target == &expected_target
    ));
}

#[gpui::test]
fn selection_color_bound_and_read_only_rows_gate_the_correct_resource_level(
    cx: &mut TestAppContext,
) {
    let reference = super::super::DesignSelectionPaintReference::paint(
        "one",
        super::super::DesignSelectionPaintCollection::Fill,
        "one-fill",
        0,
    );
    let style = super::super::DesignPaintStyleSelection::page("surface");
    let base_row = super::super::DesignSelectionColor::new(
        "aggregate-blue",
        DesignColor::BLUE,
        [reference.clone()],
    )
    .with_style_context([DesignPaint::solid(DesignColor::BLUE)], None);
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.selection_color_aggregate = super::super::DesignSelectionColors::new([base_row.clone()]);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = AuxiliaryColorPickerTarget::SelectionColor {
        target: DesignPanelTarget::Nodes {
            node_ids: vec!["one".into(), "two".into()],
        },
        selection_color_id: base_row.id.clone(),
    };

    panel.update(visual_cx, |panel, cx| {
        let style_bound = base_row.clone().with_style_context(
            [DesignPaint::solid(DesignColor::BLUE)],
            Some(super::super::DesignPaintStyleBinding::new(
                style.clone(),
                "Surface",
            )),
        );
        first.selection_color_aggregate = super::super::DesignSelectionColors::new([style_bound]);
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            panel.auxiliary_color_leaf_editability(&target),
            (false, false),
            "a collection Paint style owns the complete paint"
        );
        assert!(!panel.auxiliary_color_editable(&target));
        panel.emit_selection_color_variable_create(base_row.id.clone(), cx);
        panel.emit_selection_color_paint_style_detach(base_row.id.clone(), cx);

        let variable_bound = base_row
            .clone()
            .with_binding(super::super::DesignPaintBinding::new(
                "brand-variable",
                "Brand",
            ));
        first.selection_color_aggregate =
            super::super::DesignSelectionColors::new([variable_bound]);
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert_eq!(
            panel.auxiliary_color_leaf_editability(&target),
            (false, true),
            "a Solid Color-variable locks the color leaf without conflating paint opacity"
        );
        assert!(panel.auxiliary_color_editable(&target));
        panel.emit_selection_color_variable_create(base_row.id.clone(), cx);
        panel.emit_selection_color_variable_detach(base_row.id.clone(), cx);

        first.selection_color_aggregate =
            super::super::DesignSelectionColors::new([base_row.clone().read_only(true)]);
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.emit_selection_color_paint_style_create(base_row.id.clone(), cx);
        panel.emit_selection_color_variable_create(base_row.id.clone(), cx);
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::SelectionColorPaintStyleDetachRequested {
                style: detached_style,
                ..
            },
            DesignPanelAction::SelectionColorVariableDetachRequested {
                variable_id,
                ..
            }
        ] if detached_style == &style && variable_id.as_ref() == "brand-variable"
    ));
}

#[gpui::test]
fn host_echo_preserves_viewer_and_multiple_inspection_context(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::Width,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        let mut echo = first.clone();
        echo.opacity = 72.;
        panel.set_node(echo, cx);
        assert_eq!(
            panel.inspection_context.selection().kind(),
            DesignPanelSelectionKind::Multiple
        );
        assert!(
            panel
                .property_value_states
                .get(&DesignPanelProperty::Width)
                .is_some_and(DesignPanelPropertyValueState::is_mixed)
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                first.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_node(first, cx);
        assert!(!panel.can_edit());
        assert_eq!(
            panel.inspection_context.permissions().access_mode(),
            super::super::DesignPanelAccessMode::View
        );
    });
}

#[gpui::test]
fn typography_rules_gate_vertical_alignment_and_max_lines(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.property_is_editable(DesignPanelProperty::VerticalTextAlignment));
        assert!(!panel.property_is_editable(DesignPanelProperty::TextMaxLines));

        let mut fixed = node;
        let typography = fixed.typography.as_mut().expect("text typography");
        typography.resize = DesignTextResize::Fixed;
        typography.truncate = true;
        typography.max_lines = Some(3);
        panel.set_node(fixed, cx);

        assert!(panel.property_is_editable(DesignPanelProperty::VerticalTextAlignment));
        assert!(!panel.property_is_editable(DesignPanelProperty::TextMaxLines));
        assert!(!panel.text_max_lines_are_available());

        let mut auto_height = panel.node.clone();
        let typography = auto_height.typography.as_mut().expect("text typography");
        typography.resize = DesignTextResize::AutoHeight;
        panel.set_node(auto_height.clone(), cx);

        assert!(!panel.property_is_editable(DesignPanelProperty::VerticalTextAlignment));
        assert!(panel.property_is_editable(DesignPanelProperty::TextMaxLines));
        assert!(panel.text_max_lines_are_available());

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                auto_height.clone(),
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Vertical,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::TextMaxLines));
        assert!(!panel.text_max_lines_are_available());

        auto_height
            .layout
            .as_mut()
            .expect("text layout")
            .vertical_sizing = DesignSizingMode::Hug;
        panel.set_node(auto_height, cx);
        assert!(panel.property_is_editable(DesignPanelProperty::TextMaxLines));
        assert!(panel.text_max_lines_are_available());
    });

    assert_eq!(format_text_max_lines(None), "Auto");
    assert_eq!(format_text_max_lines(Some(3)), "3");
    assert_eq!(
        DesignPanel::property_clamp(DesignPanelProperty::FontSize)
            .expect("font-size clamp")
            .apply(0.)
            .expect("finite value"),
        1.
    );
}

#[gpui::test]
fn typography_intents_identify_whole_layer_and_selected_range(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_property(
            DesignPanelProperty::TextCase,
            DesignPanelValue::TextCase(DesignTextCase::Uppercase),
            cx,
        );
    });
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::WholeLayer,
            property: DesignPanelProperty::TextCase,
            ..
        }
    )));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        let mut node = node;
        let typography = node.typography.as_mut().expect("text typography");
        typography.resize = DesignTextResize::AutoHeight;
        typography.truncate = true;
        node.layout.as_mut().expect("text layout").vertical_sizing = DesignSizingMode::Hug;
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::auto_layout(
                    DesignPanelAutoLayoutDirection::Vertical,
                    super::super::DesignPanelAutoLayoutWrap::NoWrap,
                    DesignPanelAutoLayoutParticipation::InFlow,
                ),
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("text edit mode"),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::FontSize,
            DesignPanelValue::Number(18.),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::TextLeadingTrim,
            DesignPanelValue::TextLeadingTrim(DesignTextLeadingTrim::CapHeight),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::TextResize,
            DesignPanelValue::TextResize(DesignTextResize::Fixed),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::TextMaxLines,
            DesignPanelValue::OptionalNumber(Some(3.)),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::MaxHeight,
            DesignPanelValue::OptionalNumber(Some(96.)),
            cx,
        );
    });
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            property: DesignPanelProperty::FontSize,
            value: DesignPanelValue::Number(18.),
            ..
        }
    )));
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::WholeLayer,
            property: DesignPanelProperty::TextResize,
            value: DesignPanelValue::TextResize(DesignTextResize::Fixed),
            ..
        }
    )));
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            property: DesignPanelProperty::TextLeadingTrim,
            value: DesignPanelValue::TextLeadingTrim(DesignTextLeadingTrim::CapHeight),
            ..
        }
    )));
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            target: DesignTypographyTarget::WholeLayer,
            property: DesignPanelProperty::TextMaxLines,
            value: DesignPanelValue::OptionalNumber(Some(3.)),
            ..
        }
    )));
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::MaxHeight,
            value: DesignPanelValue::OptionalNumber(Some(96.)),
            ..
        }
    )));
}

#[gpui::test]
fn font_browser_emits_exact_apply_and_import_intents_and_rejects_unavailable_rows(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let local = crate::design::DesignFontSelection::local("inter", "regular");
    let library = crate::design::DesignFontSelection::library(
        "type-library",
        "Type Library",
        "archivo",
        "bold",
    );
    let missing = crate::design::DesignFontSelection::local("missing", "regular");
    let catalog = DesignFontViewData::ready([
        crate::design::DesignFontFamily::local(
            "inter",
            "Inter",
            [crate::design::DesignFontStyle::imported(
                "regular", "Regular",
            )],
        ),
        crate::design::DesignFontFamily {
            id: "archivo".into(),
            name: "Archivo".into(),
            source: crate::design::DesignFontSource::Library {
                library_id: "type-library".into(),
                library_name: "Type Library".into(),
            },
            styles: vec![crate::design::DesignFontStyle {
                id: "bold".into(),
                name: "Bold".into(),
                weight: Some(700),
                italic: false,
                availability: DesignFontAvailability::Available,
                preview: None,
            }],
        },
        crate::design::DesignFontFamily::local(
            "missing",
            "Missing Sans",
            [crate::design::DesignFontStyle {
                id: "regular".into(),
                name: "Regular".into(),
                weight: None,
                italic: false,
                availability: DesignFontAvailability::Missing {
                    reason: "Not installed".into(),
                },
                preview: None,
            }],
        ),
    ]);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_font_view_data(catalog, cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("text edit mode"),
            cx,
        );
        panel.emit_font_apply(local.clone(), cx);
        panel.emit_font_import(library.clone(), cx);
        panel.emit_font_apply(library.clone(), cx);
        panel.emit_font_apply(missing.clone(), cx);
        panel.emit_font_import(missing.clone(), cx);
    });

    let actions = captured_actions.borrow();
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyFontApplyRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            font,
            ..
        } if font == &local
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyFontImportRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            font,
            ..
        } if font == &library
    )));
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::TypographyFontApplyRequested { .. }
                    | DesignPanelAction::TypographyFontImportRequested { .. }
            ))
            .count(),
        2,
    );
}

#[gpui::test]
fn typography_details_are_lossless_host_controlled_and_respect_read_only_states(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let typography = node.typography.as_mut().expect("text typography");
    typography.paragraph_indent = 12.;
    typography.list = DesignTextList::Bulleted;
    typography.list_spacing = 8.;
    typography.hanging_punctuation = true;
    typography.hanging_lists = true;
    typography.decoration = DesignTextDecoration::Underline;
    typography.decoration_details = Some(crate::design::DesignTextDecorationDetails {
        style: DesignTextDecorationStyle::Wavy,
        offset: DesignTextDecorationMetric::Pixels(2.),
        thickness: DesignTextDecorationMetric::Percent(120.),
        color: DesignTextDecorationColor::Solid(DesignColor::BLUE),
        skip_ink: true,
    });
    let liga = DesignOpenTypeFeatureTag::registered("liga").expect("liga is a registered tag");
    let future = DesignOpenTypeFeatureTag::opaque("future:dots");
    typography.open_type_features = vec![
        crate::design::DesignOpenTypeFeature::new(liga.clone(), "Standard ligatures", true, true),
        crate::design::DesignOpenTypeFeature::new(future.clone(), "Future dots", false, false)
            .unavailable("Unsupported"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        for (property, value) in [
            (
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(12.),
            ),
            (
                DesignPanelProperty::TextHangingPunctuation,
                DesignPanelValue::Bool(true),
            ),
            (
                DesignPanelProperty::TextHangingLists,
                DesignPanelValue::Bool(true),
            ),
            (
                DesignPanelProperty::ListSpacing,
                DesignPanelValue::Number(8.),
            ),
            (
                DesignPanelProperty::TextDecorationStyle,
                DesignPanelValue::TextDecorationStyle(DesignTextDecorationStyle::Wavy),
            ),
            (
                DesignPanelProperty::TextDecorationOffset,
                DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Pixels(2.)),
            ),
            (
                DesignPanelProperty::TextDecorationThickness,
                DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Percent(120.)),
            ),
            (
                DesignPanelProperty::TextDecorationColor,
                DesignPanelValue::TextDecorationColor(DesignTextDecorationColor::Solid(
                    DesignColor::BLUE,
                )),
            ),
            (
                DesignPanelProperty::TextDecorationSkipInk,
                DesignPanelValue::Bool(true),
            ),
        ] {
            assert_eq!(panel.current_property_value(property), Some(value));
        }

        panel.set_property_value_state(
            DesignPanelProperty::TextDecorationColor,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::TextDecorationColor(
                DesignTextDecorationColor::Solid(DesignColor::BLUE),
            ))
            .read_only_with_reason("Decoration color is bound"),
            cx,
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::TextDecorationColor));
        panel.type_settings_open = true;
        panel.type_settings_tab = TypographySettingsTab::Details;
        drop(panel.render_typography(cx));

        for (property, value) in [
            (
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(20.),
            ),
            (
                DesignPanelProperty::TextHangingPunctuation,
                DesignPanelValue::Bool(false),
            ),
            (
                DesignPanelProperty::TextHangingLists,
                DesignPanelValue::Bool(false),
            ),
            (
                DesignPanelProperty::ListSpacing,
                DesignPanelValue::Number(12.),
            ),
            (
                DesignPanelProperty::TextDecorationStyle,
                DesignPanelValue::TextDecorationStyle(DesignTextDecorationStyle::Dotted),
            ),
            (
                DesignPanelProperty::TextDecorationOffset,
                DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Pixels(-1.)),
            ),
            (
                DesignPanelProperty::TextDecorationThickness,
                DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Pixels(3.)),
            ),
            (
                DesignPanelProperty::TextDecorationSkipInk,
                DesignPanelValue::Bool(false),
            ),
        ] {
            panel.emit_property(property, value, cx);
        }
        panel.emit_property(
            DesignPanelProperty::TextDecorationColor,
            DesignPanelValue::TextDecorationColor(DesignTextDecorationColor::Auto),
            cx,
        );
        panel.emit_open_type_feature(liga.clone(), false, cx);
        panel.emit_open_type_feature(future.clone(), true, cx);
    });

    let actions = captured_actions.borrow();
    for (property, value) in [
        (
            DesignPanelProperty::ParagraphIndent,
            DesignPanelValue::Number(20.),
        ),
        (
            DesignPanelProperty::TextHangingPunctuation,
            DesignPanelValue::Bool(false),
        ),
        (
            DesignPanelProperty::TextHangingLists,
            DesignPanelValue::Bool(false),
        ),
        (
            DesignPanelProperty::ListSpacing,
            DesignPanelValue::Number(12.),
        ),
        (
            DesignPanelProperty::TextDecorationStyle,
            DesignPanelValue::TextDecorationStyle(DesignTextDecorationStyle::Dotted),
        ),
        (
            DesignPanelProperty::TextDecorationOffset,
            DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Pixels(-1.)),
        ),
        (
            DesignPanelProperty::TextDecorationThickness,
            DesignPanelValue::TextDecorationMetric(DesignTextDecorationMetric::Pixels(3.)),
        ),
        (
            DesignPanelProperty::TextDecorationSkipInk,
            DesignPanelValue::Bool(false),
        ),
    ] {
        assert!(actions.iter().any(|action| matches!(
            action,
            DesignPanelAction::TypographyPropertyChangeRequested {
                target: DesignTypographyTarget::WholeLayer,
                property: emitted_property,
                value: emitted_value,
                ..
            } if emitted_property == &property && emitted_value == &value
        )));
    }
    assert!(!actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyPropertyChangeRequested {
            property: DesignPanelProperty::TextDecorationColor,
            ..
        }
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyOpenTypeFeatureChangeRequested {
            target: DesignTypographyTarget::WholeLayer,
            tag,
            enabled: false,
            ..
        } if tag == &liga
    )));
    assert!(!actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyOpenTypeFeatureChangeRequested { tag, .. }
            if tag == &future
    )));
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .typography
                .as_ref()
                .expect("text typography")
                .paragraph_indent
        }),
        12.,
        "detail controls must leave host view data unchanged"
    );
}

#[gpui::test]
fn paragraph_indent_uses_the_continuous_typography_edit_lifecycle(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .paragraph_indent = 12.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+4", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    let edits = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyPropertyEditRequested {
                target,
                property,
                value,
                phase,
                ..
            } => Some((*target, *property, value.clone(), *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (
                DesignTypographyTarget::WholeLayer,
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(12.),
                DesignPanelEditPhase::Begin,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(16.),
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignTypographyTarget::WholeLayer,
                DesignPanelProperty::ParagraphIndent,
                DesignPanelValue::Number(16.),
                DesignPanelEditPhase::Commit,
            ),
        ]
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .typography
                .as_ref()
                .expect("text typography")
                .paragraph_indent
        }),
        12.,
        "the numeric detail editor must remain host controlled"
    );
}

#[gpui::test]
fn paragraph_indent_and_list_spacing_follow_current_text_context_gates(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let typography = node.typography.as_mut().expect("text typography");
    typography.horizontal_alignment = DesignTextHorizontalAlignment::Center;
    typography.list = DesignTextList::None;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.property_is_editable(DesignPanelProperty::ParagraphIndent));
        assert!(!panel.property_is_editable(DesignPanelProperty::ListSpacing));

        let mut next = panel.node.clone();
        let typography = next.typography.as_mut().expect("text typography");
        typography.horizontal_alignment = DesignTextHorizontalAlignment::Left;
        typography.list = DesignTextList::Numbered;
        panel.set_node(next, cx);

        assert!(panel.property_is_editable(DesignPanelProperty::ParagraphIndent));
        assert!(panel.property_is_editable(DesignPanelProperty::ListSpacing));
    });
}

#[gpui::test]
fn list_spacing_cancel_emits_a_complete_selected_range_edit_lifecycle(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let typography = node.typography.as_mut().expect("text typography");
    typography.list = DesignTextList::Bulleted;
    typography.list_spacing = 8.;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node,
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                )
                .with_edit_mode(DesignPanelEditMode::Text)
                .expect("text edit mode"),
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::ListSpacing,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+4", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(false, window, cx);
        });
    });

    let phases = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyPropertyEditRequested {
                target,
                property: DesignPanelProperty::ListSpacing,
                phase,
                ..
            } => Some((*target, *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            (
                DesignTypographyTarget::SelectedTextRange,
                DesignPanelEditPhase::Begin,
            ),
            (
                DesignTypographyTarget::SelectedTextRange,
                DesignPanelEditPhase::Preview,
            ),
            (
                DesignTypographyTarget::SelectedTextRange,
                DesignPanelEditPhase::Cancel,
            ),
        ]
    );
}

#[gpui::test]
fn typography_style_picker_is_retained_but_closes_when_entering_native_viewer_projection(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let picker = visual_cx.read(|app| panel.read(app).typography_style_picker.clone());

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_typography_style_picker(window, cx);
            assert!(panel.typography_style_picker_open);
            drop(panel.render_typography(cx));

            panel.set_node(node.clone(), cx);
            assert!(panel.typography_style_picker_open);
            assert_eq!(panel.typography_style_picker, picker);

            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node,
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::viewer(),
                ),
                cx,
            );
            panel.sync_typography_style_picker(cx);
            assert!(
                !panel.typography_style_picker_open,
                "the viewer projection cannot retain an unreachable editor popover"
            );
            assert_eq!(
                panel.typography_style_picker, picker,
                "the retained presentation entity remains reusable after returning to edit"
            );
        });
        assert!(picker.read(app).is_disabled());
    });
    visual_cx.run_until_parked();
    visual_cx.simulate_keystrokes("escape");
    visual_cx.run_until_parked();

    assert!(!visual_cx.read(|app| panel.read(app).typography_style_picker_open));
}

#[gpui::test]
fn bound_text_style_replaces_only_style_owned_typography_leaves(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("path", "Text path", DesignPanelNodeKind::TextPath);
    node.typography
        .as_mut()
        .expect("TextPath typography")
        .style_binding = Some(super::super::DesignTypographyStyleBinding::new(
        super::super::DesignTypographyStyleSelection::page("body"),
        "Body / Default",
    ));
    node.text_path_start_data = super::super::DesignTextPathStartData::new(2, 0.35);
    node.capabilities = Some(
        super::super::DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::TextPath)
            .with_sections([DesignPanelSection::Typography]),
    );
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.typography_style_binding().map(|binding| binding.name),
            Some("Body / Default".into()),
            "the resolved Text style name remains inspectable"
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TextPathStartSegment),
            Some(DesignPanelValue::Integer(2)),
            "TextPath placement is independent from the Text style"
        );
        drop(panel.render_typography(cx));
        drop(panel.render_typography_style_button(cx));
    });
}

#[gpui::test]
fn typography_style_picker_emits_exact_host_controlled_apply_and_detach_intents(
    cx: &mut TestAppContext,
) {
    let page_selection = super::super::DesignTypographyStyleSelection::page("page-body");
    let library_selection =
        super::super::DesignTypographyStyleSelection::library("acme", "library-body");
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .style_binding = Some(super::super::DesignTypographyStyleBinding::new(
        page_selection.clone(),
        "Body",
    ));
    let view_data = super::super::DesignTypographyStyleViewData::new(
        [super::super::DesignTypographyStyle::new(
            "page-body",
            "Body",
            "Inter",
            "Regular",
            16.,
        )],
        [super::super::DesignTypographyStyleLibrary::new(
            "acme",
            "Acme",
            [super::super::DesignTypographyStyle::new(
                "library-body",
                "Body",
                "Source Sans",
                "Regular",
                18.,
            )],
        )],
    );
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_typography_style_view_data(view_data.clone(), cx);
        assert_eq!(panel.typography_style_view_data(), &view_data);
        panel.sync_typography_style_picker(cx);
    });
    let picker = visual_cx.read(|app| panel.read(app).typography_style_picker.clone());
    picker.update(visual_cx, |picker, cx| {
        picker.request_apply(library_selection.clone(), cx);
        picker.request_detach(cx);
    });
    visual_cx.run_until_parked();

    let actions = captured_actions.borrow();
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyStyleApplyRequested {
            node_id,
            target: DesignTypographyTarget::WholeLayer,
            style,
        } if node_id.as_ref() == "text" && style == &library_selection
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyStyleDetachRequested {
            node_id,
            target: DesignTypographyTarget::WholeLayer,
            style,
        } if node_id.as_ref() == "text" && style == &page_selection
    )));
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .typography
                .as_ref()
                .expect("text typography")
                .style_binding
                .as_ref()
                .expect("controlled binding")
                .selection
                .clone()
        }),
        page_selection,
        "picker intents must not mutate the controlled binding"
    );
}

#[gpui::test]
fn typography_style_picker_targets_text_ranges_and_obeys_read_only_gates(cx: &mut TestAppContext) {
    let selection = super::super::DesignTypographyStyleSelection::page("display");
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography
        .as_mut()
        .expect("text typography")
        .style_binding = Some(super::super::DesignTypographyStyleBinding::new(
        selection.clone(),
        "Display",
    ));
    let view_data = super::super::DesignTypographyStyleViewData::new(
        [super::super::DesignTypographyStyle::new(
            "display", "Display", "Inter", "Bold", 48.,
        )],
        [],
    );
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_typography_style_view_data(view_data, cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("text edit mode"),
            cx,
        );
        panel.sync_typography_style_picker(cx);
    });
    let picker = visual_cx.read(|app| panel.read(app).typography_style_picker.clone());
    picker.update(visual_cx, |picker, cx| {
        picker.request_apply(selection.clone(), cx);
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::TypographyStyleApplyRequested {
            target: DesignTypographyTarget::SelectedTextRange,
            style,
            ..
        } if style == &selection
    )));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::TypographyStyle,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::TypographyStyle(Some(
                selection.clone(),
            )))
            .read_only_with_reason("Typography style is locked"),
            cx,
        );
        panel.sync_typography_style_picker(cx);
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_apply(selection.clone(), cx);
        picker.request_detach(cx);
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().is_empty());

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states([], cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.sync_typography_style_picker(cx);
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_apply(selection.clone(), cx);
        picker.request_detach(cx);
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().is_empty());
}

#[gpui::test]
fn type_settings_renders_both_interactive_tabs_and_preserves_secondary_intents(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let typography = node.typography.as_mut().expect("text typography");
    typography.resize = DesignTextResize::AutoHeight;
    typography.truncate = true;
    typography.max_lines = Some(2);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.type_settings_open = true;
        panel.type_settings_tab = TypographySettingsTab::Basics;
        drop(panel.render_typography(cx));
        panel.type_settings_tab = TypographySettingsTab::Details;
        drop(panel.render_typography(cx));

        for (property, value) in [
            (
                DesignPanelProperty::TextCase,
                DesignPanelValue::TextCase(DesignTextCase::Uppercase),
            ),
            (
                DesignPanelProperty::TextDecoration,
                DesignPanelValue::TextDecoration(DesignTextDecoration::Underline),
            ),
            (
                DesignPanelProperty::ParagraphSpacing,
                DesignPanelValue::Number(8.),
            ),
            (
                DesignPanelProperty::TextList,
                DesignPanelValue::TextList(DesignTextList::Bulleted),
            ),
            (
                DesignPanelProperty::TextResize,
                DesignPanelValue::TextResize(DesignTextResize::AutoHeight),
            ),
            (
                DesignPanelProperty::TextTruncate,
                DesignPanelValue::Bool(false),
            ),
            (
                DesignPanelProperty::TextMaxLines,
                DesignPanelValue::OptionalNumber(Some(3.)),
            ),
        ] {
            panel.emit_property(property, value, cx);
        }
    });

    let secondary_properties = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyPropertyChangeRequested { property, .. } => {
                Some(*property)
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    for property in [
        DesignPanelProperty::TextCase,
        DesignPanelProperty::TextDecoration,
        DesignPanelProperty::ParagraphSpacing,
        DesignPanelProperty::TextList,
        DesignPanelProperty::TextResize,
        DesignPanelProperty::TextTruncate,
        DesignPanelProperty::TextMaxLines,
    ] {
        assert!(secondary_properties.contains(&property));
    }
}

#[gpui::test]
fn type_setting_number_field_keeps_its_native_trigger_and_restores_it(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    host.update(visual_cx, |host, cx| {
        host.render_type_setting_probe = true;
        cx.notify();
    });
    panel.update(visual_cx, |panel, cx| {
        panel.type_settings_open = true;
        panel.type_settings_tab = TypographySettingsTab::Basics;
        cx.notify();
    });
    visual_cx.run_until_parked();

    let selector = "focus-probe-type-setting-paragraph-spacing-number-activate";
    let paragraph_spacing = visual_cx
        .debug_bounds(selector)
        .expect("Paragraph spacing native Button")
        .center();
    visual_cx.simulate_click(paragraph_spacing, Modifiers::none());
    visual_cx.run_until_parked();
    let return_handle = visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.property_editor.as_ref().map(|editor| editor.property),
            Some(DesignPanelProperty::ParagraphSpacing)
        );
        let return_focus = panel
            .editor_focus_return
            .as_ref()
            .expect("Type Setting activation should retain its Button");
        assert_eq!(
            return_focus.origin,
            EditorFocusOrigin::TypeSetting(DesignPanelProperty::ParagraphSpacing)
        );
        return_focus.handle.clone()
    });
    assert!(
        visual_cx.debug_bounds(selector).is_some(),
        "the same native Button remains mounted invisibly while its Input is active"
    );

    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property_from_control(
                EditorFocusOrigin::TypeSetting(DesignPanelProperty::ParagraphSpacing),
                DesignPanelProperty::ParagraphSpacing,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
            assert_eq!(
                panel.property_editor.as_ref().map(|editor| editor.property),
                Some(DesignPanelProperty::ParagraphSpacing)
            );
        });
    });
    visual_cx.run_until_parked();

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(false, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));
}

#[gpui::test]
fn escape_closes_transient_type_settings(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.type_settings_open = true;
            panel.type_settings_tab = TypographySettingsTab::Details;
            panel.focus_handle.focus(window, cx);
            cx.notify();
        });
    });
    visual_cx.run_until_parked();
    visual_cx.simulate_keystrokes("escape");
    visual_cx.run_until_parked();

    assert!(!visual_cx.read(|app| panel.read(app).type_settings_open));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).type_settings_tab),
        TypographySettingsTab::Basics
    );
}

#[gpui::test]
fn mixed_numeric_state_opens_an_empty_controlled_editor(cx: &mut TestAppContext) {
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.width = 100.;
    let mut second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Rectangle);
    second.width = 240.;
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                super::super::DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::Width,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Width,
                DesignPanelValue::Number(0.),
                window,
                cx,
            );
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    assert_eq!(
        visual_cx.read(|app| input.read(app).value().to_string()),
        ""
    );

    visual_cx.update(|window, app| {
        input.update(app, |input, cx| input.set_value("180", window, cx));
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action.targeted_node_action(),
        Some((
            DesignPanelTarget::Nodes { node_ids },
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Width,
                value: DesignPanelValue::Number(180.),
                phase: DesignPanelEditPhase::Commit,
                ..
            },
        )) if node_ids == &vec![SharedString::from("one"), SharedString::from("two")]
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.width),
        100.,
        "mixed edits must remain host controlled"
    );
}

#[gpui::test]
fn bound_generic_property_cannot_be_overwritten_before_detach(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Opacity,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "variable-opacity",
                "Opacity / Muted",
                super::super::DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(64.),
            )),
            cx,
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::Opacity));
        panel.emit_property(
            DesignPanelProperty::Opacity,
            DesignPanelValue::Number(100.),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().is_empty());
}

#[gpui::test]
fn retained_picker_syncs_and_emits_host_controlled_paint_changes(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let original = node.fills[0].clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: original.id.clone(),
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(target.clone());
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    assert_eq!(
        visual_cx.read(|app| picker.read(app).paint().cloned()),
        Some(original.clone())
    );

    let stale_candidate = DesignPaint::solid(DesignColor::BLACK);
    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::Edit {
            target: super::super::paint_picker::PaintPickerTarget {
                node_id: "other-node".into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: original.id.clone(),
            },
            edit: Box::new(DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                value: DesignPaintValue::Payload(stale_candidate.payload.clone()),
            }),
            phase: DesignPanelEditPhase::Commit,
        });
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "events for a stale controlled target must be ignored"
    );

    let candidate = DesignPaint::solid(DesignColor::rgb(0x14, 0xae, 0x5c));
    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::Edit {
            target: super::super::paint_picker::PaintPickerTarget {
                node_id: "rectangle".into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: original.id.clone(),
            },
            edit: Box::new(DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                value: DesignPaintValue::Payload(candidate.payload.clone()),
            }),
            phase: DesignPanelEditPhase::Commit,
        });
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintEditRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            index: 0,
            edit: DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                value: DesignPaintValue::Payload(payload),
            },
            phase: DesignPanelEditPhase::Commit,
            ..
        } if node_id.as_ref() == "rectangle" && *payload == candidate.payload
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills[0].clone()),
        original,
        "picker candidates must not mutate host view data"
    );
}

#[gpui::test]
fn paint_shader_edits_revalidate_current_definition_id_and_value_kind(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("shader-node", "Shader", DesignPanelNodeKind::Rectangle);
    node.fills = vec![DesignPaint::shader("noise", "Noise").with_id("shader-fill")];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let shader_view_data = DesignShaderViewData::new(
        [DesignShaderDefinition::new(
            "noise",
            "Noise",
            true,
            [DesignShaderPropertyDefinition::new(
                "scale",
                "Scale",
                DesignShaderPropertyKind::Number,
            )
            .with_default(DesignShaderPropertyValue::Number(1.))],
        )],
        [],
    );

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_shader_view_data(shader_view_data, cx);
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "shader-fill".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    let target = super::super::paint_picker::PaintPickerTarget {
        node_id: "shader-node".into(),
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "shader-fill".into(),
    };
    for (definition_id, value) in [
        (
            "scale",
            DesignShaderPropertyValue::Text("incompatible".into()),
        ),
        ("missing", DesignShaderPropertyValue::Number(2.)),
        ("scale", DesignShaderPropertyValue::Number(2.)),
    ] {
        picker.update(visual_cx, |_, cx| {
            cx.emit(PaintPickerEvent::Edit {
                target: target.clone(),
                edit: Box::new(DesignPaintEdit {
                    property: DesignPaintProperty::ShaderProperty {
                        definition_id: definition_id.into(),
                    },
                    value: DesignPaintValue::ShaderProperty(value.clone()),
                }),
                phase: DesignPanelEditPhase::Commit,
            });
        });
    }
    visual_cx.run_until_parked();

    let actions = captured_actions.borrow();
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::PaintEditRequested {
                    edit: DesignPaintEdit {
                        property: DesignPaintProperty::ShaderProperty { .. },
                        ..
                    },
                    ..
                }
            ))
            .count(),
        1
    );
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintEditRequested {
            edit: DesignPaintEdit {
                property: DesignPaintProperty::ShaderProperty { definition_id },
                value:
                    DesignPaintValue::ShaderProperty(DesignShaderPropertyValue::Number(value)),
            },
            ..
        } if definition_id.as_ref() == "scale" && *value == 2.
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills[0].clone()),
        DesignPaint::shader("noise", "Noise").with_id("shader-fill"),
        "validated shader intents must remain host controlled"
    );
}

#[gpui::test]
fn retained_auxiliary_color_picker_preserves_rgba_phases_and_page_identity(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("page-proxy", "Page", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let original = DesignColor::rgba(0x21, 0x43, 0x65, 0x47);
    let rgb_candidate = DesignColor::rgb(0x13, 0x9d, 0xe2);
    let page = DesignPageViewData::new(
        "page-stable",
        super::super::DesignPageBackground::new(original),
        super::super::DesignLocalResourceViewData::default(),
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(page.clone(), cx);
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.page_background_picker_open = true;
            panel.open_auxiliary_color_picker(
                AuxiliaryColorPickerTarget::PageBackground {
                    page_id: "page-stable".into(),
                },
                window,
                cx,
            );
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    let picker_target = visual_cx.read(|app| {
        picker
            .read(app)
            .target()
            .cloned()
            .expect("retained auxiliary target")
    });
    assert_eq!(
        picker_target.node_id.as_ref(),
        AuxiliaryColorPickerTarget::PICKER_NODE_ID
    );
    assert_eq!(
        picker_target.paint_id.as_ref(),
        "page-background:page-stable"
    );

    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::Edit {
            target: picker_target.clone(),
            edit: Box::new(DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(rgb_candidate),
            }),
            phase: DesignPanelEditPhase::Begin,
        });
    });
    visual_cx.run_until_parked();
    let rgb_with_original_alpha = DesignColor::rgba(
        rgb_candidate.red,
        rgb_candidate.green,
        rgb_candidate.blue,
        original.alpha,
    );
    assert!(matches!(
        captured_actions.borrow().last(),
        Some(DesignPanelAction::PageBackgroundEditRequested {
            page_id,
            color,
            phase: DesignPanelEditPhase::Begin,
        }) if page_id.as_ref() == "page-stable" && *color == rgb_with_original_alpha
    ));

    let mut preview_page = page.clone();
    preview_page.background.color = rgb_with_original_alpha;
    panel.update(visual_cx, |panel, cx| {
        panel.set_page_view_data(preview_page, cx);
    });
    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::Edit {
            target: picker_target,
            edit: Box::new(DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(37.),
            }),
            phase: DesignPanelEditPhase::Preview,
        });
    });
    visual_cx.run_until_parked();
    let preview_alpha = (0.37 * f32::from(u8::MAX)).round() as u8;
    let rgba_preview = DesignColor::rgba(
        rgb_candidate.red,
        rgb_candidate.green,
        rgb_candidate.blue,
        preview_alpha,
    );
    assert!(matches!(
        captured_actions.borrow().last(),
        Some(DesignPanelAction::PageBackgroundEditRequested {
            page_id,
            color,
            phase: DesignPanelEditPhase::Preview,
        }) if page_id.as_ref() == "page-stable" && *color == rgba_preview
    ));

    let mut preview_page = page;
    preview_page.background.color = rgba_preview;
    panel.update(visual_cx, |panel, cx| {
        panel.set_page_view_data(preview_page, cx);
        assert!(panel.auxiliary_color_picker.is_some());
        assert!(panel.active_paint_edit.is_some());
        assert!(panel.auxiliary_color_picker.as_ref().is_some_and(|target| {
            target.matches_picker_target(
                &panel
                    .active_paint_edit
                    .as_ref()
                    .expect("active edit")
                    .target,
            )
        }));
        panel.prepare_paint_picker_for_dismissal(cx);
        panel.cancel_active_paint_edit(cx);
        assert!(panel.active_paint_edit.is_none());
    });
    visual_cx.run_until_parked();
    let last_action = captured_actions.borrow().last().cloned();
    assert!(
        matches!(
            &last_action,
            Some(DesignPanelAction::PageBackgroundEditRequested {
                page_id,
                color,
                phase: DesignPanelEditPhase::Cancel,
            }) if page_id.as_ref() == "page-stable" && *color == rgba_preview
        ),
        "unexpected cancel action: {last_action:?}; all actions: {:?}",
        captured_actions.borrow().as_slice(),
    );
    let phases = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PageBackgroundEditRequested { phase, .. } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        [
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Cancel,
        ],
        "controlled input synchronization must not leak edit events"
    );

    let action_count = captured_actions.borrow().len();
    panel.update(visual_cx, |panel, cx| {
        panel.auxiliary_color_picker = None;
        panel.page_background_picker_open = false;
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::viewer()),
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_auxiliary_color_picker(
                AuxiliaryColorPickerTarget::PageBackground {
                    page_id: "page-stable".into(),
                },
                window,
                cx,
            );
            assert!(panel.auxiliary_color_picker.is_some());
            assert_eq!(
                panel.paint_picker.read(cx).color_only_editability(),
                (false, false)
            );
            assert!(!panel.paint_picker.read(cx).is_disabled());
        });
    });
    assert_eq!(captured_actions.borrow().len(), action_count);
}

#[gpui::test]
fn media_capabilities_crop_cancel_and_video_preview_keep_stable_identity(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("media-node", "Media", DesignPanelNodeKind::Rectangle);
    let mut image = DesignPaint::image(super::super::DesignPaintSource::new(
        "image-source",
        "Image",
    ))
    .with_id("image-fill");
    let crop_transform = super::super::DesignPaintTransform {
        tx: 0.2,
        ..super::super::DesignPaintTransform::IDENTITY
    };
    let DesignPaintPayload::Image(image_payload) = &mut image.payload else {
        panic!("image payload");
    };
    image_payload.placement = super::super::DesignMediaPaintPlacement::Crop {
        transform: crop_transform,
    };
    let video = DesignPaint::video(super::super::DesignPaintSource::new(
        "video-source",
        "Video",
    ))
    .with_id("video-fill");
    node.fills = vec![image, video];

    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let image_target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "image-fill".into(),
    };
    let media_view_data = DesignMediaPaintViewData::new([
        super::super::DesignMediaPaintView::new(DesignPanelCollection::Fill, "image-fill", 0)
            .with_capabilities(super::super::DesignMediaPaintCapabilities::property_editor_only())
            .with_crop_tool(super::super::DesignMediaCropToolState {
                active: true,
                transform: crop_transform,
                zoom: 1.5,
                aspect_ratio: super::super::DesignMediaCropAspectRatio::Original,
            }),
        super::super::DesignMediaPaintView::new(DesignPanelCollection::Fill, "video-fill", 1)
            .with_video_preview(super::super::DesignVideoPreviewState::ready(20., 4., false)),
    ]);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_media_paint_view_data(media_view_data, cx);
            panel.active_picker = Some(image_target.clone());
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    assert!(!visual_cx.read(|app| picker.read(app).properties_editing_is_disabled()));
    assert!(visual_cx.read(|app| picker.read(app).source_replacement_is_disabled()));
    for action in DesignMediaSourceAction::ALL {
        assert!(visual_cx.read(|app| picker.read(app).media_source_action_is_disabled(action)));
    }

    panel.update(visual_cx, |panel, cx| {
        panel.emit_crop_cancel_if_active(&image_target, cx);
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintMediaCropActionRequested {
            node_id,
            paint_id,
            index: 0,
            action: DesignMediaCropAction::Cancel,
            ..
        } if node_id.as_ref() == "media-node" && paint_id.as_ref() == "image-fill"
    )));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                panel.node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintMediaCropActionRequested {
            paint_id,
            action: DesignMediaCropAction::Cancel,
            ..
        } if paint_id.as_ref() == "image-fill"
    )));
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                panel.node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
    });
    visual_cx.run_until_parked();
    captured_actions.borrow_mut().clear();
    let video_target = super::super::paint_picker::PaintPickerTarget {
        node_id: "media-node".into(),
        collection: DesignPanelCollection::Fill,
        index: 1,
        paint_id: "video-fill".into(),
    };
    panel.update(visual_cx, |panel, cx| {
        panel.active_picker = Some(PaintPickerTarget {
            collection: DesignPanelCollection::Fill,
            index: 1,
            paint_id: "video-fill".into(),
        });
        panel.paint_picker.update(cx, |_, cx| {
            cx.emit(PaintPickerEvent::VideoPreviewActionRequested {
                target: video_target,
                action: super::super::DesignVideoPreviewAction::Seek { seconds: 8. },
            });
        });
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintVideoPreviewActionRequested {
            node_id,
            paint_id,
            index: 1,
            action: super::super::DesignVideoPreviewAction::Seek { seconds },
            ..
        } if node_id.as_ref() == "media-node"
            && paint_id.as_ref() == "video-fill"
            && *seconds == 8.
    )));
}

#[gpui::test]
fn media_source_actions_preserve_source_identity_type_and_independent_permissions(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("media-node", "Media", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::image(super::super::DesignPaintSource::new(
            "image-source",
            "Image",
        ))
        .with_id("image-fill"),
        DesignPaint::video(super::super::DesignPaintSource::new(
            "video-source",
            "Video",
        ))
        .with_id("video-fill"),
    ];
    let controlled = node.fills.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let media_view_data = DesignMediaPaintViewData::new([
        super::super::DesignMediaPaintView::new(DesignPanelCollection::Fill, "image-fill", 0)
            .with_capabilities(
                DesignMediaPaintCapabilities::editor().with_source_actions(true, false, true),
            ),
        super::super::DesignMediaPaintView::new(DesignPanelCollection::Fill, "video-fill", 1),
    ]);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_media_paint_view_data(media_view_data, cx);
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "image-fill".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    assert!(!visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::Upload)
    }));
    assert!(visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::MakeImage)
    }));
    assert!(!visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::EditImage)
    }));

    let image_target = super::super::paint_picker::PaintPickerTarget {
        node_id: "media-node".into(),
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "image-fill".into(),
    };
    picker.update(visual_cx, |_, cx| {
        for (source_id, action) in [
            ("image-source", DesignMediaSourceAction::Upload),
            ("image-source", DesignMediaSourceAction::MakeImage),
            ("image-source", DesignMediaSourceAction::EditImage),
            ("stale-source", DesignMediaSourceAction::Upload),
        ] {
            cx.emit(PaintPickerEvent::MediaSourceActionRequested {
                target: image_target.clone(),
                source_id: source_id.into(),
                action,
            });
        }
    });
    visual_cx.run_until_parked();

    let observed_image_actions = captured_actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PaintMediaSourceActionRequested {
                paint_id,
                index: 0,
                source_id,
                action,
                ..
            } if paint_id.as_ref() == "image-fill" && source_id.as_ref() == "image-source" => {
                Some(*action)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        observed_image_actions,
        [
            DesignMediaSourceAction::Upload,
            DesignMediaSourceAction::EditImage,
        ]
    );

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 1,
                paint_id: "video-fill".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    assert!(!visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::Upload)
    }));
    assert!(visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::MakeImage)
    }));
    assert!(visual_cx.read(|app| {
        picker
            .read(app)
            .media_source_action_is_disabled(DesignMediaSourceAction::EditImage)
    }));
    let video_target = super::super::paint_picker::PaintPickerTarget {
        node_id: "media-node".into(),
        collection: DesignPanelCollection::Fill,
        index: 1,
        paint_id: "video-fill".into(),
    };
    picker.update(visual_cx, |_, cx| {
        for action in [
            DesignMediaSourceAction::MakeImage,
            DesignMediaSourceAction::Upload,
        ] {
            cx.emit(PaintPickerEvent::MediaSourceActionRequested {
                target: video_target.clone(),
                source_id: "video-source".into(),
                action,
            });
        }
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintMediaSourceActionRequested {
            paint_id,
            index: 1,
            source_id,
            action: DesignMediaSourceAction::Upload,
            ..
        } if paint_id.as_ref() == "video-fill" && source_id.as_ref() == "video-source"
    )));
    assert!(!captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintMediaSourceActionRequested {
            paint_id,
            action: DesignMediaSourceAction::MakeImage
                | DesignMediaSourceAction::EditImage,
            ..
        } if paint_id.as_ref() == "video-fill"
    )));

    let action_count = captured_actions.borrow().len();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                panel.node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_media_source_action(
            PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 1,
                paint_id: "video-fill".into(),
            },
            &"video-source".into(),
            DesignMediaSourceAction::Upload,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(captured_actions.borrow().len(), action_count);
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills.clone()),
        controlled,
        "media source workflows remain host controlled until echoed"
    );
}

#[gpui::test]
fn media_file_drop_revalidates_stable_identity_source_kind_format_and_permissions(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("media-node", "Media", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::solid(DesignColor::BLACK).with_id("background"),
        DesignPaint::image(super::super::DesignPaintSource::new(
            "image-source",
            "Cover",
        ))
        .with_id("stable-media"),
    ];
    let controlled = node.fills.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "stable-media".into(),
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_media_paint_view_data(
            DesignMediaPaintViewData::new([super::super::DesignMediaPaintView::new(
                DesignPanelCollection::Fill,
                "stable-media",
                1,
            )]),
            cx,
        );
        panel.emit_media_source_drop_from_paths(
            target.clone(),
            &[PathBuf::from("replacement.WEBM")],
            cx,
        );
        panel.emit_media_source_drop_from_paths(
            target.clone(),
            &[PathBuf::from("first.png"), PathBuf::from("second.png")],
            cx,
        );
        panel.emit_media_source_drop_from_paths(target.clone(), &[PathBuf::from("vector.svg")], cx);
        panel.emit_media_source_drop_from_paths(target.clone(), &[PathBuf::from("scan.tiff")], cx);
        panel.emit_media_source_drop(
            target.clone(),
            &"stale-source".into(),
            DesignMediaKind::Image,
            DesignMediaDroppedFile::new(
                PathBuf::from("replacement.png"),
                super::super::DesignMediaFileKind::Png,
            ),
            cx,
        );
        panel.emit_media_source_drop(
            target.clone(),
            &"image-source".into(),
            DesignMediaKind::Video,
            DesignMediaDroppedFile::new(
                PathBuf::from("replacement.png"),
                super::super::DesignMediaFileKind::Png,
            ),
            cx,
        );
        panel.emit_media_source_drop(
            target.clone(),
            &"image-source".into(),
            DesignMediaKind::Image,
            DesignMediaDroppedFile::new(
                PathBuf::from("replacement.png"),
                super::super::DesignMediaFileKind::Mov,
            ),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert!(
        visual_cx.debug_bounds("design-fill-media-drop-1").is_some(),
        "the collapsed media swatch is a dedicated ExternalPaths target"
    );
    assert!(
        visual_cx.debug_bounds("design-fill-media-drop-0").is_none(),
        "non-media swatches do not claim external file drops"
    );
    assert!(matches!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PaintMediaSourceDropRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paint_id,
            index: 1,
            expected_source_id,
            expected_media_kind: DesignMediaKind::Image,
            file,
        }] if node_id.as_ref() == "media-node"
            && paint_id.as_ref() == "stable-media"
            && expected_source_id.as_ref() == "image-source"
            && file.kind == super::super::DesignMediaFileKind::Webm
    ));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills.clone()),
        controlled,
        "the panel emits a file intent without mutating its controlled paint"
    );

    let action_count = captured_actions.borrow().len();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                panel.node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_media_source_drop_from_paths(target, &[PathBuf::from("replacement.png")], cx);
    });
    visual_cx.run_until_parked();
    assert_eq!(captured_actions.borrow().len(), action_count);
}

#[gpui::test]
fn paint_variable_view_data_emits_a_stable_leaf_binding_intent(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills[0] = DesignPaint::solid(DesignColor::WHITE).with_id("stable-fill");
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    let view_data =
        super::super::DesignPaintVariableViewData::new([super::super::DesignVariable::page(
            "remote-brand",
            "Brand / Primary",
            "colors",
            "Colors",
            super::super::DesignVariableResolvedType::Color,
        )
        .with_source(super::super::DesignVariableSource::library("acme", "Acme"))
        .with_import_state(super::super::DesignVariableImportState::Imported)
        .with_resolved_value(super::super::DesignVariableResolvedValue::Color(
            DesignColor::BLACK,
        ))]);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "stable-fill".into(),
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_paint_variable_view_data(view_data.clone(), cx);
            panel.active_picker = Some(target);
            panel.sync_paint_picker(window, cx);
        });
    });
    assert_eq!(
        visual_cx.read(|app| panel.read(app).paint_variable_view_data().clone()),
        view_data
    );

    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::ColorVariableApplyRequested {
            target: super::super::paint_picker::PaintPickerTarget {
                node_id: "rectangle".into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "stable-fill".into(),
            },
            color_target: super::super::DesignPaintColorTarget::Solid,
            variable_id: "remote-brand".into(),
        });
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintColorVariableApplyRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paint_id,
            index: 0,
            color_target: super::super::DesignPaintColorTarget::Solid,
            variable_id,
        } if node_id.as_ref() == "rectangle"
            && paint_id.as_ref() == "stable-fill"
            && variable_id.as_ref() == "remote-brand"
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills[0].color),
        DesignColor::WHITE,
        "variable selection remains host controlled"
    );
}

#[gpui::test]
fn picker_creation_menu_revalidates_the_active_leaf_before_whole_style_creation(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::gradient(
            super::super::DesignPaintKind::LinearGradient,
            vec![
                super::super::DesignGradientStop::new(0., DesignColor::PURPLE).with_id("start"),
                super::super::DesignGradientStop::new(1., DesignColor::WHITE).with_id("end"),
            ],
        )
        .with_id("gradient"),
        DesignPaint::solid(DesignColor::BLACK).with_id("solid"),
    ];
    let expected_paints = node.fills.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "gradient".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());

    picker.update(visual_cx, |picker, cx| {
        picker.request_paint_style_create(cx);
    });
    visual_cx.run_until_parked();
    assert!(matches!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PaintStyleCreateRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paints,
        }] if node_id.as_ref() == "rectangle" && paints == &expected_paints
    ));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, _| {
        panel.active_picker = Some(PaintPickerTarget {
            collection: DesignPanelCollection::Fill,
            index: 1,
            paint_id: "solid".into(),
        });
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_paint_style_create(cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a creation menu opened for a stale paint must not create a collection style"
    );
}

#[gpui::test]
fn a_bound_color_does_not_disable_unrelated_picker_controls(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills[0] =
        DesignPaint::from_payload(DesignPaintPayload::Solid(super::super::DesignSolidPaint {
            color: DesignColor::PURPLE,
            binding: Some(super::super::DesignPaintBinding::new(
                "brand",
                "Brand color",
            )),
        }))
        .with_id("bound-fill");
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_paint_variable_view_data(
                super::super::DesignPaintVariableViewData::new([
                    super::super::DesignVariable::page(
                        "replacement-variable",
                        "Replacement",
                        "colors",
                        "Colors",
                        super::super::DesignVariableResolvedType::Color,
                    )
                    .with_resolved_value(
                        super::super::DesignVariableResolvedValue::Color(DesignColor::BLACK),
                    ),
                ]),
                cx,
            );
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "bound-fill".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());

    assert!(
        !visual_cx.read(|app| picker.read(app).is_disabled()),
        "the selected bound color is locked inside the picker, not by disabling the whole picker"
    );
    picker.update(visual_cx, |picker, cx| {
        picker.request_color_variable_apply("replacement-variable".into(), cx);
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintColorVariableApplyRequested {
            variable_id,
            ..
        } if variable_id.as_ref() == "replacement-variable"
    )));
}

#[gpui::test]
fn picker_host_requests_preserve_stable_leaf_identity_and_exact_editability_gates(
    cx: &mut TestAppContext,
) {
    let resolved_bound_color = DesignColor::rgba(0x34, 0x78, 0xbc, 0x9a);
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills[0] = DesignPaint::gradient(
        super::super::DesignPaintKind::LinearGradient,
        vec![
            super::super::DesignGradientStop::new(0., resolved_bound_color)
                .with_id("brand-stop")
                .with_binding(super::super::DesignPaintBinding::new(
                    "brand-variable",
                    "Brand",
                )),
            super::super::DesignGradientStop::new(1., DesignColor::WHITE).with_id("end-stop"),
        ],
    )
    .with_id("stable-gradient");
    node.fills
        .push(DesignPaint::solid(DesignColor::BLACK).with_id("other-fill"));

    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "stable-gradient".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());

    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::SourceReplaceRequested {
            target: super::super::paint_picker::PaintPickerTarget {
                node_id: "rectangle".into(),
                collection: DesignPanelCollection::Fill,
                index: 1,
                paint_id: "other-fill".into(),
            },
        });
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a source request from a non-active paint must be ignored"
    );

    picker.update(visual_cx, |picker, cx| {
        picker.request_color_variable_create(cx);
        picker.request_eyedropper(cx);
    });
    visual_cx.run_until_parked();
    assert_eq!(captured_actions.borrow().len(), 1);
    assert!(matches!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PaintColorVariableCreateRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paint_id,
            index: 0,
            color_target: DesignPaintColorTarget::GradientStop {
                stop_id,
                index: 0,
            },
            color,
        }] if node_id.as_ref() == "rectangle"
            && paint_id.as_ref() == "stable-gradient"
            && stop_id.as_ref() == "brand-stop"
            && *color == resolved_bound_color
    ));
    assert!(
        visual_cx.read(|app| { panel.read(app).node.fills[0].selected_color_is_bound(0) }),
        "creating a style must not detach the existing binding"
    );

    captured_actions.borrow_mut().clear();
    let gradient = node
        .fills
        .iter_mut()
        .find(|paint| paint.id.as_ref() == "stable-gradient")
        .expect("stable gradient");
    let DesignPaintPayload::Gradient(gradient_payload) = &mut gradient.payload else {
        panic!("gradient payload");
    };
    gradient_payload.stops[0].binding = None;
    gradient.sync_legacy_projection();
    node.fills.swap(0, 1);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_node(node.clone(), cx);
            panel.sync_paint_picker(window, cx);
        });
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_eyedropper(cx);
    });
    visual_cx.run_until_parked();
    assert!(matches!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::PaintEyedropperRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            paint_id,
            index: 1,
            color_target: DesignPaintColorTarget::GradientStop {
                stop_id,
                index: 0,
            },
        }] if node_id.as_ref() == "rectangle"
            && paint_id.as_ref() == "stable-gradient"
            && stop_id.as_ref() == "brand-stop"
    ));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.sync_paint_picker(window, cx);
        });
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_color_variable_create(cx);
        picker.request_eyedropper(cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "viewer permissions must disable both host requests"
    );

    let mut read_only_node = node;
    let read_only_gradient = read_only_node
        .fills
        .iter_mut()
        .find(|paint| paint.id.as_ref() == "stable-gradient")
        .expect("stable gradient");
    read_only_gradient.read_only = true;
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                read_only_node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.sync_paint_picker(window, cx);
        });
    });
    picker.update(visual_cx, |picker, cx| {
        picker.request_color_variable_create(cx);
        picker.request_eyedropper(cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured_actions.borrow().is_empty(),
        "a host read-only paint must disable both host requests"
    );
}

#[gpui::test]
fn stable_paint_id_keeps_the_picker_open_after_host_reorder(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills
        .push(DesignPaint::solid(DesignColor::WHITE).with_id("secondary-fill"));
    let active_id = node.fills[0].id.clone();
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: active_id.clone(),
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(target.clone());
            panel.sync_paint_picker(window, cx);
        });
    });

    node.fills.swap(0, 1);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_node(node, cx);
            panel.sync_paint_picker(window, cx);
            assert!(panel.active_picker.is_some());
        });
    });

    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    let picker_target = visual_cx.read(|app| picker.read(app).target().cloned().expect("target"));
    assert_eq!(picker_target.paint_id, active_id);
    assert_eq!(picker_target.index, 1);
}

#[gpui::test]
fn paint_reorder_emits_a_stable_host_intent_without_mutating_view_data(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    node.fills
        .push(DesignPaint::solid(DesignColor::BLUE).with_id("fill-second"));
    let first = node.fills[0].clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_paint_reorder(DesignPanelCollection::Fill, &first, 0, 1, cx);
        panel.node.fills[0].read_only = true;
        panel.emit_paint_reorder(DesignPanelCollection::Fill, &first, 0, 1, cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(captured_actions.borrow().len(), 1);
    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PaintReorderRequested {
            paint_id,
            from_index: 0,
            to_index: 1,
            ..
        } if *paint_id == first.id
    )));
    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.fills[0].id.clone()),
        first.id
    );
}

#[gpui::test]
fn effect_variants_resolve_only_their_typed_leaf_controls(cx: &mut TestAppContext) {
    let drop_shadow = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::DropShadow(super::super::DesignDropShadowEffect {
            color: DesignColor::rgba(12, 34, 56, 120),
            offset: super::super::DesignEffectVector::new(-3., 7.),
            radius: 11.,
            spread: -2.,
            blend_mode: DesignBlendMode::Multiply,
            show_behind_node: true,
        }),
    );
    let inner_shadow = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::InnerShadow(super::super::DesignInnerShadowEffect {
            color: DesignColor::PURPLE,
            offset: super::super::DesignEffectVector::new(2., 5.),
            radius: 8.,
            spread: 1.,
            blend_mode: DesignBlendMode::Screen,
        }),
    );
    let normal_blur = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::LayerBlur(DesignBlurEffect::normal(14.)),
    );
    let progressive_blur = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::BackgroundBlur(DesignBlurEffect::progressive(
            2.,
            18.,
            super::super::DesignEffectVector::new(0.2, 0.3),
            super::super::DesignEffectVector::new(0.8, 0.9),
        )),
    );
    let duotone_noise = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::Noise(super::super::DesignNoiseEffect {
            colors: DesignNoiseColors::Duotone {
                color: DesignColor::BLACK,
                secondary_color: DesignColor::WHITE,
            },
            size: super::super::DesignEffectVector::new(3., 4.),
            density: 0.45,
            blend_mode: DesignBlendMode::Overlay,
        }),
    );
    let multitone_noise = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::Noise(super::super::DesignNoiseEffect {
            colors: DesignNoiseColors::Multitone { opacity: 0.65 },
            ..super::super::DesignNoiseEffect::default()
        }),
    );
    let texture = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::Texture(super::super::DesignTextureEffect {
            size: super::super::DesignEffectVector::new(6., 9.),
            radius: 3.,
            clip_to_shape: true,
        }),
    );
    let glass = super::super::DesignEffect::from_settings(
        true,
        DesignEffectSettings::Glass(super::super::DesignGlassEffect {
            light_intensity: 0.7,
            light_angle: 35.,
            refraction: 0.4,
            depth: 16.,
            dispersion: 0.2,
            frost: 5.,
            splay: 0.8,
        }),
    );
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Rectangle);
    node.effects = vec![
        drop_shadow,
        inner_shadow,
        normal_blur,
        progressive_blur,
        duotone_noise,
        multitone_noise,
        texture,
        glass,
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectShadowColor(0)),
            Some(DesignPanelValue::Color(DesignColor::rgba(12, 34, 56, 120)))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectDropShadowShowBehindNode(0)),
            Some(DesignPanelValue::Bool(true))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectShadowBlendMode(1)),
            Some(DesignPanelValue::BlendMode(DesignBlendMode::Screen))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectBlurRadius(2)),
            Some(DesignPanelValue::Number(14.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectProgressiveBlurStartOffsetX(3)),
            Some(DesignPanelValue::Number(0.2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectProgressiveBlurEndRadius(3)),
            Some(DesignPanelValue::Number(18.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectNoiseSecondaryColor(4)),
            Some(DesignPanelValue::Color(DesignColor::WHITE))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectNoiseOpacity(4)),
            None,
            "duotone noise must not expose multitone opacity"
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectNoiseOpacity(5)),
            Some(DesignPanelValue::Number(0.65))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectTextureSizeY(6)),
            Some(DesignPanelValue::Number(9.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectTextureClipToShape(6)),
            Some(DesignPanelValue::Bool(true))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectGlassDepth(7)),
            Some(DesignPanelValue::Number(16.))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::EffectGlassSplay(7)),
            Some(DesignPanelValue::Number(0.8))
        );
        assert!(
            panel
                .option_properties()
                .contains(&DesignPanelProperty::EffectBlurType(2))
        );
        assert!(
            panel
                .option_properties()
                .contains(&DesignPanelProperty::EffectNoiseType(4))
        );
        assert!(
            panel
                .property_options(DesignPanelProperty::EffectShadowBlendMode(0))
                .expect("effect blend modes")
                .iter()
                .all(|option| {
                    option.value != DesignPanelValue::BlendMode(DesignBlendMode::PassThrough)
                }),
            "pass-through is a layer blend mode, not an effect blend mode"
        );
    });
}

#[gpui::test]
fn effect_edits_remove_and_reorder_use_stable_ids(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Rectangle);
    node.effects = vec![
        super::super::DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow-a"),
        super::super::DesignEffect::new(DesignEffectKind::InnerShadow).with_id("shadow-b"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_property(
            DesignPanelProperty::EffectShadowBlur(0),
            DesignPanelValue::Number(12.),
            cx,
        );
        panel.emit_effect_reorder("shadow-b".into(), 1, 0, cx);
        panel.emit_remove(DesignPanelCollection::Effect, 0, cx);
    });
    visual_cx.run_until_parked();

    let actions = captured_actions.borrow();
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::EffectEditRequested {
            effect_id,
            index: 0,
            property: DesignPanelProperty::EffectShadowBlur(0),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if effect_id.as_ref() == "shadow-a"
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::EffectReorderRequested {
            effect_id,
            from_index: 1,
            to_index: 0,
            ..
        } if effect_id.as_ref() == "shadow-b"
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::EffectRemoveRequested {
            effect_id,
            index: 0,
            ..
        } if effect_id.as_ref() == "shadow-a"
    )));
}

#[gpui::test]
fn shader_complex_leaf_edits_are_phased_and_rebase_by_effect_and_definition_id(
    cx: &mut TestAppContext,
) {
    let line = DesignShaderProperty::new(
        "line-definition",
        "Ray",
        DesignShaderPropertyKind::Line,
        DesignShaderPropertyValue::Line {
            start: super::super::DesignEffectVector::new(0.1, 0.2),
            end: super::super::DesignEffectVector::new(0.9, 0.8),
        },
    );
    let text = DesignShaderProperty::new(
        "text-definition",
        "Label",
        DesignShaderPropertyKind::Text,
        DesignShaderPropertyValue::Text("Glass".into()),
    );
    let shader = DesignEffect::from_settings(
        true,
        DesignEffectSettings::Shader(DesignShaderEffect::new(
            "shader-definition",
            "Shader",
            [text, line],
        )),
    )
    .with_id("shader-effect");
    let mut node = DesignPanelNode::new("shader-node", "Shader", DesignPanelNodeKind::Rectangle);
    node.effects = vec![
        shader,
        DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow-effect"),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_shader_property_field(
                DesignPanelProperty::EffectShaderProperty(0, 1),
                ShaderPropertyEditorField::LineEndX,
                window,
                cx,
            );
            let mut next = node;
            next.effects.swap(0, 1);
            let DesignEffectSettings::Shader(shader) = &mut next.effects[1].settings else {
                panic!("stable Shader effect");
            };
            shader.properties.swap(0, 1);
            panel.set_node(next, cx);
            assert!(matches!(
                panel.property_editor.as_ref().map(|editor| editor.property),
                Some(DesignPanelProperty::EffectShaderProperty(1, 0))
            ));
        });
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+0.25", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::EffectEditRequested {
                effect_id,
                index,
                property,
                shader_property_id: Some(shader_property_id),
                value: DesignPanelValue::ShaderProperty(value),
                phase,
                ..
            } => Some((
                effect_id.clone(),
                *index,
                *property,
                shader_property_id.clone(),
                value.clone(),
                *phase,
            )),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(edits.len(), 3);
    assert_eq!(edits[0].0.as_ref(), "shader-effect");
    assert_eq!(edits[0].1, 0);
    assert_eq!(edits[0].2, DesignPanelProperty::EffectShaderProperty(0, 1));
    assert_eq!(edits[0].3.as_ref(), "line-definition");
    assert_eq!(edits[0].5, DesignPanelEditPhase::Begin);
    for edit in &edits[1..] {
        assert_eq!(edit.0.as_ref(), "shader-effect");
        assert_eq!(edit.1, 1);
        assert_eq!(edit.2, DesignPanelProperty::EffectShaderProperty(1, 0));
        assert_eq!(edit.3.as_ref(), "line-definition");
    }
    assert!(matches!(
        &edits[1].4,
        DesignShaderPropertyValue::Line { end, .. }
            if (end.x - 1.15).abs() < f32::EPSILON
    ));
    assert_eq!(edits[1].5, DesignPanelEditPhase::Preview);
    assert_eq!(edits[2].5, DesignPanelEditPhase::Commit);
}

#[gpui::test]
fn shader_field_focus_rebases_and_reopens_the_exact_leaf(cx: &mut TestAppContext) {
    let line = DesignShaderProperty::new(
        "line-definition",
        "Ray",
        DesignShaderPropertyKind::Line,
        DesignShaderPropertyValue::Line {
            start: super::super::DesignEffectVector::new(0.1, 0.2),
            end: super::super::DesignEffectVector::new(0.9, 0.8),
        },
    );
    let text = DesignShaderProperty::new(
        "text-definition",
        "Label",
        DesignShaderPropertyKind::Text,
        DesignShaderPropertyValue::Text("Glass".into()),
    );
    let shader = DesignEffect::from_settings(
        true,
        DesignEffectSettings::Shader(DesignShaderEffect::new(
            "shader-definition",
            "Shader",
            [text, line],
        )),
    )
    .with_id("shader-effect");
    let mut node = DesignPanelNode::new("shader-node", "Shader", DesignPanelNodeKind::Rectangle);
    node.effects = vec![
        shader,
        DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow-effect"),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.focus_handle.focus(window, cx);
            panel.activate_shader_property_field_from_control(
                DesignPanelProperty::EffectShaderProperty(0, 1),
                ShaderPropertyEditorField::LineEndX,
                window,
                cx,
            );
        });
    });
    visual_cx.run_until_parked();
    let return_handle = visual_cx.read(|app| {
        let panel = panel.read(app);
        let return_focus = panel
            .editor_focus_return
            .as_ref()
            .expect("Shader field activation should retain its exact cell");
        assert_eq!(
            return_focus.origin,
            EditorFocusOrigin::ShaderField {
                property: DesignPanelProperty::EffectShaderProperty(0, 1),
                field: ShaderPropertyEditorField::LineEndX,
            }
        );
        return_focus.handle.clone()
    });

    panel.update(visual_cx, |panel, cx| {
        node.effects.swap(0, 1);
        let DesignEffectSettings::Shader(shader) = &mut node.effects[1].settings else {
            panic!("stable Shader effect");
        };
        shader.properties.swap(0, 1);
        panel.set_node(node.clone(), cx);
        assert_eq!(
            panel
                .editor_focus_return
                .as_ref()
                .map(|return_focus| return_focus.origin.clone()),
            Some(EditorFocusOrigin::ShaderField {
                property: DesignPanelProperty::EffectShaderProperty(1, 0),
                field: ShaderPropertyEditorField::LineEndX,
            })
        );
    });
    visual_cx.run_until_parked();

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_shader_property_field_from_control(
                DesignPanelProperty::EffectShaderProperty(1, 0),
                ShaderPropertyEditorField::LineEndX,
                window,
                cx,
            );
            assert!(panel.property_editor.as_ref().is_some_and(|editor| {
                editor.property == DesignPanelProperty::EffectShaderProperty(1, 0)
                    && matches!(
                        editor.kind,
                        PropertyEditorKind::Shader {
                            field: ShaderPropertyEditorField::LineEndX,
                            ..
                        }
                    )
            }));
        });
    });
}

#[gpui::test]
fn shader_semantic_kind_echo_cancels_the_original_typed_transaction(cx: &mut TestAppContext) {
    let property = DesignShaderProperty::new(
        "shape-definition",
        "Shape",
        DesignShaderPropertyKind::Circle,
        DesignShaderPropertyValue::Circle {
            center: super::super::DesignEffectVector::new(0.5, 0.5),
            radius: 0.4,
        },
    );
    let effect = DesignEffect::from_settings(
        true,
        DesignEffectSettings::Shader(DesignShaderEffect::new(
            "shader-definition",
            "Shader",
            [property],
        )),
    )
    .with_id("shader-effect");
    let mut node = DesignPanelNode::new("shader-node", "Shader", DesignPanelNodeKind::Rectangle);
    node.effects = vec![effect];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_shader_property_field(
                DesignPanelProperty::EffectShaderProperty(0, 0),
                ShaderPropertyEditorField::CircleRadius,
                window,
                cx,
            );
            let mut next = node;
            let DesignEffectSettings::Shader(shader) = &mut next.effects[0].settings else {
                panic!("Shader effect");
            };
            shader.properties[0].kind = DesignShaderPropertyKind::Point;
            shader.properties[0].value =
                DesignShaderPropertyValue::Point(super::super::DesignEffectVector::new(0.5, 0.5));
            panel.set_node(next, cx);
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::EffectEditRequested {
                effect_id,
                shader_property_id: Some(shader_property_id),
                value:
                    DesignPanelValue::ShaderProperty(DesignShaderPropertyValue::Circle {
                        radius, ..
                    }),
                phase,
                ..
            } if effect_id.as_ref() == "shader-effect"
                && shader_property_id.as_ref() == "shape-definition"
                && (*radius - 0.4).abs() < f32::EPSILON =>
            {
                Some(*phase)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn shader_resource_and_variable_requests_are_stable_and_opaque_values_are_inert(
    cx: &mut TestAppContext,
) {
    let asset = DesignShaderProperty::new(
        "image-definition",
        "Image",
        DesignShaderPropertyKind::Image,
        DesignShaderPropertyValue::AssetId("image:current".into()),
    );
    let alias = DesignShaderProperty::new(
        "alias-definition",
        "Alias",
        DesignShaderPropertyKind::Number,
        DesignShaderPropertyValue::VariableAlias {
            variable_id: "variable:amount".into(),
        },
    );
    let color_point = DesignShaderProperty::new(
        "color-point-definition",
        "Color point",
        DesignShaderPropertyKind::ColorPoint,
        DesignShaderPropertyValue::ColorPoint {
            point: super::super::DesignEffectVector::new(0.5, 0.5),
            color: DesignColor::BLUE,
            variable_id: Some("variable:color".into()),
        },
    );
    let opaque = DesignShaderProperty::new(
        "opaque-definition",
        "Future",
        DesignShaderPropertyKind::Unsupported,
        DesignShaderPropertyValue::Opaque {
            type_name: "FUTURE".into(),
            payload: "{\"future\":true}".into(),
        },
    );
    let effect = DesignEffect::from_settings(
        true,
        DesignEffectSettings::Shader(DesignShaderEffect::new(
            "shader-definition",
            "Shader",
            [asset, alias, color_point, opaque],
        )),
    )
    .with_id("shader-effect");
    let mut node = DesignPanelNode::new("shader-node", "Shader", DesignPanelNodeKind::Rectangle);
    node.effects = vec![effect];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.request_effect_shader_property_editor(
            0,
            0,
            DesignShaderPropertyEditorTarget::Value,
            DesignShaderPropertyEditorKind::Resource,
            cx,
        );
        panel.request_effect_shader_property_editor(
            0,
            1,
            DesignShaderPropertyEditorTarget::Value,
            DesignShaderPropertyEditorKind::Variable,
            cx,
        );
        panel.request_effect_shader_property_variable_detach(
            0,
            1,
            DesignShaderPropertyEditorTarget::Value,
            "variable:amount".into(),
            cx,
        );
        panel.request_effect_shader_property_variable_detach(
            0,
            2,
            DesignShaderPropertyEditorTarget::ColorPointColor,
            "variable:color".into(),
            cx,
        );
        panel.request_effect_shader_property_editor(
            0,
            3,
            DesignShaderPropertyEditorTarget::Value,
            DesignShaderPropertyEditorKind::Variable,
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::EffectShaderProperty(0, 3),
            DesignPanelValue::ShaderProperty(DesignShaderPropertyValue::Opaque {
                type_name: "FUTURE".into(),
                payload: "{\"future\":true}".into(),
            }),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 4, "opaque rows must emit no unchanged edit");
    assert!(matches!(
        &captured[0],
        DesignPanelAction::EffectShaderPropertyEditorRequested {
            effect_id,
            shader_property_id,
            property_index: 0,
            property_kind: DesignShaderPropertyKind::Image,
            target: DesignShaderPropertyEditorTarget::Value,
            editor: DesignShaderPropertyEditorKind::Resource,
            current_value: DesignShaderPropertyValue::AssetId(asset_id),
            ..
        } if effect_id.as_ref() == "shader-effect"
            && shader_property_id.as_ref() == "image-definition"
            && asset_id.as_ref() == "image:current"
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::EffectShaderPropertyEditorRequested {
            shader_property_id,
            editor: DesignShaderPropertyEditorKind::Variable,
            current_value: DesignShaderPropertyValue::VariableAlias { variable_id },
            ..
        } if shader_property_id.as_ref() == "alias-definition"
            && variable_id.as_ref() == "variable:amount"
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::EffectShaderPropertyVariableDetachRequested {
            shader_property_id,
            target: DesignShaderPropertyEditorTarget::Value,
            variable_id,
            ..
        } if shader_property_id.as_ref() == "alias-definition"
            && variable_id.as_ref() == "variable:amount"
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::EffectShaderPropertyVariableDetachRequested {
            shader_property_id,
            target: DesignShaderPropertyEditorTarget::ColorPointColor,
            variable_id,
            ..
        } if shader_property_id.as_ref() == "color-point-definition"
            && variable_id.as_ref() == "variable:color"
    ));
}

#[gpui::test]
fn stable_effect_settings_target_survives_host_reorder_and_capabilities_gate_leaves(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Group);
    node.effects = vec![
        super::super::DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow-a"),
        super::super::DesignEffect::new(DesignEffectKind::InnerShadow).with_id("shadow-b"),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.active_effect_settings = Some(EffectSettingsTarget {
            index: 1,
            effect_id: "shadow-b".into(),
        });
        assert!(!panel.property_is_editable(DesignPanelProperty::EffectShadowSpread(0)));
        assert!(
            !panel.property_is_editable(DesignPanelProperty::EffectDropShadowShowBehindNode(0))
        );
        node.effects.swap(0, 1);
        panel.set_node(node.clone(), cx);
        assert_eq!(
            panel
                .active_effect_settings
                .as_ref()
                .map(|target| target.index),
            Some(0)
        );
    });
}

#[gpui::test]
fn effect_style_and_variable_intents_remain_controlled(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Rectangle);
    node.effects =
        vec![super::super::DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow")];
    let original = node.effects.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_effect_style_view_data(
            DesignEffectStyleViewData::new(
                [super::super::DesignEffectStyle::new(
                    "raised",
                    "Raised",
                    [DesignEffectKind::DropShadow],
                )],
                [],
            ),
            cx,
        );
        panel.set_effect_variable_view_data(
            DesignEffectVariableViewData::new([super::super::DesignEffectVariable::new(
                "shadow-color",
                "Shadow",
                "Semantic",
                super::super::DesignEffectVariableKind::Color,
            )]),
            cx,
        );
        panel.emit_effect_style_apply(DesignEffectStyleSelection::page("raised"), cx);
        panel.emit_effect_variable_action(0, DesignEffectVariableField::Color, cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.effects.clone()),
        original
    );
    let actions = captured_actions.borrow();
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::EffectStyleApplyRequested { style, .. }
            if style.style_id.as_ref() == "raised"
    )));
    assert!(actions.iter().any(|action| matches!(
        action,
        DesignPanelAction::EffectVariableApplyRequested {
            effect_id,
            field: DesignEffectVariableField::Color,
            variable_id,
            ..
        } if effect_id.as_ref() == "shadow" && variable_id.as_ref() == "shadow-color"
    )));
}

#[gpui::test]
fn bound_effect_style_cannot_be_recreated_before_detach(cx: &mut TestAppContext) {
    let selection = DesignEffectStyleSelection::page("raised");
    let mut node = DesignPanelNode::new("effects", "Effects", DesignPanelNodeKind::Rectangle);
    node.effects =
        vec![super::super::DesignEffect::new(DesignEffectKind::DropShadow).with_id("shadow")];
    node.effect_style_binding = Some(super::super::DesignEffectStyleBinding::new(
        selection.clone(),
        "Raised",
    ));
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_effect_style_create(cx);
        panel.emit_effect_style_detach(cx);
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured_actions.borrow().as_slice(),
        [DesignPanelAction::EffectStyleDetachRequested { style, .. }]
            if style == &selection
    ));
}

#[gpui::test]
fn stroke_panel_indexes_only_paints_and_gates_shared_geometry(cx: &mut TestAppContext) {
    let mut rectangle =
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let mut stroke = DesignStroke::for_node(
        DesignPanelNodeKind::Rectangle,
        DesignPaint::solid(DesignColor::BLACK),
        3.,
        DesignStrokeAlign::Inside,
    );
    stroke.add_paint(DesignPaint::solid(DesignColor::PURPLE));
    stroke.weights = super::super::DesignStrokeWeights::custom(1., 2., 3., 4.);
    rectangle.stroke = Some(stroke);
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::StrokeWeightRight),
            Some(DesignPanelValue::Number(2.))
        );
        assert_eq!(
            panel
                .property_options(DesignPanelProperty::StrokeAlign)
                .expect("rectangle alignment options")
                .len(),
            3
        );
        assert_eq!(panel.node.stroke.as_ref().expect("stroke").paints.len(), 2);
    });

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("line", "Line", DesignPanelNodeKind::Line),
            cx,
        );
    });
    visual_cx.run_until_parked();
    visual_cx.read(|app| {
        let panel = panel.read(app);
        let alignment = panel
            .property_options(DesignPanelProperty::StrokeAlign)
            .expect("a canonical line exposes its fixed Center position");
        assert_eq!(
            alignment
                .iter()
                .map(|option| option.value.clone())
                .collect::<Vec<_>>(),
            vec![DesignPanelValue::StrokeAlign(DesignStrokeAlign::Center)]
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::StrokeAlign),
            Some(DesignPanelValue::StrokeAlign(DesignStrokeAlign::Center))
        );
        assert_eq!(
            panel
                .property_options(DesignPanelProperty::StrokeStartCap)
                .expect("line endpoint options")
                .iter()
                .map(|option| option.label.as_ref())
                .collect::<Vec<_>>(),
            DesignStrokeCap::ALL
                .iter()
                .map(|cap| cap.label())
                .collect::<Vec<_>>()
        );
        assert!(
            !panel
                .option_properties()
                .contains(&DesignPanelProperty::StrokeWeightMode),
            "line nodes do not expose individual-side editors"
        );
    });
}

#[gpui::test]
fn lossless_draw_strokes_expose_exact_forms_domains_and_topology_gates(cx: &mut TestAppContext) {
    let mut vector = DesignPanelNode::new("vector", "Vector", DesignPanelNodeKind::Vector);
    let stroke = vector.stroke.as_mut().expect("vector stroke");
    stroke.variable_width = Some(DesignVariableWidthStroke::custom([
        DesignVariableWidthPoint::new(0.75, 1.5),
        DesignVariableWidthPoint::new(0.25, 0.5),
    ]));
    let (host, visual_cx) = setup(vector, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::StrokeVariableWidthPointPosition(0)),
            Some(DesignPanelValue::Number(0.75))
        );
        assert_eq!(
            panel
                .property_options(DesignPanelProperty::StrokeVariableWidth)
                .expect("variable-width options")
                .len(),
            8
        );
        assert_eq!(
            panel
                .property_options(DesignPanelProperty::StrokeType)
                .expect("editable complex-stroke forms")
                .iter()
                .map(|option| option.value.clone())
                .collect::<Vec<_>>(),
            DesignStrokeType::EDITABLE
                .into_iter()
                .map(DesignPanelValue::StrokeType)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            panel
                .property_options(DesignPanelProperty::StrokeDashMode)
                .expect("dash-mode options")
                .iter()
                .map(|option| option.value.clone())
                .collect::<Vec<_>>(),
            DesignStrokeDashMode::ALL
                .into_iter()
                .map(DesignPanelValue::StrokeDashMode)
                .collect::<Vec<_>>()
        );
        assert!(panel.property_is_editable(DesignPanelProperty::StrokeDashMode));
        assert!(!panel.property_is_editable(DesignPanelProperty::StrokeDashPattern));
        panel.emit_property(
            DesignPanelProperty::StrokeVariableWidthPointPosition(0),
            DesignPanelValue::Number(0.8),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::StrokeVariableWidthPointPosition(0),
            DesignPanelValue::Number(1.01),
            cx,
        );

        let mut branching = panel.node.clone();
        branching
            .stroke
            .as_mut()
            .expect("vector stroke")
            .edit_context = super::super::DesignStrokeEditContext::branching(3);
        panel.set_node(branching, cx);
        assert!(!panel.property_is_editable(DesignPanelProperty::StrokeVariableWidth));

        let mut scatter = DesignPanelNode::new("scatter", "Scatter", DesignPanelNodeKind::Vector);
        scatter
            .stroke
            .as_mut()
            .expect("scatter stroke")
            .complex_stroke =
            DesignComplexStroke::ScatterBrush(super::super::DesignScatterBrushStroke::default());
        panel.set_node(scatter, cx);
        assert!(panel.property_is_editable(DesignPanelProperty::StrokeScatterGap));
        assert!(!panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeScatterGap,
            &DesignPanelValue::Number(0.24),
        ));
        assert!(panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeScatterGap,
            &DesignPanelValue::Number(0.25),
        ));
        panel.emit_property(
            DesignPanelProperty::StrokeScatterGap,
            DesignPanelValue::Number(0.25),
            cx,
        );

        let mut dynamic = DesignPanelNode::new("dynamic", "Dynamic", DesignPanelNodeKind::Vector);
        let dynamic_stroke = dynamic.stroke.as_mut().expect("dynamic stroke");
        dynamic_stroke.complex_stroke =
            DesignComplexStroke::Dynamic(super::super::DesignDynamicStroke::default());
        panel.set_node(dynamic, cx);
        assert!(!panel.property_is_editable(DesignPanelProperty::StrokeVariableWidth));
        assert!(panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeDynamicFrequency,
            &DesignPanelValue::Number(0.01),
        ));
        assert!(!panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeDynamicFrequency,
            &DesignPanelValue::Number(20.01),
        ));
        assert!(panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeDynamicSmoothen,
            &DesignPanelValue::Number(1.),
        ));
        assert!(!panel.layout_property_value_is_applicable(
            DesignPanelProperty::StrokeDynamicSmoothen,
            &DesignPanelValue::Number(1.01),
        ));

        let mut opaque = DesignPanelNode::new("opaque", "Opaque", DesignPanelNodeKind::Vector);
        opaque
            .stroke
            .as_mut()
            .expect("opaque stroke")
            .complex_stroke =
            DesignComplexStroke::Opaque(super::super::DesignOpaqueComplexStroke::new(
                "CUSTOM",
                "Host brush",
                "{\"brushName\":\"CUSTOM\"}",
            ));
        panel.set_node(opaque, cx);
        assert!(!panel.property_is_editable(DesignPanelProperty::StrokeType));
        assert!(!panel.property_is_editable(DesignPanelProperty::StrokeVariableWidth));
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::StrokeType),
            Some(DesignPanelValue::StrokeType(DesignStrokeType::Opaque))
        );
    });

    let captured_actions = captured_actions.borrow();
    assert_eq!(captured_actions.len(), 2);
    assert!(matches!(
        &captured_actions[0],
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::StrokeVariableWidthPointPosition(0),
            value: DesignPanelValue::Number(value),
            ..
        } if *value == 0.8
    ));
    assert!(matches!(
        &captured_actions[1],
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::StrokeScatterGap,
            value: DesignPanelValue::Number(value),
            ..
        } if *value == 0.25
    ));
}

#[gpui::test]
fn appearance_defaults_to_the_compact_summary_and_resets_disclosures(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle),
        cx,
    );
    let panel = panel(&host, visual_cx);

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(!panel.appearance_blend_mode_open);
        assert!(!panel.appearance_corner_details_are_visible());
    });

    panel.update(visual_cx, |panel, cx| {
        panel.appearance_blend_mode_open = true;
        panel.appearance_corner_details_open = true;
        panel.set_node(
            DesignPanelNode::new("ellipse", "Ellipse", DesignPanelNodeKind::Ellipse),
            cx,
        );
        assert!(!panel.appearance_blend_mode_open);
        assert!(!panel.appearance_corner_details_are_visible());
        drop(panel.render_layer(cx));
    });
}

#[gpui::test]
fn appearance_visibility_is_host_controlled_and_emits_a_typed_property_intent(
    cx: &mut TestAppContext,
) {
    let mut rectangle =
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    rectangle.visible = false;
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::Visible),
            Some(DesignPanelValue::Bool(false))
        );
        drop(panel.render_appearance_header(cx));
        panel.emit_property(
            DesignPanelProperty::Visible,
            DesignPanelValue::Bool(true),
            cx,
        );
        assert!(
            !panel.node.visible,
            "the reusable panel waits for the host echo"
        );
    });

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            node_id,
            property: DesignPanelProperty::Visible,
            value: DesignPanelValue::Bool(true),
        } if node_id.as_ref() == "rectangle"
    )));
}

#[gpui::test]
fn mixed_visibility_resolves_to_show_all_and_uniform_states_toggle_on_design_surface(
    cx: &mut TestAppContext,
) {
    let mut first = DesignPanelNode::new("one", "One", DesignPanelNodeKind::Rectangle);
    first.visible = true;
    let mut second = DesignPanelNode::new("two", "Two", DesignPanelNodeKind::Ellipse);
    second.visible = false;
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["one".into(), "two".into()],
    };
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        panel.expanded_sections.clear();
        assert_eq!(
            panel.visibility_control_state(DesignPanelProperty::Visible, panel.node.visible),
            VisibilityControlState::Mixed,
        );
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-appearance-visible")
            .is_some(),
        "mixed visibility must keep the shared visibility control mounted",
    );

    let visibility = visual_cx
        .debug_bounds("design-appearance-visible")
        .expect("Design visibility control")
        .center();
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    {
        let captured = captured.borrow();
        assert_eq!(captured.len(), 1);
        let (action_target, action) = captured[0]
            .targeted_node_action()
            .expect("multiple-selection visibility must carry the exact target");
        assert_eq!(action_target, &target);
        assert!(matches!(
            action,
            DesignPanelAction::PropertyChangeRequested {
                node_id,
                property: DesignPanelProperty::Visible,
                value: DesignPanelValue::Bool(true),
            } if node_id.as_ref() == "one"
        ));
    }
    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(
            panel.node.visible,
            "activation must not mutate the host-controlled first-node snapshot",
        );
        assert!(
            panel
                .property_value_states
                .get(&DesignPanelProperty::Visible)
                .is_some_and(DesignPanelPropertyValueState::is_mixed),
            "activation must wait for a uniform host echo",
        );
    });

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Bool(true)),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel
            .read(app)
            .visibility_control_state(DesignPanelProperty::Visible, panel.read(app).node.visible,)),
        VisibilityControlState::Visible,
    );
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::TargetedNodeActionRequested { target: action_target, action }]
            if action_target == &target
                && matches!(
                    action.as_ref(),
                    DesignPanelAction::PropertyChangeRequested {
                        node_id,
                        property: DesignPanelProperty::Visible,
                        value: DesignPanelValue::Bool(false),
                    } if node_id.as_ref() == "one"
                )
    ));

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Bool(false)),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel
            .read(app)
            .visibility_control_state(DesignPanelProperty::Visible, panel.read(app).node.visible,)),
        VisibilityControlState::Hidden,
    );
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::TargetedNodeActionRequested { target: action_target, action }]
            if action_target == &target
                && matches!(
                    action.as_ref(),
                    DesignPanelAction::PropertyChangeRequested {
                        node_id,
                        property: DesignPanelProperty::Visible,
                        value: DesignPanelValue::Bool(true),
                    } if node_id.as_ref() == "one"
                )
    ));

    captured.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_state(
            DesignPanelProperty::Visible,
            DesignPanelPropertyValueState::Mixed.read_only(),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel
            .read(app)
            .visibility_control_state(DesignPanelProperty::Visible, panel.read(app).node.visible,)),
        VisibilityControlState::Mixed,
    );
    visual_cx.simulate_click(visibility, Modifiers::none());
    visual_cx.run_until_parked();
    assert!(
        captured.borrow().is_empty(),
        "read-only mixed visibility must stay inert",
    );
}

#[gpui::test]
fn appearance_blend_options_reserve_pass_through_for_containers(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle),
        cx,
    );
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let leaf_options = panel
            .property_options(DesignPanelProperty::BlendMode)
            .expect("rectangle blend options");
        assert_eq!(leaf_options.len(), DesignBlendMode::NON_PASS_THROUGH.len());
        assert!(!leaf_options.iter().any(|option| {
            option.value == DesignPanelValue::BlendMode(DesignBlendMode::PassThrough)
        }));

        panel.set_node(
            DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame),
            cx,
        );
        let container_options = panel
            .property_options(DesignPanelProperty::BlendMode)
            .expect("frame blend options");
        assert_eq!(container_options.len(), DesignBlendMode::ALL.len());
        assert!(container_options.iter().any(|option| {
            option.value == DesignPanelValue::BlendMode(DesignBlendMode::PassThrough)
        }));
    });
}

#[gpui::test]
fn fill_header_style_action_opens_the_whole_collection_style_browser(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle),
        cx,
    );
    let panel = panel(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_paint_style_browser(DesignPanelCollection::Fill, window, cx);
            assert_eq!(
                panel.paint_style_browser_open,
                Some(DesignPanelCollection::Fill)
            );
            assert!(panel.active_picker.is_none());
        });
    });
}

#[gpui::test]
fn whole_paint_style_intents_never_target_a_leaf_color(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::solid(DesignColor::BLACK).with_id("fill-a"),
        DesignPaint::solid(DesignColor::WHITE).with_id("fill-b"),
    ];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let local_selection = super::super::DesignPaintStyleSelection::page("style-local-multi");
    let library_selection =
        super::super::DesignPaintStyleSelection::library("library", "style-remote");
    let view_data = super::super::DesignPaintStyleViewData::new(
        [super::super::DesignPaintStyle::new(
            "style-local-multi",
            "Multi",
            [
                DesignPaint::solid(DesignColor::PURPLE).with_id("style-a"),
                DesignPaint::solid(DesignColor::BLUE).with_id("style-b"),
            ],
        )],
        [super::super::DesignPaintStyleLibrary::new(
            "library",
            "Library",
            [super::super::DesignPaintStyle::new(
                "style-remote",
                "Remote",
                [DesignPaint::solid(DesignColor::WHITE).with_id("style-remote-paint")],
            )
            .with_import_state(super::super::DesignPaintStyleImportState::Available)],
        )],
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_paint_style_view_data(view_data, cx);
        panel.emit_paint_style_apply(DesignPanelCollection::Fill, local_selection.clone(), cx);
        panel.emit_paint_style_import(DesignPanelCollection::Fill, library_selection.clone(), cx);
        panel.emit_paint_style_create(DesignPanelCollection::Fill, cx);

        node.fill_style_binding = Some(super::super::DesignPaintStyleBinding::new(
            local_selection.clone(),
            "Multi",
        ));
        panel.set_node(node.clone(), cx);
        panel.emit_paint_style_detach(DesignPanelCollection::Fill, cx);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 4);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::PaintStyleApplyRequested {
            node_id,
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
            style,
        } if node_id.as_ref() == "rectangle" && style == &local_selection
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::PaintStyleImportRequested {
            collection: DesignPanelCollection::Fill,
            style,
            ..
        } if style == &library_selection
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::PaintStyleCreateRequested {
            collection: DesignPanelCollection::Fill,
            paints,
            ..
        } if paints.iter().map(|paint| paint.id.as_ref()).collect::<Vec<_>>()
            == vec!["fill-a", "fill-b"]
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::PaintStyleDetachRequested {
            collection: DesignPanelCollection::Fill,
            style,
            ..
        } if style == &local_selection
    ));
}

#[gpui::test]
fn appearance_reveals_host_authored_advanced_corner_values(cx: &mut TestAppContext) {
    let mut rectangle =
        DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    rectangle.corner_radii = [4., 8., 12., 16.];
    let (host, visual_cx) = setup(rectangle, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(
            panel.appearance_corner_details_are_visible(),
            "non-uniform host values remain discoverable without presentation state"
        );
    });
}

#[gpui::test]
fn corner_controls_follow_figmas_exact_supported_node_matrix(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text),
        cx,
    );
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        for kind in [
            DesignPanelNodeKind::Frame,
            DesignPanelNodeKind::Section,
            DesignPanelNodeKind::Component,
            DesignPanelNodeKind::ComponentSet,
            DesignPanelNodeKind::Instance,
            DesignPanelNodeKind::Slot,
            DesignPanelNodeKind::Rectangle,
            DesignPanelNodeKind::Ellipse,
            DesignPanelNodeKind::Polygon,
            DesignPanelNodeKind::Star,
            DesignPanelNodeKind::Vector,
            DesignPanelNodeKind::BooleanOperation,
        ] {
            panel.set_node(DesignPanelNode::new("text", kind.label(), kind), cx);
            let expected_radius = panel.node.corner_radii[0];
            assert!(panel.node.corner_capabilities.uniform_radius);
            assert!(panel.property_is_editable(DesignPanelProperty::CornerRadius));
            assert_eq!(
                panel.current_property_value(DesignPanelProperty::CornerRadius),
                Some(DesignPanelValue::Number(expected_radius))
            );
            drop(panel.render_layer(cx));
        }

        for kind in [
            DesignPanelNodeKind::Group,
            DesignPanelNodeKind::TransformGroup,
            DesignPanelNodeKind::Text,
            DesignPanelNodeKind::TextPath,
            DesignPanelNodeKind::Line,
            DesignPanelNodeKind::Slice,
            DesignPanelNodeKind::Other,
        ] {
            panel.set_node(DesignPanelNode::new("text", kind.label(), kind), cx);
            assert!(!panel.node.corner_capabilities.has_any());
            assert!(!panel.property_is_editable(DesignPanelProperty::CornerRadius));
            assert_eq!(
                panel.current_property_value(DesignPanelProperty::CornerRadius),
                None
            );
        }
    });
}

#[gpui::test]
fn frame_fill_show_in_exports_is_controlled_and_capability_gated(cx: &mut TestAppContext) {
    let frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(frame, cx);
    let panel = panel(&host, visual_cx);
    let captured_actions = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::FillShowsInExports),
            Some(DesignPanelValue::Bool(true))
        );
        assert!(panel.property_is_editable(DesignPanelProperty::FillShowsInExports));
        panel.emit_property(
            DesignPanelProperty::FillShowsInExports,
            DesignPanelValue::Bool(false),
            cx,
        );
        assert_eq!(panel.node.fill_shows_in_exports, Some(true));
    });
    visual_cx.run_until_parked();

    assert!(captured_actions.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::FillShowsInExports,
            value: DesignPanelValue::Bool(false),
            ..
        }
    )));

    captured_actions.borrow_mut().clear();
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(
            DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle),
            cx,
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::FillShowsInExports),
            None
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::FillShowsInExports));
        panel.emit_property(
            DesignPanelProperty::FillShowsInExports,
            DesignPanelValue::Bool(false),
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert!(captured_actions.borrow().is_empty());
}

#[test]
fn effect_numeric_clamps_match_each_models_domain() {
    let normalized = DesignPanel::property_clamp(DesignPanelProperty::EffectGlassRefraction(0))
        .expect("normalized clamp");
    assert_eq!(normalized.apply(-1.).expect("finite"), 0.);
    assert_eq!(normalized.apply(2.).expect("finite"), 1.);

    let radius = DesignPanel::property_clamp(DesignPanelProperty::EffectShadowBlur(0))
        .expect("radius clamp");
    assert_eq!(radius.apply(-4.).expect("finite"), 0.);
    assert_eq!(radius.apply(24.).expect("finite"), 24.);

    let depth =
        DesignPanel::property_clamp(DesignPanelProperty::EffectGlassDepth(0)).expect("depth clamp");
    assert_eq!(depth.apply(0.).expect("finite"), 1.);
    assert_eq!(depth.apply(32.).expect("finite"), 32.);

    assert!(
        DesignPanel::property_clamp(DesignPanelProperty::EffectShadowSpread(0)).is_none(),
        "shadow spread intentionally allows negative values"
    );
}

#[gpui::test]
fn canonical_export_view_normalizes_sizing_and_emits_stable_ids(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("slice", "Slice", DesignPanelNodeKind::Slice);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["slice".into()],
    };
    let mut configuration =
        DesignExportConfiguration::new("stable-svg-row", DesignExportFormat::Svg);
    configuration.sizing = DesignExportSizing::Width(640.);

    panel.update(visual_cx, |panel, cx| {
        panel.set_export_view_data(
            DesignExportViewData {
                target: target.clone(),
                configurations: vec![configuration],
                mode: DesignExportMode::Static,
                static_capabilities: Default::default(),
                preview: Some(DesignExportPreviewState::Idle),
                animated: None,
            },
            cx,
        );
        panel.emit_export_configuration_change(
            "stable-svg-row".into(),
            DesignExportConfigurationChange::Suffix("-icon".into()),
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::ExportSizing(0)),
            Some(DesignPanelValue::ExportSizing(DesignExportSizing::Scale(
                1.
            )))
        );
        assert!(panel.export_preview_available());
    });

    let captured = actions(&host, visual_cx);
    assert!(captured.borrow().iter().any(|action| {
        matches!(
            action,
            DesignPanelAction::ExportConfigurationChangeRequested {
                target: DesignPanelTarget::Nodes { node_ids },
                configuration_id,
                change: DesignExportConfigurationChange::Suffix(suffix),
                phase: DesignPanelEditPhase::Commit,
            } if node_ids == &vec![SharedString::from("slice")]
                && configuration_id.as_ref() == "stable-svg-row"
                && suffix.as_ref() == "-icon"
        )
    }));
}

#[gpui::test]
fn animated_export_is_host_controlled_and_preview_is_omitted_for_multiple_selection(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("motion-frame", "Motion frame", DesignPanelNodeKind::Frame);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["motion-frame".into()],
    };
    let settings = DesignAnimatedExportSettings::Mp4 {
        sizing: DesignExportSizing::Scale(1.),
        fps: DesignVideoExportFps::Fps30,
        quality: DesignExportImageQuality::High,
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_export_view_data(
            DesignExportViewData {
                target: target.clone(),
                configurations: vec![DesignExportConfiguration::new(
                    "static",
                    DesignExportFormat::Png,
                )],
                mode: DesignExportMode::Animated,
                static_capabilities: Default::default(),
                preview: Some(DesignExportPreviewState::Ready(
                    super::super::DesignExportPreview::new(1920, 1080)
                        .with_estimated_output("1.8 MB"),
                )),
                animated: Some(super::super::DesignAnimatedExportViewData::new(
                    super::super::DesignAnimatedExportCapability::eligible(1920, 1080),
                    settings.clone(),
                )),
            },
            cx,
        );
        panel.emit_animated_export_change(
            DesignAnimatedExportChange::VideoFps(DesignVideoExportFps::Fps60),
            cx,
        );
        panel.emit_animated_export(cx);
        panel.emit_export_mode_change(DesignExportMode::Static, cx);
        assert!(!panel.export_preview_available());
    });
    visual_cx.run_until_parked();

    assert!(captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::AnimatedExportChangeRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            change: DesignAnimatedExportChange::VideoFps(DesignVideoExportFps::Fps60),
            phase: DesignPanelEditPhase::Commit,
        } if node_ids == &vec![SharedString::from("motion-frame")]
    )));
    assert!(captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::AnimatedExportRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            settings: emitted,
        } if node_ids == &vec![SharedString::from("motion-frame")] && emitted == &settings
    )));
    assert!(captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::ExportModeChangeRequested {
            target: DesignPanelTarget::Nodes { node_ids },
            mode: DesignExportMode::Static,
        } if node_ids == &vec![SharedString::from("motion-frame")]
    )));

    panel.update(visual_cx, |panel, cx| {
        let mut view_data = panel
            .active_export_view_data()
            .expect("export view")
            .clone();
        view_data.mode = DesignExportMode::Static;
        panel.set_export_view_data(view_data, cx);
        assert!(panel.export_preview_available());
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(
                    node.clone(),
                    DesignPanelNode::new(
                        "motion-frame-two",
                        "Motion frame two",
                        DesignPanelNodeKind::Frame,
                    ),
                ),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        assert!(!panel.export_preview_available());
    });
}

#[gpui::test]
fn component_property_edits_emit_stable_typed_values(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    let label_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "label")
        .expect("label property");
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.emit_property(
            DesignPanelProperty::ComponentProperty(label_index),
            DesignPanelValue::Text("Proceed".into()),
            cx,
        );
    });

    assert!(captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyChangeRequested {
            node_id,
            property_id,
            value: DesignComponentPropertyValue::Text(value),
        } if node_id.as_ref() == "button-instance"
            && property_id.as_ref() == "label"
            && value.as_ref() == "Proceed"
    )));
    assert!(!captured.borrow().iter().any(|action| matches!(
        action,
        DesignPanelAction::PropertyChangeRequested {
            property: DesignPanelProperty::ComponentProperty(_),
            ..
        }
    )));
}

#[gpui::test]
fn component_property_variables_emit_exact_apply_import_and_detach_intents(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("button-main", "Button", DesignPanelNodeKind::Component);
    let boolean_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "show-icon")
        .expect("boolean property");
    let text_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "label")
        .expect("text property");
    node.component_properties[boolean_index].default_value_binding =
        Some(super::super::DesignComponentPropertyVariableBinding::new(
            "current-visibility",
            "Current visibility",
            super::super::DesignVariableResolvedValue::Boolean(true),
        ));
    let original = node.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let visibility = DesignVariable::page(
        "next-visibility",
        "Next visibility",
        "behavior",
        "Behavior",
        super::super::DesignVariableResolvedType::Boolean,
    )
    .with_resolved_value(super::super::DesignVariableResolvedValue::Boolean(false));
    let campaign_label = DesignVariable::page(
        "campaign-label",
        "Campaign label",
        "copy",
        "Component copy",
        super::super::DesignVariableResolvedType::String,
    )
    .with_source(DesignVariableSource::library(
        "copy-library",
        "Copy library",
    ))
    .with_import_state(DesignVariableImportState::Available)
    .with_resolved_value(super::super::DesignVariableResolvedValue::String(
        "Start free trial\nNo credit card required".into(),
    ));

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_variable_view_data(
            DesignVariableViewData::new([visibility, campaign_label]),
            cx,
        );
        assert!(
            !panel.property_is_editable(DesignPanelProperty::ComponentProperty(boolean_index)),
            "an active definition default binding gates raw value editing"
        );
        panel.emit_component_property_variable_apply(boolean_index, "next-visibility".into(), cx);
        panel.emit_component_property_variable_import(text_index, "campaign-label".into(), cx);
        panel.emit_component_property_variable_detach(
            boolean_index,
            "current-visibility".into(),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert_eq!(visual_cx.read(|app| panel.read(app).node.clone()), original);
    let captured = captured.borrow();
    assert_eq!(captured.len(), 3);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::ComponentPropertyVariableApplyRequested {
            target,
            variable_id,
            ..
        } if target.property_id.as_ref() == "show-icon"
            && target.field
                == super::super::DesignComponentPropertyVariableField::DefinitionDefaultValue
            && target.field.api_name() == "defaultValue"
            && target.resolved_type == super::super::DesignVariableResolvedType::Boolean
            && variable_id.as_ref() == "next-visibility"
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::ComponentPropertyVariableImportRequested {
            target,
            variable_id,
            ..
        } if target.property_id.as_ref() == "label"
            && target.resolved_type == super::super::DesignVariableResolvedType::String
            && variable_id.as_ref() == "campaign-label"
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::ComponentPropertyVariableDetachRequested {
            target,
            variable_id,
            ..
        } if target.property_id.as_ref() == "show-icon"
            && variable_id.as_ref() == "current-visibility"
    ));
}

#[gpui::test]
fn component_swap_browser_intents_preserve_catalog_identity_and_controlled_state(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    let swap_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "leading-icon")
        .expect("instance-swap property");
    let original = node.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let imported = DesignComponentSwapCandidate::library(
        super::super::DesignComponentReference::remote(
            "icon-check",
            "Check",
            "Product foundations",
        ),
        "library-key-check",
        "product-foundations",
        "Product foundations",
    )
    .with_import_state(super::super::DesignComponentImportState::Imported);
    let available = DesignComponentSwapCandidate::library(
        super::super::DesignComponentReference::remote(
            "icon-sparkle",
            "Sparkle",
            "Marketing components",
        ),
        "library-key-sparkle",
        "marketing-components",
        "Marketing components",
    );
    let imported_selection = imported.selection();
    let available_selection = available.selection();

    panel.update(visual_cx, |panel, cx| {
        panel.set_component_swap_view_data(
            DesignComponentSwapViewData::new([imported, available]),
            cx,
        );
        panel.set_component_swap_preview(swap_index, Some(imported_selection.clone()), cx);
        panel.set_component_swap_preview(swap_index, None, cx);
        panel.emit_component_swap_import(swap_index, available_selection.clone(), cx);
        panel.emit_component_swap_apply(swap_index, Some(imported_selection.clone()), cx);
        panel.emit_component_swap_apply(swap_index, None, cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(visual_cx.read(|app| panel.read(app).node.clone()), original);
    let captured = captured.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapPreviewRequested {
            selection: Some(selection),
            ..
        } if selection == &imported_selection
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapPreviewRequested {
            selection: None,
            ..
        }
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapImportRequested { selection, .. }
            if selection == &available_selection
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapApplyRequested {
            selection: Some(selection),
            ..
        } if selection == &imported_selection
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapApplyRequested {
            selection: None,
            ..
        }
    )));
}

#[gpui::test]
fn multiline_component_text_emits_begin_preview_commit_and_cancel_without_local_mutation(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    let label_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "label")
        .expect("label property");
    node.component_properties[label_index] = node.component_properties[label_index]
        .clone()
        .with_multiline(true);
    let original = node.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_component_multiline_editor(label_index, window, cx);
            panel.component_multiline_input.update(cx, |input, cx| {
                input.set_value("Line one\nLine two", window, cx);
            });
            panel.preview_component_multiline(cx);
            panel.finish_component_multiline_editor(true, window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_component_multiline_editor(label_index, window, cx);
            panel.component_multiline_input.update(cx, |input, cx| {
                input.set_value("Temporary\ncopy", window, cx);
            });
            panel.preview_component_multiline(cx);
            panel.finish_component_multiline_editor(false, window, cx);
        });
    });
    visual_cx.run_until_parked();

    assert_eq!(visual_cx.read(|app| panel.read(app).node.clone()), original);
    let captured = captured.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            property_id,
            phase: DesignPanelEditPhase::Begin,
            ..
        } if property_id.as_ref() == "label"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            value: DesignComponentPropertyValue::Text(value),
            phase: DesignPanelEditPhase::Preview,
            ..
        } if value.as_ref() == "Line one\nLine two"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            value: DesignComponentPropertyValue::Text(value),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if value.as_ref() == "Line one\nLine two"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            value: DesignComponentPropertyValue::Text(value),
            phase: DesignPanelEditPhase::Cancel,
            ..
        } if value.as_ref() == "Continue"
    )));
}

#[gpui::test]
fn multiline_focus_survives_reorder_and_restores_after_save_and_cancel(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    let label_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "label")
        .expect("label property");
    node.component_properties[label_index] = node.component_properties[label_index]
        .clone()
        .with_multiline(true);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.focus_handle.focus(window, cx);
            panel.open_component_multiline_editor_from_control(label_index, window, cx);
        });
    });
    visual_cx.run_until_parked();
    let return_handle = visual_cx.read(|app| {
        let panel = panel.read(app);
        let return_focus = panel
            .editor_focus_return
            .as_ref()
            .expect("multiline activation should retain its row");
        assert_eq!(
            return_focus.origin,
            EditorFocusOrigin::ComponentMultiline("label".into())
        );
        return_focus.handle.clone()
    });

    panel.update(visual_cx, |panel, cx| {
        let other_index = if label_index == 0 { 1 } else { 0 };
        node.component_properties.swap(label_index, other_index);
        panel.set_node(node.clone(), cx);
        assert_eq!(
            panel
                .component_multiline_editor
                .as_ref()
                .map(|editor| editor.property_id.as_ref()),
            Some("label")
        );
    });
    visual_cx.run_until_parked();

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_component_multiline_editor(true, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            let index = panel
                .component_property_index("label")
                .expect("rebased label property");
            panel.open_component_multiline_editor_from_control(index, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .component_multiline_editor
                .as_ref()
                .map(|editor| editor.property_id.clone())
        }),
        Some(SharedString::from("label"))
    );

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_component_multiline_editor(false, window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert!(visual_cx.update(|window, _| return_handle.is_focused(window)));
}

#[gpui::test]
fn permission_downgrade_cancels_multiline_and_clears_swap_preview(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    let label_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "label")
        .expect("label property");
    node.component_properties[label_index] = node.component_properties[label_index]
        .clone()
        .with_multiline(true);
    let swap_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "leading-icon")
        .expect("swap property");
    let candidate = DesignComponentSwapCandidate::local(
        super::super::DesignComponentReference::local("icon-check", "Check"),
        "icon-check-key",
        "icons-page",
        "Icons",
    );
    let selection = candidate.selection();
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_component_swap_view_data(DesignComponentSwapViewData::new([candidate]), cx);
            panel.open_component_multiline_editor(label_index, window, cx);
            panel.set_component_swap_preview(swap_index, Some(selection.clone()), cx);
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node.clone(),
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::viewer(),
                ),
                cx,
            );
            assert!(panel.component_multiline_editor.is_none());
            assert!(panel.component_swap_hovered.is_none());
        });
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            property_id,
            value: DesignComponentPropertyValue::Text(value),
            phase: DesignPanelEditPhase::Cancel,
            ..
        } if property_id.as_ref() == "label" && value.as_ref() == "Continue"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentSwapPreviewRequested {
            property_id,
            selection: None,
            ..
        } if property_id.as_ref() == "leading-icon"
    )));
    assert!(!captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyEditRequested {
            phase: DesignPanelEditPhase::Commit,
            ..
        }
    )));
}

#[gpui::test]
fn nested_component_property_navigation_remains_available_to_viewers(cx: &mut TestAppContext) {
    let main = super::super::DesignComponentReference::local("badge-main", "Badge");
    let mut node = DesignPanelNode::new("button-instance", "Button", DesignPanelNodeKind::Instance);
    node.component_properties.push(
        super::super::DesignComponentProperty::text("nested-label", "Badge label", "New", "Ready")
            .from_nested_instance("badge-instance", "Status badge", Some(main)),
    );
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
    });
    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(panel.component_action_is_enabled(
            &DesignPanelAction::ComponentPropertyNestedInstanceSelectRequested {
                node_id: "button-instance".into(),
                property_id: "nested-label".into(),
                instance_id: "badge-instance".into(),
            }
        ));
        assert!(panel.component_action_is_enabled(
            &DesignPanelAction::ComponentPropertyNestedInstanceGoToMainRequested {
                node_id: "button-instance".into(),
                property_id: "nested-label".into(),
                instance_id: "badge-instance".into(),
                main_component_id: "badge-main".into(),
            }
        ));
    });
}

fn component_authoring_test_node() -> DesignPanelNode {
    let mut node =
        DesignPanelNode::new("button-main", "Button main", DesignPanelNodeKind::Component);
    node.component_properties.insert(
        0,
        super::super::DesignComponentProperty::variant(
            "state",
            "State",
            "Default",
            "Default",
            vec!["Default".into(), "Hover".into()],
        ),
    );
    let definitions = node
        .component_properties
        .iter()
        .map(|property| {
            let mut definition = super::super::DesignComponentPropertyDefinitionAuthoring::editable(
                property.id.clone(),
            );
            if property.definition.kind() == DesignComponentPropertyKind::Variant {
                definition = definition.with_variant_options([
                    super::super::DesignComponentVariantOptionAuthoring::editable(
                        "state-default",
                        "Default",
                    ),
                    super::super::DesignComponentVariantOptionAuthoring::editable(
                        "state-hover",
                        "Hover",
                    ),
                ]);
            }
            definition
        })
        .collect();
    let show_icon = super::super::DesignComponentPropertyChoice::new(
        "show-icon",
        "Show icon",
        DesignComponentPropertyKind::Boolean,
    );
    let label = super::super::DesignComponentPropertyChoice::new(
        "label",
        "Label",
        DesignComponentPropertyKind::Text,
    );
    let context = node
        .component_context
        .as_mut()
        .expect("component preset context");
    context.authoring = Some(super::super::DesignComponentAuthoringViewData {
        create_kinds: vec![
            DesignComponentPropertyKind::Variant,
            DesignComponentPropertyKind::Boolean,
            DesignComponentPropertyKind::Text,
            DesignComponentPropertyKind::InstanceSwap,
            DesignComponentPropertyKind::Slot,
        ],
        definitions,
        applied_properties: vec![
            super::super::DesignAppliedComponentPropertyControl::new(
                "visible-control",
                "icon-layer",
                "Icon layer",
                super::super::DesignComponentPropertyApplicationSurface::Appearance,
                [show_icon.clone(), label],
            )
            .applied_to("show-icon"),
        ],
        exposure_candidates: vec![
            super::super::DesignNestedComponentPropertyExposureCandidate::new(
                "nested-visible",
                "nested-icon",
                "Nested icon",
                show_icon,
            ),
        ],
    });
    node
}

#[gpui::test]
fn component_authoring_dialog_is_live_modal_and_escape_clears_each_draft(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(component_authoring_test_node(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let dialog_layer = visual_cx.update(|_, app| app.new(|_| TestDialogLayer));

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.begin_component_property_create(DesignComponentPropertyKind::Boolean, window, cx);
            assert!(panel.component_authoring_dialog_open);
            assert!(!panel.component_authoring_uses_inline_modal_fallback());
        });
        assert!(window.has_active_dialog(app));
    });
    _ = visual_cx.draw(
        gpui::point(px(0.), px(0.)),
        gpui::size(px(1280.), px(720.)),
        |_, _| dialog_layer.clone().into_any_element(),
    );
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert_eq!(
            panel.component_authoring_dialog_last_rendered_kind,
            Some(DesignComponentPropertyKind::Boolean),
            "the native Dialog builder renders the Boolean draft",
        );
    });

    panel.update(visual_cx, |panel, cx| {
        let draft = panel
            .component_property_create_draft
            .as_mut()
            .expect("live create draft");
        draft.kind = DesignComponentPropertyKind::Slot;
        draft.default_variable_id = None;
        draft.definition = DesignComponentPropertyDefinition::Slot {
            default_value: super::super::DesignSlotValue::default(),
            settings: super::super::DesignSlotSettings::default(),
        };
        cx.notify();
    });
    _ = visual_cx.draw(
        gpui::point(px(0.), px(0.)),
        gpui::size(px(1280.), px(720.)),
        |_, _| dialog_layer.clone().into_any_element(),
    );
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert_eq!(
            panel.component_authoring_dialog_last_rendered_kind,
            Some(DesignComponentPropertyKind::Slot),
            "the Dialog builder rereads the current typed draft every render",
        );
    });

    visual_cx.simulate_keystrokes("escape");
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert!(panel.component_property_create_draft.is_none());
        assert!(panel.component_property_edit_modal.is_none());
        assert!(!panel.component_authoring_dialog_open);
        assert!(!panel.component_authoring_uses_inline_modal_fallback());
    });
    visual_cx.update(|window, app| {
        assert!(!window.has_active_dialog(app));
    });

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_component_property_edit("label".into(), window, cx);
            assert!(panel.component_authoring_dialog_open);
            assert!(!panel.component_authoring_uses_inline_modal_fallback());
        });
        assert!(window.has_active_dialog(app));
    });
    _ = visual_cx.draw(
        gpui::point(px(0.), px(0.)),
        gpui::size(px(1280.), px(720.)),
        |_, _| dialog_layer.into_any_element(),
    );
    visual_cx.run_until_parked();
    visual_cx.simulate_keystrokes("escape");
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert!(panel.component_property_create_draft.is_none());
        assert!(panel.component_property_edit_modal.is_none());
        assert!(!panel.component_authoring_dialog_open);
    });
    visual_cx.update(|window, app| {
        assert!(!window.has_active_dialog(app));
    });
    assert!(
        captured.borrow().is_empty(),
        "canceling uncommitted modal drafts emits no document intent",
    );
}

#[gpui::test]
fn component_property_edit_modal_emits_one_atomic_combined_slot_intent(cx: &mut TestAppContext) {
    let mut node = component_authoring_test_node();
    let property_id = SharedString::from("content-slot");
    let old_description = Some(SharedString::from("Original Slot guidance"));
    let old_documentation_links = vec![super::super::DesignDocumentationLink::new(
        "Original Slot docs",
        "https://example.com/slot/original",
    )];
    let old_settings = super::super::DesignSlotSettings {
        minimum_children: Some(1),
        maximum_children: Some(4),
        ..super::super::DesignSlotSettings::default()
    };
    let old_definition = DesignComponentPropertyDefinition::Slot {
        default_value: super::super::DesignSlotValue::default(),
        settings: old_settings.clone(),
    };
    let mut property = super::super::DesignComponentProperty::slot(
        property_id.clone(),
        "Content",
        super::super::DesignSlotValue::default(),
        super::super::DesignSlotValue::default(),
        old_settings,
        super::super::DesignSlotState::default(),
    )
    .with_description("Original Slot guidance");
    property.documentation_links = old_documentation_links.clone();
    node.component_properties.push(property);
    node.component_context
        .as_mut()
        .and_then(|context| context.authoring.as_mut())
        .expect("component authoring fixture")
        .definitions
        .push(
            super::super::DesignComponentPropertyDefinitionAuthoring::editable(property_id.clone()),
        );

    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.open_component_property_edit(property_id.clone(), window, cx);
            panel
                .component_authoring_default_input
                .update(cx, |input, cx| {
                    input.set_value("Updated Slot guidance", window, cx);
                });
            panel
                .component_authoring_slot_minimum_input
                .update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            panel
                .component_authoring_slot_maximum_input
                .update(cx, |input, cx| {
                    input.set_value("2", window, cx);
                });
            let draft = panel
                .component_property_edit_modal
                .as_mut()
                .expect("open Edit property modal");
            draft
                .documentation_links
                .push(super::super::DesignDocumentationLink::new(
                    "Insertion behavior",
                    "https://example.com/slot/insertion",
                ));
            let DesignComponentPropertyDefinition::Slot { settings, .. } = &mut draft.definition
            else {
                panic!("Slot edit draft");
            };
            settings.stretch_child_on_insert = false;
            settings.display_empty = false;
            settings.preferred_values_only = true;
            panel.finish_component_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();
    panel.update(visual_cx, |panel, _| {
        assert!(!panel.component_authoring_dialog_open);
        assert!(panel.component_property_edit_modal.is_none());
    });
    visual_cx.update(|window, app| {
        assert!(
            !window.has_active_dialog(app),
            "a valid Save closes the one native Dialog exactly once",
        );
    });

    let actions = captured.borrow();
    assert_eq!(
        actions.len(),
        1,
        "one modal confirmation must produce one host transaction",
    );
    let DesignPanelAction::ComponentPropertyDefinitionEditRequested {
        node_id,
        property_id: emitted_property_id,
        expected_description,
        description,
        expected_documentation_links,
        documentation_links,
        expected_definition,
        definition,
    } = &actions[0]
    else {
        panic!("combined modal confirmation must use the atomic edit intent");
    };
    assert_eq!(node_id.as_ref(), "button-main");
    assert_eq!(emitted_property_id, &property_id);
    assert_eq!(expected_description, &old_description);
    assert_eq!(
        description.as_ref().map(SharedString::as_ref),
        Some("Updated Slot guidance"),
    );
    assert_eq!(
        expected_documentation_links, &old_documentation_links,
        "documentation links remain lossless in the stale guard",
    );
    assert_eq!(documentation_links.len(), 2);
    assert_eq!(expected_definition, &old_definition);
    let DesignComponentPropertyDefinition::Slot { settings, .. } = definition else {
        panic!("atomic definition remains typed as Slot");
    };
    assert_eq!(settings.minimum_children, None);
    assert_eq!(settings.maximum_children, Some(2));
    assert!(!settings.stretch_child_on_insert);
    assert!(!settings.display_empty);
    assert!(settings.preferred_values_only);
    drop(actions);

    panel.update(visual_cx, |panel, _| {
        let stale = DesignPanelAction::ComponentPropertyDefinitionEditRequested {
            node_id: "button-main".into(),
            property_id,
            expected_description: Some("Stale description".into()),
            description: Some("Must not apply".into()),
            expected_documentation_links: old_documentation_links.clone(),
            documentation_links: old_documentation_links,
            expected_definition: old_definition.clone(),
            definition: DesignComponentPropertyDefinition::Slot {
                default_value: super::super::DesignSlotValue::default(),
                settings: super::super::DesignSlotSettings {
                    maximum_children: Some(1),
                    ..super::super::DesignSlotSettings::default()
                },
            },
        };
        assert!(
            !panel.component_authoring_action_is_enabled(&stale),
            "a stale combined edit is rejected before emission",
        );
    });
}

#[gpui::test]
fn component_authoring_intents_preserve_stable_identity_and_variant_partition(
    cx: &mut TestAppContext,
) {
    let node = component_authoring_test_node();
    let original = node.clone();
    let regular_order = node
        .component_properties
        .iter()
        .filter(|property| {
            DesignComponentPropertyPartition::for_kind(property.definition.kind())
                == DesignComponentPropertyPartition::Regular
        })
        .map(|property| property.id.clone())
        .collect::<Vec<_>>();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let create = DesignPanelAction::ComponentPropertyDefinitionCreateRequested {
        node_id: "button-main".into(),
        kind: DesignComponentPropertyKind::Text,
        name: "New text".into(),
        description: None,
        documentation_links: Vec::new(),
        definition: DesignComponentPropertyDefinition::Text {
            default_value: "Text".into(),
            multiline: false,
        },
        default_variable_id: None,
        partition: DesignComponentPropertyPartition::Regular,
        expected_property_order: regular_order.clone(),
        after_property_id: Some("content".into()),
    };
    let rename = DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
        node_id: "button-main".into(),
        property_id: "label".into(),
        original_name: "Label".into(),
        expected_name: "Label".into(),
        name: "Label".into(),
        phase: DesignPanelEditPhase::Begin,
    };
    let valid_reorder = DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
        node_id: "button-main".into(),
        property_id: "show-icon".into(),
        partition: DesignComponentPropertyPartition::Regular,
        original_property_order: regular_order.clone(),
        expected_property_order: regular_order.clone(),
        before_property_id: Some("label".into()),
        phase: DesignPanelEditPhase::Begin,
    };
    let duplicate_property_order = vec![regular_order[0].clone(); regular_order.len()];
    let duplicate_property_reorder =
        DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            node_id: "button-main".into(),
            property_id: "show-icon".into(),
            partition: DesignComponentPropertyPartition::Regular,
            original_property_order: duplicate_property_order,
            expected_property_order: regular_order.clone(),
            before_property_id: Some("label".into()),
            phase: DesignPanelEditPhase::Begin,
        };
    let cross_partition = DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
        node_id: "button-main".into(),
        property_id: "show-icon".into(),
        partition: DesignComponentPropertyPartition::Regular,
        original_property_order: regular_order.clone(),
        expected_property_order: regular_order,
        before_property_id: Some("state".into()),
        phase: DesignPanelEditPhase::Begin,
    };
    let duplicate_option_reorder = DesignPanelAction::ComponentVariantOptionReorderRequested {
        node_id: "button-main".into(),
        property_id: "state".into(),
        option_id: "state-hover".into(),
        original_option_order: vec!["state-default".into(), "state-default".into()],
        expected_option_order: vec!["state-default".into(), "state-hover".into()],
        before_option_id: Some("state-default".into()),
        phase: DesignPanelEditPhase::Begin,
    };
    let option_rename = DesignPanelAction::ComponentVariantOptionRenameRequested {
        node_id: "button-main".into(),
        property_id: "state".into(),
        option_id: "state-hover".into(),
        original_name: "Hover".into(),
        expected_name: "Hover".into(),
        name: "Hover".into(),
        phase: DesignPanelEditPhase::Begin,
    };
    let preview = |preview| DesignPanelAction::NestedComponentPropertyPreviewRequested {
        node_id: "button-main".into(),
        candidate_id: "nested-visible".into(),
        nested_instance_id: "nested-icon".into(),
        nested_property_id: "show-icon".into(),
        preview,
    };

    panel.update(visual_cx, |panel, cx| {
        assert!(panel.component_authoring_action_is_enabled(&create));
        assert!(panel.component_authoring_action_is_enabled(&rename));
        assert!(panel.component_authoring_action_is_enabled(&valid_reorder));
        assert!(
            !panel.component_authoring_action_is_enabled(&duplicate_property_reorder),
            "duplicate IDs are not an exact partition membership proof",
        );
        assert!(!panel.component_authoring_action_is_enabled(&cross_partition));
        assert!(
            !panel.component_authoring_action_is_enabled(&duplicate_option_reorder),
            "duplicate IDs are not an exact Variant-value membership proof",
        );
        assert!(panel.component_authoring_action_is_enabled(&option_rename));
        assert!(panel.component_authoring_action_is_enabled(&preview(true)));
        assert!(
            panel
                .component_authoring_view_data()
                .expect("authoring projection")
                .preserves_variant_partition(&panel.node.component_properties)
        );

        drop(panel.render_component_definition_authoring(cx));
        drop(panel.render_applied_component_property_controls(
            DesignComponentPropertyApplicationSurface::Appearance,
            cx,
        ));
        drop(panel.render_nested_component_property_exposures(cx));

        panel.emit_component_authoring_action(rename.clone(), cx);
        panel.emit_component_authoring_action(valid_reorder.clone(), cx);
        panel.emit_component_authoring_action(option_rename.clone(), cx);
        panel.emit_component_authoring_action(preview(true), cx);
        panel.emit_component_authoring_action(preview(false), cx);
        assert_eq!(panel.node, original, "the panel must wait for host echoes");

        let mut echoed = panel.node.clone();
        echoed
            .component_properties
            .retain(|property| property.id.as_ref() != "label");
        echoed
            .component_context
            .as_mut()
            .and_then(|context| context.authoring.as_mut())
            .expect("authoring projection")
            .definitions
            .retain(|definition| definition.property_id.as_ref() != "label");
        panel.set_node(echoed, cx);
        assert!(
            !panel.component_authoring_action_is_enabled(&rename),
            "a stale property identity must not fall back to its old index"
        );
        panel.emit_component_authoring_action(rename.clone(), cx);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 5);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
            property_id,
            ..
        } if property_id.as_ref() == "label"
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
            property_id,
            before_property_id: Some(before),
            partition: DesignComponentPropertyPartition::Regular,
            ..
        } if property_id.as_ref() == "show-icon" && before.as_ref() == "label"
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::ComponentVariantOptionRenameRequested {
            option_id,
            ..
        } if option_id.as_ref() == "state-hover"
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::NestedComponentPropertyPreviewRequested { preview: true, .. }
    ));
    assert!(matches!(
        &captured[4],
        DesignPanelAction::NestedComponentPropertyPreviewRequested { preview: false, .. }
    ));
}

#[gpui::test]
fn component_authoring_native_create_inline_rename_and_drag_are_balanced_and_host_controlled(
    cx: &mut TestAppContext,
) {
    let node = component_authoring_test_node();
    let original = node.clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            drop(panel.render_component_definition_authoring(cx));

            panel.begin_component_property_create(DesignComponentPropertyKind::Text, window, cx);
            panel
                .component_authoring_name_input
                .update(cx, |input, cx| {
                    input.set_value("Supporting text", window, cx);
                });
            panel
                .component_authoring_default_input
                .update(cx, |input, cx| {
                    input.set_value("Details", window, cx);
                });
            panel.submit_component_property_create(window, cx);

            panel.begin_component_property_rename("label".into(), window, cx);
            panel
                .component_authoring_name_input
                .update(cx, |input, cx| {
                    input.set_value("Primary label", window, cx);
                });
            panel.preview_component_authoring_name(cx);
            panel.finish_component_authoring_name_edit(true, window, cx);

            let original_order =
                panel.component_property_partition_order(DesignComponentPropertyPartition::Regular);
            let drag = ComponentPropertyDefinitionDrag {
                node_id: panel.node.id.clone(),
                property_id: "content".into(),
                property_name: "Content".into(),
                partition: DesignComponentPropertyPartition::Regular,
                original_order,
            };
            panel.begin_component_property_reorder(&drag, cx);
            panel.preview_component_property_reorder(Some("label".into()), cx);
            panel.finish_component_property_reorder(false, cx);

            assert_eq!(
                panel.node, original,
                "native authoring presentation must wait for every host echo"
            );
            assert!(panel.component_property_create_draft.is_none());
            assert!(panel.component_authoring_name_editor.is_none());
            assert!(panel.component_property_reorder.is_none());
        });
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::ComponentPropertyDefinitionCreateRequested {
            kind: DesignComponentPropertyKind::Text,
            name,
            definition: DesignComponentPropertyDefinition::Text {
                default_value,
                ..
            },
            ..
        } if name.as_ref() == "Supporting text" && default_value.as_ref() == "Details"
    )));
    let rename_phases = captured
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::ComponentPropertyDefinitionRenameRequested {
                property_id,
                phase,
                ..
            } if property_id.as_ref() == "label" => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rename_phases,
        vec![
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
        ]
    );
    let reorder_phases = captured
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::ComponentPropertyDefinitionReorderRequested {
                property_id,
                phase,
                ..
            } if property_id.as_ref() == "content" => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reorder_phases,
        vec![
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Cancel,
        ]
    );
}

#[gpui::test]
fn component_authoring_respects_permissions_item_capabilities_and_echoed_state(
    cx: &mut TestAppContext,
) {
    let node = component_authoring_test_node();
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let apply = DesignPanelAction::ComponentPropertyApplyToLayerRequested {
        node_id: "button-main".into(),
        control_id: "visible-control".into(),
        layer_id: "icon-layer".into(),
        surface: DesignComponentPropertyApplicationSurface::Appearance,
        property_id: "label".into(),
    };
    let switch = DesignPanelAction::ComponentPropertySwitchOnLayerRequested {
        node_id: "button-main".into(),
        control_id: "visible-control".into(),
        layer_id: "icon-layer".into(),
        surface: DesignComponentPropertyApplicationSurface::Appearance,
        from_property_id: "show-icon".into(),
        to_property_id: "label".into(),
    };
    let detach = DesignPanelAction::ComponentPropertyDetachFromLayerRequested {
        node_id: "button-main".into(),
        control_id: "visible-control".into(),
        layer_id: "icon-layer".into(),
        surface: DesignComponentPropertyApplicationSurface::Appearance,
        property_id: "show-icon".into(),
    };
    let expose = DesignPanelAction::NestedComponentPropertyExposeRequested {
        node_id: "button-main".into(),
        candidate_id: "nested-visible".into(),
        nested_instance_id: "nested-icon".into(),
        nested_property_id: "show-icon".into(),
    };

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(!panel.component_authoring_action_is_enabled(&apply));
        assert!(panel.component_authoring_action_is_enabled(&switch));
        assert!(panel.component_authoring_action_is_enabled(&detach));
        assert!(panel.component_authoring_action_is_enabled(&expose));
    });

    panel.update(visual_cx, |panel, cx| {
        let mut echoed = node.clone();
        let authoring = echoed
            .component_context
            .as_mut()
            .and_then(|context| context.authoring.as_mut())
            .expect("authoring projection");
        authoring.applied_properties[0].applied_property_id = None;
        panel.set_node(echoed.clone(), cx);
        assert!(panel.component_authoring_action_is_enabled(&apply));
        assert!(!panel.component_authoring_action_is_enabled(&switch));
        assert!(!panel.component_authoring_action_is_enabled(&detach));

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                echoed,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(!panel.component_authoring_action_is_enabled(&apply));
        assert!(!panel.component_authoring_action_is_enabled(&expose));
    });
}

#[gpui::test]
fn slot_definition_settings_emit_typed_transactions_and_not_values(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("slot", "Content", DesignPanelNodeKind::Slot);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(!panel.property_is_editable(DesignPanelProperty::ComponentProperty(0)));
        assert!(panel.property_is_editable(DesignPanelProperty::SlotStretchChildOnInsert(0)));
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::SlotMinimumInstances(0)),
            Some(DesignPanelValue::OptionalNumber(None))
        );
        panel.emit_property(
            DesignPanelProperty::SlotStretchChildOnInsert(0),
            DesignPanelValue::Bool(false),
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::SlotMaximumInstances(0),
            DesignPanelValue::OptionalNumber(Some(6.)),
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::SlotMinimumInstances(0),
            DesignPanelValue::OptionalNumber(None),
            DesignPanelEditPhase::Commit,
            cx,
        );
    });

    let captured = captured.borrow();
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::SlotSettingsChangeRequested {
            property_id,
            change: DesignSlotSettingsChange::StretchChildOnInsert(false),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if property_id.as_ref() == "content"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::SlotSettingsChangeRequested {
            property_id,
            change: DesignSlotSettingsChange::MaximumInstances(Some(6)),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if property_id.as_ref() == "content"
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::SlotSettingsChangeRequested {
            property_id,
            change: DesignSlotSettingsChange::MinimumInstances(None),
            phase: DesignPanelEditPhase::Commit,
            ..
        } if property_id.as_ref() == "content"
    )));
}

#[gpui::test]
fn component_actions_honor_role_reset_availability_and_advisory_slot_limits(
    cx: &mut TestAppContext,
) {
    let mut instance = DesignPanelNode::new("instance", "Button", DesignPanelNodeKind::Instance);
    instance
        .component_context
        .as_mut()
        .expect("component context")
        .main_component
        .as_mut()
        .expect("main component")
        .availability = super::super::DesignComponentAvailability::Missing;
    let (host, visual_cx) = setup(instance, cx);
    let panel = panel(&host, visual_cx);

    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert!(!panel.component_action_is_enabled(
            &DesignPanelAction::GoToMainComponentRequested {
                node_id: "instance".into(),
            }
        ));
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::DetachInstanceRequested {
                node_id: "instance".into(),
            })
        );
        assert!(panel.component_action_is_enabled(
            &DesignPanelAction::ResetInstanceOverridesRequested {
                node_id: "instance".into(),
            }
        ));
    });

    panel.update(visual_cx, |panel, cx| {
        let mut slot = DesignPanelNode::new("slot-instance", "Content", DesignPanelNodeKind::Slot);
        slot.component_context = Some(super::super::DesignComponentContext {
            role: DesignComponentRole::SlotInstance,
            main_component: Some(super::super::DesignComponentReference::local(
                "main", "Main",
            )),
            description: None,
            documentation_links: Vec::new(),
            overrides: super::super::DesignComponentOverrideSummary {
                overridden_property_count: 1,
                nested_override_count: 0,
                reset_state: DesignComponentResetState::Resettable,
            },
            authoring: None,
        });
        let property = slot
            .component_properties
            .first_mut()
            .expect("slot property");
        property.resolved_value =
            DesignComponentPropertyValue::Slot(super::super::DesignSlotValue {
                children: (0..5)
                    .map(|index| {
                        super::super::DesignSlotChild::instance(
                            format!("child-{index}"),
                            format!("Child {index}"),
                        )
                    })
                    .collect(),
            });
        property.value = property.resolved_value.display_value();
        property.slot_state = Some(super::super::DesignSlotState {
            violations: Vec::new(),
            reset_state: DesignComponentResetState::Resettable,
        });
        property.refresh_slot_violations();
        panel.set_node(slot, cx);
    });
    visual_cx.run_until_parked();

    visual_cx.read(|app| {
        let panel = panel.read(app);
        let add = DesignPanelAction::SlotAddInstanceRequested {
            node_id: "slot-instance".into(),
            property_id: "content".into(),
            preferred_component: None,
        };
        assert!(
            panel.component_action_is_enabled(&add),
            "Figma maximum layer counts are guidance and must not disable Add"
        );
        assert!(
            panel
                .node
                .component_properties
                .first()
                .and_then(|property| property.slot_state.as_ref())
                .is_some_and(|state| state.violations.iter().any(|violation| matches!(
                    violation,
                    super::super::DesignSlotViolation::AboveMaximum {
                        maximum: 4,
                        actual: 5,
                    }
                )))
        );
        let warning = panel.node.component_properties[0]
            .slot_state
            .as_ref()
            .and_then(|state| {
                state.violations.iter().find(|violation| {
                    matches!(
                        violation,
                        super::super::DesignSlotViolation::AboveMaximum { .. }
                    )
                })
            })
            .expect("above-maximum advisory");
        assert_eq!(
            warning.label().as_ref(),
            "Recommended maximum is 4 layers; currently 5",
        );
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::SlotResetRequested {
                node_id: "slot-instance".into(),
                property_id: "content".into(),
            })
        );
        assert!(
            !panel.component_action_is_enabled(&DesignPanelAction::DetachInstanceRequested {
                node_id: "slot-instance".into(),
            })
        );
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::SlotChildSelectRequested {
                node_id: "slot-instance".into(),
                property_id: "content".into(),
                child_node_id: "child-0".into(),
            })
        );
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::SlotChildRemoveRequested {
                node_id: "slot-instance".into(),
                property_id: "content".into(),
                child_node_id: "child-0".into(),
                index: 0,
            })
        );
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::SlotChildReorderRequested {
                node_id: "slot-instance".into(),
                property_id: "content".into(),
                child_node_id: "child-0".into(),
                from_index: 0,
                to_index: 1,
            })
        );
        assert!(
            panel.component_action_is_enabled(&DesignPanelAction::SlotChildReplaceRequested {
                node_id: "slot-instance".into(),
                property_id: "content".into(),
                child_node_id: "child-0".into(),
                replacement: super::super::DesignComponentReference::local(
                    "replacement",
                    "Replacement",
                ),
            })
        );
    });
}

#[test]
fn slot_limit_guidelines_distinguish_met_unmet_and_unknown_host_state() {
    let children = vec![
        super::super::DesignSlotChild::instance("preferred", "Preferred"),
        super::super::DesignSlotChild::instance("wrong-instance", "Wrong instance"),
        super::super::DesignSlotChild::layer(
            "wrong-layer",
            "Loose layer",
            DesignPanelNodeKind::Text,
        ),
    ];
    let settings = super::super::DesignSlotSettings {
        minimum_children: Some(2),
        maximum_children: Some(2),
        preferred_values_only: true,
        ..super::super::DesignSlotSettings::default()
    };
    let value = super::super::DesignSlotValue { children };
    let mut property = DesignComponentProperty::slot(
        "content",
        "Content",
        super::super::DesignSlotValue::default(),
        value,
        settings,
        super::super::DesignSlotState {
            violations: vec![
                DesignSlotViolation::NonPreferredChild {
                    child_id: "wrong-layer".into(),
                    child_name: "Loose layer".into(),
                },
                DesignSlotViolation::AboveMaximum {
                    maximum: 2,
                    actual: 3,
                },
                DesignSlotViolation::NonPreferredValue {
                    instance_id: "wrong-instance".into(),
                    component_name: "Wrong instance".into(),
                },
            ],
            reset_state: DesignComponentResetState::Clean,
        },
    );

    let guidelines = slot_limit_guidelines(&property);
    assert_eq!(
        guidelines
            .iter()
            .map(|guideline| (guideline.kind, guideline.status))
            .collect::<Vec<_>>(),
        vec![
            (
                SlotLimitGuidelineKind::MinimumLayers,
                SlotLimitGuidelineStatus::Met,
            ),
            (
                SlotLimitGuidelineKind::MaximumLayers,
                SlotLimitGuidelineStatus::Unmet,
            ),
            (
                SlotLimitGuidelineKind::PreferredInstancesOnly,
                SlotLimitGuidelineStatus::Unmet,
            ),
        ],
    );
    assert_eq!(
        guidelines[0].label.as_ref(),
        "Minimum layers: 2 · Currently 3"
    );
    assert_eq!(
        slot_preferred_violation_layer_ids(&property),
        vec![
            SharedString::from("wrong-instance"),
            SharedString::from("wrong-layer"),
        ],
        "bulk selection follows the authoritative Slot child order, not violation order",
    );

    property.slot_state = None;
    assert!(
        slot_limit_guidelines(&property)
            .iter()
            .all(|guideline| guideline.status == SlotLimitGuidelineStatus::Unknown),
        "missing host validation state must never be presented as success",
    );
    assert!(slot_preferred_violation_layer_ids(&property).is_empty());
}

#[gpui::test]
fn slot_limits_disclosure_emits_exact_view_layers_and_reconciles_host_echo(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("slot-instance", "Card", DesignPanelNodeKind::Slot);
    node.component_context = Some(super::super::DesignComponentContext {
        role: DesignComponentRole::SlotInstance,
        main_component: Some(super::super::DesignComponentReference::local(
            "card-main",
            "Card",
        )),
        description: None,
        documentation_links: Vec::new(),
        overrides: super::super::DesignComponentOverrideSummary::default(),
        authoring: None,
    });
    let preferred = super::super::DesignComponentReference::local("approved", "Approved content");
    let mut approved =
        super::super::DesignSlotChild::instance("approved-layer", "Approved content");
    approved.main_component = Some(preferred.clone());
    let mut wrong = super::super::DesignSlotChild::instance("wrong-layer", "Unapproved content");
    wrong.main_component = Some(super::super::DesignComponentReference::local(
        "unapproved",
        "Unapproved content",
    ));
    node.component_properties = vec![DesignComponentProperty::slot(
        "content",
        "Content",
        super::super::DesignSlotValue::default(),
        super::super::DesignSlotValue {
            children: vec![approved, wrong],
        },
        super::super::DesignSlotSettings {
            minimum_children: Some(1),
            maximum_children: Some(3),
            preferred_values_only: true,
            preferred_values: vec![preferred],
            ..super::super::DesignSlotSettings::default()
        },
        super::super::DesignSlotState {
            violations: vec![DesignSlotViolation::NonPreferredValue {
                instance_id: "wrong-layer".into(),
                component_name: "Unapproved content".into(),
            }],
            reset_state: DesignComponentResetState::Resettable,
        },
    )];

    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    visual_cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(px(160.), px(360.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-420.))),
        ..Default::default()
    });
    visual_cx.run_until_parked();
    let limits = visual_cx
        .debug_bounds("slot-limits-content")
        .expect("configured consuming Slot renders a Limits disclosure");
    visual_cx.simulate_click(limits.center(), Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| panel.read(app).open_slot_limits.clone()),
        Some(SharedString::from("content")),
    );

    let view_layers = visual_cx
        .debug_bounds("design-slot-view-layers-content")
        .expect("unmet preferred-only guidance renders View layers");
    visual_cx.simulate_click(view_layers.center(), Modifiers::none());
    assert_eq!(
        captured.borrow().last(),
        Some(&DesignPanelAction::SlotLimitLayersSelectRequested {
            node_id: "slot-instance".into(),
            property_id: "content".into(),
            child_node_ids: vec!["wrong-layer".into()],
        }),
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_node(node.clone(), cx);
        assert!(
            panel
                .open_slot_limits
                .as_ref()
                .is_some_and(|property_id| property_id.as_ref() == "content")
        );
        node.component_properties[0].slot_state = None;
        panel.set_node(node, cx);
        assert!(
            panel
                .open_slot_limits
                .as_ref()
                .is_some_and(|property_id| property_id.as_ref() == "content"),
            "unknown validation remains inspectable as a neutral guideline",
        );
        let DesignComponentPropertyDefinition::Slot { settings, .. } =
            &mut panel.node.component_properties[0].definition
        else {
            panic!("Slot definition");
        };
        settings.minimum_children = None;
        settings.maximum_children = None;
        settings.preferred_values_only = false;
        panel.reconcile_slot_limits_state();
        assert!(panel.open_slot_limits.is_none());
    });
}

#[gpui::test]
fn slot_limit_layers_selection_rejects_a_stale_node_id_before_emitting(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("current-slot", "Card", DesignPanelNodeKind::Slot);
    node.component_context = Some(super::super::DesignComponentContext {
        role: DesignComponentRole::SlotInstance,
        main_component: Some(super::super::DesignComponentReference::local(
            "card-main",
            "Card",
        )),
        description: None,
        documentation_links: Vec::new(),
        overrides: super::super::DesignComponentOverrideSummary::default(),
        authoring: None,
    });
    let preferred = super::super::DesignComponentReference::local("approved", "Approved content");
    let mut wrong = super::super::DesignSlotChild::instance("wrong-layer", "Unapproved content");
    wrong.main_component = Some(super::super::DesignComponentReference::local(
        "unapproved",
        "Unapproved content",
    ));
    node.component_properties = vec![DesignComponentProperty::slot(
        "content",
        "Content",
        super::super::DesignSlotValue::default(),
        super::super::DesignSlotValue {
            children: vec![wrong],
        },
        super::super::DesignSlotSettings {
            preferred_values_only: true,
            preferred_values: vec![preferred],
            ..super::super::DesignSlotSettings::default()
        },
        super::super::DesignSlotState {
            violations: vec![DesignSlotViolation::NonPreferredValue {
                instance_id: "wrong-layer".into(),
                component_name: "Unapproved content".into(),
            }],
            reset_state: DesignComponentResetState::Resettable,
        },
    )];

    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let stale = DesignPanelAction::SlotLimitLayersSelectRequested {
        node_id: "stale-slot".into(),
        property_id: "content".into(),
        child_node_ids: vec!["wrong-layer".into()],
    };
    let exact = DesignPanelAction::SlotLimitLayersSelectRequested {
        node_id: "current-slot".into(),
        property_id: "content".into(),
        child_node_ids: vec!["wrong-layer".into()],
    };

    panel.update(visual_cx, |panel, cx| {
        assert!(
            !panel.component_action_is_enabled(&stale),
            "an otherwise exact Slot action cannot target a stale selected node",
        );
        panel.emit_component_action(stale.clone(), cx);

        assert!(panel.component_action_is_enabled(&exact));
        panel.emit_component_action(exact.clone(), cx);
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [exact],
        "only the exact current-node action crosses the controlled intent boundary",
    );
}

#[gpui::test]
fn generic_property_variables_emit_controlled_apply_import_and_detach_intents(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame);
    let original_width = node.width;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let width = DesignVariable::page(
        "size-card",
        "Card",
        "sizes",
        "Layout sizes",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_scopes([DesignVariableScope::WidthHeight])
    .with_resolved_value(super::super::DesignVariableResolvedValue::Float(320.));
    let radius = DesignVariable::page(
        "radius-container",
        "Container",
        "radius",
        "Radius",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_source(DesignVariableSource::library("library", "Library"))
    .with_scopes([DesignVariableScope::CornerRadius])
    .with_import_state(DesignVariableImportState::Available);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_variable_view_data(DesignVariableViewData::new([width, radius]), cx);
        panel.set_property_value_state(
            DesignPanelProperty::Width,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        panel.emit_property_variable_apply(DesignPanelProperty::Width, "size-card".into(), cx);
        panel.emit_property_variable_apply(
            DesignPanelProperty::CornerRadius,
            "radius-container".into(),
            cx,
        );
        panel.emit_property_variable_import(
            DesignPanelProperty::CornerRadius,
            "radius-container".into(),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::Opacity,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "opacity-variable",
                "Opacity",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(80.),
            )),
            cx,
        );
        panel.emit_property_variable_detach(
            DesignPanelProperty::Opacity,
            "opacity-variable".into(),
            cx,
        );
    });
    visual_cx.run_until_parked();

    assert_eq!(
        visual_cx.read(|app| panel.read(app).node.width),
        original_width
    );
    let captured = captured.borrow();
    assert_eq!(captured.len(), 3);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::PropertyVariableApplyRequested {
            target,
            variable_id,
            ..
        } if target.property == DesignPanelProperty::Width
            && variable_id.as_ref() == "size-card"
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::PropertyVariableImportRequested {
            target,
            variable_id,
            ..
        } if target.fields.len() == 4
            && variable_id.as_ref() == "radius-container"
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::PropertyVariableDetachRequested {
            target,
            variable_id,
            ..
        } if target.property == DesignPanelProperty::Opacity
            && variable_id.as_ref() == "opacity-variable"
    ));
}

#[gpui::test]
fn typography_variable_controls_keep_family_style_and_numeric_weight_distinct(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let family = DesignVariable::page(
        "font-family-body",
        "Body family",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::String,
    )
    .with_scopes([DesignVariableScope::FontFamily])
    .with_resolved_value(super::super::DesignVariableResolvedValue::String(
        "Inter".into(),
    ));
    let style = DesignVariable::page(
        "font-style-body",
        "Body style",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::String,
    )
    .with_scopes([DesignVariableScope::FontStyle])
    .with_resolved_value(super::super::DesignVariableResolvedValue::String(
        "Medium".into(),
    ));
    let weight = DesignVariable::page(
        "font-weight-body",
        "Body weight",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_scopes([DesignVariableScope::FontWeight])
    .with_resolved_value(super::super::DesignVariableResolvedValue::Float(500.));
    let importable_weight = DesignVariable::page(
        "font-weight-display",
        "Display weight",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_source(super::super::DesignVariableSource::library(
        "type-library",
        "Type library",
    ))
    .with_scopes([DesignVariableScope::FontWeight])
    .with_import_state(DesignVariableImportState::Available)
    .with_resolved_value(super::super::DesignVariableResolvedValue::Float(700.));

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_variable_view_data(
            DesignVariableViewData::new([family, style, weight, importable_weight]),
            cx,
        );
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("text edit mode"),
            cx,
        );

        let expected_target = panel.typography_target(DesignPanelProperty::FontWeight);
        for (property, field, resolved_type, scope) in [
            (
                DesignPanelProperty::FontFamily,
                "fontFamily",
                super::super::DesignVariableResolvedType::String,
                DesignVariableScope::FontFamily,
            ),
            (
                DesignPanelProperty::FontStyle,
                "fontStyle",
                super::super::DesignVariableResolvedType::String,
                DesignVariableScope::FontStyle,
            ),
            (
                DesignPanelProperty::FontWeight,
                "fontWeight",
                super::super::DesignVariableResolvedType::Float,
                DesignVariableScope::FontWeight,
            ),
        ] {
            let target = panel
                .property_variable_target(property)
                .expect("typography variable target");
            assert_eq!(target.fields[0].api_name(), field);
            assert_eq!(target.resolved_type, resolved_type);
            assert_eq!(target.compatible_scope, Some(scope));
            assert_eq!(target.typography_target, Some(expected_target));
            assert!(panel.property_variable_button_state(property).is_some());
        }
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::FontWeight),
            Some(DesignPanelValue::Number(400.))
        );
        assert!(panel.layout_property_value_is_applicable(
            DesignPanelProperty::FontWeight,
            &DesignPanelValue::Number(1.)
        ));
        assert!(!panel.layout_property_value_is_applicable(
            DesignPanelProperty::FontWeight,
            &DesignPanelValue::Number(0.)
        ));
        drop(panel.render_font_browser(cx));
        drop(panel.render_typography(cx));

        panel.set_property_value_state(
            DesignPanelProperty::FontFamily,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontFamily,
            "font-family-body".into(),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::FontStyle,
            DesignPanelPropertyValueState::Unset,
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontStyle,
            "font-style-body".into(),
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontWeight,
            "font-weight-body".into(),
            cx,
        );
        panel.emit_property_variable_import(
            DesignPanelProperty::FontWeight,
            "font-weight-display".into(),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::FontWeight,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "font-weight-body",
                "Typography / Body weight",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::Number(500.),
            )),
            cx,
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::FontWeight));
        panel.emit_property_variable_detach(
            DesignPanelProperty::FontWeight,
            "font-weight-body".into(),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::FontWeight,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(500.))
                .read_only_with_reason("Locked range"),
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontWeight,
            "font-weight-body".into(),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 5);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::PropertyVariableApplyRequested { target, .. }
            if target.property == DesignPanelProperty::FontFamily
                && target.typography_target.is_some()
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::PropertyVariableApplyRequested { target, .. }
            if target.property == DesignPanelProperty::FontStyle
                && target.typography_target.is_some()
    ));
    assert!(matches!(
        &captured[2],
        DesignPanelAction::PropertyVariableApplyRequested {
            target,
            variable_id,
            ..
        } if target.property == DesignPanelProperty::FontWeight
            && target.resolved_type == super::super::DesignVariableResolvedType::Float
            && variable_id.as_ref() == "font-weight-body"
    ));
    assert!(matches!(
        &captured[3],
        DesignPanelAction::PropertyVariableImportRequested {
            target,
            variable_id,
            ..
        } if target.property == DesignPanelProperty::FontWeight
            && variable_id.as_ref() == "font-weight-display"
    ));
    assert!(matches!(
        &captured[4],
        DesignPanelAction::PropertyVariableDetachRequested {
            target,
            variable_id,
            ..
        } if target.property == DesignPanelProperty::FontWeight
            && variable_id.as_ref() == "font-weight-body"
    ));
}

#[gpui::test]
fn font_weight_uses_the_selected_range_continuous_edit_lifecycle(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography.as_mut().expect("text typography").weight = 400.;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let expected_target = visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node,
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                )
                .with_edit_mode(DesignPanelEditMode::Text)
                .expect("text edit mode"),
                cx,
            );
            let target = panel.typography_target(DesignPanelProperty::FontWeight);
            panel.activate_property(
                DesignPanelProperty::FontWeight,
                DesignPanelValue::Number(500.),
                window,
                cx,
            );
            target
        })
    });
    let input = visual_cx.read(|app| panel.read(app).property_input.clone());
    visual_cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_value("+100", window, cx);
        });
    });
    visual_cx.run_until_parked();
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.finish_property_edit(true, window, cx);
        });
    });
    visual_cx.run_until_parked();

    let edits = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TypographyPropertyEditRequested {
                target,
                property: DesignPanelProperty::FontWeight,
                value,
                phase,
                ..
            } => Some((*target, value.clone(), *phase)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edits,
        vec![
            (
                expected_target,
                DesignPanelValue::Number(400.),
                DesignPanelEditPhase::Begin,
            ),
            (
                expected_target,
                DesignPanelValue::Number(500.),
                DesignPanelEditPhase::Preview,
            ),
            (
                expected_target,
                DesignPanelValue::Number(500.),
                DesignPanelEditPhase::Commit,
            ),
        ]
    );
    assert_eq!(
        visual_cx.read(|app| {
            panel
                .read(app)
                .node
                .typography
                .as_ref()
                .expect("text typography")
                .weight
        }),
        400.,
        "the inspector remains controlled until the host echoes the edit"
    );
}

#[gpui::test]
fn selected_text_range_revision_terminates_the_previous_range_transaction(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.typography.as_mut().expect("text typography").weight = 400.;
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            let first_range = DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("Text mode")
            .with_text_range_revision(7)
            .expect("exact range");
            panel.set_inspection_context(first_range, cx);
            let mut same_range_echo = node.clone();
            same_range_echo.opacity = 88.;
            panel.set_node(same_range_echo, cx);
            assert_eq!(panel.inspection_context.text_range_revision(), Some(7));
            panel.activate_property(
                DesignPanelProperty::FontWeight,
                DesignPanelValue::Number(400.),
                window,
                cx,
            );

            let second_range = DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("Text mode")
            .with_text_range_revision(8)
            .expect("next exact range");
            panel.set_inspection_context(second_range, cx);
            assert!(panel.property_editor.is_none());
            assert_eq!(panel.inspection_context.text_range_revision(), Some(8));
            panel.emit_property(
                DesignPanelProperty::FontWeight,
                DesignPanelValue::Number(500.),
                cx,
            );
        });
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::TypographyPropertyEditRequested {
                target: DesignTypographyTarget::SelectedTextRangeRevision(7),
                property: DesignPanelProperty::FontWeight,
                phase: DesignPanelEditPhase::Begin,
                ..
            },
            DesignPanelAction::TypographyPropertyEditRequested {
                target: DesignTypographyTarget::SelectedTextRangeRevision(7),
                property: DesignPanelProperty::FontWeight,
                phase: DesignPanelEditPhase::Cancel,
                ..
            },
            DesignPanelAction::TypographyPropertyChangeRequested {
                target: DesignTypographyTarget::SelectedTextRangeRevision(8),
                property: DesignPanelProperty::FontWeight,
                ..
            }
        ]
    ));
}

#[gpui::test]
fn fill_intents_follow_exact_text_range_revisions_while_stroke_stays_layer_wide(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    node.fills = vec![DesignPaint::solid(DesignColor::WHITE).with_id("text-fill")];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            let first_range = DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("Text mode")
            .with_text_range_revision(7)
            .expect("exact Fill range");
            panel.set_inspection_context(first_range, cx);
            assert_eq!(
                panel.paint_target(DesignPanelCollection::Fill),
                DesignPaintTarget::SelectedTextRangeRevision(7)
            );
            assert_eq!(
                panel.paint_target(DesignPanelCollection::Stroke),
                DesignPaintTarget::WholeLayer
            );
            panel.activate_property(
                DesignPanelProperty::PaintOpacity {
                    collection: DesignPanelCollection::Fill,
                    index: 0,
                },
                DesignPanelValue::Number(100.),
                window,
                cx,
            );

            let second_range = DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("Text mode")
            .with_text_range_revision(8)
            .expect("next exact Fill range");
            panel.set_inspection_context(second_range, cx);
            assert!(panel.property_editor.is_none());
            panel.emit_property(
                DesignPanelProperty::PaintOpacity {
                    collection: DesignPanelCollection::Fill,
                    index: 0,
                },
                DesignPanelValue::Number(80.),
                cx,
            );
            panel.emit_add(DesignPanelCollection::Fill, cx);
            panel.emit_add(DesignPanelCollection::Stroke, cx);
            panel.emit_paint_style_create(DesignPanelCollection::Fill, cx);
        });
    });
    visual_cx.run_until_parked();

    let observed = captured.borrow();
    assert!(matches!(
        observed.as_slice(),
        [
            DesignPanelAction::PaintEditRequested {
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::SelectedTextRangeRevision(7),
                phase: DesignPanelEditPhase::Begin,
                ..
            },
            DesignPanelAction::PaintEditRequested {
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::SelectedTextRangeRevision(7),
                phase: DesignPanelEditPhase::Cancel,
                ..
            },
            DesignPanelAction::PaintEditRequested {
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::SelectedTextRangeRevision(8),
                phase: DesignPanelEditPhase::Commit,
                ..
            },
            DesignPanelAction::CollectionItemAddRequested {
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::SelectedTextRangeRevision(8),
                ..
            },
            DesignPanelAction::CollectionItemAddRequested {
                collection: DesignPanelCollection::Stroke,
                target: DesignPaintTarget::WholeLayer,
                ..
            },
            DesignPanelAction::PaintStyleCreateRequested {
                collection: DesignPanelCollection::Fill,
                target: DesignPaintTarget::SelectedTextRangeRevision(8),
                ..
            }
        ]
    ));
}

#[gpui::test]
fn font_weight_variable_intent_preserves_the_exact_multiple_selection_target(
    cx: &mut TestAppContext,
) {
    let first = DesignPanelNode::new("text-one", "One", DesignPanelNodeKind::Text);
    let second = DesignPanelNode::new("text-two", "Two", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let variable = DesignVariable::page(
        "font-weight-body",
        "Body weight",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_scopes([DesignVariableScope::FontWeight])
    .with_resolved_value(super::super::DesignVariableResolvedValue::Float(500.));

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::FontWeight,
            DesignPanelPropertyValueState::Mixed,
            cx,
        );
        panel.set_property_variable_view_data(DesignVariableViewData::new([variable]), cx);
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontWeight,
            "font-weight-body".into(),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 1);
    let (target, leaf) = captured[0]
        .targeted_node_action()
        .expect("multi-selection variable intent uses a target envelope");
    assert_eq!(
        target,
        &DesignPanelTarget::Nodes {
            node_ids: vec!["text-one".into(), "text-two".into()],
        }
    );
    assert!(matches!(
        leaf,
        DesignPanelAction::PropertyVariableApplyRequested {
            node_id,
            target,
            variable_id,
        } if node_id.as_ref() == "text-one"
            && target.property == DesignPanelProperty::FontWeight
            && target.typography_target.is_some()
            && variable_id.as_ref() == "font-weight-body"
    ));
}

#[gpui::test]
fn generic_property_variables_obey_text_range_read_only_and_viewer_gates(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("text", "Text", DesignPanelNodeKind::Text);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let font_size = DesignVariable::page(
        "font-size-body",
        "Body",
        "typography",
        "Typography",
        super::super::DesignVariableResolvedType::Float,
    )
    .with_scopes([DesignVariableScope::FontSize]);

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_variable_view_data(DesignVariableViewData::new([font_size]), cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Text)
            .expect("text edit context"),
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontSize,
            "font-size-body".into(),
            cx,
        );
        panel.set_property_value_state(
            DesignPanelProperty::FontSize,
            DesignPanelPropertyValueState::Uniform(DesignPanelValue::Number(16.))
                .read_only_with_reason("Locked"),
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontSize,
            "font-size-body".into(),
            cx,
        );
        panel.set_property_value_states([], cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_property_variable_apply(
            DesignPanelProperty::FontSize,
            "font-size-body".into(),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 1);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::PropertyVariableApplyRequested { target, .. }
            if target.typography_target
                == Some(DesignTypographyTarget::SelectedTextRange)
    ));
}

#[gpui::test]
fn vector_edit_emits_stable_id_phases_without_mutating_controlled_vertices(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("vector", "Vector", DesignPanelNodeKind::Vector);
    node.vector_edit = Some(DesignVectorEditViewData::new([
        super::super::DesignVectorVertexViewData::new("vertex-b", 48., 24.)
            .with_corner_radius(Some(12.))
            .with_handle_mirroring(Some(DesignHandleMirroring::Angle))
            .selected(true),
        super::super::DesignVectorVertexViewData::new("vertex-a", 16., 24.)
            .with_topology(super::super::DesignVectorVertexTopology::Endpoint)
            .with_corner_radius(Some(4.))
            .with_handle_mirroring(Some(DesignHandleMirroring::None))
            .selected(true),
    ]));
    let original = node.vector_edit.clone();
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Vector)
            .expect("vector edit context"),
            cx,
        );
        assert!(panel.render_vector_edit_selection(cx).is_some());
        assert_eq!(
            format_vector_number_state(DesignVectorSelectionValue::Mixed).as_ref(),
            "Mixed"
        );
        assert_eq!(
            format_vector_handle_state(DesignVectorSelectionValue::Mixed).as_ref(),
            "Mixed"
        );
        panel.emit_vector_vertex_selection(
            vec!["vertex-a".into()],
            DesignPanelEditPhase::Commit,
            cx,
        );
        for (phase, value) in [
            (DesignPanelEditPhase::Begin, 48.),
            (DesignPanelEditPhase::Preview, 52.),
            (DesignPanelEditPhase::Commit, 56.),
            (DesignPanelEditPhase::Cancel, 48.),
        ] {
            panel.emit_property_edit(
                DesignPanelProperty::VectorVertexX,
                DesignPanelValue::Number(value),
                phase,
                cx,
            );
        }
        panel.emit_property_edit(
            DesignPanelProperty::VectorVertexCornerRadius,
            DesignPanelValue::Number(8.),
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::VectorHandleMirroring,
            DesignPanelValue::HandleMirroring(DesignHandleMirroring::AngleAndLength),
            DesignPanelEditPhase::Commit,
            cx,
        );
        assert_eq!(panel.node.vector_edit, original);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert!(matches!(
        &captured[0],
        DesignPanelAction::VectorVertexSelectionEditRequested {
            node_id,
            selected_vertex_ids,
            phase: DesignPanelEditPhase::Commit,
        } if node_id.as_ref() == "vector"
            && selected_vertex_ids == &[SharedString::from("vertex-a")]
    ));
    let coordinate_phases = captured
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::VectorVertexPositionEditRequested {
                vertex_ids,
                axis: DesignVectorCoordinateAxis::X,
                phase,
                ..
            } if vertex_ids
                == &[
                    SharedString::from("vertex-b"),
                    SharedString::from("vertex-a"),
                ] =>
            {
                Some(*phase)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        coordinate_phases,
        vec![
            DesignPanelEditPhase::Begin,
            DesignPanelEditPhase::Preview,
            DesignPanelEditPhase::Commit,
            DesignPanelEditPhase::Cancel,
        ]
    );
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::VectorVertexCornerRadiusEditRequested {
            vertex_ids,
            radius,
            phase: DesignPanelEditPhase::Commit,
            ..
        } if vertex_ids
            == &[SharedString::from("vertex-b"), SharedString::from("vertex-a")]
            && (*radius - 8.).abs() < f32::EPSILON
    )));
    assert!(captured.iter().any(|action| matches!(
        action,
        DesignPanelAction::VectorHandleMirroringEditRequested {
            vertex_ids,
            mirroring: DesignHandleMirroring::AngleAndLength,
            phase: DesignPanelEditPhase::Commit,
            ..
        } if vertex_ids
            == &[SharedString::from("vertex-b"), SharedString::from("vertex-a")]
    )));
}

#[gpui::test]
fn vector_edit_honors_viewer_read_only_branch_and_text_path_gates(cx: &mut TestAppContext) {
    let mut branch = DesignPanelNode::new("branch", "Branch", DesignPanelNodeKind::Vector);
    branch.vector_edit = Some(DesignVectorEditViewData::new([
        super::super::DesignVectorVertexViewData::new("junction", 12., 20.)
            .with_topology(super::super::DesignVectorVertexTopology::Branch)
            .with_corner_radius(None)
            .with_handle_mirroring(None)
            .selected(true),
    ]));
    let (host, visual_cx) = setup(branch.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                branch.clone(),
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Vector)
            .expect("branch vector edit context"),
            cx,
        );
        assert!(panel.render_vector_edit_selection(cx).is_some());
        assert!(panel.property_is_editable(DesignPanelProperty::VectorVertexX));
        assert!(!panel.property_is_editable(DesignPanelProperty::VectorVertexCornerRadius));
        assert!(!panel.property_is_editable(DesignPanelProperty::VectorHandleMirroring));
        panel.emit_property(
            DesignPanelProperty::VectorVertexCornerRadius,
            DesignPanelValue::Number(4.),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::VectorHandleMirroring,
            DesignPanelValue::HandleMirroring(DesignHandleMirroring::Angle),
            cx,
        );
        panel.emit_property(
            DesignPanelProperty::VectorVertexX,
            DesignPanelValue::Number(18.),
            cx,
        );

        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                branch,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        assert!(panel.render_vector_edit_selection(cx).is_none());

        let mut text_path =
            DesignPanelNode::new("text-path", "Text path", DesignPanelNodeKind::TextPath);
        text_path.vector_edit = Some(DesignVectorEditViewData::new([
            super::super::DesignVectorVertexViewData::new("path-vertex", 4., 8.)
                .with_handle_mirroring(Some(DesignHandleMirroring::Angle))
                .selected(true),
        ]));
        panel.set_node(text_path.clone(), cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                text_path,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Vector)
            .expect("TextPath vector edit context"),
            cx,
        );
        assert!(panel.render_vector_edit_selection(cx).is_some());
        panel.emit_property(
            DesignPanelProperty::VectorHandleMirroring,
            DesignPanelValue::HandleMirroring(DesignHandleMirroring::AngleAndLength),
            cx,
        );

        let mut locked =
            DesignPanelNode::new("locked", "Locked vector", DesignPanelNodeKind::Vector);
        locked.vector_edit = Some(
            DesignVectorEditViewData::new([super::super::DesignVectorVertexViewData::new(
                "locked-vertex",
                0.,
                0.,
            )
            .selected(true)])
            .read_only(true)
            .with_disabled_reason("Locked by host"),
        );
        panel.set_node(locked.clone(), cx);
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                locked,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            )
            .with_edit_mode(DesignPanelEditMode::Vector)
            .expect("read-only vector edit context"),
            cx,
        );
        assert!(panel.render_vector_edit_selection(cx).is_some());
        assert!(!panel.property_is_editable(DesignPanelProperty::VectorVertexX));
        panel.emit_vector_vertex_selection(
            vec!["locked-vertex".into()],
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 2);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::VectorVertexPositionEditRequested {
            node_id,
            vertex_ids,
            axis: DesignVectorCoordinateAxis::X,
            phase: DesignPanelEditPhase::Commit,
            ..
        } if node_id.as_ref() == "branch"
            && vertex_ids == &[SharedString::from("junction")]
    ));
    assert!(matches!(
        &captured[1],
        DesignPanelAction::VectorHandleMirroringEditRequested {
            node_id,
            vertex_ids,
            mirroring: DesignHandleMirroring::AngleAndLength,
            ..
        } if node_id.as_ref() == "text-path"
            && vertex_ids == &[SharedString::from("path-vertex")]
    ));
}

#[gpui::test]
fn same_id_echo_cancels_repeat_offset_when_its_unit_changes(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("repeat", "Repeat", DesignPanelNodeKind::TransformGroup);
    node.transform_modifiers = vec![DesignRepeatModifier::linear(
        "repeat-stable",
        DesignRepeatAxis::Horizontal,
    )];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::TransformRepeatOffset(0),
                DesignPanelValue::Number(100.),
                window,
                cx,
            );
            let mut next = node;
            next.transform_modifiers[0].unit = DesignTransformUnit::Pixels;
            panel.set_node(next, cx);
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::TransformModifierChangeRequested {
                modifier_id,
                change: DesignTransformModifierChange::Offset(offset),
                phase,
                ..
            } if modifier_id.as_ref() == "repeat-stable"
                && (*offset - 100.).abs() < f32::EPSILON =>
            {
                Some(*phase)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn same_id_echo_cancels_when_the_underlying_figma_field_changes(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("layout", "Layout", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Gap,
                DesignPanelValue::Number(node.layout.as_ref().expect("horizontal layout").gap),
                window,
                cx,
            );
            let mut next = node;
            next.layout.as_mut().expect("layout").mode = DesignLayoutMode::Grid;
            panel.set_node(next, cx);
            assert!(panel.property_editor.is_none());
            assert!(
                panel
                    .node
                    .layout
                    .as_ref()
                    .and_then(DesignLayout::grid_dimensions)
                    .is_none(),
                "an incomplete host echo remains incomplete rather than inventing tracks"
            );
            assert!(
                !panel.property_is_editable(DesignPanelProperty::GridColumnCount)
                    && !panel.property_is_editable(DesignPanelProperty::GridRowCount),
                "atomic dimensions stay disabled until both host vectors are positive"
            );
            drop(panel.render_grid_dimensions_picker(
                panel.node.layout.as_ref().expect("Grid layout"),
                cx,
            ));
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Gap,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn same_id_echo_terminates_the_original_vector_selection(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("vector", "Vector", DesignPanelNodeKind::Vector);
    node.vector_edit = Some(DesignVectorEditViewData::new([
        super::super::DesignVectorVertexViewData::new("vertex-a", 12., 20.).selected(true),
        super::super::DesignVectorVertexViewData::new("vertex-b", 48., 20.),
    ]));
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::single(
                    node.clone(),
                    DesignPanelParentLayout::Freeform,
                    DesignPanelPermissions::editor(),
                )
                .with_edit_mode(DesignPanelEditMode::Vector)
                .expect("vector context"),
                cx,
            );
            panel.activate_property(
                DesignPanelProperty::VectorVertexX,
                DesignPanelValue::Number(12.),
                window,
                cx,
            );

            let mut next = node;
            next.vector_edit = Some(DesignVectorEditViewData::new([
                super::super::DesignVectorVertexViewData::new("vertex-a", 12., 20.),
                super::super::DesignVectorVertexViewData::new("vertex-b", 48., 20.).selected(true),
            ]));
            panel.set_node(next, cx);
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::VectorVertexPositionEditRequested {
                vertex_ids,
                axis: DesignVectorCoordinateAxis::X,
                phase,
                ..
            } if vertex_ids == &[SharedString::from("vertex-a")] => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn same_id_paint_payload_change_terminates_an_active_crop(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("media", "Media", DesignPanelNodeKind::Rectangle);
    let mut paint = DesignPaint::image(super::super::DesignPaintSource::new(
        "image-source",
        "Image",
    ))
    .with_id("media-fill");
    let DesignPaintPayload::Image(image) = &mut paint.payload else {
        panic!("image paint");
    };
    image.placement = super::super::DesignMediaPaintPlacement::Crop {
        transform: super::super::DesignPaintTransform::IDENTITY,
    };
    node.fills = vec![paint];
    let mut next = node.clone();
    next.fills = vec![DesignPaint::solid(DesignColor::PURPLE).with_id("media-fill")];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_media_paint_view_data(
            DesignMediaPaintViewData::new([super::super::DesignMediaPaintView::new(
                DesignPanelCollection::Fill,
                "media-fill",
                0,
            )
            .with_crop_tool(super::super::DesignMediaCropToolState {
                active: true,
                ..super::super::DesignMediaCropToolState::default()
            })]),
            cx,
        );
        panel.active_picker = Some(PaintPickerTarget {
            collection: DesignPanelCollection::Fill,
            index: 0,
            paint_id: "media-fill".into(),
        });
        panel.set_node(next, cx);
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PaintMediaCropActionRequested {
            node_id,
            paint_id,
            index: 0,
            action: DesignMediaCropAction::Cancel,
            ..
        }] if node_id.as_ref() == "media" && paint_id.as_ref() == "media-fill"
    ));
}

#[gpui::test]
fn host_reorder_without_stable_leaf_ids_cancels_indexed_numeric_edit(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("grid", "Grid", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Grid);
    node.layout.as_mut().expect("grid layout").grid_columns =
        vec![DesignGridTrack::fixed(100.), DesignGridTrack::fixed(200.)];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::GridColumnTrackValue(0),
                DesignPanelValue::Number(100.),
                window,
                cx,
            );
            let mut next = node;
            next.layout
                .as_mut()
                .expect("grid layout")
                .grid_columns
                .swap(0, 1);
            panel.set_node(next, cx);
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::GridColumnTrackValue(0),
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn bound_paint_style_locks_raw_collection_edits_until_detach(cx: &mut TestAppContext) {
    let selection = super::super::DesignPaintStyleSelection::page("brand-fill");
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::solid(DesignColor::BLACK).with_id("fill-a"),
        DesignPaint::solid(DesignColor::WHITE).with_id("fill-b"),
    ];
    node.fill_style_binding = Some(super::super::DesignPaintStyleBinding::new(
        selection.clone(),
        "Brand fill",
    ));
    let first_paint = node.fills[0].clone();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert!(
            !panel.property_is_editable(DesignPanelProperty::PaintOpacity {
                collection: DesignPanelCollection::Fill,
                index: 0,
            })
        );
        assert!(
            !panel.property_is_editable(DesignPanelProperty::PaintVisible {
                collection: DesignPanelCollection::Fill,
                index: 0,
            })
        );
        panel.emit_add(DesignPanelCollection::Fill, cx);
        panel.emit_remove(DesignPanelCollection::Fill, 0, cx);
        panel.emit_paint_reorder(DesignPanelCollection::Fill, &first_paint, 0, 1, cx);
        panel.emit_paint_edit(
            PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "fill-a".into(),
            },
            DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(50.),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_paint_style_detach(DesignPanelCollection::Fill, cx);
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PaintStyleDetachRequested {
            collection: DesignPanelCollection::Fill,
            style,
            ..
        }] if style == &selection
    ));
}

#[gpui::test]
fn read_only_paint_locks_row_controls_and_terminates_an_active_opacity_edit(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills = vec![DesignPaint::solid(DesignColor::BLACK).with_id("fill")];
    let mut read_only_node = node.clone();
    read_only_node.fills[0].read_only = true;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let opacity = DesignPanelProperty::PaintOpacity {
        collection: DesignPanelCollection::Fill,
        index: 0,
    };
    let visibility = DesignPanelProperty::PaintVisible {
        collection: DesignPanelCollection::Fill,
        index: 0,
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            assert!(panel.property_is_editable(opacity));
            assert!(panel.property_is_editable(visibility));
            panel.activate_property(opacity, DesignPanelValue::Number(100.), window, cx);
            assert!(panel.property_editor.is_some());

            panel.set_node(read_only_node, cx);
            assert!(panel.property_editor.is_none());
            assert!(!panel.property_is_editable(opacity));
            assert!(!panel.property_is_editable(visibility));

            panel.emit_property(opacity, DesignPanelValue::Number(50.), cx);
            panel.emit_property(visibility, DesignPanelValue::Bool(false), cx);
            panel.emit_paint_edit(
                PaintPickerTarget {
                    collection: DesignPanelCollection::Fill,
                    index: 0,
                    paint_id: "fill".into(),
                },
                DesignPaintEdit {
                    property: DesignPaintProperty::Opacity,
                    value: DesignPaintValue::Number(50.),
                },
                DesignPanelEditPhase::Commit,
                cx,
            );
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PaintEditRequested {
                paint_id,
                edit:
                    DesignPaintEdit {
                        property: DesignPaintProperty::Opacity,
                        ..
                    },
                phase,
                ..
            } if paint_id.as_ref() == "fill" => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn color_and_opacity_only_fill_hides_structure_and_filters_intents(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("synthetic", "Synthetic", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::solid(DesignColor::rgb(0x12, 0x34, 0x56)).with_id("fill-a"),
        DesignPaint::solid(DesignColor::rgb(0x65, 0x43, 0x21)).with_id("fill-b"),
    ];
    node.capabilities = Some(
        DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::Rectangle)
            .with_fill_edit_mode(DesignPaintCollectionEditMode::ColorAndOpacityOnly),
    );
    let full_node = {
        let mut full_node = node.clone();
        full_node.capabilities = Some(DesignPanelNodeCapabilities::for_node_kind(
            DesignPanelNodeKind::Rectangle,
        ));
        full_node
    };
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = PaintPickerTarget {
        collection: DesignPanelCollection::Fill,
        index: 0,
        paint_id: "fill-a".into(),
    };

    visual_cx.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(px(160.), px(360.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-420.))),
        ..Default::default()
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-fill-paint-opacity-0")
            .is_some()
    );
    for hidden in ["design-add-fill", "design-fill-paint-visible-0"] {
        assert!(
            visual_cx.debug_bounds(hidden).is_none(),
            "{hidden} must not be exposed for a color-and-opacity-only Fill"
        );
    }

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = Some(target.clone());
            panel.sync_paint_picker(window, cx);
            cx.notify();
        });
    });
    visual_cx.run_until_parked();

    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    assert_eq!(
        visual_cx.read(|app| picker.read(app).color_only_title().cloned()),
        Some(SharedString::from("Fill color")),
        "the retained picker must use its color-only rendering path"
    );

    panel.update(visual_cx, |panel, cx| {
        let opacity = DesignPanelProperty::PaintOpacity {
            collection: DesignPanelCollection::Fill,
            index: 0,
        };
        let visibility = DesignPanelProperty::PaintVisible {
            collection: DesignPanelCollection::Fill,
            index: 0,
        };
        assert!(panel.property_is_editable(opacity));
        assert!(!panel.property_is_editable(visibility));

        panel.emit_add(DesignPanelCollection::Fill, cx);
        panel.emit_remove(DesignPanelCollection::Fill, 0, cx);
        let first_paint = panel
            .node
            .fills
            .first()
            .expect("the fixture has a first Fill")
            .clone();
        panel.emit_paint_reorder(DesignPanelCollection::Fill, &first_paint, 0, 1, cx);
        for edit in [
            DesignPaintEdit {
                property: DesignPaintProperty::Visible,
                value: DesignPaintValue::Bool(false),
            },
            DesignPaintEdit {
                property: DesignPaintProperty::BlendMode,
                value: DesignPaintValue::BlendMode(DesignBlendMode::Multiply),
            },
            DesignPaintEdit {
                property: DesignPaintProperty::Payload,
                value: DesignPaintValue::Payload(DesignPaint::solid(DesignColor::BLACK).payload),
            },
        ] {
            panel.emit_paint_edit(target.clone(), edit, DesignPanelEditPhase::Commit, cx);
        }
        panel.emit_paint_edit(
            target.clone(),
            DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(DesignColor::rgb(0xaa, 0xbb, 0xcc)),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_paint_edit(
            target.clone(),
            DesignPaintEdit {
                property: DesignPaintProperty::Opacity,
                value: DesignPaintValue::Number(42.),
            },
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    let actions = captured.borrow();
    assert!(matches!(
        actions.as_slice(),
        [
            DesignPanelAction::PaintEditRequested {
                edit: DesignPaintEdit {
                    property: DesignPaintProperty::Color,
                    ..
                },
                ..
            },
            DesignPanelAction::PaintEditRequested {
                edit: DesignPaintEdit {
                    property: DesignPaintProperty::Opacity,
                    ..
                },
                ..
            }
        ]
    ));
    drop(actions);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_node(full_node, cx);
            panel.sync_paint_picker(window, cx);
        });
    });
    visual_cx.run_until_parked();
    assert_eq!(
        visual_cx.read(|app| picker.read(app).color_only_title().cloned()),
        None,
        "a same-target capability upgrade must restore the full picker"
    );
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.active_picker = None;
            panel.sync_paint_picker(window, cx);
            cx.notify();
        });
    });
    visual_cx.run_until_parked();
    for restored in ["design-add-fill", "design-fill-paint-visible-0"] {
        assert!(
            visual_cx.debug_bounds(restored).is_some(),
            "{restored} must return with full Fill controls"
        );
    }
}

#[gpui::test]
fn controlled_leaf_state_downgrade_terminates_an_active_editor(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.width = 120.;
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.activate_property(
                DesignPanelProperty::Width,
                DesignPanelValue::Number(120.),
                window,
                cx,
            );
            panel.set_property_value_state(
                DesignPanelProperty::Width,
                DesignPanelPropertyValueState::bound(
                    super::super::DesignPanelPropertyBinding::new(
                        "width-variable",
                        "Width",
                        DesignPanelBindingKind::Variable,
                        DesignPanelValue::Number(120.),
                    ),
                ),
                cx,
            );
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::PropertyEditRequested {
                property: DesignPanelProperty::Width,
                phase,
                ..
            } => Some(*phase),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn export_editor_rebases_by_configuration_id_and_cancels_if_format_invalidates_it(
    cx: &mut TestAppContext,
) {
    let node = DesignPanelNode::new("slice", "Slice", DesignPanelNodeKind::Slice);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["slice".into()],
    };
    let view_data = DesignExportViewData {
        target: target.clone(),
        configurations: vec![
            DesignExportConfiguration::new("export-a", DesignExportFormat::Png),
            DesignExportConfiguration::new("export-b", DesignExportFormat::Jpg),
        ],
        mode: DesignExportMode::Static,
        static_capabilities: Default::default(),
        preview: None,
        animated: None,
    };

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_export_view_data(view_data.clone(), cx);
            panel.activate_property(
                DesignPanelProperty::ExportSizing(0),
                DesignPanelValue::ExportSizing(DesignExportSizing::Scale(1.)),
                window,
                cx,
            );

            let mut reordered = view_data;
            reordered.configurations.swap(0, 1);
            panel.set_export_view_data(reordered.clone(), cx);
            assert_eq!(
                panel.property_editor.as_ref().map(|editor| editor.property),
                Some(DesignPanelProperty::ExportSizing(1))
            );

            reordered.configurations[1] =
                DesignExportConfiguration::new("export-a", DesignExportFormat::Svg);
            panel.set_export_view_data(reordered, cx);
            assert!(panel.property_editor.is_none());
        });
    });
    visual_cx.run_until_parked();

    let phases = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::ExportConfigurationChangeRequested {
                configuration_id,
                change: DesignExportConfigurationChange::Sizing(DesignExportSizing::Scale(scale)),
                phase,
                ..
            } if configuration_id.as_ref() == "export-a" && (*scale - 1.).abs() < f32::EPSILON => {
                Some(*phase)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec![DesignPanelEditPhase::Begin, DesignPanelEditPhase::Cancel]
    );
}

#[gpui::test]
fn component_swap_catalog_invalidation_clears_the_host_preview(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("instance", "Button", DesignPanelNodeKind::Instance);
    let swap_index = node
        .component_properties
        .iter()
        .position(|property| property.id.as_ref() == "leading-icon")
        .expect("instance-swap property");
    let candidate = DesignComponentSwapCandidate::local(
        super::super::DesignComponentReference::local("icon-check", "Check"),
        "icon-check-key",
        "icons-page",
        "Icons",
    );
    let selection = candidate.selection();
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_component_swap_view_data(DesignComponentSwapViewData::new([candidate]), cx);
        panel.set_component_swap_preview(swap_index, Some(selection.clone()), cx);
        panel.set_component_swap_view_data(DesignComponentSwapViewData::default(), cx);
        assert!(panel.component_swap_hovered.is_none());
    });
    visual_cx.run_until_parked();

    let previews = captured
        .borrow()
        .iter()
        .filter_map(|action| match action {
            DesignPanelAction::ComponentSwapPreviewRequested { selection, .. } => {
                Some(selection.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(previews, vec![Some(selection), None]);
}

#[gpui::test]
fn media_capability_echo_cancels_an_active_paint_transaction(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("media", "Media", DesignPanelNodeKind::Rectangle);
    node.fills = vec![
        DesignPaint::image(super::super::DesignPaintSource::new(
            "image-source",
            "Image",
        ))
        .with_id("media-fill"),
    ];
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_media_paint_view_data(
            DesignMediaPaintViewData::new([super::super::DesignMediaPaintView::new(
                DesignPanelCollection::Fill,
                "media-fill",
                0,
            )
            .with_capabilities(super::super::DesignMediaPaintCapabilities::editor())]),
            cx,
        );
        panel.active_paint_edit = Some(ActivePaintEdit {
            target: PickerEventTarget {
                node_id: "media".into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "media-fill".into(),
            },
            edit: DesignPaintEdit {
                property: DesignPaintProperty::MediaFilter(
                    super::super::DesignImageFilter::Exposure,
                ),
                value: DesignPaintValue::Number(0.5),
            },
        });
        panel.set_media_paint_view_data(
            DesignMediaPaintViewData::new([super::super::DesignMediaPaintView::new(
                DesignPanelCollection::Fill,
                "media-fill",
                0,
            )
            .with_capabilities(super::super::DesignMediaPaintCapabilities::viewer())]),
            cx,
        );
        assert!(panel.active_paint_edit.is_none());
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PaintEditRequested {
            node_id,
            paint_id,
            edit: DesignPaintEdit {
                property: DesignPaintProperty::MediaFilter(
                    super::super::DesignImageFilter::Exposure
                ),
                ..
            },
            phase: DesignPanelEditPhase::Cancel,
            ..
        }] if node_id.as_ref() == "media" && paint_id.as_ref() == "media-fill"
    ));
}

#[gpui::test]
fn page_background_read_only_echo_terminates_the_color_transaction(cx: &mut TestAppContext) {
    let (host, visual_cx) = setup(
        DesignPanelNode::new("placeholder", "Page", DesignPanelNodeKind::Frame),
        cx,
    );
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let original = DesignColor::rgba(0x12, 0x34, 0x56, 0x78);
    let target = AuxiliaryColorPickerTarget::PageBackground {
        page_id: "page".into(),
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::page(DesignPanelPermissions::editor()),
            cx,
        );
        panel.set_page_view_data(
            DesignPageViewData::new(
                "page",
                super::super::DesignPageBackground::new(original),
                super::super::DesignLocalResourceViewData::default(),
            ),
            cx,
        );
        panel.auxiliary_color_picker = Some(target.clone());
        panel.page_background_picker_open = true;
        panel.active_paint_edit = Some(ActivePaintEdit {
            target: PickerEventTarget {
                node_id: AuxiliaryColorPickerTarget::PICKER_NODE_ID.into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: target.picker_paint_id(),
            },
            edit: DesignPaintEdit {
                property: DesignPaintProperty::Color,
                value: DesignPaintValue::Color(DesignColor::WHITE),
            },
        });
        panel.set_page_view_data(
            DesignPageViewData::new(
                "page",
                super::super::DesignPageBackground::new(original).read_only("Locked"),
                super::super::DesignLocalResourceViewData::default(),
            ),
            cx,
        );
        assert!(panel.active_paint_edit.is_none());
        assert_eq!(panel.auxiliary_color_picker.as_ref(), Some(&target));
        assert!(panel.page_background_picker_open);
    });
    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.sync_paint_picker(window, cx);
            assert_eq!(
                panel.paint_picker.read(cx).color_only_editability(),
                (false, false)
            );
            assert!(!panel.paint_picker.read(cx).is_disabled());
        });
    });
    visual_cx.run_until_parked();

    assert!(matches!(
        captured.borrow().as_slice(),
        [DesignPanelAction::PageBackgroundEditRequested {
            page_id,
            color,
            phase: DesignPanelEditPhase::Cancel,
        }] if page_id.as_ref() == "page"
            && *color == DesignColor::rgba(0xff, 0xff, 0xff, 0x78)
    ));
}

#[gpui::test]
fn text_path_native_flip_is_exact_capability_gated_and_keyboard_accessible(
    cx: &mut TestAppContext,
) {
    let mut node = DesignPanelNode::new("path", "Circular title", DesignPanelNodeKind::TextPath);
    node.capabilities = Some(
        super::super::DesignPanelNodeCapabilities::for_node_kind(DesignPanelNodeKind::TextPath)
            .with_sections([DesignPanelSection::Typography]),
    );
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    assert!(
        visual_cx
            .debug_bounds("design-text-path-flip-orientation")
            .is_some(),
        "native TextPath Flip belongs in Typography"
    );
    assert!(
        visual_cx
            .debug_bounds("design-text-path-start-segment")
            .is_none(),
        "Plugin-API start coordinates are not native sidebar controls"
    );

    let flip = visual_cx
        .debug_bounds("design-text-path-flip-orientation")
        .expect("native flip button")
        .center();
    visual_cx.simulate_click(flip, Modifiers::none());
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::TextPathFlipOrientationRequested {
            node_id: "path".into(),
        }]
    );

    captured.borrow_mut().clear();
    visual_cx.simulate_keystrokes("enter");
    visual_cx.run_until_parked();
    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::TextPathFlipOrientationRequested {
            node_id: "path".into(),
        }],
        "the focused native Button must activate from the keyboard"
    );

    captured.borrow_mut().clear();
    let mut locked = node.clone();
    locked
        .text_path
        .as_mut()
        .expect("TextPath view data")
        .can_flip_orientation = false;
    panel.update(visual_cx, |panel, cx| {
        panel.set_node(locked, cx);
        panel.emit_text_path_flip_orientation(cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured.borrow().is_empty(),
        "the host capability must gate both pointer and programmatic activation"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.emit_text_path_flip_orientation(cx);
    });
    visual_cx.run_until_parked();
    assert!(
        captured.borrow().is_empty(),
        "viewer permissions must keep native Flip inert"
    );
}

#[gpui::test]
fn text_path_start_data_is_controlled_typed_and_node_gated(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("path", "Circular title", DesignPanelNodeKind::TextPath);
    node.text_path
        .as_mut()
        .expect("TextPath native view data")
        .show_start_data_debug_controls = true;
    node.text_path_start_data = super::super::DesignTextPathStartData::new(2, 0.35);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TextPathStartSegment),
            Some(DesignPanelValue::Integer(2))
        );
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::TextPathStartPosition),
            Some(DesignPanelValue::Ratio(0.35))
        );
        panel.emit_property_edit(
            DesignPanelProperty::TextPathStartSegment,
            DesignPanelValue::Integer(4),
            DesignPanelEditPhase::Commit,
            cx,
        );
        panel.emit_property_edit(
            DesignPanelProperty::TextPathStartPosition,
            DesignPanelValue::Ratio(1.25),
            DesignPanelEditPhase::Commit,
            cx,
        );
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 1);
    assert!(matches!(
        &captured[0],
        DesignPanelAction::TextPathStartChangeRequested {
            node_id,
            data: super::super::DesignTextPathStartData {
                segment: 4,
                position,
            },
            phase: DesignPanelEditPhase::Commit,
        } if node_id.as_ref() == "path" && (*position - 0.35).abs() < f32::EPSILON
    ));
}

#[gpui::test]
fn add_auto_layout_emits_exact_ordered_target_without_optimistic_mutation(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Group);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Rectangle);
    let third = DesignPanelNode::new("third", "Third", DesignPanelNodeKind::Ellipse);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["second".into(), "first".into(), "third".into()],
    };

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::with_remaining(second, first.clone(), [third]),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(target.clone()),
            cx,
        );
    });
    visual_cx.run_until_parked();

    let add = visual_cx
        .debug_bounds("design-add-auto-layout")
        .expect("eligible multiple selection should expose Add auto layout")
        .center();
    visual_cx.simulate_click(add, Modifiers::none());
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [DesignPanelAction::AddAutoLayoutRequested { target }]
    );
    visual_cx.read(|app| {
        let panel = panel.read(app);
        assert_eq!(panel.node().id.as_ref(), "second");
        assert_eq!(panel.node().kind, DesignPanelNodeKind::Rectangle);
        assert_eq!(
            panel
                .node()
                .layout
                .as_ref()
                .expect("controlled node layout snapshot")
                .mode,
            DesignLayoutMode::None,
            "the panel must wait for the host's wrapper/frame echo"
        );
        assert_eq!(
            panel
                .inspection_context()
                .selection()
                .items()
                .iter()
                .map(|node| node.id.as_ref())
                .collect::<Vec<_>>(),
            ["second", "first", "third"]
        );
    });
}

#[gpui::test]
fn add_auto_layout_rejects_stale_reordered_and_disabled_projections(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Group);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
                node_ids: vec!["second".into(), "first".into()],
            }),
            cx,
        );
        assert!(panel.add_auto_layout_view_data_for_context().is_none());
        assert!(!panel.emit_add_auto_layout(cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-add-auto-layout").is_none(),
        "ordered target mismatch must keep a late projection inert"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
                node_ids: vec!["first".into(), "second".into()],
            })
            .disabled("One selected layer is locked"),
            cx,
        );
        assert!(
            panel.add_auto_layout_view_data_for_context().is_some(),
            "an eligible disabled host projection stays visible"
        );
        assert!(!panel.emit_add_auto_layout(cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-add-auto-layout").is_some(),
        "the native disabled affordance should explain host unavailability"
    );
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn add_auto_layout_requires_edit_permission_and_omits_active_owners(cx: &mut TestAppContext) {
    let group = DesignPanelNode::new("group", "Group", DesignPanelNodeKind::Group);
    let (host, visual_cx) = setup(group.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    let view_data = DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
        node_ids: vec!["group".into()],
    });

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                group,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_add_auto_layout_view_data(view_data, cx);
        assert!(!panel.emit_add_auto_layout(cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-add-auto-layout").is_none(),
        "view-only inspection renders no document-mutating Layout affordance"
    );

    let owner = DesignPanelNode::new("owner", "Owner", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                owner,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_add_auto_layout_view_data(
            DesignAddAutoLayoutViewData::eligible(DesignPanelTarget::Nodes {
                node_ids: vec!["owner".into()],
            }),
            cx,
        );
        assert!(panel.add_auto_layout_view_data_for_context().is_none());
        assert!(!panel.emit_add_auto_layout(cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx.debug_bounds("design-add-auto-layout").is_none(),
        "an active auto-layout owner must keep its existing direction controls"
    );
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn smart_selection_spacing_emits_phased_exact_target_intents_without_mutating_view_data(
    cx: &mut TestAppContext,
) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["first".into(), "second".into()],
    };
    let view_data = DesignSmartSelectionViewData::horizontal(
        target.clone(),
        super::super::DesignSmartSelectionSpacingViewData::uniform(24.),
    )
    .with_operation(
        DesignSmartSelectionOperation::DistributeHorizontal,
        DesignSmartSelectionAvailability::Available,
    )
    .with_operation(
        DesignSmartSelectionOperation::TidyUp,
        DesignSmartSelectionAvailability::Available,
    );
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_smart_selection_view_data(view_data.clone(), cx);
        assert_eq!(
            panel.current_property_value(DesignPanelProperty::SmartSelectionHorizontalSpacing),
            Some(DesignPanelValue::Number(24.))
        );
        assert!(panel.property_is_editable(DesignPanelProperty::SmartSelectionHorizontalSpacing));
        assert!(!panel.property_is_editable(DesignPanelProperty::SmartSelectionVerticalSpacing));

        for (value, phase) in [
            (24., DesignPanelEditPhase::Begin),
            (36., DesignPanelEditPhase::Preview),
            (24., DesignPanelEditPhase::Cancel),
        ] {
            panel.emit_property_edit(
                DesignPanelProperty::SmartSelectionHorizontalSpacing,
                DesignPanelValue::Number(value),
                phase,
                cx,
            );
        }
        panel.emit_smart_selection_arrange(DesignSmartSelectionOperation::DistributeHorizontal, cx);
        assert_eq!(panel.smart_selection_view_data(), Some(&view_data));
        drop(panel.render_position(cx));
    });
    visual_cx.run_until_parked();
    assert!(
        visual_cx
            .debug_bounds("design-smart-selection-horizontal-spacing")
            .is_some()
    );
    assert!(
        visual_cx
            .debug_bounds("design-smart-selection-vertical-spacing")
            .is_none()
    );
    assert!(
        visual_cx
            .debug_bounds("design-smart-selection-distribute-horizontal")
            .is_some()
    );
    assert!(
        visual_cx
            .debug_bounds("design-smart-selection-tidy-up")
            .is_some()
    );

    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target: target.clone(),
                axis: DesignSmartSelectionAxis::Horizontal,
                value: 24.,
                phase: DesignPanelEditPhase::Begin,
            },
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target: target.clone(),
                axis: DesignSmartSelectionAxis::Horizontal,
                value: 36.,
                phase: DesignPanelEditPhase::Preview,
            },
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target: target.clone(),
                axis: DesignSmartSelectionAxis::Horizontal,
                value: 24.,
                phase: DesignPanelEditPhase::Cancel,
            },
            DesignPanelAction::SmartSelectionArrangeRequested {
                target,
                operation: DesignSmartSelectionOperation::DistributeHorizontal,
            },
        ]
    );
}

#[gpui::test]
fn smart_selection_rejects_reordered_stale_and_read_only_projections(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
    let exact_target = DesignPanelTarget::Nodes {
        node_ids: vec!["first".into(), "second".into()],
    };
    let stale_target = DesignPanelTarget::Nodes {
        node_ids: vec!["second".into(), "first".into()],
    };
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::multiple(
                DesignPanelMultipleSelection::new(first, second),
                DesignPanelParentLayout::Mixed,
                DesignPanelPermissions::editor(),
            ),
            cx,
        );
        panel.set_smart_selection_view_data(
            DesignSmartSelectionViewData::horizontal(
                stale_target,
                super::super::DesignSmartSelectionSpacingViewData::uniform(12.),
            )
            .with_operation(
                DesignSmartSelectionOperation::TidyUp,
                DesignSmartSelectionAvailability::Available,
            ),
            cx,
        );
        assert!(panel.smart_selection_view_data_for_context().is_none());
        panel.emit_property(
            DesignPanelProperty::SmartSelectionHorizontalSpacing,
            DesignPanelValue::Number(18.),
            cx,
        );
        panel.emit_smart_selection_arrange(DesignSmartSelectionOperation::TidyUp, cx);

        let read_only = DesignSmartSelectionViewData::two_dimensional(
            exact_target,
            super::super::DesignSmartSelectionSpacingViewData::uniform(12.),
            super::super::DesignSmartSelectionSpacingViewData::mixed(),
        )
        .with_operation(
            DesignSmartSelectionOperation::TidyUp,
            DesignSmartSelectionAvailability::Available,
        )
        .read_only("Locked layer");
        panel.set_smart_selection_view_data(read_only, cx);
        assert!(
            panel.smart_selection_property_is_mixed(
                DesignPanelProperty::SmartSelectionVerticalSpacing
            )
        );
        assert!(!panel.property_is_editable(DesignPanelProperty::SmartSelectionHorizontalSpacing));
        panel.emit_property(
            DesignPanelProperty::SmartSelectionHorizontalSpacing,
            DesignPanelValue::Number(18.),
            cx,
        );
        panel.emit_smart_selection_arrange(DesignSmartSelectionOperation::TidyUp, cx);
        drop(panel.render_position(cx));
    });
    visual_cx.run_until_parked();
    assert!(captured.borrow().is_empty());
}

#[gpui::test]
fn smart_selection_host_echo_cancels_the_active_exact_target_draft(cx: &mut TestAppContext) {
    let first = DesignPanelNode::new("first", "First", DesignPanelNodeKind::Rectangle);
    let second = DesignPanelNode::new("second", "Second", DesignPanelNodeKind::Ellipse);
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["first".into(), "second".into()],
    };
    let projection = |value| {
        DesignSmartSelectionViewData::horizontal(
            target.clone(),
            super::super::DesignSmartSelectionSpacingViewData::uniform(value),
        )
    };
    let (host, visual_cx) = setup(first.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_inspection_context(
                DesignPanelInspectionContext::multiple(
                    DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                    DesignPanelParentLayout::Mixed,
                    DesignPanelPermissions::editor(),
                ),
                cx,
            );
            panel.set_smart_selection_view_data(projection(24.), cx);
            panel.activate_property(
                DesignPanelProperty::SmartSelectionHorizontalSpacing,
                DesignPanelValue::Number(25.),
                window,
                cx,
            );
            assert!(panel.property_editor.is_some());

            panel.set_smart_selection_view_data(projection(32.), cx);
            assert!(panel.property_editor.is_none());
            assert_eq!(
                panel.current_property_value(DesignPanelProperty::SmartSelectionHorizontalSpacing),
                Some(DesignPanelValue::Number(32.))
            );
        });
    });
    visual_cx.run_until_parked();

    assert_eq!(
        captured.borrow().as_slice(),
        [
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target: target.clone(),
                axis: DesignSmartSelectionAxis::Horizontal,
                value: 24.,
                phase: DesignPanelEditPhase::Begin,
            },
            DesignPanelAction::SmartSelectionSpacingEditRequested {
                target: target.clone(),
                axis: DesignSmartSelectionAxis::Horizontal,
                value: 24.,
                phase: DesignPanelEditPhase::Cancel,
            },
        ]
    );

    for context in [
        DesignPanelInspectionContext::multiple(
            DesignPanelMultipleSelection::new(first.clone(), second.clone()),
            DesignPanelParentLayout::Mixed,
            DesignPanelPermissions::viewer(),
        ),
        DesignPanelInspectionContext::multiple(
            DesignPanelMultipleSelection::new(second.clone(), first.clone()),
            DesignPanelParentLayout::Mixed,
            DesignPanelPermissions::editor(),
        ),
    ] {
        captured.borrow_mut().clear();
        visual_cx.update(|window, app| {
            panel.update(app, |panel, cx| {
                panel.set_inspection_context(
                    DesignPanelInspectionContext::multiple(
                        DesignPanelMultipleSelection::new(first.clone(), second.clone()),
                        DesignPanelParentLayout::Mixed,
                        DesignPanelPermissions::editor(),
                    ),
                    cx,
                );
                panel.set_smart_selection_view_data(projection(32.), cx);
                panel.activate_property(
                    DesignPanelProperty::SmartSelectionHorizontalSpacing,
                    DesignPanelValue::Number(33.),
                    window,
                    cx,
                );
                panel.set_inspection_context(context.clone(), cx);
                assert!(panel.property_editor.is_none());
            });
        });
        visual_cx.run_until_parked();
        assert_eq!(
            captured.borrow().as_slice(),
            [
                DesignPanelAction::SmartSelectionSpacingEditRequested {
                    target: target.clone(),
                    axis: DesignSmartSelectionAxis::Horizontal,
                    value: 32.,
                    phase: DesignPanelEditPhase::Begin,
                },
                DesignPanelAction::SmartSelectionSpacingEditRequested {
                    target: target.clone(),
                    axis: DesignSmartSelectionAxis::Horizontal,
                    value: 32.,
                    phase: DesignPanelEditPhase::Cancel,
                },
            ],
            "permission and ordered-selection changes cancel the captured exact target"
        );
    }
}

#[gpui::test]
fn menu_preview_is_balanced_and_cleans_up_before_commit(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    panel.update(visual_cx, |panel, cx| {
        let candidate = DesignPanelValue::BlendMode(DesignBlendMode::Multiply);
        panel.set_property_menu_preview(
            DesignPanelProperty::BlendMode,
            candidate.clone(),
            true,
            cx,
        );
        panel.set_property_menu_preview(
            DesignPanelProperty::BlendMode,
            candidate.clone(),
            true,
            cx,
        );
        panel.emit_property(DesignPanelProperty::BlendMode, candidate.clone(), cx);
        panel.set_property_menu_preview(DesignPanelProperty::BlendMode, candidate, false, cx);
        panel.cancel_menu_preview(cx);
        assert!(panel.active_menu_preview.is_none());
        assert_eq!(panel.node.blend_mode, DesignBlendMode::Normal);
    });
    visual_cx.run_until_parked();

    let captured = captured.borrow();
    assert_eq!(captured.len(), 3);
    let (
        DesignPanelAction::MenuPreviewRequested {
            preview: begin,
            phase: DesignMenuPreviewPhase::Begin,
        },
        DesignPanelAction::MenuPreviewRequested {
            preview: end,
            phase: DesignMenuPreviewPhase::End,
        },
        DesignPanelAction::PropertyChangeRequested {
            node_id,
            property: DesignPanelProperty::BlendMode,
            value: DesignPanelValue::BlendMode(DesignBlendMode::Multiply),
        },
    ) = (&captured[0], &captured[1], &captured[2])
    else {
        panic!("preview commit should emit Begin, matching End, then commit");
    };
    assert_eq!(begin, end);
    assert!(matches!(
        begin,
        DesignMenuPreview::NodeProperty {
            target: DesignPanelTarget::Nodes { node_ids },
            property: DesignPanelProperty::BlendMode,
            original: DesignPanelValue::BlendMode(DesignBlendMode::Normal),
            candidate: DesignPanelValue::BlendMode(DesignBlendMode::Multiply),
        } if node_ids == &vec![SharedString::from("rectangle")]
    ));
    assert_eq!(node_id.as_ref(), "rectangle");
}

#[gpui::test]
fn menu_preview_stale_echo_and_exact_paint_effect_targets(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    node.fills = vec![DesignPaint::solid(DesignColor::BLUE).with_id("stable-paint")];
    node.effects = vec![DesignEffect::new(DesignEffectKind::DropShadow).with_id("stable-effect")];
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);

    visual_cx.update(|window, app| {
        panel.update(app, |panel, cx| {
            panel.set_property_menu_preview(
                DesignPanelProperty::EffectKind(0),
                DesignPanelValue::EffectKind(DesignEffectKind::InnerShadow),
                true,
                cx,
            );
            let mut echoed = node.clone();
            echoed.opacity = 92.;
            panel.set_node(echoed, cx);
            panel.active_picker = Some(PaintPickerTarget {
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "stable-paint".into(),
            });
            panel.sync_paint_picker(window, cx);
        });
    });
    let picker = visual_cx.read(|app| panel.read(app).paint_picker.clone());
    picker.update(visual_cx, |_, cx| {
        cx.emit(PaintPickerEvent::BlendModePreview {
            target: PickerEventTarget {
                node_id: "rectangle".into(),
                collection: DesignPanelCollection::Fill,
                index: 0,
                paint_id: "stable-paint".into(),
            },
            original: DesignBlendMode::Normal,
            candidate: DesignBlendMode::Multiply,
            phase: DesignMenuPreviewPhase::Begin,
        });
    });
    visual_cx.run_until_parked();

    let observed = captured.borrow();
    assert!(matches!(
        observed.as_slice(),
        [
            DesignPanelAction::MenuPreviewRequested {
                preview: DesignMenuPreview::EffectProperty {
                    effect_id,
                    index: 0,
                    ..
                },
                phase: DesignMenuPreviewPhase::Begin,
            },
            DesignPanelAction::MenuPreviewRequested {
                preview: DesignMenuPreview::EffectProperty {
                    effect_id: end_effect_id,
                    index: 0,
                    ..
                },
                phase: DesignMenuPreviewPhase::End,
            },
            DesignPanelAction::MenuPreviewRequested {
                preview: DesignMenuPreview::PaintProperty {
                    paint_id,
                    index: 0,
                    target: DesignPaintTarget::WholeLayer,
                    ..
                },
                phase: DesignMenuPreviewPhase::Begin,
            },
        ] if effect_id.as_ref() == "stable-effect"
            && end_effect_id == effect_id
            && paint_id.as_ref() == "stable-paint"
    ));

    drop(observed);
    panel.update(visual_cx, |panel, cx| {
        panel.cancel_menu_preview(cx);
        panel.set_property_value_state(
            DesignPanelProperty::BlendMode,
            DesignPanelPropertyValueState::bound(super::super::DesignPanelPropertyBinding::new(
                "blend-variable",
                "Blend",
                DesignPanelBindingKind::Variable,
                DesignPanelValue::BlendMode(DesignBlendMode::Normal),
            )),
            cx,
        );
        panel.set_property_menu_preview(
            DesignPanelProperty::BlendMode,
            DesignPanelValue::BlendMode(DesignBlendMode::Screen),
            true,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured
            .borrow()
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::MenuPreviewRequested {
                    phase: DesignMenuPreviewPhase::Begin,
                    ..
                }
            ))
            .count(),
        2,
        "bound controls must not start another preview"
    );

    panel.update(visual_cx, |panel, cx| {
        panel.set_property_value_states(
            std::iter::empty::<(
                DesignPanelProperty,
                DesignPanelPropertyValueState<DesignPanelValue>,
            )>(),
            cx,
        );
        let viewer_node = panel.node.clone();
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                viewer_node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_property_menu_preview(
            DesignPanelProperty::BlendMode,
            DesignPanelValue::BlendMode(DesignBlendMode::Screen),
            true,
            cx,
        );
    });
    visual_cx.run_until_parked();
    assert_eq!(
        captured
            .borrow()
            .iter()
            .filter(|action| matches!(
                action,
                DesignPanelAction::MenuPreviewRequested {
                    phase: DesignMenuPreviewPhase::Begin,
                    ..
                }
            ))
            .count(),
        2,
        "viewer controls must not start another preview"
    );
}

#[gpui::test]
fn every_editable_tab_stop_binds_shared_activation_or_owns_its_editing(cx: &mut TestAppContext) {
    let frame = DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame)
        .with_layout_mode(DesignLayoutMode::Horizontal);
    let (host, visual_cx) = setup(frame, cx);
    let panel = panel(&host, visual_cx);
    visual_cx.update(|window, app| {
        panel.read(app).focus_handle.clone().focus(window, app);
        let mut visited = Vec::new();
        for _ in 0..512 {
            window.focus_next(app);
            let focused = window
                .focused(app)
                .expect("the rendered Design panel should expose at least one tab stop");
            let identity = format!("{focused:?}");
            if visited.contains(&identity) {
                break;
            }
            visited.push(identity.clone());
            let contexts = window.context_stack();
            let has_keyboard_path = contexts.iter().any(|context| {
                context.contains(crate::atoms::CONTROL_KEY_CONTEXT)
                    || context.contains("Input")
                    || context.contains("Select")
            });
            assert!(
                has_keyboard_path,
                "tab stop {} ({identity}) must bind the shared Enter/Space activation \
                 command or own its editing; context stack: {contexts:?}",
                visited.len(),
            );
        }
        assert!(
            visited.len() > 8,
            "the editable fixture should expose a meaningful tab order, saw {}",
            visited.len(),
        );
    });
}

#[gpui::test]
fn section_add_button_emits_one_add_from_pointer_enter_and_space(cx: &mut TestAppContext) {
    let node = DesignPanelNode::new("rectangle", "Rectangle", DesignPanelNodeKind::Rectangle);
    let (host, visual_cx) = setup(node, cx);
    let captured = actions(&host, visual_cx);
    crate::test_support::assert_pointer_and_keyboard_parity(
        visual_cx,
        "design-add-fill",
        &captured,
        DesignPanelAction::CollectionItemAddRequested {
            node_id: "rectangle".into(),
            collection: DesignPanelCollection::Fill,
            target: DesignPaintTarget::WholeLayer,
        },
    );
}

#[gpui::test]
fn viewer_representation_button_activates_from_the_keyboard(cx: &mut TestAppContext) {
    let mut node = DesignPanelNode::new("text", "Hero title", DesignPanelNodeKind::Text);
    node.stroke = Some(DesignStroke::for_node(
        DesignPanelNodeKind::Text,
        DesignPaint::solid(DesignColor::BLACK),
        2.,
        DesignStrokeAlign::Inside,
    ));
    let target = DesignPanelTarget::Nodes {
        node_ids: vec!["text".into()],
    };
    let view_data = DesignViewerPropertiesViewData::new(
        target.clone(),
        [super::super::DesignViewerPropertySection::new(
            "borders",
            "Borders",
            [
                super::super::DesignViewerPropertyRow::new("weight", "Weight", "2px")
                    .with_property(DesignPanelProperty::StrokeWeight),
            ],
        )
        .with_copy_value(SharedString::from("border: 2px solid rgb(30, 30, 30);"))
        .with_color_representation(DesignViewerColorRepresentation::Css)],
    );
    let (host, visual_cx) = setup(node.clone(), cx);
    let panel = panel(&host, visual_cx);
    let captured = actions(&host, visual_cx);
    panel.update(visual_cx, |panel, cx| {
        panel.set_inspection_context(
            DesignPanelInspectionContext::single(
                node,
                DesignPanelParentLayout::Freeform,
                DesignPanelPermissions::viewer(),
            ),
            cx,
        );
        panel.set_viewer_properties_view_data(view_data, cx);
    });
    visual_cx.run_until_parked();

    visual_cx.update(|window, app| {
        panel.read(app).focus_handle.clone().focus(window, app);
    });
    let mut found = false;
    for _ in 0..64 {
        visual_cx.update(|window, app| {
            window.focus_next(app);
        });
        captured.borrow_mut().clear();
        visual_cx.simulate_keystrokes("enter");
        visual_cx.run_until_parked();
        let emitted = captured.borrow().clone();
        if emitted.iter().any(|action| {
            matches!(
                action,
                DesignPanelAction::ViewerSectionRepresentationChangeRequested { .. }
            )
        }) {
            assert_eq!(
                emitted.len(),
                1,
                "Enter on a representation Button must activate exactly once: {emitted:?}"
            );
            assert!(matches!(
                &emitted[0],
                DesignPanelAction::ViewerSectionRepresentationChangeRequested {
                    target: emitted_target,
                    section_id,
                    ..
                } if emitted_target == &target && section_id.as_ref() == "borders"
            ));
            found = true;
            break;
        }
    }
    assert!(
        found,
        "a viewer representation Button must be reachable and Enter-activatable"
    );
    captured.borrow_mut().clear();
}

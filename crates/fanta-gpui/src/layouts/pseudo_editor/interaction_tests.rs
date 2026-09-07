use gpui::{AppContext as _, Modifiers, TestAppContext, px, size};

use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;
use crate::{
    assets::{AssetsPanel, AssetsViewData},
    design::{DesignPanel, DesignPanelNode, DesignPanelNodeKind},
    layers::LayersPanel,
    pages::PagesPanel,
    prototype::{PrototypePanel, PrototypeViewData},
    timeline::{Timeline, TimelineViewData},
    toolbar::{EditorToolbar, ToolbarMode, ToolbarTool},
    variables::{VariablesPage, VariablesViewData},
};

fn mount(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, PseudoEditor, PseudoEditorAction> {
    let (host, actions, cx) = mount_component(cx, |window, cx| {
        let pages = cx.new(|cx| PagesPanel::new("pages", Vec::new(), window, cx));
        let layers = cx.new(|cx| LayersPanel::new("layers", Vec::new(), window, cx));
        let assets = cx.new(|cx| AssetsPanel::new("assets", AssetsViewData::default(), window, cx));
        let design = cx.new(|cx| {
            DesignPanel::new(
                "design",
                DesignPanelNode::new("frame", "Frame", DesignPanelNodeKind::Frame),
                window,
                cx,
            )
        });
        let prototype =
            cx.new(|cx| PrototypePanel::new("prototype", PrototypeViewData::default(), cx));
        let timeline = cx.new(|cx| Timeline::new("timeline", TimelineViewData::default(), cx));
        let toolbar = cx.new(|cx| {
            EditorToolbar::new(
                "toolbar",
                ToolbarMode::Design,
                ToolbarTool::Move,
                100,
                window,
                cx,
            )
        });
        let variables = cx.new(|cx| {
            VariablesPage::new(
                "variables",
                VariablesViewData {
                    document_name: "Test".into(),
                    collections: Vec::new(),
                    selected_collection_id: "".into(),
                    groups: Vec::new(),
                    selected_group_id: "".into(),
                    modes: Vec::new(),
                    variables: Vec::new(),
                },
                window,
                cx,
            )
        });
        PseudoEditor::new(
            "test-editor",
            PseudoEditorChildren {
                pages,
                layers,
                assets,
                design,
                prototype,
                timeline,
                toolbar,
                variables,
            },
            cx,
        )
    });
    cx.simulate_resize(size(px(1440.), px(900.)));
    (host, actions, cx)
}

#[gpui::test]
fn left_surface_tabs_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-left-pages",
        &actions,
        PseudoEditorAction::LeftSurfaceChanged {
            surface: PseudoEditorLeftSurface::Pages,
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-left-layers",
        &actions,
        PseudoEditorAction::LeftSurfaceChanged {
            surface: PseudoEditorLeftSurface::Layers,
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-left-assets",
        &actions,
        PseudoEditorAction::LeftSurfaceChanged {
            surface: PseudoEditorLeftSurface::Assets,
        },
    );
}

#[gpui::test]
fn right_surface_tabs_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-right-design",
        &actions,
        PseudoEditorAction::RightSurfaceChanged {
            surface: PseudoEditorRightSurface::Design,
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-right-prototype",
        &actions,
        PseudoEditorAction::RightSurfaceChanged {
            surface: PseudoEditorRightSurface::Prototype,
        },
    );
}

#[gpui::test]
fn present_and_share_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-present",
        &actions,
        PseudoEditorAction::PresentRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-share",
        &actions,
        PseudoEditorAction::ShareRequested,
    );
}

#[gpui::test]
fn rails_hold_their_design_widths_on_comfortable_windows(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);

    let left = cx
        .debug_bounds("pseudo-editor-left-rail")
        .expect("the left rail should render");
    let right = cx
        .debug_bounds("pseudo-editor-right-rail")
        .expect("the right rail should render");
    assert_eq!(f32::from(left.size.width), PSEUDO_EDITOR_LEFT_RAIL_WIDTH);
    assert_eq!(f32::from(right.size.width), PSEUDO_EDITOR_RIGHT_RAIL_WIDTH);

    let strip = cx
        .debug_bounds("pseudo-editor-timeline-strip")
        .expect("the timeline strip should render");
    assert_eq!(f32::from(strip.size.height), PSEUDO_EDITOR_TIMELINE_HEIGHT);
}

#[gpui::test]
fn canvas_keeps_positive_size_at_the_shell_minimum_window(cx: &mut TestAppContext) {
    let (_host, _actions, cx) = mount(cx);

    cx.simulate_resize(size(
        px(PSEUDO_EDITOR_MIN_WIDTH),
        px(PSEUDO_EDITOR_MIN_HEIGHT),
    ));
    cx.run_until_parked();

    let canvas = cx
        .debug_bounds("pseudo-editor-canvas")
        .expect("the canvas should render at the shell minimum");
    assert!(
        f32::from(canvas.size.width) >= PSEUDO_EDITOR_MIN_CANVAS_WIDTH,
        "the canvas must keep a positive width at {PSEUDO_EDITOR_MIN_WIDTH} px, got {:?}",
        canvas.size.width
    );
    assert!(
        f32::from(canvas.size.height) >= PSEUDO_EDITOR_MIN_CANVAS_HEIGHT,
        "the canvas must keep a positive height at {PSEUDO_EDITOR_MIN_HEIGHT} px, got {:?}",
        canvas.size.height
    );

    // Both rails compress to their floors instead of overflowing the window.
    let left = cx
        .debug_bounds("pseudo-editor-left-rail")
        .expect("the left rail should render at the shell minimum");
    let right = cx
        .debug_bounds("pseudo-editor-right-rail")
        .expect("the right rail should render at the shell minimum");
    assert_eq!(
        f32::from(left.size.width),
        PSEUDO_EDITOR_LEFT_RAIL_MIN_WIDTH
    );
    assert_eq!(
        f32::from(right.size.width),
        PSEUDO_EDITOR_RIGHT_RAIL_MIN_WIDTH
    );
    assert!(
        f32::from(right.right()) <= PSEUDO_EDITOR_MIN_WIDTH + 0.5,
        "the right rail must stay inside the window, got {:?}",
        right.right()
    );

    // The timeline strip yields to its floor rather than squeezing the canvas.
    let strip = cx
        .debug_bounds("pseudo-editor-timeline-strip")
        .expect("the timeline strip should render at the shell minimum");
    assert_eq!(
        f32::from(strip.size.height),
        PSEUDO_EDITOR_TIMELINE_MIN_HEIGHT
    );
    assert!(
        f32::from(strip.bottom()) <= PSEUDO_EDITOR_MIN_HEIGHT + 0.5,
        "the timeline strip must stay inside the window, got {:?}",
        strip.bottom()
    );
}

#[gpui::test]
fn variables_overlay_opens_and_closes_from_the_shell_controls(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert!(
        cx.debug_bounds("pseudo-editor-close-variables").is_none(),
        "the variables overlay should start hidden"
    );

    // Opening emits a constant intent, so the parity matrix stays valid even
    // though the overlay mounts above the control.
    assert_pointer_and_keyboard_parity(
        cx,
        "pseudo-editor-variables",
        &actions,
        PseudoEditorAction::VariablesVisibilityChanged { visible: true },
    );

    // Closing unmounts the control, so assert the single pointer path directly
    // instead of using the parity matrix.
    let close = cx
        .debug_bounds("pseudo-editor-close-variables")
        .expect("the variables overlay should expose its close control");
    cx.simulate_click(close.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PseudoEditorAction::VariablesVisibilityChanged { visible: false }]
    );
    assert!(
        cx.debug_bounds("pseudo-editor-close-variables").is_none(),
        "the variables overlay should hide after closing"
    );
}

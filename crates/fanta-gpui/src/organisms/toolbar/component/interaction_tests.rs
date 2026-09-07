use std::{cell::RefCell, rc::Rc};

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, KeyBinding, Modifiers,
    MouseButton, MouseDownEvent, ParentElement as _, Render, Subscription, TestAppContext,
    VisualTestContext, Window, actions, div, point, px, size,
};
use gpui_component::{IconName, Root};

use super::*;
use crate::toolbar::{ToolbarChromeControl, ToolbarControlValue};

actions!(toolbar_input_regression, [CompetingDeleteSelection]);

struct TestHost {
    toolbar: Entity<EditorToolbar>,
    actions: Rc<RefCell<Vec<ToolbarAction>>>,
    competing_delete_count: usize,
    /// Mouse-down presses that reached the host canvas underneath the dock.
    canvas_presses: usize,
    _subscription: Subscription,
}

impl TestHost {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let toolbar = cx.new(|cx| {
            EditorToolbar::new(
                "test-toolbar",
                ToolbarMode::Design,
                ToolbarTool::Move,
                100,
                window,
                cx,
            )
        });
        let actions = Rc::new(RefCell::new(Vec::new()));
        let captured_actions = actions.clone();
        let subscription = cx.subscribe(&toolbar, move |_, _, action: &ToolbarAction, _| {
            captured_actions.borrow_mut().push(action.clone());
        });
        Self {
            toolbar,
            actions,
            competing_delete_count: 0,
            canvas_presses: 0,
            _subscription: subscription,
        }
    }
}

impl Render for TestHost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("test-canvas")
            .debug_selector(|| "test-canvas".to_owned())
            .relative()
            .size_full()
            .key_context("TestCanvas")
            .on_action(cx.listener(|this, _: &CompetingDeleteSelection, _, cx| {
                this.competing_delete_count += 1;
                cx.stop_propagation();
            }))
            // The canvas probe: a real host canvas listens for presses the
            // same way, so anything the dock fails to occlude lands here.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.canvas_presses += 1;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(18.))
                    .flex()
                    .px_4()
                    .gap(px(8.))
                    .justify_center()
                    .child(self.toolbar.clone())
                    .child(
                        div()
                            .debug_selector(|| "test-host-trailing-controls".to_owned())
                            .w(px(72.))
                            .h(px(40.))
                            .flex_none(),
                    ),
            )
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
        .expect("toolbar test host should be installed");
    (host, cx)
}

#[gpui::test]
fn mode_controls_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let actions = cx.read(|app| host.read(app).actions.clone());
    let bounds = cx
        .debug_bounds("toolbar-mode-motion")
        .expect("Motion mode control should render");

    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ModeChangeRequested {
            mode: ToolbarMode::Motion,
        }]
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter space");
    assert_eq!(
        actions.borrow().as_slice(),
        &[
            ToolbarAction::ModeChangeRequested {
                mode: ToolbarMode::Motion,
            },
            ToolbarAction::ModeChangeRequested {
                mode: ToolbarMode::Motion,
            },
        ]
    );
    assert!(
        cx.debug_bounds("toolbar-mode-draw").is_none(),
        "the mode tray is Design / Motion / Dev only"
    );
}

#[gpui::test]
fn actions_query_accepts_backward_and_forward_deletion(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
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
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "ab"
    );

    cx.simulate_keystrokes("left delete");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a"
    );

    cx.simulate_keystrokes("b c shift-left shift-left backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a"
    );

    cx.simulate_keystrokes("secondary-a delete");
    assert!(cx.read(|app| toolbar.read(app).command_input.read(app).value().is_empty()));

    cx.simulate_keystrokes("a b c space d e f");
    #[cfg(target_os = "macos")]
    cx.simulate_keystrokes("alt-backspace");
    #[cfg(not(target_os = "macos"))]
    cx.simulate_keystrokes("ctrl-backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "abc "
    );

    #[cfg(target_os = "macos")]
    {
        cx.simulate_keystrokes("x y z cmd-backspace");
        assert!(cx.read(|app| toolbar.read(app).command_input.read(app).value().is_empty()));
    }

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_agent(window, cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("h e l l o backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).ai_input.read(app).value()),
        "hell"
    );

    assert_eq!(
        cx.read(|app| host.read(app).competing_delete_count),
        0,
        "focused toolbar input must suppress a competing canvas delete action"
    );
}

#[gpui::test]
fn intact_input_bindings_do_not_double_dispatch_the_fallback(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("a b backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a",
        "the fallback must defer to an intact Input binding"
    );
}

#[gpui::test]
fn higher_precedence_global_delete_binding_cannot_steal_intact_input_deletion(
    cx: &mut TestAppContext,
) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        app.bind_keys([KeyBinding::new("backspace", CompetingDeleteSelection, None)]);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("a b backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a"
    );
    assert_eq!(cx.read(|app| host.read(app).competing_delete_count), 0);
}

#[gpui::test]
fn intentional_input_delete_remap_keeps_precedence_over_the_fallback(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
        app.bind_keys([KeyBinding::new(
            "backspace",
            CompetingDeleteSelection,
            Some("Input"),
        )]);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("a b backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "ab"
    );
    assert_eq!(cx.read(|app| host.read(app).competing_delete_count), 1);
}

#[gpui::test]
fn outer_chord_starting_with_backspace_cannot_delay_input_deletion(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
        app.clear_key_bindings();
        app.bind_keys([KeyBinding::new(
            "backspace x",
            CompetingDeleteSelection,
            Some("TestCanvas"),
        )]);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("a b backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a"
    );
    cx.update(|window, _| {
        assert!(
            !window.has_pending_keystrokes(),
            "the outer chord must not retain Backspace as a pending prefix"
        );
    });
    assert_eq!(cx.read(|app| host.read(app).competing_delete_count), 0);
}

#[gpui::test]
fn fallback_deletion_discards_an_unmatched_pending_outer_chord(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
        app.clear_key_bindings();
        app.bind_keys([KeyBinding::new(
            "ctrl-k x",
            CompetingDeleteSelection,
            Some("TestCanvas"),
        )]);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("a b ctrl-k");
    cx.update(|window, _| {
        assert!(
            window.has_pending_keystrokes(),
            "Ctrl+K should start the chord"
        );
    });

    cx.simulate_keystrokes("backspace");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "a"
    );
    cx.update(|window, _| {
        assert!(
            !window.has_pending_keystrokes(),
            "handled deletion must discard the stale chord prefix"
        );
    });

    cx.simulate_keystrokes("c");
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "ac",
        "discarding the chord must preserve text-input focus"
    );
    assert_eq!(cx.read(|app| host.read(app).competing_delete_count), 0);
}

#[gpui::test]
fn dock_contains_every_persistent_toolbar_surface(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.simulate_resize(size(px(900.), px(700.)));

    for mode in ToolbarMode::ALL {
        cx.update(|_, app| {
            toolbar.update(app, |toolbar, cx| toolbar.set_mode(*mode, cx));
        });
        cx.run_until_parked();

        let root = cx
            .debug_bounds("editor-toolbar")
            .expect("toolbar root should render");
        let surface = cx
            .debug_bounds("editor-toolbar-surface")
            .expect("toolbar dock should render");

        assert_eq!(
            surface, root,
            "root should adopt the dock's intrinsic bounds"
        );
        for selector in [
            "toolbar-primary-row",
            "toolbar-utility-row",
            "toolbar-zoom-control",
            "toolbar-agent-launcher",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} should render"));
            assert!(
                bounds.is_contained_within(&surface),
                "{mode:?} {selector} {bounds:?} escaped dock {surface:?}"
            );
        }
        assert!(
            cx.debug_bounds("toolbar-zoom-out").is_some()
                && cx.debug_bounds("toolbar-zoom-in").is_some(),
            "a wide {mode:?} dock should keep the full zoom cluster"
        );

        assert!(
            root.size.height < px(170.),
            "the intrinsic {mode:?} dock should not expand into a canvas overlay: {root:?}"
        );
        assert!(root.size.width < px(900.));
    }

    // 380 px leaves the dock 268 px (a 258 px utility row): the full zoom
    // row (273 px) no longer fits but the percent-only row (223 px) does.
    cx.simulate_resize(size(px(380.), px(700.)));
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_mode(ToolbarMode::Motion, cx));
    });
    cx.run_until_parked();
    let root = cx
        .debug_bounds("editor-toolbar")
        .expect("narrow toolbar root should render");
    let primary_viewport = cx
        .debug_bounds("toolbar-primary-viewport")
        .expect("narrow primary viewport should render");
    assert!(
        root.size.width <= px(268.),
        "narrow dock escaped its host wrapper: {root:?}"
    );
    assert!(primary_viewport.is_contained_within(&root));
    let utility_viewport = cx
        .debug_bounds("toolbar-utility-viewport")
        .expect("narrow utility viewport should render");
    let utility_row = cx
        .debug_bounds("toolbar-utility-row")
        .expect("narrow utility row should render");
    assert!(utility_viewport.is_contained_within(&root));
    let utility_scroll = cx.read(|app| toolbar.read(app).utility_scroll_handle.clone());
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_none()
            && cx.debug_bounds("toolbar-zoom-in").is_none(),
        "a narrow dock should shed the zoom steppers before scrolling its utility row"
    );
    assert!(
        cx.debug_bounds("toolbar-zoom-menu-trigger").is_some(),
        "the collapsed zoom cluster should keep its percent menu trigger"
    );
    for selector in ["toolbar-zoom-control", "toolbar-agent-launcher"] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should render at a narrow width"));
        assert!(
            bounds.is_contained_within(&utility_row),
            "narrow {selector} {bounds:?} escaped its scrollable row {utility_row:?}"
        );
        // A control clipped by the dock edge must stay reachable by
        // scrolling its own row.
        let clipped = bounds.right() - utility_viewport.right();
        assert!(
            utility_scroll.max_offset().x >= clipped,
            "narrow {selector} {bounds:?} is unreachable in viewport {utility_viewport:?}"
        );
    }

    cx.simulate_resize(size(px(220.), px(700.)));
    cx.run_until_parked();
    let root = cx
        .debug_bounds("editor-toolbar")
        .expect("very narrow toolbar should render");
    let utility_viewport = cx
        .debug_bounds("toolbar-utility-viewport")
        .expect("utility overflow viewport should render");
    assert!(utility_viewport.is_contained_within(&root));
    assert!(
        cx.debug_bounds("toolbar-zoom-control").is_none(),
        "a very narrow dock should collapse the zoom cluster entirely"
    );
    assert_eq!(
        cx.debug_bounds("toolbar-mode-motion")
            .expect("mode target should remain measurable")
            .size,
        size(px(28.), px(28.))
    );
    assert_eq!(
        cx.debug_bounds("toolbar-agent-launcher")
            .expect("Agent target should remain measurable")
            .size,
        size(px(36.), px(36.))
    );
}

#[gpui::test]
fn collapsed_zoom_cluster_keeps_its_menu_and_fits_without_scrolling(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());
    cx.simulate_resize(size(px(380.), px(700.)));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_none()
            && cx.debug_bounds("toolbar-zoom-in").is_none(),
        "a 258 px utility row should ride the percent-only tier"
    );

    // In the percent-only tier the whole utility row reflows into its
    // viewport: collapse, not scrolling, absorbs the lost width.
    let viewport = cx
        .debug_bounds("toolbar-utility-viewport")
        .expect("utility viewport should render");
    for selector in [
        "toolbar-zoom-control",
        "toolbar-agent-launcher",
        "toolbar-mode-design",
    ] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should render"));
        assert!(
            bounds.is_contained_within(&viewport),
            "{selector} {bounds:?} should fit the collapsed row viewport {viewport:?}"
        );
    }
    let utility_scroll = cx.read(|app| toolbar.read(app).utility_scroll_handle.clone());
    assert_eq!(
        f32::from(utility_scroll.max_offset().x),
        0.,
        "the collapsed utility row must not need to scroll"
    );

    // The percent trigger keeps the full zoom menu.
    let trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("percent trigger should render");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-flyout").is_some(),
        "the percent trigger should still open the zoom menu"
    );
    actions.borrow_mut().clear();
    let fit = cx
        .debug_bounds("toolbar-zoom-entry-zoom-to-fit")
        .expect("Zoom to fit should render");
    cx.simulate_click(fit.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToFit,
        }]
    );

    // Re-widening the host restores the steppers.
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_some()
            && cx.debug_bounds("toolbar-zoom-in").is_some(),
        "a re-widened dock should restore the zoom steppers"
    );
}

#[gpui::test]
fn actions_palette_is_anchored_eight_pixels_above_its_trigger(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    let trigger = cx
        .debug_bounds("toolbar-tool-actions")
        .expect("Actions trigger should render");
    let palette = cx
        .debug_bounds("toolbar-actions-palette")
        .expect("Actions palette should render");

    assert_eq!(palette.bottom() + px(POPOVER_GAP), trigger.top());
    assert_eq!(palette.right(), trigger.right());
}

#[gpui::test]
fn opening_a_popup_does_not_reflow_a_contextual_dock(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_mode(ToolbarMode::Motion, cx));
    });
    cx.run_until_parked();

    let dock_before = cx
        .debug_bounds("editor-toolbar-surface")
        .expect("Motion dock should render");
    let trigger_before = cx
        .debug_bounds("toolbar-tool-actions")
        .expect("Actions trigger should render");

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    assert_eq!(cx.debug_bounds("editor-toolbar-surface"), Some(dock_before));
    assert_eq!(
        cx.debug_bounds("toolbar-tool-actions"),
        Some(trigger_before)
    );
}

#[gpui::test]
fn tool_menu_is_anchored_to_its_own_caret(cx: &mut TestAppContext) {
    let (_, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();

    let trigger = cx
        .debug_bounds("toolbar-group-move-tools-trigger")
        .expect("Move tools caret should render");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();

    let menu = cx
        .debug_bounds("toolbar-tool-flyout")
        .expect("Move tools flyout should render");
    assert_eq!(menu.bottom() + px(POPOVER_GAP), trigger.top());
    assert!(menu.left() <= trigger.left());
    assert!(menu.right() >= trigger.right());
}

#[gpui::test]
fn disclosure_triggers_close_their_own_open_popups(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    cx.run_until_parked();
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    for (trigger, popup) in [
        ("toolbar-group-move-tools-trigger", "toolbar-tool-flyout"),
        ("toolbar-zoom-menu-trigger", "toolbar-zoom-flyout"),
        ("toolbar-agent-launcher", "toolbar-agent-composer"),
        ("toolbar-tool-actions", "toolbar-actions-palette"),
    ] {
        let trigger_bounds = cx
            .debug_bounds(trigger)
            .unwrap_or_else(|| panic!("{trigger} should render"));
        cx.simulate_click(trigger_bounds.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds(popup).is_some(), "{popup} should open");

        let trigger_bounds = cx
            .debug_bounds(trigger)
            .unwrap_or_else(|| panic!("{trigger} should remain in place"));
        cx.simulate_click(trigger_bounds.center(), Modifiers::none());
        cx.run_until_parked();
        assert!(
            cx.debug_bounds(popup).is_none(),
            "{trigger} should close its own {popup}"
        );
        if matches!(popup, "toolbar-agent-composer" | "toolbar-actions-palette") {
            assert!(
                cx.update(|window, app| toolbar.focus_handle(app).is_focused(window)),
                "{trigger} should restore toolbar focus after removing its text input"
            );
        }
    }
}

#[gpui::test]
fn narrow_popups_remain_inside_the_window_and_actions_stays_visible(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(420.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    let root = cx
        .debug_bounds("editor-toolbar")
        .expect("toolbar should render");
    let trigger = cx
        .debug_bounds("toolbar-tool-actions")
        .expect("pinned Actions trigger should render");
    let palette = cx
        .debug_bounds("toolbar-actions-palette")
        .expect("responsive Actions palette should render");
    assert!(trigger.is_contained_within(&root));
    assert!(palette.left() >= px(8.));
    assert!(palette.right() <= px(412.));
    assert_eq!(palette.bottom() + px(POPOVER_GAP), trigger.top());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_agent(window, cx));
    });
    cx.run_until_parked();
    let composer = cx
        .debug_bounds("toolbar-agent-composer")
        .expect("responsive Agent composer should render");
    assert!(composer.left() >= px(8.));
    assert!(composer.right() <= px(412.));

    cx.simulate_resize(size(px(220.), px(700.)));
    cx.run_until_parked();
    let composer = cx
        .debug_bounds("toolbar-agent-composer")
        .expect("very narrow Agent composer should render");
    for selector in [
        "toolbar-agent-suggestions-viewport",
        "toolbar-agent-context-label",
        "toolbar-agent-mention-hint",
    ] {
        let content = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should render"));
        assert!(
            content.is_contained_within(&composer),
            "{selector} {content:?} escaped Agent composer {composer:?}"
        );
    }
}

#[gpui::test]
fn tall_popups_scroll_and_remain_inside_a_short_window(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(240.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();
    let palette = cx
        .debug_bounds("toolbar-actions-palette")
        .expect("Actions palette should render in a short window");
    assert!(palette.top() >= px(8.));
    assert!(palette.bottom() <= px(232.));
    assert_eq!(
        cx.debug_bounds("toolbar-command-generate-a-design")
            .expect("command row should render")
            .size
            .height,
        px(42.),
        "scrolling must preserve command-row hit targets"
    );

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.toggle_tool_group(ToolbarToolGroup::Shape, false, cx);
        });
    });
    cx.run_until_parked();
    let tool_menu = cx
        .debug_bounds("toolbar-tool-flyout")
        .expect("large tool menu should render in a short window");
    assert!(tool_menu.top() >= px(8.));
    assert!(tool_menu.bottom() <= px(232.));
    assert_eq!(
        cx.debug_bounds("toolbar-flyout-rectangle")
            .expect("tool row should render")
            .size
            .height,
        px(38.),
        "scrolling must preserve tool-row hit targets"
    );

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.toggle_zoom(false, cx));
    });
    cx.run_until_parked();
    let zoom_menu = cx
        .debug_bounds("toolbar-zoom-flyout")
        .expect("zoom menu should render in a short window");
    assert!(
        zoom_menu.top() >= px(8.),
        "zoom menu escaped above the safe margin: {zoom_menu:?}"
    );
    assert!(
        zoom_menu.bottom() <= px(232.),
        "zoom menu escaped below the safe margin: {zoom_menu:?}"
    );
    assert_eq!(
        cx.debug_bounds("toolbar-zoom-entry-zoom-in")
            .expect("zoom row should render")
            .size
            .height,
        px(30.),
        "scrolling must preserve zoom-row hit targets"
    );
}

#[gpui::test]
fn choosing_an_actions_scope_returns_focus_to_the_query(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("t e s t");
    let scope = cx
        .debug_bounds("toolbar-actions-scope-assets")
        .expect("Assets scope should render");
    cx.simulate_click(scope.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_scope),
        CommandScope::Assets
    );
    cx.simulate_keystrokes("backspace");

    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "tes"
    );
}

#[gpui::test]
fn choosing_tool_and_zoom_menu_items_restores_toolbar_focus(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());

    let group_trigger = cx
        .debug_bounds("toolbar-group-move-tools-trigger")
        .expect("Move tools trigger should render");
    cx.simulate_click(group_trigger.center(), Modifiers::none());
    cx.run_until_parked();
    let hand = cx
        .debug_bounds("toolbar-flyout-hand")
        .expect("Hand menu item should render");
    cx.simulate_click(hand.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("toolbar-tool-flyout").is_none());
    assert!(cx.update(|window, app| toolbar.focus_handle(app).is_focused(window)));

    let zoom_trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("Zoom trigger should render");
    cx.simulate_click(zoom_trigger.center(), Modifiers::none());
    cx.run_until_parked();
    let zoom = cx
        .debug_bounds("toolbar-zoom-entry-zoom-to-100")
        .expect("100 percent zoom entry should render");
    cx.simulate_click(zoom.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("toolbar-zoom-flyout").is_none());
    assert!(cx.update(|window, app| toolbar.focus_handle(app).is_focused(window)));
}

#[gpui::test]
fn zoom_menu_entries_emit_commands_and_typed_zoom_intents(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let actions = cx.read(|app| host.read(app).actions.clone());

    let trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("Zoom trigger should render");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    for selector in [
        "toolbar-zoom-entry-zoom-in",
        "toolbar-zoom-entry-zoom-out",
        "toolbar-zoom-entry-zoom-to-fit",
        "toolbar-zoom-entry-zoom-to-selection",
        "toolbar-zoom-entry-zoom-to-100",
        "toolbar-zoom-entry-zoom-to-50",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} should render in the zoom menu"
        );
    }

    actions.borrow_mut().clear();
    let fit = cx
        .debug_bounds("toolbar-zoom-entry-zoom-to-fit")
        .expect("Zoom to fit should render");
    cx.simulate_click(fit.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToFit,
        }]
    );
    assert!(cx.debug_bounds("toolbar-zoom-flyout").is_none());

    let trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("Zoom trigger should remain in place");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    actions.borrow_mut().clear();
    let half = cx
        .debug_bounds("toolbar-zoom-entry-zoom-to-50")
        .expect("Zoom to 50% should render");
    cx.simulate_click(half.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 50 }]
    );
}

#[gpui::test]
fn zoom_menu_arrows_home_end_highlight_and_enter_commits(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let actions = cx.read(|app| host.read(app).actions.clone());

    let trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("Zoom trigger should render");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToFit,
        }],
        "Down/Down/Enter should commit the third zoom entry"
    );
    assert!(cx.debug_bounds("toolbar-zoom-flyout").is_none());

    let trigger = cx
        .debug_bounds("toolbar-zoom-menu-trigger")
        .expect("Zoom trigger should remain in place");
    cx.simulate_click(trigger.center(), Modifiers::none());
    cx.run_until_parked();
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("end enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 50 }],
        "End/Enter should commit the last zoom entry"
    );
}

#[gpui::test]
fn shift_zoom_shortcuts_emit_figma_zoom_bindings(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let actions = cx.read(|app| host.read(app).actions.clone());
    cx.update(|window, app| {
        let focus_handle = host.read(app).toolbar.read(app).focus_handle.clone();
        focus_handle.focus(window, app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("shift-1");
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToFit,
        }]
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-2");
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToSelection,
        }]
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-0");
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 100 }]
    );
}

#[gpui::test]
fn zoom_steppers_step_the_multiplicative_ladder(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());

    let zoom_in = cx
        .debug_bounds("toolbar-zoom-in")
        .expect("zoom-in stepper should render");
    cx.simulate_click(zoom_in.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 200 }],
        "zoom in from 100 should reach the next ladder step"
    );

    actions.borrow_mut().clear();
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_zoom_percent(110, cx));
    });
    cx.run_until_parked();
    let zoom_out = cx
        .debug_bounds("toolbar-zoom-out")
        .expect("zoom-out stepper should render");
    cx.simulate_click(zoom_out.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 100 }],
        "zoom out from an off-ladder value should snap to the previous step"
    );

    actions.borrow_mut().clear();
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_zoom_percent(3_200, cx));
    });
    cx.run_until_parked();
    let zoom_in = cx
        .debug_bounds("toolbar-zoom-in")
        .expect("zoom-in stepper should remain in place");
    cx.simulate_click(zoom_in.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ZoomChangeRequested { percent: 3_200 }],
        "the ladder should clamp at 3,200 percent"
    );
}

#[gpui::test]
fn resources_tool_activates_via_pointer_and_shift_i(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let actions = cx.read(|app| host.read(app).actions.clone());

    let resources = cx
        .debug_bounds("toolbar-tool-resources")
        .expect("Resources tool should render in the Design layout");
    cx.simulate_click(resources.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ToolChangeRequested {
            mode: ToolbarMode::Design,
            tool: ToolbarTool::Resources,
        }]
    );

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("shift-i");
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ToolChangeRequested {
            mode: ToolbarMode::Design,
            tool: ToolbarTool::Resources,
        }]
    );
}

#[gpui::test]
fn motion_style_chip_offers_host_candidates(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_mode(ToolbarMode::Motion, cx);
            toolbar.set_motion_options(
                MotionToolbarOptions {
                    animation_style: "Spring".into(),
                    available_animation_styles: vec!["Fade in".into(), "Spring".into()],
                    ..MotionToolbarOptions::default()
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    let chip = cx
        .debug_bounds("toolbar-secondary-motion-style")
        .expect("animation style chip should render");
    cx.simulate_click(chip.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-motion-style-editor").is_some(),
        "the style chip should open a candidate menu"
    );

    actions.borrow_mut().clear();
    let candidate = cx
        .debug_bounds("toolbar-motion-style-option-fade-in")
        .expect("host-supplied style should render");
    cx.simulate_click(candidate.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ControlChangeRequested {
            mode: ToolbarMode::Motion,
            control: ToolbarSecondaryControl::MotionAnimationStyle,
            value: ToolbarControlValue::Choice("Fade in".into()),
        }]
    );
    assert!(
        cx.debug_bounds("toolbar-motion-style-editor").is_none(),
        "choosing a candidate should close the editor"
    );
}

#[gpui::test]
fn motion_style_editor_arrows_move_the_highlight_and_enter_commits(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_mode(ToolbarMode::Motion, cx));
    });
    cx.run_until_parked();

    let chip = cx
        .debug_bounds("toolbar-secondary-motion-style")
        .expect("animation style chip should render");
    cx.simulate_click(chip.center(), Modifiers::none());
    cx.run_until_parked();

    // The editor opens highlighted on the accepted value ("Fade in", index
    // 0); Right moves to "Spring" and Enter commits it.
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("right enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ControlChangeRequested {
            mode: ToolbarMode::Motion,
            control: ToolbarSecondaryControl::MotionAnimationStyle,
            value: ToolbarControlValue::Choice("Spring".into()),
        }],
        "Right should move the highlight and Enter should commit it"
    );
    assert!(cx.debug_bounds("toolbar-motion-style-editor").is_none());
}

#[gpui::test]
fn flyout_arrow_navigation_highlights_and_commits(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let actions = cx.read(|app| host.read(app).actions.clone());

    let caret = cx
        .debug_bounds("toolbar-group-move-tools-trigger")
        .expect("Move tools caret should render");
    cx.simulate_click(caret.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("toolbar-tool-flyout").is_some());

    actions.borrow_mut().clear();
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ToolChangeRequested {
            mode: ToolbarMode::Design,
            tool: ToolbarTool::Scale,
        }],
        "arrow navigation should highlight Scale and Enter should commit it"
    );
    assert!(cx.debug_bounds("toolbar-tool-flyout").is_none());
}

#[gpui::test]
fn fuzzy_actions_query_ranks_the_best_match_first(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());

    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();

    // "zf" is not a substring of any label; the word-boundary subsequence
    // ranks Zoom to fit first and Enter runs the highlighted best match.
    cx.simulate_keystrokes("z f");
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-command-zoom-to-fit").is_some(),
        "a scattered query should still surface its subsequence match"
    );
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::CommandInvoked {
            command: ToolbarCommand::ZoomToFit,
        }]
    );
    // Reopening must not have accumulated a stray newline from the Enter
    // keystroke's key_char falling through the propagated action.
    cx.update(|window, app| {
        toolbar.update(app, |toolbar, cx| toolbar.open_actions(window, cx));
    });
    cx.run_until_parked();
    assert_eq!(
        cx.read(|app| toolbar.read(app).command_input.read(app).value()),
        "zf"
    );
}

#[gpui::test]
fn narrow_primary_row_shows_edge_fade_affordances(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| toolbar.set_mode(ToolbarMode::Motion, cx));
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-primary-fade-end").is_none(),
        "a wide dock should not show an overflow fade"
    );

    cx.simulate_resize(size(px(420.), px(700.)));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-primary-fade-end").is_some(),
        "clipped trailing tools should surface an end fade"
    );
    assert!(
        cx.debug_bounds("toolbar-primary-fade-start").is_none(),
        "an unscrolled row should not fade its start edge"
    );

    cx.update(|_, app| {
        let handle = toolbar.read(app).primary_scroll_handle.clone();
        let max = handle.max_offset().x;
        handle.set_offset(point(-max, px(0.)));
        toolbar.update(app, |_, cx| cx.notify());
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-primary-fade-start").is_some(),
        "scrolling to the end should surface a start fade"
    );
    assert!(
        cx.debug_bounds("toolbar-primary-fade-end").is_none(),
        "the end fade should clear once the row is fully scrolled"
    );
}

fn example_chrome_controls(left_active: bool) -> Vec<ToolbarChromeControl> {
    vec![
        ToolbarChromeControl::new("fit", IconName::Maximize, "Fit to view").shortcut("⇧ 1"),
        ToolbarChromeControl::new(
            "left-sidebar",
            if left_active {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            },
            "Toggle layers sidebar",
        )
        .active(left_active),
        ToolbarChromeControl::new(
            "right-sidebar",
            IconName::PanelRightOpen,
            "Toggle inspector sidebar",
        ),
    ]
}

#[gpui::test]
fn dock_occludes_the_host_canvas_beneath_it(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_chrome_controls(example_chrome_controls(false), cx);
        });
    });
    cx.run_until_parked();
    let presses = |cx: &mut VisualTestContext| cx.read(|app| host.read(app).canvas_presses);

    // A tool press activates the tool and stops at the dock.
    let text = cx
        .debug_bounds("toolbar-tool-text")
        .expect("Text tool should render");
    cx.simulate_click(text.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ToolChangeRequested {
            mode: ToolbarMode::Design,
            tool: ToolbarTool::Text,
        }]
    );
    assert_eq!(presses(cx), 0, "a tool press must not reach the canvas");

    // Dock padding (no control under the pointer) still stops the press.
    let surface = cx
        .debug_bounds("editor-toolbar-surface")
        .expect("dock surface should render");
    cx.simulate_click(surface.origin + point(px(3.), px(3.)), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(presses(cx), 0, "dock padding must not reach the canvas");

    // The mode tray gap and the host chrome capsule are dock too.
    let tray = cx
        .debug_bounds("toolbar-mode-design")
        .expect("Design mode tile should render");
    cx.simulate_click(tray.origin + point(px(-3.), px(14.)), Modifiers::none());
    let fit = cx
        .debug_bounds("toolbar-chrome-fit")
        .expect("fit chrome control should render");
    cx.simulate_click(fit.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(presses(cx), 0, "chrome controls must not reach the canvas");
    assert_eq!(
        actions.borrow().last(),
        Some(&ToolbarAction::ChromeControlInvoked { id: "fit".into() })
    );

    // Outside the dock the canvas is live.
    let canvas = cx
        .debug_bounds("test-canvas")
        .expect("host canvas should render");
    cx.simulate_click(canvas.origin + point(px(20.), px(20.)), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        presses(cx),
        1,
        "presses beside the dock must reach the canvas"
    );
    assert!(
        !surface.contains(&(canvas.origin + point(px(20.), px(20.)))),
        "the probe point must lie outside the dock"
    );
}

#[gpui::test]
fn chrome_controls_render_in_the_dock_and_emit_typed_intents(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.simulate_resize(size(px(900.), px(700.)));
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    let actions = cx.read(|app| host.read(app).actions.clone());

    assert!(
        cx.debug_bounds("toolbar-chrome-cluster").is_none(),
        "no capsule renders until the host supplies chrome controls"
    );

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_chrome_controls(example_chrome_controls(true), cx);
        });
    });
    cx.run_until_parked();

    let surface = cx
        .debug_bounds("editor-toolbar-surface")
        .expect("dock surface should render");
    let row = cx
        .debug_bounds("toolbar-utility-row")
        .expect("utility row should render");
    let cluster = cx
        .debug_bounds("toolbar-chrome-cluster")
        .expect("chrome capsule should render");
    let agent = cx
        .debug_bounds("toolbar-agent-launcher")
        .expect("Agent launcher should render");
    assert!(cluster.is_contained_within(&surface));
    assert!(cluster.is_contained_within(&row));
    assert!(
        cluster.left() > agent.right(),
        "the capsule trails the Agent launcher: {cluster:?} vs {agent:?}"
    );
    for selector in [
        "toolbar-chrome-fit",
        "toolbar-chrome-left-sidebar",
        "toolbar-chrome-right-sidebar",
    ] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should render"));
        assert!(bounds.is_contained_within(&cluster));
        assert_eq!(bounds.size, size(px(28.), px(28.)));
    }
    assert!(
        cx.debug_bounds("toolbar-chrome-left-sidebar-active")
            .is_some()
            && cx.debug_bounds("toolbar-chrome-fit-active").is_none()
            && cx
                .debug_bounds("toolbar-chrome-right-sidebar-active")
                .is_none(),
        "only the active control should carry the active treatment"
    );

    // Pointer activation, then Enter/Space parity on the focused control.
    let left = cx
        .debug_bounds("toolbar-chrome-left-sidebar")
        .expect("left sidebar control should render");
    cx.simulate_click(left.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[ToolbarAction::ChromeControlInvoked {
            id: "left-sidebar".into(),
        }]
    );
    actions.borrow_mut().clear();
    cx.simulate_keystrokes("enter space");
    assert_eq!(
        actions.borrow().as_slice(),
        &[
            ToolbarAction::ChromeControlInvoked {
                id: "left-sidebar".into(),
            },
            ToolbarAction::ChromeControlInvoked {
                id: "left-sidebar".into(),
            },
        ]
    );

    // The host echoes the toggled state; the toolbar never flips it itself.
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            assert!(toolbar.chrome_controls()[1].active);
            toolbar.set_chrome_controls(example_chrome_controls(false), cx);
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-chrome-left-sidebar-active")
            .is_none()
    );
    assert!(
        cx.debug_bounds("toolbar-chrome-left-sidebar-inactive")
            .is_some()
    );

    // Duplicate ids collapse to the first occurrence.
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_chrome_controls(
                [
                    ToolbarChromeControl::new("fit", IconName::Maximize, "Fit"),
                    ToolbarChromeControl::new("fit", IconName::Minimize, "Fit again"),
                ],
                cx,
            );
            assert_eq!(toolbar.chrome_controls().len(), 1);
            assert_eq!(toolbar.chrome_controls()[0].label, "Fit");
        });
    });
    cx.run_until_parked();

    // Clearing removes the capsule.
    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_chrome_controls(std::iter::empty(), cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("toolbar-chrome-cluster").is_none());
}

#[gpui::test]
fn chrome_capsule_makes_the_zoom_cluster_reflow_earlier(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    cx.simulate_resize(size(px(420.), px(700.)));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_some()
            && cx.debug_bounds("toolbar-zoom-in").is_some(),
        "without chrome the 298 px utility row keeps the full zoom cluster"
    );

    cx.update(|_, app| {
        toolbar.update(app, |toolbar, cx| {
            toolbar.set_chrome_controls(example_chrome_controls(false), cx);
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-control").is_none(),
        "the chrome capsule claims the width the zoom cluster would need"
    );
    let viewport = cx
        .debug_bounds("toolbar-utility-viewport")
        .expect("utility viewport should render");
    for selector in [
        "toolbar-chrome-cluster",
        "toolbar-agent-launcher",
        "toolbar-mode-design",
    ] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} should render"));
        assert!(
            bounds.is_contained_within(&viewport),
            "{selector} {bounds:?} should fit the collapsed row viewport {viewport:?}"
        );
    }
    let utility_scroll = cx.read(|app| toolbar.read(app).utility_scroll_handle.clone());
    assert_eq!(
        f32::from(utility_scroll.max_offset().x),
        0.,
        "the collapsed utility row must not need to scroll"
    );
}

#[gpui::test]
fn zoom_tiers_switch_exactly_where_the_row_stops_fitting(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let toolbar = cx.read(|app| host.read(app).toolbar.clone());
    // The test host wraps the dock in 16 px side padding, an 8 px gap, and a
    // 72 px trailing block; the dock's 1 px border and 4 px padding on each
    // side leave a utility row 122 px narrower than the window.
    let host_chrome = 122.;
    let scroll = cx.read(|app| toolbar.read(app).utility_scroll_handle.clone());
    let overflow = |cx: &mut VisualTestContext| {
        cx.run_until_parked();
        f32::from(scroll.max_offset().x)
    };

    cx.simulate_resize(size(
        px(TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH + host_chrome),
        px(700.),
    ));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_some(),
        "at the steppers floor the full row still fits"
    );
    assert_eq!(overflow(cx), 0., "the full row must fit without scrolling");

    cx.simulate_resize(size(
        px(TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH + host_chrome - 1.),
        px(700.),
    ));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-out").is_none()
            && cx.debug_bounds("toolbar-zoom-menu-trigger").is_some(),
        "one pixel below the steppers floor the steppers shed"
    );
    assert_eq!(overflow(cx), 0., "the percent-only row must fit");

    cx.simulate_resize(size(
        px(TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH + host_chrome),
        px(700.),
    ));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-menu-trigger").is_some(),
        "at the cluster floor the percent-only row still fits"
    );
    assert_eq!(overflow(cx), 0.);

    cx.simulate_resize(size(
        px(TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH + host_chrome - 1.),
        px(700.),
    ));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("toolbar-zoom-control").is_none(),
        "one pixel below the cluster floor the zoom cluster hides"
    );
    assert_eq!(
        overflow(cx),
        0.,
        "the mode tray and Agent launcher must fit"
    );
}

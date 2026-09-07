use gpui::{Modifiers, TestAppContext, size};

use crate::test_support::{assert_pointer_and_keyboard_parity, mount_component};

use super::*;

fn mount(
    cx: &mut TestAppContext,
) -> crate::test_support::Mounted<'_, PrototypePanel, PrototypePanelAction> {
    mount_component(cx, |_, cx| {
        PrototypePanel::new("test-prototype", PrototypeViewData::default(), cx)
    })
}

#[gpui::test]
fn surface_tabs_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-surface-design",
        &actions,
        PrototypePanelAction::SurfaceChangeRequested {
            surface: PrototypePanelSurface::Design,
        },
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-surface-prototype",
        &actions,
        PrototypePanelAction::SurfaceChangeRequested {
            surface: PrototypePanelSurface::Prototype,
        },
    );
}

#[gpui::test]
fn header_and_settings_controls_share_pointer_enter_and_space_activation(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-zoom",
        &actions,
        PrototypePanelAction::ZoomMenuRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-device",
        &actions,
        PrototypePanelAction::DeviceMenuRequested,
    );
    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-background",
        &actions,
        PrototypePanelAction::BackgroundEditRequested,
    );
}

#[gpui::test]
fn panel_stays_laid_out_and_operable_at_its_floor_size(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    // Host device names have no length contract; the settings row must
    // truncate instead of widening past the floored panel.
    let panel = cx.read(|app| host.read(app).component.clone());
    panel.update(cx, |panel, cx| {
        panel.set_view_data(
            PrototypeViewData {
                device_name: "An intentionally very long device name that cannot fit the row"
                    .into(),
                ..PrototypeViewData::default()
            },
            cx,
        );
    });
    cx.simulate_resize(size(
        px(PROTOTYPE_PANEL_MIN_WIDTH),
        px(PROTOTYPE_PANEL_MIN_HEIGHT),
    ));
    cx.run_until_parked();

    for selector in [
        "prototype-surface-design",
        "prototype-surface-prototype",
        "prototype-zoom",
        "prototype-device",
        "prototype-background",
    ] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` should render at the floor size"));
        assert!(
            f32::from(bounds.right()) <= PROTOTYPE_PANEL_MIN_WIDTH + 0.5,
            "`{selector}` must stay inside the {PROTOTYPE_PANEL_MIN_WIDTH}px floor, got {:?}",
            bounds.right()
        );
    }

    // Activation still flows at the floor width.
    assert_pointer_and_keyboard_parity(
        cx,
        "prototype-device",
        &actions,
        PrototypePanelAction::DeviceMenuRequested,
    );
}

#[gpui::test]
fn hint_dismissals_emit_once_and_hide_only_their_own_hint(cx: &mut TestAppContext) {
    let (_host, actions, cx) = mount(cx);

    // Dismissal mutates component-local state, so assert each pointer path
    // directly instead of using the parity matrix.
    let connection = cx
        .debug_bounds("prototype-dismiss-connection")
        .expect("connection hint dismiss control should render");
    cx.simulate_click(connection.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PrototypePanelAction::HintDismissed {
            hint: PrototypeHint::CreatingConnection,
        }]
    );
    assert!(
        cx.debug_bounds("prototype-dismiss-connection").is_none(),
        "connection hint should hide after dismissal"
    );

    actions.borrow_mut().clear();
    let running = cx
        .debug_bounds("prototype-dismiss-running")
        .expect("running hint should survive the connection hint dismissal");
    cx.simulate_click(running.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        &[PrototypePanelAction::HintDismissed {
            hint: PrototypeHint::RunningPrototype,
        }]
    );
    assert!(
        cx.debug_bounds("prototype-dismiss-running").is_none(),
        "running hint should hide after dismissal"
    );
}

#[gpui::test]
fn restore_hints_re_renders_dismissed_hints(cx: &mut TestAppContext) {
    let (host, actions, cx) = mount(cx);

    for selector in ["prototype-dismiss-connection", "prototype-dismiss-running"] {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("control `{selector}` should render"));
        cx.simulate_click(bounds.center(), Modifiers::none());
        cx.run_until_parked();
    }
    assert_eq!(
        actions.borrow().as_slice(),
        &[
            PrototypePanelAction::HintDismissed {
                hint: PrototypeHint::CreatingConnection,
            },
            PrototypePanelAction::HintDismissed {
                hint: PrototypeHint::RunningPrototype,
            },
        ]
    );

    let panel = cx.read(|app| host.read(app).component.clone());
    panel.update(cx, |panel, cx| panel.restore_hints(cx));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("prototype-dismiss-connection").is_some(),
        "connection hint should re-render after restore_hints"
    );
    assert!(
        cx.debug_bounds("prototype-dismiss-running").is_some(),
        "running hint should re-render after restore_hints"
    );
}

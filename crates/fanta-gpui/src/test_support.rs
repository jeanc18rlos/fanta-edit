//! Shared GPUI interaction-test scaffolding.
//!
//! Every panel used to hand-roll the same mount/subscribe/capture harness and
//! re-prove pointer/keyboard activation parity per control. This module owns
//! both once: [`mount_component`] mounts any intent-emitting component behind a
//! probe host that records its typed actions, and
//! [`assert_pointer_and_keyboard_parity`] asserts the §9 contract — pointer
//! click, Enter, and Space each emit the same intent exactly once — against a
//! control's debug selector.

use std::{cell::RefCell, fmt::Debug, rc::Rc};

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, Modifiers, ParentElement as _,
    Render, Styled as _, Subscription, TestAppContext, VisualTestContext, Window, div,
};
use gpui_component::Root;

/// Test host that mounts one component and records every intent it emits.
pub(crate) struct ProbeHost<C: Render, A: 'static> {
    pub(crate) component: Entity<C>,
    pub(crate) actions: Rc<RefCell<Vec<A>>>,
    _subscription: Subscription,
}

impl<C: Render, A: 'static> Render for ProbeHost<C, A> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.component.clone())
    }
}

/// A mounted probe: the host entity, the captured action log, and the window
/// test context.
pub(crate) type Mounted<'a, C, A> = (
    Entity<ProbeHost<C, A>>,
    Rc<RefCell<Vec<A>>>,
    &'a mut VisualTestContext,
);

/// Initializes the app, mounts `build`'s component in a rooted window, and
/// returns the probe host plus the captured action log.
pub(crate) fn mount_component<C, A>(
    cx: &mut TestAppContext,
    build: impl FnOnce(&mut Window, &mut Context<C>) -> C + 'static,
) -> Mounted<'_, C, A>
where
    C: Render + EventEmitter<A>,
    A: Clone + 'static,
{
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::init(cx);
    });
    let host_slot = Rc::new(RefCell::new(None));
    let captured_host = host_slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let host = cx.new(|cx| {
            let component = cx.new(|cx| build(window, cx));
            let actions: Rc<RefCell<Vec<A>>> = Rc::new(RefCell::new(Vec::new()));
            let captured_actions = actions.clone();
            let subscription = cx.subscribe(&component, move |_, _, action: &A, _| {
                captured_actions.borrow_mut().push(action.clone());
            });
            ProbeHost {
                component,
                actions,
                _subscription: subscription,
            }
        });
        *captured_host.borrow_mut() = Some(host.clone());
        Root::new(host, window, cx)
    });
    let host = host_slot
        .borrow_mut()
        .take()
        .expect("probe host should be installed");
    let actions = cx.read(|app| host.read(app).actions.clone());
    (host, actions, cx)
}

/// Asserts the §9 activation contract for one control: a pointer click, Enter,
/// and Space each emit exactly `expected` once.
///
/// The click focuses the control, so the subsequent keystrokes target it. Only
/// valid for controls whose emission does not depend on component-local state
/// mutated by activation; those controls assert their sequences directly.
pub(crate) fn assert_pointer_and_keyboard_parity<A>(
    cx: &mut VisualTestContext,
    selector: &'static str,
    actions: &Rc<RefCell<Vec<A>>>,
    expected: A,
) where
    A: Clone + PartialEq + Debug,
{
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("control `{selector}` should render"));

    actions.borrow_mut().clear();
    cx.simulate_click(bounds.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(
        actions.borrow().as_slice(),
        std::slice::from_ref(&expected),
        "pointer activation for `{selector}`"
    );

    for keystroke in ["enter", "space"] {
        actions.borrow_mut().clear();
        cx.simulate_keystrokes(keystroke);
        cx.run_until_parked();
        assert_eq!(
            actions.borrow().as_slice(),
            std::slice::from_ref(&expected),
            "{keystroke} activation for `{selector}`"
        );
    }
    actions.borrow_mut().clear();
}

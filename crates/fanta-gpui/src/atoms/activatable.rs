//! Unified pointer/keyboard activation for custom Fanta controls.
//!
//! ARCHITECTURE.md §9 requires pointer activation and Enter/Space activation
//! to converge on the same component methods. Historically every control
//! hand-wired an `on_action(ActivateControl)` / `on_click` pair with a copied
//! closure body; [`ControlExt::on_activate`] owns that convergence so a control
//! registers one handler and keyboard parity holds by construction.

use std::rc::Rc;

use gpui::{App, ClickEvent, InteractiveElement as _, StatefulInteractiveElement, Window};
use gpui_component::button::Button;

use super::{ActivateControl, CONTROL_KEY_CONTEXT};

/// Activation payload shared by the pointer and keyboard paths.
///
/// Handlers that need to branch (for example to skip pointer-only affordances
/// such as drag initiation) can inspect [`ActivateEvent::keyboard`]; most
/// handlers ignore it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivateEvent {
    /// True when the activation came from the Enter/Space command path.
    pub keyboard: bool,
}

/// Builder extensions shared by activatable Fanta controls.
///
/// §16 contract: the atom owns pointer/keyboard activation convergence; the
/// caller supplies exactly one handler and never wires `on_click`/`on_action`
/// pairs by hand.
pub trait ControlExt: StatefulInteractiveElement + Sized {
    /// Registers one activation handler for both pointer clicks and the
    /// Enter/Space [`ActivateControl`] command.
    ///
    /// Keyboard-synthesized click events are suppressed so a keystroke
    /// activates exactly once regardless of how the platform synthesizes
    /// clicks for focused elements.
    fn on_activate(
        self,
        handler: impl Fn(&ActivateEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        let handler = Rc::new(handler);
        let keyboard_handler = handler.clone();
        self.on_action(move |_: &ActivateControl, window, cx| {
            keyboard_handler(&ActivateEvent { keyboard: true }, window, cx);
        })
        .on_click(move |event: &ClickEvent, window, cx| {
            if event.is_keyboard() {
                return;
            }
            handler(&ActivateEvent { keyboard: false }, window, cx);
        })
    }
}

impl<E: StatefulInteractiveElement> ControlExt for E {}

/// [`ControlExt::on_activate`] for `gpui_component::Button`.
///
/// The fork's `Button` owns its focus handle and click slot instead of being a
/// `Stateful<Div>`, so it cannot take the blanket impl. This mirror registers
/// the same convergence: the shared `FantaControl` key context, one handler for
/// the Enter/Space [`ActivateControl`] command, and the pointer click path with
/// keyboard-synthesized clicks suppressed so a keystroke activates exactly
/// once.
pub trait ButtonControlExt: Sized {
    /// Registers one activation handler for both pointer clicks and the
    /// Enter/Space [`ActivateControl`] command.
    fn on_activate(
        self,
        handler: impl Fn(&ActivateEvent, &mut Window, &mut App) + 'static,
    ) -> Button;

    /// Keyboard-only activation for a Button whose pointer path is owned by a
    /// wrapping surface — for example a `Popover` trigger, which toggles on
    /// the wrapper's mouse-down. Binds the shared key context and the
    /// Enter/Space [`ActivateControl`] command without registering a click
    /// handler, so pointer activation is not doubled (§9 contract: the
    /// action/pointer split is deliberate at such call sites).
    fn on_keyboard_activate(self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Button;
}

impl ButtonControlExt for Button {
    fn on_activate(
        self,
        handler: impl Fn(&ActivateEvent, &mut Window, &mut App) + 'static,
    ) -> Button {
        let handler = Rc::new(handler);
        let keyboard_handler = handler.clone();
        self.key_context(CONTROL_KEY_CONTEXT)
            .on_action(move |_: &ActivateControl, window, cx| {
                keyboard_handler(&ActivateEvent { keyboard: true }, window, cx);
            })
            .on_click(move |event: &ClickEvent, window, cx| {
                if event.is_keyboard() {
                    return;
                }
                handler(&ActivateEvent { keyboard: false }, window, cx);
            })
    }

    fn on_keyboard_activate(self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Button {
        self.key_context(CONTROL_KEY_CONTEXT)
            .on_action(move |_: &ActivateControl, window, cx| {
                handler(window, cx);
            })
    }
}

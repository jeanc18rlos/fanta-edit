//! Atoms: the crate's smallest shared interaction and presentation units.
//!
//! The single Enter/Space activation command, the activatable-control builder
//! extensions, the compact button atom, the code-native vector icon set, and
//! the bounds/truncation utilities. Molecules (`crate::molecules`) and the
//! feature organisms compose these instead of re-implementing key contexts,
//! focus rings, and listener pairs per control (ARCHITECTURE.md §9, §16).
//!
//! This module is the curated public atoms API: hosts build custom chrome
//! from these pieces so their controls share the library's activation,
//! focus-ring, icon, and truncation contracts. Drawing internals (the
//! stroke-path builder behind [`ControlIcon`]) stay crate-private.

use gpui::{App, KeyBinding, actions};

mod activatable;
mod bounds;
mod button;
mod truncate;
pub(crate) mod vector_icon;

pub use activatable::{ActivateEvent, ButtonControlExt, ControlExt};
pub use bounds::track_bounds;
pub use button::icon_button;
pub use truncate::truncating_label;
pub use vector_icon::{ControlIcon, render_control_icon};

actions!(fanta_controls, [ActivateControl]);

/// Key context used by focusable custom controls that behave like buttons.
///
/// §16 contract: [`crate::init`] binds Enter/Space to [`ActivateControl`] in
/// this context; the atoms builders set it automatically, so callers only
/// reach for it when hand-building a focusable control from raw elements.
pub const CONTROL_KEY_CONTEXT: &str = "FantaControl";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", ActivateControl, Some(CONTROL_KEY_CONTEXT)),
        KeyBinding::new("space", ActivateControl, Some(CONTROL_KEY_CONTEXT)),
    ]);
}

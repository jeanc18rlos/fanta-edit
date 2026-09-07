use gpui::{App, KeyBinding, actions};

actions!(fanta_design, [CancelDesignInteraction]);

pub(crate) const DESIGN_PANEL_KEY_CONTEXT: &str = "FantaDesignPanel";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "escape",
        CancelDesignInteraction,
        Some(DESIGN_PANEL_KEY_CONTEXT),
    )]);
}

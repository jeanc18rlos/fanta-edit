use gpui::{App, KeyBinding, actions};

actions!(
    fanta_layers,
    [
        OpenLayerContextMenu,
        CloseLayersOverlay,
        ConfirmLayersTextEntry,
        FocusPreviousLayer,
        FocusNextLayer,
        CollapseLayer,
        ExpandLayer,
        ToggleLayerVisibility,
        ToggleLayerLock
    ]
);

pub(crate) const LAYERS_PANEL_KEY_CONTEXT: &str = "FantaLayersPanel";
pub(crate) const LAYERS_TEXT_ENTRY_KEY_CONTEXT: &str = "FantaLayersPanelTextEntry";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "ctrl-enter",
            OpenLayerContextMenu,
            Some(LAYERS_PANEL_KEY_CONTEXT),
        ),
        KeyBinding::new("escape", CloseLayersOverlay, Some(LAYERS_PANEL_KEY_CONTEXT)),
        // The focused single-line Input handles Enter by emitting PressEnter
        // and then propagating the action; this binding consumes the
        // propagated keystroke so the unhandled fall-through cannot type the
        // keystroke's "\n" key_char into the single-line field.
        KeyBinding::new(
            "enter",
            ConfirmLayersTextEntry,
            Some(LAYERS_TEXT_ENTRY_KEY_CONTEXT),
        ),
        KeyBinding::new("up", FocusPreviousLayer, Some(LAYERS_PANEL_KEY_CONTEXT)),
        KeyBinding::new("down", FocusNextLayer, Some(LAYERS_PANEL_KEY_CONTEXT)),
        KeyBinding::new("left", CollapseLayer, Some(LAYERS_PANEL_KEY_CONTEXT)),
        KeyBinding::new("right", ExpandLayer, Some(LAYERS_PANEL_KEY_CONTEXT)),
        KeyBinding::new(
            "shift-secondary-h",
            ToggleLayerVisibility,
            Some(LAYERS_PANEL_KEY_CONTEXT),
        ),
        KeyBinding::new(
            "shift-secondary-l",
            ToggleLayerLock,
            Some(LAYERS_PANEL_KEY_CONTEXT),
        ),
    ]);
}

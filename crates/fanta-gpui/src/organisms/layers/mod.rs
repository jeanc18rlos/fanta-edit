//! Figma-like document layer navigation components.

mod commands;
mod model;
mod panel;

pub use commands::{
    CloseLayersOverlay, CollapseLayer, ConfirmLayersTextEntry, ExpandLayer, FocusNextLayer,
    FocusPreviousLayer, OpenLayerContextMenu, ToggleLayerLock, ToggleLayerVisibility,
};
pub use model::{
    LayersPanelAction, LayersPanelContextAction, LayersPanelDropPosition, LayersPanelItem,
    LayersPanelNodeKind, LayersPanelSelectionMode,
};
pub use panel::{
    LAYERS_PANEL_MIN_HEIGHT, LAYERS_PANEL_MIN_WIDTH, LayersDropValidator, LayersPanel,
};

pub(crate) use commands::{LAYERS_PANEL_KEY_CONTEXT, LAYERS_TEXT_ENTRY_KEY_CONTEXT};

pub(crate) fn init(cx: &mut gpui::App) {
    commands::init(cx);
}

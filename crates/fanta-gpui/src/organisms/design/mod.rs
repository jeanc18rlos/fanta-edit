//! Host-controlled Figma-like Design inspector.

mod commands;
mod context;
mod field_value;
mod format;
mod model;
mod paint_picker;
mod panel;
mod sections;
mod typography_style_picker;

pub use context::*;
pub use model::*;
pub use panel::DesignPanel;
pub use sections::{
    DesignPanelSectionBand, design_panel_section_band,
    design_panel_section_is_visible_in_workspace, resolve_design_panel_sections,
    resolve_design_panel_sections_with_export,
};

pub(crate) use commands::{CancelDesignInteraction, DESIGN_PANEL_KEY_CONTEXT};
pub(crate) fn init(cx: &mut gpui::App) {
    commands::init(cx);
}

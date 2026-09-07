//! Headless, host-controlled editor toolbar with Figma-like tool flyouts,
//! mode switching, Actions search, and contextual Agent prompting.

mod commands;
mod component;
mod icons;
mod model;

pub use commands::{
    CloseToolbarOverlay, ConfirmToolbarTextEntry, DecrementToolbarControl, EnterDevMode,
    FirstToolbarCommand, IncrementToolbarControl, LastToolbarCommand, NextToolbarCommand,
    OpenToolbarActions, OpenToolbarAgent, PreviousToolbarCommand, SelectAnnotationTool,
    SelectArrowTool, SelectCommentTool, SelectEllipseTool, SelectFrameTool, SelectHandTool,
    SelectImageVideoTool, SelectLineTool, SelectMeasureTool, SelectMoveTool, SelectPenTool,
    SelectPencilTool, SelectRectangleTool, SelectResourcesTool, SelectScaleTool, SelectSectionTool,
    SelectSliceTool, SelectTextTool, ZoomCanvasTo100, ZoomCanvasToFit, ZoomCanvasToSelection,
};
pub use component::{
    EditorToolbar, TOOLBAR_ZOOM_CLUSTER_MIN_WIDTH, TOOLBAR_ZOOM_STEPPERS_MIN_WIDTH,
};
pub use model::{
    AgentToolbarOptions, DevToolbarOptions, MotionToolbarOptions, ToolbarAction,
    ToolbarChromeControl, ToolbarCommand, ToolbarControlValue, ToolbarItem, ToolbarMode,
    ToolbarSecondaryControl, ToolbarTool, ToolbarToolGroup,
};

pub(crate) fn init(cx: &mut gpui::App) {
    commands::init(cx);
}

pub(crate) use commands::TOOLBAR_TEXT_ENTRY_KEY_CONTEXT;
pub(crate) use icons::render_tool_icon;

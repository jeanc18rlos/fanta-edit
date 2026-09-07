//! Direct path-selection tool — STUB. Placeholder so the tool appears in the
//! toolbar's navigation group. NOT YET IMPLEMENTED: a direct-selection cursor
//! that selects individual vector anchors/subpaths (Figma/Illustrator's white
//! arrow), distinct from the existing "Edit Path" (node-editing) tool. Tracked
//! as a follow-up.

use crate::context::ToolContext;
use crate::event::ToolEvent;
use crate::tool::{Tool, ToolResponse};

/// Placeholder direct path-selection tool. Currently a no-op.
#[derive(Debug, Default)]
pub struct PathSelectTool;

impl PathSelectTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for PathSelectTool {
    fn name(&self) -> &'static str {
        "path_select"
    }

    fn handle_event(&mut self, _ctx: &mut ToolContext, _event: ToolEvent) -> ToolResponse {
        // STUB — direct anchor/subpath selection is not implemented yet.
        ToolResponse::empty()
    }
}

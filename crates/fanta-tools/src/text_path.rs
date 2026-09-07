//! Text-on-path tool — STUB. Placeholder so the tool appears in the toolbar's
//! text group. NOT YET IMPLEMENTED: typing text that flows along a vector path
//! (Figma "Type on a path"). Needs text-engine work (glyph placement along an
//! arc-length parameterization), so it is intentionally deferred. Tracked as a
//! follow-up.

use crate::context::ToolContext;
use crate::event::ToolEvent;
use crate::tool::{Tool, ToolResponse};

/// Placeholder text-on-path tool. Currently a no-op.
#[derive(Debug, Default)]
pub struct TextPathTool;

impl TextPathTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for TextPathTool {
    fn name(&self) -> &'static str {
        "text_path"
    }

    fn handle_event(&mut self, _ctx: &mut ToolContext, _event: ToolEvent) -> ToolResponse {
        // STUB — text-on-path layout is not implemented yet.
        ToolResponse::empty()
    }
}

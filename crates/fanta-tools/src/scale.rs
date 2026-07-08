//! Scale tool — STUB. Placeholder so the tool appears in the toolbar's
//! navigation group. NOT YET IMPLEMENTED: Figma's Scale (K) resizes the
//! selection while scaling strokes, text, effects, and corner radii
//! proportionally (unlike the plain resize handles). Tracked as a follow-up.

use crate::context::ToolContext;
use crate::event::ToolEvent;
use crate::tool::{Tool, ToolResponse};

/// Placeholder scale tool. Currently a no-op; selecting it does nothing yet.
#[derive(Debug, Default)]
pub struct ScaleTool;

impl ScaleTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for ScaleTool {
    fn name(&self) -> &'static str {
        "scale"
    }

    fn handle_event(&mut self, _ctx: &mut ToolContext, _event: ToolEvent) -> ToolResponse {
        // STUB — proportional scale interaction is not implemented yet.
        ToolResponse::empty()
    }
}

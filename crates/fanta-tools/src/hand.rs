//! The hand (pan) tool — drags the viewport, never mutates the doc.
//!
//! ## Behavior
//!
//! - **Press** records the screen anchor and switches the cursor to "grabbing."
//! - **Move** while a press is active applies `fanta_canvas::pan` with the
//!   delta from the last position; updates the anchor so future moves are
//!   delta-from-current, not delta-from-press.
//! - **Release** drops the anchor; the cursor returns to "grab."
//! - **Escape** mid-pan does nothing — there is no draft state to abort, and
//!   the press-release pair will naturally clean up.
//!
//! Mirrors the hand tool from Figma / Sketch / tldraw: while held, the world
//! moves *with* the cursor. The math lives in `fanta_canvas::viewport::pan`;
//! this file is just the state machine around it.

use crate::context::ToolContext;
use crate::event::{Button, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolResponse};
use fanta_canvas::pan;
use glam::DVec2;

/// State machine for the hand tool. One instance is reused across many
/// gestures — fields reset between presses, not between moves.
#[derive(Debug, Default)]
pub struct HandTool {
    /// Last-seen screen position while the primary button is held. `None`
    /// when no drag is active.
    last_screen: Option<DVec2>,
}

impl HandTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the tool is currently mid-drag. Exposed for tests and the
    /// shell's cursor-hint logic.
    pub fn is_panning(&self) -> bool {
        self.last_screen.is_some()
    }
}

impl Tool for HandTool {
    fn name(&self) -> &'static str {
        "hand"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        let ToolEvent::Pointer(p) = event else {
            // Hand tool ignores key events. Escape mid-pan is meaningless.
            return ToolResponse::empty();
        };
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                self.last_screen = Some(DVec2::from(screen));
                ToolResponse::cursor(CursorHint::Grabbing)
            }
            PointerEvent::Move { screen, .. } => {
                let Some(prev) = self.last_screen else {
                    // Hover when no drag is active — cursor stays "grab."
                    return ToolResponse::cursor(CursorHint::Grab);
                };
                let current = DVec2::from(screen);
                let delta = current - prev;
                if delta.length_squared() > 0.0 {
                    *ctx.viewport = pan(ctx.viewport, delta);
                }
                self.last_screen = Some(current);
                ToolResponse::cursor(CursorHint::Grabbing)
            }
            PointerEvent::Release {
                button: Button::Primary,
                ..
            } => {
                self.last_screen = None;
                ToolResponse::cursor(CursorHint::Grab)
            }
            // Non-primary buttons and scroll fall through; the shell handles scroll
            // zooming directly via the viewport.
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.last_screen = None;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        // Drop the anchor so a re-activate doesn't think a drag is in progress.
        self.last_screen = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModifierKeys;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, Viewport};

    fn ctx_and_viewport() -> (Doc, Viewport) {
        (Doc::new(), Viewport::default())
    }

    fn press(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 1,
        })
    }

    fn moved(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Move {
            screen,
            modifiers: ModifierKeys::empty(),
        })
    }

    fn release(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Release {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
        })
    }

    #[test]
    fn press_starts_pan_with_grabbing_cursor() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        let r = tool.handle_event(&mut ctx, press([100.0, 100.0]));
        assert!(tool.is_panning());
        assert_eq!(r.cursor, Some(CursorHint::Grabbing));
    }

    #[test]
    fn move_after_press_pans_viewport_proportionally() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let zoom_before = viewport.zoom;
        let center_before = viewport.center;
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        tool.handle_event(&mut ctx, moved([110.0, 95.0]));
        // Zoom unchanged.
        assert_eq!(viewport.zoom, zoom_before);
        // Center moved by -(delta / zoom). delta=(10, -5), zoom=1 → center shifts (-10, 5).
        assert!((viewport.center[0] - (center_before[0] - 10.0)).abs() < 1e-9);
        assert!((viewport.center[1] - (center_before[1] + 5.0)).abs() < 1e-9);
    }

    #[test]
    fn release_clears_pan_state() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([0.0, 0.0]));
        tool.handle_event(&mut ctx, release([0.0, 0.0]));
        assert!(!tool.is_panning());
    }

    #[test]
    fn move_without_press_does_not_change_viewport() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let center_before = viewport.center;
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        let r = tool.handle_event(&mut ctx, moved([50.0, 50.0]));
        assert_eq!(viewport.center, center_before);
        // Hover cursor returns "grab," not "grabbing."
        assert_eq!(r.cursor, Some(CursorHint::Grab));
    }

    #[test]
    fn key_events_are_ignored() {
        use crate::event::{KeyEvent, LogicalKey};
        let (mut doc, mut viewport) = ctx_and_viewport();
        let center_before = viewport.center;
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([0.0, 0.0]));
        let r = tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        // The escape was a no-op, and the press state is still active.
        assert!(tool.is_panning());
        assert!(r.overlays.is_empty());
        assert_eq!(viewport.center, center_before);
    }

    #[test]
    fn release_after_no_press_is_idempotent() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        let r = tool.handle_event(&mut ctx, release([0.0, 0.0]));
        assert_eq!(r.cursor, Some(CursorHint::Grab));
        assert!(!tool.is_panning());
    }

    #[test]
    fn multiple_moves_accumulate_delta_relative_to_last() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([100.0, 100.0]));
        tool.handle_event(&mut ctx, moved([110.0, 100.0])); // +10 x
        tool.handle_event(&mut ctx, moved([130.0, 100.0])); // +20 x more
        // Net: 30 px right at zoom 1 → center moved -30 in x.
        assert!((viewport.center[0] - (-30.0)).abs() < 1e-9);
    }

    #[test]
    fn deactivate_resets_pan_state() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([0.0, 0.0]));
        assert!(tool.is_panning());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_panning());
    }

    #[test]
    fn zoom_does_not_change_during_pan() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        viewport.zoom = 3.0;
        let zoom_before = viewport.zoom;
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([0.0, 0.0]));
        tool.handle_event(&mut ctx, moved([100.0, 50.0]));
        tool.handle_event(&mut ctx, release([100.0, 50.0]));
        assert_eq!(viewport.zoom, zoom_before);
    }

    #[test]
    fn pan_under_zoom_scales_world_delta() {
        let (mut doc, mut viewport) = ctx_and_viewport();
        viewport.zoom = 2.0;
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut tool = HandTool::new();
        tool.handle_event(&mut ctx, press([0.0, 0.0]));
        tool.handle_event(&mut ctx, moved([20.0, 0.0])); // 20 px at zoom 2 = 10 world units
        // World delta = 20 / 2 = 10 → center shifts by -10.
        assert!((viewport.center[0] - (-10.0)).abs() < 1e-9);
    }
}

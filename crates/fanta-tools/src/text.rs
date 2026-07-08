//! Text creation tool.
//!
//! ## Behavior
//!
//! - **Press** records the origin (snapped via the engine).
//! - **Move** updates the in-progress box; a [`ToolOverlay::PreviewRect`] shows
//!   the dragged bounds. The doc is untouched until release.
//! - **Release** commits a [`TextNode`]:
//!   - A bare *click* (no meaningful drag) creates an auto-sizing label
//!     ([`TextAutoResize::WidthAndHeight`]) — Figma's click-to-place text.
//!   - A *drag* creates a fixed-width, auto-height paragraph box
//!     ([`TextAutoResize::Height`]) sized to the drag.
//! - **Escape** mid-drag clears the draft without mutating the doc.
//!
//! The node is created with a `"Text"` placeholder and the tool signals
//! `wants_exit`. The shell (see `AppState::dispatch_pointer`) detects the
//! freshly-created text node, drops straight into the existing inline text
//! editor, and selects the placeholder so the first keystroke replaces it —
//! exactly like double-clicking an existing text node.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, NodeData, Operation, TextAutoResize, TextNode, Transform2D};
use glam::DVec2;

/// Placeholder content for a freshly-created text node. Fully selected on entry
/// so the first character the user types replaces it.
pub const PLACEHOLDER: &str = "Text";

/// Below this drag distance (world units) a press+release is treated as a
/// click-to-place rather than a drag-to-size.
const CLICK_SLOP: f64 = 4.0;

/// Default auto-size label box for a bare click (a layout hint; the box hugs the
/// text once `WidthAndHeight` resizing kicks in).
const DEFAULT_W: f64 = 140.0;
const DEFAULT_H: f64 = 28.0;

/// In-flight drag state for the text tool. World-space.
#[derive(Debug, Clone, Copy)]
struct Draft {
    origin: DVec2,
    current: DVec2,
}

impl Draft {
    fn rect(&self) -> Bounds {
        let a = self.origin;
        let b = self.current;
        Bounds::from_min_max(a.min(b), a.max(b))
    }
}

/// State machine for the text tool.
#[derive(Debug, Default)]
pub struct TextTool {
    draft: Option<Draft>,
}

impl TextTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the tool is mid-drag.
    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for TextTool {
    fn name(&self) -> &'static str {
        "text"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.draft = None;
    }
}

impl TextTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            } => {
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                let origin = snap.world;
                self.draft = Some(Draft {
                    origin,
                    current: origin,
                });
                let mut response = ToolResponse::cursor(CursorHint::Text);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Move { screen, .. } => {
                let Some(mut draft) = self.draft else {
                    return ToolResponse::cursor(CursorHint::Text);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                self.draft = Some(draft);
                let mut response =
                    ToolResponse::cursor(CursorHint::Text).with_overlay(ToolOverlay::PreviewRect {
                        world_rect: draft.rect(),
                    });
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                ..
            } => {
                let Some(mut draft) = self.draft.take() else {
                    return ToolResponse::cursor(CursorHint::Default);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                let rect = draft.rect();

                // Click-to-place (no meaningful drag) → auto-size label.
                // Drag-to-size → fixed-width, auto-height paragraph box.
                let is_click = rect.width() < CLICK_SLOP && rect.height() < CLICK_SLOP;
                let (origin, w, h, auto) = if is_click {
                    (
                        draft.origin,
                        DEFAULT_W,
                        DEFAULT_H,
                        TextAutoResize::WidthAndHeight,
                    )
                } else {
                    (
                        DVec2::new(rect.min_x, rect.min_y),
                        rect.width().max(DEFAULT_H),
                        rect.height().max(DEFAULT_H),
                        TextAutoResize::Height,
                    )
                };

                let mut tn = TextNode::new(PLACEHOLDER, w, h);
                tn.auto_resize = auto;
                tn.style.color = ctx.new_shape_fill;
                let mut node = CanvasNode::new(NodeData::Text(tn));
                // Position the text box via the node transform (TextNode has no
                // x/y of its own — only the layout box size).
                node.transform = Transform2D::translation(origin.x, origin.y);
                ctx.place_new_node_on_active_page(&mut node);
                let id = node.id;
                if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                    tracing::warn!(target: "fanta-tools.text", "create failed: {e}");
                } else {
                    ctx.doc.selection.select_only(id);
                }

                let mut response = ToolResponse::exit().with_cursor(CursorHint::Text);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, k: KeyEvent) -> ToolResponse {
        if matches!(k.key, LogicalKey::Escape) && self.draft.is_some() {
            self.draft = None;
            return ToolResponse::cursor(CursorHint::Default);
        }
        ToolResponse::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::ModifierKeys;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, Viewport};

    fn pe_press(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 1,
        })
    }
    fn pe_move(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Move {
            screen,
            modifiers: ModifierKeys::empty(),
        })
    }
    fn pe_release(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Release {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
        })
    }

    fn ctx_pieces() -> (Doc, Viewport, SnapEngine, DVec2) {
        (
            Doc::new(),
            Viewport::default(),
            SnapEngine {
                zoom: 1.0,
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        )
    }

    #[test]
    fn press_records_origin_no_doc_mutation() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        assert!(tool.is_drafting());
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn click_creates_autosize_text_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        tool.handle_event(&mut ctx, pe_release([400.0, 300.0]));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        let t = match &doc.scene.get(id).unwrap().data {
            NodeData::Text(t) => t,
            _ => panic!("expected text node"),
        };
        assert_eq!(t.content, PLACEHOLDER);
        assert_eq!(t.auto_resize, TextAutoResize::WidthAndHeight);
    }

    #[test]
    fn drag_creates_fixed_width_box() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        // Press at world (0,0), drag to world (300, 120).
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        tool.handle_event(&mut ctx, pe_move([700.0, 420.0]));
        tool.handle_event(&mut ctx, pe_release([700.0, 420.0]));
        let id = doc.scene.roots()[0];
        let t = match &doc.scene.get(id).unwrap().data {
            NodeData::Text(t) => t,
            _ => panic!("expected text node"),
        };
        assert_eq!(t.auto_resize, TextAutoResize::Height);
        assert!((t.local_size[0] - 300.0).abs() < 1e-6);
        assert!((t.local_size[1] - 120.0).abs() < 1e-6);
    }

    #[test]
    fn release_selects_new_node_and_exits() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        let r = tool.handle_event(&mut ctx, pe_release([400.0, 300.0]));
        assert!(r.wants_exit);
        let id = doc.scene.roots()[0];
        assert_eq!(doc.selection.as_slice(), &[id]);
    }

    #[test]
    fn escape_aborts_without_creating_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        tool.handle_event(&mut ctx, pe_move([500.0, 360.0]));
        tool.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert_eq!(doc.scene.len(), 0);
        assert!(!tool.is_drafting());
    }

    #[test]
    fn release_without_press_is_safe() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        let r = tool.handle_event(&mut ctx, pe_release([100.0, 100.0]));
        assert!(!r.wants_exit);
        assert_eq!(doc.scene.len(), 0);
    }

    #[test]
    fn deactivate_clears_draft() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        tool.handle_event(&mut ctx, pe_press([400.0, 300.0]));
        assert!(tool.is_drafting());
        tool.deactivate(&mut ctx);
        assert!(!tool.is_drafting());
    }

    #[test]
    fn new_text_is_positioned_at_press_world() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = TextTool::new();
        // Press at screen (400,300) == world (0,0) at identity viewport.
        tool.handle_event(&mut ctx, pe_press([500.0, 360.0]));
        tool.handle_event(&mut ctx, pe_release([500.0, 360.0]));
        let id = doc.scene.roots()[0];
        let b = doc.scene.world_bounds(id).unwrap();
        // World origin of the press is (100, 60); the box's min should be there.
        assert!((b.min_x - 100.0).abs() < 1.0, "min_x {}", b.min_x);
        assert!((b.min_y - 60.0).abs() < 1.0, "min_y {}", b.min_y);
    }
}

//! Section creation tool — drag-draws a Figma SECTION: an organizational
//! container that carries a box + faint background and a name label, but never
//! clips its children (unlike a FRAME). Same drag interaction as the frame/rect
//! tools; only the committed node differs.
//!
//! A section reuses [`NodeData::Group`] (there is no dedicated node variant),
//! distinguished by `meta.figma_type == "SECTION"` — the same tag the `.fig`
//! importer sets. That tag makes the renderer suppress the border and skip the
//! clip, and makes the editor draw the section's name label, exactly like an
//! imported section. See `fanta-render`'s `is_figma_section` and the editor's
//! frame-label overlay.

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, SnapGuide, Tool, ToolOverlay, ToolResponse};
use fanta_doc::{Bounds, CanvasNode, Color, Fill, GroupNode, NodeData, Operation};
use glam::DVec2;

/// Faint neutral fill so a freshly drawn section is visible as a region without
/// dominating its contents. Matches Figma's light section background.
const SECTION_FILL: Color = Color::rgb(0xEC, 0xEC, 0xEF);

/// In-flight drag state.
#[derive(Debug, Clone, Copy)]
struct Draft {
    origin: DVec2,
    current: DVec2,
}

impl Draft {
    fn rect(&self, modifiers: ModifierKeys) -> Bounds {
        let mut dx = self.current.x - self.origin.x;
        let mut dy = self.current.y - self.origin.y;
        if modifiers.contains(ModifierKeys::SHIFT) {
            let m = dx.abs().max(dy.abs());
            dx = m.copysign(if dx == 0.0 { 1.0 } else { dx });
            dy = m.copysign(if dy == 0.0 { 1.0 } else { dy });
        }
        if modifiers.contains(ModifierKeys::ALT) {
            let half = DVec2::new(dx, dy);
            let min = self.origin - half;
            let max = self.origin + half;
            Bounds::from_min_max(min.min(max), min.max(max))
        } else {
            let a = self.origin;
            let b = self.origin + DVec2::new(dx, dy);
            Bounds::from_min_max(a.min(b), a.max(b))
        }
    }
}

/// State machine for the section tool.
#[derive(Debug, Default)]
pub struct SectionTool {
    draft: Option<Draft>,
}

impl SectionTool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_drafting(&self) -> bool {
        self.draft.is_some()
    }
}

impl Tool for SectionTool {
    fn name(&self) -> &'static str {
        "section"
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

impl SectionTool {
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
                let mut response = ToolResponse::cursor(CursorHint::Crosshair);
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Move { screen, modifiers } => {
                let Some(mut draft) = self.draft else {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                self.draft = Some(draft);
                let rect = draft.rect(modifiers);
                let mut response = ToolResponse::cursor(CursorHint::Crosshair)
                    .with_overlay(ToolOverlay::PreviewRect { world_rect: rect });
                for o in SnapGuide::from_snap_result(&snap) {
                    response.overlays.push(o);
                }
                response
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            } => {
                let Some(mut draft) = self.draft.take() else {
                    return ToolResponse::cursor(CursorHint::Default);
                };
                let world = ctx.screen_to_world(DVec2::from(screen));
                let snap = ctx.snap.snap_point(world, &ctx.doc.scene, &[]);
                draft.current = snap.world;
                let rect = draft.rect(modifiers);
                if rect.width() > 0.0 && rect.height() > 0.0 {
                    let group = GroupNode {
                        // `clip_size` carries the box bounds; the SECTION tag
                        // disables the actual clip at render time.
                        clip_size: Some([rect.width(), rect.height()]),
                        background: Some(Fill::solid(SECTION_FILL)),
                        ..GroupNode::default()
                    };
                    let mut node = CanvasNode::new(NodeData::Group(group));
                    node.name = "Section".to_owned();
                    node.transform = fanta_doc::Transform2D::translation(rect.min_x, rect.min_y);
                    node.meta = serde_json::json!({
                        "figma_type": "SECTION",
                        "clip_content": false,
                    });
                    ctx.place_new_node_on_active_page(&mut node);
                    let id = node.id;
                    if let Err(e) = ctx.doc.apply(Operation::create_node(node)) {
                        tracing::warn!(target: "fanta-tools.section", "create failed: {e}");
                    } else {
                        ctx.doc.selection.select_only(id);
                    }
                }
                let mut response = ToolResponse::exit().with_cursor(CursorHint::Default);
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
    use fanta_canvas::SnapEngine;
    use fanta_doc::{Doc, Viewport};

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

    fn press(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 1,
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
    fn release_commits_a_tagged_section_group() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = SectionTool::new();
        tool.handle_event(&mut ctx, press([400.0, 300.0]));
        tool.handle_event(&mut ctx, release([560.0, 420.0]));
        assert_eq!(doc.scene.len(), 1);
        let id = doc.scene.roots()[0];
        let node = doc.scene.get(id).unwrap();
        assert_eq!(node.name, "Section");
        assert!(matches!(&node.data, NodeData::Group(g) if g.clip_size.is_some()));
        assert_eq!(
            node.meta.get("figma_type").and_then(|v| v.as_str()),
            Some("SECTION")
        );
        assert_eq!(
            node.meta.get("clip_content").and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn zero_size_release_creates_no_node() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        let mut tool = SectionTool::new();
        tool.handle_event(&mut ctx, press([400.0, 300.0]));
        tool.handle_event(&mut ctx, release([400.0, 300.0]));
        assert_eq!(doc.scene.len(), 0);
    }
}

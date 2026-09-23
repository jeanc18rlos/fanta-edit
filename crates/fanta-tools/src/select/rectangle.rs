use super::SelectTool;
use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse, bounds_from_corners};
use fanta_canvas::{HitPrecision, MarqueeMode, hit_test_deep, hit_test_within_screen};
use fanta_doc::{NodeId, Selection};
use glam::DVec2;
use std::collections::HashSet;

use super::state::DRAG_THRESHOLD_PX;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RectangleSelectionOperation {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

#[derive(Default)]
pub struct RectangleSelectTool {
    screen_press: Option<DVec2>,
}

impl RectangleSelectTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn apply_hits(
        operation: RectangleSelectionOperation,
        selection: &mut Selection,
        hits: impl IntoIterator<Item = NodeId>,
    ) {
        match operation {
            RectangleSelectionOperation::Replace => selection.replace_with(hits),
            RectangleSelectionOperation::Add => {
                for id in hits {
                    selection.add(id);
                }
            }
            RectangleSelectionOperation::Subtract | RectangleSelectionOperation::Intersect => {
                let hit_ids: HashSet<_> = hits.into_iter().collect();
                let keep_hits = operation == RectangleSelectionOperation::Intersect;
                let retained: Vec<_> = selection
                    .iter()
                    .copied()
                    .filter(|id| hit_ids.contains(id) == keep_hits)
                    .collect();
                selection.replace_with(retained);
            }
        }
    }

    fn release(
        &mut self,
        ctx: &mut ToolContext,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let Some(screen_press) = self.screen_press.take() else {
            return ToolResponse::cursor(CursorHint::Crosshair);
        };
        if (screen - screen_press).length() >= DRAG_THRESHOLD_PX {
            let mode = if modifiers.contains(ModifierKeys::ALT) {
                MarqueeMode::Intersects
            } else {
                MarqueeMode::Contains
            };
            let hits = hit_test_within_screen(
                &ctx.doc.scene,
                ctx.viewport,
                ctx.screen_size,
                bounds_from_corners(screen_press, screen),
                mode,
                ctx.scope(),
            );
            Self::apply_hits(
                ctx.rectangle_selection_operation,
                &mut ctx.doc.selection,
                hits,
            );
        } else {
            let hits = hit_test_deep(
                &ctx.doc.scene,
                ctx.screen_to_world(screen),
                HitPrecision::Path,
                ctx.scope(),
            );
            let target = hits
                .first()
                .copied()
                .map(|leaf| SelectTool::resolve_in_scope(&ctx.doc.scene, leaf, ctx.scope()));
            Self::apply_hits(
                ctx.rectangle_selection_operation,
                &mut ctx.doc.selection,
                target,
            );
        }
        ToolResponse::cursor(CursorHint::Crosshair)
    }
}

impl Tool for RectangleSelectTool {
    fn name(&self) -> &'static str {
        "rectangle-select"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            }) => {
                self.screen_press = Some(DVec2::from(screen));
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, .. }) => {
                let Some(screen_press) = self.screen_press else {
                    return ToolResponse::cursor(CursorHint::Crosshair);
                };
                let screen = DVec2::from(screen);
                let response = ToolResponse::cursor(CursorHint::Crosshair);
                if (screen - screen_press).length() < DRAG_THRESHOLD_PX {
                    return response;
                }
                response.with_overlay(ToolOverlay::Marquee {
                    screen_rect: bounds_from_corners(screen_press, screen),
                })
            }
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            }) => self.release(ctx, DVec2::from(screen), modifiers),
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.screen_press.take().is_some() {
                    ToolResponse::cursor(CursorHint::Crosshair)
                } else {
                    ToolResponse::exit().with_cursor(CursorHint::Default)
                }
            }
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.screen_press = None;
    }

    fn deactivate(&mut self, _ctx: &mut ToolContext) {
        self.screen_press = None;
    }
}

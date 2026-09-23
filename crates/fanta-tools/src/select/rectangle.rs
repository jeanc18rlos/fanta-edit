use super::SelectTool;
use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse, bounds_from_corners};
use fanta_canvas::{HitPrecision, MarqueeMode, hit_test_deep, hit_test_within_screen};
use fanta_doc::{Bounds, NodeData, NodeFlags, NodeId, Scene, Selection};
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

    pub(super) fn apply_hits(
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

    fn marquee_matches(bounds: Bounds, marquee: Bounds, mode: MarqueeMode) -> bool {
        match mode {
            MarqueeMode::Contains => {
                bounds.min_x >= marquee.min_x
                    && bounds.min_y >= marquee.min_y
                    && bounds.max_x <= marquee.max_x
                    && bounds.max_y <= marquee.max_y
            }
            MarqueeMode::Intersects => bounds.intersects(&marquee),
        }
    }

    fn frame_surface_hits<'a>(
        scene: &'a Scene,
        scope: Option<NodeId>,
        pages: &'a [NodeId],
        marquee: Bounds,
        mode: MarqueeMode,
    ) -> impl Iterator<Item = NodeId> + 'a {
        let candidates = match scope {
            Some(root) => scene.children_of(Some(root)),
            None => scene.roots(),
        };
        candidates.iter().copied().filter(move |&id| {
            if pages.contains(&id) {
                return false;
            }
            let Some(node) = scene.get(id) else {
                return false;
            };
            let NodeData::Group(group) = &node.data else {
                return false;
            };
            group.is_frame_surface()
                && !node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
                && !scene.ancestors_of(id).any(|ancestor| {
                    ancestor
                        .flags
                        .intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
                })
                && scene
                    .world_bounds(id)
                    .is_some_and(|bounds| Self::marquee_matches(bounds, marquee, mode))
        })
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
            let mode = if ctx.draw_content_only || modifiers.contains(ModifierKeys::ALT) {
                MarqueeMode::Intersects
            } else {
                MarqueeMode::Contains
            };
            let screen_rect = bounds_from_corners(screen_press, screen);
            let world_press = ctx.screen_to_world(screen_press);
            let world_end = ctx.screen_to_world(screen);
            let world_rect =
                Bounds::from_min_max(world_press.min(world_end), world_press.max(world_end));
            let scope = ctx.scope();
            let leaves = hit_test_within_screen(
                &ctx.doc.scene,
                ctx.viewport,
                ctx.screen_size,
                screen_rect,
                mode,
                scope,
            );
            let mut seen = HashSet::new();
            let mut hits = Vec::new();
            for leaf in leaves {
                if ctx.draw_content_only {
                    if let Some(target) = super::region::content_target(&ctx.doc.scene, leaf, scope)
                        && ctx
                            .doc
                            .scene
                            .world_bounds(target)
                            .is_some_and(|bounds| Self::marquee_matches(bounds, world_rect, mode))
                        && seen.insert(target)
                    {
                        hits.push(target);
                    }
                    continue;
                }
                let container = SelectTool::resolve_in_scope(&ctx.doc.scene, leaf, scope);
                let target = if ctx
                    .doc
                    .scene
                    .world_bounds(container)
                    .is_some_and(|bounds| Self::marquee_matches(bounds, world_rect, mode))
                {
                    container
                } else {
                    leaf
                };
                if seen.insert(target) {
                    hits.push(target);
                }
            }
            if !ctx.draw_content_only {
                for frame in Self::frame_surface_hits(
                    &ctx.doc.scene,
                    scope,
                    ctx.doc.pages(),
                    world_rect,
                    mode,
                ) {
                    if seen.insert(frame) {
                        hits.push(frame);
                    }
                }
            }
            if ctx.draw_content_only {
                super::region::apply_draw_region(
                    ctx,
                    super::region::DrawSelectionShape::Rectangle(world_rect),
                    hits,
                );
            } else {
                Self::apply_hits(
                    ctx.rectangle_selection_operation,
                    &mut ctx.doc.selection,
                    hits,
                );
            }
        } else {
            if ctx.draw_content_only {
                ctx.draw_selection_region = None;
            }
            let hits = hit_test_deep(
                &ctx.doc.scene,
                ctx.screen_to_world(screen),
                HitPrecision::Path,
                ctx.scope(),
            );
            let target = if ctx.draw_content_only {
                hits.into_iter().find_map(|leaf| {
                    super::region::content_target(&ctx.doc.scene, leaf, ctx.scope())
                })
            } else {
                hits.first()
                    .copied()
                    .map(|leaf| SelectTool::resolve_in_scope(&ctx.doc.scene, leaf, ctx.scope()))
            };
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

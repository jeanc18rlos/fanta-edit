use fanta_canvas::{HitPrecision, hit_test_deep};
use fanta_doc::{
    BooleanNode, BooleanOp, Bounds, CanvasNode, Color, Fill, GroupNode, NodeData, NodeId,
    Operation, PathData, Transform2D, VectorNode,
};
use glam::DVec2;

use super::region::{DrawSelectionShape, DrawShapeOperation, content_target};
use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse, bounds_from_corners};

#[derive(Default)]
pub struct CropTool {
    press: Option<DVec2>,
    pointer: Option<DVec2>,
    pending: Option<Bounds>,
}

impl CropTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn bounds(&self, ctx: &ToolContext, end: DVec2) -> Option<Bounds> {
        let press = self.press?;
        let start = ctx.screen_to_world(press);
        let mut end = ctx.screen_to_world(end);
        if let Some(aspect) = ctx.crop_aspect_ratio
            && aspect.is_finite()
            && aspect > 0.0
        {
            let delta = end - start;
            let sign_x = delta.x.signum();
            let sign_y = delta.y.signum();
            if delta.x.abs() / aspect > delta.y.abs() {
                end.y = start.y + sign_y * delta.x.abs() / aspect;
            } else {
                end.x = start.x + sign_x * delta.y.abs() * aspect;
            }
        }
        let bounds = bounds_from_corners(start, end);
        (bounds.width() > 1e-6 && bounds.height() > 1e-6 && bounds.is_finite()).then_some(bounds)
    }

    fn selected_content(&self, ctx: &ToolContext, crop: Bounds) -> Vec<NodeId> {
        let selected: Vec<_> = ctx
            .doc
            .selection
            .iter()
            .copied()
            .filter(|id| content_target(&ctx.doc.scene, *id, ctx.scope()) == Some(*id))
            .filter(|id| {
                ctx.doc
                    .scene
                    .world_bounds(*id)
                    .is_some_and(|bounds| bounds.intersects(&crop))
            })
            .collect();
        if !selected.is_empty() {
            return selected;
        }
        hit_test_deep(
            &ctx.doc.scene,
            crop.center(),
            HitPrecision::Path,
            ctx.scope(),
        )
        .into_iter()
        .find_map(|id| content_target(&ctx.doc.scene, id, ctx.scope()))
        .into_iter()
        .collect()
    }

    fn apply(&mut self, ctx: &mut ToolContext) {
        let region = ctx.draw_selection_region.clone();
        let Some(crop) = self
            .pending
            .take()
            .or_else(|| region.as_ref().and_then(|region| region.shape.bounds()))
        else {
            return;
        };
        let mut targets = self.selected_content(ctx, crop);
        if let Some(region) = &region {
            targets.retain(|id| region.targets.contains(id));
        }
        if targets.is_empty() {
            return;
        }
        ctx.doc.history.begin("Crop", &mut ctx.doc.scene);
        let mut cropped = Vec::new();
        for id in targets {
            match crop_content(ctx, id, crop, region.as_ref().map(|region| &region.shape)) {
                Ok(group) => cropped.push(group),
                Err(error) => {
                    tracing::warn!(target: "fanta-tools.crop", "crop failed: {error}");
                    if let Err(abort_error) = ctx.doc.history.abort(&mut ctx.doc.scene) {
                        tracing::error!(target: "fanta-tools.crop", "crop rollback failed: {abort_error}");
                    }
                    return;
                }
            }
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
        ctx.doc.selection.replace_with(cropped);
        ctx.draw_selection_region = None;
    }

    fn overlay(&self) -> ToolResponse {
        let mut response = ToolResponse::cursor(CursorHint::Crosshair);
        if let Some(bounds) = self.pending {
            response = response.with_overlay(ToolOverlay::PreviewRect { world_rect: bounds });
        }
        response
    }
}

impl Tool for CropTool {
    fn name(&self) -> &'static str {
        "crop"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            }) => {
                self.press = Some(DVec2::from(screen));
                self.pointer = Some(DVec2::from(screen));
                self.pending = None;
                self.overlay()
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, .. }) => {
                if self.press.is_some() {
                    self.pointer = Some(DVec2::from(screen));
                    if let Some(bounds) = self.bounds(ctx, DVec2::from(screen)) {
                        return ToolResponse::cursor(CursorHint::Crosshair)
                            .with_overlay(ToolOverlay::PreviewRect { world_rect: bounds });
                    }
                }
                self.overlay()
            }
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                ..
            }) => {
                self.pending = self.bounds(ctx, DVec2::from(screen));
                self.press = None;
                self.pointer = None;
                self.overlay()
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Enter => {
                self.apply(ctx);
                self.overlay()
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.pending.take().is_some() || self.press.take().is_some() {
                    self.pointer = None;
                    self.overlay()
                } else {
                    ToolResponse::exit().with_cursor(CursorHint::Default)
                }
            }
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.press = None;
        self.pointer = None;
        self.pending = None;
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.activate(ctx);
    }
}

fn crop_content(
    ctx: &mut ToolContext,
    id: NodeId,
    crop: Bounds,
    selection_shape: Option<&DrawSelectionShape>,
) -> Result<NodeId, String> {
    let node = ctx.doc.scene.get(id).ok_or("Missing content")?;
    if !matches!(
        node.data,
        NodeData::Vector(_) | NodeData::Bitmap(_) | NodeData::Boolean(_)
    ) {
        return Err("Only vector and image content can be cropped".into());
    }
    let parent = node.parent;
    let index = node.index;
    let old_local = node.transform;
    let old_world = ctx
        .doc
        .scene
        .world_transform(id)
        .ok_or("Missing content transform")?;
    let parent_world = parent
        .and_then(|parent| ctx.doc.scene.world_transform(parent))
        .unwrap_or(Transform2D::IDENTITY);
    let [a, b, c, d, _, _] = parent_world.to_components();
    if (a * d - b * c).abs() < 1e-12 {
        return Err("Cannot crop content inside a flattened transform".into());
    }
    let crop_world = Transform2D::translation(crop.min_x, crop.min_y);
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([crop.width(), crop.height()]),
        clip_size: Some([crop.width(), crop.height()]),
        ..GroupNode::default()
    }));
    group.name = "Crop".into();
    group.parent = parent;
    group.index = index;
    group.transform = crop_world.then(&parent_world.inverse());
    let group_id = group.id;
    ctx.doc
        .apply(Operation::create_node(group))
        .map_err(|error| error.to_string())?;
    if let Some(selection_shape) = selection_shape {
        create_selection_mask(ctx, group_id, selection_shape, crop, true)?;
    }
    ctx.doc
        .apply(Operation::Reparent {
            id,
            old_parent: parent,
            old_index: index,
            new_parent: Some(group_id),
            new_index: ctx.doc.scene.next_child_index(Some(group_id)),
        })
        .map_err(|error| error.to_string())?;
    let new_local = old_world.then(&crop_world.inverse());
    ctx.doc
        .apply(Operation::SetTransform {
            id,
            old: old_local,
            new: new_local,
        })
        .map_err(|error| error.to_string())?;
    Ok(group_id)
}

fn create_selection_mask(
    ctx: &mut ToolContext,
    parent: NodeId,
    shape: &DrawSelectionShape,
    crop: Bounds,
    is_mask: bool,
) -> Result<NodeId, String> {
    let data = match shape {
        DrawSelectionShape::Combined { operation, .. } => {
            let op = match operation {
                DrawShapeOperation::Union => BooleanOp::Union,
                DrawShapeOperation::Subtract => BooleanOp::Subtract,
                DrawShapeOperation::Intersect => BooleanOp::Intersect,
            };
            NodeData::Boolean(BooleanNode {
                op,
                fills: smallvec::smallvec![Fill::solid(Color::WHITE)],
                strokes: smallvec::smallvec![],
            })
        }
        _ => NodeData::Vector(VectorNode {
            path: selection_mask_path(shape, crop)?,
            fills: smallvec::smallvec![Fill::solid(Color::WHITE)],
            ..VectorNode::default()
        }),
    };
    let mut mask = CanvasNode::new(data);
    mask.name = "Selection mask".into();
    mask.parent = Some(parent);
    mask.index = ctx.doc.scene.next_child_index(Some(parent));
    mask.is_mask = is_mask;
    let id = mask.id;
    ctx.doc
        .apply(Operation::create_node(mask))
        .map_err(|error| error.to_string())?;
    if let DrawSelectionShape::Combined { first, second, .. } = shape {
        create_selection_mask(ctx, id, first, crop, false)?;
        create_selection_mask(ctx, id, second, crop, false)?;
    }
    Ok(id)
}

fn selection_mask_path(shape: &DrawSelectionShape, crop: Bounds) -> Result<PathData, String> {
    let path = match shape {
        DrawSelectionShape::Rectangle(bounds) => PathData::rect(
            bounds.min_x - crop.min_x,
            bounds.min_y - crop.min_y,
            bounds.width(),
            bounds.height(),
        ),
        DrawSelectionShape::Ellipse(bounds) => PathData::ellipse(
            bounds.center().x - crop.min_x,
            bounds.center().y - crop.min_y,
            bounds.width() * 0.5,
            bounds.height() * 0.5,
        ),
        DrawSelectionShape::Polygon(points) => {
            let Some(first) = points.first() else {
                return Err("The lasso selection is empty".into());
            };
            if points.len() < 3 {
                return Err("The lasso selection needs at least three points".into());
            }
            let mut path = PathData::new();
            path.move_to(first[0] - crop.min_x, first[1] - crop.min_y);
            for point in points.iter().skip(1) {
                path.line_to(point[0] - crop.min_x, point[1] - crop.min_y);
            }
            path.close();
            path
        }
        DrawSelectionShape::RasterRuns { quads, .. } => {
            if quads.is_empty() {
                return Err("The pixel selection is empty".into());
            }
            let mut path = PathData::new();
            for quad in quads {
                path.move_to(quad[0][0] - crop.min_x, quad[0][1] - crop.min_y);
                for point in quad.iter().skip(1) {
                    path.line_to(point[0] - crop.min_x, point[1] - crop.min_y);
                }
                path.close();
            }
            path
        }
        DrawSelectionShape::Combined { .. } => {
            return Err("A combined selection needs a compound mask".into());
        }
    };
    Ok(path)
}

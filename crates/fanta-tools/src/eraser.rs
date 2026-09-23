use crate::context::ToolContext;
use crate::event::{Button, LogicalKey, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolResponse};
use fanta_canvas::{BEZIER_FLATTEN_STEPS, hit_test::on_active_page};
use fanta_doc::{
    BooleanNode, BooleanOp, Bounds, CanvasNode, NodeData, NodeFlags, NodeId, Operation, PathData,
    PathSegment, Scene, SceneError, Transform2D,
};
use glam::DVec2;
use std::collections::HashMap;

#[derive(Default)]
pub struct EraserTool {
    last_world: Option<DVec2>,
    max_stroke_radius: f64,
    cached_scene: Option<(u64, u64, Option<NodeId>)>,
    deleted_widest_stroke: bool,
    active_subtractions: HashMap<NodeId, NodeId>,
}

impl EraserTool {
    pub fn new() -> Self {
        Self::default()
    }

    fn refresh_stroke_radius(&mut self, ctx: &ToolContext) {
        let key = (
            ctx.doc.scene.instance_id(),
            ctx.doc.scene.revision(),
            ctx.scope(),
        );
        if self.cached_scene != Some(key) {
            self.max_stroke_radius = max_stroke_radius(&ctx.doc.scene, key.2);
            self.cached_scene = Some(key);
        }
    }

    fn erase_segment(&mut self, ctx: &mut ToolContext, end: DVec2) {
        self.refresh_stroke_radius(ctx);
        let start = self.last_world.unwrap_or(end);
        if self.last_world == Some(end) {
            return;
        }
        self.last_world = Some(end);
        let radius = (ctx.new_stroke_width * 0.5).clamp(0.5, 2500.0);
        // Vector bounds cover their paths but not the paint around those paths.
        let padding = DVec2::splat(radius + self.max_stroke_radius);
        let query = Bounds::from_min_max(start.min(end) - padding, start.max(end) + padding);
        let scope = ctx.scope();
        let candidates = ctx.doc.scene.rect_query_where(query, |id, _| {
            scope.is_none_or(|root| id != root && on_active_page(&ctx.doc.scene, id, root))
        });
        for id in candidates {
            if !editable_mark(&ctx.doc.scene, id) {
                continue;
            }
            if !mark_intersects_sweep(&ctx.doc.scene, id, start, end, radius) {
                continue;
            }
            let removed_widest_stroke = self.max_stroke_radius > 0.0
                && stroke_radius(&ctx.doc.scene, id)
                    .is_some_and(|mark_radius| mark_radius >= self.max_stroke_radius);
            if let Err(error) = self.subtract_sweep(ctx, id, start, end, radius) {
                tracing::warn!(target: "fanta-tools.eraser", "erase failed: {error}");
                if let Err(error) = ctx.doc.abort_transaction() {
                    tracing::warn!(target: "fanta-tools.eraser", "rollback failed: {error}");
                }
                self.last_world = None;
                self.cached_scene = None;
                self.active_subtractions.clear();
                return;
            }
            ctx.doc.selection.remove(id);
            self.deleted_widest_stroke |= removed_widest_stroke;
        }
        self.cached_scene = Some((ctx.doc.scene.instance_id(), ctx.doc.scene.revision(), scope));
    }

    fn subtract_sweep(
        &mut self,
        ctx: &mut ToolContext,
        id: NodeId,
        start: DVec2,
        end: DVec2,
        radius: f64,
    ) -> Result<(), SceneError> {
        let Some(original_world) = ctx.doc.scene.world_transform(id) else {
            return Ok(());
        };
        let original_determinant = original_world.0.matrix2.determinant();
        if !original_determinant.is_finite() || original_determinant.abs() < 1e-12 {
            return Ok(());
        }
        let boolean_id = if is_erased_mark(&ctx.doc.scene, id) {
            id
        } else {
            wrap_mark(ctx, id)?
        };
        let Some(world_transform) = ctx.doc.scene.world_transform(boolean_id) else {
            return Ok(());
        };
        let determinant = world_transform.0.matrix2.determinant();
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return Ok(());
        }
        let inverse = world_transform.inverse();
        let mut swept_area = capsule(start, end, radius);
        swept_area.map_points_mut(|point| {
            let local = inverse.transform_point(DVec2::from(point));
            [local.x, local.y]
        });

        if let Some(&operand_id) = self.active_subtractions.get(&boolean_id) {
            if let Some(node) = ctx.doc.scene.get(operand_id)
                && let NodeData::Vector(vector) = &node.data
            {
                let old = node.data.clone();
                let mut updated = vector.clone();
                updated.path.segments.extend(swept_area.segments);
                ctx.doc.apply(Operation::ReplaceData {
                    id: operand_id,
                    old: Box::new(old),
                    new: Box::new(NodeData::Vector(updated)),
                })?;
                return Ok(());
            }
        }

        let mut operand = CanvasNode::new(NodeData::Vector(fanta_doc::VectorNode {
            path: swept_area,
            ..Default::default()
        }));
        operand.parent = Some(boolean_id);
        operand.index = ctx.doc.scene.next_child_index(Some(boolean_id));
        let operand_id = operand.id;
        ctx.doc.apply(Operation::create_node(operand))?;
        self.active_subtractions.insert(boolean_id, operand_id);
        Ok(())
    }

    fn finish(&mut self, ctx: &mut ToolContext) {
        if self.last_world.take().is_some() {
            ctx.doc.history.commit(&mut ctx.doc.scene);
        }
        if self.deleted_widest_stroke {
            self.cached_scene = None;
            self.deleted_widest_stroke = false;
        }
        self.active_subtractions.clear();
    }
}

impl Tool for EraserTool {
    fn name(&self) -> &'static str {
        "eraser"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(PointerEvent::Press {
                screen,
                button: Button::Primary,
                ..
            }) => {
                self.finish(ctx);
                self.active_subtractions.clear();
                ctx.doc.history.begin("Erase strokes", &mut ctx.doc.scene);
                self.erase_segment(ctx, ctx.screen_to_world(DVec2::from(screen)));
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Pointer(PointerEvent::Move { screen, .. }) if self.last_world.is_some() => {
                self.erase_segment(ctx, ctx.screen_to_world(DVec2::from(screen)));
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Pointer(PointerEvent::Release {
                screen,
                button: Button::Primary,
                ..
            }) if self.last_world.is_some() => {
                self.erase_segment(ctx, ctx.screen_to_world(DVec2::from(screen)));
                self.finish(ctx);
                ToolResponse::cursor(CursorHint::Crosshair)
            }
            ToolEvent::Key(key) if key.key == LogicalKey::Escape => {
                if self.last_world.take().is_some() {
                    if let Err(error) = ctx.doc.abort_transaction() {
                        tracing::warn!(target: "fanta-tools.eraser", "cancel failed: {error}");
                    }
                    self.cached_scene = None;
                    self.deleted_widest_stroke = false;
                    self.active_subtractions.clear();
                    ToolResponse::cursor(CursorHint::Crosshair)
                } else {
                    ToolResponse::exit()
                }
            }
            _ => ToolResponse::empty(),
        }
    }

    fn activate(&mut self, ctx: &mut ToolContext) {
        self.finish(ctx);
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.finish(ctx);
    }
}

fn wrap_mark(ctx: &mut ToolContext, id: NodeId) -> Result<NodeId, SceneError> {
    let source = ctx
        .doc
        .scene
        .get(id)
        .cloned()
        .ok_or(SceneError::NotFound(id))?;
    let NodeData::Vector(vector) = &source.data else {
        return Err(SceneError::InvariantViolated(
            "eraser source is not a vector".into(),
        ));
    };
    let fills = if vector.fills.is_empty() {
        let Some(stroke) = vector.strokes.first() else {
            return Err(SceneError::InvariantViolated(
                "eraser source has no paint".into(),
            ));
        };
        let mut outlined = vector.clone();
        outlined.path = vector
            .path
            .stroke_to_fill(stroke.width, stroke.cap, stroke.join);
        outlined.fills.push(stroke.paint.clone());
        outlined.strokes.clear();
        outlined.parametric = None;
        ctx.doc.apply(Operation::ReplaceData {
            id,
            old: Box::new(source.data.clone()),
            new: Box::new(NodeData::Vector(outlined)),
        })?;
        std::iter::once(stroke.paint.clone()).collect()
    } else {
        vector.fills.clone()
    };

    let mut wrapper = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op: BooleanOp::Subtract,
        fills,
        strokes: Default::default(),
    }));
    wrapper.parent = source.parent;
    wrapper.index = source.index;
    wrapper.name = source.name;
    wrapper.transform = source.transform;
    wrapper.constraints = source.constraints;
    wrapper.opacity = source.opacity;
    wrapper.blend_mode = source.blend_mode;
    wrapper.effects = source.effects;
    wrapper.blurs = source.blurs;
    wrapper.flags = source.flags;
    wrapper.is_mask = source.is_mask;
    wrapper.mask_type = source.mask_type;
    wrapper.scroll_behavior = source.scroll_behavior;
    wrapper.layout_child = source.layout_child;
    wrapper.bindings = source.bindings;
    wrapper.reactions = source.reactions;
    wrapper.meta = source.meta;
    if let Some(meta) = wrapper.meta.as_object_mut() {
        meta.insert("fanta_eraser_result".into(), true.into());
    } else {
        wrapper.meta = serde_json::json!({ "fanta_eraser_result": true });
    }
    let wrapper_id = wrapper.id;
    ctx.doc.apply(Operation::create_node(wrapper))?;
    ctx.doc.apply(Operation::Reparent {
        id,
        old_parent: source.parent,
        old_index: source.index,
        new_parent: Some(wrapper_id),
        new_index: ctx.doc.scene.next_child_index(Some(wrapper_id)),
    })?;
    if source.transform != Transform2D::IDENTITY {
        ctx.doc.apply(Operation::SetTransform {
            id,
            old: source.transform,
            new: Transform2D::IDENTITY,
        })?;
    }
    Ok(wrapper_id)
}

fn capsule(start: DVec2, end: DVec2, radius: f64) -> PathData {
    if (end - start).length_squared() <= f64::EPSILON {
        return PathData::ellipse(start.x, start.y, radius, radius);
    }
    let tangent = (end - start).normalize();
    let normal = DVec2::new(-tangent.y, tangent.x);
    let n = normal * radius;
    let t = tangent * radius;
    let k = 0.552_284_749_830_793_3;
    let mut path = PathData::new();
    let point = start + n;
    path.move_to(point.x, point.y);
    let point = end + n;
    path.line_to(point.x, point.y);
    let (control1, control2, point) = (end + n + t * k, end + t + n * k, end + t);
    path.cubic_to(
        control1.x, control1.y, control2.x, control2.y, point.x, point.y,
    );
    let (control1, control2, point) = (end + t - n * k, end - n + t * k, end - n);
    path.cubic_to(
        control1.x, control1.y, control2.x, control2.y, point.x, point.y,
    );
    let point = start - n;
    path.line_to(point.x, point.y);
    let (control1, control2, point) = (start - n - t * k, start - t - n * k, start - t);
    path.cubic_to(
        control1.x, control1.y, control2.x, control2.y, point.x, point.y,
    );
    let (control1, control2, point) = (start - t + n * k, start + n - t * k, start + n);
    path.cubic_to(
        control1.x, control1.y, control2.x, control2.y, point.x, point.y,
    );
    path.close();
    path
}

fn max_stroke_radius(scene: &Scene, scope: Option<NodeId>) -> f64 {
    let roots: Vec<_> = scope.map_or_else(|| scene.roots().to_vec(), |root| vec![root]);
    roots
        .into_iter()
        .flat_map(|root| scene.descendants_of(root))
        .filter_map(|id| stroke_radius(scene, id))
        .fold(0.0, f64::max)
}

fn stroke_radius(scene: &Scene, id: NodeId) -> Option<f64> {
    let node = scene.get(id)?;
    let NodeData::Vector(vector) = &node.data else {
        return None;
    };
    if !vector.fills.is_empty() || vector.strokes.is_empty() {
        return None;
    }
    let transform = scene.world_transform(id)?;
    let scale = transform
        .transform_vector(DVec2::X)
        .length()
        .max(transform.transform_vector(DVec2::Y).length());
    Some(
        vector
            .strokes
            .iter()
            .map(|stroke| stroke.width.max(0.0) * scale * 0.5)
            .fold(0.0, f64::max),
    )
}

fn editable_mark(scene: &Scene, id: NodeId) -> bool {
    let Some(node) = scene.get(id) else {
        return false;
    };
    if node.flags.intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
        || scene.ancestors_of(id).any(|ancestor| {
            ancestor
                .flags
                .intersects(NodeFlags::HIDDEN | NodeFlags::LOCKED)
                || matches!(ancestor.data, NodeData::Boolean(_))
        })
    {
        return false;
    }
    if is_erased_mark(scene, id) {
        return true;
    }
    let NodeData::Vector(vector) = &node.data else {
        return false;
    };
    node.meta.get("fanta_draw_mark") == Some(&serde_json::json!("brush"))
        || (vector.fills.is_empty()
            && vector.strokes.len() == 1
            && vector.strokes.first().is_some_and(|stroke| {
                stroke.width.is_finite() && stroke.width > 0.0 && stroke.dash.is_empty()
            }))
}

fn is_erased_mark(scene: &Scene, id: NodeId) -> bool {
    scene.get(id).is_some_and(|node| {
        matches!(node.data, NodeData::Boolean(_))
            && node.meta.get("fanta_eraser_result") == Some(&serde_json::Value::Bool(true))
    })
}

fn mark_intersects_sweep(scene: &Scene, id: NodeId, start: DVec2, end: DVec2, radius: f64) -> bool {
    let Some(node) = scene.get(id) else {
        return false;
    };
    if is_erased_mark(scene, id) {
        return scene
            .children_of(Some(id))
            .first()
            .is_some_and(|source| mark_intersects_sweep(scene, *source, start, end, radius));
    }
    let NodeData::Vector(vector) = &node.data else {
        return false;
    };
    let Some(transform) = scene.world_transform(id) else {
        return false;
    };
    let filled = !vector.fills.is_empty()
        || scene
            .ancestors_of(id)
            .any(|ancestor| matches!(ancestor.data, NodeData::Boolean(_)));
    let mark_radius = stroke_radius(scene, id).unwrap_or(0.0);
    let threshold = radius + mark_radius;
    let mut first = None;
    let mut previous = None;
    let mut outline = Vec::new();
    let mut inside_fill = false;
    for segment in &vector.path.segments {
        match segment {
            PathSegment::Move { to } => {
                if filled && outline.len() >= 3 {
                    inside_fill |=
                        point_in_polygon(&outline, start) || point_in_polygon(&outline, end);
                }
                outline.clear();
                let point = transform.transform_point(DVec2::from(*to));
                first = Some(point);
                previous = Some(point);
                outline.push(point);
            }
            PathSegment::Line { to } => {
                let point = transform.transform_point(DVec2::from(*to));
                if trace_point(point, &mut previous, &mut outline, start, end, threshold) {
                    return true;
                }
            }
            PathSegment::Quad { ctrl, to } => {
                let Some(from) = previous else {
                    continue;
                };
                let control = transform.transform_point(DVec2::from(*ctrl));
                let to = transform.transform_point(DVec2::from(*to));
                for step in 1..=BEZIER_FLATTEN_STEPS {
                    let t = f64::from(step) / f64::from(BEZIER_FLATTEN_STEPS);
                    let remaining = 1.0 - t;
                    let point =
                        remaining * remaining * from + 2.0 * remaining * t * control + t * t * to;
                    if trace_point(point, &mut previous, &mut outline, start, end, threshold) {
                        return true;
                    }
                }
            }
            PathSegment::Cubic { ctrl1, ctrl2, to } => {
                let Some(from) = previous else {
                    continue;
                };
                let control1 = transform.transform_point(DVec2::from(*ctrl1));
                let control2 = transform.transform_point(DVec2::from(*ctrl2));
                let to = transform.transform_point(DVec2::from(*to));
                for step in 1..=BEZIER_FLATTEN_STEPS {
                    let t = f64::from(step) / f64::from(BEZIER_FLATTEN_STEPS);
                    let remaining = 1.0 - t;
                    let point = remaining.powi(3) * from
                        + 3.0 * remaining.powi(2) * t * control1
                        + 3.0 * remaining * t * t * control2
                        + t.powi(3) * to;
                    if trace_point(point, &mut previous, &mut outline, start, end, threshold) {
                        return true;
                    }
                }
            }
            PathSegment::Close => {
                let (Some(previous), Some(first)) = (previous, first) else {
                    continue;
                };
                if segment_distance(start, end, previous, first) <= threshold {
                    return true;
                }
            }
        }
    }
    if !filled || outline.len() < 3 {
        return inside_fill;
    }
    inside_fill || point_in_polygon(&outline, start) || point_in_polygon(&outline, end)
}

fn trace_point(
    point: DVec2,
    previous: &mut Option<DVec2>,
    outline: &mut Vec<DVec2>,
    start: DVec2,
    end: DVec2,
    threshold: f64,
) -> bool {
    let intersects = previous
        .as_ref()
        .is_some_and(|previous| segment_distance(start, end, *previous, point) <= threshold);
    *previous = Some(point);
    outline.push(point);
    intersects
}

fn point_in_polygon(points: &[DVec2], point: DVec2) -> bool {
    let mut inside = false;
    let mut previous = points[points.len() - 1];
    for &current in points {
        if (current.y > point.y) != (previous.y > point.y) {
            let x = current.x
                + (point.y - current.y) * (previous.x - current.x) / (previous.y - current.y);
            if point.x < x {
                inside = !inside;
            }
        }
        previous = current;
    }
    inside
}

fn segment_distance(a: DVec2, b: DVec2, c: DVec2, d: DVec2) -> f64 {
    let side_a = (b - a).perp_dot(c - a);
    let side_b = (b - a).perp_dot(d - a);
    let side_c = (d - c).perp_dot(a - c);
    let side_d = (d - c).perp_dot(b - c);
    let boxes_overlap = a.x.min(b.x) <= c.x.max(d.x)
        && c.x.min(d.x) <= a.x.max(b.x)
        && a.y.min(b.y) <= c.y.max(d.y)
        && c.y.min(d.y) <= a.y.max(b.y);
    if boxes_overlap && side_a * side_b <= 0.0 && side_c * side_d <= 0.0 {
        return 0.0;
    }
    point_segment_distance(a, c, d)
        .min(point_segment_distance(b, c, d))
        .min(point_segment_distance(c, a, b))
        .min(point_segment_distance(d, a, b))
}

fn point_segment_distance(point: DVec2, start: DVec2, end: DVec2) -> f64 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f64::EPSILON {
        return (point - start).length();
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    (point - (start + segment * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brush::BrushTool;
    use crate::event::{KeyEvent, ModifierKeys};
    use crate::pencil::PencilTool;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{
        CanvasNode, Color, Doc, GroupNode, NodeData, Operation, PathData, Stroke, VectorNode,
        Viewport,
    };

    fn pointer(screen: [f64; 2], phase: &str) -> ToolEvent {
        let modifiers = ModifierKeys::empty();
        ToolEvent::Pointer(match phase {
            "press" => PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count: 1,
            },
            "move" => PointerEvent::Move { screen, modifiers },
            _ => PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            },
        })
    }

    fn painted_at(scene: &Scene, wrapper: NodeId, point: DVec2) -> bool {
        let Some((&source, subtractors)) = scene.children_of(Some(wrapper)).split_first() else {
            return false;
        };
        mark_intersects_sweep(scene, source, point, point, 0.0)
            && subtractors
                .iter()
                .all(|id| !mark_intersects_sweep(scene, *id, point, point, 0.0))
    }

    #[test]
    fn eraser_cuts_only_the_middle_of_a_pencil_line() {
        let mut doc = Doc::new();
        let mut path = PathData::new();
        path.move_to(-60.0, 0.0).line_to(60.0, 0.0);
        let original = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            strokes: [Stroke::solid(Color::BLACK, 12.0)].into_iter().collect(),
            ..VectorNode::default()
        }));
        let original_id = original.id;
        doc.apply(Operation::create_node(original))
            .expect("create pencil line");
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        ctx.new_stroke_width = 20.0;
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "release"));

        let wrapper = ctx.doc.scene.roots()[0];
        assert!(is_erased_mark(&ctx.doc.scene, wrapper));
        assert_eq!(ctx.doc.scene.children_of(Some(wrapper)).len(), 2);
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(-40.0, 0.0)));
        assert!(!painted_at(&ctx.doc.scene, wrapper, DVec2::ZERO));
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(40.0, 0.0)));
        assert!(ctx.doc.undo().expect("undo eraser"));
        assert_eq!(ctx.doc.scene.roots(), &[original_id]);
        assert_eq!(ctx.doc.scene.len(), 1);
        assert!(ctx.doc.redo().expect("redo eraser"));
        assert!(!painted_at(&ctx.doc.scene, wrapper, DVec2::ZERO));
    }

    #[test]
    fn eraser_drag_keeps_one_subtraction_operand_and_covers_the_sweep() {
        let mut doc = Doc::new();
        let mut path = PathData::new();
        path.move_to(-80.0, 0.0).line_to(80.0, 0.0);
        let original = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            strokes: [Stroke::solid(Color::BLACK, 12.0)].into_iter().collect(),
            ..VectorNode::default()
        }));
        doc.apply(Operation::create_node(original))
            .expect("create pencil line");
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        ctx.new_stroke_width = 20.0;
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        eraser.handle_event(&mut ctx, pointer([425.0, 300.0], "move"));
        eraser.handle_event(&mut ctx, pointer([450.0, 300.0], "release"));

        let wrapper = ctx.doc.scene.roots()[0];
        let children = ctx.doc.scene.children_of(Some(wrapper));
        assert_eq!(children.len(), 2);
        let subtraction = ctx.doc.scene.get(children[1]).expect("subtraction operand");
        let NodeData::Vector(vector) = &subtraction.data else {
            panic!("subtraction must contain vector geometry");
        };
        assert_eq!(
            vector
                .path
                .segments
                .iter()
                .filter(|segment| matches!(segment, PathSegment::Move { .. }))
                .count(),
            3
        );
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(-40.0, 0.0)));
        for x in [0.0, 20.0, 40.0] {
            assert!(!painted_at(&ctx.doc.scene, wrapper, DVec2::new(x, 0.0)));
        }
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(70.0, 0.0)));
    }

    #[test]
    fn eraser_cuts_brush_outline_without_deleting_its_ends() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        ctx.new_stroke_width = 20.0;
        let mut brush = BrushTool::new();
        brush.handle_event(&mut ctx, pointer([340.0, 300.0], "press"));
        brush.handle_event(&mut ctx, pointer([460.0, 300.0], "release"));
        let brush_id = ctx.doc.scene.roots()[0];
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "release"));

        let wrapper = ctx.doc.scene.roots()[0];
        assert!(is_erased_mark(&ctx.doc.scene, wrapper));
        assert_eq!(
            ctx.doc.scene.get(brush_id).and_then(|node| node.parent),
            Some(wrapper)
        );
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(-40.0, 0.0)));
        assert!(!painted_at(&ctx.doc.scene, wrapper, DVec2::ZERO));
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(40.0, 0.0)));
    }

    #[test]
    fn eraser_partially_subtracts_pencil_and_brush_as_one_undo_step() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let size = DVec2::new(800.0, 600.0);
        {
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
            let mut pencil = PencilTool::new();
            pencil.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
            pencil.handle_event(&mut ctx, pointer([440.0, 300.0], "release"));
            let mut brush = BrushTool::new();
            brush.handle_event(&mut ctx, pointer([460.0, 300.0], "press"));
            brush.handle_event(&mut ctx, pointer([500.0, 300.0], "release"));
        }
        assert_eq!(doc.scene.len(), 2);
        let original_ids = doc.scene.roots().to_vec();
        {
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
            ctx.new_stroke_width = 24.0;
            let mut eraser = EraserTool::new();
            eraser.handle_event(&mut ctx, pointer([420.0, 300.0], "press"));
            assert_eq!(ctx.doc.scene.len(), 4);
            eraser.handle_event(&mut ctx, pointer([480.0, 300.0], "move"));
            eraser.handle_event(&mut ctx, pointer([480.0, 300.0], "release"));
        }
        assert_eq!(doc.scene.roots().len(), 2);
        for id in &original_ids {
            let wrapper = doc.scene.get(*id).and_then(|node| node.parent);
            assert!(wrapper.is_some_and(|wrapper| is_erased_mark(&doc.scene, wrapper)));
        }
        assert_eq!(doc.scene.len(), 6);
        assert!(doc.undo().expect("undo eraser gesture"));
        assert_eq!(doc.scene.len(), 2);
        for id in &original_ids {
            assert!(doc.scene.get(*id).is_some_and(|node| node.parent.is_none()));
        }
        assert!(doc.redo().expect("redo eraser gesture"));
        assert_eq!(doc.scene.len(), 6);
    }

    #[test]
    fn eraser_reaches_the_visible_edge_of_a_wide_pencil_stroke() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        ctx.new_stroke_width = 80.0;
        let mut pencil = PencilTool::new();
        pencil.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        pencil.handle_event(&mut ctx, pointer([440.0, 300.0], "release"));
        assert_eq!(ctx.doc.scene.len(), 1);

        ctx.new_stroke_width = 2.0;
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([420.0, 330.0], "press"));
        eraser.handle_event(&mut ctx, pointer([420.0, 330.0], "release"));
        assert_eq!(ctx.doc.scene.len(), 3);
        let wrapper = ctx.doc.scene.roots()[0];
        assert!(is_erased_mark(&ctx.doc.scene, wrapper));
        assert!(painted_at(&ctx.doc.scene, wrapper, DVec2::new(20.0, 0.0)));
        assert!(!painted_at(&ctx.doc.scene, wrapper, DVec2::new(20.0, 30.0)));
        eraser.handle_event(&mut ctx, pointer([600.0, 450.0], "press"));
        assert_eq!(eraser.max_stroke_radius, 0.0);
        eraser.handle_event(&mut ctx, pointer([600.0, 450.0], "release"));
        assert!(ctx.doc.undo().expect("undo erasure"));
        assert_eq!(ctx.doc.scene.len(), 1);
    }

    #[test]
    fn eraser_partially_subtracts_quadratic_and_cubic_stroked_vectors() {
        for cubic in [false, true] {
            let mut doc = Doc::new();
            let mut path = PathData::new();
            path.move_to(-50.0, 0.0);
            if cubic {
                path.cubic_to(-50.0, -66.666, 50.0, -66.666, 50.0, 0.0);
            } else {
                path.quad_to(0.0, -100.0, 50.0, 0.0);
            }
            let mark = CanvasNode::new(NodeData::Vector(VectorNode {
                path,
                strokes: [Stroke::solid(Color::rgba(0, 0, 0, 255), 8.0)]
                    .into_iter()
                    .collect(),
                ..VectorNode::default()
            }));
            doc.apply(Operation::create_node(mark))
                .expect("create curved vector mark");
            let mut viewport = Viewport::default();
            let mut ctx = ToolContext::new(
                &mut doc,
                &mut viewport,
                SnapEngine::default(),
                DVec2::new(800.0, 600.0),
            );
            let mut eraser = EraserTool::new();
            eraser.handle_event(&mut ctx, pointer([400.0, 250.0], "press"));
            eraser.handle_event(&mut ctx, pointer([400.0, 250.0], "release"));
            assert_eq!(ctx.doc.scene.len(), 3, "cubic={cubic}");
            assert!(is_erased_mark(&ctx.doc.scene, ctx.doc.scene.roots()[0]));
            assert!(ctx.doc.undo().expect("undo curved mark erasure"));
            assert_eq!(ctx.doc.scene.len(), 1, "cubic={cubic}");
        }
    }

    #[test]
    fn escape_restores_erased_mark() {
        let mut doc = Doc::new();
        let mut viewport = Viewport::default();
        let size = DVec2::new(800.0, 600.0);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
        let mut brush = BrushTool::new();
        brush.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        brush.handle_event(&mut ctx, pointer([450.0, 300.0], "release"));
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([425.0, 300.0], "press"));
        assert_eq!(ctx.doc.scene.len(), 3);
        eraser.handle_event(
            &mut ctx,
            ToolEvent::Key(KeyEvent::press(LogicalKey::Escape)),
        );
        assert_eq!(ctx.doc.scene.len(), 1);
    }

    #[test]
    fn eraser_keeps_marks_on_other_pages() {
        let mut doc = Doc::new();
        let first_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let second_page = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let first_page_id = first_page.id;
        let second_page_id = second_page.id;
        doc.apply(Operation::create_node(first_page))
            .expect("create first page");
        doc.apply(Operation::create_node(second_page))
            .expect("create second page");
        doc.add_page(first_page_id);
        doc.add_page(second_page_id);
        let mut viewport = Viewport::default();
        let size = DVec2::new(800.0, 600.0);
        let mut mark_ids = Vec::new();
        for page in [first_page_id, second_page_id] {
            doc.set_active_page(Some(page));
            let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
            let mut brush = BrushTool::new();
            brush.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
            brush.handle_event(&mut ctx, pointer([450.0, 300.0], "release"));
            mark_ids.push(ctx.doc.selection.as_slice()[0]);
        }
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, SnapEngine::default(), size);
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([425.0, 300.0], "press"));
        eraser.handle_event(&mut ctx, pointer([425.0, 300.0], "release"));
        assert!(ctx.doc.scene.get(mark_ids[0]).is_some());
        assert_eq!(
            ctx.doc.scene.get(mark_ids[0]).and_then(|node| node.parent),
            Some(first_page_id)
        );
        let erased_parent = ctx.doc.scene.get(mark_ids[1]).and_then(|node| node.parent);
        assert!(erased_parent.is_some_and(|parent| is_erased_mark(&ctx.doc.scene, parent)));
        assert_eq!(
            erased_parent.and_then(|parent| ctx.doc.scene.get(parent).and_then(|node| node.parent)),
            Some(second_page_id)
        );
    }

    #[test]
    fn eraser_leaves_filled_shapes_in_place() {
        let mut doc = Doc::new();
        let shape = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            -20.0,
            -20.0,
            40.0,
            40.0,
            Color::BLACK,
        )));
        let shape_id = shape.id;
        doc.apply(Operation::create_node(shape))
            .expect("create shape");
        let mut viewport = Viewport::default();
        let mut ctx = ToolContext::new(
            &mut doc,
            &mut viewport,
            SnapEngine::default(),
            DVec2::new(800.0, 600.0),
        );
        let mut eraser = EraserTool::new();
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "press"));
        eraser.handle_event(&mut ctx, pointer([400.0, 300.0], "release"));
        assert!(ctx.doc.scene.get(shape_id).is_some());
    }
}

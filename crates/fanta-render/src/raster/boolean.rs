use super::content::ContentPaintState;
use super::vector::{bounds_to_f32, paint_path_fills, stroke_sk_path};
use super::{
    ExpandedNode, HashMap, RenderCtx, paint_node_content, resolve_overlay, to_sk_fill_path,
    to_sk_matrix,
};
use fanta_doc::{
    BooleanNode, BooleanOp, Bounds, CanvasNode, NodeData, NodeFlags, NodeId, Scene, VectorNode,
};
use skia_safe::{Canvas, Path, PathOp};

pub(crate) enum BooleanShape {
    Baked(VectorNode, NodeFlags),
    Folded(Path),
    Unavailable,
}

impl BooleanShape {
    pub(crate) fn silhouette(&self) -> Option<Path> {
        match self {
            Self::Baked(vector, flags) => Some(vector_outline(vector, *flags)),
            Self::Folded(path) => Some(path.clone()),
            Self::Unavailable => Some(Path::new()),
        }
    }

    pub(crate) fn bounds(&self, node: &CanvasNode) -> Option<Bounds> {
        match self {
            Self::Baked(vector, flags) => crate::bounds::vector_visual_bounds(vector, *flags),
            Self::Folded(path) => Some(crate::bounds::expand_for_strokes(
                sk_path_bounds(path),
                &node.data.as_boolean()?.strokes,
                false,
            )),
            Self::Unavailable => None,
        }
    }

    pub(crate) fn paint(
        &self,
        canvas: &Canvas,
        node: &CanvasNode,
        ctx: &mut RenderCtx,
    ) -> ContentPaintState {
        match self {
            Self::Baked(vector, _) => {
                let mut painted = node.clone();
                painted.data = NodeData::Vector(vector.clone());
                paint_node_content(canvas, &painted, None, None, ctx)
            }
            Self::Folded(path) => {
                if let NodeData::Boolean(boolean) = &node.data {
                    let bounds = sk_path_bounds(path);
                    paint_path_fills(canvas, path, &boolean.fills, bounds, ctx);
                    stroke_sk_path(canvas, path, &boolean.strokes, bounds_to_f32(&bounds), ctx);
                }
                ContentPaintState::default()
            }
            Self::Unavailable => ContentPaintState::default(),
        }
    }
}

pub(crate) fn prepare_scene_boolean(node: &CanvasNode, ctx: &mut RenderCtx) -> BooleanShape {
    let scene = ctx.scene;
    let nodes = scene.descendants_of(node.id).filter_map(|id| scene.get(id));
    prepare_boolean(node, nodes, true, ctx)
}

pub(crate) fn prepare_expanded_boolean(
    node: &CanvasNode,
    expanded: &[ExpandedNode],
    children: &HashMap<NodeId, Vec<usize>>,
    ctx: &mut RenderCtx,
) -> BooleanShape {
    let mut nodes = vec![node];
    let mut pending: Vec<_> = children
        .get(&node.id)
        .into_iter()
        .flatten()
        .rev()
        .copied()
        .collect();
    while let Some(index) = pending.pop() {
        let Some(entry) = expanded.get(index) else {
            ctx.metrics.incomplete_artwork = true;
            return BooleanShape::Unavailable;
        };
        nodes.push(&entry.node);
        if let Some(descendants) = children.get(&entry.node.id) {
            pending.extend(descendants.iter().rev().copied());
        }
    }
    prepare_boolean(node, nodes.into_iter(), false, ctx)
}

fn prepare_boolean<'a>(
    node: &CanvasNode,
    nodes: impl Iterator<Item = &'a CanvasNode>,
    cache: bool,
    ctx: &mut RenderCtx,
) -> BooleanShape {
    let mut geometry = Scene::new();
    let mut resolved = Vec::new();
    for (order, source) in nodes.enumerate() {
        let mut current = if source.id == node.id {
            node.clone()
        } else {
            resolve_overlay(ctx, source.id, source)
                .as_deref()
                .unwrap_or(source)
                .clone()
        };
        if current.id == node.id {
            current.parent = None;
        }
        // Fresh clone IDs cannot break ties in authored sibling order.
        current.index = fanta_doc::IndexKey::from_raw(order as f64);
        resolved.push(current);
    }
    if geometry.insert_many(resolved).is_err() {
        ctx.metrics.incomplete_artwork = true;
        return BooleanShape::Unavailable;
    }
    let Some(boolean) = node.data.as_boolean() else {
        return BooleanShape::Unavailable;
    };
    let signature = match fanta_doc::boolean_geometry_signature(&geometry, node.id) {
        Ok(signature) => signature,
        Err(_) => {
            ctx.metrics.incomplete_artwork = true;
            return BooleanShape::Unavailable;
        }
    };
    if let Some(vector) = boolean.baked_vector(&signature) {
        return BooleanShape::Baked(vector, node.flags);
    }
    // A bake without source operands cannot be recomputed after its operation
    // changes. Never display the obsolete result as if the edit succeeded.
    if boolean.baked.is_some() && geometry.children_of(Some(node.id)).is_empty() {
        ctx.metrics.incomplete_artwork = true;
        return BooleanShape::Unavailable;
    }
    if cache
        && let Some((source, path)) = ctx.boolean_cache.entries.get(&node.id)
        && source == &signature
    {
        return BooleanShape::Folded(path.clone());
    }
    match fold_checked(&geometry, node.id, boolean.op, 0) {
        Ok(path) => {
            if cache {
                ctx.boolean_cache
                    .entries
                    .insert(node.id, (signature, path.clone()));
            }
            BooleanShape::Folded(path)
        }
        Err(()) => {
            ctx.metrics.incomplete_artwork = true;
            BooleanShape::Unavailable
        }
    }
}

pub(crate) fn paint_boolean(
    canvas: &Canvas,
    _: &BooleanNode,
    scene_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) {
    if let Some(node) = scene_id.and_then(|id| ctx.scene.get(id)) {
        prepare_scene_boolean(node, ctx).paint(canvas, node, ctx);
    } else {
        ctx.metrics.incomplete_artwork = true;
    }
}

fn sk_op(op: BooleanOp) -> PathOp {
    match op {
        BooleanOp::Union => PathOp::Union,
        BooleanOp::Subtract => PathOp::Difference,
        BooleanOp::Intersect => PathOp::Intersect,
        BooleanOp::Exclude => PathOp::XOR,
    }
}

pub(crate) fn fold_operands(scene: &Scene, id: NodeId, op: BooleanOp) -> Option<Path> {
    fold_checked(scene, id, op, 0).ok()
}

fn fold_checked(scene: &Scene, id: NodeId, op: BooleanOp, depth: usize) -> Result<Path, ()> {
    if depth > 128 {
        return Err(());
    }
    if let Some(node) = scene.get(id)
        && let Some(boolean) = node.data.as_boolean()
        && let Ok(signature) = fanta_doc::boolean_geometry_signature(scene, id)
        && let Some(vector) = boolean.baked_vector(&signature)
    {
        return Ok(vector_outline(&vector, node.flags));
    }
    let mut combined: Option<Path> = None;
    for &child in scene.children_of(Some(id)) {
        let Some(outline) = operand_outline(scene, child, depth + 1)? else {
            continue;
        };
        combined = Some(match combined {
            Some(previous) => previous.op(&outline, sk_op(op)).ok_or(())?,
            None => outline,
        });
    }
    Ok(combined.unwrap_or_default())
}

fn vector_outline(vector: &VectorNode, flags: NodeFlags) -> Path {
    let mut outline = if super::path_is_rect(&vector.path) {
        super::vector_outline_sk_path(
            &vector.path,
            vector.corner_radius,
            vector.corner_radii,
            vector.corner_smoothing,
        )
    } else {
        to_sk_fill_path(&vector.path)
    };
    if let Some([width, height]) =
        crate::bounds::vector_viewport_clip(vector, flags, || vector.path.rough_bounds())
    {
        let mut clip = Path::new();
        clip.add_rect(skia_safe::Rect::from_wh(width as f32, height as f32), None);
        if let Some(clipped) = outline.op(&clip, PathOp::Intersect) {
            outline = clipped;
        }
    }
    outline
}

fn operand_outline(scene: &Scene, id: NodeId, depth: usize) -> Result<Option<Path>, ()> {
    let node = scene.get(id).ok_or(())?;
    if node.flags.contains(NodeFlags::HIDDEN) {
        return Ok(None);
    }
    let mut local = match &node.data {
        NodeData::Vector(vector) => vector_outline(vector, node.flags),
        NodeData::Boolean(boolean) => fold_checked(scene, id, boolean.op, depth)?,
        NodeData::Group(group) => {
            let mut path = fold_checked(scene, id, BooleanOp::Union, depth)?;
            let box_path = group.clip_size.or(group.local_size).map(|[width, height]| {
                super::rounded_rect_path(
                    [0.0, 0.0, width as f32, height as f32],
                    group.corner_radius,
                    group.corner_radii,
                    group.corner_smoothing,
                )
            });
            if (group.background.is_some() || !group.background_fills.is_empty())
                && let Some(background) = &box_path
            {
                path = path.op(background, PathOp::Union).ok_or(())?;
            }
            if super::effects::group_clips_children(node, group)
                && let Some(clip) = box_path
            {
                path = path.op(&clip, PathOp::Intersect).ok_or(())?;
            }
            path
        }
        NodeData::Text(text) => to_sk_fill_path(&super::text_node_outline(text).ok_or(())?),
        NodeData::TextPath(text) => super::text_path_outline(text).into_exact().ok_or(())?,
        _ => return Err(()),
    };
    local.transform(&to_sk_matrix(&node.transform));
    Ok(Some(local))
}

fn sk_path_bounds(path: &Path) -> Bounds {
    let rect = path.compute_tight_bounds();
    Bounds::from_xywh(
        rect.left as f64,
        rect.top as f64,
        rect.width() as f64,
        rect.height() as f64,
    )
}

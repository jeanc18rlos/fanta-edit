//! Rendering for [`NodeData::Boolean`](fanta_doc::NodeData::Boolean).
//!
//! A boolean-operation node has no geometry of its own: its shape is the fold of
//! its child *operands* under the node's [`BooleanOp`]. We materialize each
//! operand's outline (a [`VectorNode`](fanta_doc::VectorNode) path, a nested
//! boolean's fold, or a group's unioned children), lift it into the boolean
//! node's local space by the operand's transform, and fold the stack with Skia
//! path-ops. The resulting path is painted with the boolean node's own fills and
//! strokes, exactly like a vector.
//!
//! Every path-op returns `Option` and a `None` (degenerate geometry) keeps the
//! accumulator rather than dropping the shape — folding never panics.

use skia_safe::{Canvas, Path, PathOp};

use fanta_doc::{BooleanNode, BooleanOp, Bounds, NodeData, NodeFlags, NodeId, Scene};

use super::vector::{bounds_to_f32, paint_path_fills, stroke_sk_path};
use super::{RenderCtx, to_sk_matrix, to_sk_path};

/// Paint a boolean-operation node: fold its operands and draw the node's fills /
/// strokes on the result. A boolean with no resolvable operands paints nothing.
pub(crate) fn paint_boolean(
    canvas: &Canvas,
    node_data: &BooleanNode,
    scene_id: Option<NodeId>,
    ctx: &mut RenderCtx,
) {
    // Folding walks the scene for the operand subtree, so a transient (instance)
    // clone with no scene id cannot be folded — nothing to draw.
    let Some(scene_id) = scene_id else {
        return;
    };

    // Top-level cache lookup, tagged with the subtree's geometry stamp — the
    // fold bakes operand geometry, order, transforms, and visibility, all of
    // which move the subtree max when they change.
    let stamp = ctx.scene.subtree_stamp(scene_id);
    if let Some(cached) = ctx
        .boolean_cache
        .entries
        .get(&scene_id)
        .filter(|(tag, _)| *tag == stamp)
        .map(|(_, path)| path.clone())
    {
        let bounds = sk_path_bounds(&cached);
        paint_path_fills(canvas, &cached, &node_data.fills, bounds, ctx);
        stroke_sk_path(
            canvas,
            &cached,
            &node_data.strokes,
            bounds_to_f32(&bounds),
            ctx,
        );
        return;
    }

    let Some(folded) = fold_operands(ctx.scene, scene_id, node_data.op) else {
        return;
    };

    // Cache the result for subsequent frames until the subtree changes.
    ctx.boolean_cache
        .entries
        .insert(scene_id, (stamp, folded.clone()));

    let bounds = sk_path_bounds(&folded);
    paint_path_fills(canvas, &folded, &node_data.fills, bounds, ctx);
    stroke_sk_path(
        canvas,
        &folded,
        &node_data.strokes,
        bounds_to_f32(&bounds),
        ctx,
    );
}

/// The Skia path-op for a [`BooleanOp`]. `Subtract` folds as successive
/// differences (first operand minus each of the rest ⇒ first minus their union);
/// `Exclude` is the symmetric difference (XOR).
fn sk_op(op: BooleanOp) -> PathOp {
    match op {
        BooleanOp::Union => PathOp::Union,
        BooleanOp::Subtract => PathOp::Difference,
        BooleanOp::Intersect => PathOp::Intersect,
        BooleanOp::Exclude => PathOp::XOR,
    }
}

/// Fold the visible operands of `boolean_id` under `op`, in the boolean node's
/// local space. `None` when no operand yields geometry.
pub(crate) fn fold_operands(scene: &Scene, boolean_id: NodeId, op: BooleanOp) -> Option<Path> {
    let mut outlines = scene
        .children_of(Some(boolean_id))
        .iter()
        .filter_map(|child| operand_outline(scene, *child));
    let mut acc = outlines.next()?;
    let path_op = sk_op(op);
    for next in outlines {
        if let Some(folded) = acc.op(&next, path_op) {
            acc = folded;
        }
        // A degenerate op leaves `acc` as-is (approximate coverage beats a
        // dropped shape), matching `to_sk_fill_path`'s fallback.
    }
    Some(acc)
}

/// One operand's outline, expressed in its **parent's** local space (its own
/// geometry lifted by its transform). Hidden operands and non-geometric variants
/// contribute nothing.
fn operand_outline(scene: &Scene, id: NodeId) -> Option<Path> {
    let node = scene.get(id)?;
    if node.flags.contains(NodeFlags::HIDDEN) {
        return None;
    }
    let local = match &node.data {
        NodeData::Vector(v) => to_sk_path(&v.path),
        NodeData::Boolean(b) => fold_operands(scene, id, b.op)?,
        NodeData::Group(_) => union_children(scene, id)?,
        // Text/bitmap/etc. have no vector outline to fold.
        _ => return None,
    };
    let mut lifted = local;
    lifted.transform(&to_sk_matrix(&node.transform));
    Some(lifted)
}

/// Union of every child operand's outline (each already in the group's local
/// space), so a group used as a boolean operand acts as its combined contents.
fn union_children(scene: &Scene, group_id: NodeId) -> Option<Path> {
    let mut outlines = scene
        .children_of(Some(group_id))
        .iter()
        .filter_map(|child| operand_outline(scene, *child));
    let mut acc = outlines.next()?;
    for next in outlines {
        if let Some(unioned) = acc.op(&next, PathOp::Union) {
            acc = unioned;
        }
    }
    Some(acc)
}

/// Tight bounds of a Skia path as a doc [`Bounds`] (empty ⇒ zero).
fn sk_path_bounds(path: &Path) -> Bounds {
    let r = path.compute_tight_bounds();
    Bounds::from_xywh(
        r.left as f64,
        r.top as f64,
        r.width() as f64,
        r.height() as f64,
    )
}

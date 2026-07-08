//! Align and distribute helpers.
//!
//! Each function takes a selection (a slice of [`NodeId`]) plus the scene
//! and returns a vector of [`Operation`]s — typically several
//! [`Operation::SetTransform`] — that the caller bundles into a single
//! [`Transaction`] for a one-step undo (Figma / Sketch convention).
//!
//! ## Behavioral model
//!
//! Alignment is in world space. The "target" coordinate is derived from the
//! selection bounds (the union of every selected node's world bounds). This
//! matches Figma's "Align Left" — it aligns to the left edge of the selection
//! box, not to canvas zero. For "Align to Page" or "Align to Last Selected,"
//! callers can pre-compute the target [`Bounds`] and call
//! [`align_to_bounds`] directly.
//!
//! Distribution requires at least three nodes by definition — with two nodes
//! they're already as far apart as possible. We follow Figma's "equal gaps"
//! interpretation: the gap between consecutive bounds along the axis is
//! made uniform, keeping the outermost nodes fixed.
//!
//! [`Transaction`]: fanta_doc::Transaction

use fanta_doc::{NodeId, Operation, Scene, Transform2D};
use glam::DVec2;

// =============================================================================
// Alignment axis enums
// =============================================================================

/// Horizontal alignment edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HAlign {
    Left,
    Center,
    Right,
}

/// Vertical alignment edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

/// Distribution axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
}

// =============================================================================
// Align — by selection bounds
// =============================================================================

/// Align the horizontal edge of each selected node to the selection's
/// corresponding edge.
pub fn align_horizontal(scene: &Scene, ids: &[NodeId], edge: HAlign) -> Vec<Operation> {
    let Some(selection_bounds) = selection_bounds(scene, ids) else {
        return Vec::new();
    };
    align_to_bounds_h(scene, ids, selection_bounds, edge)
}

/// Align the vertical edge of each selected node to the selection's
/// corresponding edge.
pub fn align_vertical(scene: &Scene, ids: &[NodeId], edge: VAlign) -> Vec<Operation> {
    let Some(selection_bounds) = selection_bounds(scene, ids) else {
        return Vec::new();
    };
    align_to_bounds_v(scene, ids, selection_bounds, edge)
}

/// Align by horizontal edge against an arbitrary target rectangle. Useful for
/// "Align to Frame" or "Align to Page" — the caller decides the target.
pub fn align_to_bounds_h(
    scene: &Scene,
    ids: &[NodeId],
    target: fanta_doc::Bounds,
    edge: HAlign,
) -> Vec<Operation> {
    let mut ops = Vec::with_capacity(ids.len());
    for &id in ids {
        let Some(bb) = scene.world_bounds(id) else {
            continue;
        };
        let target_x = match edge {
            HAlign::Left => target.min_x,
            HAlign::Center => target.center().x,
            HAlign::Right => target.max_x,
        };
        let current_x = match edge {
            HAlign::Left => bb.min_x,
            HAlign::Center => bb.center().x,
            HAlign::Right => bb.max_x,
        };
        let dx = target_x - current_x;
        if dx == 0.0 {
            continue;
        }
        if let Some(op) = translate_op(scene, id, DVec2::new(dx, 0.0)) {
            ops.push(op);
        }
    }
    ops
}

/// Align by vertical edge against an arbitrary target rectangle.
pub fn align_to_bounds_v(
    scene: &Scene,
    ids: &[NodeId],
    target: fanta_doc::Bounds,
    edge: VAlign,
) -> Vec<Operation> {
    let mut ops = Vec::with_capacity(ids.len());
    for &id in ids {
        let Some(bb) = scene.world_bounds(id) else {
            continue;
        };
        let target_y = match edge {
            VAlign::Top => target.min_y,
            VAlign::Middle => target.center().y,
            VAlign::Bottom => target.max_y,
        };
        let current_y = match edge {
            VAlign::Top => bb.min_y,
            VAlign::Middle => bb.center().y,
            VAlign::Bottom => bb.max_y,
        };
        let dy = target_y - current_y;
        if dy == 0.0 {
            continue;
        }
        if let Some(op) = translate_op(scene, id, DVec2::new(0.0, dy)) {
            ops.push(op);
        }
    }
    ops
}

// =============================================================================
// Distribute
// =============================================================================

/// Distribute nodes with equal gaps along `axis`. Outermost nodes stay fixed;
/// the in-between nodes are repositioned so the gap (or overlap) between
/// consecutive bounds is uniform.
///
/// Requires `ids.len() >= 3`; returns empty for shorter selections.
pub fn distribute(scene: &Scene, ids: &[NodeId], axis: Axis) -> Vec<Operation> {
    if ids.len() < 3 {
        return Vec::new();
    }

    // Sort by center on the chosen axis. We keep a parallel vec of bounds for
    // the pass so we don't re-resolve them.
    let mut bounded: Vec<(NodeId, fanta_doc::Bounds)> = ids
        .iter()
        .filter_map(|id| scene.world_bounds(*id).map(|bb| (*id, bb)))
        .collect();
    if bounded.len() < 3 {
        return Vec::new();
    }
    bounded.sort_by(|a, b| match axis {
        Axis::X => a.1.center().x.total_cmp(&b.1.center().x),
        Axis::Y => a.1.center().y.total_cmp(&b.1.center().y),
    });

    // Total span and total summed widths/heights — gives the uniform gap.
    let (first_min, last_max) = match axis {
        Axis::X => (bounded[0].1.min_x, bounded.last().unwrap().1.max_x),
        Axis::Y => (bounded[0].1.min_y, bounded.last().unwrap().1.max_y),
    };
    let span = last_max - first_min;
    let total_size: f64 = bounded.iter().fold(0.0, |acc, (_, bb)| {
        acc + match axis {
            Axis::X => bb.width(),
            Axis::Y => bb.height(),
        }
    });
    let gap = (span - total_size) / (bounded.len() as f64 - 1.0);

    let mut ops = Vec::with_capacity(bounded.len() - 2);
    let mut cursor = first_min;
    for (i, (id, bb)) in bounded.iter().enumerate() {
        if i == 0 || i == bounded.len() - 1 {
            // First and last stay anchored; advance the cursor past them.
            cursor = match axis {
                Axis::X => bb.max_x + gap,
                Axis::Y => bb.max_y + gap,
            };
            continue;
        }
        let want_min = cursor;
        let current_min = match axis {
            Axis::X => bb.min_x,
            Axis::Y => bb.min_y,
        };
        let delta = want_min - current_min;
        let v = match axis {
            Axis::X => DVec2::new(delta, 0.0),
            Axis::Y => DVec2::new(0.0, delta),
        };
        if delta != 0.0 {
            if let Some(op) = translate_op(scene, *id, v) {
                ops.push(op);
            }
        }
        cursor = match axis {
            Axis::X => bb.max_x + delta + gap,
            Axis::Y => bb.max_y + delta + gap,
        };
    }
    ops
}

// =============================================================================
// Internals
// =============================================================================

fn selection_bounds(scene: &Scene, ids: &[NodeId]) -> Option<fanta_doc::Bounds> {
    let mut acc: Option<fanta_doc::Bounds> = None;
    for &id in ids {
        if let Some(bb) = scene.world_bounds(id) {
            acc = Some(match acc {
                Some(a) => a.union(&bb),
                None => bb,
            });
        }
    }
    acc
}

/// Build a `SetTransform` op that translates `id` by `delta` in world space.
///
/// Translating in world space requires accounting for the node's parent
/// transform — a child's local transform is composed with the parent's, so a
/// world-space delta has to be expressed in the parent's local frame. For
/// root-level nodes the parent transform is identity and world == local.
fn translate_op(scene: &Scene, id: NodeId, world_delta: DVec2) -> Option<Operation> {
    let node = scene.get(id)?;
    let old_local = node.transform;

    // Parent's world transform (identity if no parent).
    let parent_world = match node.parent {
        Some(p) => scene.world_transform(p).unwrap_or(Transform2D::IDENTITY),
        None => Transform2D::IDENTITY,
    };

    // Express the world delta in the parent's local frame by applying the
    // inverse rotation/scale of the parent. (Translations don't move under
    // a vector — only the linear part matters.)
    let local_delta = parent_world.inverse().transform_vector(world_delta);

    let new_local = old_local.then(&Transform2D::translation(local_delta.x, local_delta.y));
    Some(Operation::SetTransform {
        id,
        old: old_local,
        new: new_local,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Color, Doc, NodeData, Operation, Transform2D, VectorNode};

    fn doc_with_rects(rects: &[(f64, f64, f64, f64)]) -> (Doc, Vec<NodeId>) {
        let mut doc = Doc::new();
        let mut ids = Vec::with_capacity(rects.len());
        for &(x, y, w, h) in rects {
            let n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
                x,
                y,
                w,
                h,
                Color::WHITE,
            )));
            ids.push(n.id);
            doc.apply(Operation::create_node(n)).unwrap();
        }
        (doc, ids)
    }

    fn apply_all(doc: &mut Doc, ops: Vec<Operation>) {
        for op in ops {
            doc.apply(op).unwrap();
        }
    }

    // ---- alignment ----------------------------------------------------------

    #[test]
    fn align_left_moves_everything_to_minimum_x() {
        let (mut doc, ids) = doc_with_rects(&[
            (0.0, 0.0, 10.0, 10.0),
            (20.0, 0.0, 10.0, 10.0),
            (50.0, 0.0, 10.0, 10.0),
        ]);
        let ops = align_horizontal(&doc.scene, &ids, HAlign::Left);
        apply_all(&mut doc, ops);
        for &id in &ids {
            assert!((doc.scene.world_bounds(id).unwrap().min_x - 0.0).abs() < 1e-9);
        }
    }

    #[test]
    fn align_right_moves_everything_to_maximum_x() {
        let (mut doc, ids) = doc_with_rects(&[
            (0.0, 0.0, 10.0, 10.0),
            (20.0, 0.0, 10.0, 10.0),
            (50.0, 0.0, 10.0, 10.0),
        ]);
        let ops = align_horizontal(&doc.scene, &ids, HAlign::Right);
        apply_all(&mut doc, ops);
        for &id in &ids {
            assert!((doc.scene.world_bounds(id).unwrap().max_x - 60.0).abs() < 1e-9);
        }
    }

    #[test]
    fn align_center_lines_up_centers() {
        let (mut doc, ids) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0), (20.0, 0.0, 30.0, 10.0)]);
        let ops = align_horizontal(&doc.scene, &ids, HAlign::Center);
        apply_all(&mut doc, ops);
        let cx_a = doc.scene.world_bounds(ids[0]).unwrap().center().x;
        let cx_b = doc.scene.world_bounds(ids[1]).unwrap().center().x;
        assert!((cx_a - cx_b).abs() < 1e-9);
    }

    #[test]
    fn align_vertical_top_works_too() {
        let (mut doc, ids) = doc_with_rects(&[(0.0, 10.0, 10.0, 10.0), (20.0, 50.0, 10.0, 10.0)]);
        let ops = align_vertical(&doc.scene, &ids, VAlign::Top);
        apply_all(&mut doc, ops);
        for &id in &ids {
            assert!((doc.scene.world_bounds(id).unwrap().min_y - 10.0).abs() < 1e-9);
        }
    }

    // ---- align respects existing transforms --------------------------------

    #[test]
    fn align_left_under_pre_existing_translation_uses_world_bounds() {
        let mut doc = Doc::new();
        let a = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        let a_id = a.id;
        doc.apply(Operation::create_node(a)).unwrap();
        let mut b = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            10.0,
            10.0,
            Color::WHITE,
        )));
        b.transform = Transform2D::translation(50.0, 0.0);
        let b_id = b.id;
        doc.apply(Operation::create_node(b)).unwrap();
        let ops = align_horizontal(&doc.scene, &[a_id, b_id], HAlign::Left);
        apply_all(&mut doc, ops);
        assert!((doc.scene.world_bounds(a_id).unwrap().min_x - 0.0).abs() < 1e-9);
        assert!((doc.scene.world_bounds(b_id).unwrap().min_x - 0.0).abs() < 1e-9);
    }

    // ---- distribute ---------------------------------------------------------

    #[test]
    fn distribute_x_equalizes_gaps_between_three_rects() {
        // Outer rects at min_x=0 and min_x=100. Middle rect in the middle.
        // Each rect is 10 wide → 3*10 = 30 size. Total span 110 → gaps = (110-30)/2 = 40.
        let (mut doc, ids) = doc_with_rects(&[
            (0.0, 0.0, 10.0, 10.0),
            (50.0, 0.0, 10.0, 10.0),
            (100.0, 0.0, 10.0, 10.0),
        ]);
        let ops = distribute(&doc.scene, &ids, Axis::X);
        apply_all(&mut doc, ops);
        // Expect middle rect's min_x to be 0 + 10 + 40 = 50.
        let middle_min_x = doc.scene.world_bounds(ids[1]).unwrap().min_x;
        assert!(
            (middle_min_x - 50.0).abs() < 1e-9,
            "middle min_x = {middle_min_x}"
        );
    }

    #[test]
    fn distribute_y_works_too() {
        let (mut doc, ids) = doc_with_rects(&[
            (0.0, 0.0, 10.0, 10.0),
            (0.0, 70.0, 10.0, 10.0),
            (0.0, 100.0, 10.0, 10.0),
        ]);
        let ops = distribute(&doc.scene, &ids, Axis::Y);
        apply_all(&mut doc, ops);
        let a_top = doc.scene.world_bounds(ids[0]).unwrap().max_y;
        let b_top = doc.scene.world_bounds(ids[1]).unwrap().min_y;
        let b_bot = doc.scene.world_bounds(ids[1]).unwrap().max_y;
        let c_top = doc.scene.world_bounds(ids[2]).unwrap().min_y;
        let gap_a_b = b_top - a_top;
        let gap_b_c = c_top - b_bot;
        assert!((gap_a_b - gap_b_c).abs() < 1e-9, "gaps {gap_a_b} {gap_b_c}");
    }

    #[test]
    fn distribute_under_three_returns_no_ops() {
        let (doc, ids) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0), (50.0, 0.0, 10.0, 10.0)]);
        assert!(distribute(&doc.scene, &ids, Axis::X).is_empty());
        let (doc1, _) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0)]);
        assert!(distribute(&doc1.scene, &[], Axis::X).is_empty());
    }

    #[test]
    fn distribute_handles_unordered_input() {
        // Provide the IDs in non-sorted x order — distribute should reorder.
        let (mut doc, ids) = doc_with_rects(&[
            (100.0, 0.0, 10.0, 10.0),
            (0.0, 0.0, 10.0, 10.0),
            (50.0, 0.0, 10.0, 10.0),
        ]);
        let ops = distribute(&doc.scene, &ids, Axis::X);
        apply_all(&mut doc, ops);
        // Outer (id 0 at 100, id 1 at 0) untouched; middle (id 2 at 50) now at 50.
        assert!((doc.scene.world_bounds(ids[2]).unwrap().min_x - 50.0).abs() < 1e-9);
    }

    // ---- align_to_bounds variant --------------------------------------------

    #[test]
    fn align_to_bounds_uses_caller_provided_target() {
        let (mut doc, ids) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0), (20.0, 0.0, 10.0, 10.0)]);
        // Align everything to the right edge of a target frame at x=200.
        let target = fanta_doc::Bounds::from_xywh(100.0, 0.0, 100.0, 100.0);
        let ops = align_to_bounds_h(&doc.scene, &ids, target, HAlign::Right);
        apply_all(&mut doc, ops);
        for &id in &ids {
            assert!((doc.scene.world_bounds(id).unwrap().max_x - 200.0).abs() < 1e-9);
        }
    }

    // ---- no-op detection ----------------------------------------------------

    #[test]
    fn align_returns_no_ops_when_already_aligned() {
        let (doc, ids) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0), (0.0, 20.0, 10.0, 10.0)]);
        let ops = align_horizontal(&doc.scene, &ids, HAlign::Left);
        assert!(ops.is_empty(), "should not emit ops when already aligned");
    }

    #[test]
    fn align_missing_node_is_skipped_silently() {
        let (doc, mut ids) = doc_with_rects(&[(0.0, 0.0, 10.0, 10.0)]);
        ids.push(fanta_doc::NodeId::new()); // bogus, not in scene
        let ops = align_horizontal(&doc.scene, &ids, HAlign::Left);
        // The valid node is already at min_x=0 (selection min_x); 0 delta → no op.
        // The invalid id contributes nothing. Result: empty ops, no panic.
        assert!(ops.is_empty());
    }
}

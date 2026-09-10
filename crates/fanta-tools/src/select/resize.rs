//! The resize gesture: corner/edge-handle drag of a single-node selection.
//!
//! Like the move gesture, the drag is a transient preview — each frame writes
//! the computed transform straight into the scene (no open transaction) and the
//! single `SetTransform { old, new }` is recorded on release. The math runs in
//! the node's own (possibly rotated) frame so an existing rotation survives the
//! resize.

use super::state::{Phase, ResizeChildTransform, ResizingState, SelectTool};
use crate::context::ToolContext;
use crate::event::ModifierKeys;
use crate::tool::{CursorHint, ToolResponse};
use fanta_canvas::{ResizeHandle, SnapResult};
use fanta_doc::{Bounds, NodeData, NodeId, Operation, TextAutoResize};
use glam::DVec2;

/// The authored box used by resize handles. Non-clipping groups may have
/// descendants outside that box; their visual/culling bounds intentionally
/// union the overflow, while authoring handles remain attached to the box.
pub(super) fn authored_resize_bounds(ctx: &ToolContext, id: NodeId) -> Option<Bounds> {
    let node = ctx.doc.scene.get(id)?;
    match &node.data {
        NodeData::Group(group) => group
            .clip_size
            .or(group.local_size)
            .map(|[width, height]| Bounds::from_xywh(0.0, 0.0, width, height))
            .or_else(|| ctx.interaction_bounds(id)),
        _ => ctx.interaction_bounds(id),
    }
}

impl SelectTool {
    pub(super) fn begin_resize(
        &mut self,
        ctx: &mut ToolContext,
        node_id: NodeId,
        handle: ResizeHandle,
    ) -> ToolResponse {
        let Some(node) = ctx.doc.scene.get(node_id) else {
            return ToolResponse::empty();
        };
        // Local bounds are required for the transform math. If the node
        // doesn't expose any (e.g. an empty group), fall back to gracefully
        // doing nothing — the press becomes a no-op.
        let Some(scene_local) = ctx.doc.scene.local_bounds(node_id) else {
            return ToolResponse::empty();
        };
        let original_transform = node.transform;
        let scene_world_transform = ctx
            .doc
            .scene
            .world_transform(node_id)
            .unwrap_or(original_transform);
        let parent_world_transform = node
            .parent
            .and_then(|parent| ctx.doc.scene.world_transform(parent))
            .unwrap_or(fanta_doc::Transform2D::IDENTITY);
        let parent_determinant = parent_world_transform.0.matrix2.determinant();
        if !parent_determinant.is_finite() || parent_determinant.abs() <= f64::EPSILON {
            return ToolResponse::empty();
        }
        let parent_world_inverse = parent_world_transform.inverse();
        let legacy_group_origin = match &node.data {
            NodeData::Group(group) if group.clip_size.is_none() && group.local_size.is_none() => {
                Some(DVec2::new(scene_local.min_x, scene_local.min_y))
            }
            _ => None,
        };
        let original_local = if legacy_group_origin.is_some() {
            Bounds::from_xywh(0.0, 0.0, scene_local.width(), scene_local.height())
        } else {
            authored_resize_bounds(ctx, node_id).unwrap_or(scene_local)
        };
        let original_world_transform =
            legacy_group_origin.map_or(scene_world_transform, |origin| {
                fanta_doc::Transform2D::translation(origin.x, origin.y).then(&scene_world_transform)
            });
        let normalized_children = legacy_group_origin
            .map(|origin| {
                let inverse_origin = fanta_doc::Transform2D::translation(-origin.x, -origin.y);
                ctx.doc
                    .scene
                    .children_of(Some(node_id))
                    .iter()
                    .filter_map(|child_id| {
                        let child = ctx.doc.scene.get(*child_id)?;
                        Some(ResizeChildTransform {
                            id: *child_id,
                            original: child.transform,
                            normalized: child.transform.then(&inverse_origin),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let original_size = match &node.data {
            NodeData::Group(g) => g
                .clip_size
                .or(g.local_size)
                .unwrap_or([original_local.width(), original_local.height()]),
            NodeData::Vector(v) => v
                .local_size
                .unwrap_or([original_local.width(), original_local.height()]),
            _ => [original_local.width(), original_local.height()],
        };
        // Text and containers resize their BOX, not their transform scale: a
        // scaled text transform stretches glyphs, and a scaled group transform
        // stretches every descendant. Capture press-time data so preview can
        // rewrite the box and commit it beside the translation-only transform.
        let box_data = match &node.data {
            NodeData::Text(_) | NodeData::Group(_) => Some(Box::new(node.data.clone())),
            _ => None,
        };
        // Collect snap candidates once for the gesture, excluding the resized
        // node so it can't snap to itself. The drag is transient (see below),
        // so we do NOT open a history transaction here — that happens once on
        // release in `on_release`.
        let snap_candidates = ctx.snap.collect_candidates(&ctx.doc.scene, &[node_id]);
        let state = ResizingState {
            handle,
            node_id,
            original_local,
            original_transform,
            original_world_transform,
            parent_world_inverse,
            original_size,
            snap_candidates,
            box_data,
            normalized_children,
        };
        self.phase = Phase::Resizing(Box::new(state));
        ToolResponse::cursor(CursorHint::Move)
    }

    pub(super) fn compute_resize_response(
        &self,
        ctx: &mut ToolContext,
        state: &ResizingState,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let cursor_world = fanta_canvas::screen_to_world(screen, ctx.viewport, ctx.screen_size);
        let lock_aspect = modifiers.contains(ModifierKeys::SHIFT);
        let from_center = modifiers.contains(ModifierKeys::ALT);

        // Snap is intentionally OFF during resize in v0: `SnapEngine::snap_
        // bounds` returns a delta meant to shift the whole bounds, not a
        // per-edge candidate, and adapting it for "snap only the dragged edge"
        // needs a dedicated query that belongs in `fanta-canvas`. Adding it
        // lands with handle snap-to-edges, planned alongside multi-node resize.
        let snap = SnapResult::default();

        // Box-resize (text): grow/shrink the content box and reflow, instead of
        // baking a scale into the transform that would stretch the glyphs. The
        // helper returns the rotation-preserving transform (no scale) plus the
        // new box `[w, h]`.
        if state.box_data.is_some() {
            let (new_world_transform, w, h) = fanta_canvas::resize_box_keep_rotation(
                state.original_world_transform,
                state.original_local,
                state.handle,
                cursor_world,
                lock_aspect,
                from_center,
            );
            let new_transform = new_world_transform.then(&state.parent_world_inverse);
            for child in &state.normalized_children {
                if let Some(node) = ctx.doc.scene.get_mut(child.id) {
                    node.transform = child.normalized;
                }
            }
            if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
                n.transform = new_transform;
                apply_box_size(&mut n.data, w, h, state.handle);
            }
            let mut response = ToolResponse::cursor(CursorHint::Move);
            for o in Self::snap_overlays(&snap) {
                response.overlays.push(o);
            }
            return response;
        }

        // Compute the new transform in the node's OWN (possibly rotated) frame
        // so an existing rotation is preserved across the resize — a rotated
        // node resizes about its rotated axes and keeps its angle, instead of
        // snapping upright (the prior v0 limitation). For an unrotated node this
        // reduces to the same scale+translate the old path produced. The helper
        // projects the cursor into the node's un-rotated frame internally, pins
        // the dragged handle's world anchor, and clamps to a minimum extent in
        // that frame (the geometrically correct place to guard against a
        // zero/negative size).
        let new_world_transform = fanta_canvas::resize_transform_keep_rotation(
            state.original_world_transform,
            state.original_local,
            state.handle,
            cursor_world,
            lock_aspect,
            from_center,
        );
        let new_transform = new_world_transform.then(&state.parent_world_inverse);

        // Transient preview: write the transform straight into the scene, NOT
        // through `ctx.doc.apply`. A long resize would otherwise pile thousands
        // of `SetTransform` ops onto the open transaction. The committed state
        // still goes through history — `on_release` records one
        // `SetTransform { old: original, new: final }` — so the H1 invariant
        // (committed state is undoable) holds while the drag stays history-free.
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = new_transform;
        }
        let mut response = ToolResponse::cursor(CursorHint::Move);
        for o in Self::snap_overlays(&snap) {
            response.overlays.push(o);
        }
        response
    }

    /// Finalize a resize: same strategy as [`SelectTool::commit_move`] but for
    /// the single resized node — the scene holds the final preview transform;
    /// record one `SetTransform { old: original, new: final }`.
    ///
    /// [`SelectTool::commit_move`]: super::SelectTool::commit_move
    pub(super) fn commit_resize(&self, ctx: &mut ToolContext, state: &ResizingState) {
        let Some(new) = ctx.doc.scene.get(state.node_id).map(|n| n.transform) else {
            return;
        };
        // The box-resize path also mutated the node's variant data (the text box
        // `local_size`); record that as a `ReplaceData` next to the transform op
        // so the whole gesture is one undo step. Read the final data before the
        // reset-and-replay below.
        let final_data = state
            .box_data
            .is_some()
            .then(|| ctx.doc.scene.get(state.node_id).map(|n| n.data.clone()))
            .flatten();
        let data_changed =
            matches!((&state.box_data, &final_data), (Some(old), Some(new)) if old.as_ref() != new);
        if new == state.original_transform && !data_changed {
            return;
        }
        ctx.doc.history.begin("Resize", &mut ctx.doc.scene);
        // Reset both transform and data to their press-time values so the ops'
        // `apply` drives them back to `new` through the history chokepoint.
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = state.original_transform;
            if let Some(old) = &state.box_data {
                n.data = old.as_ref().clone();
            }
        }
        for child in &state.normalized_children {
            if let Some(node) = ctx.doc.scene.get_mut(child.id) {
                node.transform = child.original;
            }
        }
        if new != state.original_transform {
            if let Err(e) = ctx.doc.apply(Operation::SetTransform {
                id: state.node_id,
                old: state.original_transform,
                new,
            }) {
                tracing::warn!(target: "fanta-tools.select", "resize commit op failed: {e}");
            }
        }
        if let (true, Some(old), Some(new_data)) = (data_changed, &state.box_data, final_data) {
            if let Err(e) = ctx.doc.apply(Operation::ReplaceData {
                id: state.node_id,
                old: old.clone(),
                new: Box::new(new_data),
            }) {
                tracing::warn!(target: "fanta-tools.select", "resize data op failed: {e}");
            }
        }
        for child in &state.normalized_children {
            if child.original == child.normalized {
                continue;
            }
            if let Err(error) = ctx.doc.apply(Operation::SetTransform {
                id: child.id,
                old: child.original,
                new: child.normalized,
            }) {
                let rollback = ctx.doc.abort_transaction();
                tracing::warn!(
                    target: "fanta-tools.select",
                    "materializing legacy group bounds failed: {error}; rollback: {rollback:?}"
                );
                return;
            }
        }
        // If the resized node is (or was) a container, apply Figma-style constraints
        // to its children in the SAME transaction. Keeping these operations
        // before commit makes the parent box and all responsive child moves one
        // undo step instead of one step per constrained child.
        if ctx.doc.scene.get(state.node_id).is_some_and(|node| {
            node.can_have_children()
                && !matches!(&node.data, NodeData::Group(group) if group.auto_layout.is_some())
        }) {
            let old_size = state.original_size;
            let new_size =
                ctx.doc
                    .scene
                    .get(state.node_id)
                    .map_or(old_size, |node| match &node.data {
                        NodeData::Group(group) => {
                            group.clip_size.or(group.local_size).unwrap_or(old_size)
                        }
                        _ => ctx
                            .doc
                            .scene
                            .local_bounds(state.node_id)
                            .map(|bounds| [bounds.width(), bounds.height()])
                            .unwrap_or(old_size),
                    });
            if old_size != new_size {
                if let Err(error) =
                    ctx.doc
                        .apply_constraints_for_parent_resize(state.node_id, old_size, new_size)
                {
                    let rollback = ctx.doc.abort_transaction();
                    tracing::warn!(
                        target: "fanta-tools.select",
                        "apply_constraints after resize failed: {error}; rollback: {rollback:?}"
                    );
                    return;
                }
            }
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
    }

    /// Abort a resize with no transaction: restore the node's press-time
    /// transform (and box data, for the text box path) directly. Used on Esc and
    /// tool deactivation.
    pub(super) fn restore_resize(ctx: &mut ToolContext, state: &ResizingState) {
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = state.original_transform;
            if let Some(old) = &state.box_data {
                n.data = old.as_ref().clone();
            }
        }
        for child in &state.normalized_children {
            if let Some(node) = ctx.doc.scene.get_mut(child.id) {
                node.transform = child.original;
            }
        }
    }
}

/// Write a text or group node's new box during a box resize. A group updates
/// `clip_size` (frame) or its non-clipping `local_size` (plain group), leaving
/// descendant transforms untouched. Text also pins its resize mode so the new
/// dimensions actually take effect:
/// a horizontal-only drag keeps the box auto-height (fixed width → reflow, height
/// follows the text); any vertical drag fixes the box (Figma `NONE`) so the
/// dragged height is honored. Auto-width text (which never wraps) is switched to
/// a wrapping mode so the new width reflows instead of being ignored. Other
/// variants are left untouched.
fn apply_box_size(data: &mut NodeData, w: f64, h: f64, handle: ResizeHandle) {
    match data {
        NodeData::Text(text) => {
            text.local_size = [w, h];
            text.auto_resize = if handle.affects_y() {
                TextAutoResize::None
            } else {
                TextAutoResize::Height
            };
        }
        NodeData::Group(group) => {
            if group.clip_size.is_some() {
                group.clip_size = Some([w, h]);
            } else {
                group.local_size = Some([w, h]);
            }
        }
        _ => {}
    }
}

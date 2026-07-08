//! The resize gesture: corner/edge-handle drag of a single-node selection.
//!
//! Like the move gesture, the drag is a transient preview — each frame writes
//! the computed transform straight into the scene (no open transaction) and the
//! single `SetTransform { old, new }` is recorded on release. The math runs in
//! the node's own (possibly rotated) frame so an existing rotation survives the
//! resize.

use super::state::{Phase, ResizingState, SelectTool};
use crate::context::ToolContext;
use crate::event::ModifierKeys;
use crate::tool::{CursorHint, ToolResponse};
use fanta_canvas::{ResizeHandle, SnapResult};
use fanta_doc::{NodeData, NodeId, Operation, TextAutoResize};
use glam::DVec2;

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
        let Some(original_local) = ctx.doc.scene.local_bounds(node_id) else {
            return ToolResponse::empty();
        };
        let original_transform = node.transform;
        // Text resizes its BOX, not its scale: a scaled transform stretches the
        // glyphs (the "text distorts like an SVG" bug). Capture the press-time
        // data so the preview can rewrite `local_size` each frame and the commit
        // records a `ReplaceData` alongside the transform op. Every other variant
        // keeps the scale-resize path (`None`).
        let box_data = matches!(node.data, NodeData::Text(_)).then(|| Box::new(node.data.clone()));
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
            snap_candidates,
            box_data,
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
            let (new_transform, w, h) = fanta_canvas::resize_box_keep_rotation(
                state.original_transform,
                state.original_local,
                state.handle,
                cursor_world,
                lock_aspect,
                from_center,
            );
            if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
                n.transform = new_transform;
                apply_text_box_size(&mut n.data, w, h, state.handle);
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
        let new_transform = fanta_canvas::resize_transform_keep_rotation(
            state.original_transform,
            state.original_local,
            state.handle,
            cursor_world,
            lock_aspect,
            from_center,
        );

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
        let _ = ctx.doc.history.begin("Resize", &mut ctx.doc.scene);
        // Reset both transform and data to their press-time values so the ops'
        // `apply` drives them back to `new` through the history chokepoint.
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = state.original_transform;
            if let Some(old) = &state.box_data {
                n.data = old.as_ref().clone();
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
        if let Err(e) = ctx.doc.history.commit(&mut ctx.doc.scene) {
            tracing::warn!(target: "fanta-tools.select", "commit resize failed: {e}");
        }
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
    }
}

/// Write a text node's new box `[w, h]` into its `local_size` during a box
/// resize, and pin its resize mode so the new dimensions actually take effect:
/// a horizontal-only drag keeps the box auto-height (fixed width → reflow, height
/// follows the text); any vertical drag fixes the box (Figma `NONE`) so the
/// dragged height is honored. Auto-width text (which never wraps) is switched to
/// a wrapping mode so the new width reflows instead of being ignored. Non-text
/// variants are left untouched.
fn apply_text_box_size(data: &mut NodeData, w: f64, h: f64, handle: ResizeHandle) {
    if let NodeData::Text(t) = data {
        t.local_size = [w, h];
        t.auto_resize = if handle.affects_y() {
            TextAutoResize::None
        } else {
            TextAutoResize::Height
        };
    }
}

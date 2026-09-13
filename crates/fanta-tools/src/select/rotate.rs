//! The rotation gesture: dragging a corner rotation zone of a single-node
//! selection about its bounding-box center.
//!
//! Mirrors the resize gesture's lifecycle: the drag is a transient preview
//! (direct scene writes, no open transaction) and a single `SetTransform` op is
//! recorded on release. The swept angle is measured from the press ray to the
//! current cursor ray about the captured pivot; Shift snaps the absolute
//! orientation to 15° increments.

use super::state::{Phase, RotatingState, SelectTool};
use crate::context::ToolContext;
use crate::event::ModifierKeys;
use crate::tool::{CursorHint, ToolResponse};
use fanta_canvas::{RotateHandle, rotate_about, rotation_delta};
use fanta_doc::{Bounds, NodeId, Operation};
use glam::DVec2;

impl SelectTool {
    /// Enter the rotation phase: capture the pivot (selected node's bbox
    /// center), the press-time cursor ray and rotation, and the original
    /// transform. The drag is transient — no history transaction is opened
    /// here; the single `SetTransform` is recorded on release.
    pub(super) fn begin_rotate(
        &mut self,
        ctx: &mut ToolContext,
        node_id: NodeId,
        handle: RotateHandle,
        original_world: Bounds,
        screen_press: DVec2,
    ) -> ToolResponse {
        let Some(node) = ctx.doc.scene.get(node_id) else {
            return ToolResponse::empty();
        };
        let original_transform = node.transform;
        let original_world_transform = ctx
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
        // The transformed local-box center is the center of the oriented box.
        // It also avoids treating an AABB from a skewed parent as node-local.
        let pivot = super::resize::authored_resize_bounds(ctx, node_id)
            .map(|bounds| original_world_transform.transform_point(bounds.center()))
            .unwrap_or_else(|| original_world.center());
        let press_world =
            fanta_canvas::screen_to_world(screen_press, ctx.viewport, ctx.screen_size);
        // The node's current rotation, for absolute 15°-snap with Shift.
        let base_angle = fanta_canvas::transform_angle(&original_world_transform);
        let state = RotatingState {
            handle,
            node_id,
            pivot,
            press_world,
            base_angle,
            original_transform,
            original_world_transform,
            parent_world_inverse,
        };
        self.phase = Phase::Rotating(Box::new(state));
        ToolResponse::cursor(CursorHint::Move)
    }

    /// Per-frame rotation preview. Measures the angle swept from the press ray
    /// to the current cursor ray about the pivot (Shift snaps the resulting
    /// absolute orientation to 15°), composes a rotate-about-pivot onto the
    /// press-time transform, and writes it straight into the scene — transient,
    /// like the move/resize previews.
    pub(super) fn compute_rotate_response(
        &self,
        ctx: &mut ToolContext,
        state: &RotatingState,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let cursor_world = fanta_canvas::screen_to_world(screen, ctx.viewport, ctx.screen_size);
        let snap = modifiers.contains(ModifierKeys::SHIFT);
        let delta = rotation_delta(
            state.pivot,
            state.press_world,
            cursor_world,
            state.base_angle,
            snap,
        );
        // Compose: rotate the press-time transform about the world pivot. `then`
        // applies the original first, then the rotation — so the node spins in
        // place about its bbox center without translating otherwise.
        let new_world_transform = state
            .original_world_transform
            .then(&rotate_about(state.pivot, delta));
        let new_transform = new_world_transform.then(&state.parent_world_inverse);
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = new_transform;
        }
        ToolResponse::cursor(CursorHint::Move)
    }

    /// Finalize a rotation: same strategy as [`SelectTool::commit_resize`].
    /// The scene holds the final preview transform; record one
    /// `SetTransform { old: original, new: final }` so the gesture undoes in a
    /// single step.
    ///
    /// [`SelectTool::commit_resize`]: super::SelectTool::commit_resize
    pub(super) fn commit_rotate(&self, ctx: &mut ToolContext, state: &RotatingState) {
        let Some(new) = ctx.doc.scene.get(state.node_id).map(|n| n.transform) else {
            return;
        };
        if new == state.original_transform {
            return;
        }
        ctx.doc.history.begin("Rotate", &mut ctx.doc.scene);
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = state.original_transform;
        }
        if let Err(e) = ctx.doc.apply(Operation::SetTransform {
            id: state.node_id,
            old: state.original_transform,
            new,
        }) {
            tracing::warn!(target: "fanta-tools.select", "rotate commit op failed: {e}");
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
    }

    /// Abort a rotation with no transaction: restore the node's press-time
    /// transform directly. Used on Esc and tool deactivation.
    pub(super) fn restore_rotate(ctx: &mut ToolContext, state: &RotatingState) {
        if let Some(n) = ctx.doc.scene.get_mut(state.node_id) {
            n.transform = state.original_transform;
        }
    }
}

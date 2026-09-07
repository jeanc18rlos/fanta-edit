//! Drag-to-reparent — ported from OpenPencil's `reparentOutsideNodes` +
//! `findMoveDropTarget` / `doReorderChild`.
//!
//! These run inside the already-open "Move" transaction (see
//! [`SelectTool::commit_move`]) so the whole move + reparent + reorder gesture
//! collapses into one undo step. Two cases per dropped node: drop INTO a
//! container under the dropped center, or pop OUT of a frame the node was
//! dragged entirely clear of. Both preserve the node's world position by
//! rebasing its local transform into the new parent's space.
//!
//! [`SelectTool::commit_move`]: super::SelectTool::commit_move

use super::state::SelectTool;
use crate::context::ToolContext;
use fanta_doc::{IndexKey, NodeId, Operation, Transform2D};
use glam::DVec2;

impl SelectTool {
    /// Decide whether a just-dropped node `id` should change parents, and if so
    /// emit the structural ops (within the already-open "Move" transaction).
    ///
    /// Ported from OpenPencil. Two cases, evaluated in order:
    ///
    /// 1. **Drop INTO a container** (`findMoveDropTarget` + `doReorderChild`):
    ///    if the node's dropped center lands over a group/frame that is a *valid*
    ///    new parent (not the node, not its own descendant, not already its
    ///    parent), reparent into it. We pick the topmost such container under the
    ///    center — Figma's "drop where the cursor is" semantic — and insert at the
    ///    top of its z-order (`next_child_index`), the sibling insertion index.
    ///
    /// 2. **Drop OUTSIDE the current parent** (`reparentOutsideNodes`): if the
    ///    node still has a non-root parent and its world bounds fall *entirely*
    ///    outside that parent's world bounds, pop it up to the grandparent (or
    ///    root). This keeps a node from rendering clipped/orphaned under a frame
    ///    it was dragged out of.
    ///
    /// In both cases the node's world position is preserved: reparenting changes
    /// the ancestor chain, so the local transform is rebased into the new parent's
    /// space (`new_local = world.then(&new_parent_world.inverse())`) and recorded
    /// as a `SetTransform`. The reparent itself is one `Operation::Reparent`
    /// carrying the new parent + insertion index. No model change — both ops exist.
    pub(super) fn reparent_on_drop(ctx: &mut ToolContext, id: NodeId) {
        let Some(node) = ctx.doc.scene.get(id) else {
            return;
        };
        let current_parent = node.parent;
        let world = match ctx.doc.scene.world_transform(id) {
            Some(w) => w,
            None => return,
        };
        let Some(world_bounds) = ctx.doc.scene.world_bounds(id) else {
            return;
        };

        // Case 1: drop INTO a container under the dropped center.
        let center = world_bounds.center();
        if let Some(target) = Self::drop_target_container(ctx, id, current_parent, center) {
            let new_index = ctx.doc.scene.next_child_index(Some(target));
            Self::reparent_into(ctx, id, Some(target), new_index, world);
            return;
        }

        // Case 2: drop OUTSIDE the current parent's bounds → pop to grandparent.
        let Some(parent_id) = current_parent else {
            return; // already at root; nothing to pop out of.
        };
        let Some(parent_bounds) = ctx.doc.scene.world_bounds(parent_id) else {
            return;
        };
        let fully_outside = !world_bounds.intersects(&parent_bounds);
        if fully_outside {
            // Grandparent (the parent's parent), or root when the parent is
            // already top-level — mirrors OpenPencil's `parent.parentId ??
            // currentPageId`.
            let grandparent = ctx.doc.scene.get(parent_id).and_then(|p| p.parent);
            let new_index = ctx.doc.scene.next_child_index(grandparent);
            Self::reparent_into(ctx, id, grandparent, new_index, world);
        }
    }

    /// Find the topmost group/frame under `center` that `id` may legally be
    /// reparented into. Excludes `id` itself, any descendant of `id` (a cycle),
    /// and `id`'s current parent (reparenting into the same parent is a no-op —
    /// a pure z-order reorder, handled by a separate gesture). Returns `None`
    /// when there is no eligible container (so the caller falls through to the
    /// drop-outside check).
    fn drop_target_container(
        ctx: &ToolContext,
        id: NodeId,
        current_parent: Option<NodeId>,
        center: DVec2,
    ) -> Option<NodeId> {
        // Walk every node under the point, top-z first, and take the first group
        // that is a valid parent. `hit_test_deep` returns leaves; groups never
        // "catch" themselves there, so we instead test containers explicitly via
        // their world bounds, descending so the *innermost* containing frame wins
        // (Figma drops into the deepest frame under the cursor).
        let mut best: Option<(NodeId, usize)> = None;
        // Scope the drop-target search to the active page's subtree. `.fig`
        // pages share a world origin, so an unscoped search could reparent the
        // dropped node into an invisible frame on another page directly under
        // the cursor. `None` (single-page / hand-authored docs) searches every
        // root, preserving the old behavior.
        match ctx.scope() {
            Some(page) => {
                Self::find_container(ctx, page, id, current_parent, center, 0, &mut best);
            }
            None => {
                for &root in ctx.doc.scene.roots() {
                    Self::find_container(ctx, root, id, current_parent, center, 0, &mut best);
                }
            }
        }
        best.map(|(node, _)| node)
    }

    /// Depth-first descent that records the deepest valid container whose world
    /// bounds contain `center`. `depth` breaks ties toward the innermost frame.
    fn find_container(
        ctx: &ToolContext,
        node_id: NodeId,
        dragged: NodeId,
        current_parent: Option<NodeId>,
        center: DVec2,
        depth: usize,
        best: &mut Option<(NodeId, usize)>,
    ) {
        // Never descend into the dragged subtree — reparenting into a descendant
        // would create a cycle (set_parent would reject it anyway, but skipping
        // here avoids proposing an invalid target).
        if node_id == dragged {
            return;
        }
        let Some(node) = ctx.doc.scene.get(node_id) else {
            return;
        };
        if node.flags.contains(fanta_doc::NodeFlags::HIDDEN) {
            return;
        }
        let Some(bounds) = ctx.doc.scene.world_bounds(node_id) else {
            return;
        };
        if !bounds.contains_point(center) {
            return;
        }
        // A valid container: a group that accepts children, is not the dragged
        // node's current parent (that would be a no-op reparent), and is not the
        // dragged node itself (guarded above).
        if node.can_have_children() && Some(node_id) != current_parent {
            match best {
                Some((_, best_depth)) if *best_depth >= depth => {}
                _ => *best = Some((node_id, depth)),
            }
        }
        for &child in ctx.doc.scene.children_of(Some(node_id)) {
            Self::find_container(ctx, child, dragged, current_parent, center, depth + 1, best);
        }
    }

    /// Reparent `id` under `new_parent` at `new_index`, preserving its world
    /// transform `world`. Emits one `Operation::Reparent` then one
    /// `Operation::SetTransform` rebasing the local transform into the new
    /// parent's coordinate space — both inside the caller's open transaction.
    fn reparent_into(
        ctx: &mut ToolContext,
        id: NodeId,
        new_parent: Option<NodeId>,
        new_index: IndexKey,
        world: Transform2D,
    ) {
        let Some(node) = ctx.doc.scene.get(id) else {
            return;
        };
        let old_parent = node.parent;
        let old_index = node.index;
        let old_local = node.transform;

        if let Err(e) = ctx.doc.apply(Operation::Reparent {
            id,
            old_parent,
            old_index,
            new_parent,
            new_index,
        }) {
            // set_parent rejects cycles / non-container parents; if it does,
            // leave the node where it is rather than corrupt the transaction.
            tracing::warn!(target: "fanta-tools.select", "reparent op failed: {e}");
            return;
        }

        // Rebase the local transform so the node keeps its world position under
        // the new parent. new_parent == None → root, whose world transform is
        // identity, so the new local is just the world transform.
        let parent_world = match new_parent {
            Some(p) => ctx
                .doc
                .scene
                .world_transform(p)
                .unwrap_or(Transform2D::IDENTITY),
            None => Transform2D::IDENTITY,
        };
        let new_local = world.then(&parent_world.inverse());
        if new_local != old_local {
            if let Err(e) = ctx.doc.apply(Operation::SetTransform {
                id,
                old: old_local,
                new: new_local,
            }) {
                tracing::warn!(target: "fanta-tools.select", "reparent rebase op failed: {e}");
            }
        }
    }
}

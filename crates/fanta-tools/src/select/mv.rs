//! The move gesture and the per-frame `on_move` state-machine dispatcher.
//!
//! `on_move` is the single Move-event entry point: it drives every phase
//! transition (PressedOnEmpty → Marquee, PressedOnNode → Moving) and forwards
//! to the resize/rotate preview helpers for those phases. The remaining
//! functions here own the move proper — building the move set, the transient
//! per-frame preview (`compute_move_response`), the single-transaction commit
//! (`commit_move`), and the Esc/abort restore (`restore_move`).

use super::state::{DRAG_THRESHOLD_PX, Phase, SelectTool};
use crate::context::ToolContext;
use crate::event::ModifierKeys;
use crate::tool::{CursorHint, MovingSelection, ToolOverlay, ToolResponse, bounds_from_corners};
use fanta_canvas::{MarqueeMode, SnapResult};
use fanta_doc::{NodeId, Operation, Transform2D};
use glam::DVec2;
use smallvec::SmallVec;
use std::collections::HashSet;

impl SelectTool {
    pub(super) fn on_move(
        &mut self,
        ctx: &mut ToolContext,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        // Match against the *current* phase. The transitions that fire here
        // are: PressedOnEmpty → Marquee (on first move past threshold);
        // PressedOnNode → Moving (on first move past threshold); subsequent
        // Move events update the active phase in place.
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        match phase {
            Phase::Idle => {
                // Hover — return the default cursor. The shell uses this to
                // restore the cursor after a previous gesture cleared it.
                self.phase = Phase::Idle;
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::PressedOnEmpty { screen_press } => {
                if (screen - screen_press).length() < DRAG_THRESHOLD_PX {
                    // Still a click in progress; restore.
                    self.phase = Phase::PressedOnEmpty { screen_press };
                    return ToolResponse::empty();
                }
                let mode = if modifiers.contains(ModifierKeys::ALT) {
                    MarqueeMode::Intersects
                } else {
                    MarqueeMode::Contains
                };
                self.phase = Phase::Marquee {
                    screen_press,
                    screen_current: screen,
                    mode,
                };
                ToolResponse::cursor(CursorHint::Crosshair).with_overlay(ToolOverlay::Marquee {
                    screen_rect: bounds_from_corners(screen_press, screen),
                })
            }
            Phase::Marquee {
                screen_press, mode, ..
            } => {
                let new_mode = if modifiers.contains(ModifierKeys::ALT) {
                    MarqueeMode::Intersects
                } else {
                    MarqueeMode::Contains
                };
                self.phase = Phase::Marquee {
                    screen_press,
                    screen_current: screen,
                    mode: if new_mode == mode { mode } else { new_mode },
                };
                ToolResponse::cursor(CursorHint::Crosshair).with_overlay(ToolOverlay::Marquee {
                    screen_rect: bounds_from_corners(screen_press, screen),
                })
            }
            Phase::PressedOnNode {
                screen_press,
                target,
                extending,
            } => {
                if (screen - screen_press).length() < DRAG_THRESHOLD_PX {
                    self.phase = Phase::PressedOnNode {
                        screen_press,
                        target,
                        extending,
                    };
                    return ToolResponse::empty();
                }
                // Promote to Moving. Ensure the target is in the selection — if
                // not (clicked a non-selected node and dragged), replace the
                // selection with just it. Figma fires the same way.
                if !ctx.doc.selection.contains(target) {
                    ctx.doc.selection.select_only(target);
                }
                // Build the move set: the selection's *top-level* nodes only.
                //
                // A marquee over a nested design selects parent frames AND their
                // descendants. Translating every selected node would move a child
                // twice — once via its own local transform, once via its
                // also-selected ancestor — so the child drifts by ~2× the delta
                // and detaches from its frame. We therefore drop any selected
                // node that has a selected ancestor; its subtree rides along with
                // that ancestor for free (descendants inherit the parent
                // transform), which is the correct Figma move semantic.
                let selected: HashSet<NodeId> = ctx.doc.selection.iter().copied().collect();
                // Collect ids and press-time transforms together so a node that
                // fails `scene.get` is dropped as a unit and can never desync the
                // id↔transform pairing (see `MovingSelection::moving`). Selection
                // order is preserved so the first-node snap anchor is stable.
                let moving: SmallVec<[(NodeId, Transform2D); 8]> = ctx
                    .doc
                    .selection
                    .iter()
                    .copied()
                    .filter(|id| {
                        // Keep only nodes with no ancestor in the selection.
                        // `ancestors_of` yields ancestor nodes (not the node
                        // itself), so a top-level selected node — even one nested
                        // under unselected frames — passes this test.
                        !ctx.doc
                            .scene
                            .ancestors_of(*id)
                            .any(|anc| selected.contains(&anc.id))
                    })
                    .filter_map(|id| ctx.doc.scene.get(id).map(|n| (id, n.transform)))
                    .collect();
                let press_world =
                    fanta_canvas::screen_to_world(screen_press, ctx.viewport, ctx.screen_size);
                // Capture the first moved node's press-time world bounds for
                // snapping (the live bounds get overwritten by the transient
                // preview).
                let first_press_bounds = moving
                    .first()
                    .and_then(|(id, _)| ctx.doc.scene.world_bounds(*id));
                // Collect snap candidates ONCE for the whole gesture, excluding
                // the moving nodes. They stay valid because the neighbors don't
                // move (see `MovingSelection::snap_candidates`). Excluding only
                // the top-level movers is enough: their descendants ride along,
                // so they are not stationary neighbors to snap against either.
                let moving_ids: SmallVec<[NodeId; 8]> = moving.iter().map(|(id, _)| *id).collect();
                let snap_candidates = ctx.snap.collect_candidates(&ctx.doc.scene, &moving_ids);
                // No `history.begin` here: the drag is a transient preview that
                // writes transforms directly; the single transaction is opened
                // and committed on release (`on_release`).
                let selection = MovingSelection {
                    press_world,
                    moving,
                    first_press_bounds,
                    snap_candidates,
                };
                self.phase = Phase::Moving(Box::new(selection));
                // Fall through into Moving's update logic below by re-firing
                // ourselves with the same event. Simpler in code: directly
                // call the in-Moving update.
                self.update_moving(ctx, screen)
            }
            Phase::Moving(mut sel) => {
                let response = self.compute_move_response(ctx, &mut sel, screen);
                self.phase = Phase::Moving(sel);
                response
            }
            Phase::Resizing(state) => {
                let response = self.compute_resize_response(ctx, &state, screen, modifiers);
                self.phase = Phase::Resizing(state);
                response
            }
            Phase::Rotating(state) => {
                let response = self.compute_rotate_response(ctx, &state, screen, modifiers);
                self.phase = Phase::Rotating(state);
                response
            }
        }
    }

    fn update_moving(&mut self, ctx: &mut ToolContext, screen: DVec2) -> ToolResponse {
        let Phase::Moving(mut sel) = std::mem::replace(&mut self.phase, Phase::Idle) else {
            return ToolResponse::empty();
        };
        let response = self.compute_move_response(ctx, &mut sel, screen);
        self.phase = Phase::Moving(sel);
        response
    }

    fn compute_move_response(
        &self,
        ctx: &mut ToolContext,
        sel: &mut MovingSelection,
        screen: DVec2,
    ) -> ToolResponse {
        let current_world = fanta_canvas::screen_to_world(screen, ctx.viewport, ctx.screen_size);
        // Fixed-anchor delta: always measured from the press anchor against the
        // immutable press-time transforms. Nothing on `sel` mutates per frame,
        // so the drag can never compound — reaching a cursor in one jump and
        // over many frames produce the identical transform.
        let raw_delta = current_world - sel.press_world;

        // Snap the first node's *press-time* bounds, shifted by the running
        // delta, against the candidates cached at gesture start. This is the
        // hot path: `snap_bounds_with` does NOT walk the scene — it reuses the
        // O(neighbors) snapshot, so the per-frame cost is independent of the
        // total node count. That is the whole point of this change: the old
        // `snap_bounds` walked every node (44k on the stress doc) per
        // mouse-move.
        let snap_result = match sel.first_press_bounds {
            Some(bb) => {
                let target_bb = fanta_doc::Bounds {
                    min_x: bb.min_x + raw_delta.x,
                    min_y: bb.min_y + raw_delta.y,
                    max_x: bb.max_x + raw_delta.x,
                    max_y: bb.max_y + raw_delta.y,
                };
                ctx.snap.snap_bounds_with(&sel.snap_candidates, target_bb)
            }
            None => SnapResult::default(),
        };

        let snap_dx = snap_result.x.map(|s| s.delta).unwrap_or(0.0);
        let snap_dy = snap_result.y.map(|s| s.delta).unwrap_or(0.0);
        let effective_delta = DVec2::new(raw_delta.x + snap_dx, raw_delta.y + snap_dy);

        // Transient preview: write each node's transform STRAIGHT into the
        // scene via `get_mut`, NOT through `ctx.doc.apply`. `new = old.then(
        // translation)` composes the press-time transform with the world delta
        // — translation after a linear map is a translation in the parent's
        // frame, the Figma move semantic for root-level and nested nodes alike.
        // Direct writes keep the drag history- and allocation-free; the single
        // transaction committed in `commit_move` (on release) is what upholds
        // the undo invariant. `get_mut` invalidates the world-transform cache,
        // so geometry queries stay correct mid-drag. `sel.moving` holds only
        // top-level nodes, so each moves exactly once and its descendants follow
        // by inheritance — never the double-shift of moving a child and its
        // already-selected parent.
        for &(id, old) in sel.moving.iter() {
            let new = old.then(&Transform2D::translation(
                effective_delta.x,
                effective_delta.y,
            ));
            let _ = ctx.doc.scene.set_transform(id, new);
        }
        // `set_transform` bypasses `Doc::apply`, so the master revision that
        // keys memoized instance expansions is bumped here; otherwise instances
        // of a master whose child is being dragged lag the whole gesture.
        ctx.doc.components.bump_rev_for_nodes(
            &ctx.doc.scene,
            sel.moving.iter().map(|&(id, _)| id),
        );

        let mut response = ToolResponse::cursor(CursorHint::Move);
        for o in Self::snap_overlays(&snap_result) {
            response.overlays.push(o);
        }
        response
    }

    /// Finalize a move: the scene already holds the final preview transforms
    /// (written directly each frame). Record the gesture as ONE transaction —
    /// one `SetTransform { old: press-time, new: final }` per node — so undo is
    /// a single step holding one op per node, not the thousands a per-frame
    /// commit would accumulate. We momentarily restore each node to its old
    /// transform so the op's `apply` re-establishes the final value through the
    /// history chokepoint (preserving the H1 invariant: committed state goes
    /// through history). Nodes whose final equals their original contribute no
    /// op, so a zero-distance drag commits an empty (discarded) transaction.
    pub(super) fn commit_move(&self, ctx: &mut ToolContext, sel: &MovingSelection) {
        ctx.doc.history.begin("Move", &mut ctx.doc.scene);
        for &(id, old) in sel.moving.iter() {
            let Some(new) = ctx.doc.scene.get(id).map(|n| n.transform) else {
                continue;
            };
            if new == old {
                continue;
            }
            // Reset to the press-time value so `apply` drives it back to `new`
            // through the op path, recording a clean old→new pair.
            let _ = ctx.doc.scene.set_transform(id, old);
            if let Err(e) = ctx.doc.apply(Operation::SetTransform { id, old, new }) {
                tracing::warn!(target: "fanta-tools.select", "move commit op failed: {e}");
            }
        }
        // Drag-to-reparent — ported from OpenPencil's `reparentOutsideNodes` +
        // `findMoveDropTarget`/`doReorderChild` (op1: vue/src/shared/input/
        // drop-target.ts, core/src/editor/structure/reorder.ts). The translate
        // ops above are already in the open "Move" transaction; reparent ops join
        // the SAME transaction so the whole gesture (move + reparent + reorder)
        // collapses into one undo step, matching Figma. We process drop-into
        // before drop-out so a node dragged from one frame straight into another
        // lands in the target rather than first popping to root.
        for &(id, _) in sel.moving.iter() {
            Self::reparent_on_drop(ctx, id);
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
    }

    /// Abort a move with no transaction: restore each node's press-time
    /// transform directly. Used on Esc and tool deactivation. Because the drag
    /// never touched history, there is nothing to roll back through it — the
    /// direct restore returns the scene to exactly its pre-gesture state.
    pub(super) fn restore_move(ctx: &mut ToolContext, sel: &MovingSelection) {
        for &(id, old) in sel.moving.iter() {
            let _ = ctx.doc.scene.set_transform(id, old);
        }
        ctx.doc.components.bump_rev_for_nodes(
            &ctx.doc.scene,
            sel.moving.iter().map(|&(id, _)| id),
        );
    }
}

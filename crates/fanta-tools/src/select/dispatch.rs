//! Event entry points and the click / marquee-release / nudge / escape paths.
//!
//! Holds the [`Tool`] impl and the pointer/key fan-out (`handle_pointer`,
//! `handle_key`), the press-time handle/hit-test decision (`on_press`), the
//! release-time commit dispatch (`on_release`), and arrow-key nudging
//! (`on_arrow`). The per-gesture preview/commit logic lives in the `mv`,
//! `resize`, and `rotate` sibling submodules.

use super::state::{NUDGE_LARGE, NUDGE_SMALL, Phase, SelectTool};
use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::tool::{CursorHint, Tool, ToolResponse, bounds_from_corners};
use fanta_canvas::{
    DEFAULT_HANDLE_THRESHOLD, DEFAULT_ROTATE_THRESHOLD, HitPrecision, hit_test_deep,
    hit_test_resize_handle, hit_test_resize_handle_oriented, hit_test_rotate_handle,
    hit_test_rotate_handle_oriented, hit_test_within_screen, transform_angle,
};
use fanta_doc::{NodeId, Operation, Scene, Transform2D};
use glam::DVec2;
use smallvec::SmallVec;

impl Tool for SelectTool {
    fn name(&self) -> &'static str {
        "select"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(ctx, k),
        }
    }

    fn activate(&mut self, _ctx: &mut ToolContext) {
        self.enter_idle();
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        // Abort any in-flight move or resize so we don't strand the transient
        // preview. The drag wrote transforms directly (no open transaction), so
        // we revert by restoring the press-time transforms rather than calling
        // `history.abort`, which would have nothing to roll back.
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Moving(sel) => Self::restore_move(ctx, &sel),
            Phase::Resizing(state) => Self::restore_resize(ctx, &state),
            Phase::Rotating(state) => Self::restore_rotate(ctx, &state),
            _ => {}
        }
        self.enter_idle();
    }
}

impl SelectTool {
    fn handle_pointer(&mut self, ctx: &mut ToolContext, p: PointerEvent) -> ToolResponse {
        match p {
            PointerEvent::Press {
                screen,
                button: Button::Primary,
                modifiers,
                count,
            } => self.on_press(ctx, DVec2::from(screen), modifiers, count),
            PointerEvent::Move { screen, modifiers } => {
                self.on_move(ctx, DVec2::from(screen), modifiers)
            }
            PointerEvent::Release {
                screen,
                button: Button::Primary,
                modifiers,
            } => self.on_release(ctx, DVec2::from(screen), modifiers),
            _ => ToolResponse::empty(),
        }
    }

    fn handle_key(&mut self, ctx: &mut ToolContext, k: KeyEvent) -> ToolResponse {
        match k.key {
            LogicalKey::Escape => {
                // Cancel an in-flight transient drag by restoring press-time
                // transforms directly (there is no open transaction to abort).
                match std::mem::replace(&mut self.phase, Phase::Idle) {
                    Phase::Moving(sel) => Self::restore_move(ctx, &sel),
                    Phase::Resizing(state) => Self::restore_resize(ctx, &state),
                    Phase::Rotating(state) => Self::restore_rotate(ctx, &state),
                    _ => {}
                }
                ctx.doc.selection.clear();
                self.scope = None;
                self.enter_idle();
                ToolResponse::cursor(CursorHint::Default)
            }
            LogicalKey::ArrowLeft
            | LogicalKey::ArrowRight
            | LogicalKey::ArrowUp
            | LogicalKey::ArrowDown => self.on_arrow(ctx, k),
            _ => ToolResponse::empty(),
        }
    }

    fn on_press(
        &mut self,
        ctx: &mut ToolContext,
        screen: DVec2,
        modifiers: ModifierKeys,
        count: u8,
    ) -> ToolResponse {
        // First: did the user grab a handle of the (single) selected node?
        // Precedence matches Figma — the corner resize square wins when the
        // cursor is right on it, and the rotation ring just OUTSIDE the corner
        // catches only when the resize hit-test misses. Both win over a plain
        // node-hit so dragging near a small node's corner doesn't demote to a
        // move.
        if !modifiers.extend_selection() && ctx.doc.selection.len() == 1 {
            if let Some(&id) = ctx.doc.selection.iter().next() {
                if let Some((local, world_transform)) =
                    super::resize::authored_resize_bounds(&ctx.doc.scene, id)
                        .zip(ctx.doc.scene.world_transform(id))
                    && let Some(world) = local.try_transformed(&world_transform)
                {
                    // When the node is rotated, hit-test the ORIENTED handle
                    // positions (so the grab points match the drawn rotated box);
                    // otherwise the AABB fast path. The rotate pivot stays
                    // `world.center()`, which equals the oriented box center for a
                    // pure rotation.
                    let oriented = (transform_angle(&world_transform).abs() > 1e-4)
                        .then_some((world_transform, local));
                    let resize_hit = match oriented {
                        Some((wt, local)) => hit_test_resize_handle_oriented(
                            local,
                            &wt,
                            screen,
                            ctx.viewport,
                            ctx.screen_size,
                            DEFAULT_HANDLE_THRESHOLD,
                        ),
                        None => hit_test_resize_handle(
                            world,
                            screen,
                            ctx.viewport,
                            ctx.screen_size,
                            DEFAULT_HANDLE_THRESHOLD,
                        ),
                    };
                    if let Some(handle) = resize_hit {
                        return self.begin_resize(ctx, id, handle);
                    }
                    let rotate_hit = match oriented {
                        Some((wt, local)) => hit_test_rotate_handle_oriented(
                            local,
                            &wt,
                            screen,
                            ctx.viewport,
                            ctx.screen_size,
                            DEFAULT_ROTATE_THRESHOLD,
                        ),
                        None => hit_test_rotate_handle(
                            world,
                            screen,
                            ctx.viewport,
                            ctx.screen_size,
                            DEFAULT_ROTATE_THRESHOLD,
                        ),
                    };
                    if let Some(rot) = rotate_hit {
                        return self.begin_rotate(ctx, id, rot, world, screen);
                    }
                }
            }
        }

        // Container-first selection (Figma). `hit_test_deep` returns every hit
        // top-z first; `deep[0]` is what a naive topmost pick would grab (the
        // deepest frame/leaf under the cursor). We then walk up to the outermost
        // descendant of the current scope, so a click *anywhere* inside a frame
        // selects the frame — even when its body is fully covered by children.
        // Double-click drills one level deeper, tracked by `self.scope`.
        let world = ctx.screen_to_world(screen);
        let scope_page = ctx.scope();
        let deep = hit_test_deep(&ctx.doc.scene, world, HitPrecision::Path, scope_page);
        let Some(&leaf) = deep.first() else {
            // Empty canvas: exit any entered scope; release will clear/marquee.
            self.scope = None;
            self.phase = Phase::PressedOnEmpty {
                screen_press: screen,
            };
            return ToolResponse::cursor(CursorHint::Crosshair);
        };

        // Drop a stale (deleted) or out-of-subtree entered scope so resolution
        // falls back to page level when the click leaves the entered container.
        if let Some(scope) = self.scope {
            let in_scope = ctx.doc.scene.get(scope).is_some()
                && (leaf == scope || ctx.doc.scene.ancestors_of(leaf).any(|a| a.id == scope));
            if !in_scope {
                self.scope = None;
            }
        }

        // Double-click drills in: enter the container the single click would
        // select, then resolve to its child toward the cursor (one level down).
        if count >= 2 {
            let here = Self::resolve_in_scope(&ctx.doc.scene, leaf, self.scope.or(scope_page));
            self.scope = Some(here);
        }

        let base = self.scope.or(scope_page);
        let extending = modifiers.extend_selection();
        // Container-first (Figma) resolves a fresh click to the outermost frame.
        // BUT if the press lands *within the current selection* — the deepest hit
        // is an already-selected node or a descendant of one — grab that selected
        // node so a drag MOVES the existing selection instead of re-resolving up
        // to its frame. Without this, clicking an already-selected child re-picks
        // its container and the drag moves the whole frame; the keyboard nudge
        // path acts on `selection` directly and so never had this bug. Skipped
        // while extending (Shift/Cmd, which toggles membership) and on a
        // double-click (which must always DRILL in, even into a selected frame).
        let grabbed = (count < 2 && !extending)
            .then(|| {
                std::iter::once(leaf)
                    .chain(ctx.doc.scene.ancestors_of(leaf).map(|a| a.id))
                    .find(|id| ctx.doc.selection.contains(*id))
            })
            .flatten();
        let target = grabbed.unwrap_or_else(|| Self::resolve_in_scope(&ctx.doc.scene, leaf, base));
        self.phase = Phase::PressedOnNode {
            screen_press: screen,
            target,
            extending,
        };
        ToolResponse::cursor(CursorHint::Move)
    }

    /// Resolve the deepest hit `leaf` to the outermost descendant of `container`
    /// on the path down to it — the direct child of `container` that contains
    /// `leaf` (or `leaf` itself when it is already a direct child). This is what
    /// turns "deepest frame under the cursor" into "the top-level frame" at page
    /// scope. When `container` is `None` (no entered scope and no active page),
    /// the leaf's own page root is used so the result is a top-level node and
    /// never the page backdrop. Falls back to `leaf` when nothing on the chain
    /// is a direct child of `container` (e.g. `container == leaf`).
    fn resolve_in_scope(scene: &Scene, leaf: NodeId, container: Option<NodeId>) -> NodeId {
        let container = container.or_else(|| Self::page_root_of(scene, leaf));
        if scene.get(leaf).and_then(|n| n.parent) == container {
            return leaf;
        }
        for anc in scene.ancestors_of(leaf) {
            if anc.parent == container {
                return anc.id;
            }
        }
        leaf
    }

    /// The topmost root ancestor of `id` (the end of its parent chain), or `id`
    /// itself when it is already a root.
    fn page_root_of(scene: &Scene, id: NodeId) -> Option<NodeId> {
        let mut cur = id;
        while let Some(p) = scene.get(cur).and_then(|n| n.parent) {
            cur = p;
        }
        Some(cur)
    }

    fn on_release(
        &mut self,
        ctx: &mut ToolContext,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        match phase {
            Phase::Idle => ToolResponse::cursor(CursorHint::Default),
            Phase::PressedOnNode {
                target, extending, ..
            } => {
                // Released without crossing the drag threshold — pure click.
                if extending || modifiers.extend_selection() {
                    ctx.doc.selection.toggle(target);
                } else {
                    ctx.doc.selection.select_only(target);
                }
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::PressedOnEmpty { .. } => {
                // Released without dragging on empty — clear the selection
                // unless we were extending (then it's a no-op).
                if !modifiers.extend_selection() {
                    ctx.doc.selection.clear();
                }
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::Marquee {
                screen_press, mode, ..
            } => {
                let screen_rect = bounds_from_corners(screen_press, screen);
                let hits = hit_test_within_screen(
                    &ctx.doc.scene,
                    ctx.viewport,
                    ctx.screen_size,
                    screen_rect,
                    mode,
                    ctx.doc.active_page(),
                );
                if modifiers.extend_selection() {
                    for id in hits {
                        ctx.doc.selection.add(id);
                    }
                } else {
                    ctx.doc.selection.replace_with(hits);
                }
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::Moving(sel) => {
                // The drag was transient (direct scene writes). Record the net
                // change as one transaction so undo collapses the whole gesture.
                self.commit_move(ctx, &sel);
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::Resizing(state) => {
                self.commit_resize(ctx, &state);
                ToolResponse::cursor(CursorHint::Default)
            }
            Phase::Rotating(state) => {
                self.commit_rotate(ctx, &state);
                ToolResponse::cursor(CursorHint::Default)
            }
        }
    }

    fn on_arrow(&mut self, ctx: &mut ToolContext, k: KeyEvent) -> ToolResponse {
        if ctx.doc.selection.is_empty() {
            return ToolResponse::empty();
        }
        let step = if k.modifiers.contains(ModifierKeys::SHIFT) {
            NUDGE_LARGE
        } else {
            NUDGE_SMALL
        };
        let delta = match k.key {
            LogicalKey::ArrowLeft => DVec2::new(-step, 0.0),
            LogicalKey::ArrowRight => DVec2::new(step, 0.0),
            LogicalKey::ArrowUp => DVec2::new(0.0, -step),
            LogicalKey::ArrowDown => DVec2::new(0.0, step),
            _ => return ToolResponse::empty(),
        };

        // Build ops first, then apply within one transaction so undo is one
        // step. We collect into a SmallVec to avoid a borrow of `ctx.doc`
        // across the apply loop.
        let ids: SmallVec<[NodeId; 8]> = ctx.doc.selection.iter().copied().collect();
        let mut ops: SmallVec<[Operation; 8]> = SmallVec::new();
        for id in &ids {
            if let Some(n) = ctx.doc.scene.get(*id) {
                let old = n.transform;
                let new = old.then(&Transform2D::translation(delta.x, delta.y));
                ops.push(Operation::SetTransform { id: *id, old, new });
            }
        }
        ctx.doc.history.begin("Nudge", &mut ctx.doc.scene);
        for op in ops {
            if let Err(e) = ctx.doc.apply(op) {
                tracing::warn!(target: "fanta-tools.select", "nudge op failed: {e}");
            }
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
        ToolResponse::empty()
    }
}

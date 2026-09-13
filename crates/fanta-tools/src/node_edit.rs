//! Node-edit tool — the direct-selection ("white arrow") tool for vector
//! paths: select, move, and reshape individual anchors and Bézier handles.
//!
//! ## Gestures (each commits as ONE undo step)
//!
//! - **Click an anchor** selects it (Shift toggles membership). **Drag**
//!   moves every selected anchor, controls riding along; the drag previews by
//!   writing the path directly into the scene and commits the path and clipping
//!   state on release.
//! - **Drag a handle** rotates/extends that control point. While the anchor
//!   is smooth the opposite handle mirrors the angle (length preserved); the
//!   **Alt** drag breaks the pair into a corner.
//! - **Double-click an anchor** toggles smooth ⇄ corner (handles rebuilt
//!   collinear at 1/3 the adjacent chord, or retracted).
//! - **Click a segment** within 6 screen px (hover shows the "+" glyph)
//!   inserts an anchor at that point via an exact de Casteljau split.
//! - **Delete** removes the selected anchors, rejoining neighbors with a line
//!   (min 2 anchors open / 3 closed — guarded in [`node_math`]).
//! - **Marquee** over empty canvas selects the anchors inside it (Shift adds).
//! - **Escape** drops the anchor selection, then exits back to Select.
//!
//! The tool reads/writes the document only through [`ToolContext`], so it
//! works on any canvas tab (project, component-edit, …) the shell scopes it
//! to. All geometry math lives in the pure [`node_math`] module.
//!
//! [`node_math`]: crate::node_math

use crate::context::ToolContext;
use crate::event::{Button, KeyEvent, LogicalKey, ModifierKeys, PointerEvent, ToolEvent};
use crate::node_math::{
    self, AnchorId, AnchorInfo, HandleSide, closest_point_on_path, enumerate_anchors,
};
use crate::tool::{CursorHint, Tool, ToolOverlay, ToolResponse, bounds_from_corners};
use fanta_doc::{Doc, NodeData, NodeFlags, NodeId, Operation, PathData, Transform2D};
use glam::DVec2;
use std::collections::BTreeSet;

/// Screen-px radius around an anchor square that counts as a hit.
const ANCHOR_HIT_PX: f64 = 5.0;
/// Screen-px radius around a handle circle that counts as a hit.
const HANDLE_HIT_PX: f64 = 5.0;
/// Screen-px distance to a segment that shows the "+" glyph / inserts.
const SEGMENT_HIT_PX: f64 = 6.0;
/// Screen-px a press must travel before a click becomes a drag.
const DRAG_THRESHOLD_PX: f64 = 3.0;

#[derive(Debug)]
struct PathEditSnapshot {
    path: PathData,
    local_size: Option<[f64; 2]>,
    unclipped: bool,
}

/// In-flight gesture state.
#[derive(Debug)]
enum Phase {
    Idle,
    /// Primary down on an anchor; not yet decided click vs drag.
    PressedAnchor {
        screen_press: DVec2,
        original: PathEditSnapshot,
        /// Anchors that will move (the selection at press time).
        moving: Vec<AnchorId>,
    },
    /// Anchor drag past the threshold — live preview, single commit on release.
    DraggingAnchor {
        screen_press: DVec2,
        original: PathEditSnapshot,
        moving: Vec<AnchorId>,
    },
    /// Handle drag — live preview, single commit on release.
    DraggingHandle {
        screen_press: DVec2,
        anchor: AnchorId,
        side: HandleSide,
        original: PathEditSnapshot,
        /// Whether the anchor was smooth at press (mirror unless Alt).
        smooth_at_press: bool,
        moved: bool,
    },
    /// Marquee over anchors. `additive` extends the selection (Shift).
    Marquee {
        screen_press: DVec2,
        screen_current: DVec2,
        additive: bool,
    },
}

/// The node-edit (direct selection) tool state machine.
pub struct NodeEditTool {
    select_segments: bool,
    /// The vector node whose path is being edited.
    target: Option<NodeId>,
    /// Selected anchors of the target's path.
    selected: BTreeSet<AnchorId>,
    phase: Phase,
    /// Hovered insert point (segment index, world position) for the "+" glyph.
    insert_hover: Option<DVec2>,
}

impl Default for NodeEditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeEditTool {
    pub fn new() -> Self {
        Self {
            select_segments: false,
            target: None,
            selected: BTreeSet::new(),
            phase: Phase::Idle,
            insert_hover: None,
        }
    }

    pub(crate) fn for_path_selection() -> Self {
        Self {
            select_segments: true,
            ..Self::new()
        }
    }

    /// The node currently being edited (for tests / shell introspection).
    pub fn target(&self) -> Option<NodeId> {
        self.target
    }

    /// Number of selected anchors (for tests).
    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    /// Whether an anchor/handle drag is in progress (for tests).
    pub fn is_dragging(&self) -> bool {
        matches!(
            self.phase,
            Phase::DraggingAnchor { .. } | Phase::DraggingHandle { .. }
        )
    }
}

impl Tool for NodeEditTool {
    fn name(&self) -> &'static str {
        "node-edit"
    }

    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse {
        if self.select_segments
            && self.target.is_some_and(|id| {
                !eligible_selection_target(ctx, id) || !ctx.doc.selection.contains(id)
            })
        {
            self.abort_drag(ctx);
            self.target = None;
            self.selected.clear();
            self.insert_hover = None;
        }
        // The shell swaps tools without calling `activate`, so the edit
        // target is (re-)derived lazily from the doc selection — this is what
        // makes "double-click a vector with Select" land here already armed.
        self.ensure_target(ctx);
        match event {
            ToolEvent::Pointer(p) => self.handle_pointer(ctx, p),
            ToolEvent::Key(k) => self.handle_key(ctx, k),
        }
    }

    fn overlays_after_document_change(&self, doc: &Doc) -> Option<Vec<ToolOverlay>> {
        Some(self.path_overlays(doc))
    }

    fn activate(&mut self, ctx: &mut ToolContext) {
        self.selected.clear();
        self.phase = Phase::Idle;
        self.insert_hover = None;
        self.target = None;
        self.ensure_target(ctx);
    }

    fn deactivate(&mut self, ctx: &mut ToolContext) {
        self.abort_drag(ctx);
        self.target = None;
        self.selected.clear();
        self.phase = Phase::Idle;
        self.insert_hover = None;
    }
}

/// Whether `id` is a vector node in the scene.
fn is_vector(ctx: &ToolContext, id: NodeId) -> bool {
    ctx.doc
        .scene
        .get(id)
        .is_some_and(|n| n.data.as_vector().is_some())
}

fn eligible_selection_target(ctx: &ToolContext, id: NodeId) -> bool {
    let scene = &ctx.doc.scene;
    let Some(node) = scene.get(id) else {
        return false;
    };
    if node.data.as_vector().is_none()
        || node.flags.intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
        || scene.ancestors_of(id).any(|ancestor| {
            ancestor
                .flags
                .intersects(NodeFlags::LOCKED | NodeFlags::HIDDEN)
                || matches!(ancestor.data, NodeData::Boolean(_))
        })
        || ctx.scope().is_some_and(|root| {
            id != root && !scene.ancestors_of(id).any(|ancestor| ancestor.id == root)
        })
    {
        return false;
    }
    scene.world_transform(id).is_some_and(|transform| {
        transform.0.is_finite() && transform.0.matrix2.determinant().abs() > f64::EPSILON
    })
}

impl NodeEditTool {
    // -- target / geometry helpers --------------------------------------------

    fn retarget_selection(&mut self, ctx: &mut ToolContext, screen: DVec2) {
        if !matches!(self.phase, Phase::Idle) {
            self.abort_drag(ctx);
        }
        if let Some(path) = self.target_path(ctx) {
            let world = self.target_world(ctx);
            let anchors = enumerate_anchors(&path);
            // Visible controls can extend outside the object's bounds and overlap other objects.
            if self.anchor_at(ctx, &anchors, &world, screen).is_some()
                || self.handle_at(ctx, &anchors, &world, screen).is_some()
                || self.segment_at(ctx, &path, &world, screen).is_some()
            {
                return;
            }
        }
        let world = ctx.screen_to_world(screen);
        let radius = DVec2::splat(SEGMENT_HIT_PX);
        let hit_bounds = fanta_doc::Bounds::from_min_max(
            ctx.screen_to_world(screen - radius),
            ctx.screen_to_world(screen + radius),
        );
        // Query the full screen-space tolerance before refining geometry, so thin
        // paths and paths behind another vector's empty bounds remain reachable.
        let candidates = ctx.doc.scene.rect_query_where(hit_bounds, |id, bounds| {
            bounds.intersects(&hit_bounds)
                && eligible_selection_target(ctx, id)
                && ctx.doc.scene.ancestors_of(id).all(|ancestor| {
                    let NodeData::Group(group) = &ancestor.data else {
                        return true;
                    };
                    let Some([width, height]) = group.clip_size else {
                        return true;
                    };
                    if ancestor
                        .meta
                        .get("clip_content")
                        .and_then(serde_json::Value::as_bool)
                        == Some(false)
                    {
                        return true;
                    }
                    ctx.doc
                        .scene
                        .world_transform(ancestor.id)
                        .is_some_and(|transform| {
                            if !transform.is_finite()
                                || transform.0.matrix2.determinant().abs() <= f64::EPSILON
                            {
                                return false;
                            }
                            let local = transform.inverse().transform_point(world);
                            local.x >= 0. && local.y >= 0. && local.x <= width && local.y <= height
                        })
                })
        });
        let previous_target = self.target;
        for id in candidates.into_iter().rev() {
            self.target = Some(id);
            let on_path = self.target_path(ctx).is_some_and(|path| {
                let world = self.target_world(ctx);
                self.anchor_at(ctx, &enumerate_anchors(&path), &world, screen)
                    .is_some()
                    || self.segment_at(ctx, &path, &world, screen).is_some()
                    || self.in_filled_path(ctx, screen)
            });
            if on_path {
                if previous_target != self.target {
                    self.selected.clear();
                }
                ctx.doc.selection.select_only(id);
                return;
            }
        }
        self.target = previous_target;
    }

    fn in_filled_path(&self, ctx: &ToolContext, screen: DVec2) -> bool {
        self.target
            .and_then(|id| ctx.doc.scene.get(id))
            .and_then(|node| node.data.as_vector())
            .is_some_and(|vector| {
                !vector.fills.is_empty()
                    && fanta_canvas::point_in_path(
                        &vector.path,
                        self.screen_to_local(ctx, &self.target_world(ctx), screen),
                    )
            })
    }

    /// Keep `target` pointed at a live vector node: when it is unset (or its
    /// node vanished / changed kind), adopt the first selected vector from the
    /// doc. Dropping the target also drops the per-path anchor selection.
    fn ensure_target(&mut self, ctx: &ToolContext) {
        let eligible = |id| {
            if self.select_segments {
                eligible_selection_target(ctx, id)
            } else {
                is_vector(ctx, id)
            }
        };
        if self.target.is_some_and(eligible) {
            return;
        }
        self.target = ctx.doc.selection.iter().copied().find(|&id| eligible(id));
        self.selected.clear();
    }

    /// The target's current path (cloned — paths in edit scope are small).
    fn target_path(&self, ctx: &ToolContext) -> Option<PathData> {
        let id = self.target?;
        Some(ctx.doc.scene.get(id)?.data.as_vector()?.path.clone())
    }

    fn target_snapshot(&self, ctx: &ToolContext) -> Option<PathEditSnapshot> {
        let node = ctx.doc.scene.get(self.target?)?;
        let vector = node.data.as_vector()?;
        Some(PathEditSnapshot {
            path: vector.path.clone(),
            local_size: vector.local_size,
            unclipped: node.flags.contains(NodeFlags::UNCLIPPED_VECTOR),
        })
    }

    /// World transform of the target node (identity if unresolvable).
    fn target_world(&self, ctx: &ToolContext) -> Transform2D {
        self.target
            .and_then(|id| ctx.doc.scene.world_transform(id))
            .unwrap_or(Transform2D::IDENTITY)
    }

    /// Map a path-local point to screen px.
    fn local_to_screen(&self, ctx: &ToolContext, world_t: &Transform2D, local: DVec2) -> DVec2 {
        ctx.world_to_screen(world_t.transform_point(local))
    }

    /// Map a screen-px point into the target's path-local space.
    fn screen_to_local(&self, ctx: &ToolContext, world_t: &Transform2D, screen: DVec2) -> DVec2 {
        world_t
            .inverse()
            .transform_point(ctx.screen_to_world(screen))
    }

    fn write_path_direct(
        &self,
        ctx: &mut ToolContext,
        path: PathData,
        original: &PathEditSnapshot,
    ) {
        let Some(node) = self.target.and_then(|id| ctx.doc.scene.get_mut(id)) else {
            return;
        };
        let Some(vector) = node.data.as_vector_mut() else {
            return;
        };
        let changed = path != original.path;
        // Authored viewport boxes no longer describe edited geometry, including
        // negative coordinates and strokes. Persist the opt-out so load-time
        // legacy backfill cannot recreate a clip around the edited path.
        vector.local_size = if changed { None } else { original.local_size };
        node.flags
            .set(NodeFlags::UNCLIPPED_VECTOR, changed || original.unclipped);
        vector.path = path;
    }

    fn commit_path(&self, ctx: &mut ToolContext, original: PathEditSnapshot) {
        let Some(id) = self.target else {
            return;
        };
        let Some(node) = ctx.doc.scene.get(id) else {
            return;
        };
        let Some(vector) = node.data.as_vector() else {
            return;
        };
        if vector.path == original.path {
            return;
        }
        let mut old_data = vector.clone();
        old_data.path = original.path;
        old_data.local_size = original.local_size;
        let mut new_data = vector.clone();
        if self.select_segments {
            new_data.parametric = None;
        }
        let new_flags = node.flags;
        let mut old_flags = new_flags;
        old_flags.set(NodeFlags::UNCLIPPED_VECTOR, original.unclipped);
        if let Some(node) = ctx.doc.scene.get_mut(id) {
            node.data = NodeData::Vector(old_data.clone());
            node.flags
                .set(NodeFlags::UNCLIPPED_VECTOR, original.unclipped);
        }
        let mut operations = vec![Operation::ReplaceData {
            id,
            old: Box::new(NodeData::Vector(old_data)),
            new: Box::new(NodeData::Vector(new_data)),
        }];
        if new_flags != old_flags {
            operations.push(Operation::SetFlags {
                id,
                old: old_flags,
                new: new_flags,
            });
        }
        ctx.doc.history.begin("Edit path", &mut ctx.doc.scene);
        for operation in operations {
            if let Err(error) = ctx.doc.apply(operation) {
                if let Err(rollback) = ctx.doc.abort_transaction() {
                    tracing::error!(target: "fanta-tools.node_edit", "path rollback failed: {rollback}");
                }
                tracing::warn!(target: "fanta-tools.node_edit", "path commit failed: {error}");
                return;
            }
        }
        ctx.doc.history.commit(&mut ctx.doc.scene);
    }

    fn apply_path_edit(&self, ctx: &mut ToolContext, new_path: PathData) {
        let Some(original) = self.target_snapshot(ctx) else {
            return;
        };
        if new_path == original.path {
            return;
        }
        self.write_path_direct(ctx, new_path, &original);
        self.commit_path(ctx, original);
    }

    fn abort_drag(&mut self, ctx: &mut ToolContext) {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::DraggingAnchor { original, .. } | Phase::DraggingHandle { original, .. } => {
                self.write_path_direct(ctx, original.path.clone(), &original);
            }
            _ => {}
        }
    }

    // -- hit-testing ------------------------------------------------------------

    /// The anchor whose square is within [`ANCHOR_HIT_PX`] of `screen`.
    fn anchor_at(
        &self,
        ctx: &ToolContext,
        anchors: &[AnchorInfo],
        world_t: &Transform2D,
        screen: DVec2,
    ) -> Option<AnchorId> {
        let mut best: Option<(AnchorId, f64)> = None;
        for a in anchors {
            let s = self.local_to_screen(ctx, world_t, a.pos);
            let d = (s - screen).length();
            if d <= ANCHOR_HIT_PX && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((a.id, d));
            }
        }
        best.map(|(id, _)| id)
    }

    /// The visible handle (selected anchors + their neighbors) within
    /// [`HANDLE_HIT_PX`] of `screen`.
    fn handle_at(
        &self,
        ctx: &ToolContext,
        anchors: &[AnchorInfo],
        world_t: &Transform2D,
        screen: DVec2,
    ) -> Option<(AnchorId, HandleSide)> {
        let visible = self.handle_visible_set(anchors);
        let mut best: Option<(AnchorId, HandleSide, f64)> = None;
        for a in anchors {
            if !visible.contains(&a.id) {
                continue;
            }
            for (side, ctrl) in [(HandleSide::In, a.ctrl_in), (HandleSide::Out, a.ctrl_out)] {
                let Some(c) = ctrl else { continue };
                let s = self.local_to_screen(ctx, world_t, c);
                let d = (s - screen).length();
                if d <= HANDLE_HIT_PX && best.is_none_or(|(_, _, bd)| d < bd) {
                    best = Some((a.id, side, d));
                }
            }
        }
        best.map(|(id, side, _)| (id, side))
    }

    /// Anchors whose handles are visible: the selected anchors plus each
    /// selected anchor's subpath neighbors (wrapping when closed) — the
    /// Illustrator visibility rule.
    fn handle_visible_set(&self, anchors: &[AnchorInfo]) -> BTreeSet<AnchorId> {
        let mut out = BTreeSet::new();
        // Per-subpath anchor counts for neighbor wraparound.
        let mut counts: Vec<usize> = Vec::new();
        for a in anchors {
            if a.id.subpath >= counts.len() {
                counts.resize(a.id.subpath + 1, 0);
            }
            counts[a.id.subpath] = counts[a.id.subpath].max(a.id.index + 1);
        }
        let closed: Vec<bool> = {
            let mut v = vec![false; counts.len()];
            for a in anchors {
                v[a.id.subpath] = a.closed;
            }
            v
        };
        for &id in &self.selected {
            out.insert(id);
            let Some(&n) = counts.get(id.subpath) else {
                continue;
            };
            if n == 0 {
                continue;
            }
            let wrap = closed[id.subpath];
            if wrap {
                out.insert(AnchorId {
                    subpath: id.subpath,
                    index: (id.index + n - 1) % n,
                });
                out.insert(AnchorId {
                    subpath: id.subpath,
                    index: (id.index + 1) % n,
                });
            } else {
                if let Some(prev) = id.index.checked_sub(1) {
                    out.insert(AnchorId {
                        subpath: id.subpath,
                        index: prev,
                    });
                }
                if id.index + 1 < n {
                    out.insert(AnchorId {
                        subpath: id.subpath,
                        index: id.index + 1,
                    });
                }
            }
        }
        out
    }

    /// The nearest on-curve segment point within [`SEGMENT_HIT_PX`] of
    /// `screen`, for the insert gesture. Distance is measured in screen px.
    fn segment_at(
        &self,
        ctx: &ToolContext,
        path: &PathData,
        world_t: &Transform2D,
        screen: DVec2,
    ) -> Option<(usize, f64, DVec2)> {
        if self.select_segments {
            // Local-space proximity is distorted by non-uniform scale and skew.
            let mut screen_path = path.clone();
            screen_path.map_points_mut(|point| {
                self.local_to_screen(ctx, world_t, DVec2::from(point))
                    .to_array()
            });
            let hit = closest_point_on_path(&screen_path, screen)?;
            return (hit.dist <= SEGMENT_HIT_PX)
                .then(|| {
                    node_math::eval_segment(path, hit.seg_index, hit.t)
                        .map(|point| (hit.seg_index, hit.t, point))
                })
                .flatten();
        }
        let local = self.screen_to_local(ctx, world_t, screen);
        let hit = closest_point_on_path(path, local)?;
        let on_screen = self.local_to_screen(ctx, world_t, hit.pos);
        ((on_screen - screen).length() <= SEGMENT_HIT_PX).then_some((hit.seg_index, hit.t, hit.pos))
    }

    // -- overlays ---------------------------------------------------------------

    fn path_overlays(&self, doc: &Doc) -> Vec<ToolOverlay> {
        let Some(id) = self.target else {
            return Vec::new();
        };
        let Some(vector) = doc.scene.get(id).and_then(|node| node.data.as_vector()) else {
            return Vec::new();
        };
        if self.select_segments && !doc.selection.contains(id) {
            return Vec::new();
        }
        let world_t = doc
            .scene
            .world_transform(id)
            .unwrap_or(Transform2D::IDENTITY);
        let anchors = enumerate_anchors(&vector.path);
        let visible = self.handle_visible_set(&anchors);
        let mut overlays = Vec::new();
        for a in &anchors {
            if visible.contains(&a.id) {
                let wa = world_t.transform_point(a.pos);
                for ctrl in [a.ctrl_in, a.ctrl_out].into_iter().flatten() {
                    let wc = world_t.transform_point(ctrl);
                    overlays.push(ToolOverlay::PathHandle {
                        world_anchor: [wa.x, wa.y],
                        world_ctrl: [wc.x, wc.y],
                    });
                }
            }
        }
        for a in &anchors {
            let w = world_t.transform_point(a.pos);
            overlays.push(ToolOverlay::PathAnchor {
                world: [w.x, w.y],
                selected: self.selected.contains(&a.id),
            });
        }
        overlays
    }

    fn base_overlays(&self, ctx: &ToolContext, response: &mut ToolResponse) {
        if !self.target.is_some_and(|id| is_vector(ctx, id)) {
            return;
        }
        response.overlays.extend(self.path_overlays(ctx.doc));
        if let Some(p) = self.insert_hover {
            response
                .overlays
                .push(ToolOverlay::PathInsertHint { world: [p.x, p.y] });
        }
    }

    /// A response carrying the base overlays plus a cursor hint.
    fn overlay_response(&self, ctx: &ToolContext, cursor: CursorHint) -> ToolResponse {
        let mut r = ToolResponse::cursor(cursor);
        self.base_overlays(ctx, &mut r);
        r
    }

    // -- pointer ------------------------------------------------------------------

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
                button: Button::Primary,
                modifiers,
                ..
            } => self.on_release(ctx, modifiers),
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
        self.insert_hover = None;
        if self.select_segments {
            self.retarget_selection(ctx, screen);
        }
        // No target yet: adopt the vector under the cursor.
        if !self.select_segments
            && (self.target.is_none() || !self.target.is_some_and(|id| is_vector(ctx, id)))
        {
            let world = ctx.screen_to_world(screen);
            let hit = ctx
                .hit_test(world, fanta_canvas::HitPrecision::Bounds)
                .filter(|&id| is_vector(ctx, id));
            if let Some(id) = hit {
                self.target = Some(id);
                self.selected.clear();
                ctx.doc.selection.select_only(id);
            } else {
                return ToolResponse::cursor(CursorHint::Default);
            }
        }
        let Some(original) = self.target_snapshot(ctx) else {
            return ToolResponse::cursor(CursorHint::Default);
        };
        let path = &original.path;
        let world_t = self.target_world(ctx);
        let anchors = enumerate_anchors(&path);

        // 1) Anchor under the cursor.
        if let Some(id) = self.anchor_at(ctx, &anchors, &world_t, screen) {
            if count >= 2 {
                // Double-click: toggle smooth ⇄ corner — one undo step.
                if let Some(new_path) = node_math::toggle_smooth(&path, id) {
                    self.apply_path_edit(ctx, new_path);
                }
                self.selected.clear();
                self.selected.insert(id);
                self.phase = Phase::Idle;
                return self.overlay_response(ctx, CursorHint::Default);
            }
            let shift = modifiers.contains(ModifierKeys::SHIFT);
            if shift && self.selected.contains(&id) {
                // Shift-click a selected anchor: deselect, no drag.
                self.selected.remove(&id);
                self.phase = Phase::Idle;
                return self.overlay_response(ctx, CursorHint::Default);
            }
            if shift {
                self.selected.insert(id);
            } else if !self.selected.contains(&id) {
                self.selected.clear();
                self.selected.insert(id);
            }
            self.phase = Phase::PressedAnchor {
                screen_press: screen,
                original,
                moving: self.selected.iter().copied().collect(),
            };
            return self.overlay_response(ctx, CursorHint::Move);
        }

        // 2) Handle under the cursor (selected anchors + neighbors only).
        if let Some((id, side)) = self.handle_at(ctx, &anchors, &world_t, screen) {
            let smooth = node_math::anchor_is_smooth(&path, id);
            self.phase = Phase::DraggingHandle {
                screen_press: screen,
                anchor: id,
                side,
                original,
                smooth_at_press: smooth,
                moved: false,
            };
            return self.overlay_response(ctx, CursorHint::Move);
        }

        // 3) Segment within 6 px: insert an anchor at the split point.
        if let Some((seg_index, t, _)) = self.segment_at(ctx, &path, &world_t, screen) {
            if self.select_segments {
                if let Some(anchors) = node_math::segment_anchors(&path, seg_index) {
                    let shift = modifiers.contains(ModifierKeys::SHIFT);
                    if shift && anchors.iter().all(|anchor| self.selected.contains(anchor)) {
                        for anchor in anchors {
                            self.selected.remove(&anchor);
                        }
                        self.phase = Phase::Idle;
                    } else {
                        if !shift && !anchors.iter().all(|anchor| self.selected.contains(anchor)) {
                            self.selected.clear();
                        }
                        self.selected.extend(anchors);
                        self.phase = Phase::PressedAnchor {
                            screen_press: screen,
                            original,
                            moving: self.selected.iter().copied().collect(),
                        };
                    }
                    return self.overlay_response(ctx, CursorHint::Move);
                }
                return self.overlay_response(ctx, CursorHint::Default);
            }
            if let Some((new_path, point)) = node_math::insert_at(&path, seg_index, t) {
                self.apply_path_edit(ctx, new_path.clone());
                // Select the freshly inserted anchor (nearest to the split point).
                self.selected.clear();
                if let Some(a) = enumerate_anchors(&new_path).into_iter().min_by(|a, b| {
                    let da = (a.pos - point).length();
                    let db = (b.pos - point).length();
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                }) {
                    self.selected.insert(a.id);
                }
                self.phase = Phase::Idle;
                return self.overlay_response(ctx, CursorHint::Default);
            }
        }

        if self.select_segments && self.in_filled_path(ctx, screen) {
            self.selected.extend(anchors.iter().map(|anchor| anchor.id));
            self.phase = Phase::PressedAnchor {
                screen_press: screen,
                original,
                moving: self.selected.iter().copied().collect(),
            };
            return self.overlay_response(ctx, CursorHint::Move);
        }

        // 4) Empty space: marquee over anchors.
        self.phase = Phase::Marquee {
            screen_press: screen,
            screen_current: screen,
            additive: modifiers.contains(ModifierKeys::SHIFT),
        };
        self.overlay_response(ctx, CursorHint::Crosshair)
    }

    fn on_move(
        &mut self,
        ctx: &mut ToolContext,
        screen: DVec2,
        modifiers: ModifierKeys,
    ) -> ToolResponse {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => {
                // Hover: show the insert "+" glyph near a segment (but not on
                // top of an anchor or visible handle).
                self.insert_hover = None;
                let mut cursor = CursorHint::Default;
                if let Some(path) = self.target_path(ctx) {
                    let world_t = self.target_world(ctx);
                    let anchors = enumerate_anchors(&path);
                    let on_anchor = self.anchor_at(ctx, &anchors, &world_t, screen).is_some();
                    let on_handle = self.handle_at(ctx, &anchors, &world_t, screen).is_some();
                    if on_anchor || on_handle {
                        cursor = CursorHint::Move;
                    } else if let Some((_, _, local)) =
                        self.segment_at(ctx, &path, &world_t, screen)
                    {
                        if self.select_segments {
                            cursor = CursorHint::Move;
                        } else {
                            self.insert_hover = Some(world_t.transform_point(local));
                            cursor = CursorHint::Crosshair;
                        }
                    }
                }
                self.overlay_response(ctx, cursor)
            }
            Phase::PressedAnchor {
                screen_press,
                original,
                moving,
            } => {
                if (screen - screen_press).length() < DRAG_THRESHOLD_PX {
                    self.phase = Phase::PressedAnchor {
                        screen_press,
                        original,
                        moving,
                    };
                    return self.overlay_response(ctx, CursorHint::Move);
                }
                self.phase = Phase::DraggingAnchor {
                    screen_press,
                    original,
                    moving,
                };
                self.update_anchor_drag(ctx, screen)
            }
            Phase::DraggingAnchor {
                screen_press,
                original,
                moving,
            } => {
                self.phase = Phase::DraggingAnchor {
                    screen_press,
                    original,
                    moving,
                };
                self.update_anchor_drag(ctx, screen)
            }
            Phase::DraggingHandle {
                screen_press,
                anchor,
                side,
                original,
                smooth_at_press,
                ..
            } => {
                let world_t = self.target_world(ctx);
                let local = self.screen_to_local(ctx, &world_t, screen);
                // Mirror while the anchor was smooth at press; Alt breaks the
                // pair into a corner (checked live so Alt can engage mid-drag).
                let mirror = smooth_at_press && !modifiers.contains(ModifierKeys::ALT);
                let original_control = enumerate_anchors(&original.path)
                    .into_iter()
                    .find(|candidate| candidate.id == anchor)
                    .and_then(|candidate| match side {
                        HandleSide::In => candidate.ctrl_in,
                        HandleSide::Out => candidate.ctrl_out,
                    });
                if screen == screen_press || original_control == Some(local) {
                    self.write_path_direct(ctx, original.path.clone(), &original);
                } else if let Some(new_path) =
                    node_math::set_handle(&original.path, anchor, side, local, mirror)
                {
                    self.write_path_direct(ctx, new_path, &original);
                }
                self.phase = Phase::DraggingHandle {
                    screen_press,
                    anchor,
                    side,
                    original,
                    smooth_at_press,
                    moved: true,
                };
                self.overlay_response(ctx, CursorHint::Move)
            }
            Phase::Marquee {
                screen_press,
                additive,
                ..
            } => {
                self.phase = Phase::Marquee {
                    screen_press,
                    screen_current: screen,
                    additive,
                };
                let mut r = self.overlay_response(ctx, CursorHint::Crosshair);
                r.overlays.push(ToolOverlay::Marquee {
                    screen_rect: bounds_from_corners(screen_press, screen),
                });
                r
            }
        }
    }

    /// Recompute the anchor-drag preview from the press-time snapshot — the
    /// fixed-anchor delta means the drag never compounds across frames.
    fn update_anchor_drag(&mut self, ctx: &mut ToolContext, screen: DVec2) -> ToolResponse {
        let Phase::DraggingAnchor {
            screen_press,
            original,
            moving,
        } = &self.phase
        else {
            return ToolResponse::empty();
        };
        let world_t = self.target_world(ctx);
        let inv = world_t.inverse();
        let press_local = inv.transform_point(ctx.screen_to_world(*screen_press));
        let cur_local = inv.transform_point(ctx.screen_to_world(screen));
        let delta = cur_local - press_local;
        if delta == DVec2::ZERO {
            self.write_path_direct(ctx, original.path.clone(), original);
            return self.overlay_response(ctx, CursorHint::Move);
        }
        let mut path = original.path.clone();
        let snapshot = enumerate_anchors(&original.path);
        for id in moving {
            let Some(orig) = snapshot.iter().find(|a| a.id == *id) else {
                continue;
            };
            if let Some(next) = node_math::move_anchor(&path, *id, orig.pos + delta) {
                path = next;
            }
        }
        self.write_path_direct(ctx, path, original);
        self.overlay_response(ctx, CursorHint::Move)
    }

    fn on_release(&mut self, ctx: &mut ToolContext, modifiers: ModifierKeys) -> ToolResponse {
        match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Idle => self.overlay_response(ctx, CursorHint::Default),
            Phase::PressedAnchor { .. } => {
                // Click without drag — selection already applied at press.
                self.overlay_response(ctx, CursorHint::Default)
            }
            Phase::DraggingAnchor { original, .. } => {
                self.commit_path(ctx, original);
                self.overlay_response(ctx, CursorHint::Default)
            }
            Phase::DraggingHandle {
                original, moved, ..
            } => {
                if moved {
                    self.commit_path(ctx, original);
                }
                self.overlay_response(ctx, CursorHint::Default)
            }
            Phase::Marquee {
                screen_press,
                screen_current,
                additive,
            } => {
                let additive = additive || modifiers.contains(ModifierKeys::SHIFT);
                if !additive {
                    self.selected.clear();
                }
                if let Some(path) = self.target_path(ctx) {
                    let world_t = self.target_world(ctx);
                    let rect = bounds_from_corners(screen_press, screen_current);
                    for a in enumerate_anchors(&path) {
                        let s = self.local_to_screen(ctx, &world_t, a.pos);
                        if rect.contains_point(s) {
                            self.selected.insert(a.id);
                        }
                    }
                }
                self.overlay_response(ctx, CursorHint::Default)
            }
        }
    }

    // -- keys -----------------------------------------------------------------------

    fn handle_key(&mut self, ctx: &mut ToolContext, k: KeyEvent) -> ToolResponse {
        match k.key {
            LogicalKey::Delete => {
                if self.selected.is_empty() {
                    return self.overlay_response(ctx, CursorHint::Default);
                }
                if let Some(path) = self.target_path(ctx) {
                    let ids: Vec<AnchorId> = self.selected.iter().copied().collect();
                    if let Some(new_path) = node_math::delete_anchors(&path, &ids) {
                        self.apply_path_edit(ctx, new_path);
                    }
                }
                self.selected.clear();
                self.overlay_response(ctx, CursorHint::Default)
            }
            LogicalKey::Escape => {
                if self.is_dragging() {
                    self.abort_drag(ctx);
                    return self.overlay_response(ctx, CursorHint::Default);
                }
                if !self.selected.is_empty() {
                    // First Escape: drop the anchor selection.
                    self.selected.clear();
                    return self.overlay_response(ctx, CursorHint::Default);
                }
                // Second Escape: exit back to the select tool.
                ToolResponse::exit().with_cursor(CursorHint::Default)
            }
            _ => ToolResponse::empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_canvas::SnapEngine;
    use fanta_doc::{CanvasNode, Doc, VectorNode, Viewport};

    fn press(screen: [f64; 2]) -> ToolEvent {
        press_mod(screen, ModifierKeys::empty())
    }
    fn press_mod(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers,
            count: 1,
        })
    }
    fn dbl(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Press {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
            count: 2,
        })
    }
    fn mv(screen: [f64; 2]) -> ToolEvent {
        mv_mod(screen, ModifierKeys::empty())
    }
    fn mv_mod(screen: [f64; 2], modifiers: ModifierKeys) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Move { screen, modifiers })
    }
    fn release(screen: [f64; 2]) -> ToolEvent {
        ToolEvent::Pointer(PointerEvent::Release {
            screen,
            button: Button::Primary,
            modifiers: ModifierKeys::empty(),
        })
    }
    fn key(k: LogicalKey) -> ToolEvent {
        ToolEvent::Key(KeyEvent::press(k))
    }

    fn ctx_pieces() -> (Doc, Viewport, SnapEngine, DVec2) {
        (
            Doc::new(),
            Viewport::default(),
            SnapEngine {
                zoom: 1.0,
                targets: fanta_canvas::SnapTargets::empty(),
                ..Default::default()
            },
            DVec2::new(800.0, 600.0),
        )
    }

    /// Identity viewport (800×600): world (0,0) is at screen (400,300).
    fn w2s(world: [f64; 2]) -> [f64; 2] {
        [world[0] + 400.0, world[1] + 300.0]
    }

    /// A vector polyline M(0,0) L(100,0) L(100,100) added to the doc.
    fn add_polyline(doc: &mut Doc) -> NodeId {
        let mut path = PathData::new();
        path.move_to(0.0, 0.0)
            .line_to(100.0, 0.0)
            .line_to(100.0, 100.0);
        let node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills: Default::default(),
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        }));
        let id = node.id;
        doc.apply(Operation::create_node(node)).expect("create");
        id
    }

    fn node_path(doc: &Doc, id: NodeId) -> PathData {
        match &doc.scene.get(id).expect("node").data {
            NodeData::Vector(v) => v.path.clone(),
            _ => panic!("expected vector"),
        }
    }

    fn tool_on(doc: &mut Doc, id: NodeId) -> NodeEditTool {
        doc.selection.select_only(id);
        NodeEditTool::new()
    }

    #[test]
    fn activate_adopts_selected_vector_as_target() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        assert_eq!(tool.target(), Some(id));
    }

    #[test]
    fn click_anchor_selects_it_and_emits_filled_overlay() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, press(w2s([100.0, 0.0])));
        let r = tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        assert_eq!(tool.selected_count(), 1);
        let filled = r
            .overlays
            .iter()
            .filter(|o| matches!(o, ToolOverlay::PathAnchor { selected: true, .. }))
            .count();
        let hollow = r
            .overlays
            .iter()
            .filter(|o| {
                matches!(
                    o,
                    ToolOverlay::PathAnchor {
                        selected: false,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(filled, 1);
        assert_eq!(hollow, 2);
    }

    #[test]
    fn shift_click_toggles_anchor_membership() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, press(w2s([0.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([0.0, 0.0])));
        tool.handle_event(&mut ctx, press_mod(w2s([100.0, 0.0]), ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        assert_eq!(tool.selected_count(), 2);
        // Shift-click a selected anchor removes it.
        tool.handle_event(&mut ctx, press_mod(w2s([100.0, 0.0]), ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        assert_eq!(tool.selected_count(), 1);
    }

    #[test]
    fn drag_anchor_moves_it_and_commits_one_undo_step() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        let undo_before = ctx.doc.history.undo_depth();
        tool.handle_event(&mut ctx, press(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, mv(w2s([120.0, -10.0])));
        assert!(tool.is_dragging());
        tool.handle_event(&mut ctx, mv(w2s([140.0, -20.0])));
        tool.handle_event(&mut ctx, release(w2s([140.0, -20.0])));
        let path = node_path(&doc, id);
        let anchors = node_math::enumerate_anchors(&path);
        assert!((anchors[1].pos - DVec2::new(140.0, -20.0)).length() < 1e-9);
        assert_eq!(
            doc.history.undo_depth(),
            undo_before + 1,
            "whole drag is one undo step"
        );
        // Undo restores the press-time geometry.
        doc.undo().expect("undo");
        let restored = node_math::enumerate_anchors(&node_path(&doc, id));
        assert!((restored[1].pos - DVec2::new(100.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn drag_moves_all_selected_anchors_together() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        // Select anchors 0 and 1, then drag anchor 1 by (+10, +5).
        tool.handle_event(&mut ctx, press(w2s([0.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([0.0, 0.0])));
        tool.handle_event(&mut ctx, press_mod(w2s([100.0, 0.0]), ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, mv_mod(w2s([110.0, 5.0]), ModifierKeys::SHIFT));
        tool.handle_event(&mut ctx, release(w2s([110.0, 5.0])));
        let anchors = node_math::enumerate_anchors(&node_path(&doc, id));
        assert!((anchors[0].pos - DVec2::new(10.0, 5.0)).length() < 1e-9);
        assert!((anchors[1].pos - DVec2::new(110.0, 5.0)).length() < 1e-9);
        assert!(
            (anchors[2].pos - DVec2::new(100.0, 100.0)).length() < 1e-9,
            "unselected anchor stays put"
        );
    }

    #[test]
    fn double_click_anchor_toggles_smooth_then_corner() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, dbl(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        let path = node_path(ctx.doc, id);
        assert!(node_math::anchor_is_smooth(
            &path,
            AnchorId {
                subpath: 0,
                index: 1
            }
        ));
        // Double-click again: back to a corner (handles retracted).
        tool.handle_event(&mut ctx, dbl(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        let path = node_path(ctx.doc, id);
        let a = node_math::enumerate_anchors(&path)[1];
        assert!(a.ctrl_in.is_none() && a.ctrl_out.is_none());
    }

    #[test]
    fn handle_drag_mirrors_when_smooth_and_alt_breaks() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        // Make anchor 1 smooth, then select it so its handles are visible.
        tool.handle_event(&mut ctx, dbl(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        let path = node_path(ctx.doc, id);
        let a = node_math::enumerate_anchors(&path)[1];
        let out_ctrl = a.ctrl_out.expect("smooth out handle");
        // Drag the OUT handle somewhere new (mirror expected).
        tool.handle_event(&mut ctx, press(w2s([out_ctrl.x, out_ctrl.y])));
        tool.handle_event(&mut ctx, mv(w2s([140.0, 40.0])));
        tool.handle_event(&mut ctx, release(w2s([140.0, 40.0])));
        let path = node_path(ctx.doc, id);
        let a = node_math::enumerate_anchors(&path)[1];
        assert!((a.ctrl_out.expect("out") - DVec2::new(140.0, 40.0)).length() < 1e-9);
        assert!(
            node_math::is_smooth(&node_math::AnchorPt {
                pos: a.pos,
                ctrl_in: a.ctrl_in,
                ctrl_out: a.ctrl_out,
            }),
            "mirrored drag keeps the anchor smooth"
        );
        // Now Alt-drag the same handle: the opposite handle must NOT follow.
        let frozen_in = a.ctrl_in.expect("in handle");
        tool.handle_event(&mut ctx, press(w2s([140.0, 40.0])));
        tool.handle_event(&mut ctx, mv_mod(w2s([150.0, -30.0]), ModifierKeys::ALT));
        tool.handle_event(&mut ctx, release(w2s([150.0, -30.0])));
        let path = node_path(ctx.doc, id);
        let a = node_math::enumerate_anchors(&path)[1];
        assert!((a.ctrl_out.expect("out") - DVec2::new(150.0, -30.0)).length() < 1e-9);
        assert!(
            (a.ctrl_in.expect("in") - frozen_in).length() < 1e-9,
            "alt drag breaks the mirror"
        );
    }

    #[test]
    fn click_segment_inserts_anchor_via_split() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        // Hover near the middle of the first edge: insert hint appears.
        let r = tool.handle_event(&mut ctx, mv(w2s([50.0, 3.0])));
        assert!(
            r.overlays
                .iter()
                .any(|o| matches!(o, ToolOverlay::PathInsertHint { .. })),
            "hovering a segment shows the + glyph"
        );
        // Click there: a new anchor lands at (50, 0).
        tool.handle_event(&mut ctx, press(w2s([50.0, 3.0])));
        tool.handle_event(&mut ctx, release(w2s([50.0, 3.0])));
        let anchors = node_math::enumerate_anchors(&node_path(&doc, id));
        assert_eq!(anchors.len(), 4);
        assert!((anchors[1].pos - DVec2::new(50.0, 0.0)).length() < 0.5);
        assert_eq!(tool.selected_count(), 1, "inserted anchor is selected");
    }

    #[test]
    fn delete_removes_selected_anchor_and_rejoins() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, press(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, key(LogicalKey::Delete));
        let anchors = node_math::enumerate_anchors(&node_path(&doc, id));
        assert_eq!(anchors.len(), 2);
        assert!((anchors[1].pos - DVec2::new(100.0, 100.0)).length() < 1e-9);
        assert_eq!(tool.selected_count(), 0);
    }

    #[test]
    fn marquee_selects_anchors_inside_rect() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        // Marquee around the two top anchors (0,0) and (100,0).
        tool.handle_event(&mut ctx, press(w2s([-20.0, -20.0])));
        let r = tool.handle_event(&mut ctx, mv(w2s([120.0, 20.0])));
        assert!(
            r.overlays
                .iter()
                .any(|o| matches!(o, ToolOverlay::Marquee { .. }))
        );
        tool.handle_event(&mut ctx, release(w2s([120.0, 20.0])));
        assert_eq!(tool.selected_count(), 2);
    }

    #[test]
    fn escape_clears_selection_then_exits() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, press(w2s([0.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([0.0, 0.0])));
        assert_eq!(tool.selected_count(), 1);
        let r1 = tool.handle_event(&mut ctx, key(LogicalKey::Escape));
        assert!(!r1.wants_exit, "first escape only drops the selection");
        assert_eq!(tool.selected_count(), 0);
        let r2 = tool.handle_event(&mut ctx, key(LogicalKey::Escape));
        assert!(r2.wants_exit, "second escape exits to select");
    }

    #[test]
    fn escape_mid_drag_aborts_without_history() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        let undo_before = ctx.doc.history.undo_depth();
        tool.handle_event(&mut ctx, press(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, mv(w2s([160.0, 40.0])));
        tool.handle_event(&mut ctx, key(LogicalKey::Escape));
        let anchors = node_math::enumerate_anchors(&node_path(&doc, id));
        assert!(
            (anchors[1].pos - DVec2::new(100.0, 0.0)).length() < 1e-9,
            "aborted drag restores the press-time path"
        );
        assert_eq!(doc.history.undo_depth(), undo_before, "no history entry");
    }

    #[test]
    fn press_adopts_vector_under_cursor_when_unselected() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let id = add_polyline(&mut doc);
        doc.selection.clear();
        let mut tool = NodeEditTool::new();
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        assert_eq!(tool.target(), None);
        tool.handle_event(&mut ctx, press(w2s([50.0, 50.0])));
        tool.handle_event(&mut ctx, release(w2s([50.0, 50.0])));
        assert_eq!(tool.target(), Some(id));
        assert!(doc.selection.contains(id));
    }

    #[test]
    fn handles_render_for_selected_anchor_and_neighbors() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        // A curvy path so every anchor owns handles.
        let mut path = PathData::new();
        path.move_to(0.0, 0.0)
            .cubic_to(10.0, 20.0, 40.0, 20.0, 50.0, 0.0)
            .cubic_to(60.0, -20.0, 90.0, -20.0, 100.0, 0.0)
            .cubic_to(110.0, 20.0, 140.0, 20.0, 150.0, 0.0);
        let node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills: Default::default(),
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        }));
        let id = node.id;
        doc.apply(Operation::create_node(node)).expect("create");
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        // Select the second anchor (50, 0): its handles + the neighbors' show.
        tool.handle_event(&mut ctx, press(w2s([50.0, 0.0])));
        let r = tool.handle_event(&mut ctx, release(w2s([50.0, 0.0])));
        let handle_count = r
            .overlays
            .iter()
            .filter(|o| matches!(o, ToolOverlay::PathHandle { .. }))
            .count();
        // Anchor 1 has in+out (2); neighbor 0 has out (1); neighbor 2 has
        // in+out (2) → 5 stems. The far anchor 3's handles stay hidden.
        assert_eq!(handle_count, 5);
    }

    #[test]
    fn delete_guard_keeps_open_path_at_two_anchors() {
        let (mut doc, mut viewport, snap, size) = ctx_pieces();
        let mut path = PathData::new();
        path.move_to(0.0, 0.0).line_to(100.0, 0.0);
        let node = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            fills: Default::default(),
            strokes: Default::default(),
            corner_radius: None,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        }));
        let id = node.id;
        doc.apply(Operation::create_node(node)).expect("create");
        let mut tool = tool_on(&mut doc, id);
        let mut ctx = ToolContext::new(&mut doc, &mut viewport, snap, size);
        tool.activate(&mut ctx);
        tool.handle_event(&mut ctx, press(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, release(w2s([100.0, 0.0])));
        tool.handle_event(&mut ctx, key(LogicalKey::Delete));
        assert_eq!(
            node_math::enumerate_anchors(&node_path(&doc, id)).len(),
            2,
            "guard refuses to drop below 2 anchors on an open path"
        );
    }
    fn viewport_curve_fixture() -> (Doc, NodeId, NodeData) {
        let mut doc = Doc::new();
        let mut path = PathData::new();
        path.move_to(0., 160.).quad_to(100., 0., 200., 160.);
        let vector = CanvasNode::new(NodeData::Vector(VectorNode {
            path,
            local_size: Some([200., 160.]),
            strokes: [fanta_doc::Stroke::solid(fanta_doc::Color::BLACK, 4.)]
                .into_iter()
                .collect(),
            ..Default::default()
        }));
        let original = vector.data.clone();
        let id = vector.id;
        doc.apply(Operation::create_node(vector))
            .expect("create clipped curve");
        doc.selection.select_only(id);
        doc.history = Default::default();
        (doc, id, original)
    }

    fn viewport_edit_tool(select_segments: bool) -> NodeEditTool {
        if select_segments {
            NodeEditTool::for_path_selection()
        } else {
            NodeEditTool::new()
        }
    }

    #[test]
    fn path_edit_viewport_detaches_during_drag_and_undo_restores_it() {
        for select_segments in [false, true] {
            for delta in [DVec2::splat(40.), DVec2::splat(-40.)] {
                let (mut doc, id, original) = viewport_curve_fixture();
                let mut viewport = Viewport::default();
                let mut ctx = ToolContext::new(
                    &mut doc,
                    &mut viewport,
                    SnapEngine::default(),
                    DVec2::new(800., 600.),
                );
                let mut tool = viewport_edit_tool(select_segments);
                tool.activate(&mut ctx);
                let start = DVec2::new(0., 160.);
                let end = start + delta;
                tool.handle_event(&mut ctx, press(w2s(start.to_array())));
                tool.handle_event(&mut ctx, mv(w2s(end.to_array())));
                let vector = ctx
                    .doc
                    .scene
                    .get(id)
                    .expect("curve")
                    .data
                    .as_vector()
                    .expect("vector");
                assert_eq!(
                    vector.local_size, None,
                    "an edited path must escape its old viewport during preview"
                );
                let anchors = node_math::enumerate_anchors(&vector.path);
                assert_eq!(anchors.first().expect("moved anchor").pos, end);
                assert_eq!(ctx.doc.history.undo_depth(), 0);
                tool.handle_event(&mut ctx, release(w2s(end.to_array())));
                let edited = ctx.doc.scene.get(id).expect("curve").data.clone();
                assert_eq!(edited.as_vector().expect("vector").local_size, None);
                assert_eq!(
                    ctx.doc.scene.get(id).expect("curve").transform,
                    Transform2D::IDENTITY
                );
                assert_eq!(ctx.doc.history.undo_depth(), 1);
                assert!(ctx.doc.undo().expect("undo path edit"));
                assert_eq!(ctx.doc.scene.get(id).expect("curve").data, original);
                assert!(ctx.doc.redo().expect("redo path edit"));
                assert_eq!(ctx.doc.scene.get(id).expect("curve").data, edited);
            }
        }
    }

    #[test]
    fn path_edit_viewport_click_and_subthreshold_move_preserve_import() {
        for select_segments in [false, true] {
            let (mut doc, id, original) = viewport_curve_fixture();
            let original_meta = serde_json::json!({"clip_content": true, "import_note": "keep"});
            doc.scene.get_mut(id).expect("curve").meta = original_meta.clone();
            let mut viewport = Viewport::default();
            let mut ctx = ToolContext::new(
                &mut doc,
                &mut viewport,
                SnapEngine::default(),
                DVec2::new(800., 600.),
            );
            let mut tool = viewport_edit_tool(select_segments);
            tool.activate(&mut ctx);
            for end in [[0., 160.], [1., 161.]] {
                tool.handle_event(&mut ctx, press(w2s([0., 160.])));
                tool.handle_event(&mut ctx, mv(w2s(end)));
                tool.handle_event(&mut ctx, release(w2s(end)));
                assert_eq!(ctx.doc.scene.get(id).expect("curve").data, original);
                assert_eq!(ctx.doc.scene.get(id).expect("curve").meta, original_meta);
                assert_eq!(ctx.doc.history.undo_depth(), 0);
            }
        }
    }

    #[test]
    fn path_edit_viewport_cancel_switch_and_return_restore_import_exactly() {
        for select_segments in [false, true] {
            for drag_handle in [false, true] {
                for (ending, original_meta) in [
                    (
                        "escape",
                        serde_json::json!({"clip_content": true, "import_note": "keep"}),
                    ),
                    (
                        "deactivate",
                        serde_json::json!({"clip_content": false, "nested": {"value": 7}}),
                    ),
                    ("return", serde_json::json!(["opaque", 42])),
                ] {
                    let (mut doc, id, original) = viewport_curve_fixture();
                    doc.scene.get_mut(id).expect("curve").meta = original_meta.clone();
                    let mut viewport = Viewport::default();
                    let mut ctx = ToolContext::new(
                        &mut doc,
                        &mut viewport,
                        SnapEngine::default(),
                        DVec2::new(800., 600.),
                    );
                    let mut tool = viewport_edit_tool(select_segments);
                    tool.activate(&mut ctx);
                    let start = if drag_handle {
                        tool.handle_event(&mut ctx, press(w2s([0., 160.])));
                        tool.handle_event(&mut ctx, release(w2s([0., 160.])));
                        node_math::enumerate_anchors(&node_path(ctx.doc, id))
                            .first()
                            .expect("first anchor")
                            .ctrl_out
                            .expect("quadratic handle")
                            .to_array()
                    } else {
                        [0., 160.]
                    };
                    tool.handle_event(&mut ctx, press(w2s(start)));
                    tool.handle_event(&mut ctx, mv(w2s([-40., 200.])));
                    assert_eq!(
                        ctx.doc
                            .scene
                            .get(id)
                            .expect("curve")
                            .data
                            .as_vector()
                            .expect("vector")
                            .local_size,
                        None
                    );
                    ctx.doc
                        .scene
                        .get_mut(id)
                        .expect("curve")
                        .flags
                        .insert(NodeFlags::EXCLUDE_FROM_AI);
                    match ending {
                        "escape" => {
                            tool.handle_event(&mut ctx, key(LogicalKey::Escape));
                        }
                        "deactivate" => tool.deactivate(&mut ctx),
                        "return" => {
                            tool.handle_event(&mut ctx, mv(w2s(start)));
                        }
                        _ => unreachable!(),
                    }
                    assert_eq!(
                        ctx.doc.scene.get(id).expect("curve").data,
                        original,
                        "{ending} must restore both authored quadratic and viewport"
                    );
                    assert_eq!(ctx.doc.scene.get(id).expect("curve").meta, original_meta);
                    assert_eq!(
                        ctx.doc.scene.get(id).expect("curve").flags,
                        NodeFlags::EXCLUDE_FROM_AI
                    );
                    let end = if ending == "return" {
                        start
                    } else {
                        [-40., 200.]
                    };
                    tool.handle_event(&mut ctx, release(w2s(end)));
                    assert_eq!(ctx.doc.scene.get(id).expect("curve").data, original);
                    assert_eq!(ctx.doc.scene.get(id).expect("curve").meta, original_meta);
                    assert_eq!(
                        ctx.doc.scene.get(id).expect("curve").flags,
                        NodeFlags::EXCLUDE_FROM_AI
                    );
                    assert_eq!(ctx.doc.history.undo_depth(), 0);
                }
            }
        }
    }

    #[test]
    fn path_edit_viewport_handle_drag_and_immediate_delete_are_undoable() {
        for select_segments in [false, true] {
            let (mut doc, id, original) = viewport_curve_fixture();
            let mut viewport = Viewport::default();
            let mut ctx = ToolContext::new(
                &mut doc,
                &mut viewport,
                SnapEngine::default(),
                DVec2::new(800., 600.),
            );
            let mut tool = viewport_edit_tool(select_segments);
            tool.activate(&mut ctx);
            tool.handle_event(&mut ctx, press(w2s([0., 160.])));
            tool.handle_event(&mut ctx, release(w2s([0., 160.])));
            let anchors = node_math::enumerate_anchors(&node_path(ctx.doc, id));
            let handle = anchors
                .first()
                .expect("anchor")
                .ctrl_out
                .expect("quadratic control handle");
            tool.handle_event(&mut ctx, press(w2s(handle.to_array())));
            tool.handle_event(&mut ctx, mv(w2s([-40., -40.])));
            assert_eq!(
                ctx.doc
                    .scene
                    .get(id)
                    .expect("curve")
                    .data
                    .as_vector()
                    .expect("vector")
                    .local_size,
                None
            );
            tool.handle_event(&mut ctx, release(w2s([-40., -40.])));
            assert_eq!(ctx.doc.history.undo_depth(), 1);
            assert!(ctx.doc.undo().expect("undo handle"));
            assert_eq!(ctx.doc.scene.get(id).expect("curve").data, original);

            let id = add_polyline(ctx.doc);
            ctx.doc
                .scene
                .get_mut(id)
                .expect("polyline")
                .data
                .as_vector_mut()
                .expect("vector")
                .local_size = Some([100., 100.]);
            ctx.doc.selection.select_only(id);
            let original = ctx.doc.scene.get(id).expect("polyline").data.clone();
            ctx.doc.history = Default::default();
            let mut tool = viewport_edit_tool(select_segments);
            tool.activate(&mut ctx);
            tool.handle_event(&mut ctx, press(w2s([100., 0.])));
            tool.handle_event(&mut ctx, release(w2s([100., 0.])));
            tool.handle_event(&mut ctx, key(LogicalKey::Delete));
            let vector = ctx
                .doc
                .scene
                .get(id)
                .expect("polyline")
                .data
                .as_vector()
                .expect("vector");
            assert_eq!(node_math::enumerate_anchors(&vector.path).len(), 2);
            assert_eq!(vector.local_size, None);
            assert_eq!(ctx.doc.history.undo_depth(), 1);
            assert!(ctx.doc.undo().expect("undo anchor delete"));
            assert_eq!(ctx.doc.scene.get(id).expect("polyline").data, original);
        }
    }
    #[test]
    fn path_edit_viewport_metadata_and_backfill_intent_share_one_undo_step() {
        for select_segments in [false, true] {
            for original_meta in [
                serde_json::json!({"clip_content": true, "import_note": "keep", "nested": {"value": 7}}),
                serde_json::json!(["opaque", 42]),
                serde_json::json!("opaque extension"),
                serde_json::Value::Null,
            ] {
                let (mut doc, id, _) = viewport_curve_fixture();
                let node = doc.scene.get_mut(id).expect("curve");
                node.meta = original_meta.clone();
                node.flags.insert(NodeFlags::EXCLUDE_FROM_AI);
                let original = node.clone();
                let mut viewport = Viewport::default();
                let mut ctx = ToolContext::new(
                    &mut doc,
                    &mut viewport,
                    SnapEngine::default(),
                    DVec2::new(800., 600.),
                );
                let mut tool = viewport_edit_tool(select_segments);
                tool.activate(&mut ctx);
                tool.handle_event(&mut ctx, press(w2s([0., 160.])));
                tool.handle_event(&mut ctx, mv(w2s([40., 200.])));
                let preview = ctx.doc.scene.get(id).expect("curve");
                assert_eq!(preview.meta, original_meta);
                assert_eq!(preview.flags, original.flags | NodeFlags::UNCLIPPED_VECTOR);
                tool.handle_event(&mut ctx, release(w2s([40., 200.])));
                assert_eq!(ctx.doc.history.undo_depth(), 1);
                fanta_doc::backfill_vector_viewports(&mut ctx.doc.scene);
                let edited = ctx.doc.scene.get(id).expect("curve");
                assert_eq!(edited.data.as_vector().expect("vector").local_size, None);
                assert_eq!(edited.meta, original_meta);
                assert_eq!(edited.flags, original.flags | NodeFlags::UNCLIPPED_VECTOR);
                assert!(ctx.doc.undo().expect("undo path and viewport intent"));
                let restored = ctx.doc.scene.get(id).expect("curve");
                assert_eq!(restored.data, original.data);
                assert_eq!(restored.meta, original.meta);
                assert_eq!(restored.flags, original.flags);
                assert!(ctx.doc.redo().expect("redo path and viewport intent"));
                let edited = ctx.doc.scene.get(id).expect("curve");
                assert_eq!(edited.data.as_vector().expect("vector").local_size, None);
                assert_eq!(edited.meta, original_meta);
                assert_eq!(edited.flags, original.flags | NodeFlags::UNCLIPPED_VECTOR);
            }
        }
    }
}

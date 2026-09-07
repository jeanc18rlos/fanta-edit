//! Core state for the select tool: the interaction-phase enum, the per-gesture
//! captured-state structs, tuning constants, and the [`SelectTool`] struct with
//! its phase-introspection helpers.
//!
//! The gesture behavior itself lives in sibling submodules (`dispatch`, `mv`,
//! `resize`, `rotate`, `reparent`); they all reach into the [`Phase`] and the
//! `*State` structs defined here, so those items are `pub(super)`.

use crate::tool::{SnapGuide, ToolOverlay};
use fanta_canvas::{MarqueeMode, ResizeHandle, RotateHandle, SnapCandidates, SnapResult};
use fanta_doc::{Bounds, NodeId, Transform2D};
use glam::DVec2;
use smallvec::SmallVec;

/// Phase the select tool is currently in. Stored explicitly so callers (and
/// tests) can introspect; mirrors tldraw's "interaction state."
#[derive(Debug, Clone)]
pub(super) enum Phase {
    /// No drag in progress.
    Idle,
    /// Button is down on a node; we haven't decided yet whether this is a
    /// move (drag exceeded the threshold) or a click (release without drag).
    PressedOnNode {
        screen_press: DVec2,
        target: NodeId,
        /// Whether the press carried an extend-selection modifier. Determines
        /// the click-release behavior (toggle vs. replace).
        extending: bool,
    },
    /// Press on empty space. The marquee preview shows even before the user
    /// has dragged — Figma reveals it as soon as you move past 1 pixel; we
    /// match that by switching to `Marquee` at the first `Move`.
    PressedOnEmpty { screen_press: DVec2 },
    /// Active marquee drag. `screen_current` is kept on the phase rather than
    /// recomputed each frame so the shell can later expose a "current marquee"
    /// query for hover-state UX even on idle frames between events.
    Marquee {
        screen_press: DVec2,
        #[allow(dead_code)]
        screen_current: DVec2,
        mode: MarqueeMode,
    },
    /// Active multi-node move. Boxed because the inner `MovingSelection`
    /// carries SmallVecs sized for the common case (8 selected nodes), which
    /// makes the variant the largest one and bloats every other variant's
    /// size when stored inline.
    Moving(Box<crate::tool::MovingSelection>),
    /// Active resize via a corner / edge handle. v0 only supports
    /// single-node selection so the math is unambiguous; multi-node resize
    /// requires picking a reference rectangle (selection bbox vs. first node)
    /// and that decision is its own design pass.
    Resizing(Box<ResizingState>),
    /// Active rotation via a corner rotation zone. Like resize, v0 rotates a
    /// single-node selection about its own bounding-box center so the
    /// transform is unambiguous; the pivot for a future multi-node rotate is
    /// the selection-bbox center, captured the same way.
    Rotating(Box<RotatingState>),
}

/// Per-resize state captured at press time so the math is reproducible on
/// every Move event.
#[derive(Debug, Clone)]
pub(super) struct ResizingState {
    pub(super) handle: ResizeHandle,
    pub(super) node_id: NodeId,
    /// Node-local bounds at press time. Defines the source rect the
    /// computed transform maps to the requested world rect each frame.
    /// The resize math works in the node's own (possibly rotated) frame from
    /// `original_transform` + these local bounds, so the press-time world AABB
    /// is no longer needed — see [`SelectTool::compute_resize_response`].
    ///
    /// [`SelectTool::compute_resize_response`]: super::SelectTool
    pub(super) original_local: Bounds,
    /// Node transform at press time. This is the immutable "old" value: the
    /// transient preview overwrites the node's transform each frame, and the
    /// single commit on release records `SetTransform { old, new: final }` so
    /// the whole resize undoes in one step. Also the value restored directly
    /// on Escape / abort.
    pub(super) original_transform: Transform2D,
    /// Complete node-local → world transform at press time. Resize math consumes
    /// world-space pointer coordinates, so nested nodes must use this composed
    /// transform rather than treating their parent-local transform as world.
    pub(super) original_world_transform: Transform2D,
    /// World → parent-local transform captured at press time. The resize helper
    /// returns a world transform; composing through this produces the node's new
    /// parent-local transform for preview and history.
    pub(super) parent_world_inverse: Transform2D,
    /// Effective "parent size" at press time for the purpose of constraints
    /// (clip/local size for Groups, local_size for Vectors, or local bounds fallback).
    /// Captured so that on commit we can apply child constraints when resizing
    /// a container (making imported constraints "live" during interactive resize).
    pub(super) original_size: [f64; 2],
    /// Snap candidates captured once at press. Stored for parity with the move
    /// gesture and to keep the collection out of the per-frame path; resize
    /// snapping itself is off in v0 (see [`SelectTool::compute_resize_response`]),
    /// so these are not consulted yet — but they are gesture-stable for the
    /// same reason the move ones are.
    ///
    /// [`SelectTool::compute_resize_response`]: super::SelectTool
    #[allow(dead_code)]
    pub(super) snap_candidates: SnapCandidates,
    /// `Some` when the node resizes its CONTENT BOX rather than its transform
    /// scale — text reflows and groups change their own box without scaling
    /// descendants. Holds the node's variant data at press time: the preview
    /// rewrites the box size each frame, and commit records a
    /// `ReplaceData { old: this, new: final }` alongside the transform op so the
    /// resize undoes in one step. `None` is the default scale-resize path.
    pub(super) box_data: Option<Box<fanta_doc::NodeData>>,
    /// Immediate-child transform normalization needed when an old plain group
    /// has no authored box. The group is rebased to its derived content origin
    /// and its children receive the inverse translation, preserving every
    /// descendant's world geometry while materializing a `[0, 0, w, h]` box.
    /// Preview writes `normalized`; cancel restores `original`; commit records
    /// the same changes beside the group resize in one transaction.
    pub(super) normalized_children: Vec<ResizeChildTransform>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResizeChildTransform {
    pub(super) id: NodeId,
    pub(super) original: Transform2D,
    pub(super) normalized: Transform2D,
}

/// Per-rotation state captured at press time so the math is reproducible on
/// every Move event. Mirrors [`ResizingState`]: the drag is a transient
/// preview (direct scene writes, no open transaction) and the single
/// `SetTransform` op is recorded on release.
#[derive(Debug, Clone)]
pub(super) struct RotatingState {
    #[allow(dead_code)]
    pub(super) handle: RotateHandle,
    pub(super) node_id: NodeId,
    /// World-space pivot the rotation turns about — the selected node's
    /// bounding-box center at press time.
    pub(super) pivot: DVec2,
    /// World-space cursor at press time. The swept angle is measured from this
    /// ray (pivot → press) to the current cursor ray each frame, so a tiny
    /// initial twitch doesn't jump the node.
    pub(super) press_world: DVec2,
    /// The node's rotation (radians) at press time. Used as the base for Shift
    /// 15°-increment snapping so the snapped orientation is absolute, not
    /// relative to wherever the drag started.
    pub(super) base_angle: f64,
    /// Node transform at press time — the immutable "old" value. The transient
    /// preview overwrites the node's transform each frame; release records
    /// `SetTransform { old, new: final }` so the whole rotation undoes in one
    /// step. Also the value restored directly on Escape / abort.
    pub(super) original_transform: Transform2D,
    /// Complete node-local → world transform at press time. The rotation is
    /// composed in world space so a transformed parent cannot skew the result.
    pub(super) original_world_transform: Transform2D,
    /// World → parent-local transform used to store the rigid world result.
    pub(super) parent_world_inverse: Transform2D,
}

/// Minimum screen pixels a drag must cover before we commit to a "move" or
/// "marquee" interpretation. 3 px matches Figma's threshold.
pub(super) const DRAG_THRESHOLD_PX: f64 = 3.0;

/// Arrow-nudge step in world units when no modifier is held.
pub(super) const NUDGE_SMALL: f64 = 1.0;

/// Arrow-nudge step with Shift — Figma's "big nudge" matches this multiplier.
pub(super) const NUDGE_LARGE: f64 = 10.0;

/// State machine for the select tool. Reusable across the app's lifetime —
/// fields reset between gestures via [`enter_idle`].
///
/// [`enter_idle`]: SelectTool::enter_idle
#[derive(Debug)]
pub struct SelectTool {
    pub(super) phase: Phase,
    /// The container the user has "entered" via double-click ("drill-in"
    /// scope). `None` = page level: a single click selects the outermost
    /// top-level frame under the cursor (Figma's container-first selection).
    /// `Some(id)` = clicks resolve to direct children of `id`. Persists across
    /// gestures (it is NOT reset by [`enter_idle`]); cleared on an empty-canvas
    /// click, on Escape, and when a click lands outside the entered subtree.
    pub(super) scope: Option<NodeId>,
}

impl Default for SelectTool {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectTool {
    /// New tool in the idle phase.
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            scope: None,
        }
    }

    /// The currently "entered" drill-in container, if any (exposed for tests).
    pub fn entered_scope(&self) -> Option<NodeId> {
        self.scope
    }

    /// Whether a marquee is currently being drawn (exposed for tests).
    pub fn is_marquee_active(&self) -> bool {
        matches!(self.phase, Phase::Marquee { .. })
    }

    /// Whether a move gesture is currently in progress.
    pub fn is_moving(&self) -> bool {
        matches!(self.phase, Phase::Moving(_))
    }

    /// Whether the tool is in the idle (no-drag) phase.
    pub fn is_idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    /// Whether a resize gesture is in progress (handle drag).
    pub fn is_resizing(&self) -> bool {
        matches!(self.phase, Phase::Resizing(_))
    }

    /// Whether a rotation gesture is in progress (rotation-zone drag).
    pub fn is_rotating(&self) -> bool {
        matches!(self.phase, Phase::Rotating(_))
    }

    pub(super) fn enter_idle(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Build the snap-guide overlays from a snap result, plus the existing
    /// preview overlay set, with no allocation when the snap was empty.
    pub(super) fn snap_overlays(snap_result: &SnapResult) -> SmallVec<[ToolOverlay; 4]> {
        let guides = SnapGuide::from_snap_result(snap_result);
        let mut out: SmallVec<[ToolOverlay; 4]> = SmallVec::new();
        for g in guides {
            out.push(g);
        }
        out
    }
}

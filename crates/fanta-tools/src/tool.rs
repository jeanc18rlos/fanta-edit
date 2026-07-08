//! The [`Tool`] trait — the interface every tool implements.
//!
//! ## Design
//!
//! Doc mutations go through [`ToolContext::doc.apply`] inside `handle_event`
//! directly. The return value of `handle_event` — a [`ToolResponse`] — is
//! purely *render hints*: transient overlays the UI must draw (selection box,
//! snap guides, in-flight preview), the cursor the shell should display, and
//! a flag the tool sets when it wants to be replaced (the rect tool, for
//! instance, may flip back to the select tool after committing).
//!
//! ## Why split mutation from rendering hints
//!
//! Mixing the two would make the tool layer responsible for the visual style
//! of overlays, which `fanta-render` and `fanta-app` should own. By returning
//! semantic descriptions ("there is a marquee rectangle here," "snap guide
//! along x=42") the shell can render them with whichever visual treatment
//! matches the app's theme.
//!
//! Mirrors tldraw's tool state-machine separation and OpenPencil #1's split
//! between input-binding and effect-application layers — they ship the same
//! shape with different naming.

use crate::context::ToolContext;
use crate::event::ToolEvent;
use fanta_canvas::{SnapCandidates, SnapResult};
use fanta_doc::{Bounds, NodeId};
use glam::DVec2;
use smallvec::SmallVec;

/// A tool's reaction to one event.
///
/// Always carry-everything: the response is small (`SmallVec`-backed) and
/// constructing one per event is cheap. Returning `default()` is the "I had
/// nothing to say" response.
#[derive(Debug, Clone, Default)]
pub struct ToolResponse {
    /// Transient overlays the renderer should draw on top of the canvas this
    /// frame. Examples: a marquee selection rectangle, a snap guide line, a
    /// shape preview during a drag.
    pub overlays: SmallVec<[ToolOverlay; 4]>,

    /// Cursor the shell should display. `None` = keep the current cursor.
    pub cursor: Option<CursorHint>,

    /// The tool is done and wants the shell to switch back to the previous
    /// tool (typically the select tool). Set by shape tools after a commit,
    /// matching Figma's "single-shot mode" — one shape, then back to select.
    pub wants_exit: bool,
}

impl ToolResponse {
    /// Common shorthand for "I have nothing to say." Equivalent to
    /// [`Self::default`], but reads better at the end of a `match` arm.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Shorthand: response that only carries a cursor hint.
    pub fn cursor(cursor: CursorHint) -> Self {
        Self {
            cursor: Some(cursor),
            ..Self::default()
        }
    }

    /// Shorthand: response that signals the tool wants to be replaced.
    pub fn exit() -> Self {
        Self {
            wants_exit: true,
            ..Self::default()
        }
    }

    /// Push an overlay onto the response (builder style).
    pub fn with_overlay(mut self, overlay: ToolOverlay) -> Self {
        self.overlays.push(overlay);
        self
    }

    /// Replace the cursor hint (builder style).
    pub fn with_cursor(mut self, cursor: CursorHint) -> Self {
        self.cursor = Some(cursor);
        self
    }
}

/// A semantic description of an overlay. The renderer turns this into pixels
/// according to the app's visual theme.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolOverlay {
    /// A marquee-selection rectangle in **screen** coordinates. Drawn with the
    /// app's selection-rect treatment (typically a 1-px tinted outline + a
    /// translucent fill).
    Marquee { screen_rect: Bounds },

    /// An axis-aligned snap guide spanning the viewport. Coordinates are in
    /// **world** space — the renderer projects to screen and clips.
    SnapGuide(SnapGuide),

    /// A live preview of a rectangle the user is dragging. In **world** space
    /// because the rect tool produces world-space coords.
    PreviewRect { world_rect: Bounds },

    /// A live preview of an ellipse the user is dragging. World-space; the
    /// renderer treats the rect as the ellipse's bounding box.
    PreviewEllipse { world_rect: Bounds },

    /// A live preview of a line segment in world space.
    ///
    /// Reused by the polygon and star tools to preview their in-flight outline:
    /// each emits one `PreviewLine` per edge of the resolved corner ring, so the
    /// renderer draws the full N-gon / star outline as a fan of dashed
    /// segments without needing a dedicated polygon/star overlay variant. See
    /// [`crate::polygon::polyline_overlays`].
    PreviewLine {
        world_start: [f64; 2],
        world_end: [f64; 2],
    },

    /// A path anchor square (node-edit tool) at a world position, drawn at a
    /// fixed screen size. `selected` anchors render filled with the accent;
    /// unselected ones hollow (white fill + accent border), matching the
    /// selection-handle treatment. // track node-tool
    PathAnchor { world: [f64; 2], selected: bool },

    /// A Bézier control handle (node-edit tool): a 1-px stem from the anchor's
    /// world position to the control's, with a small circle at the control.
    PathHandle {
        world_anchor: [f64; 2],
        world_ctrl: [f64; 2],
    },

    /// An insert-anchor hint — the pen "+" glyph — at a world position on a
    /// path segment the cursor is hovering (node-edit tool).
    PathInsertHint { world: [f64; 2] },
}

/// One snap guide — the world-space line the renderer should draw to indicate
/// which candidate caught.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapGuide {
    pub axis: SnapGuideAxis,
    /// Position along the perpendicular axis. For a vertical guide this is `x`;
    /// for a horizontal guide it's `y`.
    pub world_position: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapGuideAxis {
    Vertical,
    Horizontal,
}

impl SnapGuide {
    /// Build a guide pair from a snap result. Returns up to two guides — one
    /// per axis — depending on which axes snapped.
    ///
    /// Encapsulating this here keeps the tool implementations free of the
    /// "construct guides from snap result" boilerplate, which is identical
    /// across every tool that snaps.
    pub fn from_snap_result(result: &SnapResult) -> SmallVec<[ToolOverlay; 2]> {
        let mut out: SmallVec<[ToolOverlay; 2]> = SmallVec::new();
        if let Some(s) = result.x {
            out.push(ToolOverlay::SnapGuide(SnapGuide {
                axis: SnapGuideAxis::Vertical,
                world_position: s.at,
            }));
        }
        if let Some(s) = result.y {
            out.push(ToolOverlay::SnapGuide(SnapGuide {
                axis: SnapGuideAxis::Horizontal,
                world_position: s.at,
            }));
        }
        out
    }
}

/// Cursor the shell should display. The list intentionally tracks Figma's
/// cursor vocabulary; the renderer maps to native cursors per-platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorHint {
    /// Default arrow.
    Default,
    /// Crosshair — used during shape draws and the marquee.
    Crosshair,
    /// Move/grab — used when hovering a selectable node.
    Move,
    /// Open hand — the hand tool when idle.
    Grab,
    /// Closed hand — the hand tool mid-drag.
    Grabbing,
    /// Text-entry I-beam (reserved for the future text tool).
    Text,
}

/// The interface every tool implements.
///
/// Tools are tiny state machines. Each one keeps its own per-gesture state
/// (e.g. "anchor world position," "preview rect") as struct fields and
/// transitions those fields in `handle_event` based on the input shape and
/// the modifier set.
pub trait Tool {
    /// Stable identifier for the tool — used in tracing logs and by the shell
    /// for active-tool indicators.
    fn name(&self) -> &'static str;

    /// Process one event. Mutate the [`ToolContext`] in-place (via
    /// `ctx.doc.apply`, `ctx.viewport`, etc.) and return rendering hints.
    fn handle_event(&mut self, ctx: &mut ToolContext, event: ToolEvent) -> ToolResponse;

    /// Hook called when the tool becomes active. Defaults to no-op. Tools that
    /// must reset internal state on (re-)activation override this.
    fn activate(&mut self, _ctx: &mut ToolContext) {}

    /// Hook called when the tool is being deactivated (the user switched to
    /// another tool). Tools that have an in-flight gesture must abort it here
    /// so the doc doesn't end up in a half-committed state.
    fn deactivate(&mut self, _ctx: &mut ToolContext) {}
}

/// Smallvec helper used by tool tests to build a Bounds from two screen-space
/// points. Kept here (rather than as a free function in `event.rs`) because
/// the canonical use is "compute the marquee bounds for a preview overlay."
pub fn bounds_from_corners(a: DVec2, b: DVec2) -> Bounds {
    let min = a.min(b);
    let max = a.max(b);
    Bounds::from_min_max(min, max)
}

/// Convenience selector type used by tools that maintain an internal selection
/// of recently-moved nodes (the select tool uses this during a drag).
#[derive(Debug, Default, Clone)]
pub struct MovingSelection {
    /// World position the press happened at — the anchor for delta calculations.
    /// Fixed for the whole gesture; the per-frame delta is computed relative to
    /// it from the *press-time* transforms, so the drag never compounds.
    pub press_world: DVec2,
    /// The nodes being moved paired with each node's transform at press time.
    ///
    /// ## Why a paired vec instead of two parallel ones
    ///
    /// The drag writes `old.then(translation)` to each node every frame and the
    /// release records one `SetTransform { old, new: final }` per node, so each
    /// id MUST stay matched to *its own* press-time transform. Storing them as
    /// `(id, transform)` tuples makes that pairing structural: a node that fails
    /// `scene.get` at capture time is dropped as a unit and can never shift the
    /// id↔transform correspondence of the survivors. The previous design kept
    /// two `zip`-ed `SmallVec`s built by independent passes, where one missing
    /// node desynced every later pair (node K inherited node K+1's transform).
    ///
    /// ## Why only top-level selected nodes live here
    ///
    /// This holds the selection's *roots* — selected nodes with no selected
    /// ancestor. Descendants of a moved node inherit the parent's transform, so
    /// translating a child whose ancestor is also moving would shift it twice
    /// (~2× delta) and tear it out of its frame. Pruning to top-level nodes is
    /// the correct Figma move semantic: the subtree travels with its root.
    pub moving: SmallVec<[(NodeId, fanta_doc::Transform2D); 8]>,
    /// The first moved node's world bounds at press time. Snapping nudges this
    /// box (translated by the running delta) against the cached candidates.
    /// Captured once because the transient preview overwrites the node's live
    /// transform every frame, so the live bounds can't stand in for the
    /// press-time bounds.
    pub first_press_bounds: Option<Bounds>,
    /// Snap candidates captured once at press. The non-dragged neighbors don't
    /// move during the gesture, so this snapshot stays valid for every frame —
    /// reusing it replaces the per-frame full-scene walk with an O(neighbors)
    /// query. See [`fanta_canvas::SnapCandidates`].
    pub snap_candidates: SnapCandidates,
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_canvas::AxisSnap;
    use fanta_canvas::SnapKind;

    #[test]
    fn snap_guide_from_result_emits_per_axis() {
        let mut r = SnapResult {
            world: DVec2::new(10.0, 20.0),
            x: Some(AxisSnap {
                kind: SnapKind::Grid,
                at: 10.0,
                delta: 0.0,
            }),
            y: None,
        };
        let g = SnapGuide::from_snap_result(&r);
        assert_eq!(g.len(), 1);
        match &g[0] {
            ToolOverlay::SnapGuide(s) => {
                assert_eq!(s.axis, SnapGuideAxis::Vertical);
                assert!((s.world_position - 10.0).abs() < 1e-9);
            }
            _ => panic!("expected snap guide"),
        }
        r.y = Some(AxisSnap {
            kind: SnapKind::Grid,
            at: 20.0,
            delta: 0.0,
        });
        let g2 = SnapGuide::from_snap_result(&r);
        assert_eq!(g2.len(), 2);
    }

    #[test]
    fn response_default_is_empty() {
        let r = ToolResponse::empty();
        assert!(r.overlays.is_empty());
        assert!(r.cursor.is_none());
        assert!(!r.wants_exit);
    }

    #[test]
    fn response_builders_compose() {
        let r = ToolResponse::cursor(CursorHint::Crosshair).with_overlay(ToolOverlay::Marquee {
            screen_rect: Bounds::from_xywh(0.0, 0.0, 10.0, 10.0),
        });
        assert_eq!(r.cursor, Some(CursorHint::Crosshair));
        assert_eq!(r.overlays.len(), 1);
    }

    #[test]
    fn bounds_from_corners_handles_inverted_drag() {
        // Press at (50, 50), drag to (10, 10) — the rect should still be valid.
        let b = bounds_from_corners(DVec2::new(50.0, 50.0), DVec2::new(10.0, 10.0));
        assert_eq!(b.min_x, 10.0);
        assert_eq!(b.max_x, 50.0);
        assert_eq!(b.min_y, 10.0);
        assert_eq!(b.max_y, 50.0);
    }

    #[test]
    fn exit_response_signals_wants_exit() {
        let r = ToolResponse::exit();
        assert!(r.wants_exit);
    }
}

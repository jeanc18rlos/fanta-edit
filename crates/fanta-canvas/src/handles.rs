//! Resize-handle geometry.
//!
//! The select tool puts 8 grab points around a selected node's world bounds —
//! 4 corners and 4 edge midpoints. This module computes their **screen-space**
//! positions (so the renderer and the hit-test agree) and provides the math
//! to convert a cursor drag on a handle into new world bounds.
//!
//! ## Why screen-space hit-test
//!
//! Handles look fixed-size on the screen at all zoom levels. The hit-test
//! threshold is expressed in screen pixels for the same reason snap
//! thresholds are: behavior must be zoom-stable so the handle stays equally
//! easy to grab whether you're at 25% or 4000% zoom.

use crate::viewport::{screen_to_world, world_to_screen};
use fanta_doc::{Bounds, Transform2D, Viewport};
use glam::DVec2;
use serde::{Deserialize, Serialize};

/// One of the 8 resize handles around a selection's bounding rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeHandle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl ResizeHandle {
    /// All 8 handles in clockwise order starting at the top-left. Useful for
    /// `for h in ResizeHandle::ALL` iteration during draw / hit-test.
    pub const ALL: [ResizeHandle; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    /// Just the four corners. The select tool may want to ignore edge
    /// handles in compact-selection modes.
    pub const CORNERS: [ResizeHandle; 4] = [
        Self::NorthWest,
        Self::NorthEast,
        Self::SouthEast,
        Self::SouthWest,
    ];

    /// Whether dragging this handle changes the X axis.
    pub fn affects_x(self) -> bool {
        !matches!(self, Self::North | Self::South)
    }

    /// Whether dragging this handle changes the Y axis.
    pub fn affects_y(self) -> bool {
        !matches!(self, Self::East | Self::West)
    }

    /// The handle directly opposite — useful for "anchor at opposite corner."
    pub fn opposite(self) -> Self {
        match self {
            Self::NorthWest => Self::SouthEast,
            Self::North => Self::South,
            Self::NorthEast => Self::SouthWest,
            Self::East => Self::West,
            Self::SouthEast => Self::NorthWest,
            Self::South => Self::North,
            Self::SouthWest => Self::NorthEast,
            Self::West => Self::East,
        }
    }

    /// World-space anchor point: the point on `bounds` that stays fixed when
    /// this handle is dragged. For corners it's the opposite corner; for
    /// edges it's the opposite midpoint.
    pub fn anchor_world(self, bounds: Bounds) -> DVec2 {
        match self {
            Self::NorthWest => DVec2::new(bounds.max_x, bounds.max_y),
            Self::North => DVec2::new(bounds.center().x, bounds.max_y),
            Self::NorthEast => DVec2::new(bounds.min_x, bounds.max_y),
            Self::East => DVec2::new(bounds.min_x, bounds.center().y),
            Self::SouthEast => DVec2::new(bounds.min_x, bounds.min_y),
            Self::South => DVec2::new(bounds.center().x, bounds.min_y),
            Self::SouthWest => DVec2::new(bounds.max_x, bounds.min_y),
            Self::West => DVec2::new(bounds.max_x, bounds.center().y),
        }
    }

    /// World-space position of this handle on the bounds. The "moved" corner
    /// during a drag; what the cursor latches onto.
    pub fn handle_world(self, bounds: Bounds) -> DVec2 {
        match self {
            Self::NorthWest => DVec2::new(bounds.min_x, bounds.min_y),
            Self::North => DVec2::new(bounds.center().x, bounds.min_y),
            Self::NorthEast => DVec2::new(bounds.max_x, bounds.min_y),
            Self::East => DVec2::new(bounds.max_x, bounds.center().y),
            Self::SouthEast => DVec2::new(bounds.max_x, bounds.max_y),
            Self::South => DVec2::new(bounds.center().x, bounds.max_y),
            Self::SouthWest => DVec2::new(bounds.min_x, bounds.max_y),
            Self::West => DVec2::new(bounds.min_x, bounds.center().y),
        }
    }
}

/// One of the 4 rotation affordances around a selection. Each sits **outside**
/// the matching corner resize handle — the standard Figma "grab the air just
/// past the corner to rotate" gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotateHandle {
    NorthWest,
    NorthEast,
    SouthEast,
    SouthWest,
}

impl RotateHandle {
    /// All 4 rotation zones, clockwise from the top-left.
    pub const ALL: [RotateHandle; 4] = [
        Self::NorthWest,
        Self::NorthEast,
        Self::SouthEast,
        Self::SouthWest,
    ];

    /// The corner resize handle this rotation zone wraps. The rotation zone is
    /// positioned just outside this corner along the outward diagonal.
    pub fn corner(self) -> ResizeHandle {
        match self {
            Self::NorthWest => ResizeHandle::NorthWest,
            Self::NorthEast => ResizeHandle::NorthEast,
            Self::SouthEast => ResizeHandle::SouthEast,
            Self::SouthWest => ResizeHandle::SouthWest,
        }
    }

    /// Outward diagonal unit-ish direction (in screen space, y-down) pointing
    /// away from the selection center toward this corner. Used to offset the
    /// rotation zone past the corner handle.
    fn outward(self) -> DVec2 {
        match self {
            Self::NorthWest => DVec2::new(-1.0, -1.0),
            Self::NorthEast => DVec2::new(1.0, -1.0),
            Self::SouthEast => DVec2::new(1.0, 1.0),
            Self::SouthWest => DVec2::new(-1.0, 1.0),
        }
    }
}

/// Default handle hit-test threshold in screen pixels.
pub const DEFAULT_HANDLE_THRESHOLD: f64 = 8.0;

/// How far (screen pixels) past a corner resize handle the rotation zone's
/// center sits, along the outward diagonal. Figma puts the rotate ring a small
/// gap outside the corner; this matches that feel and keeps the zone clear of
/// the resize handle so the two never fight for the same pixel.
pub const ROTATE_HANDLE_OFFSET: f64 = 14.0;

/// Default rotation-zone hit radius in screen pixels. Slightly larger than the
/// resize threshold because the zone is an open ring of empty space, not a
/// drawn square — a generous catch radius is the Figma feel.
pub const DEFAULT_ROTATE_THRESHOLD: f64 = 12.0;

/// Return the handle in `handles` whose screen position (via `pos_fn`) is
/// closest to `screen_point` and within `threshold` screen pixels; `None`
/// otherwise. Closest-wins (rather than first-match) so overlapping handles on
/// small selections never produce surprises. Shared by every resize/rotate
/// hit-test, axis-aligned and oriented alike.
fn closest_within<H: Copy>(
    handles: &[H],
    screen_point: DVec2,
    threshold: f64,
    pos_fn: impl Fn(H) -> DVec2,
) -> Option<H> {
    let mut best: Option<(H, f64)> = None;
    for &h in handles {
        let d = (pos_fn(h) - screen_point).length();
        if d <= threshold && best.is_none_or(|(_, prev)| d < prev) {
            best = Some((h, d));
        }
    }
    best.map(|(h, _)| h)
}

/// Screen-space position of `handle` on the (world-space) `bounds`. Returns
/// **logical** screen pixels; the renderer scales to physical separately. This
/// is [`handle_screen_position_oriented`] with an identity transform (an
/// axis-aligned box is an oriented box that happens not to be rotated).
pub fn handle_screen_position(
    handle: ResizeHandle,
    world_bounds: Bounds,
    viewport: &Viewport,
    screen_size: DVec2,
) -> DVec2 {
    handle_screen_position_oriented(
        handle,
        world_bounds,
        &Transform2D::IDENTITY,
        viewport,
        screen_size,
    )
}

/// Hit-test the 8 handles for a single world-space rectangle. Returns the
/// handle whose screen position is closest to `screen_point` and within
/// `threshold` screen-pixels; `None` otherwise.
pub fn hit_test_resize_handle(
    world_bounds: Bounds,
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
    threshold: f64,
) -> Option<ResizeHandle> {
    closest_within(&ResizeHandle::ALL, screen_point, threshold, |h| {
        handle_screen_position(h, world_bounds, viewport, screen_size)
    })
}

/// Screen-space center of `handle`'s rotation zone for the (world-space)
/// `world_bounds`. The zone sits `ROTATE_HANDLE_OFFSET` screen pixels past the
/// matching corner handle along the outward screen diagonal — so it tracks the
/// corner under pan/zoom and stays a fixed visual gap outside it regardless of
/// world rotation (the offset is applied in screen space).
pub fn rotate_handle_screen_position(
    handle: RotateHandle,
    world_bounds: Bounds,
    viewport: &Viewport,
    screen_size: DVec2,
) -> DVec2 {
    let corner = handle_screen_position(handle.corner(), world_bounds, viewport, screen_size);
    let out = handle.outward();
    let len = out.length().max(f64::EPSILON);
    corner + (out / len) * ROTATE_HANDLE_OFFSET
}

/// Hit-test the 4 rotation zones for a single world-space rectangle. Returns
/// the rotation handle whose screen zone-center is closest to `screen_point`
/// and within `threshold` screen-pixels; `None` otherwise.
///
/// Closest-wins for the same reason [`hit_test_resize_handle`] uses it. The
/// caller is expected to consult the resize handles *first* (a press right on
/// the corner square is a resize); only when that misses does the surrounding
/// rotation ring catch — matching Figma's precedence.
pub fn hit_test_rotate_handle(
    world_bounds: Bounds,
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
    threshold: f64,
) -> Option<RotateHandle> {
    closest_within(&RotateHandle::ALL, screen_point, threshold, |h| {
        rotate_handle_screen_position(h, world_bounds, viewport, screen_size)
    })
}

// --- Oriented (rotation-following) handle geometry ---------------------------
//
// The functions above place handles on a node's axis-aligned WORLD bounds (an
// AABB). When the node carries a rotation, the selection box should follow that
// rotation — an oriented box. These variants take the node's LOCAL bounds + its
// WORLD transform (rotation/scale folded in) and project each handle's local
// point through the transform, so the corners track the rotated box. Screen-space
// thresholds + closest-wins are reused verbatim.

/// Screen position of `handle` for a node given its LOCAL bounds + WORLD
/// transform — the oriented counterpart of [`handle_screen_position`]. Reuses
/// [`ResizeHandle::handle_world`] to get the point on the local box, then pushes
/// it through `world_t` (rotation included) before projecting to screen.
pub fn handle_screen_position_oriented(
    handle: ResizeHandle,
    local: Bounds,
    world_t: &Transform2D,
    viewport: &Viewport,
    screen_size: DVec2,
) -> DVec2 {
    let world = world_t.transform_point(handle.handle_world(local));
    world_to_screen(world, viewport, screen_size)
}

/// Oriented counterpart of [`hit_test_resize_handle`] — picks the closest of the
/// 8 handles on the rotated box within `threshold` screen px.
pub fn hit_test_resize_handle_oriented(
    local: Bounds,
    world_t: &Transform2D,
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
    threshold: f64,
) -> Option<ResizeHandle> {
    closest_within(&ResizeHandle::ALL, screen_point, threshold, |h| {
        handle_screen_position_oriented(h, local, world_t, viewport, screen_size)
    })
}

/// Oriented counterpart of [`rotate_handle_screen_position`]: the zone sits
/// `ROTATE_HANDLE_OFFSET` screen px outside the corner along the box's OWN
/// outward diagonal (derived from `corner − center` in screen space), so the
/// rotation ring follows the rotated box.
pub fn rotate_handle_screen_position_oriented(
    handle: RotateHandle,
    local: Bounds,
    world_t: &Transform2D,
    viewport: &Viewport,
    screen_size: DVec2,
) -> DVec2 {
    let corner =
        handle_screen_position_oriented(handle.corner(), local, world_t, viewport, screen_size);
    let center = world_to_screen(
        world_t.transform_point(local.center()),
        viewport,
        screen_size,
    );
    let out = corner - center;
    let len = out.length().max(f64::EPSILON);
    corner + (out / len) * ROTATE_HANDLE_OFFSET
}

/// Oriented counterpart of [`hit_test_rotate_handle`].
pub fn hit_test_rotate_handle_oriented(
    local: Bounds,
    world_t: &Transform2D,
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
    threshold: f64,
) -> Option<RotateHandle> {
    closest_within(&RotateHandle::ALL, screen_point, threshold, |h| {
        rotate_handle_screen_position_oriented(h, local, world_t, viewport, screen_size)
    })
}

/// Snap increment for rotation when Shift is held — 15° in radians, matching
/// Figma's rotate-snap.
pub const ROTATE_SNAP_RADIANS: f64 = std::f64::consts::PI / 12.0;

/// The signed angle (radians) of the vector `pivot → point`, measured in the
/// same screen/world convention the caller passes points in. Returns `None`
/// when `point` coincides with `pivot` (no defined direction).
pub fn angle_about(pivot: DVec2, point: DVec2) -> Option<f64> {
    let v = point - pivot;
    if v.length() < f64::EPSILON {
        None
    } else {
        Some(v.y.atan2(v.x))
    }
}

/// Compute the rotation delta (radians) for a rotate drag about `pivot`.
///
/// `press_point` is where the gesture started, `cursor_point` is the current
/// cursor — both in the **same** coordinate space (world space, so a positive
/// delta is a mathematically positive rotation in doc coords). The returned
/// value is the angle swept from press to cursor.
///
/// When `snap` is set, the **absolute** orientation (the base angle the drag
/// started at, plus the swept delta) is rounded to the nearest
/// [`ROTATE_SNAP_RADIANS`] and the delta needed to reach that snapped
/// orientation is returned — so the selection lands on clean 15° multiples
/// relative to its press-time angle, the Figma behavior.
pub fn rotation_delta(
    pivot: DVec2,
    press_point: DVec2,
    cursor_point: DVec2,
    base_angle: f64,
    snap: bool,
) -> f64 {
    let Some(a0) = angle_about(pivot, press_point) else {
        return 0.0;
    };
    let Some(a1) = angle_about(pivot, cursor_point) else {
        return 0.0;
    };
    let mut delta = a1 - a0;
    if snap {
        // Round the resulting absolute orientation to the nearest increment,
        // then back out the delta that achieves it.
        let target = base_angle + delta;
        let snapped = (target / ROTATE_SNAP_RADIANS).round() * ROTATE_SNAP_RADIANS;
        delta = snapped - base_angle;
    }
    delta
}

/// Build a transform that rotates by `radians` about the world-space `pivot`:
/// translate the pivot to the origin, rotate, translate back. Compose this
/// *after* a node's existing world transform to rotate it in place.
pub fn rotate_about(pivot: DVec2, radians: f64) -> Transform2D {
    Transform2D::translation(-pivot.x, -pivot.y)
        .then(&Transform2D::rotation(radians))
        .then(&Transform2D::translation(pivot.x, pivot.y))
}

/// Compute new world bounds during a resize drag.
///
/// `original` is the world bounds captured at press time. `handle` is the
/// handle being dragged. `cursor_world` is the current cursor position in
/// world space (mapped through `screen_to_world` for screen drags).
///
/// Modifiers:
/// - `lock_aspect` (Shift): preserve the original aspect ratio.
/// - `from_center` (Alt): the anchor becomes the original center, not the
///   opposite corner / edge.
pub fn resize_bounds(
    original: Bounds,
    handle: ResizeHandle,
    cursor_world: DVec2,
    lock_aspect: bool,
    from_center: bool,
) -> Bounds {
    let anchor = if from_center {
        original.center()
    } else {
        handle.anchor_world(original)
    };
    let handle_pt = handle.handle_world(original);

    // Free deltas on each axis. If the handle doesn't affect an axis, the
    // delta on that axis is zero (edges resize one axis only).
    let dx = if handle.affects_x() {
        cursor_world.x - handle_pt.x
    } else {
        0.0
    };
    let dy = if handle.affects_y() {
        cursor_world.y - handle_pt.y
    } else {
        0.0
    };

    // New handle position after applying deltas.
    let mut new_handle = DVec2::new(handle_pt.x + dx, handle_pt.y + dy);

    if lock_aspect && handle.affects_x() && handle.affects_y() {
        let orig_w = original.width().max(f64::EPSILON);
        let orig_h = original.height().max(f64::EPSILON);
        let aspect = orig_w / orig_h;
        let signed_w = new_handle.x - anchor.x;
        let signed_h = new_handle.y - anchor.y;
        let abs_w = signed_w.abs();
        let abs_h = signed_h.abs();
        let target_w;
        let target_h;
        if abs_w / aspect > abs_h {
            target_w = abs_w;
            target_h = abs_w / aspect;
        } else {
            target_h = abs_h;
            target_w = abs_h * aspect;
        }
        new_handle.x = anchor.x + target_w * signed_w.signum().max(-1.0).max(0.0).max(0.0);
        // signum can be 0 for zero — use the original sign instead.
        new_handle.x = anchor.x + if signed_w >= 0.0 { target_w } else { -target_w };
        new_handle.y = anchor.y + if signed_h >= 0.0 { target_h } else { -target_h };
    }

    // Build the new bounds from the (possibly aspect-locked) handle + anchor.
    let mut bx0 = anchor.x.min(new_handle.x);
    let mut by0 = anchor.y.min(new_handle.y);
    let mut bx1 = anchor.x.max(new_handle.x);
    let mut by1 = anchor.y.max(new_handle.y);

    if from_center {
        // In from-center mode, the new bounds are mirrored around the anchor.
        // The handle reach defines half-extents on its affected axes.
        let hx = (new_handle.x - anchor.x).abs();
        let hy = (new_handle.y - anchor.y).abs();
        if handle.affects_x() {
            bx0 = anchor.x - hx;
            bx1 = anchor.x + hx;
        } else {
            bx0 = original.min_x;
            bx1 = original.max_x;
        }
        if handle.affects_y() {
            by0 = anchor.y - hy;
            by1 = anchor.y + hy;
        } else {
            by0 = original.min_y;
            by1 = original.max_y;
        }
    } else {
        // For edge handles, the non-affected axis stays at the original.
        if !handle.affects_x() {
            bx0 = original.min_x;
            bx1 = original.max_x;
        }
        if !handle.affects_y() {
            by0 = original.min_y;
            by1 = original.max_y;
        }
    }

    Bounds {
        min_x: bx0,
        min_y: by0,
        max_x: bx1,
        max_y: by1,
    }
}

/// Extract the rotation angle (radians) embedded in a transform's linear part.
/// For a pure rotate∘scale this is the rotation component; for a sheared
/// transform it's the angle of the transformed x-axis, which is the best
/// single-angle answer and the one Figma's resize math assumes.
pub fn transform_angle(t: &Transform2D) -> f64 {
    let c = t.to_components(); // [a, b, c, d, tx, ty] — x-axis is (a, b).
    c[1].atan2(c[0])
}

/// Compute the new node transform for a resize drag **that preserves the
/// node's rotation**.
///
/// The old resize path rebuilt the transform as a pure axis-aligned
/// scale+translate (`[sx,0,0,sy,tx,ty]`), which silently dropped any rotation
/// the node already carried — a rotated rectangle snapped back upright the
/// instant you grabbed a handle. This computes the resize in the node's own
/// **un-rotated frame** instead: the cursor is projected onto the node's
/// rotated axes, the new local extent is computed there, and the result is
/// recomposed with the original rotation, pinning the dragged handle's anchor
/// to its world position so the gesture feels identical to the un-rotated case.
///
/// Arguments:
/// - `original` — the node's transform at press time (local → world for a
///   single, root-level selection, matching the v0 resize assumption).
/// - `local` — the node's local-space bounds.
/// - `handle` — the corner / edge being dragged.
/// - `cursor_world` — current cursor in world space.
/// - `lock_aspect` (Shift) / `from_center` (Alt) — same semantics as
///   [`resize_bounds`].
pub fn resize_transform_keep_rotation(
    original: Transform2D,
    local: Bounds,
    handle: ResizeHandle,
    cursor_world: DVec2,
    lock_aspect: bool,
    from_center: bool,
) -> Transform2D {
    // The node's rotation, and the rotate-only basis it induces. Work in a
    // frame where that rotation is removed (`r_inv`): there the node — and the
    // resize — is axis-aligned, so all the existing `resize_bounds` math
    // applies unchanged.
    let theta = transform_angle(&original);
    let r = Transform2D::rotation(theta);
    let r_inv = r.inverse();

    // `frame = original ∘ r_inv` maps node-local space into the un-rotated
    // frame. It is (up to numerical noise) a pure scale+translate, so the local
    // bounds become an axis-aligned rect we can resize directly.
    let frame = original.then(&r_inv);
    let frame_bounds = local.transformed(&frame);

    // Project the cursor into the same un-rotated frame and run the standard
    // resize there.
    let cursor_frame = r_inv.transform_point(cursor_world);
    let mut new_frame_bounds =
        resize_bounds(frame_bounds, handle, cursor_frame, lock_aspect, from_center);

    // Guard against collapsing the node to zero / negative extent in the
    // un-rotated frame — the geometrically correct place to clamp, since that
    // is where the resize is axis-aligned. Anchor the clamp on the fixed
    // (anchor) edge so the dragged edge is the one that gets pushed back.
    let min_size = 0.5;
    if new_frame_bounds.width().abs() < min_size {
        if new_frame_bounds.max_x >= new_frame_bounds.min_x {
            new_frame_bounds.max_x = new_frame_bounds.min_x + min_size;
        } else {
            new_frame_bounds.min_x = new_frame_bounds.max_x + min_size;
        }
    }
    if new_frame_bounds.height().abs() < min_size {
        if new_frame_bounds.max_y >= new_frame_bounds.min_y {
            new_frame_bounds.max_y = new_frame_bounds.min_y + min_size;
        } else {
            new_frame_bounds.min_y = new_frame_bounds.max_y + min_size;
        }
    }

    // Rebuild the local→frame map (axis-aligned scale+translate) onto the new
    // bounds, exactly as the legacy path did — but in the un-rotated frame.
    let local_w = local.width().max(f64::EPSILON);
    let local_h = local.height().max(f64::EPSILON);
    let sx = new_frame_bounds.width() / local_w;
    let sy = new_frame_bounds.height() / local_h;
    let tx = new_frame_bounds.min_x - local.min_x * sx;
    let ty = new_frame_bounds.min_y - local.min_y * sy;
    let new_frame = Transform2D::from_components([sx, 0.0, 0.0, sy, tx, ty]);

    // Recompose: apply the axis-aligned map, then rotate back by theta. The
    // anchor stayed fixed in the un-rotated frame, and rotation is the same on
    // both sides, so the dragged handle's world anchor is preserved.
    new_frame.then(&r)
}

/// Resize a node by changing its **content-box extent** instead of baking a
/// scale into its transform — the correct resize for content whose pixels/glyphs
/// must NOT stretch (a text box reflows; a bitmap could re-fit). Returns the new
/// transform (the existing linear part is preserved byte-for-byte) and the new
/// box size `[w, h]` the caller writes into the node's `local_size`. The dragged
/// handle's world anchor is pinned exactly as in
/// [`resize_transform_keep_rotation`]; the only difference is that the new
/// extent flows into the box size rather than the transform's scale, so glyphs
/// keep their authored size while the box grows/shrinks.
///
/// `original` must map the box directly into the same coordinate space as
/// `cursor_world`. For a nested node the caller passes its world transform and
/// rebases the returned transform into parent space.
pub fn resize_box_keep_rotation(
    original: Transform2D,
    local: Bounds,
    handle: ResizeHandle,
    cursor_world: DVec2,
    lock_aspect: bool,
    from_center: bool,
) -> (Transform2D, f64, f64) {
    let determinant = original.0.matrix2.determinant();
    if !determinant.is_finite() || determinant.abs() <= f64::EPSILON {
        return (original, local.width(), local.height());
    }

    // Project the cursor through the complete affine transform. Working in the
    // node's own coordinates preserves existing scale/shear as well as rotation:
    // resizing changes only the content-box extent, never the glyph transform.
    let cursor_local = original.inverse().transform_point(cursor_world);
    let mut new_local_bounds = resize_bounds(local, handle, cursor_local, lock_aspect, from_center);

    // Clamp to a minimum local extent so the text box cannot collapse.
    let min_size = 0.5;
    if new_local_bounds.width() < min_size {
        new_local_bounds.max_x = new_local_bounds.min_x + min_size;
    }
    if new_local_bounds.height() < min_size {
        new_local_bounds.max_y = new_local_bounds.min_y + min_size;
    }

    // The stored content box keeps its original local origin and receives only
    // the new extent. Translate that origin onto the resized bounds' new minimum
    // while preserving the complete original matrix. This pins the opposite
    // handle for north/west drags without scaling the content.
    let local_delta = DVec2::new(
        new_local_bounds.min_x - local.min_x,
        new_local_bounds.min_y - local.min_y,
    );
    let new_transform = Transform2D(glam::DAffine2 {
        matrix2: original.0.matrix2,
        translation: original.0.translation + original.0.matrix2.mul_vec2(local_delta),
    });
    (
        new_transform,
        new_local_bounds.width(),
        new_local_bounds.height(),
    )
}

/// Convenience: hit-test handles in screen space, mapping the cursor
/// through the viewport for the caller.
pub fn hit_test_resize_handle_screen(
    world_bounds: Bounds,
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
    threshold: f64,
) -> Option<ResizeHandle> {
    hit_test_resize_handle(world_bounds, screen_point, viewport, screen_size, threshold)
}

/// Convenience: convert a screen-space cursor position to world coords for
/// the resize math.
pub fn cursor_world_from_screen(
    screen_point: DVec2,
    viewport: &Viewport,
    screen_size: DVec2,
) -> DVec2 {
    screen_to_world(screen_point, viewport, screen_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vp() -> Viewport {
        Viewport {
            center: [0.0, 0.0],
            zoom: 1.0,
        }
    }
    fn size() -> DVec2 {
        DVec2::new(800.0, 600.0)
    }

    #[test]
    fn opposite_handles_are_diagonal() {
        assert_eq!(ResizeHandle::NorthWest.opposite(), ResizeHandle::SouthEast);
        assert_eq!(ResizeHandle::North.opposite(), ResizeHandle::South);
        assert_eq!(ResizeHandle::West.opposite(), ResizeHandle::East);
    }

    #[test]
    fn oriented_handles_match_aabb_when_unrotated_and_follow_rotation() {
        let local = Bounds::from_xywh(0.0, 0.0, 120.0, 80.0);
        let (vp, ss) = (vp(), size());

        // Identity transform: the oriented handles coincide with the AABB handles
        // (here `local` IS the world AABB), so nothing regresses for upright nodes.
        let ident = Transform2D::IDENTITY;
        for &h in &ResizeHandle::ALL {
            let aabb = handle_screen_position(h, local, &vp, ss);
            let ori = handle_screen_position_oriented(h, local, &ident, &vp, ss);
            assert!(
                (aabb - ori).length() < 1e-6,
                "{h:?} identity oriented == aabb"
            );
        }

        // Rotate 30° about the box center: the NW corner leaves the AABB's NW
        // corner (the box now follows the rotation), and the hit-test snaps to it.
        let rot = rotate_about(local.center(), std::f64::consts::FRAC_PI_6);
        let nw_aabb = handle_screen_position(ResizeHandle::NorthWest, local, &vp, ss);
        let nw_ori = handle_screen_position_oriented(ResizeHandle::NorthWest, local, &rot, &vp, ss);
        assert!(
            (nw_aabb - nw_ori).length() > 5.0,
            "rotated NW handle moves off the AABB NW: aabb {nw_aabb:?} oriented {nw_ori:?}"
        );
        let hit =
            hit_test_resize_handle_oriented(local, &rot, nw_ori, &vp, ss, DEFAULT_HANDLE_THRESHOLD);
        assert_eq!(
            hit,
            Some(ResizeHandle::NorthWest),
            "oriented hit-test picks NW"
        );
        // The AABB hit-test would MISS at the rotated NW position (the bug).
        let aabb_hit = hit_test_resize_handle(local, nw_ori, &vp, ss, DEFAULT_HANDLE_THRESHOLD);
        assert_ne!(
            aabb_hit,
            Some(ResizeHandle::NorthWest),
            "AABB hit-test does not catch the rotated NW corner"
        );
    }

    #[test]
    fn affects_axis_matches_intent() {
        assert!(ResizeHandle::NorthWest.affects_x());
        assert!(ResizeHandle::NorthWest.affects_y());
        assert!(!ResizeHandle::North.affects_x());
        assert!(ResizeHandle::North.affects_y());
        assert!(ResizeHandle::East.affects_x());
        assert!(!ResizeHandle::East.affects_y());
    }

    #[test]
    fn handle_world_returns_correct_corners() {
        let b = Bounds::from_xywh(10.0, 20.0, 30.0, 40.0);
        assert_eq!(
            ResizeHandle::NorthWest.handle_world(b),
            DVec2::new(10.0, 20.0)
        );
        assert_eq!(
            ResizeHandle::NorthEast.handle_world(b),
            DVec2::new(40.0, 20.0)
        );
        assert_eq!(
            ResizeHandle::SouthEast.handle_world(b),
            DVec2::new(40.0, 60.0)
        );
        assert_eq!(
            ResizeHandle::SouthWest.handle_world(b),
            DVec2::new(10.0, 60.0)
        );
    }

    #[test]
    fn anchor_is_opposite_of_handle() {
        let b = Bounds::from_xywh(10.0, 20.0, 30.0, 40.0);
        // SE handle anchored at NW corner.
        assert_eq!(
            ResizeHandle::SouthEast.anchor_world(b),
            DVec2::new(10.0, 20.0)
        );
        // N edge anchored at S edge midpoint.
        assert_eq!(ResizeHandle::North.anchor_world(b), DVec2::new(25.0, 60.0));
    }

    #[test]
    fn hit_test_finds_corner_when_clicked_dead_center() {
        let world_b = Bounds::from_xywh(-10.0, -10.0, 20.0, 20.0);
        // SE corner is at world (10, 10) → screen center + (10, 10).
        let screen_pt = world_to_screen(DVec2::new(10.0, 10.0), &vp(), size());
        let h = hit_test_resize_handle(world_b, screen_pt, &vp(), size(), 8.0).unwrap();
        assert_eq!(h, ResizeHandle::SouthEast);
    }

    #[test]
    fn hit_test_misses_when_far_from_any_handle() {
        let world_b = Bounds::from_xywh(-10.0, -10.0, 20.0, 20.0);
        // Way off any corner.
        let screen_pt = DVec2::new(700.0, 100.0);
        assert!(hit_test_resize_handle(world_b, screen_pt, &vp(), size(), 8.0).is_none());
    }

    #[test]
    fn hit_test_threshold_is_zoom_stable() {
        // Same world rect, two zoom levels. A click 6 logical pixels from a
        // handle should hit at both zooms (threshold is screen-space).
        let world_b = Bounds::from_xywh(-10.0, -10.0, 20.0, 20.0);
        for &zoom in &[0.25, 1.0, 4.0] {
            let v = Viewport {
                center: [0.0, 0.0],
                zoom,
            };
            let center = world_to_screen(DVec2::new(10.0, 10.0), &v, size());
            let click = center + DVec2::new(5.0, 0.0);
            let h = hit_test_resize_handle(world_b, click, &v, size(), 8.0);
            assert_eq!(h, Some(ResizeHandle::SouthEast), "missed at zoom {zoom}");
        }
    }

    #[test]
    fn resize_corner_extends_rect_to_cursor() {
        let original = Bounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        // Drag SE corner from (10, 10) to (20, 30).
        let new = resize_bounds(
            original,
            ResizeHandle::SouthEast,
            DVec2::new(20.0, 30.0),
            false,
            false,
        );
        assert_eq!(new, Bounds::from_xywh(0.0, 0.0, 20.0, 30.0));
    }

    #[test]
    fn resize_north_edge_only_changes_y() {
        let original = Bounds::from_xywh(0.0, 10.0, 10.0, 10.0);
        // Drag N edge upward; cursor at (5, 5) (slightly above the original top).
        let new = resize_bounds(
            original,
            ResizeHandle::North,
            DVec2::new(5.0, 5.0),
            false,
            false,
        );
        // X axis unchanged; Y top went up.
        assert_eq!(new.min_x, 0.0);
        assert_eq!(new.max_x, 10.0);
        assert_eq!(new.min_y, 5.0);
        assert_eq!(new.max_y, 20.0);
    }

    #[test]
    fn resize_with_from_center_grows_symmetrically() {
        let original = Bounds::from_xywh(-10.0, -10.0, 20.0, 20.0);
        let new = resize_bounds(
            original,
            ResizeHandle::SouthEast,
            DVec2::new(20.0, 20.0),
            false,
            true,
        );
        // Cursor at (20, 20) is +20 from anchor (origin); mirror to -20.
        assert_eq!(new, Bounds::from_xywh(-20.0, -20.0, 40.0, 40.0));
    }

    #[test]
    fn resize_with_aspect_lock_preserves_ratio() {
        let original = Bounds::from_xywh(0.0, 0.0, 10.0, 20.0); // 1:2 aspect
        // Cursor at (30, 25): without aspect would give 30x25 (3:2.5 = 1.2 aspect).
        // With lock, the larger axis wins; here 30 / (1/2) = 60 > 25, so X wins,
        // resulting in 30 wide × 60 tall.
        let new = resize_bounds(
            original,
            ResizeHandle::SouthEast,
            DVec2::new(30.0, 25.0),
            true,
            false,
        );
        assert!((new.width() - 30.0).abs() < 1e-9);
        assert!((new.height() - 60.0).abs() < 1e-9);
    }

    #[test]
    fn resize_into_negative_quadrant_inverts_correctly() {
        // Drag SE handle past NW corner — bounds should flip without becoming
        // degenerate. We just check that min < max on both axes.
        let original = Bounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let new = resize_bounds(
            original,
            ResizeHandle::SouthEast,
            DVec2::new(-5.0, -5.0),
            false,
            false,
        );
        assert!(new.min_x <= new.max_x);
        assert!(new.min_y <= new.max_y);
    }

    #[test]
    fn cursor_world_round_trips_through_viewport() {
        let v = Viewport {
            center: [50.0, 100.0],
            zoom: 2.5,
        };
        let s = DVec2::new(800.0, 600.0);
        let p_world = DVec2::new(123.4, -56.7);
        let p_screen = world_to_screen(p_world, &v, s);
        let back = cursor_world_from_screen(p_screen, &v, s);
        assert!((back - p_world).length() < 1e-9);
    }

    // ------------------------------------------------------------------------
    // Rotation handles
    // ------------------------------------------------------------------------

    #[test]
    fn rotate_zone_sits_just_outside_its_corner() {
        // The rotate zone center is offset OUTWARD from the corner handle along
        // the diagonal, by exactly ROTATE_HANDLE_OFFSET screen pixels.
        let world_b = Bounds::from_xywh(-20.0, -20.0, 40.0, 40.0);
        // Screen-space center of the selection (origin in world → screen center).
        let sel_center = world_to_screen(DVec2::ZERO, &vp(), size());
        for &h in &RotateHandle::ALL {
            let corner = handle_screen_position(h.corner(), world_b, &vp(), size());
            let zone = rotate_handle_screen_position(h, world_b, &vp(), size());
            let d = (zone - corner).length();
            assert!(
                (d - ROTATE_HANDLE_OFFSET).abs() < 1e-9,
                "{h:?} zone is {d} px from its corner, expected {ROTATE_HANDLE_OFFSET}"
            );
            // The zone sits farther from the selection center than the corner —
            // it is OUTSIDE the handle, the Figma rotate affordance.
            assert!(
                (zone - sel_center).length() > (corner - sel_center).length(),
                "{h:?} rotate zone should be outside its corner"
            );
        }
    }

    #[test]
    fn hit_test_rotate_catches_just_outside_corner_and_misses_on_corner() {
        let world_b = Bounds::from_xywh(-20.0, -20.0, 40.0, 40.0);
        // A click right at the rotate zone center hits the rotation handle.
        let zone = rotate_handle_screen_position(RotateHandle::SouthEast, world_b, &vp(), size());
        let got = hit_test_rotate_handle(world_b, zone, &vp(), size(), DEFAULT_ROTATE_THRESHOLD);
        assert_eq!(got, Some(RotateHandle::SouthEast));
        // A click far from any corner misses.
        let far = DVec2::new(0.0, 0.0); // selection center — no zone there
        let miss = hit_test_rotate_handle(world_b, far, &vp(), size(), DEFAULT_ROTATE_THRESHOLD);
        assert!(miss.is_none());
    }

    #[test]
    fn rotation_delta_measures_swept_angle() {
        // Pivot at origin; press to the +X axis (angle 0), cursor on +Y axis.
        // In y-down screen-style coords atan2(1, 0) = +90°, but we feed WORLD
        // points here, so the convention is whatever the caller uses — we just
        // check the swept delta is a quarter turn.
        let pivot = DVec2::ZERO;
        let press = DVec2::new(10.0, 0.0);
        let cursor = DVec2::new(0.0, 10.0);
        let d = rotation_delta(pivot, press, cursor, 0.0, false);
        assert!((d - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn rotation_delta_snaps_to_15_degree_increments() {
        let pivot = DVec2::ZERO;
        let press = DVec2::new(10.0, 0.0);
        // Cursor at ~20° above +X. Base angle 0 → snapped absolute should be 15°.
        let twenty = 20.0_f64.to_radians();
        let cursor = DVec2::new(20.0 * twenty.cos(), 20.0 * twenty.sin());
        let d = rotation_delta(pivot, press, cursor, 0.0, true);
        assert!(
            (d - 15.0_f64.to_radians()).abs() < 1e-9,
            "expected snap to 15°, got {} deg",
            d.to_degrees()
        );
        // ~40° snaps to 45°.
        let forty = 40.0_f64.to_radians();
        let cursor2 = DVec2::new(20.0 * forty.cos(), 20.0 * forty.sin());
        let d2 = rotation_delta(pivot, press, cursor2, 0.0, true);
        assert!(
            (d2 - 45.0_f64.to_radians()).abs() < 1e-9,
            "expected snap to 45°, got {} deg",
            d2.to_degrees()
        );
    }

    #[test]
    fn rotation_delta_snap_is_absolute_against_base_angle() {
        // A node already rotated 10°: pressing and dragging a tiny bit with snap
        // should land on the nearest 15° multiple (15°), i.e. a +5° delta.
        let pivot = DVec2::ZERO;
        let base = 10.0_f64.to_radians();
        let press = DVec2::new(10.0, 0.0); // ray angle 0
        let four = 4.0_f64.to_radians();
        let cursor = DVec2::new(10.0 * four.cos(), 10.0 * four.sin()); // swept +4°
        // absolute target = base(10) + 4 = 14 → snaps to 15 → delta = 5°.
        let d = rotation_delta(pivot, press, cursor, base, true);
        assert!(
            (d - 5.0_f64.to_radians()).abs() < 1e-9,
            "expected +5° to reach 15° absolute, got {} deg",
            d.to_degrees()
        );
    }

    #[test]
    fn transform_angle_recovers_rotation() {
        let t = Transform2D::rotation(30.0_f64.to_radians());
        assert!((transform_angle(&t) - 30.0_f64.to_radians()).abs() < 1e-9);
        // Rotation composed with scale still recovers the angle.
        let t2 =
            Transform2D::scale_xy(2.0, 3.0).then(&Transform2D::rotation(30.0_f64.to_radians()));
        assert!((transform_angle(&t2) - 30.0_f64.to_radians()).abs() < 1e-9);
    }

    #[test]
    fn rotate_about_pivot_keeps_pivot_fixed() {
        let pivot = DVec2::new(5.0, 7.0);
        let t = rotate_about(pivot, 1.234);
        let moved = t.transform_point(pivot);
        assert!((moved - pivot).length() < 1e-9);
    }

    // ------------------------------------------------------------------------
    // Resize preserves rotation
    // ------------------------------------------------------------------------

    #[test]
    fn unrotated_resize_keep_rotation_matches_legacy_scale_translate() {
        // With no rotation, the rotation-preserving path must reduce to the same
        // result as the old pure scale+translate construction.
        let local = Bounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let original = Transform2D::IDENTITY; // node world == local
        // Drag SE corner from (10,10) to (20,30).
        let cursor = DVec2::new(20.0, 30.0);
        let t = resize_transform_keep_rotation(
            original,
            local,
            ResizeHandle::SouthEast,
            cursor,
            false,
            false,
        );
        // New world bounds should be [0,0,20,30].
        let nb = local.transformed(&t);
        assert!((nb.min_x).abs() < 1e-9 && (nb.min_y).abs() < 1e-9);
        assert!((nb.width() - 20.0).abs() < 1e-9, "{}", nb.width());
        assert!((nb.height() - 30.0).abs() < 1e-9, "{}", nb.height());
        // And no rotation introduced.
        assert!(transform_angle(&t).abs() < 1e-9);
    }

    #[test]
    fn resize_preserves_existing_rotation_angle() {
        // A 10x10 node rotated 90°, anchored so the rotation is visible.
        let local = Bounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let theta = 90.0_f64.to_radians();
        let original = Transform2D::rotation(theta);
        // The SE local corner (10,10) maps under 90° rotation to world (-10,10).
        // Drag it outward along its (rotated) diagonal: pick a cursor that the
        // un-rotated frame interprets as growing the rect.
        let cursor = original.transform_point(DVec2::new(20.0, 25.0));
        let t = resize_transform_keep_rotation(
            original,
            local,
            ResizeHandle::SouthEast,
            cursor,
            false,
            false,
        );
        // Rotation angle is unchanged (still 90°).
        let a = transform_angle(&t);
        assert!(
            (a - theta).abs() < 1e-6,
            "rotation drifted to {} deg (expected 90)",
            a.to_degrees()
        );
        // The NW corner (the anchor for an SE drag) stays pinned in world space.
        let anchor_before = original.transform_point(DVec2::new(0.0, 0.0));
        let anchor_after = t.transform_point(DVec2::new(0.0, 0.0));
        assert!(
            (anchor_before - anchor_after).length() < 1e-6,
            "anchor moved from {anchor_before:?} to {anchor_after:?}"
        );
        // And the node actually grew along its local axes.
        let new_local_size_x =
            (t.transform_point(DVec2::new(10.0, 0.0)) - t.transform_point(DVec2::ZERO)).length();
        assert!(
            new_local_size_x > 10.0,
            "expected growth, got {new_local_size_x}"
        );
    }

    #[test]
    fn box_resize_preserves_existing_scale_and_pins_the_opposite_edge() {
        let local = Bounds::from_xywh(0.0, 0.0, 100.0, 40.0);
        let original = Transform2D::from_components([2.0, 0.0, 0.0, 0.5, 10.0, 20.0]);
        let original_east = original.transform_point(DVec2::new(100.0, 20.0));
        let cursor = original.transform_point(DVec2::new(25.0, 20.0));

        let (resized, width, height) =
            resize_box_keep_rotation(original, local, ResizeHandle::West, cursor, false, false);

        assert_eq!(
            &resized.to_components()[..4],
            &original.to_components()[..4],
            "content-box resize must not change the content transform"
        );
        assert!((width - 75.0).abs() < 1e-9);
        assert!((height - 40.0).abs() < 1e-9);
        let resized_east = resized.transform_point(DVec2::new(width, height * 0.5));
        assert!(
            (resized_east - original_east).length() < 1e-9,
            "west-edge drag moved the fixed east edge from {original_east:?} to {resized_east:?}"
        );
    }
}

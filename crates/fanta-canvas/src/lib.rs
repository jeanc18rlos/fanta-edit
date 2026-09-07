//! Viewport and interaction primitives for the Fanta engine.
//!
//! Pure-math layer between `fanta-doc` and `fanta-tools`. No rendering deps —
//! everything here is testable without a window, a GPU, or even Skia.
//!
//! ## Modules
//!
//! - [`viewport`] — screen ↔ world conversion, pan, zoom-at-cursor, fit-bounds.
//! - [`hit_test`] — refined topmost / deep / marquee hit queries with both
//!   AABB-fast and path-precise modes.
//! - [`snap`] — grid + edge + center snapping engine producing both the
//!   adjusted position and which guides should be drawn.
//! - [`align`] — align / distribute helpers that return ready-to-apply
//!   [`fanta_doc::Operation`] vectors.

#![forbid(unsafe_code)]

pub mod align;
pub mod handles;
pub mod hit_test;
pub mod snap;
pub mod viewport;

pub use align::{
    Axis, HAlign, VAlign, align_horizontal, align_to_bounds_h, align_to_bounds_v, align_vertical,
    distribute,
};
pub use handles::{
    DEFAULT_HANDLE_THRESHOLD, DEFAULT_ROTATE_THRESHOLD, ROTATE_HANDLE_OFFSET, ROTATE_SNAP_RADIANS,
    ResizeHandle, RotateHandle, angle_about, cursor_world_from_screen, handle_screen_position,
    handle_screen_position_oriented, hit_test_resize_handle, hit_test_resize_handle_oriented,
    hit_test_resize_handle_screen, hit_test_rotate_handle, hit_test_rotate_handle_oriented,
    resize_bounds, resize_box_keep_rotation, resize_transform_keep_rotation, rotate_about,
    rotate_handle_screen_position, rotate_handle_screen_position_oriented, rotation_delta,
    transform_angle,
};
pub use hit_test::{
    BEZIER_FLATTEN_STEPS, HitPrecision, MarqueeMode, hit_test, hit_test_deep, hit_test_screen,
    hit_test_within, hit_test_within_screen, node_screen_bounds, point_in_path,
};
pub use snap::{
    AxisSnap, SnapCandidates, SnapEngine, SnapGrid, SnapKind, SnapResult, SnapTargets,
    SnapThresholds,
};
pub use viewport::{
    MAX_ZOOM, MIN_ZOOM, fit_bounds, pan, screen_to_world, world_to_screen, zoom_at,
};

/// Measurement helpers for annotations, guides, and Figma-like measurements.
/// Use with hit_test and bounds for distances, gaps, etc. (Figma prototype / inspect fidelity).
pub fn point_distance(a: glam::DVec2, b: glam::DVec2) -> f64 {
    (b - a).length()
}

/// Horizontal distance between two points (absolute delta x). Useful for
/// measurement annotations that report X gaps.
pub fn horizontal_distance(a: glam::DVec2, b: glam::DVec2) -> f64 {
    (b.x - a.x).abs()
}

/// Vertical distance between two points (absolute delta y).
pub fn vertical_distance(a: glam::DVec2, b: glam::DVec2) -> f64 {
    (b.y - a.y).abs()
}

/// Axis-aligned distance between two axis-aligned rectangles (returns the
/// smallest gap along x or y, or 0 if overlapping in that axis). Mirrors
/// Figma's measurement lines between frames/layers.
pub fn rect_gap(a: glam::DVec2, a_size: [f64; 2], b: glam::DVec2, b_size: [f64; 2]) -> [f64; 2] {
    // a: top-left, size [w,h]; b same.
    let a_right = a.x + a_size[0];
    let a_bottom = a.y + a_size[1];
    let b_right = b.x + b_size[0];
    let b_bottom = b.y + b_size[1];

    let gap_x = if a_right < b.x {
        b.x - a_right
    } else if b_right < a.x {
        a.x - b_right
    } else {
        0.0
    };
    let gap_y = if a_bottom < b.y {
        b.y - a_bottom
    } else if b_bottom < a.y {
        a.y - b_bottom
    } else {
        0.0
    };
    [gap_x, gap_y]
}

/// Euclidean distance between the closest points of two AABB rects (0 if overlapping).
pub fn bounds_distance(
    a_min: glam::DVec2,
    a_max: glam::DVec2,
    b_min: glam::DVec2,
    b_max: glam::DVec2,
) -> f64 {
    let dx = if a_max.x < b_min.x {
        b_min.x - a_max.x
    } else if b_max.x < a_min.x {
        a_min.x - b_max.x
    } else {
        0.0
    };
    let dy = if a_max.y < b_min.y {
        b_min.y - a_max.y
    } else if b_max.y < a_min.y {
        a_min.y - b_max.y
    } else {
        0.0
    };
    (dx * dx + dy * dy).sqrt()
}

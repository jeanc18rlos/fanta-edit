//! Viewport and interaction primitives for Fantaisa.
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

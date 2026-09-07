//! Viewport math — the bridge between screen pixels and world coordinates.
//!
//! The renderer composes a transform on every frame; the tool layer needs the
//! inverse to map pointer events back into world space. Centralizing the
//! math here means both code paths agree exactly — drift between the two
//! mappings is a category of bug we never want to encounter.
//!
//! ## Conventions
//!
//! - Doc coordinates are `f64` (`world`).
//! - Screen coordinates are `f64` (`screen`). Window/GPU code narrows to
//!   `f32` at its own boundary.
//! - Y axis grows down on screen (matches GPUI / Skia / native windowing).
//!   World Y also grows down by convention so transforms are direct without
//!   sign flips.
//! - A [`Viewport`] is `Doc`-owned; this module provides pure functions that
//!   take one by reference.

use fanta_doc::{Bounds, Viewport};
use glam::DVec2;

/// Convert a screen-pixel position into world-space coordinates.
///
/// The screen origin is the top-left of the viewport rectangle of size
/// `screen_size`. The viewport's `center` maps to the middle of that
/// rectangle at zoom 1.0.
pub fn screen_to_world(screen: DVec2, viewport: &Viewport, screen_size: DVec2) -> DVec2 {
    let center = DVec2::from(viewport.center);
    let half = screen_size * 0.5;
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    (screen - half) * inv_zoom + center
}

/// Convert a world-space position into screen-pixel coordinates. Inverse of
/// [`screen_to_world`].
pub fn world_to_screen(world: DVec2, viewport: &Viewport, screen_size: DVec2) -> DVec2 {
    let center = DVec2::from(viewport.center);
    let half = screen_size * 0.5;
    (world - center) * viewport.zoom + half
}

/// Pan the viewport by a screen-space delta — exactly what a hand-tool drag
/// emits. Positive delta moves what's under the cursor *with* the cursor
/// (Figma / Photoshop convention).
pub fn pan(viewport: &Viewport, screen_delta: DVec2) -> Viewport {
    let inv_zoom = 1.0 / viewport.zoom.max(f64::EPSILON);
    let world_delta = screen_delta * inv_zoom;
    Viewport {
        center: [
            viewport.center[0] - world_delta.x,
            viewport.center[1] - world_delta.y,
        ],
        zoom: viewport.zoom,
    }
}

/// Zoom around a fixed screen position. This is the math you want for
/// scroll-wheel zoom or pinch zoom — the world point under the cursor must
/// stay under the cursor as zoom changes. If we just multiplied `zoom`, the
/// world would shift sideways and the canvas would feel like it's slipping.
///
/// `factor` > 1 zooms in; < 1 zooms out. Min and max clamps prevent NaN /
/// near-singular matrices in the renderer.
pub fn zoom_at(
    viewport: &Viewport,
    screen_anchor: DVec2,
    factor: f64,
    screen_size: DVec2,
) -> Viewport {
    let world_before = screen_to_world(screen_anchor, viewport, screen_size);
    let new_zoom = (viewport.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let provisional = Viewport {
        center: viewport.center,
        zoom: new_zoom,
    };
    let world_after = screen_to_world(screen_anchor, &provisional, screen_size);
    // Shift center so `world_before` maps back to `screen_anchor` at the new zoom.
    let drift = world_after - world_before;
    Viewport {
        center: [
            provisional.center[0] - drift.x,
            provisional.center[1] - drift.y,
        ],
        zoom: new_zoom,
    }
}

/// Compute a viewport that fits `bounds` into `screen_size` with `padding`
/// pixels of breathing room on each side. Aspect-preserving — uses the
/// tighter of the X / Y fit ratios.
pub fn fit_bounds(bounds: Bounds, screen_size: DVec2, padding_px: f64) -> Viewport {
    let bw = bounds.width().max(f64::EPSILON);
    let bh = bounds.height().max(f64::EPSILON);
    let usable_w = (screen_size.x - 2.0 * padding_px).max(1.0);
    let usable_h = (screen_size.y - 2.0 * padding_px).max(1.0);
    let zoom = (usable_w / bw).min(usable_h / bh).clamp(MIN_ZOOM, MAX_ZOOM);
    let center = bounds.center();
    Viewport {
        center: [center.x, center.y],
        zoom,
    }
}

/// Reasonable bounds for the zoom factor. Outside this range floats lose
/// precision and the renderer's `f32` projection breaks down.
pub const MIN_ZOOM: f64 = 1.0 / 4096.0;
pub const MAX_ZOOM: f64 = 4096.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn vp(cx: f64, cy: f64, zoom: f64) -> Viewport {
        Viewport {
            center: [cx, cy],
            zoom,
        }
    }

    #[test]
    fn screen_world_round_trips_at_identity() {
        let v = vp(0.0, 0.0, 1.0);
        let size = DVec2::new(800.0, 600.0);
        for &p in &[
            DVec2::ZERO,
            DVec2::new(1.0, 2.0),
            DVec2::new(-100.0, 50.0),
            DVec2::new(799.0, 599.0),
        ] {
            let w = screen_to_world(p, &v, size);
            let back = world_to_screen(w, &v, size);
            assert!((back - p).length() < 1e-9, "round trip failed at {p:?}");
        }
    }

    #[test]
    fn screen_world_round_trips_under_pan_and_zoom() {
        let v = vp(123.5, -50.0, 2.75);
        let size = DVec2::new(800.0, 600.0);
        for &p in &[
            DVec2::ZERO,
            DVec2::new(400.0, 300.0),
            DVec2::new(123.0, 456.0),
        ] {
            let w = screen_to_world(p, &v, size);
            let back = world_to_screen(w, &v, size);
            assert!((back - p).length() < 1e-9);
        }
    }

    #[test]
    fn screen_center_maps_to_world_center() {
        let v = vp(50.0, -25.0, 1.5);
        let size = DVec2::new(800.0, 600.0);
        let w = screen_to_world(size * 0.5, &v, size);
        assert!((w - DVec2::new(50.0, -25.0)).length() < 1e-9);
    }

    #[test]
    fn pan_does_not_change_zoom() {
        let v = vp(0.0, 0.0, 2.0);
        let p = pan(&v, DVec2::new(50.0, 30.0));
        assert_eq!(p.zoom, 2.0);
    }

    #[test]
    fn pan_moves_world_under_cursor_with_cursor() {
        // Cursor at (400, 300) world-space sees (0, 0). Pan by (+10, 0) screen.
        // After: the same world point (0, 0) should now appear at (410, 300).
        let v = vp(0.0, 0.0, 1.0);
        let size = DVec2::new(800.0, 600.0);
        let after = pan(&v, DVec2::new(10.0, 0.0));
        let screen_for_origin = world_to_screen(DVec2::ZERO, &after, size);
        assert!((screen_for_origin - DVec2::new(410.0, 300.0)).length() < 1e-9);
    }

    #[test]
    fn zoom_at_keeps_anchor_point_under_cursor() {
        let v = vp(0.0, 0.0, 1.0);
        let size = DVec2::new(800.0, 600.0);
        let anchor = DVec2::new(200.0, 150.0);
        let world_before = screen_to_world(anchor, &v, size);
        // Try a range of zoom factors — all should preserve the anchor.
        for factor in [0.25, 0.5, 1.5, 3.0, 7.0] {
            let v2 = zoom_at(&v, anchor, factor, size);
            let world_after_anchor = screen_to_world(anchor, &v2, size);
            assert!(
                (world_after_anchor - world_before).length() < 1e-7,
                "factor {factor} drifted by {}",
                (world_after_anchor - world_before).length()
            );
        }
    }

    #[test]
    fn zoom_at_clamps_at_extremes() {
        let v = vp(0.0, 0.0, 1.0);
        let size = DVec2::new(800.0, 600.0);
        let huge = zoom_at(&v, DVec2::new(400.0, 300.0), 1e9, size);
        assert!(huge.zoom <= MAX_ZOOM);
        let tiny = zoom_at(&v, DVec2::new(400.0, 300.0), 1e-12, size);
        assert!(tiny.zoom >= MIN_ZOOM);
    }

    #[test]
    fn fit_bounds_chooses_aspect_preserving_zoom() {
        let bounds = Bounds::from_xywh(0.0, 0.0, 100.0, 200.0); // tall
        let size = DVec2::new(400.0, 400.0);
        let v = fit_bounds(bounds, size, 0.0);
        // The tighter axis is height (400 / 200 = 2.0); X allows 4.0; we
        // expect zoom = 2.0 (aspect-preserving fit).
        assert!((v.zoom - 2.0).abs() < 1e-9);
        // Center of fit equals center of bounds.
        assert_eq!(v.center, [50.0, 100.0]);
    }

    #[test]
    fn fit_bounds_honors_padding() {
        let bounds = Bounds::from_xywh(0.0, 0.0, 100.0, 100.0);
        let size = DVec2::new(400.0, 400.0);
        let v_no_pad = fit_bounds(bounds, size, 0.0);
        let v_with_pad = fit_bounds(bounds, size, 50.0);
        assert!(v_with_pad.zoom < v_no_pad.zoom);
    }

    #[test]
    fn fit_bounds_zero_dimensions_does_not_panic() {
        let degenerate = Bounds::from_xywh(0.0, 0.0, 0.0, 0.0);
        let v = fit_bounds(degenerate, DVec2::new(400.0, 400.0), 0.0);
        // Whatever zoom we land on, it should be finite and inside the clamp.
        assert!(v.zoom.is_finite());
        assert!(v.zoom >= MIN_ZOOM && v.zoom <= MAX_ZOOM);
    }

    #[test]
    fn pan_preserves_world_to_screen_for_other_points_proportionally() {
        // Sanity: a pan by 50 screen px at zoom=2 should shift world center
        // by 25 world units in the opposite direction. The test confirms
        // that's what `pan` produces (and locks in the sign convention).
        let v = vp(0.0, 0.0, 2.0);
        let p = pan(&v, DVec2::new(50.0, -20.0));
        assert!((p.center[0] - (-25.0)).abs() < 1e-9);
        assert!((p.center[1] - 10.0).abs() < 1e-9);
    }
}

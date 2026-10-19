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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    #[default]
    Contain,
    Cover,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera2D {
    pub center: [f64; 2],
    pub scale: [f64; 2],
}

impl Default for Camera2D {
    fn default() -> Self {
        Viewport::default().into()
    }
}

impl From<Viewport> for Camera2D {
    fn from(viewport: Viewport) -> Self {
        Self {
            center: viewport.center,
            scale: [viewport.zoom; 2],
        }
    }
}

impl Camera2D {
    pub fn is_valid(&self) -> bool {
        self.center.into_iter().all(f64::is_finite)
            && self
                .scale
                .into_iter()
                .all(|scale| scale.is_finite() && scale > 0.0)
    }

    /// A scalar cannot represent fill's independent axes; this explicit projection
    /// keeps legacy camera inspection useful without being used for rendering.
    pub fn scalar_viewport(&self) -> Viewport {
        Viewport {
            center: self.center,
            zoom: self.scale[0].min(self.scale[1]),
        }
    }
}

pub fn camera_screen_to_world(screen: DVec2, camera: &Camera2D, screen_size: DVec2) -> DVec2 {
    (screen - screen_size * 0.5) / DVec2::from(camera.scale) + DVec2::from(camera.center)
}

pub fn camera_world_to_screen(world: DVec2, camera: &Camera2D, screen_size: DVec2) -> DVec2 {
    (world - DVec2::from(camera.center)) * DVec2::from(camera.scale) + screen_size * 0.5
}

pub fn pan_camera(camera: &Camera2D, screen_delta: DVec2) -> Camera2D {
    if !camera.is_valid() || !screen_delta.is_finite() {
        return *camera;
    }
    let panned = Camera2D {
        center: (DVec2::from(camera.center) - screen_delta / DVec2::from(camera.scale)).to_array(),
        ..*camera
    };
    if panned.is_valid() { panned } else { *camera }
}

pub fn zoom_camera_at(
    camera: &Camera2D,
    screen_anchor: DVec2,
    factor: f64,
    screen_size: DVec2,
) -> Camera2D {
    if !camera.is_valid()
        || !screen_anchor.is_finite()
        || !screen_size.is_finite()
        || !factor.is_finite()
        || factor <= 0.0
    {
        return *camera;
    }
    let minimum_factor = (MIN_ZOOM / camera.scale[0]).max(MIN_ZOOM / camera.scale[1]);
    let maximum_factor = (MAX_ZOOM / camera.scale[0]).min(MAX_ZOOM / camera.scale[1]);
    if minimum_factor > maximum_factor {
        return *camera;
    }
    let factor = factor.clamp(minimum_factor, maximum_factor);
    let scale = DVec2::from(camera.scale) * factor;
    let world_before = camera_screen_to_world(screen_anchor, camera, screen_size);
    Camera2D {
        center: (world_before - (screen_anchor - screen_size * 0.5) / scale).to_array(),
        scale: scale.to_array(),
    }
}

/// Extreme aspect ratios clamp each fitted axis to the legacy projection limits;
/// subsequent gestures preserve that bounded ratio with a common zoom factor.
pub fn fit_camera(bounds: Bounds, screen_size: DVec2, padding_px: f64, mode: FitMode) -> Camera2D {
    let center = bounds.center();
    if !center.is_finite() || !bounds.width().is_finite() || !bounds.height().is_finite() {
        return Camera2D::default();
    }
    let size = DVec2::new(
        bounds.width().max(f64::EPSILON),
        bounds.height().max(f64::EPSILON),
    );
    let usable = (screen_size - DVec2::splat(2.0 * padding_px)).max(DVec2::ONE);
    let ratios = usable / size;
    let scale = match mode {
        FitMode::Contain => DVec2::splat(ratios.min_element()),
        FitMode::Cover => DVec2::splat(ratios.max_element()),
        FitMode::Fill => ratios,
    }
    .clamp(DVec2::splat(MIN_ZOOM), DVec2::splat(MAX_ZOOM));
    Camera2D {
        center: center.to_array(),
        scale: scale.to_array(),
    }
}

pub fn rebase_camera(camera: &Camera2D, old_fit: &Camera2D, new_fit: &Camera2D) -> Camera2D {
    if !camera.is_valid() || !old_fit.is_valid() || !new_fit.is_valid() {
        return *new_fit;
    }
    let minimum_factor = (MIN_ZOOM / new_fit.scale[0]).max(MIN_ZOOM / new_fit.scale[1]);
    let maximum_factor = (MAX_ZOOM / new_fit.scale[0]).min(MAX_ZOOM / new_fit.scale[1]);
    if minimum_factor > maximum_factor {
        return *new_fit;
    }
    let factor = (camera.scale[0] / old_fit.scale[0])
        .min(camera.scale[1] / old_fit.scale[1])
        .clamp(minimum_factor, maximum_factor);
    Camera2D {
        center: (DVec2::from(new_fit.center)
            + (DVec2::from(camera.center) - DVec2::from(old_fit.center)))
        .to_array(),
        scale: (DVec2::from(new_fit.scale) * factor).to_array(),
    }
}

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

    #[test]
    fn camera_fit_modes_map_translated_frame_corners_and_round_trip() {
        let bounds = Bounds::from_xywh(400.0, 60.0, 200.0, 100.0);
        let size = DVec2::splat(300.0);
        for (mode, scale, origin, corner) in [
            (FitMode::Contain, [1.5, 1.5], [0.0, 75.0], [300.0, 225.0]),
            (FitMode::Cover, [3.0, 3.0], [-150.0, 0.0], [450.0, 300.0]),
            (FitMode::Fill, [1.5, 3.0], [0.0, 0.0], [300.0, 300.0]),
        ] {
            let camera = fit_camera(bounds, size, 0.0, mode);
            assert_eq!(camera.center, [500.0, 110.0]);
            assert_eq!(camera.scale, scale);
            assert_eq!(
                camera_world_to_screen(DVec2::new(400.0, 60.0), &camera, size).to_array(),
                origin
            );
            assert_eq!(
                camera_world_to_screen(DVec2::new(600.0, 160.0), &camera, size).to_array(),
                corner
            );
            for point in [DVec2::ZERO, DVec2::new(105.0, 80.0), size] {
                let world = camera_screen_to_world(point, &camera, size);
                assert!((camera_world_to_screen(world, &camera, size) - point).length() < 1e-9);
            }
        }
    }

    #[test]
    fn camera_contain_and_uniform_adapters_preserve_scalar_math() {
        let bounds = Bounds::from_xywh(-150.0, 200.0, 170.0, 430.0);
        let size = DVec2::new(600.0, 400.0);
        let viewport = fit_bounds(bounds, size, 25.0);
        let camera = fit_camera(bounds, size, 25.0, FitMode::Contain);
        assert_eq!(camera, Camera2D::from(viewport));
        assert_eq!(camera.scalar_viewport(), viewport);
        let point = DVec2::new(132.0, 255.0);
        assert_eq!(
            camera_world_to_screen(point, &camera, size),
            world_to_screen(point, &viewport, size)
        );
        assert!(
            (camera_screen_to_world(point, &camera, size)
                - screen_to_world(point, &viewport, size))
            .length()
                < 1e-12
        );
    }

    #[test]
    fn camera_fill_anchored_zoom_and_pan_preserve_both_axes() {
        let size = DVec2::splat(300.0);
        let camera = fit_camera(
            Bounds::from_xywh(400.0, 60.0, 200.0, 100.0),
            size,
            0.0,
            FitMode::Fill,
        );
        let anchor = DVec2::new(60.0, 90.0);
        assert_eq!(
            camera_screen_to_world(anchor, &camera, size),
            DVec2::new(440.0, 90.0)
        );
        let zoomed = zoom_camera_at(&camera, anchor, 2.0, size);
        assert_eq!(zoomed.center, [470.0, 100.0]);
        assert_eq!(zoomed.scale, [3.0, 6.0]);
        assert_eq!(
            camera_screen_to_world(anchor, &zoomed, size),
            DVec2::new(440.0, 90.0)
        );
        let panned = pan_camera(&zoomed, DVec2::new(30.0, -12.0));
        assert_eq!(panned.center, [460.0, 102.0]);
        assert_eq!(panned.scale, [3.0, 6.0]);
        assert_eq!(
            camera_world_to_screen(DVec2::new(440.0, 90.0), &panned, size),
            anchor + DVec2::new(30.0, -12.0)
        );
    }

    #[test]
    fn camera_zoom_clamps_jointly_without_changing_the_axis_ratio() {
        let camera = Camera2D {
            center: [25.0, -50.0],
            scale: [1.5, 3.0],
        };
        let size = DVec2::splat(300.0);
        let anchor = DVec2::new(73.0, 129.0);
        let world = camera_screen_to_world(anchor, &camera, size);
        for factor in [1e-100, 1e100] {
            let changed = zoom_camera_at(&camera, anchor, factor, size);
            assert_eq!(changed.scale[1] / changed.scale[0], 2.0);
            assert!(
                changed
                    .scale
                    .into_iter()
                    .all(|axis| (MIN_ZOOM..=MAX_ZOOM).contains(&axis))
            );
            assert!((camera_screen_to_world(anchor, &changed, size) - world).length() < 1e-7);
        }
        for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(zoom_camera_at(&camera, anchor, factor, size), camera);
        }
        assert_eq!(pan_camera(&camera, DVec2::new(f64::NAN, 0.0)), camera);
    }

    #[test]
    fn camera_rebase_preserves_manual_zoom_and_world_center_displacement() {
        let bounds = Bounds::from_xywh(400.0, 60.0, 200.0, 100.0);
        let old_fit = fit_camera(bounds, DVec2::splat(300.0), 0.0, FitMode::Fill);
        let camera = Camera2D {
            center: [470.0, 100.0],
            scale: [3.0, 6.0],
        };
        let new_fit = fit_camera(bounds, DVec2::new(400.0, 200.0), 0.0, FitMode::Fill);
        let rebased = rebase_camera(&camera, &old_fit, &new_fit);
        assert_eq!(rebased.center, [470.0, 100.0]);
        assert_eq!(rebased.scale, [4.0, 4.0]);
        let cover = fit_camera(bounds, DVec2::splat(300.0), 0.0, FitMode::Cover);
        assert_eq!(rebase_camera(&camera, &old_fit, &cover).scale, [6.0, 6.0]);
    }

    #[test]
    fn camera_degenerate_bounds_stay_finite_and_serialization_is_transient()
    -> Result<(), serde_json::Error> {
        for mode in [FitMode::Contain, FitMode::Cover, FitMode::Fill] {
            let camera = fit_camera(
                Bounds::from_xywh(-5.0, 7.0, 0.0, 0.0),
                DVec2::splat(300.0),
                0.0,
                mode,
            );
            assert!(camera.is_valid());
            assert_eq!(camera.center, [-5.0, 7.0]);
            assert_eq!(camera.scale, [MAX_ZOOM; 2]);
        }
        assert_eq!(serde_json::to_string(&FitMode::Fill)?, "\"fill\"");
        let viewport = vp(1.0, 2.0, 3.0);
        assert_eq!(
            serde_json::to_value(viewport)?,
            serde_json::json!({"center":[1.0,2.0],"zoom":3.0})
        );
        Ok(())
    }

    #[test]
    fn camera_rebase_jointly_clamps_manual_zoom_at_extreme_resize_and_mode_change() {
        let old_fit = Camera2D {
            center: [100.0, 50.0],
            scale: [1.5, 3.0],
        };
        let camera = Camera2D {
            center: [90.0, 55.0],
            scale: [3.0, 6.0],
        };
        let new_fit = Camera2D {
            center: [100.0, 50.0],
            scale: [1024.0, 3072.0],
        };
        let rebased = rebase_camera(&camera, &old_fit, &new_fit);
        assert_eq!(rebased.center, [90.0, 55.0]);
        assert_eq!(rebased.scale[1], MAX_ZOOM);
        assert!((rebased.scale[1] / rebased.scale[0] - 3.0).abs() < 1e-12);
        let tiny_fit = Camera2D {
            center: [100.0, 50.0],
            scale: [MIN_ZOOM, MIN_ZOOM * 3.0],
        };
        let zoomed_out = Camera2D {
            center: old_fit.center,
            scale: [0.75, 1.5],
        };
        assert_eq!(
            rebase_camera(&zoomed_out, &old_fit, &tiny_fit).scale,
            tiny_fit.scale
        );
        let extreme = fit_camera(
            Bounds::from_xywh(0.0, 0.0, 1e20, 1e-20),
            DVec2::splat(300.0),
            0.0,
            FitMode::Fill,
        );
        assert_eq!(extreme.scale, [MIN_ZOOM, MAX_ZOOM]);
    }
}

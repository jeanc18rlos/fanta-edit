//! Overlay placement and layer compositing for
//! [`PresentSession`](crate::PresentSession).
//!
//! An overlay is a second frame drawn on top of the base frame, positioned in
//! the *viewport* (not world space) per its [`OverlayPosition`] at the base
//! frame's zoom. This module computes the overlay's viewport + on-screen rect
//! and provides the straight-alpha layer compositing (scrim + over) the session
//! uses to stack base → scrim → overlay(s). Like [`crate::transition`] it is
//! pure buffer / geometry math — no Skia, no Doc mutation.

use fanta_doc::{Bounds, OverlayPosition, Viewport};
use glam::DVec2;

/// An overlay's rectangle on the present surface, in physical pixels. Used to
/// decide whether a click landed outside a dismissable overlay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ScreenRect {
    pub min: DVec2,
    pub max: DVec2,
}

impl ScreenRect {
    /// Whether a screen point lies within the rect (edges inclusive).
    pub(crate) fn contains(&self, p: DVec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}

/// The scrim darkening applied behind a `background_dim` overlay (fraction of
/// black mixed into the backdrop).
pub(crate) const SCRIM_DIM: f64 = 0.35;

/// Compute the viewport that renders `overlay_bounds` at `base_zoom`, placed per
/// `position` within `screen_size`, plus the resulting on-screen rect.
///
/// The overlay keeps the base frame's zoom (Figma presents overlays 1:1 with
/// the frame beneath) and is positioned by choosing the viewport center so the
/// overlay's world min-corner maps to the desired screen top-left.
pub(crate) fn place_overlay(
    base_zoom: f64,
    overlay_bounds: Bounds,
    position: OverlayPosition,
    screen_size: DVec2,
) -> (Viewport, ScreenRect) {
    let z = base_zoom.max(f64::EPSILON);
    let world_min = DVec2::new(overlay_bounds.min_x, overlay_bounds.min_y);
    let size = DVec2::new(overlay_bounds.width() * z, overlay_bounds.height() * z);
    // Free room to slide the overlay within the surface (clamped ≥ 0 so an
    // overlay larger than the screen pins to the top-left rather than jumping).
    let free = (screen_size - size).max(DVec2::ZERO);
    let top_left = match position {
        OverlayPosition::Center => free * 0.5,
        OverlayPosition::TopLeft => DVec2::ZERO,
        OverlayPosition::TopCenter => DVec2::new(free.x * 0.5, 0.0),
        OverlayPosition::TopRight => DVec2::new(free.x, 0.0),
        OverlayPosition::BottomLeft => DVec2::new(0.0, free.y),
        OverlayPosition::BottomCenter => DVec2::new(free.x * 0.5, free.y),
        OverlayPosition::BottomRight => free,
        // Manual offset is measured from the surface top-left, in frame units
        // scaled to screen pixels by the zoom.
        OverlayPosition::Manual { offset } => DVec2::new(offset[0] * z, offset[1] * z),
    };
    // world_to_screen(world_min) = (world_min - center) * z + screen/2 == top_left
    //   ⇒ center = world_min - (top_left - screen/2) / z
    let center = world_min - (top_left - screen_size * 0.5) / z;
    let viewport = Viewport {
        center: [center.x, center.y],
        zoom: z,
    };
    let rect = ScreenRect {
        min: top_left,
        max: top_left + size,
    };
    (viewport, rect)
}

/// Composite `top` over `base` in place (both equal-size straight-alpha RGBA8),
/// reusing the shared pixel `over`. A `top` pixel with zero alpha leaves the
/// base untouched — that is how the transparent margin around an overlay frame
/// reveals the backdrop.
pub(crate) fn composite_over(base: &mut [u8], top: &[u8]) {
    let n = base.len().min(top.len()) / 4 * 4;
    let mut i = 0;
    while i < n {
        let ta = top[i + 3];
        if ta == 0 {
            i += 4;
            continue;
        }
        if ta == 255 {
            base[i..i + 4].copy_from_slice(&top[i..i + 4]);
            i += 4;
            continue;
        }
        let top_px = [top[i], top[i + 1], top[i + 2], top[i + 3]];
        let base_px = [base[i], base[i + 1], base[i + 2], base[i + 3]];
        base[i..i + 4].copy_from_slice(&crate::transition::over(top_px, base_px));
        i += 4;
    }
}

/// Darken every pixel's RGB toward black by `amount` (0..=1), leaving alpha
/// untouched — the backdrop dim behind a `background_dim` overlay.
pub(crate) fn apply_scrim(buffer: &mut [u8], amount: f64) {
    let keep = (1.0 - amount.clamp(0.0, 1.0)).clamp(0.0, 1.0);
    for px in buffer.chunks_mut(4) {
        if px.len() < 4 {
            continue;
        }
        for channel in px.iter_mut().take(3) {
            *channel = (*channel as f64 * keep).round().clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f64, y: f64, w: f64, h: f64) -> Bounds {
        Bounds::from_xywh(x, y, w, h)
    }

    #[test]
    fn center_places_a_small_overlay_in_the_middle() {
        let screen = DVec2::new(200.0, 200.0);
        let (_, rect) = place_overlay(
            1.0,
            bounds(1000.0, 0.0, 100.0, 100.0),
            OverlayPosition::Center,
            screen,
        );
        assert_eq!(rect.min, DVec2::new(50.0, 50.0));
        assert_eq!(rect.max, DVec2::new(150.0, 150.0));
        assert!(rect.contains(DVec2::new(100.0, 100.0)));
        assert!(!rect.contains(DVec2::new(10.0, 10.0)));
    }

    #[test]
    fn corners_pin_to_the_edges() {
        let screen = DVec2::new(200.0, 200.0);
        let b = bounds(0.0, 0.0, 100.0, 100.0);
        let (_, tl) = place_overlay(1.0, b, OverlayPosition::TopLeft, screen);
        assert_eq!(tl.min, DVec2::ZERO);
        let (_, br) = place_overlay(1.0, b, OverlayPosition::BottomRight, screen);
        assert_eq!(br.max, screen);
    }

    #[test]
    fn placement_viewport_round_trips_the_min_corner() {
        use fanta_canvas::viewport::world_to_screen;
        let screen = DVec2::new(200.0, 200.0);
        let b = bounds(1000.0, 0.0, 100.0, 100.0);
        let (viewport, rect) = place_overlay(1.0, b, OverlayPosition::Center, screen);
        let mapped = world_to_screen(DVec2::new(b.min_x, b.min_y), &viewport, screen);
        assert!(
            (mapped - rect.min).length() < 1e-9,
            "{mapped:?} vs {:?}",
            rect.min
        );
    }

    #[test]
    fn composite_over_respects_transparency() {
        // 1 opaque green px over 1 opaque red px ⇒ green.
        let mut base = vec![255, 0, 0, 255];
        composite_over(&mut base, &[0, 255, 0, 255]);
        assert_eq!(base, vec![0, 255, 0, 255]);
        // Transparent top leaves base.
        let mut base = vec![255, 0, 0, 255];
        composite_over(&mut base, &[0, 255, 0, 0]);
        assert_eq!(base, vec![255, 0, 0, 255]);
    }

    #[test]
    fn scrim_darkens_rgb_only() {
        let mut buf = vec![200, 100, 50, 255];
        apply_scrim(&mut buf, 0.5);
        assert_eq!(buf, vec![100, 50, 25, 255], "alpha preserved, rgb halved");
    }
}

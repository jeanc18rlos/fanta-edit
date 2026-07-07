//! Visible-world-rect derivation — the reference region the viewport cull
//! tests every node's (shadow-expanded) world AABB against.
use super::{Bounds, Viewport};

// ---------------------------------------------------------------------------
// Visible-world-rect derivation (the cull test's reference region)
// ---------------------------------------------------------------------------

/// Slack added to every edge of the visible world rect, in world units. It
/// keeps a node sitting exactly on the screen edge (or anti-aliased a texel
/// past it) from being culled and flickering. Tiny relative to any real node,
/// so it never causes a genuinely off-screen node to be drawn.
pub(crate) const CULL_MARGIN_WORLD: f64 = 2.0;

/// The rectangle of WORLD space currently shown by a `width` × `height` surface
/// under `viewport`, as the exact inverse of the transform
/// [`RasterRenderer::render`] applies.
///
/// `render` maps a world point `p` to a physical-pixel point `s` by (in order)
/// translating by `+half`, scaling by `effective_scale`, and translating by
/// `-center`:
///
/// ```text
///   s = (p - center) * effective_scale + half
///   effective_scale = viewport.zoom * display_scale
///   half = (width/2, height/2)
/// ```
///
/// Inverting for the world point under a given screen pixel:
///
/// ```text
///   p = (s - half) / effective_scale + center
/// ```
///
/// The surface covers screen pixels `s ∈ [0, width] × [0, height]`. The
/// transform is an orientation-preserving scale+translate, so it carries that
/// axis-aligned screen rect to an axis-aligned world rect whose corners are the
/// images of `s = (0, 0)` and `s = (width, height)`:
///
/// ```text
///   world_min = (0    - half) / effective_scale + center = center - half / effective_scale
///   world_max = (size - half) / effective_scale + center = center + half / effective_scale
/// ```
///
/// i.e. the visible world rect is `center ± half / effective_scale`, expanded
/// by [`CULL_MARGIN_WORLD`] on each side.
///
/// A non-positive or NaN `effective_scale` (a degenerate zero/negative zoom or
/// display scale) cannot be inverted; we return an infinite rect so culling is
/// disabled rather than wrongly hiding everything.
pub(crate) fn visible_world_rect(
    width: u32,
    height: u32,
    display_scale: f64,
    viewport: &Viewport,
) -> Bounds {
    let effective_scale = viewport.zoom * display_scale;
    if effective_scale.is_nan() || effective_scale <= 0.0 {
        // NaN or non-positive: disable culling with an all-encompassing rect.
        return Bounds {
            min_x: f64::NEG_INFINITY,
            min_y: f64::NEG_INFINITY,
            max_x: f64::INFINITY,
            max_y: f64::INFINITY,
        };
    }
    let half_w_world = (width as f64 * 0.5) / effective_scale;
    let half_h_world = (height as f64 * 0.5) / effective_scale;
    let cx = viewport.center[0];
    let cy = viewport.center[1];
    Bounds {
        min_x: cx - half_w_world - CULL_MARGIN_WORLD,
        min_y: cy - half_h_world - CULL_MARGIN_WORLD,
        max_x: cx + half_w_world + CULL_MARGIN_WORLD,
        max_y: cy + half_h_world + CULL_MARGIN_WORLD,
    }
}

//! Transition compositing for [`PresentSession`](crate::PresentSession).
//!
//! A navigation transition is played by rendering the outgoing and incoming
//! frames to two straight-alpha RGBA8 rasters and blending them per-pixel here.
//! This module is deliberately Skia-free and Doc-free: it is pure buffer math,
//! so the easing curves and the per-style compositing are unit-tested on their
//! own without allocating a render surface.
//!
//! Coordinate convention for the directional styles: a [`Direction`] names the
//! edge the content enters *from*, matching Figma's authoring UI. `Left` means
//! the incoming frame starts one surface-width left of the viewport and travels
//! right. Offsets are in physical destination pixels.
//!
//! [`Push`]: TransitionStyle::Push

use fanta_doc::{Direction, Easing, NodeId, TransitionStyle, Viewport};

const TIME_EPSILON_SECONDS: f64 = 1e-9;

/// A navigation transition in flight: the two endpoints plus the animation
/// parameters. The session advances `elapsed` in [`tick`](crate::PresentSession::tick)
/// and composites via [`composite`] until `elapsed >= duration`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ActiveTransition {
    pub from_frame: NodeId,
    pub from_viewport: Viewport,
    pub to_frame: NodeId,
    pub to_viewport: Viewport,
    pub style: TransitionStyle,
    pub easing: Easing,
    /// Total duration in seconds (always > 0 — an instant/zero transition is a
    /// cut and never becomes an `ActiveTransition`).
    pub duration: f64,
    /// Seconds elapsed since the transition began.
    pub elapsed: f64,
}

impl ActiveTransition {
    /// Linear progress in `0.0..=1.0` (before easing).
    pub(crate) fn progress(&self) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// Whether the transition has run its full duration.
    pub(crate) fn done(&self) -> bool {
        self.elapsed + TIME_EPSILON_SECONDS >= self.duration
    }
}

/// Map linear progress `t` through an easing curve. Endpoints are exact
/// (`ease(_, 0) == 0`, `ease(_, 1) == 1`). Springs may return values ABOVE
/// 1.0 mid-flight — that overshoot flows into the directional offsets so a
/// bouncy slide visibly overshoots its resting position; consumers that need
/// saturation (crossfade weights) clamp at their own boundary.
pub(crate) fn ease(easing: Easing, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => t,
        Easing::EaseIn => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        Easing::EaseOut => cubic_bezier(0.0, 0.0, 0.58, 1.0, t),
        Easing::EaseInOut => cubic_bezier(0.42, 0.0, 0.58, 1.0, t),
        Easing::CubicBezier { x1, y1, x2, y2 } => {
            cubic_bezier(x1 as f64, y1 as f64, x2 as f64, y2 as f64, t)
        }
        // The closed-form damped-spring sampler lives in fanta-doc so motion
        // clips and present transitions share one physical model.
        Easing::Spring {
            mass,
            stiffness,
            damping,
        } => fanta_doc::spring_progress(mass, stiffness, damping, t),
    }
}

fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, mut t: f64) -> f64 {
    t = t.clamp(0.0, 1.0);
    if t <= 0.0 || t >= 1.0 {
        return t;
    }

    let coordinate = |parameter: f64, first: f64, second: f64| {
        let inverse = 1.0 - parameter;
        3.0 * inverse * inverse * parameter * first
            + 3.0 * inverse * parameter * parameter * second
            + parameter * parameter * parameter
    };
    let derivative = |parameter: f64, first: f64, second: f64| {
        let inverse = 1.0 - parameter;
        3.0 * inverse * inverse * first
            + 6.0 * inverse * parameter * (second - first)
            + 3.0 * parameter * parameter * (1.0 - second)
    };

    let mut parameter = t;
    for _ in 0..8 {
        let error = coordinate(parameter, x1, x2) - t;
        if error.abs() <= 1e-7 {
            return coordinate(parameter, y1, y2).clamp(0.0, 1.0);
        }
        let slope = derivative(parameter, x1, x2);
        if slope.abs() <= 1e-7 {
            break;
        }
        let next = parameter - error / slope;
        if !(0.0..=1.0).contains(&next) {
            break;
        }
        parameter = next;
    }

    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        parameter = (low + high) * 0.5;
        if coordinate(parameter, x1, x2) < t {
            low = parameter;
        } else {
            high = parameter;
        }
    }
    coordinate(parameter, y1, y2).clamp(0.0, 1.0)
}

/// The unit vector, in pixel space (x right, y down), that content entering
/// from `direction` travels along.
fn travel_from(direction: Direction) -> (f64, f64) {
    match direction {
        Direction::Left => (1.0, 0.0),
        Direction::Right => (-1.0, 0.0),
        Direction::Up => (0.0, 1.0),
        Direction::Down => (0.0, -1.0),
    }
}

/// The unit vector content EXITING toward `direction` travels along. The Out
/// styles flip the direction language: `SlideOut { Left }` is Figma's "slide
/// out to left" — the outgoing frame moves leftward off the surface — so the
/// travel vector is the exact negation of the In styles' entry travel.
fn travel_toward(direction: Direction) -> (f64, f64) {
    let (x, y) = travel_from(direction);
    (-x, -y)
}

/// Whether this style animates the OUTGOING frame on top of a stationary /
/// near-stationary incoming frame (the mirrored layer roles of the Out
/// styles), as opposed to the In styles' incoming-on-top compositing.
fn is_out_style(style: TransitionStyle) -> bool {
    matches!(
        style,
        TransitionStyle::SlideOut { .. } | TransitionStyle::MoveOut { .. }
    )
}

/// Outgoing-frame parallax for [`SlideIn`](TransitionStyle::SlideIn): the old
/// frame drifts a fraction of the incoming's travel so the two styles read
/// differently (`MoveIn` keeps the old frame still).
const SLIDE_PARALLAX: f64 = 0.25;

/// Per-layer pixel offsets `(outgoing, incoming)` at eased progress `e` for a
/// directional style. Each offset shifts that layer's raster within the
/// destination.
///
/// In styles: the incoming ends at `(0, 0)`; the outgoing moves only for
/// `Push`/`SlideIn`.
///
/// Out styles mirror the roles: the OUTGOING travels a full surface toward
/// its exit edge (ending off-screen) while the incoming rests at / settles to
/// `(0, 0)` — `SlideOut` gives the incoming the same small parallax drift the
/// outgoing gets in `SlideIn` (it starts a fraction toward the opposite edge
/// and settles), `MoveOut` keeps it perfectly still beneath.
pub(crate) fn directional_offsets(
    style: TransitionStyle,
    direction: Direction,
    e: f64,
    width: f64,
    height: f64,
) -> ((f64, f64), (f64, f64)) {
    if is_out_style(style) {
        let (tx, ty) = travel_toward(direction);
        // Outgoing exits a full surface toward the named edge.
        let out_off = (tx * width * e, ty * height * e);
        let in_off = match style {
            // Mirrored SlideIn parallax: the incoming starts displaced a
            // quarter-surface on the side the outgoing is exiting AWAY from
            // and drifts into place as the outgoing leaves.
            TransitionStyle::SlideOut { .. } => (
                -tx * width * SLIDE_PARALLAX * (1.0 - e),
                -ty * height * SLIDE_PARALLAX * (1.0 - e),
            ),
            // MoveOut: the incoming is already at rest beneath.
            _ => (0.0, 0.0),
        };
        return (out_off, in_off);
    }

    let (tx, ty) = travel_from(direction);
    // Incoming starts one surface-length back along the travel axis (i.e. on the
    // side opposite the travel direction) and slides to zero.
    let in_off = (-tx * width * (1.0 - e), -ty * height * (1.0 - e));
    let out_off = match style {
        // The old frame is pushed a full surface in the travel direction.
        TransitionStyle::Push { .. } => (tx * width * e, ty * height * e),
        // The old frame drifts a little in the travel direction.
        TransitionStyle::SlideIn { .. } => (
            tx * width * e * SLIDE_PARALLAX,
            ty * height * e * SLIDE_PARALLAX,
        ),
        // MoveIn (and any non-directional style routed here) keeps it still.
        _ => (0.0, 0.0),
    };
    (out_off, in_off)
}

pub(crate) fn overlay_offset(
    style: TransitionStyle,
    e: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    match style {
        TransitionStyle::SlideIn { direction }
        | TransitionStyle::Push { direction }
        | TransitionStyle::MoveIn { direction } => {
            let (travel_x, travel_y) = travel_from(direction);
            (
                -travel_x * width * (1.0 - e.clamp(0.0, 1.0)),
                -travel_y * height * (1.0 - e.clamp(0.0, 1.0)),
            )
        }
        // Out styles on an overlay: the overlay is the single moving layer, so
        // "out" means it sits off toward its exit edge at zero visibility and
        // rests at zero offset when fully visible. Used with the exit clock
        // (visible 1 → 0) this plays the overlay sliding away toward the edge.
        TransitionStyle::SlideOut { direction } | TransitionStyle::MoveOut { direction } => {
            let (travel_x, travel_y) = travel_toward(direction);
            (
                travel_x * width * (1.0 - e.clamp(0.0, 1.0)),
                travel_y * height * (1.0 - e.clamp(0.0, 1.0)),
            )
        }
        _ => (0.0, 0.0),
    }
}

/// Composite the outgoing and incoming rasters (both `width * height * 4`
/// straight-alpha RGBA8) into a new buffer at eased progress `e`. `Instant`
/// returns the incoming frame unchanged (it should not reach here — the session
/// cuts instead — but is handled so the match is total).
pub(crate) fn composite(
    style: TransitionStyle,
    e: f64,
    width: usize,
    height: usize,
    outgoing: &[u8],
    incoming: &[u8],
) -> Vec<u8> {
    let expected = width.saturating_mul(height).saturating_mul(4);
    // Defensive: mismatched inputs would index out of bounds below.
    if outgoing.len() < expected || incoming.len() < expected {
        return incoming.to_vec();
    }

    match style {
        TransitionStyle::Instant => incoming.to_vec(),
        // The session uses a matched-layer scratch scene when one can be
        // prepared. A dissolve is the safe visual fallback for state/overlay
        // transitions and malformed documents that cannot produce that plan.
        // ScrollAnimate has no frame-vs-frame geometry of its own (its real
        // effect is the eased scroll offset the session animates); a document
        // that authored it on a NAVIGATION degrades to the same safe dissolve.
        TransitionStyle::SmartAnimate
        | TransitionStyle::Dissolve
        | TransitionStyle::ScrollAnimate => crossfade(e, expected, outgoing, incoming),
        TransitionStyle::SlideIn { direction }
        | TransitionStyle::Push { direction }
        | TransitionStyle::MoveIn { direction } => {
            let (out_off, in_off) =
                directional_offsets(style, direction, e, width as f64, height as f64);
            slide(width, height, out_off, in_off, outgoing, incoming)
        }
        // Out styles mirror the layer roles: the OUTGOING frame is the moving
        // top layer, revealing the incoming beneath — so the two rasters swap
        // seats in `slide` (whose second pair is composited on top).
        TransitionStyle::SlideOut { direction } | TransitionStyle::MoveOut { direction } => {
            let (out_off, in_off) =
                directional_offsets(style, direction, e, width as f64, height as f64);
            slide(width, height, in_off, out_off, incoming, outgoing)
        }
    }
}

/// Animate a transparent overlay layer over an already-composited backdrop.
///
/// Unlike a frame navigation, the backdrop must remain stable while an overlay
/// enters. Directional styles therefore translate only `overlay`; dissolve and
/// Smart Animate fade it in. Smart Animate uses this crossfade when there is no
/// meaningful pair of same-level frame trees to match.
pub(crate) fn composite_overlay(
    style: TransitionStyle,
    e: f64,
    width: usize,
    height: usize,
    backdrop: &[u8],
    overlay: &[u8],
) -> Vec<u8> {
    let expected = width.saturating_mul(height).saturating_mul(4);
    if backdrop.len() < expected || overlay.len() < expected {
        return backdrop.to_vec();
    }

    match style {
        TransitionStyle::Instant => overlay_over(backdrop, overlay, expected, 1.0),
        TransitionStyle::Dissolve
        | TransitionStyle::SmartAnimate
        | TransitionStyle::ScrollAnimate => overlay_over(backdrop, overlay, expected, e),
        // The overlay is the single moving layer for every directional style —
        // In styles travel it in from its entry edge, Out styles from/toward
        // its exit edge (see `overlay_offset`); the backdrop stays put.
        TransitionStyle::SlideIn { .. }
        | TransitionStyle::Push { .. }
        | TransitionStyle::MoveIn { .. }
        | TransitionStyle::SlideOut { .. }
        | TransitionStyle::MoveOut { .. } => {
            let offset = overlay_offset(style, e, width as f64, height as f64);
            shifted_overlay(width, height, offset, backdrop, overlay)
        }
    }
}

fn overlay_over(backdrop: &[u8], overlay: &[u8], len: usize, opacity: f64) -> Vec<u8> {
    let opacity = opacity.clamp(0.0, 1.0);
    let mut result = backdrop[..len].to_vec();
    for (base, top) in result
        .chunks_exact_mut(4)
        .zip(overlay[..len].chunks_exact(4))
    {
        let alpha = (f64::from(top[3]) * opacity).round().clamp(0.0, 255.0) as u8;
        let composited = over(
            [top[0], top[1], top[2], alpha],
            [base[0], base[1], base[2], base[3]],
        );
        base.copy_from_slice(&composited);
    }
    result
}

fn shifted_overlay(
    width: usize,
    height: usize,
    offset: (f64, f64),
    backdrop: &[u8],
    overlay: &[u8],
) -> Vec<u8> {
    let (offset_x, offset_y) = (offset.0.round() as i64, offset.1.round() as i64);
    let width_i64 = width as i64;
    let height_i64 = height as i64;
    let mut result = backdrop[..width * height * 4].to_vec();
    for y in 0..height_i64 {
        for x in 0..width_i64 {
            let Some(top) = sample(overlay, x - offset_x, y - offset_y, width_i64, height_i64)
            else {
                continue;
            };
            if top[3] == 0 {
                continue;
            }
            let destination = ((y * width_i64 + x) * 4) as usize;
            let bottom = [
                result[destination],
                result[destination + 1],
                result[destination + 2],
                result[destination + 3],
            ];
            result[destination..destination + 4].copy_from_slice(&over(top, bottom));
        }
    }
    result
}

/// Crossfade two straight-alpha colors in premultiplied space, then return a
/// straight-alpha buffer. Transparent endpoint RGB never leaks into the blend.
fn crossfade(e: f64, len: usize, outgoing: &[u8], incoming: &[u8]) -> Vec<u8> {
    let e = e.clamp(0.0, 1.0);
    if e <= 0.0 {
        return outgoing[..len].to_vec();
    }
    if e >= 1.0 {
        return incoming[..len].to_vec();
    }
    let mut result = Vec::with_capacity(len);
    for (outgoing, incoming) in outgoing[..len]
        .chunks_exact(4)
        .zip(incoming[..len].chunks_exact(4))
    {
        let outgoing_alpha = f64::from(outgoing[3]) / 255.0;
        let incoming_alpha = f64::from(incoming[3]) / 255.0;
        let alpha = outgoing_alpha * (1.0 - e) + incoming_alpha * e;
        for channel in 0..3 {
            let premultiplied = f64::from(outgoing[channel]) * outgoing_alpha * (1.0 - e)
                + f64::from(incoming[channel]) * incoming_alpha * e;
            let value = if alpha > 0.0 {
                premultiplied / alpha
            } else {
                0.0
            };
            result.push(value.round().clamp(0.0, 255.0) as u8);
        }
        result.push((alpha * 255.0).round().clamp(0.0, 255.0) as u8);
    }
    result
}

/// Composite the incoming layer (offset by `in_off`) over the outgoing layer
/// (offset by `out_off`), rounding offsets to whole pixels. A destination pixel
/// whose incoming sample lands outside the raster shows the outgoing frame; one
/// where both land outside is transparent.
fn slide(
    width: usize,
    height: usize,
    out_off: (f64, f64),
    in_off: (f64, f64),
    outgoing: &[u8],
    incoming: &[u8],
) -> Vec<u8> {
    let (ox, oy) = (out_off.0.round() as i64, out_off.1.round() as i64);
    let (ix, iy) = (in_off.0.round() as i64, in_off.1.round() as i64);
    let w = width as i64;
    let h = height as i64;
    let mut result = vec![0u8; width * height * 4];

    for y in 0..h {
        for x in 0..w {
            let dst = ((y * w + x) * 4) as usize;
            let out_px = sample(outgoing, x - ox, y - oy, w, h);
            let in_px = sample(incoming, x - ix, y - iy, w, h);
            let px = match (in_px, out_px) {
                (Some(top), Some(bottom)) => over(top, bottom),
                (Some(top), None) => top,
                (None, Some(bottom)) => bottom,
                (None, None) => [0, 0, 0, 0],
            };
            result[dst..dst + 4].copy_from_slice(&px);
        }
    }
    result
}

/// Read the RGBA pixel at `(x, y)`, or `None` when it lies outside the raster.
fn sample(buf: &[u8], x: i64, y: i64, w: i64, h: i64) -> Option<[u8; 4]> {
    if x < 0 || y < 0 || x >= w || y >= h {
        return None;
    }
    let idx = ((y * w + x) * 4) as usize;
    let slice = buf.get(idx..idx + 4)?;
    Some([slice[0], slice[1], slice[2], slice[3]])
}

/// Straight-alpha "source over destination" of two RGBA pixels.
pub(crate) fn over(top: [u8; 4], bottom: [u8; 4]) -> [u8; 4] {
    let ta = top[3] as f64 / 255.0;
    if ta >= 1.0 {
        return top;
    }
    if ta <= 0.0 {
        return bottom;
    }
    let ba = bottom[3] as f64 / 255.0;
    let out_a = ta + ba * (1.0 - ta);
    if out_a <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mut px = [0u8; 4];
    for c in 0..3 {
        let tc = top[c] as f64 / 255.0;
        let bc = bottom[c] as f64 / 255.0;
        let blended = (tc * ta + bc * ba * (1.0 - ta)) / out_a;
        px[c] = (blended * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    px[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(direction: Direction) -> TransitionStyle {
        TransitionStyle::Push { direction }
    }

    #[test]
    fn easing_hits_its_endpoints() {
        for easing in [
            Easing::Linear,
            Easing::EaseIn,
            Easing::EaseOut,
            Easing::EaseInOut,
            Easing::CubicBezier {
                x1: 0.4,
                y1: 0.0,
                x2: 0.2,
                y2: 1.0,
            },
        ] {
            assert_eq!(ease(easing, 0.0), 0.0, "{easing:?} at 0");
            assert_eq!(ease(easing, 1.0), 1.0, "{easing:?} at 1");
        }
        // The symmetric curve passes through its own midpoint.
        assert!((ease(Easing::EaseInOut, 0.5) - 0.5).abs() < 1e-9);
        // Ease-in starts slow, ease-out starts fast.
        assert!(ease(Easing::EaseIn, 0.25) < 0.25);
        assert!(ease(Easing::EaseOut, 0.25) > 0.25);
        assert!((ease(Easing::EaseIn, 0.25) - 0.093_464_65).abs() < 1e-6);
        assert!((ease(Easing::EaseOut, 0.25) - 0.378_138_13).abs() < 1e-6);
        assert!((ease(Easing::EaseInOut, 0.25) - 0.129_161_93).abs() < 1e-6);
        // Sample custom cubic-bezier should be monotonic and reasonable.
        let custom = ease(
            Easing::CubicBezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            },
            0.5,
        );
        assert!(custom > 0.3 && custom < 0.7);
    }

    #[test]
    fn dissolve_endpoints_are_the_source_frames() {
        let out: Vec<u8> = (0..4).flat_map(|_| [10, 10, 10, 255]).collect();
        let inc: Vec<u8> = (0..4).flat_map(|_| [200, 200, 200, 255]).collect();
        assert_eq!(
            composite(TransitionStyle::Dissolve, 0.0, 2, 2, &out, &inc),
            out
        );
        assert_eq!(
            composite(TransitionStyle::Dissolve, 1.0, 2, 2, &out, &inc),
            inc
        );
        // Midpoint averages the two.
        let mid = composite(TransitionStyle::Dissolve, 0.5, 2, 2, &out, &inc);
        assert!(
            mid.chunks_exact(4)
                .all(|pixel| pixel == [105, 105, 105, 255])
        );
    }

    #[test]
    fn dissolve_blends_translucent_pixels_in_premultiplied_space() {
        let translucent_red = [255, 0, 0, 128];
        let transparent_blue = [0, 0, 255, 0];
        let midpoint = composite(
            TransitionStyle::Dissolve,
            0.5,
            1,
            1,
            &translucent_red,
            &transparent_blue,
        );
        assert_eq!(midpoint, [255, 0, 0, 64]);

        let translucent_blue = [0, 0, 255, 64];
        let midpoint = composite(
            TransitionStyle::Dissolve,
            0.5,
            1,
            1,
            &translucent_red,
            &translucent_blue,
        );
        assert_eq!(midpoint, [170, 0, 85, 96]);
        assert_eq!(
            over(
                [midpoint[0], midpoint[1], midpoint[2], midpoint[3]],
                [255, 255, 255, 255]
            ),
            [223, 159, 191, 255]
        );
    }

    #[test]
    fn push_endpoints_swap_the_visible_frame() {
        // 3×1 rasters, distinct opaque colors.
        let width = 3;
        let height = 1;
        let out: Vec<u8> = (0..width * height).flat_map(|_| [255, 0, 0, 255]).collect();
        let inc: Vec<u8> = (0..width * height).flat_map(|_| [0, 0, 255, 255]).collect();

        // At e=0 the incoming is fully off-screen: the outgoing shows.
        let start = composite(dir(Direction::Left), 0.0, width, height, &out, &inc);
        assert_eq!(start, out, "start of push shows outgoing");
        // At e=1 the incoming has fully arrived.
        let end = composite(dir(Direction::Left), 1.0, width, height, &out, &inc);
        assert_eq!(end, inc, "end of push shows incoming");
    }

    #[test]
    fn from_left_enters_through_the_left_edge() {
        let width = 4;
        let height = 1;
        let outgoing: Vec<u8> = (0..width).flat_map(|_| [255, 0, 0, 255]).collect();
        let incoming: Vec<u8> = (0..width).flat_map(|_| [0, 0, 255, 255]).collect();
        let style = TransitionStyle::MoveIn {
            direction: Direction::Left,
        };

        let halfway = composite(style, 0.5, width, height, &outgoing, &incoming);
        let pixels: Vec<&[u8]> = halfway.chunks_exact(4).collect();
        assert_eq!(pixels[0], [0, 0, 255, 255]);
        assert_eq!(pixels[1], [0, 0, 255, 255]);
        assert_eq!(pixels[2], [255, 0, 0, 255]);
        assert_eq!(pixels[3], [255, 0, 0, 255]);
    }

    #[test]
    fn move_in_keeps_the_outgoing_visible_underneath() {
        let width = 4;
        let height = 1;
        let out: Vec<u8> = (0..width).flat_map(|_| [255, 0, 0, 255]).collect();
        let inc: Vec<u8> = (0..width).flat_map(|_| [0, 255, 0, 255]).collect();
        let style = TransitionStyle::MoveIn {
            direction: Direction::Left,
        };

        // Half-way: the incoming (opaque) covers the leading columns, the
        // stationary outgoing shows through the trailing ones — never transparent.
        let mid = composite(style, 0.5, width, height, &out, &inc);
        assert!(
            mid.chunks(4).all(|px| px[3] == 255),
            "move-in over an opaque frame is never transparent: {mid:?}"
        );
        // Fully in.
        assert_eq!(composite(style, 1.0, width, height, &out, &inc), inc);
    }

    #[test]
    fn slide_out_endpoints_swap_and_the_outgoing_moves_on_top() {
        // 4×1 rasters: outgoing red, incoming blue. SlideOut{Left} = the RED
        // frame exits toward the left edge, revealing blue beneath.
        let width = 4;
        let out: Vec<u8> = (0..width).flat_map(|_| [255, 0, 0, 255]).collect();
        let inc: Vec<u8> = (0..width).flat_map(|_| [0, 0, 255, 255]).collect();
        let style = TransitionStyle::SlideOut {
            direction: Direction::Left,
        };

        // Endpoints: fully-covering outgoing at e=0, incoming at e=1.
        assert_eq!(composite(style, 0.0, width, 1, &out, &inc), out);
        assert_eq!(composite(style, 1.0, width, 1, &out, &inc), inc);

        // Halfway the outgoing has slid 2px left: it still covers the leading
        // columns while the trailing ones show the revealed incoming.
        let mid = composite(style, 0.5, width, 1, &out, &inc);
        let pixels: Vec<&[u8]> = mid.chunks_exact(4).collect();
        assert_eq!(pixels[0], [255, 0, 0, 255], "outgoing still covers left");
        assert_eq!(pixels[1], [255, 0, 0, 255]);
        assert_eq!(pixels[2], [0, 0, 255, 255], "incoming revealed right");
        assert_eq!(pixels[3], [0, 0, 255, 255]);
    }

    #[test]
    fn move_out_reveals_a_stationary_incoming() {
        let width = 4;
        let out: Vec<u8> = (0..width).flat_map(|_| [255, 0, 0, 255]).collect();
        let inc: Vec<u8> = (0..width).flat_map(|_| [0, 255, 0, 255]).collect();
        let style = TransitionStyle::MoveOut {
            direction: Direction::Right,
        };

        // Halfway the outgoing has moved 2px right; the incoming beneath is
        // pinned at zero offset — never transparent, exits over it.
        let mid = composite(style, 0.5, width, 1, &out, &inc);
        let pixels: Vec<&[u8]> = mid.chunks_exact(4).collect();
        assert_eq!(pixels[0], [0, 255, 0, 255], "incoming revealed left");
        assert_eq!(pixels[1], [0, 255, 0, 255]);
        assert_eq!(pixels[2], [255, 0, 0, 255], "outgoing exiting right");
        assert_eq!(pixels[3], [255, 0, 0, 255]);
        assert!(mid.chunks(4).all(|px| px[3] == 255));
        assert_eq!(composite(style, 1.0, width, 1, &out, &inc), inc);
    }

    #[test]
    fn scroll_animate_on_a_navigation_degrades_to_a_crossfade() {
        // ScrollAnimate's real effect is the eased scroll offset the session
        // animates; a document that authored it on a NAVIGATION still blends
        // instead of hard-cutting.
        let out = vec![10, 10, 10, 255];
        let inc = vec![200, 200, 200, 255];
        assert_eq!(
            composite(TransitionStyle::ScrollAnimate, 0.5, 1, 1, &out, &inc),
            vec![105, 105, 105, 255]
        );
    }

    // PR-7: the spring sampler feeding `ease`. Its physical regimes are pinned
    // in fanta-doc next to the sampler itself; these cover the properties the
    // present runtime relies on.
    #[test]
    fn spring_easing_starts_at_zero_and_settles_at_one() {
        for (mass, stiffness, damping) in [
            (1.0, 100.0, 15.0), // gentle
            (1.0, 300.0, 20.0), // quick
            (1.0, 600.0, 15.0), // bouncy
            (1.0, 80.0, 20.0),  // slow
        ] {
            let spring = Easing::Spring {
                mass,
                stiffness,
                damping,
            };
            assert_eq!(ease(spring, 0.0), 0.0, "spring({stiffness}) at 0");
            assert_eq!(ease(spring, 1.0), 1.0, "spring({stiffness}) at 1");
            assert!(
                (ease(spring, 0.999) - 1.0).abs() < 1e-3,
                "spring({stiffness}) settles within 1e-3 by the end"
            );
        }
    }

    #[test]
    fn bouncy_spring_overshoots_and_gentle_stays_tame() {
        let bouncy = Easing::Spring {
            mass: 1.0,
            stiffness: 600.0,
            damping: 15.0,
        };
        let gentle = Easing::Spring {
            mass: 1.0,
            stiffness: 100.0,
            damping: 15.0,
        };
        let peak = |easing: Easing| {
            (1..200)
                .map(|i| ease(easing, f64::from(i) / 200.0))
                .fold(f64::MIN, f64::max)
        };
        // The overshoot IS the animation: bouncy must ring past 1.0 (this is
        // what the directional offsets turn into a visible position overshoot)
        // while gentle barely exceeds it.
        assert!(peak(bouncy) > 1.0, "bouncy peak {}", peak(bouncy));
        assert!(peak(gentle) <= 1.05, "gentle peak {}", peak(gentle));
    }

    #[test]
    fn mismatched_buffers_fall_back_to_incoming() {
        let out = vec![0u8; 4];
        let inc = vec![7u8; 16];
        assert_eq!(
            composite(TransitionStyle::Dissolve, 0.5, 2, 2, &out, &inc),
            inc
        );
    }

    #[test]
    fn straight_alpha_over_composites_transparency() {
        // Opaque top wins outright.
        assert_eq!(over([1, 2, 3, 255], [9, 9, 9, 255]), [1, 2, 3, 255]);
        // Fully-transparent top reveals the bottom.
        assert_eq!(over([1, 2, 3, 0], [9, 9, 9, 255]), [9, 9, 9, 255]);
    }

    #[test]
    fn overlay_dissolve_fades_over_a_stable_backdrop() {
        let backdrop = vec![255, 0, 0, 255];
        let overlay = vec![0, 255, 0, 255];
        assert_eq!(
            composite_overlay(TransitionStyle::Dissolve, 0.0, 1, 1, &backdrop, &overlay,),
            backdrop
        );
        let halfway = composite_overlay(TransitionStyle::Dissolve, 0.5, 1, 1, &backdrop, &overlay);
        assert_eq!(halfway, vec![127, 128, 0, 255]);
        assert_eq!(
            composite_overlay(TransitionStyle::Dissolve, 1.0, 1, 1, &backdrop, &overlay,),
            overlay
        );
    }

    #[test]
    fn directional_overlay_moves_only_the_overlay_layer() {
        let width = 4;
        let backdrop: Vec<u8> = (0..width).flat_map(|_| [255, 0, 0, 255]).collect();
        let overlay: Vec<u8> = (0..width).flat_map(|_| [0, 255, 0, 255]).collect();
        let halfway = composite_overlay(
            TransitionStyle::SlideIn {
                direction: Direction::Left,
            },
            0.5,
            width,
            1,
            &backdrop,
            &overlay,
        );
        let pixels: Vec<&[u8]> = halfway.chunks_exact(4).collect();
        assert_eq!(pixels[0], [0, 255, 0, 255]);
        assert_eq!(pixels[1], [0, 255, 0, 255]);
        assert_eq!(pixels[2], [255, 0, 0, 255]);
        assert_eq!(pixels[3], [255, 0, 0, 255]);
    }
}

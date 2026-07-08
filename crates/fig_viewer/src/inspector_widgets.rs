//! Shared inspector widgets: the drag-to-scrub math and the vector-style
//! glyphs (align / distribute / text-align icons) the properties panel draws
//! with plain divs so no icon assets are needed.

use gpui::prelude::*;
use gpui::{Context, Div, Empty, Hsla, Render, Window, div, px};

/// Marker payload for the panel's scrub gestures (numeric field labels and the
/// opacity slider track). Dragging it renders nothing — the gesture is a value
/// scrub, not a drag-and-drop.
#[derive(Clone)]
pub(crate) struct PanelDrag;

impl Render for PanelDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Map a horizontal scrub delta to a value: 1 unit per pixel, ×10 with Shift,
/// ×0.1 with Alt (the Figma/Fanta scrub steps). The result snaps to the step
/// grid so scrubbing lands on tidy values instead of accumulating float dust.
pub(crate) fn scrub_value(start: f64, dx: f64, shift: bool, alt: bool) -> f64 {
    let step = if shift {
        10.0
    } else if alt {
        0.1
    } else {
        1.0
    };
    let raw = start + dx * step;
    (raw / step).round() * step
}

/// Map a cursor x within a track (its left edge and width in window px) to a
/// value in `min..=max`.
pub(crate) fn track_value(x: f64, track_left: f64, track_width: f64, min: f64, max: f64) -> f64 {
    let fraction = ((x - track_left) / track_width.max(1.0)).clamp(0.0, 1.0);
    min + fraction * (max - min)
}

/// The selection-alignment button glyphs (6 edge aligns + 2 distributes),
/// mirroring the original inspector's vector icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AlignGlyph {
    Left,
    CenterH,
    Right,
    Top,
    CenterV,
    Bottom,
    DistributeH,
    DistributeV,
}

/// The typography align-strip glyphs: three horizontal-align cells followed by
/// three vertical-align cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextAlignGlyph {
    Left,
    CenterH,
    Right,
    Top,
    CenterV,
    Bottom,
}

fn bar(x: f32, y: f32, w: f32, h: f32, color: Hsla) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .rounded(px(0.5))
        .bg(color)
}

fn glyph_canvas() -> Div {
    div().size_4().relative().flex_none()
}

/// One selection-align / distribute icon drawn from bars on a 16×16 canvas.
pub(crate) fn align_glyph(kind: AlignGlyph, color: Hsla) -> Div {
    let canvas = glyph_canvas();
    match kind {
        AlignGlyph::Left => canvas
            .child(bar(3.0, 2.5, 1.5, 11.0, color))
            .child(bar(5.5, 4.5, 7.5, 2.5, color))
            .child(bar(5.5, 9.0, 4.5, 2.5, color)),
        AlignGlyph::CenterH => canvas
            .child(bar(7.25, 2.5, 1.5, 11.0, color))
            .child(bar(3.0, 4.5, 10.0, 2.5, color))
            .child(bar(5.0, 9.0, 6.0, 2.5, color)),
        AlignGlyph::Right => canvas
            .child(bar(11.5, 2.5, 1.5, 11.0, color))
            .child(bar(3.0, 4.5, 7.5, 2.5, color))
            .child(bar(6.0, 9.0, 4.5, 2.5, color)),
        AlignGlyph::Top => canvas
            .child(bar(2.5, 3.0, 11.0, 1.5, color))
            .child(bar(4.5, 5.5, 2.5, 7.5, color))
            .child(bar(9.0, 5.5, 2.5, 4.5, color)),
        AlignGlyph::CenterV => canvas
            .child(bar(2.5, 7.25, 11.0, 1.5, color))
            .child(bar(4.5, 3.0, 2.5, 10.0, color))
            .child(bar(9.0, 5.0, 2.5, 6.0, color)),
        AlignGlyph::Bottom => canvas
            .child(bar(2.5, 11.5, 11.0, 1.5, color))
            .child(bar(4.5, 3.0, 2.5, 7.5, color))
            .child(bar(9.0, 6.0, 2.5, 4.5, color)),
        AlignGlyph::DistributeH => canvas
            .child(bar(3.0, 3.0, 1.5, 10.0, color))
            .child(bar(11.5, 3.0, 1.5, 10.0, color))
            .child(bar(6.75, 5.0, 2.5, 6.0, color)),
        AlignGlyph::DistributeV => canvas
            .child(bar(3.0, 3.0, 10.0, 1.5, color))
            .child(bar(3.0, 11.5, 10.0, 1.5, color))
            .child(bar(5.0, 6.75, 6.0, 2.5, color)),
    }
}

/// One typography align icon: three text lines whose alignment (or vertical
/// pinning) shows the option.
pub(crate) fn text_align_glyph(kind: TextAlignGlyph, color: Hsla) -> Div {
    let canvas = glyph_canvas();
    match kind {
        TextAlignGlyph::Left => canvas
            .child(bar(3.0, 3.5, 10.0, 1.5, color))
            .child(bar(3.0, 7.0, 6.5, 1.5, color))
            .child(bar(3.0, 10.5, 8.5, 1.5, color)),
        TextAlignGlyph::CenterH => canvas
            .child(bar(3.0, 3.5, 10.0, 1.5, color))
            .child(bar(4.75, 7.0, 6.5, 1.5, color))
            .child(bar(3.75, 10.5, 8.5, 1.5, color)),
        TextAlignGlyph::Right => canvas
            .child(bar(3.0, 3.5, 10.0, 1.5, color))
            .child(bar(6.5, 7.0, 6.5, 1.5, color))
            .child(bar(4.5, 10.5, 8.5, 1.5, color)),
        TextAlignGlyph::Top => canvas
            .child(bar(3.0, 2.5, 10.0, 1.5, color))
            .child(bar(4.5, 5.5, 7.0, 1.5, color)),
        TextAlignGlyph::CenterV => canvas
            .child(bar(3.0, 5.5, 10.0, 1.5, color))
            .child(bar(4.5, 8.5, 7.0, 1.5, color)),
        TextAlignGlyph::Bottom => canvas
            .child(bar(4.5, 9.0, 7.0, 1.5, color))
            .child(bar(3.0, 12.0, 10.0, 1.5, color)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_moves_one_unit_per_pixel() {
        assert_eq!(scrub_value(10.0, 5.0, false, false), 15.0);
        assert_eq!(scrub_value(10.0, -12.0, false, false), -2.0);
    }

    #[test]
    fn scrub_shift_multiplies_by_ten() {
        assert_eq!(scrub_value(10.0, 5.0, true, false), 60.0);
        assert_eq!(scrub_value(100.0, -3.0, true, false), 70.0);
    }

    #[test]
    fn scrub_alt_steps_by_tenths() {
        let value = scrub_value(10.0, 5.0, false, true);
        assert!((value - 10.5).abs() < 1e-9);
        let value = scrub_value(0.0, -3.0, false, true);
        assert!((value + 0.3).abs() < 1e-9);
    }

    #[test]
    fn scrub_snaps_fractional_starts_to_the_step_grid() {
        assert_eq!(scrub_value(142.34, 1.0, false, false), 143.0);
        assert_eq!(scrub_value(142.34, 1.0, true, false), 150.0);
    }

    #[test]
    fn track_maps_and_clamps_the_cursor() {
        assert_eq!(track_value(150.0, 100.0, 100.0, 0.0, 100.0), 50.0);
        assert_eq!(track_value(90.0, 100.0, 100.0, 0.0, 100.0), 0.0);
        assert_eq!(track_value(260.0, 100.0, 100.0, 0.0, 100.0), 100.0);
        assert_eq!(track_value(125.0, 100.0, 100.0, 40.0, 80.0), 50.0);
    }
}

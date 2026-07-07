//! Convert [`fanta_doc::Color`] to Skia colors.

use fanta_doc::Color;
use skia_safe::{Color as SkColor, Color4f};

/// Convert to Skia's packed ARGB representation. Cheap; no allocation.
pub fn to_sk_color(c: Color) -> SkColor {
    SkColor::from_argb(c.a, c.r, c.g, c.b)
}

/// Convert to a 4-float color. Slightly more accurate for gradient stops and
/// any path that runs through Skia's modern color management.
pub fn to_sk_color4f(c: Color) -> Color4f {
    Color4f::new(
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a as f32 / 255.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_red_round_trips_to_skia() {
        let c = Color::rgb(255, 0, 0);
        let sk = to_sk_color(c);
        assert_eq!(sk.r(), 255);
        assert_eq!(sk.g(), 0);
        assert_eq!(sk.b(), 0);
        assert_eq!(sk.a(), 255);
    }

    #[test]
    fn alpha_passes_through_in_color4f() {
        let c = Color::rgba(128, 64, 32, 200);
        let sk4 = to_sk_color4f(c);
        assert!((sk4.a - 200.0 / 255.0).abs() < 1e-6);
    }
}

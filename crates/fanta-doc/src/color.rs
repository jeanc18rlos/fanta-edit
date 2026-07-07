//! Colors and gradients.
//!
//! Colors are stored as 4-channel sRGB-with-alpha at `u8` precision in the doc
//! (8 bits per channel matches what designers see in pickers and what's
//! lossless across PNG export). Render-time conversion to linear happens in
//! `fanta-render`; the doc stays the source-of-truth-as-typed.
//!
//! P3 / HDR support is deferred — `Color` will gain a `space: ColorSpace`
//! field when phase 4 perf work lands, and existing docs upgrade by defaulting
//! `space: ColorSpace::Srgb`.

use serde::{Deserialize, Serialize};

/// sRGB color with straight (un-premultiplied) alpha.
///
/// Channels are 0–255. Matches the format every color picker shows, every CSS
/// declaration uses, and what Skia takes when you call `Color::ARGB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Pack as 32-bit ARGB, big-endian-ish — matches Skia's `SkColor`.
    pub const fn to_argb_u32(self) -> u32 {
        (self.a as u32) << 24 | (self.r as u32) << 16 | (self.g as u32) << 8 | (self.b as u32)
    }

    /// Parse "#RRGGBB" or "#RRGGBBAA" (case-insensitive). Returns `None` on
    /// malformed input — the AI tool layer should validate before calling.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        let parse = |i: usize| -> Option<u8> { u8::from_str_radix(s.get(i..i + 2)?, 16).ok() };
        match s.len() {
            6 => Some(Self::rgb(parse(0)?, parse(2)?, parse(4)?)),
            8 => Some(Self::rgba(parse(0)?, parse(2)?, parse(4)?, parse(6)?)),
            _ => None,
        }
    }

    /// Format as "#RRGGBB" when fully opaque, "#RRGGBBAA" otherwise. This is
    /// what the `.fant.json` projection emits — readable, diffable, AI-friendly.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

/// One stop within a [`Gradient`]. Position is normalized `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: Color,
}

/// Gradient paint, stored normalized so it can be reused across nodes of any
/// size. Concrete pixel coords are computed at render time from the node's
/// local bounds. Mirrors Figma's four gradient kinds (linear / radial / angular
/// / diamond).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Gradient {
    /// Linear gradient from `start` to `end` in node-local 0–1 space.
    Linear {
        start: [f32; 2],
        end: [f32; 2],
        stops: Vec<GradientStop>,
    },
    /// Radial gradient centered at `center` with `radius` in node-local space.
    Radial {
        center: [f32; 2],
        radius: f32,
        stops: Vec<GradientStop>,
    },
    /// Angular (conic / sweep) gradient: stops are swept around `center` in
    /// node-local 0–1 space, starting from `start_angle` (radians, measured
    /// clockwise from the positive x-axis to match Figma/CSS `conic-gradient`).
    /// Renders via Skia's `sweep_gradient`. Additive: a new variant — old docs
    /// never contain it, so they round-trip byte-identical.
    Angular {
        center: [f32; 2],
        /// Sweep start angle in radians. `0.0` points along +x.
        start_angle: f32,
        stops: Vec<GradientStop>,
    },
    /// Diamond gradient: like a radial gradient but the iso-distance contours are
    /// axis-aligned diamonds (an L1 / Manhattan-distance "radial"), centered at
    /// `center` with half-extent `radius` in node-local 0–1 space. Figma's
    /// `GRADIENT_DIAMOND`. Approximated at render time with a rotated two-point
    /// construction; falls back gracefully to a radial-like spread.
    Diamond {
        center: [f32; 2],
        radius: f32,
        stops: Vec<GradientStop>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_opaque() {
        let c = Color::from_hex("#3FA9F5").unwrap();
        assert_eq!(c, Color::rgb(0x3F, 0xA9, 0xF5));
        assert_eq!(c.to_hex(), "#3FA9F5");
    }

    #[test]
    fn hex_round_trip_with_alpha() {
        let c = Color::from_hex("#3FA9F580").unwrap();
        assert_eq!(c, Color::rgba(0x3F, 0xA9, 0xF5, 0x80));
        assert_eq!(c.to_hex(), "#3FA9F580");
    }

    #[test]
    fn argb_pack_matches_expected_layout() {
        let c = Color::rgba(0x11, 0x22, 0x33, 0x44);
        assert_eq!(c.to_argb_u32(), 0x4411_2233);
    }

    #[test]
    fn invalid_hex_returns_none() {
        assert!(Color::from_hex("3FA9F5").is_none()); // no '#'
        assert!(Color::from_hex("#XYZ").is_none()); // not hex
        assert!(Color::from_hex("#FF").is_none()); // wrong length
    }

    #[test]
    fn angular_and_diamond_gradients_round_trip() {
        let stops = vec![
            GradientStop {
                position: 0.0,
                color: Color::WHITE,
            },
            GradientStop {
                position: 1.0,
                color: Color::BLACK,
            },
        ];
        let angular = Gradient::Angular {
            center: [0.5, 0.5],
            start_angle: 1.25,
            stops: stops.clone(),
        };
        let ja = serde_json::to_value(&angular).unwrap();
        assert_eq!(ja["kind"], "angular");
        let back: Gradient = serde_json::from_value(ja).unwrap();
        assert_eq!(back, angular);

        let diamond = Gradient::Diamond {
            center: [0.5, 0.5],
            radius: 0.5,
            stops,
        };
        let jd = serde_json::to_value(&diamond).unwrap();
        assert_eq!(jd["kind"], "diamond");
        let back_d: Gradient = serde_json::from_value(jd).unwrap();
        assert_eq!(back_d, diamond);
    }
}

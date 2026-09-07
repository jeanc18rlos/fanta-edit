//! Character-level text styling.
//!
//! [`TextStyle`] is the unit of styled-run formatting in this crate. It is
//! deliberately *not* `fanta_doc`'s node-level visual style: a paragraph mixes
//! many character styles within one node (bold word, colored span, larger
//! heading), so the text engine needs a per-run style type the doc layer does
//! not have. Keeping it here — rather than in `fanta-doc` — is what lets the
//! tool layer and a future `TextNode` both consume the engine without the doc
//! crate growing a text dependency it does not yet want.
//!
//! Reuses `fanta_doc::Color` on purpose: text color must round-trip through the
//! same picker / hex / `.fant.json` path as every other color in the document,
//! so a second color type here would be a correctness hazard at the boundary.

use fanta_doc::{Color, FontVariation};
use serde::{Deserialize, Serialize};

/// Named font weights mapped to OpenType numeric weight values.
///
/// Exists so call sites can say `FontWeight::Bold` instead of memorizing that
/// "bold" is the magic number 700. The numeric form is what fonts and Skia
/// actually consume; this is just an ergonomic, self-documenting front door.
/// Weights are stored numerically on [`TextStyle`] so intermediate values
/// (e.g. a 500 "medium") survive even though only the two common names are
/// enumerated here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    /// Regular text. OpenType weight 400.
    Normal,
    /// Bold text. OpenType weight 700.
    Bold,
}

impl FontWeight {
    /// The OpenType numeric weight this name corresponds to. This is the value
    /// that gets stored on [`TextStyle::weight`] and handed to the shaper.
    pub const fn to_u16(self) -> u16 {
        match self {
            FontWeight::Normal => 400,
            FontWeight::Bold => 700,
        }
    }
}

impl From<FontWeight> for u16 {
    fn from(w: FontWeight) -> Self {
        w.to_u16()
    }
}

/// Formatting applied to a contiguous run of characters.
///
/// Every field is a value type so a `TextStyle` is cheap to clone and compare —
/// the buffer compares adjacent runs for equality constantly (to merge them),
/// and `PartialEq` derived here is what makes that merge correct. `f64` is used
/// for all metrics to match `fanta-doc`'s `f64`-doc / `f32`-projection coordinate
/// decision (ARCHITECTURE.md §13): the engine reasons in logical units and only
/// narrows to `f32` when it hands geometry to Skia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// Primary font family name (e.g. "Helvetica"). The layout engine resolves
    /// this against the system font manager with fallback, so an unavailable
    /// family degrades gracefully rather than failing.
    pub font_family: String,
    /// Em size in logical pixels.
    pub size_px: f64,
    /// OpenType numeric weight (100–900). 400 = regular, 700 = bold. Stored
    /// numerically rather than as [`FontWeight`] so non-named weights survive.
    pub weight: u16,
    /// Whether the run is italic / oblique.
    pub italic: bool,
    /// Whether the run paints an underline decoration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub underline: bool,
    /// Whether the run paints a line-through decoration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub strikethrough: bool,
    /// Fill color of the glyphs, in the same sRGB space as the rest of the doc.
    pub color: Color,
    /// Extra space inserted between characters, in logical pixels. Can be
    /// negative to tighten. Applied on top of the font's native advances.
    pub letter_spacing: f64,
    /// Line height as a multiple of `size_px` (1.0 = single spacing). A
    /// multiplier rather than an absolute value so it scales correctly when the
    /// font size changes — the standard CSS `line-height` unitless behavior.
    pub line_height: f64,
    /// Metric-relative line height: `Some(p)` means the line height is `p`
    /// percent of the FONT'S INTRINSIC line height (ascent + descent + line
    /// gap from the resolved face's metrics), not of `size_px`. Figma's "auto"
    /// line height is exactly `Some(100.0)`. When present this takes
    /// precedence over the scalar [`line_height`](Self::line_height), which
    /// then only serves as a fallback for faces whose metrics cannot be
    /// resolved. `None` (the default) keeps the scalar behavior.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height_auto_percent: Option<f64>,
    /// Variable-font axis settings applied as the face's variation coordinates
    /// (e.g. `wght=350`, `wdth=75`). Empty ⇒ the face's default instance.
    /// Field-identical to [`fanta_doc::TextStyle::font_variations`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_variations: Vec<FontVariation>,
}

impl TextStyle {
    /// The default font family. Inter is what Figma uses for its UI and the
    /// canonical modern UI typeface; we ship it bundled (see
    /// `font_resolver::bundled`) so it renders identically on every platform
    /// instead of resolving to a per-OS system face. Picked over "Helvetica"
    /// (the old default, which fell through to whatever the OS happened to have)
    /// precisely so generated designs look like Figma out of the box.
    pub const DEFAULT_FAMILY: &'static str = "Inter";

    /// Construct a style with the given family and size, leaving every other
    /// field at its [`Default`] value. The common "I just want 18px Helvetica"
    /// path without spelling out all seven fields.
    pub fn new(font_family: impl Into<String>, size_px: f64) -> Self {
        Self {
            font_family: font_family.into(),
            size_px,
            ..Self::default()
        }
    }

    /// Set the weight from a named [`FontWeight`], returning `self` for
    /// builder-style chaining. Why a builder: styles are usually assembled
    /// inline at a tool call site, and chaining reads better than five
    /// successive field assignments.
    #[must_use]
    pub fn with_weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight.to_u16();
        self
    }

    /// Set italic, returning `self` for chaining.
    #[must_use]
    pub fn with_italic(mut self, italic: bool) -> Self {
        self.italic = italic;
        self
    }

    /// Set underline, returning `self` for chaining.
    #[must_use]
    pub fn with_underline(mut self, underline: bool) -> Self {
        self.underline = underline;
        self
    }

    /// Set line-through, returning `self` for chaining.
    #[must_use]
    pub fn with_strikethrough(mut self, strikethrough: bool) -> Self {
        self.strikethrough = strikethrough;
        self
    }

    /// Set the fill color, returning `self` for chaining.
    #[must_use]
    pub fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    /// Whether this style's weight is at or above the bold threshold (600).
    /// Used by the layout layer to decide the Skia font slant/weight; exposed
    /// publicly because the properties panel will want the same predicate.
    pub fn is_bold(&self) -> bool {
        self.weight >= 600
    }
}

impl Default for TextStyle {
    /// 16px regular black Helvetica, single line spacing, no extra tracking.
    /// These match the most common "body text" defaults a designer expects when
    /// they first click the text tool, so a freshly-placed text node looks
    /// reasonable with zero configuration.
    fn default() -> Self {
        Self {
            font_family: Self::DEFAULT_FAMILY.to_string(),
            size_px: 16.0,
            weight: FontWeight::Normal.to_u16(),
            italic: false,
            underline: false,
            strikethrough: false,
            color: Color::BLACK,
            letter_spacing: 0.0,
            line_height: 1.2,
            line_height_auto_percent: None,
            font_variations: Vec::new(),
        }
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_weight_maps_to_opentype_numbers() {
        assert_eq!(FontWeight::Normal.to_u16(), 400);
        assert_eq!(FontWeight::Bold.to_u16(), 700);
        assert_eq!(u16::from(FontWeight::Bold), 700);
    }

    #[test]
    fn default_style_is_body_text() {
        let s = TextStyle::default();
        assert_eq!(s.font_family, "Inter");
        assert_eq!(s.size_px, 16.0);
        assert_eq!(s.weight, 400);
        assert!(!s.italic);
        assert!(!s.underline);
        assert!(!s.strikethrough);
        assert_eq!(s.color, Color::BLACK);
        assert_eq!(s.letter_spacing, 0.0);
        assert_eq!(s.line_height, 1.2);
        assert_eq!(s.line_height_auto_percent, None);
        assert!(!s.is_bold());
    }

    #[test]
    fn builder_chaining_sets_fields() {
        let s = TextStyle::new("Inter", 24.0)
            .with_weight(FontWeight::Bold)
            .with_italic(true)
            .with_underline(true)
            .with_strikethrough(true)
            .with_color(Color::WHITE);
        assert_eq!(s.font_family, "Inter");
        assert_eq!(s.size_px, 24.0);
        assert_eq!(s.weight, 700);
        assert!(s.italic);
        assert!(s.underline);
        assert!(s.strikethrough);
        assert_eq!(s.color, Color::WHITE);
        assert!(s.is_bold());
    }

    #[test]
    fn style_round_trips_through_json() {
        let s = TextStyle::new("Helvetica", 18.0).with_weight(FontWeight::Bold);
        let json = serde_json::to_string(&s).unwrap();
        let back: TextStyle = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}

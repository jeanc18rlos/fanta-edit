//! Horizontal line alignment — a Skia-free mirror of `textlayout::TextAlign`.

use skia_safe::textlayout::TextAlign as SkTextAlign;

/// Horizontal alignment of laid-out lines within the wrap width.
///
/// A Skia-free mirror of `skia_safe::textlayout::TextAlign`, kept here so the
/// crate's public layout API has no Skia type in its signature (the same
/// renderer-agnostic discipline the rest of the crate follows — see the module
/// docs). It is field-compatible with `fanta_doc::TextAlign`, so a renderer that
/// owns a doc-level `TextAlign` converts with a one-line `match`. Defaults to
/// [`Align::Left`], matching both Skia's and the doc model's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// Lines flush to the left edge of the box.
    #[default]
    Left,
    /// Lines centered within the box.
    Center,
    /// Lines flush to the right edge of the box.
    Right,
    /// Lines stretched to fill the box width (last line stays left-aligned).
    Justify,
}

impl Align {
    /// The Skia paragraph alignment this corresponds to.
    pub(super) fn to_sk(self) -> SkTextAlign {
        match self {
            Align::Left => SkTextAlign::Left,
            Align::Center => SkTextAlign::Center,
            Align::Right => SkTextAlign::Right,
            Align::Justify => SkTextAlign::Justify,
        }
    }
}

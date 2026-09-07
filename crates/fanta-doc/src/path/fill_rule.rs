//! Path fill rule.

use serde::{Deserialize, Serialize};

/// How a path's interior is determined when contours overlap or self-intersect.
///
/// Mirrors SVG `fill-rule` / Figma `windingRule` / Skia `SkPathFillType`. The
/// doc only stores the enum; `fanta-render` maps it to `SkPathFillType` and
/// `fanta-export` to the `fill-rule` SVG attribute — a single owner here so the
/// two consumers can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillRule {
    /// Standard non-zero winding rule. The overwhelmingly common case.
    #[default]
    NonZero,
    /// Even-odd rule (alternating fill). Needed for donut / star geometry.
    EvenOdd,
}

impl FillRule {
    /// Whether this is the default ([`FillRule::NonZero`]). Used by
    /// `#[serde(skip_serializing_if)]` so unchanged paths stay byte-identical
    /// to the pre-`fill_rule` era (the field is simply absent).
    pub fn is_default(&self) -> bool {
        matches!(self, Self::NonZero)
    }
}

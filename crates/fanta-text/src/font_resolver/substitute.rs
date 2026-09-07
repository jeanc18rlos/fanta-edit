//! Proprietary→open substitution: which open Source face stands in for a
//! family we can't legally obtain, and the metric-adjust ratio that keeps the
//! substitute's advance widths close to the original's.

use super::generic::GenericFamily;

/// Families we know are proprietary (or unbundled) and cannot obtain from Google
/// Fonts, mapped to their closest open substitute by generic class (step 4).
/// Matching is case-insensitive substring so variants like "Adobe Clean Han"
/// still hit.
///
/// Two families qualify and route here:
///
/// - **Adobe Clean** (and its serif/mono relatives): Figma's own UI font and the
///   Spectrum docs' display font. Proprietary; Adobe ships open counterparts
///   (Source Sans 3 / Source Serif 4 / Source Code Pro) that are metrically far
///   closer than Inter.
/// - **Source Sans Pro / Source Serif Pro / Source Code Pro**: the *previous*
///   names for the Source families. A `.fig` authored against "Source Sans Pro"
///   names a face we don't bundle under that exact name (we bundle the renamed
///   "Source Sans 3"), so it would otherwise fall through to the generic system
///   chain (Helvetica/Arial) and shape with the wrong metrics. Routing it to the
///   bundled Source successor keeps it on the right face.
///
/// Other unknown families fall through to the generic class chain instead, which
/// is the right behavior — we don't want to guess a substitute for an arbitrary
/// name.
pub(super) fn proprietary_substitute(requested: &str) -> Option<&'static str> {
    let lower = requested.to_ascii_lowercase();
    // "Source Sans Pro" / "Source Serif Pro" → the bundled Source successor, by
    // class. (Bare "Source Code Pro" is already a bundled canonical name, so it
    // resolves directly and never reaches here.)
    if lower.contains("source sans pro") || lower.contains("source serif pro") {
        return Some(GenericFamily::classify(requested).source_substitute());
    }
    if !lower.contains("adobe clean") {
        return None;
    }
    // Within Adobe Clean, route by sub-family. "Adobe Clean Han" is CJK; we have
    // no open CJK substitute bundled, so treat it as sans (Source Sans covers
    // Latin; glyph-level fallback handles CJK via the system manager).
    Some(GenericFamily::classify(requested).source_substitute())
}

/// The horizontal **metric-adjust ratio** to apply when `requested` is rendered
/// through a metric-compatible *substitute* face (step 4), or `None` when the
/// requested family renders at its true metrics (installed exactly, bundled by
/// its own canonical name, or a Google-Fonts download).
///
/// ## Why this exists
///
/// Figma does *metric-compatible* font substitution: when a document's real font
/// is missing it renders an available face **scaled so its advance widths
/// approximate the original's**, so the text still fits the boxes the document
/// was laid out in (the same idea as CSS `size-adjust`). Without it, our open
/// substitute shapes at *its own* width, which differs from the proprietary
/// original — so fixed-width labels overflow and clip ("224 selectec"), and the
/// boxes a `.fig` baked at the original's metrics no longer hold.
///
/// The ratio is a **target advance-width / substitute advance-width** multiplier,
/// applied at layout time as proportional letter-spacing (a width-only adjust
/// that leaves the glyph cap-height untouched — the Skia paragraph API exposes
/// no per-run `scale_x`, and shrinking the font size would shrink the glyphs
/// vertically too, which we explicitly do not want). `> 1.0` widens, `< 1.0`
/// tightens.
///
/// ## Calibration
///
/// Empirically tuned against the Spectrum `.fig`: the substitute Source Sans 3
/// shapes the action-bar label "224 selected" a few percent **wider** than the
/// box Figma baked for it, so its trailing glyph clips against the fixed-width
/// "Item Counter" frame. A ratio just under 1.0 tightens the substitute back to
/// the box without visibly shrinking the type or pushing the onboarding body off
/// its two-line wrap. Returned per family so the serif substitute can carry its
/// own factor independently of the sans one.
pub(super) fn substitute_metric_ratio(requested: &str) -> Option<f32> {
    // Only the proprietary→open substitutions get an adjustment; an installed or
    // obtainable family renders at its true metrics (handled by the caller, which
    // only consults this for the substituted case).
    let lower = requested.to_ascii_lowercase();
    if lower.contains("adobe clean serif") || lower.contains("source serif pro") {
        // Serif substitution (Adobe Clean Serif / Source Serif Pro → Source
        // Serif 4). Check this before the generic "adobe clean" branch so serif
        // families do not accidentally inherit the sans metric ratio.
        Some(ADOBE_CLEAN_SERIF_METRIC_RATIO)
    } else if lower.contains("adobe clean") || lower.contains("source sans pro") {
        // Sans substitution (Adobe Clean / Source Sans Pro → Source Sans 3).
        Some(ADOBE_CLEAN_SANS_METRIC_RATIO)
    } else {
        None
    }
}

/// Width metric-adjust for the **sans** proprietary substitution (Adobe Clean /
/// Source Sans Pro → Source Sans 3). Calibrated empirically (see
/// [`substitute_metric_ratio`]): Source Sans 3 shapes the Spectrum action-bar
/// label "224 selected" ~5% wider than the box the `.fig` baked, clipping its
/// last glyph; `0.95` tightens the substitute back to fit while keeping the
/// onboarding body on two lines.
pub const ADOBE_CLEAN_SANS_METRIC_RATIO: f32 = 0.95;

/// Width metric-adjust for the **serif** proprietary substitution (Source Serif
/// Pro / Adobe Clean Serif → Source Serif 4). Source Serif 4 tracks close to the
/// originals fairly closely, but still too wide for Spectrum's fixed component
/// headers; this keeps one-line headings such as Workflow Icons and
/// Scroll-Zoom Bar inside the boxes Figma authored with Source Serif Pro.
pub const ADOBE_CLEAN_SERIF_METRIC_RATIO: f32 = 0.925;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_resolver::{SOURCE_CODE_FAMILY, SOURCE_SANS_FAMILY, SOURCE_SERIF_FAMILY};

    #[test]
    fn adobe_clean_maps_to_source_by_class() {
        assert_eq!(
            proprietary_substitute("Adobe Clean"),
            Some(SOURCE_SANS_FAMILY)
        );
        assert_eq!(
            proprietary_substitute("Adobe Clean Serif"),
            Some(SOURCE_SERIF_FAMILY)
        );
        // A code-flavored Adobe Clean name routes to Source Code Pro.
        assert_eq!(
            proprietary_substitute("Adobe Clean Mono"),
            Some(SOURCE_CODE_FAMILY)
        );
        // Non-Adobe families don't get a hardcoded substitute.
        assert_eq!(proprietary_substitute("Roboto"), None);
        assert_eq!(proprietary_substitute("Helvetica"), None);
    }

    #[test]
    fn source_pro_renames_map_to_bundled_successor() {
        // "Source Sans Pro" / "Source Serif Pro" are the *previous* names for our
        // bundled Source families; route them to the successor we ship rather
        // than letting them fall through to a generic system sans.
        assert_eq!(
            proprietary_substitute("Source Sans Pro"),
            Some(SOURCE_SANS_FAMILY)
        );
        assert_eq!(
            proprietary_substitute("Source Serif Pro"),
            Some(SOURCE_SERIF_FAMILY)
        );
    }

    #[test]
    fn substitute_metric_ratio_only_for_proprietary_renamed_families() {
        // The proprietary/renamed substitutions carry a width adjust (< 1.0)…
        assert!(substitute_metric_ratio("Adobe Clean").unwrap() < 1.0);
        assert!(substitute_metric_ratio("Source Sans Pro").unwrap() < 1.0);
        assert!(substitute_metric_ratio("Adobe Clean Serif").unwrap() < 1.0);
        assert!(substitute_metric_ratio("Source Serif Pro").unwrap() < 1.0);
        assert_eq!(
            substitute_metric_ratio("Adobe Clean Serif"),
            Some(ADOBE_CLEAN_SERIF_METRIC_RATIO)
        );
        assert_eq!(
            substitute_metric_ratio("Adobe Clean"),
            Some(ADOBE_CLEAN_SANS_METRIC_RATIO)
        );
        // …while obtainable / bundled-by-own-name / arbitrary families do not.
        assert_eq!(substitute_metric_ratio("Source Sans 3"), None);
        assert_eq!(substitute_metric_ratio("Inter"), None);
        assert_eq!(substitute_metric_ratio("Helvetica"), None);
        assert_eq!(substitute_metric_ratio("Roboto"), None);
    }
}

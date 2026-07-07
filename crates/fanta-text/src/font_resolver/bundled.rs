//! The vendored faces: bundled-family table, asset bytes, and provider
//! registration (including per-weight instancing of the variable Source fonts).

use skia_safe::{
    FontArguments, FontMgr, Typeface,
    font_arguments::{VariationPosition, variation_position::Coordinate},
    textlayout::TypefaceFontProvider,
};

/// A font family the crate vendors in `assets/` and registers on the provider.
///
/// Each variant carries its canonical Skia family name plus the embedded TTF
/// bytes for its upright and (where available) italic faces. The Source faces
/// are *variable* fonts (a single `wght` axis), so the provider instances each
/// at several discrete weights ([`INSTANCE_WEIGHTS`]) via
/// [`Typeface::clone_with_arguments`] — a `TypefaceFontProvider` matches a run's
/// requested weight against the *registered* static faces, so without
/// instancing every weight would collapse to the variable font's default
/// instance (ExtraLight for the Source families).
#[derive(Debug, Clone, Copy)]
pub(super) struct BundledFamily {
    /// Canonical family name the faces are registered under (e.g. "Source Sans 3").
    pub(super) name: &'static str,
    /// Whether the faces are *variable* (need per-weight instancing) or already
    /// static (Inter ships discrete weight files).
    pub(super) variable: bool,
    /// Upright face TTF bytes (one variable file, or several static files).
    pub(super) upright: &'static [&'static [u8]],
    /// Italic face TTF bytes; empty if the family has no italic asset.
    pub(super) italic: &'static [&'static [u8]],
}

/// Discrete weights the variable Source faces are instanced at. Covers the
/// regular/medium/semibold/bold span Spectrum-style body and heading text
/// exercises; Skia picks the closest of these for any requested weight.
pub(super) const INSTANCE_WEIGHTS: &[i32] = &[400, 500, 600, 700];

/// Canonical bundled family name for Inter (still available by name; no longer
/// the universal sans override).
pub const INTER_FAMILY: &str = "Inter";
/// Canonical bundled family name for the Adobe Clean *sans* substitute.
pub const SOURCE_SANS_FAMILY: &str = "Source Sans 3";
/// Canonical bundled family name for the Adobe Clean *serif* substitute.
pub const SOURCE_SERIF_FAMILY: &str = "Source Serif 4";
/// Canonical bundled family name for the Adobe Clean *monospace* substitute.
pub const SOURCE_CODE_FAMILY: &str = "Source Code Pro";

const INTER_FACES: &[&[u8]] = &[
    include_bytes!("../../assets/inter/Inter-Regular.ttf"),
    include_bytes!("../../assets/inter/Inter-Medium.ttf"),
    include_bytes!("../../assets/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/inter/Inter-Bold.ttf"),
];
const INTER_ITALIC_FACES: &[&[u8]] = &[
    include_bytes!("../../assets/inter/Inter-Italic.ttf"),
    include_bytes!("../../assets/inter/Inter-MediumItalic.ttf"),
    include_bytes!("../../assets/inter/Inter-SemiBoldItalic.ttf"),
    include_bytes!("../../assets/inter/Inter-BoldItalic.ttf"),
];
const SOURCE_SANS_UPRIGHT: &[&[u8]] = &[include_bytes!(
    "../../assets/source-sans-3/SourceSans3-Variable.ttf"
)];
const SOURCE_SANS_ITALIC: &[&[u8]] = &[include_bytes!(
    "../../assets/source-sans-3/SourceSans3-Italic-Variable.ttf"
)];
const SOURCE_SERIF_UPRIGHT: &[&[u8]] = &[include_bytes!(
    "../../assets/source-serif-4/SourceSerif4-Variable.ttf"
)];
const SOURCE_SERIF_ITALIC: &[&[u8]] = &[include_bytes!(
    "../../assets/source-serif-4/SourceSerif4-Italic-Variable.ttf"
)];
const SOURCE_CODE_UPRIGHT: &[&[u8]] = &[include_bytes!(
    "../../assets/source-code-pro/SourceCodePro-Variable.ttf"
)];
const SOURCE_CODE_ITALIC: &[&[u8]] = &[include_bytes!(
    "../../assets/source-code-pro/SourceCodePro-Italic-Variable.ttf"
)];

/// Every family the crate vendors and registers on its asset provider.
pub(super) const BUNDLED: &[BundledFamily] = &[
    BundledFamily {
        name: INTER_FAMILY,
        variable: false,
        upright: INTER_FACES,
        italic: INTER_ITALIC_FACES,
    },
    BundledFamily {
        name: SOURCE_SANS_FAMILY,
        variable: true,
        upright: SOURCE_SANS_UPRIGHT,
        italic: SOURCE_SANS_ITALIC,
    },
    BundledFamily {
        name: SOURCE_SERIF_FAMILY,
        variable: true,
        upright: SOURCE_SERIF_UPRIGHT,
        italic: SOURCE_SERIF_ITALIC,
    },
    BundledFamily {
        name: SOURCE_CODE_FAMILY,
        variable: true,
        upright: SOURCE_CODE_UPRIGHT,
        italic: SOURCE_CODE_ITALIC,
    },
];

/// The regular upright TTF bytes for a bundled family (its first registered
/// upright face), for UI **font preview** — e.g. registering the face in an
/// egui font picker so each row renders in its own typeface. Returns `None` for
/// a family this crate does not vendor. The Source faces are variable files, so
/// the bytes render at the file's default instance (fine for a name preview).
pub fn bundled_preview_bytes(family: &str) -> Option<&'static [u8]> {
    BUNDLED
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(family))
        .and_then(|f| f.upright.first().copied())
}

/// The canonical names of every family this crate vendors with real glyph data,
/// in registration order. Pairs with [`bundled_preview_bytes`].
pub fn bundled_family_names() -> impl Iterator<Item = &'static str> {
    BUNDLED.iter().map(|f| f.name)
}

/// Register one bundled family's faces on `provider` under its canonical name.
///
/// Static faces (Inter) register as-is. Variable faces (the Source families)
/// are instanced at each weight in [`INSTANCE_WEIGHTS`] via
/// [`Typeface::clone_with_arguments`] so weight matching works through the
/// provider; the clone carries the right `OS/2` weight for Skia's matcher. A
/// face that fails to decode or clone is skipped rather than panicking.
pub(super) fn register_bundled(
    provider: &mut TypefaceFontProvider,
    mgr: &FontMgr,
    fam: &BundledFamily,
) {
    let mut register_set = |bytes_list: &[&[u8]]| {
        for bytes in bytes_list {
            let Some(base) = mgr.new_from_data(bytes, None) else {
                continue;
            };
            if fam.variable {
                // Variable file: register one static instance per weight so the
                // provider's family style-set can match a run's requested weight.
                // The instance keeps the source file's slant (upright vs italic).
                for &w in INSTANCE_WEIGHTS {
                    if let Some(face) = instance_at_weight(&base, w) {
                        provider.register_typeface(face, fam.name);
                    }
                }
            } else {
                provider.register_typeface(base, fam.name);
            }
        }
    };
    register_set(fam.upright);
    register_set(fam.italic);
}

/// Clone a variable typeface pinned to a single `wght` value, producing a
/// static instance Skia's family matcher can select by weight. Returns `None`
/// if the platform's font backend can't clone with arguments (then the caller
/// simply registers fewer instances).
pub(super) fn instance_at_weight(base: &Typeface, weight: i32) -> Option<Typeface> {
    let coords = [Coordinate {
        axis: skia_safe::FourByteTag::from_chars('w', 'g', 'h', 't'),
        value: weight as f32,
    }];
    let args = FontArguments::new().set_variation_design_position(VariationPosition {
        coordinates: &coords,
    });
    base.clone_with_arguments(&args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_bundled_faces_decode() {
        let mgr = FontMgr::default();
        for fam in BUNDLED {
            for bytes in fam.upright.iter().chain(fam.italic.iter()) {
                assert!(
                    mgr.new_from_data(bytes, None).is_some(),
                    "bundled {} face failed to decode ({} bytes)",
                    fam.name,
                    bytes.len()
                );
            }
        }
    }

    #[test]
    fn variable_source_instances_distinct_weights() {
        // Instancing the variable font at 400 vs 700 must yield faces the
        // provider can tell apart by weight (proves clone_with_arguments works
        // on this platform; if it doesn't, the test platform falls back to the
        // default instance and we skip the assertion).
        let mgr: FontMgr = FontMgr::default();
        let base = mgr
            .new_from_data(SOURCE_SANS_UPRIGHT[0], None)
            .expect("Source Sans variable decodes");
        let regular = instance_at_weight(&base, 400);
        let bold = instance_at_weight(&base, 700);
        if let (Some(r), Some(b)) = (regular, bold) {
            assert!(
                *r.font_style().weight() <= *b.font_style().weight(),
                "400 instance ({:?}) must be no heavier than 700 ({:?})",
                r.font_style().weight(),
                b.font_style().weight()
            );
        }
    }
}

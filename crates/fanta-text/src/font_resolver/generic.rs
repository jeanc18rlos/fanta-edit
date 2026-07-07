//! Generic typographic class — serif / sans / mono classification and the
//! per-class substitute + system fallback chains.

use super::bundled::{SOURCE_CODE_FAMILY, SOURCE_SANS_FAMILY, SOURCE_SERIF_FAMILY};

/// The broad typographic class a family belongs to, used for the generic
/// last-resort fallback (step 5) and to pick the right Source substitute for a
/// proprietary family (step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenericFamily {
    /// Proportional serif faces (Times, Georgia, Adobe Clean Serif, …).
    Serif,
    /// Fixed-width faces for code (Mono, Consolas, Courier, …).
    Monospace,
    /// Proportional sans-serif — the default when nothing else matches.
    Sans,
}

impl GenericFamily {
    /// Classify a font-family name into a generic typographic class.
    ///
    /// Case-insensitive, substring-based (real `.fig` names are messy: "Adobe
    /// Clean Serif", "SF Mono", "Roboto Mono"). **Monospace is tested first** so
    /// a code face is never mistaken for serif/sans; "sans serif" is guarded
    /// back to sans. Anything unrecognized is [`Sans`](GenericFamily::Sans).
    pub fn classify(family: &str) -> Self {
        let lower = family.to_ascii_lowercase();

        const MONO: [&str; 7] = [
            "mono",
            "code",
            "consolas",
            "courier",
            "menlo",
            "monaco",
            "typewriter",
        ];
        if MONO.iter().any(|m| lower.contains(m)) {
            return GenericFamily::Monospace;
        }

        const SERIF: [&str; 9] = [
            "serif",
            "times",
            "georgia",
            "garamond",
            "minion",
            "playfair",
            "merriweather",
            "spectral",
            "noto serif",
        ];
        if SERIF.iter().any(|s| lower.contains(s)) {
            if lower.contains("sans") {
                return GenericFamily::Sans;
            }
            return GenericFamily::Serif;
        }

        GenericFamily::Sans
    }

    /// The canonical bundled Source family that substitutes for a proprietary
    /// family of this class (step 4). All three are Adobe's own open fonts,
    /// metrically close to Adobe Clean.
    pub(super) fn source_substitute(self) -> &'static str {
        match self {
            GenericFamily::Serif => SOURCE_SERIF_FAMILY,
            GenericFamily::Monospace => SOURCE_CODE_FAMILY,
            GenericFamily::Sans => SOURCE_SANS_FAMILY,
        }
    }

    /// Ordered concrete *system* faces to try for this class as the final
    /// safety net (step 5). Cross-platform supersets; only consulted when every
    /// earlier step missed, so casting a wide net maximizes landing on the
    /// right class on whatever machine runs.
    pub(super) fn system_chain(self) -> &'static [&'static str] {
        match self {
            GenericFamily::Serif => &[
                "Georgia",
                "Times New Roman",
                "Times",
                "PT Serif",
                "Noto Serif",
                "Liberation Serif",
                "DejaVu Serif",
                "serif",
            ],
            GenericFamily::Monospace => &[
                "SF Mono",
                "Menlo",
                "Consolas",
                "DejaVu Sans Mono",
                "Liberation Mono",
                "Courier New",
                "monospace",
            ],
            GenericFamily::Sans => &[
                "Helvetica",
                "Arial",
                "Segoe UI",
                "Roboto",
                "Noto Sans",
                "Liberation Sans",
                "DejaVu Sans",
                "sans-serif",
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_routes_serif_sans_mono() {
        assert_eq!(
            GenericFamily::classify("Adobe Clean Serif"),
            GenericFamily::Serif
        );
        assert_eq!(
            GenericFamily::classify("Source Serif 4"),
            GenericFamily::Serif
        );
        assert_eq!(
            GenericFamily::classify("Times New Roman"),
            GenericFamily::Serif
        );
        assert_eq!(GenericFamily::classify("Adobe Clean"), GenericFamily::Sans);
        assert_eq!(GenericFamily::classify("Helvetica"), GenericFamily::Sans);
        // "sans serif" contains "serif" but is sans.
        assert_eq!(
            GenericFamily::classify("Open Sans Serif"),
            GenericFamily::Sans
        );
        for m in [
            "Roboto Mono",
            "SF Mono",
            "Source Code Pro",
            "Consolas",
            "DejaVu Sans Mono",
        ] {
            assert_eq!(GenericFamily::classify(m), GenericFamily::Monospace, "{m}");
        }
    }
}

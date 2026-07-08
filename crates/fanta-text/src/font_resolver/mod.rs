//! Per-family font resolution — render text in the document's *actual* fonts.
//!
//! ## Why this exists (the bug it fixes)
//!
//! A `.fig` names the fonts its text was laid out in (Figma's "Adobe Clean", a
//! designer's "Roboto", a code span's "Source Code Pro"). We don't have the
//! proprietary ones installed, and the previous engine papered over *every*
//! missing family with bundled **Inter** as the universal sans fallback. Inter's
//! metrics differ from the real faces, so text laid out to fit a box in the
//! original font overflows or wraps in ours. Figma itself sidesteps this by
//! *downloading the document's real fonts* and rendering with them.
//!
//! This module does the same thing, resolving each requested family to the
//! closest face we can actually obtain — and crucially it stops treating Inter
//! as the blanket override. Inter is still available *by name*, but a plain or
//! unknown family no longer silently becomes Inter.
//!
//! ## The resolution chain (first hit wins)
//!
//! For a requested family name, [`FontResolver::resolve_families`] builds the
//! ordered Skia family list that makes Skia's paragraph fallback paint with the
//! first face that resolves. The intent, in priority order, is:
//!
//! 1. **Installed system font** with that exact name. The requested name always
//!    leads the list, and the engine's system [`skia_safe::FontMgr`] answers for
//!    it — so a user who installs the real "Adobe Clean" gets it verbatim.
//! 2. **A bundled face** for that family (Inter and the three Source families are
//!    vendored in-repo and registered on the provider under their canonical
//!    names — see [`bundled`]).
//! 3. **A real font-bank download** by family name: the obtainable face is
//!    fetched from `github.com/google/fonts` (OFL/Apache/UFL trees) via the raw
//!    CDN — the actual filename is read from the family's `METADATA.pb` rather
//!    than guessed — then cached on disk (`~/.cache/fanta/fonts`) and registered
//!    on the provider, reused from cache on later runs. This is what makes a
//!    document family that exists on a free bank ("Roboto", "Open Sans", "Lato",
//!    "Work Sans", "Noto Sans", …) render in its **real** font. Best-effort and
//!    **offline-safe**: no network, a 404, or any error just falls through to the
//!    next step — never a panic. See [`download`] and [`FontResolver::prewarm`].
//! 4. **Opt-in external font directories** from `FANTA_FONT_DIRS`, registered by
//!    their embedded family names. This lets a fidelity run point at local fonts
//!    that are not installed system-wide, for example an exact `Source Sans Pro`
//!    copy used by an imported `.fig`.
//! 5. **A known-proprietary → open substitute** for families we can't legally
//!    obtain. **Adobe Clean → Source Sans 3**, **Adobe Clean Serif → Source
//!    Serif 4**, **Adobe Clean / monospace code → Source Code Pro**. These are
//!    Adobe's *own* open fonts and are metrically far closer to Adobe Clean than
//!    Inter — this is the key fix for "the text doesn't fit its box".
//! 6. **Generic-class system faces** (serif / sans / mono) as a last resort, so
//!    even a wholly-unknown serif name still lands on *a* serif face.
//!
//! ## How the list maps onto Skia
//!
//! Skia resolves a `TextStyle`'s family list left-to-right against the font
//! managers installed on the [`skia_safe::textlayout::FontCollection`]: an
//! *asset* manager (our [`provider`](FontResolver::provider), carrying the
//! bundled + downloaded faces) probed first, then the *default* manager (the
//! system fonts). The same list is used for measurement and painting, so layout
//! geometry never disagrees with what's drawn.

mod bundled;
mod download;
mod external;
mod generic;
mod resolver;
mod substitute;

pub use bundled::{
    INTER_FAMILY, SOURCE_CODE_FAMILY, SOURCE_SANS_FAMILY, SOURCE_SERIF_FAMILY,
    bundled_family_names, bundled_preview_bytes,
};
pub use generic::GenericFamily;
pub use resolver::FontResolver;
pub use substitute::{ADOBE_CLEAN_SANS_METRIC_RATIO, ADOBE_CLEAN_SERIF_METRIC_RATIO};

/// Pre-download (to the on-disk cache) every family a document uses that isn't
/// bundled or covered by a proprietary→open substitute, WITHOUT constructing a
/// Skia resolver — so it is `Send` and safe to run on a background thread at
/// document open. The render-thread resolver then finds each font already
/// cached (a process-memo / disk hit) and never stalls a paint on a network
/// fetch (the fetch is otherwise triggered lazily on first shape, up to a
/// multi-second timeout).
///
/// Returns the families actually fetched this call, deduped. Offline-safe and
/// idempotent: a family that can't be downloaded — or has no network — is
/// skipped, and repeat calls are memoized.
pub fn prewarm_font_downloads<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut fetched: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for family in families {
        let family = family.trim();
        if family.is_empty() || seen.iter().any(|s| s.eq_ignore_ascii_case(family)) {
            continue;
        }
        seen.push(family.to_string());
        // Bundled (vendored glyph data) and proprietary→open substitutes never
        // need a download — mirrors `FontResolver::prewarm`'s skip logic.
        if bundled::BUNDLED
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(family))
            || substitute::proprietary_substitute(family).is_some()
        {
            continue;
        }
        if download::fetch_google_font(family).is_some() {
            fetched.push(family.to_string());
        }
    }
    fetched
}

pub(crate) use resolver::font_style;

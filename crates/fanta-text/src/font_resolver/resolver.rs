//! The resolver itself: owns the asset provider, builds the ordered family
//! list, exposes the metric-adjust ratio, and runs the best-effort download.

use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

use skia_safe::{
    FontMgr, FontStyle,
    font_style::{Slant, Weight, Width},
    textlayout::TypefaceFontProvider,
};

#[cfg(any(test, all(feature = "native-fonts", not(target_os = "emscripten"))))]
use super::bundled::INSTANCE_WEIGHTS;
use super::bundled::{BUNDLED, instance_at_weight, register_bundled};
#[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
use super::download::{download_cache, fetch_google_font};
#[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
use super::external::register_external_font_dirs;
use super::generic::GenericFamily;
use super::substitute::{proprietary_substitute, substitute_metric_ratio};

/// A read-only observer that can cross threads without sharing a Skia provider.
#[derive(Clone, Debug)]
pub struct FontGenerationObserver {
    generation: Arc<AtomicU64>,
}

impl FontGenerationObserver {
    /// Saturation cannot certify unchanged font inputs.
    pub fn current(&self) -> Option<u64> {
        let generation = self.generation.load(Ordering::Acquire);
        (generation != u64::MAX).then_some(generation)
    }

    /// Independent providers may have equal epochs but different registered faces.
    pub fn same_provider(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation)
    }
}

#[derive(Clone, Copy)]
pub struct FontRegistration<'a> {
    pub family: &'a str,
    pub weight: u16,
    pub italic: bool,
    pub face_index: u32,
    pub bytes: &'a [u8],
}

#[derive(Debug, thiserror::Error)]
pub enum FontCoverageError {
    #[error("missing supplied font family {family:?}")]
    MissingFamily { family: String },
    #[error("missing supplied face for {family:?}, weight {weight}, italic {italic}")]
    MissingFace {
        family: String,
        weight: u16,
        italic: bool,
    },
    #[error("supplied font {family:?} cannot render U+{character:04X}")]
    MissingGlyph { family: String, character: u32 },
}

#[derive(Debug, thiserror::Error)]
pub enum FontRegistrationError {
    #[error("invalid font registration for {family:?}: {reason}")]
    Invalid {
        family: String,
        reason: &'static str,
    },
    #[error("font substitution {family:?} names an unregistered face {replacement:?}")]
    MissingSubstitute { family: String, replacement: String },
}

/// Resolves font-family names to an ordered Skia family list, owning the asset
/// [`TypefaceFontProvider`] that carries the bundled (and any downloaded) faces.
///
/// One resolver backs a [`crate::layout::LayoutEngine`]. It is cheap to clone
/// (the provider is reference-counted) and is intended to live for the
/// document's lifetime so font registration happens once.
///
/// Resolution is memoized per resolver: the system [`FontMgr`] is created
/// once (constructing it on macOS enumerates every installed family over
/// XPC), each downloaded family is decoded and registered into the provider
/// at most once, and the ordered family list for a requested name is cached.
/// Without this every text run of every layout paid the manager construction
/// plus a disk read and re-registration on each cache hit — ~4 ms per text
/// node, which dominated cold frames and pans that reveal text.
#[derive(Clone)]
pub struct FontResolver {
    provider: TypefaceFontProvider,
    generation: Arc<AtomicU64>,
    /// The one system font manager for this resolver's lifetime.
    system: FontMgr,
    /// Families already decoded and registered into `provider`, with the name
    /// they were registered under (`None` when the download failed).
    #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
    registered: Arc<Mutex<HashMap<String, Option<String>>>>,
    /// Memoized [`resolve_families`](Self::resolve_families) results.
    families: Arc<Mutex<HashMap<String, Vec<String>>>>,
    /// Memoized [`is_installed_exact`](Self::is_installed_exact) probes.
    installed: Arc<Mutex<HashMap<String, bool>>>,
    supplied_families: Option<Arc<BTreeMap<String, String>>>,
}

impl FontResolver {
    /// Build a resolver with every bundled family registered.
    pub fn new() -> Self {
        let mgr = FontMgr::default();
        let mut provider = TypefaceFontProvider::new();
        for fam in BUNDLED {
            register_bundled(&mut provider, &mgr, fam);
        }
        #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
        register_external_font_dirs(&mut provider, &mgr);
        Self {
            provider,
            generation: Arc::new(AtomicU64::new(0)),
            system: mgr,
            #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
            registered: Arc::new(Mutex::new(HashMap::new())),
            families: Arc::new(Mutex::new(HashMap::new())),
            installed: Arc::new(Mutex::new(HashMap::new())),
            supplied_families: None,
        }
    }

    /// Registration epoch shared by clones of this provider. Saturation fails
    /// closed because an unchanged sentinel cannot prove stable font inputs.
    pub fn generation(&self) -> Option<u64> {
        let generation = self.generation.load(Ordering::Acquire);
        (generation != u64::MAX).then_some(generation)
    }

    /// Observe registration changes without retaining or exposing the Skia provider.
    pub fn font_generation_observer(&self) -> FontGenerationObserver {
        FontGenerationObserver {
            generation: self.generation.clone(),
        }
    }

    pub fn from_fonts<'a>(
        registrations: impl IntoIterator<Item = FontRegistration<'a>>,
        substitutions: &BTreeMap<String, String>,
    ) -> Result<Self, FontRegistrationError> {
        let decoder = FontMgr::default();
        let mut provider = TypefaceFontProvider::new();
        let mut families = BTreeMap::new();
        let mut registered_faces = std::collections::BTreeSet::new();
        for registration in registrations {
            let family = registration.family;
            let invalid = |reason| FontRegistrationError::Invalid {
                family: family.to_owned(),
                reason,
            };
            if family.trim().is_empty()
                || family.contains('\0')
                || !(1..=1000).contains(&registration.weight)
            {
                return Err(invalid("family and weight must be valid"));
            }
            if !registered_faces.insert((
                family.to_ascii_lowercase(),
                registration.weight,
                registration.italic,
            )) {
                return Err(invalid("duplicate face registration"));
            }
            let face_count = collection_face_count(registration.bytes)
                .ok_or_else(|| invalid("invalid font collection header"))?;
            if registration.face_index >= face_count {
                return Err(invalid("face index is outside the font collection"));
            }
            let face = decoder
                .new_from_data(
                    skia_safe::Data::new_copy(registration.bytes),
                    Some(registration.face_index),
                )
                .ok_or_else(|| invalid("font bytes could not be decoded"))?;
            if (face.font_style().slant() != Slant::Upright) != registration.italic {
                return Err(invalid(
                    "italic registration disagrees with the supplied face",
                ));
            }
            let face = instance_at_weight(&face, i32::from(registration.weight))
                .ok_or_else(|| invalid("font weight could not be instantiated"))?;
            if *face.font_style().weight() != i32::from(registration.weight) {
                return Err(invalid(
                    "weight registration disagrees with the supplied face",
                ));
            }
            provider.register_typeface(face, family);
            families.insert(family.to_ascii_lowercase(), family.to_owned());
        }
        let supplied_names = families.clone();
        for (family, replacement) in substitutions {
            if family.trim().is_empty() || family.contains('\0') {
                return Err(FontRegistrationError::Invalid {
                    family: family.clone(),
                    reason: "invalid substitute name",
                });
            }
            let canonical = supplied_names
                .get(&replacement.to_ascii_lowercase())
                .cloned()
                .ok_or_else(|| FontRegistrationError::MissingSubstitute {
                    family: family.clone(),
                    replacement: replacement.clone(),
                })?;
            families.insert(family.to_ascii_lowercase(), canonical);
        }
        Ok(Self {
            provider,
            generation: Arc::new(AtomicU64::new(0)),
            system: FontMgr::empty(),
            #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
            registered: Arc::new(Mutex::new(HashMap::new())),
            families: Arc::new(Mutex::new(HashMap::new())),
            installed: Arc::new(Mutex::new(HashMap::new())),
            supplied_families: Some(Arc::new(families)),
        })
    }

    pub fn supports_family(&self, family: &str) -> bool {
        self.supplied_families.as_ref().map_or_else(
            || self.is_resolved_exact(family),
            |families| families.contains_key(&family.to_ascii_lowercase()),
        )
    }

    pub(crate) fn supplied_only(&self) -> bool {
        self.supplied_families.is_some()
    }

    pub fn validate_text(
        &self,
        family: &str,
        weight: u16,
        italic: bool,
        text: &str,
    ) -> Result<(), FontCoverageError> {
        let manager: FontMgr = self.provider.clone().into();
        let requested = font_style(weight, italic);
        let face = self
            .resolve_families(family)
            .iter()
            .find_map(|resolved| manager.match_family_style(resolved, requested))
            .ok_or_else(|| FontCoverageError::MissingFamily {
                family: family.to_owned(),
            })?;
        if *face.font_style().weight() != i32::from(weight)
            || (face.font_style().slant() != Slant::Upright) != italic
        {
            return Err(FontCoverageError::MissingFace {
                family: family.to_owned(),
                weight,
                italic,
            });
        }
        for character in text.chars() {
            // Controls, joiners and variation selectors guide shaping without
            // requiring a standalone glyph in the selected face.
            if character.is_control()
                || matches!(character as u32, 0x200B..=0x200D | 0x2060 | 0xFE00..=0xFE0F | 0xE0100..=0xE01EF)
            {
                continue;
            }
            if face.unichar_to_glyph(character as i32) == 0 {
                return Err(FontCoverageError::MissingGlyph {
                    family: family.to_owned(),
                    character: character as u32,
                });
            }
        }
        Ok(())
    }

    /// The asset font provider, to install on a Skia `FontCollection` as its
    /// asset manager. Carries the bundled families plus anything downloaded
    /// this run; probed before the system manager, but only answers for the
    /// families it actually carries, so an installed system font of the same
    /// name still wins through the requested-name-leads-the-list rule.
    pub fn provider(&self) -> &TypefaceFontProvider {
        &self.provider
    }

    /// Best-effort **pre-warm**: try to download and register each family in
    /// `families` from the free font bank up front, so a subsequent live render
    /// paints obtainable families in their real faces without a layout-time
    /// network stall.
    ///
    /// Returns the subset of `families` that resolved to a genuine downloaded
    /// face — i.e. the families now available *for real* rather than via a
    /// substitute. Families that are installed, bundled, proprietary, or simply
    /// not on the bank are skipped (a proprietary name keeps its Source
    /// substitute; an unobtainable name keeps its generic fallback). Offline-safe
    /// — with no network every family is skipped and the slice comes back empty.
    ///
    /// Idempotent and cheap to re-call: the per-process memo and the on-disk
    /// cache mean an already-resolved family is a no-op.
    pub fn prewarm<'a>(&self, families: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        let mut downloaded = Vec::new();
        for fam in families {
            // Skip names we already cover or never download (installed-exact is
            // probed lazily by Skia, so we only short-circuit bundled +
            // proprietary here; the download attempt itself is cheap on a memo
            // hit and offline-safe on a miss).
            if self.is_bundled(fam) || proprietary_substitute(fam).is_some() {
                continue;
            }
            if let Some(name) = self.try_download(fam) {
                if !downloaded.iter().any(|d: &String| d == &name) {
                    downloaded.push(name);
                }
            }
        }
        downloaded
    }

    /// Whether `requested` resolves to a **real downloaded face** from the free
    /// font bank this run — as opposed to an installed face, a bundled face, a
    /// proprietary→open substitute, or a generic fallback.
    ///
    /// Used by the pre-warm reporter and tests to classify which document
    /// families render in their genuine font versus a substitute. Triggers the
    /// (memoized, offline-safe) download attempt if it hasn't run yet.
    pub fn is_downloaded(&self, requested: &str) -> bool {
        if self.is_bundled(requested) || proprietary_substitute(requested).is_some() {
            return false;
        }
        self.try_download(requested).is_some()
    }

    /// Build the ordered Skia family list for a requested family.
    ///
    /// Implements the resolution chain (module docs). The list, in order:
    ///
    /// 1. the requested name itself (installed system font wins, or a bundled /
    ///    just-downloaded face of that exact name),
    /// 2. for a known-proprietary family, its Source substitute (step 4) —
    ///    placed high so Adobe Clean lands on Source, not on a generic system
    ///    sans,
    /// 3. a best-effort Google-Fonts download of the requested family, if it
    ///    registered successfully (step 3),
    /// 4. the generic-class system chain (step 5).
    ///
    /// Names are de-duplicated case-insensitively. Skia walks the list and
    /// paints with the first face that resolves.
    pub fn resolve_families(&self, requested: &str) -> Vec<String> {
        if let Some(cached) = self
            .families
            .lock()
            .ok()
            .and_then(|memo| memo.get(requested).cloned())
        {
            return cached;
        }
        let resolved = self.resolve_families_uncached(requested);
        if let Ok(mut memo) = self.families.lock() {
            memo.insert(requested.to_string(), resolved.clone());
        }
        resolved
    }

    fn resolve_families_uncached(&self, requested: &str) -> Vec<String> {
        if let Some(families) = &self.supplied_families {
            return families
                .get(&requested.to_ascii_lowercase())
                .cloned()
                .into_iter()
                .collect();
        }
        let class = GenericFamily::classify(requested);
        let mut out: Vec<String> = Vec::new();
        let mut push = |name: &str| {
            if !name.is_empty() && !out.iter().any(|e| e.eq_ignore_ascii_case(name)) {
                out.push(name.to_string());
            }
        };

        // 1. Exact requested family (installed > bundled-by-this-name), except
        // legacy Source Pro names. Those are renamed Source families that must
        // stay deterministic across machines where the old system fonts happen
        // to be installed or cached; route them through the bundled Source
        // successor and metric adjust like Adobe Clean.
        let deterministic_source_substitute = legacy_source_pro_family(requested);
        if !deterministic_source_substitute {
            push(requested);
        }

        // 4 (proprietary substitute) — high priority so Adobe Clean uses Source,
        // not a generic sans, *before* we waste a network round-trip trying to
        // download an unobtainable family.
        if let Some(sub) = proprietary_substitute(requested) {
            push(sub);
        } else {
            // 3. Best-effort Google-Fonts download of the requested family. Only
            // attempted for families we don't already cover (not proprietary,
            // not one of our bundled canonical names). Offline-safe.
            if !self.is_bundled(requested) {
                if let Some(downloaded) = self.try_download(requested) {
                    push(&downloaded);
                }
            }
        }

        // 5. Generic-class system faces, last resort.
        for fam in class.system_chain() {
            push(fam);
        }

        // Optional resolution trace for diagnosing which document families render
        // real vs substitute. Off by default; set `FANTA_LOG_FONTS=1` to print
        // each requested family, how it was handled (bundled / substituted /
        // downloaded / installed-or-generic), and the resolved Skia chain.
        if log_fonts_enabled() {
            let kind = if self.is_bundled(requested) {
                "bundled"
            } else if proprietary_substitute(requested).is_some() {
                "substitute"
            } else if self.is_downloaded(requested) {
                "downloaded"
            } else {
                "installed-or-generic"
            };
            eprintln!("[fanta-fonts] {requested:?} -> {kind}: {out:?}");
        }

        out
    }

    /// The horizontal metric-adjust ratio to apply to a run of `requested`, or
    /// `1.0` for no adjustment.
    ///
    /// This is the metric-compatible-substitution knob: a value `< 1.0` tightens
    /// the substitute's advance widths (and `> 1.0` widens them) so the open
    /// face we actually paint approximates the proprietary original's metrics,
    /// keeping text inside the boxes the `.fig` was laid out in. See
    /// [`substitute_metric_ratio`] for the calibrated factors and rationale.
    ///
    /// The adjustment applies **only when the requested family is genuinely
    /// substituted** — i.e. it is one of our known proprietary/renamed names
    /// *and* it is not actually installed on this machine under that exact name.
    /// An installed exact "Adobe Clean", a Google-Fonts-obtainable family, and
    /// every ordinary family therefore render at their true metrics (ratio
    /// `1.0`), satisfying the invariant that only the proprietary→open swap is
    /// adjusted. The same ratio feeds both measurement and paint (the caller
    /// applies it on the single `TextStyle` used for both), so auto-layout
    /// widths and painted glyphs never disagree.
    pub fn metric_ratio_for(&self, requested: &str) -> f32 {
        if self.supplied_only() {
            return 1.0;
        }
        match substitute_metric_ratio(requested) {
            // A known substitution candidate — but skip the adjust if the real
            // face is installed under its exact name (then the requested-name-
            // leads-the-list rule paints it verbatim and no substitution occurs).
            Some(ratio) if !self.substitution_uses_exact_face(requested) => ratio,
            _ => 1.0,
        }
    }

    fn substitution_uses_exact_face(&self, requested: &str) -> bool {
        if legacy_source_pro_family(requested) {
            false
        } else {
            self.is_resolved_exact(requested)
        }
    }

    /// Whether any exact-name manager resolves `requested`, either from
    /// `FANTA_FONT_DIRS` / bundled assets or from the system. If true, the first
    /// family in the Skia list paints the requested face verbatim and must not
    /// receive substitute metric adjustment.
    fn is_resolved_exact(&self, requested: &str) -> bool {
        self.is_asset_exact(requested) || self.is_installed_exact(requested)
    }

    /// Whether the resolver's asset provider resolves `requested` under that
    /// exact family name.
    fn is_asset_exact(&self, requested: &str) -> bool {
        let mgr: FontMgr = self.provider.clone().into();
        mgr.match_family_style(requested, FontStyle::default())
            .is_some()
    }

    /// Whether the system font manager resolves `requested` to a face under that
    /// exact family name — i.e. the user has the real font installed, so no
    /// substitution (and no metric adjust) happens. Probes the default
    /// [`FontMgr`]; cheap and only called for the handful of known
    /// substitution-candidate names.
    fn is_installed_exact(&self, requested: &str) -> bool {
        if let Some(known) = self
            .installed
            .lock()
            .ok()
            .and_then(|memo| memo.get(requested).copied())
        {
            return known;
        }
        let installed = self
            .system
            .match_family_style(requested, FontStyle::default())
            .is_some();
        if let Ok(mut memo) = self.installed.lock() {
            memo.insert(requested.to_string(), installed);
        }
        installed
    }

    /// Whether `requested` is one of our bundled canonical families (so we never
    /// try to download a name we already ship).
    fn is_bundled(&self, requested: &str) -> bool {
        BUNDLED
            .iter()
            .any(|b| b.name.eq_ignore_ascii_case(requested))
    }

    /// Best-effort download of `requested` from Google Fonts' OFL repo, cache it
    /// on disk, register it on the provider, and return the family name to use.
    ///
    /// Returns `None` on *any* failure — offline, 404 (family not on Google
    /// Fonts or non-OFL), decode error — so the caller falls through. Each
    /// distinct family is attempted at most once per process
    /// ([`download_cache`]) so a miss doesn't repeatedly stall layout.
    #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
    fn try_download(&self, requested: &str) -> Option<String> {
        if self.supplied_only() {
            return None;
        }
        // Per-resolver memo first: once a family is registered into this
        // provider (or known to be unobtainable) there is nothing left to do.
        if let Some(known) = self
            .registered
            .lock()
            .ok()
            .and_then(|memo| memo.get(requested).cloned())
        {
            return known;
        }

        let cache = download_cache();
        // Copy the process-wide verdict out and release the lock before doing
        // any work: the miss arm re-locks to record its result.
        let known = cache.lock().ok()?.get(requested).cloned();
        let result = match known {
            // The process-wide memo says the bytes are (or are not) on disk,
            // but each resolver owns its own Skia font provider, so a face
            // still has to be registered into this one — exactly once.
            Some(cached) => cached.and_then(|_| self.download_and_register(requested)),
            None => {
                let result = self.download_and_register(requested);
                cache
                    .lock()
                    .ok()?
                    .insert(requested.to_string(), result.clone());
                result
            }
        };
        if let Ok(mut memo) = self.registered.lock() {
            memo.insert(requested.to_string(), result.clone());
        }
        // A newly registered face changes what `resolve_families` returns for
        // this name (and for names whose chain includes it), so drop the
        // family memo; it refills lazily on the next resolution.
        if result.is_some()
            && let Ok(mut memo) = self.families.lock()
        {
            memo.clear();
        }
        result
    }

    /// The disk-cache + network half of [`try_download`](Self::try_download).
    ///
    /// `&self`-mut is impossible (the provider lives behind a clone-shared
    /// handle), but `TypefaceFontProvider::register_typeface` only needs
    /// `&mut self`; we register on a *fresh* clone of the provider handle, which
    /// shares the same underlying Skia object, so the registration is visible to
    /// the engine's collection. This is the same reference-counted handle Skia
    /// itself mutates.
    #[cfg(all(feature = "native-fonts", not(target_os = "emscripten")))]
    fn download_and_register(&self, requested: &str) -> Option<String> {
        let bytes = fetch_google_font(requested)?;
        self.register_downloaded_bytes(requested, &bytes)
    }

    #[cfg(any(test, all(feature = "native-fonts", not(target_os = "emscripten"))))]
    fn register_downloaded_bytes(&self, requested: &str, bytes: &[u8]) -> Option<String> {
        let base = self
            .system
            .new_from_data(skia_safe::Data::new_copy(bytes), None)?;

        // Register under the requested name on a fresh clone of the provider
        // handle (which shares the same underlying Skia object the engine's
        // collection holds, so the registration is visible there). We can't
        // cheaply tell a variable download from a static one, so attempt weight
        // instancing (a no-op clone for static fonts) and always also register
        // the base face as the default instance.
        let mut provider = self.provider.clone();
        for &w in INSTANCE_WEIGHTS {
            if let Some(inst) = instance_at_weight(&base, w) {
                provider.register_typeface(inst, requested);
            }
        }
        provider.register_typeface(base, requested);
        if self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                Some(generation.saturating_add(1))
            })
            .is_err()
        {
            self.generation.store(u64::MAX, Ordering::Release);
        }
        Some(requested.to_string())
    }

    #[cfg(not(all(feature = "native-fonts", not(target_os = "emscripten"))))]
    fn try_download(&self, _requested: &str) -> Option<String> {
        None
    }
}

impl Default for FontResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether the `FANTA_LOG_FONTS` env var requests per-family resolution traces.
/// Read once and memoized so the hot path pays only a relaxed load after the
/// first call — keeping [`FontResolver::resolve_families`] cheap when off.
fn log_fonts_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("FANTA_LOG_FONTS").is_some_and(|v| {
            let v = v.to_string_lossy();
            v != "0" && !v.is_empty()
        })
    })
}

fn legacy_source_pro_family(requested: &str) -> bool {
    let lower = requested.to_ascii_lowercase();
    lower.contains("source sans pro") || lower.contains("source serif pro")
}

/// The number of faces in a font file: its collection header count, or one for
/// a single font. `None` when a collection header is truncated.
fn collection_face_count(bytes: &[u8]) -> Option<u32> {
    if bytes.get(..4) == Some(b"ttcf") {
        bytes
            .get(8..12)
            .and_then(|count| <[u8; 4]>::try_from(count).ok())
            .map(u32::from_be_bytes)
    } else {
        Some(1)
    }
}

/// The family name of each decodable face in a font file or collection, with
/// its face index.
pub fn font_family_names(bytes: &[u8]) -> Vec<(u32, String)> {
    let Some(face_count) = collection_face_count(bytes) else {
        return Vec::new();
    };
    let decoder = FontMgr::default();
    let data = skia_safe::Data::new_copy(bytes);
    (0..face_count)
        .filter_map(|index| {
            decoder
                .new_from_data(data.clone(), Some(index))
                .map(|face| (index, face.family_name()))
        })
        .collect()
}

/// Translate a (weight, italic) pair into a Skia [`FontStyle`]. Shared so the
/// layout engine and the resolver agree on what a run's style resolves to.
pub(crate) fn font_style(weight: u16, italic: bool) -> FontStyle {
    let slant = if italic {
        Slant::Italic
    } else {
        Slant::Upright
    };
    FontStyle::new(Weight::from(i32::from(weight)), Width::NORMAL, slant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supplied_faces_require_valid_bytes_and_explicit_substitutions()
    -> Result<(), FontRegistrationError> {
        let registration = FontRegistration {
            family: "Explicit",
            weight: 400,
            italic: false,
            face_index: 0,
            bytes: include_bytes!("../../assets/inter/Inter-Regular.ttf"),
        };
        let substitutions = BTreeMap::from([("Alias".to_owned(), "Explicit".to_owned())]);
        let resolver = FontResolver::from_fonts([registration], &substitutions)?;
        assert_eq!(resolver.resolve_families("Alias"), ["Explicit"]);
        assert!(resolver.resolve_families("Inter").is_empty());
        assert_eq!(resolver.metric_ratio_for("Adobe Clean"), 1.0);
        assert!(
            resolver
                .validate_text("Alias", 400, false, "Hello\nWorld")
                .is_ok()
        );
        assert!(matches!(
            resolver.validate_text("Alias", 700, false, "Hello"),
            Err(FontCoverageError::MissingFace { .. })
        ));
        assert!(matches!(
            resolver.validate_text("Alias", 400, false, "🚀"),
            Err(FontCoverageError::MissingGlyph { .. })
        ));
        assert!(FontResolver::from_fonts([registration, registration], &BTreeMap::new()).is_err());
        assert!(
            FontResolver::from_fonts(
                [FontRegistration {
                    bytes: b"invalid",
                    ..registration
                }],
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            FontResolver::from_fonts(
                [FontRegistration {
                    face_index: 1,
                    ..registration
                }],
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            FontResolver::from_fonts(
                [registration],
                &BTreeMap::from([("Alias".into(), "Missing".into())])
            )
            .is_err()
        );
        Ok(())
    }
    use crate::font_resolver::{
        INTER_FAMILY, SOURCE_CODE_FAMILY, SOURCE_SANS_FAMILY, SOURCE_SERIF_FAMILY,
    };

    #[test]
    fn registration_epoch_tracks_actual_provider_mutations_and_clones() {
        let resolver = FontResolver::new();
        let shared = resolver.clone();
        let independent = FontResolver::new();
        let observer = resolver.font_generation_observer();
        assert!(observer.same_provider(&shared.font_generation_observer()));
        assert!(!observer.same_provider(&independent.font_generation_observer()));
        assert_eq!(resolver.generation(), Some(0));
        assert_eq!(observer.current(), Some(0));
        resolver.resolve_families(INTER_FAMILY);
        assert_eq!(resolver.generation(), Some(0));
        assert!(
            resolver
                .register_downloaded_bytes("Invalid epoch face", b"not a font")
                .is_none()
        );
        assert_eq!(resolver.generation(), Some(0));
        let bytes = BUNDLED
            .first()
            .and_then(|family| family.upright.first())
            .expect("bundled font");
        assert!(
            resolver
                .register_downloaded_bytes("Retained epoch face", bytes)
                .is_some()
        );
        assert_eq!(resolver.generation(), Some(1));
        assert_eq!(
            std::thread::spawn({
                let observer = observer.clone();
                move || observer.current()
            })
            .join()
            .expect("read-only observer thread"),
            Some(1)
        );
        assert_eq!(shared.generation(), Some(1));
        assert_eq!(independent.generation(), Some(0));
        let provider: FontMgr = shared.provider().clone().into();
        assert!(
            provider
                .match_family_style("Retained epoch face", FontStyle::default())
                .is_some()
        );
        resolver.generation.store(u64::MAX - 1, Ordering::Release);
        assert!(
            resolver
                .register_downloaded_bytes("Retained saturated face", bytes)
                .is_some()
        );
        assert_eq!(resolver.generation(), None);
        assert_eq!(shared.generation(), None);
        assert_eq!(observer.current(), None);
        assert!(
            resolver
                .register_downloaded_bytes("Retained saturated second face", bytes)
                .is_some()
        );
        assert_eq!(resolver.generation(), None);
    }

    // -- metric-compatible substitution ----------------------------------

    #[test]
    fn metric_ratio_for_skips_genuinely_installed_families() {
        // `metric_ratio_for` must return 1.0 (no adjust) for a family that isn't a
        // substitution candidate at all, AND for a candidate that happens to be
        // installed under its exact name (the real font wins, so no substitution).
        let r = FontResolver::new();
        // Plain obtainable/installed families: never adjusted.
        assert_eq!(r.metric_ratio_for("Helvetica"), 1.0);
        assert_eq!(r.metric_ratio_for("Inter"), 1.0);
        assert_eq!(r.metric_ratio_for(SOURCE_SANS_FAMILY), 1.0);
        // Adobe Clean is proprietary and not installed on CI, so it IS adjusted.
        // (If a machine genuinely has Adobe Clean installed, the probe returns
        // 1.0 — the real face wins and gets no adjust, by design.)
        if FontMgr::default()
            .match_family_style("Adobe Clean", FontStyle::default())
            .is_none()
        {
            assert_eq!(
                r.metric_ratio_for("Adobe Clean"),
                crate::font_resolver::ADOBE_CLEAN_SANS_METRIC_RATIO
            );
        }
    }

    // -- resolution chain -------------------------------------------------

    #[test]
    fn adobe_clean_resolves_to_source_sans_not_inter() {
        // The key fix: Adobe Clean must resolve to Source Sans 3 (Adobe's open
        // sans), NOT Inter and NOT a generic system sans first.
        let r = FontResolver::new();
        let fams = r.resolve_families("Adobe Clean");
        assert_eq!(fams.first().map(String::as_str), Some("Adobe Clean"));
        let source = fams.iter().position(|f| f == SOURCE_SANS_FAMILY);
        assert!(source.is_some(), "Source Sans must be in chain: {fams:?}");
        // Inter is only the MISSING-font fallback (the generic chain's head);
        // the proprietary substitute must outrank it so Adobe Clean paints
        // Source Sans, never Inter.
        let inter = fams.iter().position(|f| f == INTER_FAMILY);
        if let (Some(s), Some(i)) = (source, inter) {
            assert!(s < i, "Source must precede the Inter fallback: {fams:?}");
        }
        // Source must precede any generic system sans.
        let helvetica = fams.iter().position(|f| f == "Helvetica");
        if let (Some(s), Some(h)) = (source, helvetica) {
            assert!(s < h, "Source must precede system sans: {fams:?}");
        }
    }

    #[test]
    fn adobe_clean_serif_resolves_to_source_serif() {
        let r = FontResolver::new();
        let fams = r.resolve_families("Adobe Clean Serif");
        assert_eq!(fams.first().map(String::as_str), Some("Adobe Clean Serif"));
        assert!(
            fams.iter().any(|f| f == SOURCE_SERIF_FAMILY),
            "must substitute Source Serif: {fams:?}"
        );
        assert!(
            !fams.iter().any(|f| f == INTER_FAMILY),
            "serif must never pull onto Inter: {fams:?}"
        );
    }

    #[test]
    fn requested_name_always_leads_so_installed_wins() {
        // The exact requested name leads the list, so a genuinely-installed
        // family resolves to itself through the system manager.
        let r = FontResolver::new();
        for fam in ["Helvetica", "Roboto", "Adobe Clean", "Some Custom Font"] {
            assert_eq!(
                r.resolve_families(fam).first().map(String::as_str),
                Some(fam),
                "requested name must lead for {fam}"
            );
        }
    }

    #[test]
    fn unknown_sans_falls_back_to_inter_like_figma_missing_fonts() {
        let r = FontResolver::new();
        // Requesting Inter explicitly resolves to Inter (still available).
        assert_eq!(
            r.resolve_families("Inter").first().map(String::as_str),
            Some("Inter")
        );
        // Figma renders MISSING fonts in Inter, so an unknown, uninstalled sans
        // family must land on bundled Inter — deterministically, ahead of the
        // machine's system sans faces — while the requested name still leads so
        // an installed family always wins.
        let unknown = r.resolve_families("Totally Unknown Sans XYZ");
        assert_eq!(
            unknown.first().map(String::as_str),
            Some("Totally Unknown Sans XYZ")
        );
        let inter = unknown.iter().position(|f| f == INTER_FAMILY);
        let helvetica = unknown.iter().position(|f| f == "Helvetica");
        assert!(
            inter.is_some(),
            "unknown sans must carry the Inter fallback: {unknown:?}"
        );
        if let (Some(i), Some(h)) = (inter, helvetica) {
            assert!(
                i < h,
                "Inter must precede system sans faces so the missing-font \
                 substitution is deterministic: {unknown:?}"
            );
        }
    }

    #[test]
    fn unknown_serif_lands_on_serif_class() {
        let r = FontResolver::new();
        let fams = r.resolve_families("Some Unknown Serif Display");
        assert!(
            fams.iter()
                .any(|f| f == "Georgia" || f == "Times New Roman"),
            "serif class chain present: {fams:?}"
        );
    }

    // -- bundled provider -------------------------------------------------

    #[test]
    fn provider_resolves_bundled_source_families() {
        let r = FontResolver::new();
        let mgr: FontMgr = r.provider().clone().into();
        for name in [
            INTER_FAMILY,
            SOURCE_SANS_FAMILY,
            SOURCE_SERIF_FAMILY,
            SOURCE_CODE_FAMILY,
        ] {
            assert!(
                mgr.match_family_style(name, FontStyle::default()).is_some(),
                "provider must resolve bundled family {name}"
            );
        }
    }

    #[test]
    fn font_style_honors_weight_and_italic() {
        assert_eq!(font_style(400, false).weight(), Weight::from(400));
        assert_eq!(font_style(700, false).weight(), Weight::from(700));
        assert_eq!(font_style(500, false).weight(), Weight::from(500));
        assert_eq!(font_style(400, false).slant(), Slant::Upright);
        assert_eq!(font_style(400, true).slant(), Slant::Italic);
    }

    // -- pre-warm / download classification (offline-safe invariants) --------

    #[test]
    fn prewarm_and_is_downloaded_never_claim_bundled_or_proprietary() {
        // These invariants hold with OR without a network: a bundled family
        // (we already ship it) and a proprietary family (it can't be on a free
        // bank — it substitutes) must NEVER be reported as a real download, so
        // the download attempt is correctly skipped for them. This is the
        // offline-safe contract the resolution order depends on.
        let r = FontResolver::new();
        for fam in [
            INTER_FAMILY,
            SOURCE_SANS_FAMILY,
            SOURCE_SERIF_FAMILY,
            SOURCE_CODE_FAMILY,
            "Adobe Clean",
            "Adobe Clean Serif",
        ] {
            assert!(
                !r.is_downloaded(fam),
                "{fam} is bundled/proprietary and must not be classified as a download"
            );
        }
        // prewarm of only bundled/proprietary names downloads nothing and never
        // touches the network (every name is short-circuited before the fetch),
        // so this stays deterministic and offline under plain `cargo test`. The
        // genuine online download path is exercised by the network-gated
        // integration test `tests/font_download.rs`.
        let got = r.prewarm([INTER_FAMILY, "Adobe Clean", "Adobe Clean Serif"]);
        assert!(
            got.is_empty(),
            "prewarm must skip bundled/proprietary families, got {got:?}"
        );
    }

    #[test]
    fn legacy_source_pro_families_use_deterministic_substitute() {
        assert!(legacy_source_pro_family("Source Sans Pro"));
        assert!(legacy_source_pro_family("Source Serif Pro"));
        assert!(!legacy_source_pro_family("Adobe Clean"));
        assert!(!legacy_source_pro_family(SOURCE_SANS_FAMILY));

        let r = FontResolver::new();
        let sans = r.resolve_families("Source Sans Pro");
        assert_eq!(sans.first().map(String::as_str), Some(SOURCE_SANS_FAMILY));
        let serif = r.resolve_families("Source Serif Pro");
        assert_eq!(serif.first().map(String::as_str), Some(SOURCE_SERIF_FAMILY));
    }
}

//! Network-gated proof that the free-font-bank downloader actually works:
//! it fetches a known family ("Roboto") by name, caches it, registers it, and
//! the engine shapes that family with the *real* font's metrics — distinct from
//! the generic fallback — and reuses the on-disk cache on a second resolver.
//!
//! ## Why this is `#[ignore]`d AND probe-gated
//!
//! The test requires the *online path* to genuinely fetch — so it really does
//! hit `raw.githubusercontent.com`. It is `#[ignore]`d so the default
//! `cargo test --workspace` stays hermetic (no network, no `~/Library/Caches`
//! writes); the reachability probe additionally lets an explicit `--ignored`
//! run no-op cleanly on an offline machine instead of failing.
//!
//! Run it explicitly with output:
//! `cargo test -p fanta-text --test font_download -- --ignored --nocapture`

use std::io::Read as _;
use std::time::Duration;

use fanta_text::{FontResolver, LayoutEngine, SOURCE_SANS_FAMILY, TextBuffer, TextStyle};

/// A family that is reliably on the free bank (Google Fonts `ofl/roboto/`).
const OBTAINABLE: &str = "Roboto";
/// A family that cannot exist on the bank, so it must fall to the generic
/// fallback rather than a download.
const UNOBTAINABLE: &str = "Zzqx Nonexistent Family 90210";

/// True if `raw.githubusercontent.com` answers within a short timeout. Used to
/// gate the live assertions so an offline run passes cleanly.
fn network_available() -> bool {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(8)))
        .build();
    let agent: ureq::Agent = config.into();
    // A small, stable file in the repo root; any 200 proves reachability.
    match agent
        .get("https://raw.githubusercontent.com/google/fonts/main/README.md")
        .call()
    {
        Ok(mut resp) => {
            if resp.status() != 200 {
                return false;
            }
            // Drain a little so the connection completes cleanly.
            let mut buf = [0u8; 64];
            let _ = resp.body_mut().as_reader().read(&mut buf);
            true
        }
        Err(_) => false,
    }
}

#[test]
#[ignore = "network: fetches from raw.githubusercontent.com; run with -- --ignored"]
fn downloads_caches_and_shapes_a_real_google_font() {
    if !network_available() {
        eprintln!(
            "font_download: network unavailable — skipping the live download \
             assertions (offline-safe). The downloader's pure logic is covered by \
             the unit tests in src/font_resolver/download.rs."
        );
        return;
    }

    // 1. DOWNLOAD: a fresh resolver fetches the family by name from the bank,
    //    caches it to disk, and registers it on its provider. `is_downloaded`
    //    returns true ONLY when that real network+cache+register path succeeded
    //    (it returns false for bundled/proprietary names by construction), so
    //    this is a direct proof the online fetch worked.
    let resolver = FontResolver::new();
    let downloaded = resolver.prewarm([OBTAINABLE]);
    assert!(
        downloaded.iter().any(|f| f == OBTAINABLE),
        "expected {OBTAINABLE} to be downloaded from the free bank, got {downloaded:?}"
    );
    assert!(
        resolver.is_downloaded(OBTAINABLE),
        "{OBTAINABLE} must report as a real download"
    );
    // A clearly-unobtainable family must NOT report as downloaded — it has no
    // entry on the bank, so the fetch returns None and it falls to the generic.
    assert!(
        !resolver.is_downloaded(UNOBTAINABLE),
        "{UNOBTAINABLE} cannot be on the bank and must not report as downloaded"
    );

    // 2. CACHE REUSE: a second, independent resolver (its own in-process memo)
    //    must resolve the same family — this time served from the on-disk cache
    //    written by step 1, with no fresh metadata lookup required.
    let resolver2 = FontResolver::new();
    assert!(
        resolver2.is_downloaded(OBTAINABLE),
        "{OBTAINABLE} must resolve again on a fresh resolver via the disk cache"
    );

    // 3. THE CACHED FILE IS THE REAL FONT: decode the on-disk cache file and
    //    assert it is a valid typeface whose family name is genuinely Roboto —
    //    proving we fetched the real face, not some error page or wrong file.
    let cache_path = dirs::cache_dir()
        .expect("cache dir")
        .join("fanta")
        .join("fonts")
        .join("roboto.ttf");
    let bytes = std::fs::read(&cache_path).expect("cached roboto.ttf present");
    assert!(
        bytes.len() > 50_000,
        "cached Roboto looks too small to be a real font ({} bytes)",
        bytes.len()
    );
    let mgr = skia_safe::FontMgr::default();
    let face = mgr
        .new_from_data(&bytes, None)
        .expect("cached Roboto bytes decode as a typeface");
    assert!(
        face.family_name().to_ascii_lowercase().contains("roboto"),
        "cached typeface family name should be Roboto, got {:?}",
        face.family_name()
    );

    // 4. SHAPES WITH REAL METRICS: through the full LayoutEngine, the downloaded
    //    Roboto shapes to non-empty geometry whose width differs from the bundled
    //    Source Sans 3 (a different real face). If the download had silently
    //    fallen through to a substitute, Roboto would shape with some other face;
    //    the point is it shapes with *Roboto's own* metrics, which differ from
    //    Source Sans 3's.
    //
    //    (We deliberately do NOT contrast against an unobtainable family here:
    //    the generic sans fallback chain includes "Roboto", so an unknown sans
    //    would itself resolve to the just-downloaded Roboto via the asset
    //    provider — equal widths there would be correct, not a bug.)
    let engine = LayoutEngine::new();
    let text = "The quick brown fox jumps over 1,234 lazy dogs";
    let measure = |fam: &str| {
        engine
            .layout(
                &TextBuffer::from_str(text, TextStyle::new(fam, 24.0)),
                1.0e7,
            )
            .width()
    };
    let roboto_w = measure(OBTAINABLE);
    let source_w = measure(SOURCE_SANS_FAMILY);
    assert!(
        roboto_w > 0.0 && source_w > 0.0,
        "both families must shape to non-empty geometry \
         ({OBTAINABLE}={roboto_w}, {SOURCE_SANS_FAMILY}={source_w})"
    );
    assert!(
        (roboto_w - source_w).abs() > 0.5,
        "downloaded {OBTAINABLE} ({roboto_w:.2}) must shape with its own metrics, \
         distinct from the bundled {SOURCE_SANS_FAMILY} ({source_w:.2}) — that they \
         differ confirms the real downloaded face is in use, not a bundled substitute"
    );

    eprintln!(
        "font_download: OK — fetched {OBTAINABLE} ({} KiB) from the free bank, \
         decoded as family {:?}, cached + reused it; shaped at width {roboto_w:.2} \
         vs bundled {SOURCE_SANS_FAMILY} {source_w:.2}",
        bytes.len() / 1024,
        face.family_name(),
    );
}

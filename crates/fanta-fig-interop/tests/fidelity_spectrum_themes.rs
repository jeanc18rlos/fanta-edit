//! Tier-2 E2E (Figma fixture): per-theme resolved-fill fidelity guards.
//!
//! Builds the resolved SceneSnapshot for the Darkest/Light theme pages and
//! asserts theme-specific surface/swatch fills + page disambiguation. Shared
//! fixtures (load_fixture_doc / snapshot_page / fills_match / ints) live in
//! tests/common. Fixture-gated on `FANTA_FIG_FIXTURE`. Split out of
//! fidelity_spectrum.rs.

use fanta_doc::Color;
use fanta_doc::snapshot::NodeSnapshot;

mod common;
use common::*;

/// Sanity: the fixture imports and both theme pages are present + non-trivial.
/// Skips cleanly without the fixture. Not ignored, so a CI run with the fixture
/// set exercises the import path; without it, it's a clean no-op.
#[test]
fn fidelity_pages_import_and_snapshot() {
    let Some(mut doc) = load_fixture_doc() else {
        return;
    };

    let darkest = snapshot_page(&mut doc, &["darkest", "theme"]);
    let light = snapshot_page(&mut doc, &["light", "theme"]);

    let (_, dsnap) = darkest.expect("Darkest Theme page must be present in the Spectrum fixture");
    let (_, lsnap) = light.expect("Light Theme page must be present in the Spectrum fixture");

    eprintln!(
        "Darkest snapshot nodes: {} | Light snapshot nodes: {}",
        dsnap.nodes.len(),
        lsnap.nodes.len()
    );
    assert!(
        dsnap.nodes.len() > 100,
        "Darkest page should resolve to many nodes"
    );
    assert!(
        lsnap.nodes.len() > 100,
        "Light page should resolve to many nodes"
    );

    // The historical OpenPencil report is opt-in only; the active Figma signal is
    // the invariants asserted by the tests below and the API pixel diff harness.
}

/// Figma fixture guard: the Light theme's large documentation-card `_Header`
/// surfaces resolve to the light header fill (`#ffffff`) and do not leak the
/// Darkest theme surface.
#[test]
fn fidelity_light_header_fill() {
    let Some(mut doc) = load_fixture_doc() else {
        return;
    };
    let (_, snap) =
        snapshot_page(&mut doc, &["light", "theme"]).expect("Light Theme page must be present");

    let light = Color::from_hex("#ffffff").expect("valid light surface");
    let darkest = Color::from_hex("#1d1d1d").expect("valid dark surface");
    let mut light_large_headers = 0usize;
    let mut dark_large_headers = Vec::new();

    for node in snap.nodes.iter().filter(|n| n.name == "_Header") {
        let Some(bounds) = node.abs_bounds else {
            continue;
        };
        let w = bounds[2] - bounds[0];
        let h = bounds[3] - bounds[1];
        if w < 500.0 || h < 90.0 {
            continue;
        }
        let Some([r, g, b, a]) = node.fill_rgba else {
            continue;
        };
        let got = Color::rgba(r, g, b, a);
        if fills_match(got, light) {
            light_large_headers += 1;
        }
        if fills_match(got, darkest) {
            dark_large_headers.push((node.name.clone(), ints(&[bounds[0], bounds[1], w, h])));
        }
    }

    assert!(
        light_large_headers > 0,
        "Light Theme must contain at least one large _Header resolved to #ffffff"
    );
    assert!(
        dark_large_headers.is_empty(),
        "Light Theme leaked the darkest header fill into large _Header surfaces: {dark_large_headers:?}"
    );
}

/// FIDELITY GUARD (green): on the Darkest page every card `_Header` frame must
/// resolve to `#1d1d1d`, not the light master's white.
///
/// The bug this guards (now fixed): a dark-theme `_Header` is an INSTANCE whose
/// own baked `fillPaints` can carry the LIGHT master default (white) while its
/// `styleIdForFill` points at the per-page dark FILL style
/// (`darkest/gray/gray-100` → `#1d1d1d`). Real Figma lets the FILL style win
/// for this placement. We assert that at least one large card header resolves to
/// the dark surface and that no large card header leaks the white master fill.
///
/// Fixture-gated: skips cleanly when `FANTA_FIG_FIXTURE` is unset (so
/// `cargo test --workspace` stays green without the 21 MB file); with the
/// fixture set it runs as a normal test and must PASS.
#[test]
fn fidelity_darkest_header_fill() {
    let Some(mut doc) = load_fixture_doc() else {
        return;
    };
    let (_, snap) =
        snapshot_page(&mut doc, &["darkest", "theme"]).expect("Darkest Theme page must be present");

    let dark = Color::from_hex("#1d1d1d").expect("valid dark surface");
    let white = Color::from_hex("#ffffff").expect("valid light surface");
    let mut dark_large_headers = 0usize;
    let mut white_large_headers = Vec::new();

    for node in snap.nodes.iter().filter(|n| n.name == "_Header") {
        let Some(bounds) = node.abs_bounds else {
            continue;
        };
        let w = bounds[2] - bounds[0];
        let h = bounds[3] - bounds[1];
        // Ignore tiny nested headers and color-token chips; the regression was
        // on the large documentation-card header surface.
        if w < 500.0 || h < 90.0 {
            continue;
        }
        let Some([r, g, b, a]) = node.fill_rgba else {
            continue;
        };
        let got = Color::rgba(r, g, b, a);
        if fills_match(got, dark) {
            dark_large_headers += 1;
        }
        if fills_match(got, white) {
            white_large_headers.push((node.name.clone(), ints(&[bounds[0], bounds[1], w, h])));
        }
    }

    assert!(
        dark_large_headers > 0,
        "Darkest Theme must contain at least one large _Header resolved to #1d1d1d"
    );
    assert!(
        white_large_headers.is_empty(),
        "Darkest Theme leaked the white master fill into large _Header surfaces: {white_large_headers:?}"
    );
}

/// FIDELITY GUARD (green): the `Color Background` swatch rectangles resolve to
/// their correct per-page brand colors on BOTH theme pages, and the Mobile
/// example column they live in is present in the snapshotted canvas.
///
/// The bug this guards (now fixed): the Spectrum file carries two canvases per
/// theme name, and the older `find_page` picked the first (a stale duplicate that
/// lacks the Mobile column). That dropped the rightmost section of every page —
/// and with it the `Color Background` swatches — so the comprehensive report
/// dropped them from the resolved snapshot even though the swatch's baked solid
/// paint was being read correctly all along. `find_page` now selects the richer
/// same-named Figma canvas (most descendants).
///
/// We assert on the swatches present in the section body (the off-canvas
/// component-master copies sit at large negative X; we filter to the on-canvas
/// `x > 0` ones). Each must carry the per-page brand color: `#004087` on Darkest,
/// `#0265DC` on Light. Fixture-gated: skips cleanly without the file.
#[test]
fn fidelity_color_background_swatch_fills() {
    let Some(mut doc) = load_fixture_doc() else {
        return;
    };

    // (color, page-needles): the per-page brand color the swatches resolve to.
    for (want_hex, needles, label) in [
        ("#004087", &["darkest", "theme"][..], "Darkest"),
        ("#0265DC", &["light", "theme"][..], "Light"),
    ] {
        let (_, snap) = snapshot_page(&mut doc, needles)
            .unwrap_or_else(|| panic!("{label} Theme page must be present"));

        // The richer canvas was selected: the Mobile example column resolves.
        let mobile = snap
            .nodes
            .iter()
            .filter(|n| n.name.contains("Mobile"))
            .count();
        assert!(
            mobile > 50,
            "{label}: expected the Mobile example column in the snapshotted canvas \
             ({mobile} Mobile nodes); a low count means find_page picked the stale \
             duplicate canvas again (the page-selection regression)."
        );

        // On-canvas Color Background swatches (x > 0 filters out the off-canvas
        // component-master copies that sit at large negative X).
        let want = Color::from_hex(want_hex).unwrap();
        let swatches: Vec<&NodeSnapshot> = snap
            .nodes
            .iter()
            .filter(|n| n.name == "Color Background")
            .filter(|n| n.abs_bounds.is_some_and(|b| b[0] > 0.0))
            .collect();
        assert!(
            !swatches.is_empty(),
            "{label}: no on-canvas Color Background swatches resolved"
        );
        for n in &swatches {
            let got = n
                .fill_rgba
                .unwrap_or_else(|| panic!("{label}: Color Background swatch has no resolved fill"));
            assert!(
                fills_match(Color::rgba(got[0], got[1], got[2], got[3]), want),
                "{label}: Color Background swatch must resolve to {want_hex} (baked \
                 brand color), got {got:?}. A dropped/empty fill here means the baked \
                 solid paint was lost (style pre-pass overwriting an inline paint with \
                 an empty style), or the wrong canvas was snapshotted."
            );
        }
    }
}

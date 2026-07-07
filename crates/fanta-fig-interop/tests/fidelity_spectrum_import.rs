//! Tier-2 E2E (Figma fixture): import report-counter guards.
//!
//! These read the fixture .fig directly and assert importer report counts
//! (masks, component props, frame borders / wave-2 counts). Fixture-gated on
//! `FANTA_FIG_FIXTURE`; skip cleanly when unset. Split out of fidelity_spectrum.rs.

use fanta_fig_interop::{fig_to_doc, read_fig};

/// Fixture-gated: the Spectrum file carries 46 mask nodes (all VECTOR /
/// OUTLINE → ALPHA, none LUMINANCE). Asserts the importer reads + counts every
/// one — a guard that the VECTOR-family build path also honors `mask`. Skips
/// cleanly when the fixture env var is unset.
#[test]
fn fidelity_spectrum_mask_count() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping mask-count fidelity test");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture .fig");
    let fig = read_fig(&bytes).expect("real .fig must parse");
    let (_doc, report, _assets) = fig_to_doc(&fig).expect("map .fig to doc");
    assert_eq!(report.masks_imported, 46, "Spectrum fixture mask count");
    assert_eq!(
        report.masks_luminance, 0,
        "Spectrum fixture has no luminance masks"
    );
}

/// Fixture-gated: the Spectrum file's component masters expose instance-settable
/// properties (`componentPropDefs`), and instances assign them. Before the prop
/// schema was imported, `component_props`/`instances_with_prop_values` were both
/// 0 (the schema was dropped). This guards the import: ≥1 prop parsed and a
/// substantial number of instances carry typed `prop_values`. Skips cleanly
/// without the fixture.
#[test]
fn fidelity_component_prop_schema_imported() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping prop-schema fidelity test");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture .fig");
    let fig = read_fig(&bytes).expect("real .fig must parse");
    let (_doc, report, _assets) = fig_to_doc(&fig).expect("map .fig to doc");
    assert!(
        report.component_props > 0,
        "the Spectrum fixture exposes component properties (got {})",
        report.component_props
    );
    assert!(
        report.instances_with_prop_values > 100,
        "many instances should carry typed prop_values from their assignments \
         (got {})",
        report.instances_with_prop_values
    );
}

/// Frame-border fidelity counters: how many FRAME/SECTION-style groups gained a
/// border (stroke) and/or corner rounding on import — the fix for missing frame
/// borders. Prints the counts and asserts the fixture exercises both (the
/// Spectrum file is full of bordered, rounded cards). Skips cleanly without the
/// fixture env var.
#[test]
fn frame_border_import_counts() {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping frame-border count test");
        return;
    };
    let bytes = std::fs::read(&path).expect("read fixture .fig");
    let fig = read_fig(&bytes).expect("real .fig must parse");
    let (_doc, report, _assets) = fig_to_doc(&fig).expect("map .fig to doc");
    eprintln!("\n=== FRAME-BORDER IMPORT COUNTS ===");
    eprintln!(
        "  frames with a border (stroke): {}",
        report.frames_with_stroke
    );
    eprintln!("  frames with corner rounding:   {}", report.frames_rounded);
    eprintln!(
        "  total strokes imported (all):  {}",
        report.strokes_imported
    );
    assert!(
        report.frames_with_stroke > 0,
        "the Spectrum fixture should import frame borders (got 0)"
    );
    assert!(
        report.frames_rounded > 0,
        "the Spectrum fixture should import rounded frames (got 0)"
    );

    // Wave-2 batch-1 fidelity-feature counts (informational): per-side border
    // weights (F1), wrapping auto-layout frames (F2), arc/pie/donut ellipses (F3).
    // Printed for the report; not asserted > 0 since a given fixture may not
    // exercise every one.
    eprintln!("\n=== WAVE-2 (batch 1) FIDELITY COUNTS ===");
    eprintln!(
        "  nodes with per-side border weights (F1): {}",
        report.per_side_borders
    );
    eprintln!(
        "  wrapping auto-layout frames (F2):        {}",
        report.auto_layout_wrap
    );
    eprintln!(
        "  arc/pie/donut ellipses (F3):             {}",
        report.arc_ellipses
    );
}

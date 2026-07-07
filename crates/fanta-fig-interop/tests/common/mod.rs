//! Shared fixtures for the fidelity_spectrum_* test crates. cargo treats
//! tests/common/ as a plain module (NOT its own test crate), so these helpers
//! are compiled once into each test binary that does `mod common;`.

#![allow(dead_code, unused_imports)] // shared test fixtures: not every test crate uses every helper

use fanta_doc::snapshot::{NodeSnapshot, SceneSnapshot};
use fanta_doc::{Color, Doc, NodeId};
use fanta_fig_interop::{fig_to_doc, read_fig};

pub mod op_golden;

/// Load the fixture into a `Doc`, or `None` (with an eprintln) when the env var
/// is unset — the clean-skip path used by every fixture-gated test here.
pub fn load_fixture_doc() -> Option<Doc> {
    let Ok(path) = std::env::var("FANTA_FIG_FIXTURE") else {
        eprintln!("FANTA_FIG_FIXTURE not set; skipping Tier-2 fidelity test");
        return None;
    };
    let bytes = std::fs::read(&path).expect("read fixture .fig");
    let fig = read_fig(&bytes).expect("real .fig must parse");
    let (doc, _report, _assets) = fig_to_doc(&fig).expect("map .fig to doc");
    Some(doc)
}

/// Find the page whose name contains all of `needles` (case-insensitive). The
/// Spectrum file names its theme pages "↳ 🌙 Darkest Theme" / "↳ 🔆 Light
/// Theme"; matching on the bare words is robust to the emoji/whitespace.
///
/// The Spectrum file carries **two** canvases per theme name (a canonical layout
/// plus a stale/internal duplicate Figma keeps around). They share a name but
/// differ in content: only one carries the full layout — the Mobile example
/// column on the right of every section (~280 extra nodes per page). Figma's
/// visible canvas is the richer one, so among same-named matches we select the
/// page with the most descendants. Picking the first match (the older behavior)
/// snapshotted the Mobile-less duplicate, which dropped the rightmost column of
/// every section: the `Status Light` / `Progress Bar` / `Badge - Mobile` frames
/// and, with them, their `Color Background` swatch rectangles.
/// "Most descendants" is the same disambiguator [`fanta_fig_interop::fig_to_doc`]
/// uses to pick its default active page, so the test and the importer agree.
pub fn find_page(doc: &Doc, needles: &[&str]) -> Option<NodeId> {
    doc.pages()
        .iter()
        .copied()
        .filter(|p| {
            let name = doc.page_name(*p).unwrap_or("").to_lowercase();
            needles.iter().all(|n| name.contains(&n.to_lowercase()))
        })
        .max_by_key(|p| doc.scene.descendants_of(*p).count())
}

/// Build the resolved snapshot for a named page, switching the doc's active
/// page to it first (so binding resolution sees it as active).
pub fn snapshot_page(doc: &mut Doc, needles: &[&str]) -> Option<(NodeId, SceneSnapshot)> {
    let page = find_page(doc, needles)?;
    doc.set_active_page(Some(page));
    Some((page, SceneSnapshot::of_active_page(doc)))
}

/// Per-channel tolerance for "fills match". Resolved solid colors should be
/// exact; a couple of levels absorbs any 8-bit rounding without masking a real
/// theme miss (white vs. `#1d1d1d` is ~226 levels apart).
const FILL_TOL: u8 = 4;

pub fn fills_match(a: Color, b: Color) -> bool {
    let d = |x: u8, y: u8| (x as i16 - y as i16).unsigned_abs();
    d(a.r, b.r) <= FILL_TOL as u16
        && d(a.g, b.g) <= FILL_TOL as u16
        && d(a.b, b.b) <= FILL_TOL as u16
}

pub fn ints(abs: &[f64; 4]) -> [i64; 4] {
    [abs[0] as i64, abs[1] as i64, abs[2] as i64, abs[3] as i64]
}

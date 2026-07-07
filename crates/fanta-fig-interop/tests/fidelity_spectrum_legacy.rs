//! Tier-2 E2E (Figma fixture): LEGACY OpenPencil-derived comprehensive matcher.
//!
//! Historical diagnostic only — NOT part of the default Figma fidelity signal.
//! The `legacy_openpencil_comprehensive_report` test is opt-in (gated on
//! `FANTA_LEGACY_OPENPENCIL_REPORT=1`) and prints a coverage report comparing
//! our resolved snapshot against the committed OpenPencil golden. The golden
//! types + our-node view live in tests/common/op_golden. Split out of
//! fidelity_spectrum.rs.

use std::collections::HashSet;

use fanta_doc::Color;
use fanta_doc::snapshot::NodeSnapshot;

mod common;
use common::op_golden::*;
use common::*;

/// Category of a fidelity gap, ranked into the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GapKind {
    MissingNode,
    ExtraNode,
    WrongFill,
    WrongBounds,
    WrongText,
}

impl GapKind {
    fn tag(self) -> &'static str {
        match self {
            GapKind::MissingNode => "MISSING_NODE",
            GapKind::ExtraNode => "EXTRA_NODE",
            GapKind::WrongFill => "WRONG_FILL",
            GapKind::WrongBounds => "WRONG_BOUNDS",
            GapKind::WrongText => "WRONG_TEXT",
        }
    }
}

struct Gap {
    kind: GapKind,
    /// Visual-impact score: node area × a mismatch weight.
    impact: f64,
    line: String,
}

/// Strict center tolerance (world units) and relative size tolerance — the
/// task's "≤2px center, ~2% size" exact-match budget.
const STRICT_CENTER_TOL: f64 = 2.0;
const SIZE_REL_TOL: f64 = 0.02;
/// Drift-tolerant center tolerance: absorbs the cumulative auto-layout width
/// drift so we can separately count "right structure, wrong x" from "absent".
const DRIFT_CENTER_TOL: f64 = 1500.0;

fn size_ok(gw: f64, gh: f64, ow: f64, oh: f64, rel: f64) -> bool {
    let tw = (gw * rel).max(2.0);
    let th = (gh * rel).max(2.0);
    (gw - ow).abs() <= tw && (gh - oh).abs() <= th
}

fn fill_eq(a: Color, b: Color) -> bool {
    let d = |x: u8, y: u8| (x as i16 - y as i16).unsigned_abs();
    d(a.r, b.r) <= 4 && d(a.g, b.g) <= 4 && d(a.b, b.b) <= 4
}

/// A match candidate found for a golden node.
struct MatchHit {
    our_idx_in_vec: usize,
    /// True when the match also satisfied the STRICT top-left + size budget
    /// (≤2px top-left, ≤2% size) — i.e. "bounds correct".
    strict: bool,
    dist: f64,
}

/// Candidate size filter: a node only binds if its size is within this fraction
/// of the golden's, so a `_Header` doesn't bind to a `_Cell` of the wrong scale.
/// Looser than the strict 2% so the narrower-than-golden auto-layout widths
/// still bind (and surface as WRONG_BOUNDS rather than MISSING).
const CANDIDATE_SIZE_REL_TOL: f64 = 0.30;

/// Find the best our-node for a golden node within `tol` top-left distance,
/// requiring rough size agreement; prefer nearest, then a name match. `used`
/// excludes already-bound our-nodes (one-to-one matching).
fn best_match(
    g: &FullNode,
    align: &Align,
    ours: &[OurNode],
    used: &HashSet<usize>,
    tol: f64,
) -> Option<MatchHit> {
    let (gx, gy) = align.map(g);
    let (gw, gh) = g.size();
    let mut best: Option<(f64, usize)> = None; // (score, vec idx)
    for (i, o) in ours.iter().enumerate() {
        if used.contains(&o.idx) {
            continue;
        }
        let d = ((o.x0 - gx).powi(2) + (o.y0 - gy).powi(2)).sqrt();
        if d > tol {
            continue;
        }
        if !size_ok(gw, gh, o.w, o.h, CANDIDATE_SIZE_REL_TOL) {
            continue;
        }
        let name_match = o.name == g.name || o.name.starts_with(&g.name);
        // Score: top-left distance, with a small bonus for a name match so ties
        // break toward the same-named node.
        let score = d - if name_match { 0.5 } else { 0.0 };
        if best.is_none_or(|(bs, _)| score < bs) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| {
        let o = &ours[i];
        let d = ((o.x0 - gx).powi(2) + (o.y0 - gy).powi(2)).sqrt();
        let strict = d <= STRICT_CENTER_TOL && size_ok(gw, gh, o.w, o.h, SIZE_REL_TOL);
        MatchHit {
            our_idx_in_vec: i,
            strict,
            dist: d,
        }
    })
}

struct Coverage {
    golden_total: usize,
    golden_matched_strict: usize,
    golden_matched_drift: usize,
    our_total: usize,
    our_matched: usize,
    fill_comparable: usize,
    fill_ok: usize,
    /// Subset of `fill_comparable`/`fill_ok` restricted to TEXT nodes — the glyph
    /// color fidelity. Broken out so the dark-theme text-color work has a direct
    /// pass/fail metric separate from frame/rect surface fills (a dark `Title`
    /// resolving to `#222222`/`#000000` instead of the per-page light value is a
    /// text-fill miss, not a surface miss).
    text_fill_comparable: usize,
    text_fill_ok: usize,
    bounds_comparable: usize,
    bounds_ok: usize,
    text_comparable: usize,
    text_ok: usize,
}

/// Run the comprehensive comparison and print the report; returns the coverage
/// summary so a caller can assert / log on it.
fn comprehensive_report(label: &str, golden: &FullGolden, snap: &[NodeSnapshot]) -> Coverage {
    let ours = collect_our_nodes(snap);
    let align = Align::derive(golden, &ours);

    let (golden_match, used) = match_golden_nodes(golden, &align, &ours);

    let golden_matched_strict = golden_match
        .iter()
        .filter(|m| m.as_ref().is_some_and(|h| h.strict))
        .count();
    let golden_matched_drift = golden_match.iter().filter(|m| m.is_some()).count();

    // Fidelity among matched (drift-tolerant matches count — we want to grade
    // the paint of nodes we DID locate, even if drifted).
    let mut cov = Coverage {
        golden_total: golden.nodes.len(),
        golden_matched_strict,
        golden_matched_drift,
        our_total: ours.len(),
        our_matched: used.len(),
        fill_comparable: 0,
        fill_ok: 0,
        text_fill_comparable: 0,
        text_fill_ok: 0,
        bounds_comparable: 0,
        bounds_ok: 0,
        text_comparable: 0,
        text_ok: 0,
    };

    let mut gaps: Vec<Gap> = Vec::new();
    for (gi, g) in golden.nodes.iter().enumerate() {
        grade_golden_node(g, golden_match[gi].as_ref(), &ours, &mut cov, &mut gaps);
    }
    collect_extra_nodes(golden, &align, &ours, &used, &mut gaps);

    print_report(label, golden, &align, &cov, &mut gaps);
    cov
}

/// Two-pass one-to-one matching of golden nodes to our nodes: a strict (≤2px)
/// pass in descending-area order, then a drift-tolerant pass over the leftovers
/// to separate drift from absence. Returns the per-golden match vec and the set
/// of bound our-node ids.
fn match_golden_nodes(
    golden: &FullGolden,
    align: &Align,
    ours: &[OurNode],
) -> (Vec<Option<MatchHit>>, HashSet<usize>) {
    let mut used: HashSet<usize> = HashSet::new();
    let mut golden_match: Vec<Option<MatchHit>> = Vec::with_capacity(golden.nodes.len());
    golden_match.resize_with(golden.nodes.len(), || None);

    // Bind the big, unambiguous surfaces first so small nodes don't steal a large
    // node's slot.
    let mut order: Vec<usize> = (0..golden.nodes.len()).collect();
    order.sort_by(|&a, &b| {
        golden.nodes[b]
            .area()
            .partial_cmp(&golden.nodes[a].area())
            .unwrap()
    });

    // Pass 1: strict.
    for &gi in &order {
        if let Some(hit) = best_match(&golden.nodes[gi], align, ours, &used, STRICT_CENTER_TOL) {
            if hit.strict {
                used.insert(ours[hit.our_idx_in_vec].idx);
                golden_match[gi] = Some(hit);
            }
        }
    }
    // Pass 2: drift-tolerant over still-unmatched golden nodes.
    for &gi in &order {
        if golden_match[gi].is_some() {
            continue;
        }
        if let Some(hit) = best_match(&golden.nodes[gi], align, ours, &used, DRIFT_CENTER_TOL) {
            used.insert(ours[hit.our_idx_in_vec].idx);
            golden_match[gi] = Some(hit);
        }
    }
    (golden_match, used)
}

/// Grade one golden node against its match (or absence): update the fill/bounds/
/// text coverage counters and push any fidelity gaps.
fn grade_golden_node(
    g: &FullNode,
    hit: Option<&MatchHit>,
    ours: &[OurNode],
    cov: &mut Coverage,
    gaps: &mut Vec<Gap>,
) {
    let Some(hit) = hit else {
        // MISSING: we render nothing at this golden box.
        gaps.push(Gap {
            kind: GapKind::MissingNode,
            impact: g.area(),
            line: format!(
                "{:<12} {:<26} {:<7} abs={:?} fill={} — MISSING (we render nothing here)",
                GapKind::MissingNode.tag(),
                truncate(&g.name, 26),
                g.kind,
                ints(&g.abs),
                g.fill.clone().unwrap_or_else(|| "none".into()),
            ),
        });
        return;
    };
    let o = &ours[hit.our_idx_in_vec];
    grade_fill(g, o, cov, gaps);
    grade_bounds(g, o, hit, cov, gaps);
    grade_text(g, o, cov, gaps);
}

/// FILL fidelity for a matched node.
fn grade_fill(g: &FullNode, o: &OurNode, cov: &mut Coverage, gaps: &mut Vec<Gap>) {
    let Some(want) = g.fill_color() else {
        return;
    };
    cov.fill_comparable += 1;
    let is_text = g.kind == "text";
    if is_text {
        cov.text_fill_comparable += 1;
    }
    match o.fill {
        Some(got) if fill_eq(got, want) => {
            cov.fill_ok += 1;
            if is_text {
                cov.text_fill_ok += 1;
            }
        }
        got => {
            let got_s = got.map(|c| c.to_hex()).unwrap_or_else(|| "<none>".into());
            gaps.push(Gap {
                kind: GapKind::WrongFill,
                impact: g.area(),
                line: format!(
                    "{:<12} {:<26} {:<7} abs={:?} expected fill {} but we have {}",
                    GapKind::WrongFill.tag(),
                    truncate(&g.name, 26),
                    g.kind,
                    ints(&g.abs),
                    want.to_hex(),
                    got_s,
                ),
            });
        }
    }
}

/// BOUNDS fidelity (strict center budget) for a matched node.
fn grade_bounds(
    g: &FullNode,
    o: &OurNode,
    hit: &MatchHit,
    cov: &mut Coverage,
    gaps: &mut Vec<Gap>,
) {
    cov.bounds_comparable += 1;
    if hit.strict {
        cov.bounds_ok += 1;
        return;
    }
    // Drift-matched: same node, wrong position/size.
    let (gw, gh) = g.size();
    gaps.push(Gap {
        kind: GapKind::WrongBounds,
        impact: g.area() * (hit.dist / DRIFT_CENTER_TOL).min(1.0),
        line: format!(
            "{:<12} {:<26} {:<7} golden abs={:?} but ours is off by {:.0}px (our wh={}x{} vs {}x{})",
            GapKind::WrongBounds.tag(),
            truncate(&g.name, 26),
            g.kind,
            ints(&g.abs),
            hit.dist,
            o.w.round() as i64,
            o.h.round() as i64,
            gw.round() as i64,
            gh.round() as i64,
        ),
    });
}

/// TEXT fidelity for a matched node.
fn grade_text(g: &FullNode, o: &OurNode, cov: &mut Coverage, gaps: &mut Vec<Gap>) {
    let Some(want) = &g.text else {
        return;
    };
    cov.text_comparable += 1;
    match &o.text {
        Some(got) if got == want => cov.text_ok += 1,
        got => {
            gaps.push(Gap {
                kind: GapKind::WrongText,
                impact: g.area() * 0.5,
                line: format!(
                    "{:<12} {:<26} {:<7} abs={:?} expected text {:?} but we have {:?}",
                    GapKind::WrongText.tag(),
                    truncate(&g.name, 26),
                    g.kind,
                    ints(&g.abs),
                    truncate(want, 24),
                    got.as_deref().map(|s| truncate(s, 24)),
                ),
            });
        }
    }
}

/// EXTRA nodes: our nodes that never bound to any golden node and sit inside the
/// golden content bbox (so we don't flag off-canvas masters / other pages).
fn collect_extra_nodes(
    golden: &FullGolden,
    align: &Align,
    ours: &[OurNode],
    used: &HashSet<usize>,
    gaps: &mut Vec<Gap>,
) {
    let (gx0, gy0, gx1, gy1) = golden_content_bbox(golden, align);
    for o in ours {
        if used.contains(&o.idx) {
            continue;
        }
        // Only count extras that are visible surfaces (have a fill or text) and
        // lie within the golden content region — otherwise we drown in the
        // structural frames OpenPencil's distillation collapsed.
        let cx = o.x0 + o.w * 0.5;
        let cy = o.y0 + o.h * 0.5;
        let in_region = cx >= gx0 && cx <= gx1 && cy >= gy0 && cy <= gy1;
        let paints = o.fill.is_some() || o.text.is_some();
        if in_region && paints {
            gaps.push(Gap {
                kind: GapKind::ExtraNode,
                impact: o.w * o.h * 0.25, // discounted: extras are usually benign overdraw
                line: format!(
                    "{:<12} {:<26} {:<7} our abs=[{},{},{},{}] fill={} text={:?} — EXTRA (legacy reference has nothing here)",
                    GapKind::ExtraNode.tag(),
                    truncate(&o.name, 26),
                    o.kind,
                    o.x0.round() as i64,
                    o.y0.round() as i64,
                    o.w.round() as i64,
                    o.h.round() as i64,
                    o.fill.map(|c| c.to_hex()).unwrap_or_else(|| "none".into()),
                    o.text.as_deref().map(|s| truncate(s, 16)),
                ),
            });
        }
    }
}

/// Print the coverage + fidelity + gap report to stderr (diagnostic only).
fn print_report(label: &str, golden: &FullGolden, align: &Align, cov: &Coverage, gaps: &mut [Gap]) {
    eprintln!("\n############################################################");
    eprintln!(
        "# COMPREHENSIVE FIDELITY REPORT — {label} ({})",
        golden.page
    );
    eprintln!("############################################################");
    eprintln!("align: golden→ours dx={:.0} dy={:.0}", align.dx, align.dy);
    eprintln!("\nCOVERAGE (golden ↔ ours):");
    eprintln!(
        "  golden nodes matched (strict ≤2px center, ≤2% size): {}/{} ({:.1}%)",
        cov.golden_matched_strict,
        cov.golden_total,
        pct(cov.golden_matched_strict, cov.golden_total),
    );
    eprintln!(
        "  golden nodes matched (drift-tolerant, same node wrong-pos): {}/{} ({:.1}%)",
        cov.golden_matched_drift,
        cov.golden_total,
        pct(cov.golden_matched_drift, cov.golden_total),
    );
    eprintln!(
        "  golden MISSING (we render nothing): {}",
        cov.golden_total - cov.golden_matched_drift,
    );
    eprintln!(
        "  our nodes matched: {}/{} ({:.1}%)  → unmatched (EXTRA candidates): {}",
        cov.our_matched,
        cov.our_total,
        pct(cov.our_matched, cov.our_total),
        cov.our_total - cov.our_matched,
    );
    eprintln!("\nFIDELITY AMONG MATCHED:");
    eprintln!(
        "  fills correct: {}/{} ({:.1}%)",
        cov.fill_ok,
        cov.fill_comparable,
        pct(cov.fill_ok, cov.fill_comparable),
    );
    eprintln!(
        "    of which TEXT glyph-color correct: {}/{} ({:.1}%)",
        cov.text_fill_ok,
        cov.text_fill_comparable,
        pct(cov.text_fill_ok, cov.text_fill_comparable),
    );
    eprintln!(
        "  bounds correct (strict): {}/{} ({:.1}%)",
        cov.bounds_ok,
        cov.bounds_comparable,
        pct(cov.bounds_ok, cov.bounds_comparable),
    );
    eprintln!(
        "  text correct: {}/{} ({:.1}%)",
        cov.text_ok,
        cov.text_comparable,
        pct(cov.text_ok, cov.text_comparable),
    );

    // category histogram
    eprintln!("\nGAP CATEGORIES (count):");
    for k in [
        GapKind::MissingNode,
        GapKind::WrongBounds,
        GapKind::WrongFill,
        GapKind::WrongText,
        GapKind::ExtraNode,
    ] {
        let n = gaps.iter().filter(|g| g.kind == k).count();
        eprintln!("  {:<12} {}", k.tag(), n);
    }

    gaps.sort_by(|a, b| b.impact.partial_cmp(&a.impact).unwrap());
    eprintln!("\nTOP-30 GAPS (ranked by visual impact = area × mismatch):");
    for (i, g) in gaps.iter().take(30).enumerate() {
        eprintln!("  {:>2}. {}", i + 1, g.line);
    }
    eprintln!("############################################################\n");
}

/// Golden content bbox mapped into our world space (the union of all golden
/// node boxes — wider than the Status frame because Figma sections don't clip).
fn golden_content_bbox(golden: &FullGolden, align: &Align) -> (f64, f64, f64, f64) {
    let mut x0 = f64::INFINITY;
    let mut y0 = f64::INFINITY;
    let mut x1 = f64::NEG_INFINITY;
    let mut y1 = f64::NEG_INFINITY;
    for g in &golden.nodes {
        x0 = x0.min(g.abs[0] - align.dx);
        y0 = y0.min(g.abs[1] - align.dy);
        x1 = x1.max(g.abs[0] + g.abs[2] - align.dx);
        y1 = y1.max(g.abs[1] + g.abs[3] - align.dy);
    }
    (x0, y0, x1, y1)
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn pct(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        100.0 * a as f64 / b as f64
    }
}

/// Historical diagnostic: print the old OpenPencil-derived comprehensive report
/// for BOTH pages. This is opt-in and must not be treated as the Figma oracle.
#[test]
fn legacy_openpencil_comprehensive_report() {
    if std::env::var("FANTA_LEGACY_OPENPENCIL_REPORT").as_deref() != Ok("1") {
        eprintln!("FANTA_LEGACY_OPENPENCIL_REPORT=1 not set; skipping legacy report");
        return;
    }

    let Some(mut doc) = load_fixture_doc() else {
        return;
    };

    for (needles, file, label) in [
        (
            &["darkest", "theme"][..],
            "op_ref_darkest_full.json",
            "DARKEST",
        ),
        (&["light", "theme"][..], "op_ref_light_full.json", "LIGHT"),
    ] {
        let Some((_, snap)) = snapshot_page(&mut doc, needles) else {
            panic!("page {needles:?} must be present in the Spectrum fixture");
        };
        let golden = load_full_golden(file);
        let cov = comprehensive_report(label, &golden, &snap.nodes);
        // Alignment sanity: we must at least bind the page-bg Status frame
        // (largest area, exact size/pos) — if this fails the whole report is
        // meaningless, so guard it.
        assert!(
            cov.golden_matched_strict + cov.golden_matched_drift > 0,
            "{label}: matched zero golden nodes — alignment/derivation is broken"
        );
    }
}

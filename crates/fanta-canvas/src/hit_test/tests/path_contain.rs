//! Path containment: `point_in_path`, polyline flattening, subpath closure,
//! and PathSegment round-trip.

use super::*;
// ---- point_in_path --------------------------------------------------

#[test]
fn point_in_path_for_rect() {
    let p = PathData::rect(-10.0, -10.0, 20.0, 20.0);
    assert!(point_in_path(&p, DVec2::new(0.0, 0.0)));
    assert!(!point_in_path(&p, DVec2::new(100.0, 0.0)));
}

#[test]
fn point_in_path_for_ellipse() {
    let p = PathData::ellipse(0.0, 0.0, 10.0, 5.0);
    assert!(point_in_path(&p, DVec2::new(0.0, 0.0))); // center
    assert!(point_in_path(&p, DVec2::new(5.0, 0.0))); // inside x
    assert!(!point_in_path(&p, DVec2::new(9.5, 4.5))); // corner outside ellipse
    assert!(!point_in_path(&p, DVec2::new(20.0, 0.0))); // far away
}

#[test]
fn point_in_path_handles_unclosed_subpath() {
    // A degenerate path with no Close should still work — we treat the
    // last point→first as an implicit close for even-odd evaluation.
    let mut p = PathData::new();
    p.move_to(0.0, 0.0)
        .line_to(10.0, 0.0)
        .line_to(10.0, 10.0)
        .line_to(0.0, 10.0);
    assert!(point_in_path(&p, DVec2::new(5.0, 5.0)));
}
// ---- forward-compat path closure ---------------------------------------

#[test]
fn close_then_move_starts_a_fresh_subpath() {
    // A path with two closed subpaths should give two polylines.
    let mut p = PathData::new();
    p.move_to(0.0, 0.0)
        .line_to(5.0, 0.0)
        .line_to(0.0, 5.0)
        .close();
    p.move_to(10.0, 10.0)
        .line_to(15.0, 10.0)
        .line_to(10.0, 15.0)
        .close();
    let polylines = flatten_to_polylines(&p);
    assert_eq!(polylines.len(), 2);
}

#[test]
fn flatten_includes_subpath_start_after_close() {
    let mut p = PathData::new();
    p.move_to(0.0, 0.0)
        .line_to(10.0, 0.0)
        .line_to(10.0, 10.0)
        .close();
    let polylines = flatten_to_polylines(&p);
    assert_eq!(polylines.len(), 1);
    let pl = &polylines[0];
    // Last point should be the start (closed).
    assert_eq!(pl.first().copied(), pl.last().copied());
}

// Direct use of PathSegment in match — keeps the import live for the
// future when we add segment-level accessors.
#[test]
fn segment_enum_round_trips_via_path_data() {
    let p = PathData::rect(0.0, 0.0, 1.0, 1.0);
    let count = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Line { .. }))
        .count();
    assert_eq!(count, 3);
}

use super::*;

#[test]
fn rect_produces_five_segments() {
    let p = PathData::rect(0.0, 0.0, 10.0, 20.0);
    assert_eq!(p.segments.len(), 5); // M, L, L, L, Z
    assert_eq!(p.segments[0], PathSegment::Move { to: [0.0, 0.0] });
    assert_eq!(*p.segments.last().unwrap(), PathSegment::Close);
}

#[test]
fn rect_bounds_are_tight() {
    let p = PathData::rect(5.0, 10.0, 30.0, 20.0);
    let b = p.rough_bounds().unwrap();
    assert_eq!(b.min_x, 5.0);
    assert_eq!(b.min_y, 10.0);
    assert_eq!(b.max_x, 35.0);
    assert_eq!(b.max_y, 30.0);
}

#[test]
fn rough_bounds_ignores_non_finite_points() {
    let mut p = PathData::new();
    p.move_to(f64::NAN, 0.0)
        .line_to(5.0, 10.0)
        .line_to(f64::INFINITY, 20.0)
        .line_to(-2.0, -4.0);
    let b = p.rough_bounds().unwrap();
    assert_eq!(b.min_x, -2.0);
    assert_eq!(b.min_y, -4.0);
    assert_eq!(b.max_x, 5.0);
    assert_eq!(b.max_y, 10.0);
}

#[test]
fn rough_bounds_returns_none_without_finite_points() {
    let mut p = PathData::new();
    p.move_to(f64::NAN, 0.0).line_to(0.0, f64::INFINITY);
    assert!(p.rough_bounds().is_none());
}

#[test]
fn ellipse_produces_four_cubics_and_a_close() {
    let p = PathData::ellipse(0.0, 0.0, 10.0, 5.0);
    assert_eq!(p.segments.len(), 6); // M + 4 cubics + Z
    assert!(matches!(p.segments[0], PathSegment::Move { .. }));
    assert!(matches!(p.segments[1], PathSegment::Cubic { .. }));
    assert!(matches!(p.segments[5], PathSegment::Close));
}

/// Sample a point on the arc's outer rim at angle `t` (the local-box center
/// is `(rx, ry)`).
fn rim(rx: f64, ry: f64, t: f64) -> [f64; 2] {
    [rx + rx * t.cos(), ry + ry * t.sin()]
}

#[test]
fn ellipse_arc_full_no_inner_is_a_plain_ellipse() {
    // A 360° sweep with no inner radius must equal the plain ellipse.
    let arc = PathData::ellipse_arc(20.0, 10.0, 0.0, std::f64::consts::TAU, 0.0);
    let full = PathData::ellipse(20.0, 10.0, 20.0, 10.0);
    assert_eq!(arc.segments, full.segments);
}

#[test]
fn ellipse_arc_pie_starts_and_ends_at_the_center() {
    // A 90° pie slice (quarter turn from 0): M center, L start-of-arc, one
    // cubic, Z. The first point is the box center and the last drawn point
    // before Close lands on the rim at the end angle.
    use std::f64::consts::FRAC_PI_2;
    let p = PathData::ellipse_arc(10.0, 10.0, 0.0, FRAC_PI_2, 0.0);
    assert!(
        matches!(p.segments[0], PathSegment::Move { to } if (to[0]-10.0).abs()<1e-9 && (to[1]-10.0).abs()<1e-9)
    );
    assert!(matches!(p.segments[1], PathSegment::Line { .. }));
    assert!(matches!(p.segments.last(), Some(PathSegment::Close)));
    // Exactly one cubic for a quarter turn.
    let cubics = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    assert_eq!(cubics, 1, "a 90° span is one cubic");
    // The cubic's endpoint sits on the rim at 90° (bottom of the box, y-down).
    if let PathSegment::Cubic { to, .. } = p.segments[2] {
        let want = rim(10.0, 10.0, FRAC_PI_2);
        assert!(
            (to[0] - want[0]).abs() < 1e-6 && (to[1] - want[1]).abs() < 1e-6,
            "end on rim: {to:?} vs {want:?}"
        );
    } else {
        panic!("expected a cubic");
    }
}

#[test]
fn ellipse_arc_half_sweep_uses_two_cubics() {
    // A 180° sweep needs two ≤90° cubics.
    use std::f64::consts::PI;
    let p = PathData::ellipse_arc(15.0, 15.0, 0.0, PI, 0.0);
    let cubics = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    assert_eq!(cubics, 2);
}

#[test]
fn ellipse_arc_donut_has_outer_and_reversed_inner_arcs() {
    // A donut segment: M outer-start, outer cubic(s), L inner-end, inner
    // cubic(s) back, Z. With inner > 0 there is no center Move, and there are
    // twice the cubics of the equivalent pie (outer + inner spans).
    use std::f64::consts::FRAC_PI_2;
    let p = PathData::ellipse_arc(20.0, 20.0, 0.0, FRAC_PI_2, 0.5);
    // First point on the OUTER rim (radius 20), not the center.
    if let PathSegment::Move { to } = p.segments[0] {
        let want = rim(20.0, 20.0, 0.0);
        assert!((to[0] - want[0]).abs() < 1e-6 && (to[1] - want[1]).abs() < 1e-6);
    } else {
        panic!("expected a move");
    }
    let cubics = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    assert_eq!(cubics, 2, "one outer + one inner quarter-turn cubic");
    // There is a Line connecting the outer arc end to the inner arc start.
    assert!(
        p.segments
            .iter()
            .any(|s| matches!(s, PathSegment::Line { .. }))
    );
    assert!(matches!(p.segments.last(), Some(PathSegment::Close)));
}

#[test]
fn ellipse_arc_full_ring_carves_a_hole() {
    // A full ring (360° + inner) is an outer ellipse followed by a reversed
    // inner ellipse, so the contour count is 2× a plain ellipse minus the
    // shared structure: 2 Moves, 8 cubics, 2 Closes.
    let p = PathData::ellipse_arc(30.0, 30.0, 0.0, std::f64::consts::TAU, 0.4);
    let moves = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Move { .. }))
        .count();
    let cubics = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Cubic { .. }))
        .count();
    let closes = p
        .segments
        .iter()
        .filter(|s| matches!(s, PathSegment::Close))
        .count();
    assert_eq!((moves, cubics, closes), (2, 8, 2));
}

#[test]
fn path_round_trips_through_json() {
    let mut p = PathData::new();
    p.move_to(1.0, 2.0)
        .cubic_to(3.0, 4.0, 5.0, 6.0, 7.0, 8.0)
        .close();
    let j = serde_json::to_string(&p).unwrap();
    let back: PathData = serde_json::from_str(&j).unwrap();
    assert_eq!(p, back);
}

#[test]
fn default_fill_rule_is_skipped_in_json() {
    // A path with the default (NonZero) fill rule must NOT emit "fill_rule",
    // so unchanged vector nodes stay byte-identical post-migration.
    let p = PathData::rect(0.0, 0.0, 10.0, 10.0);
    assert_eq!(p.fill_rule, FillRule::NonZero);
    let j = serde_json::to_string(&p).unwrap();
    assert!(
        !j.contains("fill_rule"),
        "default fill_rule must be skipped: {j}"
    );
    // The object form is present (no longer transparent): an object with a
    // `segments` key.
    assert!(j.starts_with("{\"segments\""), "v2 object shape: {j}");
}

#[test]
fn even_odd_fill_rule_serializes_kebab_case() {
    let mut p = PathData::rect(0.0, 0.0, 10.0, 10.0);
    p.fill_rule = FillRule::EvenOdd;
    let j = serde_json::to_value(&p).unwrap();
    assert_eq!(j["fill_rule"], "even-odd");
    let back: PathData = serde_json::from_value(j).unwrap();
    assert_eq!(back.fill_rule, FillRule::EvenOdd);
}

#[test]
fn svg_d_round_trips_mlqcz() {
    let mut p = PathData::new();
    p.move_to(1.0, 2.0)
        .line_to(3.0, 4.0)
        .quad_to(5.0, 6.0, 7.0, 8.0)
        .cubic_to(9.0, 10.0, 11.0, 12.0, 13.0, 14.0)
        .close();
    let d = p.to_svg_d();
    let back = PathData::from_svg_d(&d).unwrap();
    assert_eq!(p.segments, back.segments, "round-trip via d=\"{d}\"");
}

#[test]
fn svg_d_parses_relative_and_shorthand() {
    // m 10 10  l 5 0  h 5  v 5  z  — all relative / shorthand.
    let p = PathData::from_svg_d("m 10 10 l 5 0 h 5 v 5 z").unwrap();
    assert_eq!(
        p.segments,
        vec![
            PathSegment::Move { to: [10.0, 10.0] },
            PathSegment::Line { to: [15.0, 10.0] },
            PathSegment::Line { to: [20.0, 10.0] },
            PathSegment::Line { to: [20.0, 15.0] },
            PathSegment::Close,
        ]
    );
}

#[test]
fn svg_d_implicit_repeated_lineto_after_moveto() {
    // "M 0 0 1 1 2 2" is a Move then two implicit Lines (SVG spec).
    let p = PathData::from_svg_d("M 0 0 1 1 2 2").unwrap();
    assert_eq!(
        p.segments,
        vec![
            PathSegment::Move { to: [0.0, 0.0] },
            PathSegment::Line { to: [1.0, 1.0] },
            PathSegment::Line { to: [2.0, 2.0] },
        ]
    );
}

#[test]
fn svg_d_parses_elliptical_arc() {
    // `A` is supported via endpoint→center conversion + cubic approximation.
    let p = PathData::from_svg_d("M 0 0 A 5 5 0 0 1 10 0").unwrap();
    assert!(matches!(p.segments.first(), Some(PathSegment::Move { .. })));
    assert!(
        p.segments
            .iter()
            .any(|s| matches!(s, PathSegment::Cubic { .. })),
        "arc is approximated by cubic segments"
    );
    let end = p.segments.last().unwrap().end_point().unwrap();
    assert!(
        (end.x - 10.0).abs() < 1e-6 && end.y.abs() < 1e-6,
        "arc ends at its endpoint, got {end:?}"
    );
}

#[test]
fn svg_d_parses_smooth_cubic_and_quadratic() {
    // S reflects the prior cubic's ctrl2 about the current point; with no
    // preceding cubic, ctrl1 = the current point.
    let p = PathData::from_svg_d("M0 0 C0 5 5 5 5 0 S10 5 10 0").unwrap();
    match p.segments.as_slice() {
        [
            PathSegment::Move { .. },
            PathSegment::Cubic { .. },
            PathSegment::Cubic { ctrl1, to, .. },
        ] => {
            // reflection of (5,5) about (5,0) = (5,-5).
            assert!((ctrl1[0] - 5.0).abs() < 1e-9 && (ctrl1[1] + 5.0).abs() < 1e-9);
            assert_eq!(*to, [10.0, 0.0]);
        }
        other => panic!("expected Move,Cubic,Cubic; got {other:?}"),
    }
    // T reflects the prior quad's ctrl; a real logo path with `t` parses.
    assert!(PathData::from_svg_d("M0 0 Q2 2 4 0 t4 0").is_ok());
    // Truly unknown commands still error.
    assert_eq!(
        PathData::from_svg_d("M0 0 B 1 1").unwrap_err(),
        SvgPathError::UnsupportedCommand('B'),
    );
}

#[test]
fn svg_d_rejects_garbage_number() {
    assert!(PathData::from_svg_d("M zz 0").is_err());
}

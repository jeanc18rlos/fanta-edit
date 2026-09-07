//! Convert [`fanta_doc::PathData`] to a Skia [`Path`].
//!
//! Direct one-to-one mapping — every variant of [`PathSegment`] corresponds to
//! a single Skia path method. The conversion is `O(n)` in segment count with
//! one allocation for the result.
//!
//! [`PathSegment`]: fanta_doc::PathSegment

use fanta_doc::{FillRule, PathData, PathSegment};
use skia_safe::{Path, PathFillType};

/// The Skia path whose FILLED COVERAGE honors per-subpath fill rules
/// ([`PathData::subpath_rules`], Figma's per-`fillGeometry` `windingRule`).
///
/// A `Path` carries one fill type, so a doc path whose subpaths were authored
/// with MIXED winding rules cannot be filled as-is: the subpaths are split
/// into one group per rule (entry *i* rules the *i*-th `Move`-started subpath;
/// subpaths beyond the list use the path-level [`PathData::fill_rule`]), each
/// group is materialized with its own fill type, and the groups are unioned
/// with Skia path-ops — the boolean respects each operand's fill type, so the
/// result is exactly the union of the per-rule coverages Figma paints.
///
/// The empty/uniform-rule case (the overwhelmingly common one) returns
/// [`to_sk_path`] unchanged, and so does a failed path-op (degenerate
/// geometry) — approximate coverage beats dropping the shape. Use this for
/// FILLS and fill-derived clips/silhouettes; STROKES must keep [`to_sk_path`]
/// (the union rewrites contours, but a stroke follows the authored ones).
pub(crate) fn to_sk_fill_path(data: &PathData) -> Path {
    if data.subpath_rules.is_empty()
        || data
            .subpath_rules
            .iter()
            .all(|rule| *rule == data.fill_rule)
    {
        return to_sk_path(data);
    }

    // Split the segment list into per-rule groups. A subpath starts at each
    // `Move`; leading segments before any `Move` form subpath 0.
    let mut groups: [PathData; 2] = [PathData::new(), PathData::new()];
    let group_of = |rule: FillRule| match rule {
        FillRule::NonZero => 0,
        FillRule::EvenOdd => 1,
    };
    let mut subpath: usize = 0;
    let mut any_segment = false;
    for segment in &data.segments {
        if matches!(segment, PathSegment::Move { .. }) && any_segment {
            subpath += 1;
        }
        any_segment = true;
        let rule = data
            .subpath_rules
            .get(subpath)
            .copied()
            .unwrap_or(data.fill_rule);
        groups[group_of(rule)].segments.push(*segment);
    }
    groups[0].fill_rule = FillRule::NonZero;
    groups[1].fill_rule = FillRule::EvenOdd;

    match groups {
        [non_zero, even_odd] if non_zero.segments.is_empty() => to_sk_path(&even_odd),
        [non_zero, even_odd] if even_odd.segments.is_empty() => to_sk_path(&non_zero),
        [non_zero, even_odd] => {
            let a = to_sk_path(&non_zero);
            let b = to_sk_path(&even_odd);
            a.op(&b, skia_safe::PathOp::Union)
                .unwrap_or_else(|| to_sk_path(data))
        }
    }
}

/// Materialize a [`PathData`] into a Skia [`Path`].
///
/// This is the **single owner** of the [`FillRule`] → [`PathFillType`] mapping:
/// `fanta-export`'s PDF path renders through Skia transitively, so it inherits
/// the fill type set here rather than duplicating the mapping (per spec 07 §2
/// P3). The doc model stores only the abstract [`FillRule`]; its translation to
/// Skia's `SkPathFillType` happens exactly here so the two consumers can never
/// disagree about what a self-intersecting / nested contour fills.
pub fn to_sk_path(data: &PathData) -> Path {
    let mut path = Path::new();
    for seg in &data.segments {
        match *seg {
            PathSegment::Move { to } => {
                path.move_to((to[0] as f32, to[1] as f32));
            }
            PathSegment::Line { to } => {
                path.line_to((to[0] as f32, to[1] as f32));
            }
            PathSegment::Quad { ctrl, to } => {
                path.quad_to(
                    (ctrl[0] as f32, ctrl[1] as f32),
                    (to[0] as f32, to[1] as f32),
                );
            }
            PathSegment::Cubic { ctrl1, ctrl2, to } => {
                path.cubic_to(
                    (ctrl1[0] as f32, ctrl1[1] as f32),
                    (ctrl2[0] as f32, ctrl2[1] as f32),
                    (to[0] as f32, to[1] as f32),
                );
            }
            PathSegment::Close => {
                path.close();
            }
        }
    }
    // Apply the interior-determination rule. NonZero (the default) maps to
    // Skia's `Winding`; EvenOdd to `EvenOdd` (alternating fill — needed for
    // donut / star geometry where contours overlap). Set after the segments are
    // built because the fill type is a property of the whole path, not a segment.
    path.set_fill_type(match data.fill_rule {
        FillRule::NonZero => PathFillType::Winding,
        FillRule::EvenOdd => PathFillType::EvenOdd,
    });
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_path_bounds_match_input() {
        let p = PathData::rect(10.0, 20.0, 30.0, 40.0);
        let sk = to_sk_path(&p);
        let b = sk.compute_tight_bounds();
        assert!((b.left - 10.0).abs() < 1e-3);
        assert!((b.top - 20.0).abs() < 1e-3);
        assert!((b.right - 40.0).abs() < 1e-3);
        assert!((b.bottom - 60.0).abs() < 1e-3);
    }

    #[test]
    fn ellipse_path_is_centered_and_round_enough() {
        let p = PathData::ellipse(50.0, 50.0, 20.0, 20.0);
        let sk = to_sk_path(&p);
        let b = sk.compute_tight_bounds();
        // The cubic-approximated ellipse should be within ~0.1px of the exact
        // bbox for a 20-unit radius.
        assert!((b.center_x() - 50.0).abs() < 0.1);
        assert!((b.center_y() - 50.0).abs() < 0.1);
        assert!((b.width() - 40.0).abs() < 0.1);
        assert!((b.height() - 40.0).abs() < 0.1);
    }

    #[test]
    fn default_fill_rule_maps_to_winding() {
        // A path with the default (NonZero) fill rule must produce a Skia path
        // whose fill type is `Winding` — the renderer's single owner of the
        // FillRule → SkPathFillType mapping.
        let p = PathData::rect(0.0, 0.0, 10.0, 10.0);
        assert_eq!(p.fill_rule, FillRule::NonZero);
        let sk = to_sk_path(&p);
        assert_eq!(sk.fill_type(), PathFillType::Winding);
    }

    #[test]
    fn even_odd_fill_rule_sets_even_odd_on_the_sk_path() {
        // EvenOdd in the doc must carry through to Skia's EvenOdd fill type so
        // donut / star geometry fills as designed (the only difference between
        // this and the default is the interior rule).
        let mut p = PathData::rect(0.0, 0.0, 10.0, 10.0);
        p.fill_rule = FillRule::EvenOdd;
        let sk = to_sk_path(&p);
        assert_eq!(sk.fill_type(), PathFillType::EvenOdd);
    }
}

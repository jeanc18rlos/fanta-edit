//! Convert [`fanta_doc::PathData`] to a Skia [`Path`].
//!
//! Direct one-to-one mapping — every variant of [`PathSegment`] corresponds to
//! a single Skia path method. The conversion is `O(n)` in segment count with
//! one allocation for the result.
//!
//! [`PathSegment`]: fanta_doc::PathSegment

use fanta_doc::{FillRule, PathData, PathSegment};
use skia_safe::{Path, PathFillType};

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

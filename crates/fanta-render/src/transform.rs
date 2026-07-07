//! Convert [`fanta_doc::Transform2D`] to a Skia [`Matrix`].
//!
//! Skia's `SkMatrix::SetAffine` takes affine components in the order
//! `[scaleX, skewY, skewX, scaleY, transX, transY]`, which is exactly what
//! `Transform2D::to_components` returns (SVG matrix-order: a, b, c, d, tx, ty).
//! The two layouts agree, so this is a direct float-precision narrow.
//!
//! Doc is `f64`; Skia is `f32`. We narrow at render time on purpose (see
//! ARCHITECTURE.md §13) — pan-and-zoom precision lives in the doc, GPU buffers
//! get `f32`.

use fanta_doc::Transform2D;
use skia_safe::Matrix;

/// Build a Skia matrix from a doc transform.
pub fn to_sk_matrix(t: &Transform2D) -> Matrix {
    let [a, b, c, d, tx, ty] = t.to_components();
    let affine = [a as f32, b as f32, c as f32, d as f32, tx as f32, ty as f32];
    Matrix::from_affine(&affine)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skia_safe::Point;

    #[test]
    fn translation_maps_point_correctly() {
        let t = Transform2D::translation(10.0, 20.0);
        let m = to_sk_matrix(&t);
        let p = m.map_point(Point::new(0.0, 0.0));
        assert!((p.x - 10.0).abs() < 1e-4);
        assert!((p.y - 20.0).abs() < 1e-4);
    }

    #[test]
    fn scale_maps_point_correctly() {
        let t = Transform2D::scale(2.0);
        let m = to_sk_matrix(&t);
        let p = m.map_point(Point::new(3.0, 4.0));
        assert!((p.x - 6.0).abs() < 1e-4);
        assert!((p.y - 8.0).abs() < 1e-4);
    }
}

//! Geometric primitives — 2D affine transform and axis-aligned bounds.
//!
//! Doc coordinates are `f64` (ARCHITECTURE.md §13). The render layer projects
//! to `f32` for GPU buffers. Keeping the doc in `f64` matters at high zoom on
//! infinite canvases — even a million-unit pan stays sub-pixel exact.

use glam::{DAffine2, DMat2, DVec2};
use serde::{Deserialize, Serialize};

/// A 2D affine transform stored as the underlying [`DAffine2`].
///
/// Serializes as a 6-element array `[a, b, c, d, tx, ty]` — the standard SVG /
/// PostScript matrix layout — so the JSON projection is small and human-
/// readable. Transforms compose with `*` (parent · child, applied right-to-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform2D(pub DAffine2);

impl Transform2D {
    /// The identity transform — no translation, no rotation, no scale.
    pub const IDENTITY: Self = Self(DAffine2 {
        matrix2: DMat2::from_cols(DVec2::X, DVec2::Y),
        translation: DVec2::ZERO,
    });

    /// Pure translation.
    pub fn translation(tx: f64, ty: f64) -> Self {
        Self(DAffine2::from_translation(DVec2::new(tx, ty)))
    }

    /// Uniform scale around the origin.
    pub fn scale(s: f64) -> Self {
        Self(DAffine2::from_scale(DVec2::splat(s)))
    }

    /// Non-uniform scale around the origin.
    pub fn scale_xy(sx: f64, sy: f64) -> Self {
        Self(DAffine2::from_scale(DVec2::new(sx, sy)))
    }

    /// Rotation in radians around the origin.
    pub fn rotation(radians: f64) -> Self {
        Self(DAffine2::from_angle(radians))
    }

    /// Transform a point in the source space into the target space.
    pub fn transform_point(&self, p: DVec2) -> DVec2 {
        self.0.transform_point2(p)
    }

    /// Transform a vector (no translation component applied).
    pub fn transform_vector(&self, v: DVec2) -> DVec2 {
        self.0.transform_vector2(v)
    }

    /// Inverse transform. Panics if not invertible — design-time data should
    /// not be near-singular, so panicking is a good signal.
    pub fn inverse(&self) -> Self {
        Self(self.0.inverse())
    }

    /// Compose with another transform: `self` is applied first, then `other`.
    pub fn then(&self, other: &Self) -> Self {
        Self(other.0 * self.0)
    }

    /// The 6 components in SVG matrix order.
    pub fn to_components(&self) -> [f64; 6] {
        let m = self.0.matrix2;
        let t = self.0.translation;
        [m.x_axis.x, m.x_axis.y, m.y_axis.x, m.y_axis.y, t.x, t.y]
    }

    /// Whether every matrix/translation component is finite.
    pub fn is_finite(&self) -> bool {
        self.to_components().into_iter().all(f64::is_finite)
    }

    /// Build from 6 components in SVG matrix order.
    pub fn from_components(c: [f64; 6]) -> Self {
        Self(DAffine2 {
            matrix2: DMat2::from_cols(DVec2::new(c[0], c[1]), DVec2::new(c[2], c[3])),
            translation: DVec2::new(c[4], c[5]),
        })
    }
}

impl Default for Transform2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Serialize for Transform2D {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_components().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Transform2D {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self::from_components(<[f64; 6]>::deserialize(d)?))
    }
}

/// Axis-aligned bounding box in some coordinate space.
///
/// Used pervasively: node-local bounds, world bounds, viewport bounds, dirty
/// regions, hit-test fast rejection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Bounds {
    /// A degenerate bounds at the origin. Useful as a starting value for unions.
    pub const ZERO: Self = Self {
        min_x: 0.0,
        min_y: 0.0,
        max_x: 0.0,
        max_y: 0.0,
    };

    /// Bounds from (x, y, width, height).
    pub fn from_xywh(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            min_x: x,
            min_y: y,
            max_x: x + w,
            max_y: y + h,
        }
    }

    /// Bounds from min/max corners. Panics if min > max on either axis.
    pub fn from_min_max(min: DVec2, max: DVec2) -> Self {
        debug_assert!(min.x <= max.x && min.y <= max.y);
        Self {
            min_x: min.x,
            min_y: min.y,
            max_x: max.x,
            max_y: max.y,
        }
    }

    pub fn width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn height(&self) -> f64 {
        self.max_y - self.min_y
    }

    /// Whether every stored coordinate is finite.
    pub fn is_finite(&self) -> bool {
        self.min_x.is_finite()
            && self.min_y.is_finite()
            && self.max_x.is_finite()
            && self.max_y.is_finite()
    }

    pub fn center(&self) -> DVec2 {
        DVec2::new(
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }

    /// Test whether a point is inside (inclusive of edges).
    pub fn contains_point(&self, p: DVec2) -> bool {
        p.x >= self.min_x && p.x <= self.max_x && p.y >= self.min_y && p.y <= self.max_y
    }

    /// Union of two bounds — the smallest box containing both.
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    /// Whether two bounds overlap (used as a hit-test fast reject).
    pub fn intersects(&self, other: &Self) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    /// Transform corner-wise by an affine transform. Result is a new AABB that
    /// contains the transformed (possibly rotated) original.
    pub fn transformed(&self, t: &Transform2D) -> Self {
        self.try_transformed(t).unwrap_or(Self::ZERO)
    }

    /// Checked form of [`Self::transformed`]. Returns `None` when imported or
    /// otherwise external data carries non-finite geometry that cannot produce a
    /// meaningful AABB.
    pub fn try_transformed(&self, t: &Transform2D) -> Option<Self> {
        if !self.is_finite() || !t.is_finite() {
            return None;
        }
        let corners = [
            DVec2::new(self.min_x, self.min_y),
            DVec2::new(self.max_x, self.min_y),
            DVec2::new(self.max_x, self.max_y),
            DVec2::new(self.min_x, self.max_y),
        ];
        let mut points = corners
            .into_iter()
            .map(|c| t.transform_point(c))
            .filter(|p| p.x.is_finite() && p.y.is_finite());
        let first = points.next()?;
        let (mut min, mut max) = (first, first);
        for p in points {
            min = min.min(p);
            max = max.max(p);
        }
        Some(Self::from_min_max(min, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn identity_is_neutral() {
        let p = DVec2::new(3.0, -7.0);
        assert_eq!(Transform2D::IDENTITY.transform_point(p), p);
    }

    #[test]
    fn translation_then_inverse_returns_identity() {
        let t = Transform2D::translation(10.0, 20.0);
        let p = DVec2::new(1.0, 2.0);
        assert_eq!(t.inverse().transform_point(t.transform_point(p)), p);
    }

    #[test]
    fn bounds_union_grows_outward() {
        let a = Bounds::from_xywh(0.0, 0.0, 10.0, 10.0);
        let b = Bounds::from_xywh(5.0, 5.0, 10.0, 10.0);
        let u = a.union(&b);
        assert_eq!(u, Bounds::from_xywh(0.0, 0.0, 15.0, 15.0));
    }

    #[test]
    fn bounds_transform_under_90_deg_rotation() {
        let b = Bounds::from_xywh(1.0, 0.0, 2.0, 1.0);
        let r = b.transformed(&Transform2D::rotation(FRAC_PI_2));
        // A 1x2 rect rotated 90° around origin becomes a 2x1 rect; sanity check
        // that width/height swapped (within float tolerance).
        assert!((r.width() - 1.0).abs() < 1e-9);
        assert!((r.height() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn try_transform_bounds_rejects_non_finite_transform() {
        let b = Bounds::from_xywh(0.0, 0.0, 10.0, 20.0);
        let bad = Transform2D::from_components([1.0, 0.0, 0.0, 1.0, f64::NAN, 0.0]);
        assert!(!bad.is_finite());
        assert!(b.try_transformed(&bad).is_none());
    }

    #[test]
    fn try_transform_bounds_rejects_non_finite_bounds() {
        let b = Bounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: f64::INFINITY,
            max_y: 20.0,
        };
        assert!(!b.is_finite());
        assert!(b.try_transformed(&Transform2D::IDENTITY).is_none());
    }

    #[test]
    fn transform_components_round_trip() {
        let t = Transform2D::translation(5.0, 10.0).then(&Transform2D::rotation(0.5));
        let back = Transform2D::from_components(t.to_components());
        let p = DVec2::new(1.0, 2.0);
        let original = t.transform_point(p);
        let restored = back.transform_point(p);
        assert!((original - restored).length() < 1e-12);
    }
}

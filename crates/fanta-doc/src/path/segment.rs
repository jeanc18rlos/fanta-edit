//! Path segments.

use glam::DVec2;
use serde::{Deserialize, Serialize};

/// One segment of a path. Coordinates are in the node's local space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PathSegment {
    /// Start a new subpath at `to`. Implicitly closes any open subpath? No —
    /// SVG semantics: a new MoveTo does *not* close the previous subpath.
    Move { to: [f64; 2] },
    /// Straight line from current point to `to`.
    Line { to: [f64; 2] },
    /// Quadratic Bézier with one control point.
    Quad { ctrl: [f64; 2], to: [f64; 2] },
    /// Cubic Bézier with two control points.
    Cubic {
        ctrl1: [f64; 2],
        ctrl2: [f64; 2],
        to: [f64; 2],
    },
    /// Close the current subpath with a straight line back to its start.
    Close,
}

impl PathSegment {
    /// The terminal point of this segment, if it has one.
    pub fn end_point(&self) -> Option<DVec2> {
        match self {
            Self::Move { to }
            | Self::Line { to }
            | Self::Quad { to, .. }
            | Self::Cubic { to, .. } => Some(DVec2::new(to[0], to[1])),
            Self::Close => None,
        }
    }

    /// Apply `f` to every coordinate this segment carries — control points and
    /// the terminal point alike (a `Close` carries none). This is the single
    /// place that knows a segment's point layout, so any point-wise transform
    /// (scale, translate, …) composes on top of it rather than re-matching the
    /// variants.
    pub fn map_points(&mut self, mut f: impl FnMut([f64; 2]) -> [f64; 2]) {
        match self {
            Self::Move { to } | Self::Line { to } => *to = f(*to),
            Self::Quad { ctrl, to } => {
                *ctrl = f(*ctrl);
                *to = f(*to);
            }
            Self::Cubic { ctrl1, ctrl2, to } => {
                *ctrl1 = f(*ctrl1);
                *ctrl2 = f(*ctrl2);
                *to = f(*to);
            }
            Self::Close => {}
        }
    }
}

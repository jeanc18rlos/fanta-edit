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
}

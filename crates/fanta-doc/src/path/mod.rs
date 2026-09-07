//! Vector path data.
//!
//! SVG-style segment list — directly maps to `SkPath::moveTo/lineTo/quadTo/
//! cubicTo/close` in `fanta-render`, and round-trips with SVG `d=` attribute
//! strings on export. No arcs in v0: every arc consumer (Skia, browsers,
//! Figma) approximates them with cubics, so we do that at the producer.
//!
//! For non-trivial editing (vector networks, boolean ops) we'll add a
//! `VectorNetwork` type in a later phase — but a `PathData` always projects to
//! a single closed/open contour set, which is enough for the wedge.

mod arc;
mod data;
mod fill_rule;
mod segment;
mod svg;
#[cfg(test)]
mod tests;

pub use data::PathData;
pub use fill_rule::FillRule;
pub use segment::PathSegment;
pub use svg::SvgPathError;

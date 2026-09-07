//! Co-located unit tests for the auto-layout solver, split by concern out of
//! the former single `tests.rs`. `pub(crate) use super::*;` re-exports the
//! `layout` module surface (the `LayoutTree` trait, `solve_auto_layout`, the
//! private `size` accessors) so every child section sees it via `use super::*;`,
//! exactly as the old single-file `use super::*;` did. [`support`] holds the
//! fixtures shared across more than one section.

pub(crate) use super::*;
// `layout`'s `size` module is private, so it cannot be re-exported directly. The
// placement helpers in `support` reach it as `super::size::…`; mirror that path
// with a thin re-export module exposing its crate-visible accessors.
pub(crate) mod size {
    pub(crate) use super::super::size::*;
}
// Test-construction types the original `tests.rs` pulled in explicitly on top of
// the `super::*` surface.
pub(crate) use crate::color::Color;
pub(crate) use crate::node::{
    AutoLayout, AxisSizing, CounterAlign, GroupNode, LayoutChild, LayoutMode, NodeData, NodeFlags,
    PrimaryAlign, TextAutoResize, TextNode, VectorNode,
};
pub(crate) use crate::transform::Transform2D;

mod support;
pub(crate) use support::*;

mod counter_align;
mod flow_and_transforms;
mod grow;
mod hug;
mod stacking;
mod text;
mod wrapping;

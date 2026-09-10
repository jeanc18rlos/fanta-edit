//! Co-located unit tests for the raster renderer, split by concern to mirror
//! the production submodules. `use crate::raster::*;` (re-exported below as
//! `use super::*;` for the children) exposes the renderer's public surface and
//! the `pub(crate)` internals the tests exercise; [`support`] holds the few
//! fixtures shared across more than one section.

// Re-exported (not a private `use`) so each child section's `use super::*;`
// resolves the renderer surface + `pub(crate)` internals these tests exercise.
pub(crate) use crate::raster::*;

mod support;
pub(crate) use support::*;

mod bench;
mod blend;
mod boolean;
mod cull;
mod diamond_gradient;
mod effects;
mod effects_math;
mod fidelity;
mod image;
mod instance;
mod layer_cache;
mod masks;
mod media;
mod motion;
mod shadows;
mod shapes_basics;
mod shapes_fill_rule;
mod shapes_frame;
mod shapes_per_side_rounded;
mod shapes_stroke;
mod shapes_zero_stroke;
mod text;
mod variables;

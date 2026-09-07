//! Co-located unit tests for refined hit-testing, split by concern out of
//! the former single `mod tests` block. `pub(crate) use super::*;` re-exports
//! the module surface plus the private helpers these tests exercise; [`support`]
//! holds the fixtures shared across more than one section.

// Re-export so every child section sees the production surface + crate internals
// via `use super::*;` (matching the old single `mod tests` `use super::*;`).
pub(crate) use super::*;
// Test-construction types from fanta_doc not used by the production surface.
pub(crate) use fanta_doc::{Color, Doc, GroupNode, IndexKey, Operation, Transform2D, VectorNode};

mod support;
pub(crate) use support::*;

mod active_page;
mod api_precision;
mod boolean;
mod deep_and_marquee;
mod parity;
mod path_contain;
mod transforms;

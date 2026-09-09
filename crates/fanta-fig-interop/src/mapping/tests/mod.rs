//! Tests for the `.fig` → Doc mapping, split by concern out of the former
//! single `mapping_tests.rs` `include!` block. Shared fixtures/builders live in
//! [`support`]; each concern module reaches them — and the mapping internals —
//! through `use super::*;`.

// Re-export the mapping internals + the shared test builders so every child
// test module sees them via `use super::*;` (matching the old single-`tests`
// module's `use super::*;` scope).
pub use super::*;
pub use support::*;

mod support;

mod basics;
mod components;
mod derived_and_text;
mod gaps_and_layout;
mod gradients;
mod image_fills;
mod instance_overrides_field;
mod instance_overrides_prop_assign;
mod instance_overrides_root_duality;
mod instance_overrides_routing;
mod instance_overrides_schema;
mod instance_overrides_variant_placeholder;
mod large_documents;
mod motion;
mod prototype;
mod shared_style;
mod sibling_order;
mod skip;
mod strokes_effects;
mod variables;
mod vector_geometry;
mod vector_network;

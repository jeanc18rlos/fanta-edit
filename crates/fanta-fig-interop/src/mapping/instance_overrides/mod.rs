//! Instance override resolution (pass 4): symbolOverrides, prop assignments,
//! derivedSymbolData, nested-instance routing, and instance swaps.

// `super` for these children is THIS module, so pull the grandparent (`mapping`)
// glob down one level — that is where the shared external types and sibling
// helpers live — then each child resolves them via its own `use super::*;`.
pub(crate) use super::*;

mod apply;
mod derived;
mod paths;
mod prop_values;
mod variant_visibility;

pub(crate) use apply::*;
pub(crate) use derived::*;
pub(crate) use paths::*;
pub(crate) use prop_values::*;
pub(crate) use variant_visibility::*;

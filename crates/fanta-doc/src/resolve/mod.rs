//! Render-time resolution shared by `fanta-render` and `fanta-export`.
//!
//! Two pure, renderer-agnostic jobs live here so render and export resolve
//! identically (no drift between what you see and what you export):
//!
//! 1. **Variable resolution** ([`variables`]) — [`resolve_effective_mode`] picks
//!    the mode in force for a collection at a given node (nearest-ancestor frame
//!    pin → `Doc.active_modes` → collection default), and [`resolve_bound_value`]
//!    chases a bound variable (across collections, with a cycle guard) down to a
//!    concrete [`ResolvedVarValue`](crate::value::ResolvedVarValue).
//! 2. **Instance expansion** ([`instance`]) — [`expand_instance`] deep-clones a
//!    component master's subtree with fresh ids, applies the instance's
//!    overrides, and records each clone's def-local path back to the master (so
//!    the editor can map a clicked descendant to the override it would author).
//!    Nested instances are returned *as* instances; the renderer recurses by
//!    calling `expand_instance` again, which keeps this function one level and
//!    lets a swap-override on a nested instance take effect before its own
//!    expansion.
//!
//! Everything here is read-only over the doc (it borrows `&Scene` /
//! `&ComponentLibrary` / `&VariableRegistry`) and allocates fresh transient
//! nodes — it never mutates the document.
//!
//! Thin manifest: the public surface is re-exported here so `crate::resolve::Foo`
//! paths are unchanged.

mod instance;
mod variables;

pub use instance::{ExpandedNode, def_local_path, expand_instance};
pub use variables::{resolve_bound_value, resolve_effective_mode};

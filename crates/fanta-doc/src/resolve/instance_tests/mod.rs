//! Unit tests for instance expansion — clone-and-rewire, sparse override
//! application, nested-instance routing, variant-set selection, and baked
//! `derivedSymbolData` application.
//!
//! A child module of [`crate::resolve::instance`] (`use super::*`), kept in its
//! own file so the implementation module stays focused.

// Split out of the former single inline test module by concern. This thin root
// re-exports the production surface + the test-construction imports so each
// child section (`use super::*;`) sees exactly what the old `mod tests` did.
pub(crate) use super::*;
pub(crate) use crate::color::Color;
pub(crate) use crate::component::ComponentDef;
pub(crate) use crate::id::ComponentId;
pub(crate) use crate::node::{GroupNode, Override, TextNode, VectorNode};
pub(crate) use smallvec::smallvec;
pub(crate) use std::collections::BTreeMap;

mod support;
pub(crate) use support::*;

mod clone_and_text;
mod derived;
mod field_overrides;
mod nested;
mod prop_bindings;
mod variants;

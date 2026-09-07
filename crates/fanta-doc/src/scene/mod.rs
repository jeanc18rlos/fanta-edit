//! [`Scene`] — the storage layer for [`CanvasNode`](crate::node::CanvasNode)s.
//!
//! Holds all nodes by [`NodeId`](crate::id::NodeId) plus a maintained child
//! index so traversal, z-ordering, world-bounds, and hit-testing are all cheap.
//!
//! ## Invariants
//!
//! 1. Every node's `parent` (if `Some`) refers to an existing node in this
//!    scene that [`CanvasNode::can_have_children`](crate::node::CanvasNode::can_have_children)
//!    is true for.
//! 2. The tree is acyclic — no node is its own ancestor.
//! 3. The child index agrees with the `parent` field on every node.
//!
//! Mutating methods on [`Scene`] preserve these invariants or return an error.
//! [`Scene::validate`] is a diagnostic that re-checks the invariants from
//! scratch — used by tests and `fanta-format` after a load.
//!
//! ## Module map
//!
//! Thin manifest: [`error`] holds [`SceneError`]; [`graph`] holds the [`Scene`]
//! struct (storage + child index), the structural operations, the
//! [`Scene::validate`] diagnostic, and the [`Descendants`] / [`Ancestors`]
//! iterators; [`geometry`] holds the world-geometry, spatial-index, and
//! hit-testing methods (a second `impl Scene` block over the same fields). The
//! public surface is re-exported here so `crate::scene::Foo` paths are unchanged.

mod error;
mod geometry;
mod graph;

pub use error::SceneError;
pub use graph::{Ancestors, Descendants, Scene};

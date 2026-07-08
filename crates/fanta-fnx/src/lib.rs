//! `fanta-fnx` — the design-as-source language.
//!
//! A `.fnx` file is the readable, React/TSX-style projection of one page or
//! component subtree: each scene node becomes a JSX element whose tag is its
//! `NodeData` type and whose attributes are its serde fields, verbatim, with
//! children nested by hierarchy. Stable identity (`id`) and fractional sibling
//! order (`index`) are lifted out into an `.ids` sidecar so the source stays
//! free of opaque ULIDs while the round-trip stays lossless.
//!
//! The codec is deliberately model-agnostic: it works on `serde_json::Value`
//! (the doc's canonical per-node projection), so it round-trips any current or
//! future node field without per-field maintenance — unknown fields simply ride
//! along as `{json}` attributes.
//!
//! Round-trip contract: for a single-root subtree of node values `n`,
//! `nodes_from_tree(parse_doc(print_doc(tree.root)), tree.sidecar, tree.root_parent)`
//! reproduces `n` exactly (modulo set ordering).

mod api;
mod color;
mod convert;
mod model;
mod parse;
mod print;
mod sugar;

#[cfg(test)]
mod tests;

pub use api::{FnxSidecar, decode_subtree, encode_subtree};
pub use convert::{FnxError, FnxTree, nodes_from_tree, tree_from_nodes};
pub use model::{FnxElement, IdEntry, tag_for_type, type_for_tag};
pub use parse::parse_doc;
pub use print::print_doc;

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
mod canonicalize;
mod color;
mod convert;
mod ir;
mod kind;
mod model;
mod parse;
mod print;
mod refs;
mod source;
mod sugar;

#[cfg(test)]
mod tests;

pub use api::{
    FnxSidecar, decode_subtree, decode_subtree_with, encode_subtree, encode_subtree_with,
    reconcile_sidecar,
};
pub use canonicalize::canonicalize_legacy_source;
pub use convert::{FnxError, FnxTree, nodes_from_tree, tree_from_nodes};
pub use ir::ArtifactIr;
pub use kind::{ArtifactKind, ImportTarget, artifact_file_names, import_allowed};
pub use model::{FnxElement, IdEntry, tag_for_type, type_for_tag};
pub use parse::{parse_doc, parse_doc_with};
pub use print::{print_doc, print_doc_with};
pub use refs::{RefTable, desugar_refs, sugar_refs};
pub use source::FnxSourceMirror;

//! High-level codec: a page/component subtree's node values ⇄ a `.fnx` source
//! string + an `.ids` sidecar. This is the seam `fanta-format` plugs into,
//! replacing one-JSON-file-per-node with one readable `.fnx` (+ sidecar) per
//! page/component.

use crate::convert::{FnxError, nodes_from_tree, tree_from_nodes};
use crate::model::IdEntry;
use crate::parse::parse_doc;
use crate::print::print_doc;
use serde_json::Value;

/// The companion sidecar for one `.fnx` file: the root's external parent id
/// (so the subtree re-links into the wider scene) and the pre-order id/index of
/// every element (the identity + sibling order the readable source omits).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FnxSidecar {
    /// The id this subtree's root node's `parent` points at, or `None` for a
    /// true scene root (a page). Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_parent: Option<String>,
    /// One entry per element, in the same pre-order the `.fnx` is written/read.
    pub ids: Vec<IdEntry>,
}

/// Encode one single-root subtree (a slice of node JSON values) into `.fnx`
/// source + its sidecar. `fn_name` is the cosmetic function name.
pub fn encode_subtree(nodes: &[Value], fn_name: &str) -> Result<(String, FnxSidecar), FnxError> {
    let tree = tree_from_nodes(nodes)?;
    let text = print_doc(fn_name, &tree.root);
    Ok((
        text,
        FnxSidecar {
            root_parent: tree.root_parent,
            ids: tree.sidecar,
        },
    ))
}

/// Decode a `.fnx` source string + its sidecar back into the flat node values.
pub fn decode_subtree(text: &str, sidecar: &FnxSidecar) -> Result<Vec<Value>, FnxError> {
    let root = parse_doc(text)?;
    nodes_from_tree(&root, &sidecar.ids, sidecar.root_parent.as_deref())
}

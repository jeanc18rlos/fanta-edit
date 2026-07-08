//! The `.fnx` element model and the `NodeData::type` ↔ JSX tag bijection.
//!
//! An [`FnxElement`] is the readable shape of one scene node: its element tag
//! (derived 1:1 from the node's `type`), its attributes (every other serde
//! field of the node's JSON, verbatim), and its nested children. Node identity
//! (`id`) and sibling order (`index`) are NOT attributes — they live in the
//! [`IdEntry`] sidecar so the source file stays free of opaque ULIDs while the
//! round-trip stays lossless.

use serde_json::Value;
use std::collections::BTreeMap;

/// One node, as it appears in a `.fnx` source tree.
#[derive(Debug, Clone, PartialEq)]
pub struct FnxElement {
    /// JSX tag — a 1:1 projection of the node's `NodeData` type tag.
    pub tag: String,
    /// Every node field except `type`/`id`/`parent`/`index`, verbatim JSON.
    pub attrs: BTreeMap<String, Value>,
    /// Child elements, in render (z) order.
    pub children: Vec<FnxElement>,
}

impl FnxElement {
    pub fn new(tag: impl Into<String>) -> Self {
        Self {
            tag: tag.into(),
            attrs: BTreeMap::new(),
            children: Vec::new(),
        }
    }
}

/// Sidecar entry pinning one element (in pre-order) to its stable identity and
/// fractional sibling order — the bits a readable source file deliberately
/// omits but that must round-trip exactly.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdEntry {
    /// The scene node id (bare ULID, as it serializes inside the doc JSON).
    pub id: String,
    /// The node's `index` (fractional `IndexKey`), preserved verbatim. Omitted
    /// when absent so a defaulted index round-trips to absence.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub index: Value,
}

/// The fixed `NodeData` type ↔ JSX tag mapping. Every current `NodeData`
/// variant has exactly one tag; the mapping is total and invertible so a tag
/// alone recovers the node type losslessly.
const TYPE_TAGS: &[(&str, &str)] = &[
    ("group", "Frame"),
    ("vector", "Vector"),
    ("text", "Text"),
    ("bitmap", "Image"),
    ("video", "Video"),
    ("audio", "Audio"),
    ("node_graph", "NodeGraph"),
    ("model3d", "Model3D"),
    ("ai_artifact", "AiArtifact"),
    ("instance", "Instance"),
    ("embed", "Embed"),
];

/// JSX tag for a `NodeData` `type` string (e.g. `"group"` → `"Frame"`).
pub fn tag_for_type(ty: &str) -> Option<&'static str> {
    TYPE_TAGS
        .iter()
        .find(|(t, _)| *t == ty)
        .map(|(_, tag)| *tag)
}

/// `NodeData` `type` string for a JSX tag (e.g. `"Frame"` → `"group"`).
pub fn type_for_tag(tag: &str) -> Option<&'static str> {
    TYPE_TAGS.iter().find(|(_, t)| *t == tag).map(|(ty, _)| *ty)
}

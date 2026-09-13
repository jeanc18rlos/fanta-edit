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
    /// Structural fingerprint: the element's JSX tag at encode time. Optional
    /// so sidecars written before fingerprints existed still parse; a `None`
    /// tag marks the whole fingerprint as unknown, and reconciliation treats
    /// the entry as matching any element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// Structural fingerprint: the element's `name` attribute at encode time
    /// (`None` when the element has no string `name`). Only meaningful when
    /// `tag` is present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Structural fingerprint: the pre-order position of this element's parent
    /// at encode time (`None` for the subtree root). Optional like `tag`/`name`
    /// so sidecars written before this fingerprint existed still parse;
    /// reconciliation treats an absent value as matching any shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<u32>,
}

/// The fixed `NodeData` type ↔ JSX tag mapping. Every current `NodeData`
/// variant has exactly one tag; the mapping is total and invertible so a tag
/// alone recovers the node type losslessly.
const TYPE_TAGS: &[(&str, &str)] = &[
    ("group", "Frame"),
    ("vector", "Vector"),
    ("text", "Text"),
    ("text_path", "TextPath"),
    ("bitmap", "Image"),
    ("video", "Video"),
    ("audio", "Audio"),
    ("node_graph", "NodeGraph"),
    ("model3d", "Model3D"),
    ("ai_artifact", "AiArtifact"),
    ("instance", "Instance"),
    ("boolean", "Boolean"),
    ("embed", "Embed"),
];

/// Authoring-sugar tags accepted at the text boundary only. Deliberately a
/// SEPARATE table from [`TYPE_TAGS`]: that mapping must stay a total, invertible
/// bijection with `NodeData` (a tag alone recovers a node type), while a sugar
/// tag never names a node type at all — `<Rect>`/`<Ellipse>` desugar into a
/// canonical `Vector` during parse (see [`crate::sugar::desugar_shapes`]), so
/// no post-parse tree, sidecar fingerprint, or decoded node ever observes one.
pub(crate) const SUGAR_TAGS: &[&str] = &["Rect", "Ellipse"];

/// Whether `tag` is a tag the FNX language understands: a canonical
/// [`TYPE_TAGS`] projection or a parse-time [`SUGAR_TAGS`] spelling. The source
/// canonicalizer keys on this so a hand-added `<Rect …>` span is treated as an
/// FNX element (looks-like-fnx detection, color rewrites inside its braced
/// attributes) even though no node type carries that tag.
pub(crate) fn is_known_tag(tag: &str) -> bool {
    type_for_tag(tag).is_some() || SUGAR_TAGS.contains(&tag)
}

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

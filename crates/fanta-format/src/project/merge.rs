//! Three-way document merge for concurrent code/canvas editing.
//!
//! The editor's canvas and text agents edit the same project tree
//! concurrently: the canvas holds an in-memory `Doc` while agents rewrite
//! `.fnx` files on disk. When both sides diverge from a common base, this
//! module merges them at the document level instead of forcing the binary
//! Overwrite/Discard choice.
//!
//! The merge is a generic recursive three-way over the docs' serde JSON:
//! whichever side changed a value relative to the base wins; when both sides
//! changed the same value to the same thing it is taken once; when both
//! changed it differently, objects recurse per key (so two edits to
//! *different fields of the same node* still merge) and anything else is a
//! conflict. `scene.nodes` is an id-keyed object, so this yields node-level
//! (and within a node, field-level) granularity without any schema-specific
//! code.
//!
//! Presence state (`selection`, `viewport`, `active_page`, `history`) never
//! merges — the local editor's state always wins and is never a conflict.

use crate::error::{FormatError, Result};
use fanta_doc::Doc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Doc top-level keys that are editor presence state, not document content.
/// The local (ours) side keeps them verbatim.
const PRESENCE_KEYS: [&str; 4] = ["selection", "viewport", "active_page", "history"];

/// The outcome of a three-way merge: the merged document (valid and
/// reassembled) and the JSON paths where both sides changed the same value
/// differently. On a conflicting path the merged doc carries the local
/// (ours) value, so the caller can still adopt the merge or surface the
/// paths for resolution.
pub struct DocMerge {
    pub doc: Doc,
    pub conflicts: Vec<String>,
}

impl DocMerge {
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

/// Three-way merge `ours` (the local editor state) and `theirs` (the state
/// on disk) against their common `base` (the last state both sides agreed
/// on: the doc as of the last load/save). Fails only when the merged JSON no
/// longer forms a valid document (e.g. one side deleted a subtree the other
/// side reparented into) — callers should treat that failure like a
/// conflict.
pub fn merge_docs(base: &Doc, ours: &Doc, theirs: &Doc) -> Result<DocMerge> {
    let base = serde_json::to_value(base)?;
    let ours = serde_json::to_value(ours)?;
    let theirs = serde_json::to_value(theirs)?;

    let mut conflicts = Vec::new();
    let mut merged = merge_value(
        Some(&base),
        Some(&ours),
        Some(&theirs),
        &mut String::new(),
        &mut conflicts,
    )
    .unwrap_or_else(|| ours.clone());

    if let (Value::Object(merged), Value::Object(ours)) = (&mut merged, &ours) {
        for key in PRESENCE_KEYS {
            match ours.get(key) {
                Some(value) => {
                    merged.insert(key.to_owned(), value.clone());
                }
                None => {
                    merged.remove(key);
                }
            }
        }
        // Every operation bumps `metadata.modified_at`, so any two sides that
        // both carry edits disagree on it — a bookkeeping conflict that would
        // defeat the merge exactly in the concurrent-editing case it exists
        // for. Resolve it to the newer timestamp.
        let latest_modification = ours
            .get("metadata")
            .and_then(|metadata| metadata.get("modified_at"))
            .and_then(Value::as_i64)
            .into_iter()
            .chain(
                theirs
                    .get("metadata")
                    .and_then(|metadata| metadata.get("modified_at"))
                    .and_then(Value::as_i64),
            )
            .max();
        if let (Some(latest), Some(Value::Object(metadata))) =
            (latest_modification, merged.get_mut("metadata"))
        {
            metadata.insert("modified_at".to_owned(), Value::from(latest));
        }
        // The undo stack's ops carry whole-node snapshots from BEFORE the
        // merge; undoing one after adoption would re-install a snapshot that
        // silently reverts the other side's merged-in edits (and may
        // reference subtrees the other side deleted). Until history is
        // rebased across merges, dropping it is the only safe choice: the
        // merge is an undo barrier.
        merged.remove("history");
    }
    conflicts.retain(|path| {
        path != "metadata/modified_at"
            && !PRESENCE_KEYS
                .iter()
                .any(|key| path == key || path.starts_with(&format!("{key}/")))
    });

    let doc = Doc::from_json_str(&merged.to_string())
        .map_err(|e| FormatError::InvalidProjectTree(format!("merged doc failed to load: {e}")))?;
    Ok(DocMerge { doc, conflicts })
}

/// Result of merging one artifact's node-map edition (design N2 substrate).
#[derive(Debug, Clone)]
pub struct ArtifactMerge {
    pub header: Value,
    pub nodes: Map<String, Value>,
    /// Structured conflicts suitable for one-property-at-a-time review.
    ///
    /// Unlike [`Self::conflicts`], keys are retained as path segments, so a
    /// node id or property name containing `/` is not ambiguous. Each
    /// conflict also retains presence separately from the JSON value, which
    /// distinguishes a missing property from a property explicitly set to
    /// `null`.
    pub property_conflicts: Vec<PropertyConflict>,
    /// Backwards-compatible display paths for existing callers.
    ///
    /// New code should prefer [`Self::property_conflicts`].
    pub conflicts: Vec<String>,
}

impl ArtifactMerge {
    pub fn is_clean(&self) -> bool {
        self.property_conflicts.is_empty()
    }
}

/// One segment in a structured JSON address.
///
/// Artifact merge currently treats arrays atomically, so it only emits
/// [`Self::Key`] segments. `Index` is included in the address model so future
/// schema-aware array merge policies do not need to change the public type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum JsonPathSegment {
    Key(String),
    Index(usize),
}

/// An unambiguous location in an artifact node-map edition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum ArtifactAddress {
    Artifact {
        path: Vec<JsonPathSegment>,
    },
    Header {
        path: Vec<JsonPathSegment>,
    },
    Node {
        id: String,
        path: Vec<JsonPathSegment>,
    },
}

impl ArtifactAddress {
    /// Legacy slash-separated path used by pre-review callers and diagnostics.
    ///
    /// This representation is intentionally display-only: keys containing
    /// `/` cannot be round-tripped. Use the structured address for identity.
    pub fn legacy_path(&self) -> String {
        let (prefix, path) = match self {
            Self::Artifact { path } => ("(artifact)".to_owned(), path),
            Self::Header { path } => ("header".to_owned(), path),
            Self::Node { id, path } => (format!("nodes/{id}"), path),
        };
        path.iter().fold(prefix, |mut rendered, segment| {
            rendered.push('/');
            match segment {
                JsonPathSegment::Key(key) => rendered.push_str(key),
                JsonPathSegment::Index(index) => rendered.push_str(&index.to_string()),
            }
            rendered
        })
    }
}

/// Presence-aware JSON value used by three-way conflict review.
///
/// `Present(Value::Null)` is deliberately distinct from `Missing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
pub enum PresenceValue {
    Missing,
    Present(Value),
}

impl PresenceValue {
    fn from_option(value: Option<&Value>) -> Self {
        match value {
            Some(value) => Self::Present(value.clone()),
            None => Self::Missing,
        }
    }
}

/// A leaf (or deliberately atomic subtree) changed differently on both sides.
///
/// Conflict order is deterministic because the merge walk visits sorted
/// object keys. `address` is the stable review identity within editions with
/// the same base/ours/theirs inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyConflict {
    pub address: ArtifactAddress,
    pub base: PresenceValue,
    pub ours: PresenceValue,
    pub theirs: PresenceValue,
}

/// Three-way merge over id-keyed node maps + header JSON (not IR trees).
///
/// Substrate shape:
/// ```json
/// { "header": {…}, "nodes": { "<id>": {…} } }
/// ```
/// Same recursive rules as [`merge_docs`]; on conflict `ours` wins.
pub fn merge_artifact(
    base: &NodeMapEdition,
    ours: &NodeMapEdition,
    theirs: &NodeMapEdition,
) -> ArtifactMerge {
    let base_v = node_map_to_value(base);
    let ours_v = node_map_to_value(ours);
    let theirs_v = node_map_to_value(theirs);
    let mut raw_conflicts = Vec::new();
    let merged = merge_value_structured(
        Some(&base_v),
        Some(&ours_v),
        Some(&theirs_v),
        &mut Vec::new(),
        &mut raw_conflicts,
    )
    .unwrap_or_else(|| ours_v.clone());
    let (header, nodes) =
        value_to_node_map(&merged).unwrap_or_else(|| (ours.header.clone(), ours.nodes.clone()));
    let property_conflicts: Vec<PropertyConflict> = raw_conflicts
        .into_iter()
        .map(RawConflict::into_artifact_conflict)
        .collect();
    let conflicts = property_conflicts
        .iter()
        .map(|conflict| conflict.address.legacy_path())
        .collect();
    ArtifactMerge {
        header,
        nodes,
        property_conflicts,
        conflicts,
    }
}

/// Id-keyed node map + header for one design artifact (merge / base snapshot).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMapEdition {
    pub header: Value,
    pub nodes: Map<String, Value>,
}

impl NodeMapEdition {
    pub fn new(header: Value, nodes: Map<String, Value>) -> Self {
        Self { header, nodes }
    }

    pub fn empty() -> Self {
        Self {
            header: Value::Object(Map::new()),
            nodes: Map::new(),
        }
    }
}

fn node_map_to_value(edition: &NodeMapEdition) -> Value {
    json!({
        "header": edition.header,
        "nodes": edition.nodes,
    })
}

fn value_to_node_map(value: &Value) -> Option<(Value, Map<String, Value>)> {
    let obj = value.as_object()?;
    let header = obj.get("header").cloned().unwrap_or(Value::Null);
    let nodes = obj
        .get("nodes")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Some((header, nodes))
}

/// Generic recursive three-way merge of one JSON value. `None` means "absent
/// on that side" so insertions and deletions merge with the same rules as
/// modifications. Returns the merged value (`None` = absent in the merge)
/// and records conflict paths; on conflict `ours` wins.
pub(crate) fn merge_value(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    path: &mut String,
    conflicts: &mut Vec<String>,
) -> Option<Value> {
    if ours == theirs {
        return ours.cloned();
    }
    if ours == base {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }
    // Both sides changed, differently. Objects merge per key; everything
    // else is an atomic conflict.
    if let (Some(Value::Object(ours_map)), Some(Value::Object(theirs_map))) = (ours, theirs) {
        let base_map = base.and_then(Value::as_object);
        let empty = Map::new();
        let base_map = base_map.unwrap_or(&empty);
        let mut merged = Map::new();
        let mut keys: Vec<&String> = ours_map.keys().chain(theirs_map.keys()).collect();
        if let Some(base) = base.and_then(Value::as_object) {
            keys.extend(base.keys());
        }
        keys.sort();
        keys.dedup();
        for key in keys {
            let child_path_len = path.len();
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(key);
            let value = merge_value(
                base_map.get(key),
                ours_map.get(key),
                theirs_map.get(key),
                path,
                conflicts,
            );
            path.truncate(child_path_len);
            if let Some(value) = value {
                merged.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(merged));
    }
    conflicts.push(if path.is_empty() {
        "(document)".to_owned()
    } else {
        path.clone()
    });
    ours.cloned()
}

#[derive(Debug)]
struct RawConflict {
    path: Vec<JsonPathSegment>,
    base: PresenceValue,
    ours: PresenceValue,
    theirs: PresenceValue,
}

impl RawConflict {
    fn new(
        path: &[JsonPathSegment],
        base: Option<&Value>,
        ours: Option<&Value>,
        theirs: Option<&Value>,
    ) -> Self {
        Self {
            path: path.to_vec(),
            base: PresenceValue::from_option(base),
            ours: PresenceValue::from_option(ours),
            theirs: PresenceValue::from_option(theirs),
        }
    }

    fn into_artifact_conflict(self) -> PropertyConflict {
        let mut segments = self.path.into_iter();
        let address = match segments.next() {
            Some(JsonPathSegment::Key(scope)) if scope == "header" => ArtifactAddress::Header {
                path: segments.collect(),
            },
            Some(JsonPathSegment::Key(scope)) if scope == "nodes" => match segments.next() {
                Some(JsonPathSegment::Key(id)) => ArtifactAddress::Node {
                    id,
                    path: segments.collect(),
                },
                unexpected => ArtifactAddress::Artifact {
                    path: std::iter::once(JsonPathSegment::Key(scope))
                        .chain(unexpected)
                        .chain(segments)
                        .collect(),
                },
            },
            unexpected => ArtifactAddress::Artifact {
                path: unexpected.into_iter().chain(segments).collect(),
            },
        };
        PropertyConflict {
            address,
            base: self.base,
            ours: self.ours,
            theirs: self.theirs,
        }
    }
}

/// Structured counterpart of [`merge_value`].
///
/// Arrays are deliberately atomic. Objects recurse only when the base was
/// also an object. In particular, two different objects concurrently created
/// at the same absent key conflict as a whole: recursively combining their
/// fields would invent a node that neither side created.
fn merge_value_structured(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
    path: &mut Vec<JsonPathSegment>,
    conflicts: &mut Vec<RawConflict>,
) -> Option<Value> {
    if ours == theirs {
        return ours.cloned();
    }
    if ours == base {
        return theirs.cloned();
    }
    if theirs == base {
        return ours.cloned();
    }

    if let (
        Some(Value::Object(base_map)),
        Some(Value::Object(ours_map)),
        Some(Value::Object(theirs_map)),
    ) = (base, ours, theirs)
    {
        let mut merged = Map::new();
        let mut keys: Vec<&String> = ours_map
            .keys()
            .chain(theirs_map.keys())
            .chain(base_map.keys())
            .collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            path.push(JsonPathSegment::Key(key.clone()));
            let value = merge_value_structured(
                base_map.get(key),
                ours_map.get(key),
                theirs_map.get(key),
                path,
                conflicts,
            );
            path.pop();
            if let Some(value) = value {
                merged.insert(key.clone(), value);
            }
        }
        return Some(Value::Object(merged));
    }

    conflicts.push(RawConflict::new(path, base, ours, theirs));
    ours.cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, Doc, NodeData, NodeId};
    use serde_json::json;

    fn text_node(content: &str) -> CanvasNode {
        let data = serde_json::from_value(json!({
            "content": content,
            "local_size": [100.0, 40.0],
        }))
        .expect("text node from JSON");
        CanvasNode::new(NodeData::Text(data))
    }

    fn doc_with_two_texts() -> (Doc, NodeId, NodeId) {
        let mut doc = Doc::new();
        let a = doc.scene.insert(text_node("Alpha")).expect("insert");
        let b = doc.scene.insert(text_node("Beta")).expect("insert");
        (doc, a, b)
    }

    fn set_text(doc: &mut Doc, id: NodeId, content: &str) {
        let node = doc.scene.get_mut(id).unwrap();
        match &mut node.data {
            NodeData::Text(text) => text.content = content.to_owned(),
            _ => panic!("not a text node"),
        }
    }

    #[test]
    fn disjoint_node_edits_merge_cleanly() {
        let (base, a, b) = doc_with_two_texts();
        let mut ours = base.clone();
        set_text(&mut ours, a, "Alpha (canvas)");
        let mut theirs = base.clone();
        set_text(&mut theirs, b, "Beta (agent)");

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        assert!(merge.is_clean(), "conflicts: {:?}", merge.conflicts);
        let content = |doc: &Doc, id| match &doc.scene.get(id).unwrap().data {
            NodeData::Text(text) => text.content.clone(),
            _ => unreachable!(),
        };
        assert_eq!(content(&merge.doc, a), "Alpha (canvas)");
        assert_eq!(content(&merge.doc, b), "Beta (agent)");
    }

    #[test]
    fn different_fields_of_the_same_node_merge_cleanly() {
        let (base, a, _) = doc_with_two_texts();
        let mut ours = base.clone();
        set_text(&mut ours, a, "Renamed by canvas");
        let mut theirs = base.clone();
        theirs.scene.get_mut(a).unwrap().name = "renamed-by-agent".into();

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        assert!(merge.is_clean(), "conflicts: {:?}", merge.conflicts);
        let node = merge.doc.scene.get(a).unwrap();
        assert_eq!(node.name, "renamed-by-agent");
        match &node.data {
            NodeData::Text(text) => assert_eq!(text.content, "Renamed by canvas"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn same_field_edited_differently_is_a_conflict_and_ours_wins() {
        let (base, a, _) = doc_with_two_texts();
        let mut ours = base.clone();
        set_text(&mut ours, a, "canvas version");
        let mut theirs = base.clone();
        set_text(&mut theirs, a, "agent version");

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        assert!(!merge.is_clean());
        assert!(
            merge.conflicts.iter().any(|path| path.contains("content")),
            "conflicts: {:?}",
            merge.conflicts
        );
        match &merge.doc.scene.get(a).unwrap().data {
            NodeData::Text(text) => assert_eq!(text.content, "canvas version"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn node_added_on_disk_survives_the_merge() {
        let (base, a, _) = doc_with_two_texts();
        let mut ours = base.clone();
        set_text(&mut ours, a, "edited locally");
        let mut theirs = base.clone();
        let added = theirs
            .scene
            .insert(text_node("Fresh from agent"))
            .expect("insert");

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        assert!(merge.is_clean(), "conflicts: {:?}", merge.conflicts);
        assert!(merge.doc.scene.get(added).is_some());
        assert!(merge.doc.scene.get(a).is_some());
    }

    #[test]
    fn concurrent_timestamps_resolve_to_the_newest_and_history_is_a_barrier() {
        let (base, a, b) = doc_with_two_texts();
        let mut ours = base.clone();
        set_text(&mut ours, a, "ours edit");
        ours.metadata.modified_at = 1_000;
        let mut theirs = base.clone();
        set_text(&mut theirs, b, "theirs edit");
        theirs.metadata.modified_at = 2_000;

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        // Both sides bumped modified_at; that must never be a conflict.
        assert!(merge.is_clean(), "conflicts: {:?}", merge.conflicts);
        assert_eq!(merge.doc.metadata.modified_at, 2_000);
        // Ours' undo stack carries pre-merge whole-node snapshots; undoing
        // one would revert theirs' merged-in edits, so the merge clears it.
        assert!(!merge.doc.history.can_undo());
    }

    #[test]
    fn presence_state_never_conflicts_and_ours_is_kept() {
        let (base, a, b) = doc_with_two_texts();
        let mut ours = base.clone();
        ours.selection.replace_with(vec![a]);
        let mut theirs = base.clone();
        theirs.selection.replace_with(vec![b]);
        set_text(&mut theirs, b, "changed on disk");

        let merge = merge_docs(&base, &ours, &theirs).unwrap();
        assert!(merge.is_clean(), "conflicts: {:?}", merge.conflicts);
        assert_eq!(merge.doc.selection.as_slice(), &[a]);
    }

    fn node_map(
        header: Value,
        nodes: impl IntoIterator<Item = (&'static str, Value)>,
    ) -> NodeMapEdition {
        NodeMapEdition::new(
            header,
            nodes
                .into_iter()
                .map(|(id, value)| (id.to_owned(), value))
                .collect(),
        )
    }

    #[test]
    fn artifact_conflict_has_unambiguous_address_and_three_editions() {
        let id = "node/whose/id/contains/slashes";
        let base = node_map(json!({}), [(id, json!({"fill": "black"}))]);
        let ours = node_map(json!({}), [(id, json!({"fill": "red"}))]);
        let theirs = node_map(json!({}), [(id, json!({"fill": "blue"}))]);

        let merge = merge_artifact(&base, &ours, &theirs);

        assert_eq!(
            merge.property_conflicts,
            vec![PropertyConflict {
                address: ArtifactAddress::Node {
                    id: id.to_owned(),
                    path: vec![JsonPathSegment::Key("fill".to_owned())],
                },
                base: PresenceValue::Present(json!("black")),
                ours: PresenceValue::Present(json!("red")),
                theirs: PresenceValue::Present(json!("blue")),
            }]
        );
        // Retained only for compatibility; structured addresses are the
        // unambiguous review identity.
        assert_eq!(
            merge.conflicts,
            vec!["nodes/node/whose/id/contains/slashes/fill"]
        );
    }

    #[test]
    fn artifact_conflict_distinguishes_missing_from_explicit_null() {
        let base = node_map(json!({}), [("n", json!({"value": "base"}))]);
        let ours = node_map(json!({}), [("n", json!({}))]);
        let theirs = node_map(json!({}), [("n", json!({"value": null}))]);

        let merge = merge_artifact(&base, &ours, &theirs);
        let conflict = merge.property_conflicts.first().expect("one conflict");

        assert_eq!(
            conflict.address,
            ArtifactAddress::Node {
                id: "n".to_owned(),
                path: vec![JsonPathSegment::Key("value".to_owned())],
            }
        );
        assert_eq!(conflict.base, PresenceValue::Present(json!("base")));
        assert_eq!(conflict.ours, PresenceValue::Missing);
        assert_eq!(conflict.theirs, PresenceValue::Present(Value::Null));
        assert_eq!(merge.nodes["n"], json!({}), "ours wins the conflict");
    }

    #[test]
    fn artifact_arrays_are_atomic_and_conflicts_are_deterministically_sorted() {
        let base = node_map(json!({}), [("n", json!({"z": [1, 2], "a": 0, "b": 0}))]);
        let ours = node_map(json!({}), [("n", json!({"z": [3, 2], "a": 1, "b": 1}))]);
        let theirs = node_map(json!({}), [("n", json!({"z": [1, 4], "a": 2, "b": 2}))]);

        let merge = merge_artifact(&base, &ours, &theirs);
        let addresses: Vec<_> = merge
            .property_conflicts
            .iter()
            .map(|conflict| conflict.address.clone())
            .collect();

        assert_eq!(
            addresses,
            ["a", "b", "z"].map(|key| ArtifactAddress::Node {
                id: "n".to_owned(),
                path: vec![JsonPathSegment::Key(key.to_owned())],
            })
        );
        assert_eq!(merge.nodes["n"]["z"], json!([3, 2]));
        assert!(merge.property_conflicts.iter().all(|conflict| {
            matches!(
                &conflict.address,
                ArtifactAddress::Node { path, .. }
                    if !path.iter().any(|part| matches!(part, JsonPathSegment::Index(_)))
            )
        }));
    }

    #[test]
    fn concurrent_different_creation_of_same_node_id_is_atomic() {
        let base = NodeMapEdition::empty();
        let ours_node = json!({"name": "ours", "only_ours": true});
        let theirs_node = json!({"name": "theirs", "only_theirs": true});
        let ours = node_map(json!({}), [("same-id", ours_node.clone())]);
        let theirs = node_map(json!({}), [("same-id", theirs_node)]);

        let merge = merge_artifact(&base, &ours, &theirs);

        assert_eq!(merge.nodes["same-id"], ours_node);
        assert_eq!(
            merge.property_conflicts,
            vec![PropertyConflict {
                address: ArtifactAddress::Node {
                    id: "same-id".to_owned(),
                    path: vec![],
                },
                base: PresenceValue::Missing,
                ours: PresenceValue::Present(json!({"name": "ours", "only_ours": true})),
                theirs: PresenceValue::Present(json!({"name": "theirs", "only_theirs": true})),
            }]
        );
        assert!(
            merge.nodes["same-id"].get("only_theirs").is_none(),
            "must not synthesize a node neither side created"
        );
    }

    #[test]
    fn identical_concurrent_creation_of_same_node_id_is_clean() {
        let base = NodeMapEdition::empty();
        let created = json!({"name": "same"});
        let ours = node_map(json!({}), [("same-id", created.clone())]);
        let theirs = node_map(json!({}), [("same-id", created)]);

        let merge = merge_artifact(&base, &ours, &theirs);

        assert!(merge.is_clean());
        assert!(merge.conflicts.is_empty());
    }
}

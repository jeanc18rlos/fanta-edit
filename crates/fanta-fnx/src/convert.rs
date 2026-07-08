//! Convert between a flat slice of node JSON (one page/component subtree) and an
//! [`FnxElement`] tree + [`IdEntry`] sidecar.
//!
//! Losslessness rests on the doc model: `id`, `parent`, and `index` are always
//! present in a node's serde JSON (no `skip_serializing_if`). So the four
//! structural fields can be reconstructed with their exact serde shape — `type`
//! from the tag, `id`/`index` from the sidecar, `parent` from tree nesting —
//! and every other field is carried verbatim as an attribute.

use crate::model::{FnxElement, IdEntry, tag_for_type, type_for_tag};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Reserved structural keys that are NOT attributes: handled by tag (`type`),
/// sidecar (`id`, `index`), or nesting (`parent`).
const STRUCTURAL: [&str; 4] = ["type", "id", "parent", "index"];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FnxError {
    #[error("node is not a JSON object")]
    NotObject,
    #[error("node missing `type`")]
    MissingType,
    #[error("node missing `id`")]
    MissingId,
    #[error("unknown node type `{0}`")]
    UnknownType(String),
    #[error("unknown JSX tag `{0}`")]
    UnknownTag(String),
    #[error("subtree has no single root (found {0} roots)")]
    NotSingleRoot(usize),
    #[error("sidecar has {sidecar} entries but the tree has {elements}")]
    SidecarMismatch { sidecar: usize, elements: usize },
    #[error("parse error: {0}")]
    Parse(String),
}

/// A single-root subtree decomposed into its readable element tree, the
/// pre-order id/index sidecar, and the root's external parent id (so the tree
/// re-links into the wider scene on load).
#[derive(Debug, Clone, PartialEq)]
pub struct FnxTree {
    pub root: FnxElement,
    pub sidecar: Vec<IdEntry>,
    pub root_parent: Option<String>,
}

/// One node object → an attribute-only [`FnxElement`] (children attached by the
/// caller). Drops the four structural keys.
fn element_of(node: &Value) -> Result<FnxElement, FnxError> {
    let obj = node.as_object().ok_or(FnxError::NotObject)?;
    let ty = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or(FnxError::MissingType)?;
    let tag = tag_for_type(ty).ok_or_else(|| FnxError::UnknownType(ty.to_owned()))?;
    let mut el = FnxElement::new(tag);
    for (k, v) in obj {
        if !STRUCTURAL.contains(&k.as_str()) {
            el.attrs.insert(k.clone(), v.clone());
        }
    }
    Ok(el)
}

fn node_id(node: &Value) -> Result<&str, FnxError> {
    node.get("id")
        .and_then(Value::as_str)
        .ok_or(FnxError::MissingId)
}

fn index_of(node: &Value) -> f64 {
    node.get("index").and_then(Value::as_f64).unwrap_or(0.0)
}

/// Assemble a single-root [`FnxTree`] from one subtree's node values.
pub fn tree_from_nodes(nodes: &[Value]) -> Result<FnxTree, FnxError> {
    let mut by_id: HashMap<&str, &Value> = HashMap::with_capacity(nodes.len());
    for n in nodes {
        by_id.insert(node_id(n)?, n);
    }

    // Children grouped by in-set parent id; the root is the node whose parent
    // is absent from this subtree.
    let mut kids: HashMap<&str, Vec<&Value>> = HashMap::new();
    let mut roots: Vec<&Value> = Vec::new();
    for n in nodes {
        let parent = n.get("parent").and_then(Value::as_str);
        match parent.filter(|p| by_id.contains_key(*p)) {
            Some(p) => kids.entry(p).or_default().push(n),
            None => roots.push(n),
        }
    }
    if roots.len() != 1 {
        return Err(FnxError::NotSingleRoot(roots.len()));
    }
    let root_node = roots[0];
    let root_parent = root_node
        .get("parent")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let mut sidecar = Vec::with_capacity(nodes.len());
    let root = build(root_node, &kids, &mut sidecar)?;
    Ok(FnxTree {
        root,
        sidecar,
        root_parent,
    })
}

/// Recursively build an element + record its sidecar entry in pre-order, with
/// children sorted by their fractional `index`.
fn build(
    node: &Value,
    kids: &HashMap<&str, Vec<&Value>>,
    sidecar: &mut Vec<IdEntry>,
) -> Result<FnxElement, FnxError> {
    let id = node_id(node)?;
    let mut el = element_of(node)?;
    sidecar.push(IdEntry {
        id: id.to_owned(),
        index: node.get("index").cloned().unwrap_or(Value::Null),
    });
    if let Some(children) = kids.get(id) {
        let mut sorted = children.clone();
        sorted.sort_by(|a, b| {
            index_of(a)
                .partial_cmp(&index_of(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for child in sorted {
            el.children.push(build(child, kids, sidecar)?);
        }
    }
    Ok(el)
}

/// Rebuild the flat node values from a parsed element tree + its sidecar,
/// re-linking `parent` from nesting and `id`/`index` from the sidecar.
pub fn nodes_from_tree(
    root: &FnxElement,
    sidecar: &[IdEntry],
    root_parent: Option<&str>,
) -> Result<Vec<Value>, FnxError> {
    let count = count_elements(root);
    if count != sidecar.len() {
        return Err(FnxError::SidecarMismatch {
            sidecar: sidecar.len(),
            elements: count,
        });
    }
    let mut out = Vec::with_capacity(count);
    let mut cursor = 0usize;
    walk(root, root_parent, sidecar, &mut cursor, &mut out)?;
    Ok(out)
}

fn count_elements(el: &FnxElement) -> usize {
    1 + el.children.iter().map(count_elements).sum::<usize>()
}

fn walk(
    el: &FnxElement,
    parent: Option<&str>,
    sidecar: &[IdEntry],
    cursor: &mut usize,
    out: &mut Vec<Value>,
) -> Result<(), FnxError> {
    let entry = &sidecar[*cursor];
    *cursor += 1;
    out.push(element_to_value(el, &entry.id, parent, &entry.index)?);
    let id = entry.id.clone();
    for child in &el.children {
        walk(child, Some(&id), sidecar, cursor, out)?;
    }
    Ok(())
}

/// One element + its structural facts → a node JSON object, reproducing the
/// exact serde shape (all four structural keys always present).
fn element_to_value(
    el: &FnxElement,
    id: &str,
    parent: Option<&str>,
    index: &Value,
) -> Result<Value, FnxError> {
    let ty = type_for_tag(&el.tag).ok_or_else(|| FnxError::UnknownTag(el.tag.clone()))?;
    let mut obj = Map::new();
    for (k, v) in &el.attrs {
        obj.insert(k.clone(), v.clone());
    }
    obj.insert("type".to_owned(), Value::String(ty.to_owned()));
    obj.insert("id".to_owned(), Value::String(id.to_owned()));
    obj.insert(
        "parent".to_owned(),
        parent.map_or(Value::Null, |p| Value::String(p.to_owned())),
    );
    obj.insert("index".to_owned(), index.clone());
    Ok(Value::Object(obj))
}

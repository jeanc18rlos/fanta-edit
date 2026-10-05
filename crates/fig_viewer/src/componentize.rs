//! `componentize`: turn look-alike layers into one component.
//!
//! The first layer becomes the master. Every other one is replaced, in place,
//! by an instance of it, and wherever a copy differs from the master (its text,
//! a fill, a hidden layer, any other property) the instance carries an
//! override, so nothing on the canvas changes. Copies must match the master's
//! structure: the same kinds of layers in the same order. An override can
//! restyle a layer, but it cannot add or remove one.

use std::collections::BTreeSet;

use anyhow::{Result, bail};
use fanta_doc::{
    BoundProp, CanvasNode, Doc, NodeData, NodeFlags, NodeId, Override, OverridePath, OverrideValue,
};
use serde_json::{Map, Value, json};

/// Keys an override never carries: identity and hierarchy (the expansion owns
/// them), the layer name, and opaque source metadata.
const NEVER_DIFFED: [&str; 7] = ["id", "parent", "index", "type", "name", "meta", "flags"];

/// Keys of a copy's root that live on the instance node rather than in an
/// override: where it sits, how it takes part in its parent's layout, and its
/// box (the instance's `local_size`).
const ROOT_ON_INSTANCE: [&str; 5] = [
    "transform",
    "layout_child",
    "constraints",
    "clip_size",
    "local_size",
];

/// The overrides that make an instance of the master rooted at `master` look
/// like `copy`. Fails, naming the layer, when the two differ in structure.
pub(crate) fn overrides_for_copy(doc: &Doc, master: NodeId, copy: NodeId) -> Result<Vec<Override>> {
    let mut overrides = Vec::new();
    diff_node(doc, master, copy, &OverridePath::new(), &mut overrides)?;
    Ok(overrides)
}

fn diff_node(
    doc: &Doc,
    master_id: NodeId,
    copy_id: NodeId,
    path: &OverridePath,
    out: &mut Vec<Override>,
) -> Result<()> {
    let (Some(master), Some(copy)) = (doc.scene.get(master_id), doc.scene.get(copy_id)) else {
        bail!("node {copy_id} does not exist");
    };
    if master.data.kind_tag() != copy.data.kind_tag() {
        bail!(
            "layer \"{}\" ({copy_id}) is a {} where the master has a {}; \
             a copy must have the master's structure",
            copy.name,
            copy.data.kind_tag(),
            master.data.kind_tag()
        );
    }
    let root = path.is_empty();
    let mut push = |target_prop, value| {
        out.push(Override {
            target_path: path.clone(),
            target_prop,
            value,
        });
    };
    let hidden = copy.flags.contains(NodeFlags::HIDDEN);
    if !root && master.flags.contains(NodeFlags::HIDDEN) != hidden {
        push(
            BoundProp::Visible,
            OverrideValue::Visible { value: !hidden },
        );
    }

    let (Value::Object(master_json), Value::Object(copy_json)) =
        (serde_json::to_value(master)?, serde_json::to_value(copy)?)
    else {
        bail!("layer \"{}\" ({copy_id}) cannot be compared", copy.name);
    };
    let keys: BTreeSet<&String> = master_json.keys().chain(copy_json.keys()).collect();
    let mut patch = Map::new();
    for key in keys {
        let key = key.as_str();
        if NEVER_DIFFED.contains(&key)
            || (root && ROOT_ON_INSTANCE.contains(&key))
            || master_json.get(key) == copy_json.get(key)
        {
            continue;
        }
        match (key, &copy.data) {
            ("content", NodeData::Text(text)) => push(
                BoundProp::TextContent,
                OverrideValue::Text {
                    value: text.content.clone(),
                },
            ),
            ("content", NodeData::TextPath(text)) => push(
                BoundProp::TextContent,
                OverrideValue::Text {
                    value: text.content.clone(),
                },
            ),
            ("fills", NodeData::Vector(vector)) => push(
                BoundProp::FillColor { index: 0 },
                OverrideValue::Fills {
                    fills: vector.fills.clone(),
                },
            ),
            ("strokes", NodeData::Vector(vector)) => push(
                BoundProp::StrokeColor { index: 0 },
                OverrideValue::Strokes {
                    strokes: vector.strokes.clone(),
                },
            ),
            _ => {
                let value = match copy_json.get(key) {
                    Some(value) => value.clone(),
                    None => default_value(&master_json, key).ok_or_else(|| {
                        anyhow::anyhow!(
                            "layer \"{}\" ({copy_id}) differs from the master in `{key}`, \
                             which an override cannot express",
                            copy.name
                        )
                    })?,
                };
                patch.insert(key.to_owned(), value);
            }
        }
    }
    if !patch.is_empty() {
        // The property address is unused for a field override (as on import).
        push(
            BoundProp::Visible,
            OverrideValue::Field {
                value: Value::Object(patch),
            },
        );
    }

    let master_children = doc.scene.children_of(Some(master_id));
    let copy_children = doc.scene.children_of(Some(copy_id));
    if master_children.len() != copy_children.len() {
        bail!(
            "layer \"{}\" ({copy_id}) has {} children where the master has {}; \
             a copy must have the master's structure",
            copy.name,
            copy_children.len(),
            master_children.len()
        );
    }
    for (master_child, copy_child) in master_children.iter().zip(copy_children) {
        let mut child_path = path.clone();
        child_path.push(*master_child);
        diff_node(doc, *master_child, *copy_child, &child_path, out)?;
    }
    Ok(())
}

/// The JSON that resets `key` to its default on a clone of `master_json`. The
/// copy has no `key` because serde skips default values, so this finds the
/// value that deserializes back to that default.
fn default_value(master_json: &Map<String, Value>, key: &str) -> Option<Value> {
    [
        Value::Null,
        json!([]),
        json!(false),
        json!(0),
        json!({}),
        json!(""),
    ]
    .into_iter()
    .find(|candidate| {
        let mut merged = master_json.clone();
        merged.insert(key.to_owned(), candidate.clone());
        serde_json::from_value::<CanvasNode>(Value::Object(merged))
            .ok()
            .and_then(|node| serde_json::to_value(node).ok())
            .is_some_and(|value| value.get(key).is_none())
    })
}

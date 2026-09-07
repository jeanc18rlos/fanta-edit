//! [`ArtifactIr`] — FNX-shaped in-memory tree for one design artifact.
//!
//! Thin wrapper over [`FnxElement`] + [`FnxSidecar`]. There is no parallel
//! schema: serialization-equivalent to what `encode_subtree` / `decode_subtree`
//! already produce.

use crate::api::{FnxSidecar, decode_subtree, encode_subtree};
use crate::convert::{FnxError, element_of, nodes_from_tree, tree_from_nodes};
use crate::kind::ArtifactKind;
use crate::model::FnxElement;
use crate::parse::{parse_doc, parse_doc_with};
use crate::print::{print_doc, print_doc_with};
use crate::refs::RefTable;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// One artifact's readable IR: tag tree + identity sidecar.
#[derive(Debug, Clone)]
pub struct ArtifactIr {
    kind: ArtifactKind,
    /// Cosmetic function name in the printed source.
    fn_name: String,
    root: FnxElement,
    sidecar: FnxSidecar,
    index: Arc<ElementIndex>,
}

impl PartialEq for ArtifactIr {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.fn_name == other.fn_name
            && self.root == other.root
            && self.sidecar == other.sidecar
    }
}

/// Compact pre-order lookup metadata.
///
/// Storing a full tree path for every node makes a deeply nested artifact use
/// quadratic memory. Instead, each location points to its parent's pre-order
/// location and records only its child slot. A lookup hashes the stable id and
/// reconstructs just that node's path in O(depth).
#[derive(Debug, Clone)]
struct ElementIndex {
    by_id: HashMap<Box<str>, usize>,
    locations: Vec<ElementLocation>,
}

#[derive(Debug, Clone, Copy)]
struct ElementLocation {
    parent: Option<usize>,
    child_index: usize,
}

impl ElementIndex {
    fn build(root: &FnxElement, sidecar: &FnxSidecar) -> Self {
        let mut locations = Vec::with_capacity(sidecar.ids.len());
        let mut pending = vec![(root, None, 0)];
        while let Some((element, parent, child_index)) = pending.pop() {
            let position = locations.len();
            locations.push(ElementLocation {
                parent,
                child_index,
            });
            for (child_index, child) in element.children.iter().enumerate().rev() {
                pending.push((child, Some(position), child_index));
            }
        }

        let mut by_id = HashMap::with_capacity(sidecar.ids.len());
        for (position, entry) in sidecar.ids.iter().enumerate() {
            by_id.insert(entry.id.clone().into_boxed_str(), position);
        }
        Self { by_id, locations }
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    fn parent_position(&self, position: usize) -> Option<Option<usize>> {
        self.locations.get(position).map(|location| location.parent)
    }

    fn reverse_path(&self, mut position: usize) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        loop {
            let location = self.locations.get(position)?;
            let Some(parent) = location.parent else {
                return (position == 0).then_some(path);
            };
            // Locations are emitted in pre-order, so a parent always precedes
            // its child. Guarding that invariant prevents an accidental cycle
            // from turning lookup into an infinite loop.
            if parent >= position {
                return None;
            }
            path.push(location.child_index);
            position = parent;
        }
    }
}

impl ArtifactIr {
    fn from_parts(
        kind: ArtifactKind,
        fn_name: String,
        root: FnxElement,
        sidecar: FnxSidecar,
    ) -> Self {
        let index = Arc::new(ElementIndex::build(&root, &sidecar));
        Self {
            kind,
            fn_name,
            root,
            sidecar,
            index,
        }
    }

    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    pub fn fn_name(&self) -> &str {
        &self.fn_name
    }

    pub fn root(&self) -> &FnxElement {
        &self.root
    }

    pub fn sidecar(&self) -> &FnxSidecar {
        &self.sidecar
    }

    /// Build IR from a flat slice of node JSON values (page/component closure).
    pub fn from_nodes(
        kind: ArtifactKind,
        fn_name: impl Into<String>,
        nodes: &[Value],
    ) -> Result<Self, FnxError> {
        let fn_name = fn_name.into();
        let (text, sidecar) = encode_subtree(nodes, &fn_name)?;
        validate_unique_ids(&sidecar)?;
        let root = parse_doc(&text)?;
        Ok(Self::from_parts(kind, fn_name, root, sidecar))
    }

    /// Decode source text + sidecar into IR.
    pub fn from_source(
        kind: ArtifactKind,
        fn_name: impl Into<String>,
        source: &str,
        sidecar: FnxSidecar,
    ) -> Result<Self, FnxError> {
        validate_unique_ids(&sidecar)?;
        let root = parse_doc(source)?;
        Ok(Self::from_parts(kind, fn_name.into(), root, sidecar))
    }

    /// [`from_source`](Self::from_source) with a name-resolution context:
    /// name-based references in the source resolve into canonical ULIDs, so
    /// the retained tree stays reference-name-free (see [`crate::refs`]).
    pub fn from_source_with(
        kind: ArtifactKind,
        fn_name: impl Into<String>,
        source: &str,
        sidecar: FnxSidecar,
        refs: &RefTable,
    ) -> Result<Self, FnxError> {
        validate_unique_ids(&sidecar)?;
        let root = parse_doc_with(source, refs)?;
        Ok(Self::from_parts(kind, fn_name.into(), root, sidecar))
    }

    /// Print the IR as `.fnx` source (canonical printer).
    pub fn print(&self) -> String {
        print_doc(&self.fn_name, &self.root)
    }

    /// [`print`](Self::print) with a name-emission context: component and
    /// variable ids re-sugar into names when the table opted into
    /// `emit_names` (see [`crate::refs`]).
    pub fn print_with(&self, refs: &RefTable) -> String {
        print_doc_with(&self.fn_name, &self.root, refs)
    }

    /// Decode to flat node JSON values (structural keys restored).
    pub fn to_nodes(&self) -> Result<Vec<Value>, FnxError> {
        nodes_from_tree(
            &self.root,
            &self.sidecar.ids,
            self.sidecar.root_parent.as_deref(),
        )
    }

    /// Round-trip via encode path (scene → tree → print → parse), useful for
    /// projecting a scene-derived node list into a stable IR.
    pub fn reencode_from_nodes(
        kind: ArtifactKind,
        fn_name: impl Into<String>,
        nodes: &[Value],
    ) -> Result<Self, FnxError> {
        Self::from_nodes(kind, fn_name, nodes)
    }

    /// Decode using the high-level codec (source + sidecar).
    pub fn decode_source(
        kind: ArtifactKind,
        fn_name: impl Into<String>,
        source: &str,
        sidecar: &FnxSidecar,
    ) -> Result<(Self, Vec<Value>), FnxError> {
        let nodes = decode_subtree(source, sidecar)?;
        let ir = Self::from_source(kind, fn_name, source, sidecar.clone())?;
        // Prefer tree rebuilt from nodes for fingerprint consistency after reconcile.
        let tree = tree_from_nodes(&nodes)?;
        let sidecar = FnxSidecar {
            root_parent: tree.root_parent,
            ids: tree.sidecar,
        };
        Ok((
            Self::from_parts(ir.kind, ir.fn_name, tree.root, sidecar),
            nodes,
        ))
    }

    /// Locate one semantic element by the stable identity in the sidecar.
    pub fn element(&self, id: &str) -> Option<&FnxElement> {
        let position = self.index.position(id)?;
        // Defensively refuse a stale or malformed sidecar hit instead of
        // returning a different node.
        if self.sidecar.ids.get(position)?.id != id {
            return None;
        }
        if let Some(expected_parent) = self.sidecar.ids[position].parent_index {
            if self.index.parent_position(position)? != Some(expected_parent as usize) {
                return None;
            }
        }
        element_at_path(&self.root, self.index.reverse_path(position)?)
    }

    /// Replace the semantic tree and identity sidecar as one validated unit.
    ///
    /// Keeping both fields private prevents callers from silently making the
    /// stable-id index disagree with either representation.
    pub fn replace_structure(
        &mut self,
        root: FnxElement,
        sidecar: FnxSidecar,
    ) -> Result<(), FnxError> {
        validate_unique_ids(&sidecar)?;
        nodes_from_tree(&root, &sidecar.ids, sidecar.root_parent.as_deref())?;
        self.root = root;
        self.sidecar = sidecar;
        self.index = Arc::new(ElementIndex::build(&self.root, &self.sidecar));
        Ok(())
    }

    /// Replace one node's tag and attribute projection while retaining its
    /// existing child tree. Structural fields (`id`, `parent`, `index`) remain
    /// owned by the sidecar/tree relationship.
    pub fn patch_node_value(&mut self, id: &str, node: &Value) -> Result<(), FnxError> {
        let position = self
            .index
            .position(id)
            .ok_or_else(|| FnxError::Parse(format!("IR has no node id {id}")))?;
        if self
            .sidecar
            .ids
            .get(position)
            .map(|entry| entry.id.as_str())
            != Some(id)
        {
            return Err(FnxError::Parse(format!(
                "IR lookup index is stale for node id {id}"
            )));
        }
        if let Some(expected_parent) = self.sidecar.ids[position].parent_index {
            if self.index.parent_position(position) != Some(Some(expected_parent as usize)) {
                return Err(FnxError::Parse(format!(
                    "IR lookup index is stale for node id {id}"
                )));
            }
        }
        let replacement = element_of(node)?;
        let reverse_path = self
            .index
            .reverse_path(position)
            .ok_or_else(|| FnxError::Parse(format!("IR sidecar position {position} is absent")))?;
        let target = element_at_path_mut(&mut self.root, reverse_path)
            .ok_or_else(|| FnxError::Parse(format!("IR sidecar position {position} is absent")))?;
        let children = std::mem::take(&mut target.children);
        *target = replacement;
        target.children = children;
        if let Some(entry) = self.sidecar.ids.get_mut(position) {
            entry.tag = Some(target.tag.clone());
            entry.name = target
                .attrs
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Ok(())
    }

    /// Apply only semantic fields that differ between two serialized node
    /// values. Fields that are absent from both scene snapshots remain exactly
    /// as represented in the current IR (important for source-owned page-root
    /// sizing and future forward-compatible attributes).
    pub fn patch_node_delta(
        &mut self,
        id: &str,
        before: &Value,
        after: &Value,
    ) -> Result<(), FnxError> {
        let position = self
            .index
            .position(id)
            .ok_or_else(|| FnxError::Parse(format!("IR has no node id {id}")))?;
        if self
            .sidecar
            .ids
            .get(position)
            .map(|entry| entry.id.as_str())
            != Some(id)
        {
            return Err(FnxError::Parse(format!(
                "IR lookup index is stale for node id {id}"
            )));
        }
        if let Some(expected_parent) = self.sidecar.ids[position].parent_index {
            if self.index.parent_position(position) != Some(Some(expected_parent as usize)) {
                return Err(FnxError::Parse(format!(
                    "IR lookup index is stale for node id {id}"
                )));
            }
        }
        let before = element_of(before)?;
        let after = element_of(after)?;
        let reverse_path = self
            .index
            .reverse_path(position)
            .ok_or_else(|| FnxError::Parse(format!("IR sidecar position {position} is absent")))?;
        let target = element_at_path_mut(&mut self.root, reverse_path)
            .ok_or_else(|| FnxError::Parse(format!("IR sidecar position {position} is absent")))?;

        if before.tag != after.tag {
            target.tag = after.tag;
        }
        for key in before
            .attrs
            .keys()
            .chain(after.attrs.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let old = before.attrs.get(key);
            let new = after.attrs.get(key);
            if old == new {
                continue;
            }
            match new {
                Some(value) => {
                    target.attrs.insert(key.clone(), value.clone());
                }
                None => {
                    target.attrs.remove(key);
                }
            }
        }
        if let Some(entry) = self.sidecar.ids.get_mut(position) {
            entry.tag = Some(target.tag.clone());
            entry.name = target
                .attrs
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        Ok(())
    }
}

fn validate_unique_ids(sidecar: &FnxSidecar) -> Result<(), FnxError> {
    let mut ids = std::collections::HashSet::with_capacity(sidecar.ids.len());
    for entry in &sidecar.ids {
        if !ids.insert(entry.id.as_str()) {
            return Err(FnxError::Parse(format!(
                "duplicate sidecar node id {}",
                entry.id
            )));
        }
    }
    Ok(())
}

fn element_at_path(root: &FnxElement, reverse_path: Vec<usize>) -> Option<&FnxElement> {
    let mut element = root;
    for child_index in reverse_path.into_iter().rev() {
        element = element.children.get(child_index)?;
    }
    Some(element)
}

fn element_at_path_mut(root: &mut FnxElement, reverse_path: Vec<usize>) -> Option<&mut FnxElement> {
    let mut element = root;
    for child_index in reverse_path.into_iter().rev() {
        element = element.children.get_mut(child_index)?;
    }
    Some(element)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::IdEntry;
    use serde_json::json;

    fn frame_node(id: &str, name: &str) -> Value {
        json!({
            "type": "group",
            "id": id,
            "parent": null,
            "index": 1.0,
            "name": name,
            "opacity": 1.0,
            "blend_mode": "normal",
            "transform": [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            "scrollable": false,
        })
    }

    #[test]
    fn from_nodes_round_trips() {
        let nodes = vec![frame_node("01AAAAAAAAAAAAAAAAAAAAAAAA", "Home")];
        let ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Home", &nodes).unwrap();
        assert_eq!(ir.root.tag, "Frame");
        let back = ir.to_nodes().unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0]["name"], "Home");
        assert_eq!(back[0]["type"], "group");
    }

    #[test]
    fn print_contains_frame() {
        let nodes = vec![frame_node("01AAAAAAAAAAAAAAAAAAAAAAAA", "Home")];
        let ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Home", &nodes).unwrap();
        let text = ir.print();
        assert!(text.contains("<Frame"));
        assert!(text.contains("Home"));
    }

    #[test]
    fn indexed_lookup_follows_nested_preorder_paths() {
        let root_id = "01AAAAAAAAAAAAAAAAAAAAAAAA";
        let child_id = "01BBBBBBBBBBBBBBBBBBBBBBBB";
        let grandchild_id = "01CCCCCCCCCCCCCCCCCCCCCCCC";
        let sibling_id = "01DDDDDDDDDDDDDDDDDDDDDDDD";
        let mut root = frame_node(root_id, "Root");
        let mut child = frame_node(child_id, "Child");
        let mut grandchild = frame_node(grandchild_id, "Grandchild");
        let mut sibling = frame_node(sibling_id, "Sibling");
        root["index"] = Value::from(1);
        child["parent"] = Value::from(root_id);
        child["index"] = Value::from(1);
        grandchild["parent"] = Value::from(child_id);
        grandchild["index"] = Value::from(1);
        sibling["parent"] = Value::from(root_id);
        sibling["index"] = Value::from(2);
        let ir = ArtifactIr::from_nodes(
            ArtifactKind::Page,
            "Nested",
            &[root, sibling, grandchild, child],
        )
        .unwrap();

        assert_eq!(ir.element(root_id).unwrap().attrs["name"], "Root");
        assert_eq!(ir.element(child_id).unwrap().attrs["name"], "Child");
        assert_eq!(
            ir.element(grandchild_id).unwrap().attrs["name"],
            "Grandchild"
        );
        assert_eq!(ir.element(sibling_id).unwrap().attrs["name"], "Sibling");
    }

    fn wide_ir(child_count: usize) -> ArtifactIr {
        let mut root = FnxElement::new("Frame");
        root.attrs.insert("name".into(), Value::from("Root"));
        let mut ids = Vec::with_capacity(child_count + 1);
        ids.push(IdEntry {
            id: "root".into(),
            index: Value::from(1),
            tag: Some("Frame".into()),
            name: Some("Root".into()),
            parent_index: None,
        });
        for child_index in 0..child_count {
            let name = format!("Child {child_index}");
            let mut child = FnxElement::new("Frame");
            child.attrs.insert("name".into(), Value::from(name.clone()));
            root.children.push(child);
            ids.push(IdEntry {
                id: format!("node-{child_index}"),
                index: Value::from(child_index + 1),
                tag: Some("Frame".into()),
                name: Some(name),
                parent_index: Some(0),
            });
        }
        ArtifactIr::from_parts(
            ArtifactKind::Page,
            "Wide".into(),
            root,
            FnxSidecar {
                root_parent: None,
                ids,
            },
        )
    }

    #[test]
    fn indexed_lookup_and_patch_reach_last_of_fifty_thousand_nodes() {
        let mut ir = wide_ir(50_000);

        assert_eq!(
            ir.element("node-49999").unwrap().attrs["name"],
            "Child 49999"
        );
        ir.patch_node_delta(
            "node-49999",
            &json!({"type": "group", "name": "Child 49999", "opacity": 1.0}),
            &json!({"type": "group", "name": "Changed", "opacity": 0.5}),
        )
        .unwrap();

        let changed = ir.element("node-49999").unwrap();
        assert_eq!(changed.attrs["name"], "Changed");
        assert_eq!(changed.attrs["opacity"], 0.5);
        assert_eq!(ir.element("node-0").unwrap().attrs["name"], "Child 0");
    }

    #[test]
    fn structural_replacement_rebuilds_index() {
        let mut ir = wide_ir(2);
        let mut root = ir.root().clone();
        let mut sidecar = ir.sidecar().clone();
        root.children.swap(0, 1);
        sidecar.ids.swap(1, 2);
        ir.replace_structure(root, sidecar).unwrap();
        assert_eq!(ir.element("node-1").unwrap().attrs["name"], "Child 1");
        assert_eq!(ir.element("node-0").unwrap().attrs["name"], "Child 0");
    }

    #[test]
    fn lookup_metadata_does_not_change_semantic_equality() {
        let left = wide_ir(2);
        let mut right = left.clone();
        Arc::make_mut(&mut right.index).by_id.clear();

        assert_eq!(left, right);
    }
}

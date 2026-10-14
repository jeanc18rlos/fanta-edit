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
use std::collections::{BTreeMap, HashMap, HashSet};
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
    revision: u64,
    mutation_token: Arc<()>,
}

pub struct StructuralNodeChange {
    pub id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

pub struct PreparedStructuralPatch {
    previous_sidecar: FnxSidecar,
    previous_index: Arc<ElementIndex>,
    previous_revision: u64,
    previous_order: Vec<String>,
    previous_children: HashMap<String, Vec<String>>,
    pub(crate) order: Vec<String>,
    pub(crate) children: HashMap<String, Vec<String>>,
    pub(crate) replacements: BTreeMap<String, FnxElement>,
    sidecar: FnxSidecar,
}

impl PreparedStructuralPatch {
    pub fn sidecar(&self) -> &FnxSidecar {
        &self.sidecar
    }

    pub fn node_value_at(&self, previous: &ArtifactIr, position: usize) -> Result<Value, FnxError> {
        let entry = self
            .sidecar
            .ids
            .get(position)
            .ok_or_else(|| FnxError::Parse("structural node position is absent".into()))?;
        let element = self
            .element(previous, &entry.id)
            .ok_or_else(|| FnxError::Parse(format!("structural node {} is absent", entry.id)))?;
        value_at(element, &self.sidecar, position)
    }

    pub(crate) fn element<'a>(
        &'a self,
        previous: &'a ArtifactIr,
        id: &str,
    ) -> Option<&'a FnxElement> {
        self.replacements.get(id).or_else(|| previous.element(id))
    }

    pub(crate) fn matches(&self, previous: &ArtifactIr) -> bool {
        self.previous_sidecar == previous.sidecar
            && self.previous_revision == previous.revision
            && Arc::ptr_eq(&self.previous_index, &previous.index)
    }
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
            revision: 0,
            mutation_token: Arc::new(()),
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

    /// Every semantic writer replaces this token. Holding it keeps an old
    /// allocation alive, so a new tree cannot accidentally reuse its identity.
    pub fn synchronization_token(&self) -> Arc<()> {
        Arc::clone(&self.mutation_token)
    }

    pub fn node_value(&self, id: &str) -> Result<Value, FnxError> {
        let position = self
            .index
            .position(id)
            .ok_or_else(|| FnxError::Parse(format!("IR has no node id {id}")))?;
        let element = self
            .element(id)
            .ok_or_else(|| FnxError::Parse(format!("IR lookup index is stale for node id {id}")))?;
        value_at(element, &self.sidecar, position)
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
        let mut root = parse_doc(source)?;
        materialize_ids(&mut root, &sidecar)?;
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
        let mut root = parse_doc_with(source, refs)?;
        materialize_ids(&mut root, &sidecar)?;
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
        self.revision = self.revision.wrapping_add(1);
        self.mutation_token = Arc::new(());
        Ok(())
    }

    /// `before` values must represent the retained nodes' typed projection.
    /// Fields absent from both snapshots remain owned by the authored IR.
    pub fn prepare_structural_patch(
        &self,
        changes: &[StructuralNodeChange],
    ) -> Result<PreparedStructuralPatch, FnxError> {
        let root_id = self
            .sidecar
            .ids
            .first()
            .ok_or_else(|| FnxError::Parse("structural patch has no artifact root".into()))?
            .id
            .clone();
        let mut entries = HashMap::with_capacity(self.sidecar.ids.len() + changes.len());
        let mut parents = HashMap::with_capacity(self.sidecar.ids.len() + changes.len());
        let mut previous_children = HashMap::<String, Vec<String>>::new();
        let previous_order: Vec<_> = self
            .sidecar
            .ids
            .iter()
            .map(|entry| entry.id.clone())
            .collect();
        for (position, entry) in self.sidecar.ids.iter().enumerate() {
            let element = self.element(&entry.id).ok_or_else(|| {
                FnxError::Parse(format!("structural patch lost retained node {}", entry.id))
            })?;
            if element.attrs.get("id").and_then(Value::as_str) != Some(entry.id.as_str()) {
                return Err(FnxError::Parse("retained tree identity is stale".into()));
            }
            let parent = self
                .index
                .parent_position(position)
                .ok_or_else(|| FnxError::Parse("retained tree topology is stale".into()))?;
            let parent = parent
                .map(|parent| {
                    self.sidecar
                        .ids
                        .get(parent)
                        .map(|entry| entry.id.clone())
                        .ok_or_else(|| FnxError::Parse("retained tree parent is absent".into()))
                })
                .transpose()?;
            if let Some(parent) = &parent {
                previous_children
                    .entry(parent.clone())
                    .or_default()
                    .push(entry.id.clone());
            }
            previous_children.entry(entry.id.clone()).or_default();
            let mut entry = entry.clone();
            entry.tag = Some(element.tag.clone());
            entry.name = element
                .attrs
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_owned);
            parents.insert(entry.id.clone(), parent);
            entries.insert(entry.id.clone(), entry);
        }
        let mut replacements = BTreeMap::new();
        let mut touched = HashSet::new();
        for change in changes {
            if !touched.insert(change.id.as_str()) {
                return Err(FnxError::Parse(format!(
                    "duplicate structural change {}",
                    change.id
                )));
            }
            let existing = entries.get(&change.id);
            if existing.is_some() != change.before.is_some() {
                return Err(FnxError::Parse(format!(
                    "structural change has a stale baseline for {}",
                    change.id
                )));
            }
            if let Some(before) = &change.before {
                if before.get("id").and_then(Value::as_str) != Some(change.id.as_str()) {
                    return Err(FnxError::Parse(
                        "structural baseline identity disagrees".into(),
                    ));
                }
                if change.id != root_id
                    && (before.get("parent").and_then(Value::as_str)
                        != parents.get(&change.id).and_then(|parent| parent.as_deref())
                        || before
                            .get("index")
                            .and_then(Value::as_f64)
                            .map(f64::to_bits)
                            != existing
                                .and_then(|entry| entry.index.as_f64())
                                .map(f64::to_bits))
                {
                    return Err(FnxError::Parse(format!(
                        "structural baseline topology disagrees for {}",
                        change.id
                    )));
                }
            }
            let Some(after) = &change.after else {
                if change.id == root_id {
                    return Err(FnxError::Parse("cannot remove an artifact root".into()));
                }
                entries.remove(&change.id);
                parents.remove(&change.id);
                continue;
            };
            if after.get("id").and_then(Value::as_str) != Some(change.id.as_str()) {
                return Err(FnxError::Parse(
                    "structural replacement identity disagrees".into(),
                ));
            }
            let projected = element_of(after)?;
            let replacement = if let Some(before) = &change.before {
                let retained = self.element(&change.id).ok_or_else(|| {
                    FnxError::Parse(format!("retained node {} is absent", change.id))
                })?;
                let before = element_of(before)?;
                if before.tag == projected.tag && before.attrs == projected.attrs {
                    None
                } else {
                    let mut replacement = FnxElement {
                        tag: retained.tag.clone(),
                        attrs: retained.attrs.clone(),
                        children: Vec::new(),
                    };
                    if before.tag != projected.tag {
                        replacement.tag = projected.tag.clone();
                    }
                    for key in before
                        .attrs
                        .keys()
                        .chain(projected.attrs.keys())
                        .collect::<HashSet<_>>()
                    {
                        if before.attrs.get(key) == projected.attrs.get(key) {
                            continue;
                        }
                        match projected.attrs.get(key) {
                            Some(value) => {
                                replacement.attrs.insert(key.clone(), value.clone());
                            }
                            None => {
                                replacement.attrs.remove(key);
                            }
                        }
                    }
                    Some(replacement)
                }
            } else {
                Some(projected)
            };
            let parent = after
                .get("parent")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if change.id == root_id {
                if parent.is_some() && parent != self.sidecar.root_parent {
                    return Err(FnxError::Parse("cannot reparent an artifact root".into()));
                }
                parents.insert(change.id.clone(), None);
            } else {
                parents.insert(change.id.clone(), parent);
            }
            let element = replacement
                .as_ref()
                .or_else(|| self.element(&change.id))
                .ok_or_else(|| FnxError::Parse("structural replacement lost its element".into()))?;
            entries.insert(
                change.id.clone(),
                crate::model::IdEntry {
                    id: change.id.clone(),
                    index: after.get("index").cloned().unwrap_or(Value::Null),
                    tag: Some(element.tag.clone()),
                    name: element
                        .attrs
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    parent_index: None,
                },
            );
            if let Some(replacement) = replacement {
                replacements.insert(change.id.clone(), replacement);
            }
        }
        let mut children = HashMap::<String, Vec<String>>::with_capacity(entries.len());
        for id in entries.keys() {
            children.entry(id.clone()).or_default();
            if id == &root_id {
                continue;
            }
            let parent = parents
                .get(id)
                .and_then(|parent| parent.as_ref())
                .filter(|parent| entries.contains_key(*parent))
                .ok_or_else(|| {
                    FnxError::Parse(format!(
                        "structural patch leaves node {id} without its parent"
                    ))
                })?;
            children.entry(parent.clone()).or_default().push(id.clone());
        }
        for siblings in children.values_mut() {
            siblings.sort_by(|left, right| {
                let index = |id: &String| {
                    entries
                        .get(id)
                        .and_then(|entry| entry.index.as_f64())
                        .unwrap_or(0.0)
                };
                index(left)
                    .total_cmp(&index(right))
                    .then_with(|| left.cmp(right))
            });
        }
        let mut order = Vec::with_capacity(entries.len());
        let mut sidecar = FnxSidecar {
            root_parent: self.sidecar.root_parent.clone(),
            ids: Vec::with_capacity(entries.len()),
        };
        let mut pending = vec![(root_id, None)];
        let mut visited = HashSet::with_capacity(entries.len());
        while let Some((id, parent_index)) = pending.pop() {
            if !visited.insert(id.clone()) {
                return Err(FnxError::Parse("structural patch contains a cycle".into()));
            }
            let position =
                u32::try_from(order.len()).map_err(|error| FnxError::Parse(error.to_string()))?;
            let mut entry = entries
                .get(&id)
                .cloned()
                .ok_or_else(|| FnxError::Parse("structural patch root is absent".into()))?;
            entry.parent_index = parent_index;
            sidecar.ids.push(entry);
            order.push(id.clone());
            pending.extend(
                children
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .rev()
                    .map(|child| (child.clone(), Some(position))),
            );
        }
        if visited.len() != entries.len() {
            return Err(FnxError::Parse(
                "structural patch contains an orphan or cycle".into(),
            ));
        }
        Ok(PreparedStructuralPatch {
            previous_sidecar: self.sidecar.clone(),
            previous_index: Arc::clone(&self.index),
            previous_revision: self.revision,
            previous_order,
            previous_children,
            order,
            children,
            replacements,
            sidecar,
        })
    }

    /// Install a prepared topology while moving unchanged attribute payloads.
    /// A stale witness or assembly failure leaves the previous IR installed.
    pub fn commit_structural_patch(
        &mut self,
        patch: PreparedStructuralPatch,
    ) -> Result<(), FnxError> {
        if !patch.matches(self) {
            return Err(FnxError::Parse(
                "structural patch baseline changed before commit".into(),
            ));
        }
        let previous_token = Arc::clone(&self.mutation_token);
        self.mutation_token = Arc::new(());
        let root = std::mem::replace(&mut self.root, FnxElement::new("Frame"));
        let mut nodes = flatten_owned_tree(root, &patch.previous_order);
        let mut replaced = BTreeMap::new();
        let mut inserted = Vec::new();
        for (id, replacement) in patch.replacements {
            match nodes.insert(id.clone(), replacement) {
                Some(previous) => {
                    replaced.insert(id, previous);
                }
                None => inserted.push(id),
            }
        }
        match assemble_owned_tree(&patch.order, &patch.children, &mut nodes) {
            Ok(root) => {
                self.root = root;
                self.sidecar = patch.sidecar;
                self.index = Arc::new(ElementIndex::build(&self.root, &self.sidecar));
                self.revision = self.revision.wrapping_add(1);
                Ok(())
            }
            Err(error) => {
                for (id, previous) in replaced {
                    nodes.insert(id, previous);
                }
                for id in inserted {
                    nodes.remove(&id);
                }
                self.root = assemble_owned_tree(
                    &patch.previous_order,
                    &patch.previous_children,
                    &mut nodes,
                )?;
                self.mutation_token = previous_token;
                Err(error)
            }
        }
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
        self.revision = self.revision.wrapping_add(1);
        self.mutation_token = Arc::new(());
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
        self.revision = self.revision.wrapping_add(1);
        self.mutation_token = Arc::new(());
        Ok(())
    }
}

fn value_at(
    element: &FnxElement,
    sidecar: &FnxSidecar,
    position: usize,
) -> Result<Value, FnxError> {
    let entry = sidecar
        .ids
        .get(position)
        .ok_or_else(|| FnxError::Parse("IR node position is absent".into()))?;
    let parent = match entry.parent_index {
        Some(parent) => Some(
            sidecar
                .ids
                .get(parent as usize)
                .ok_or_else(|| FnxError::Parse("IR node parent is absent".into()))?
                .id
                .as_str(),
        ),
        None => sidecar.root_parent.as_deref(),
    };
    crate::convert::element_to_value(element, &entry.id, parent, &entry.index)
}

fn flatten_owned_tree(root: FnxElement, order: &[String]) -> HashMap<String, FnxElement> {
    let mut nodes = HashMap::with_capacity(order.len());
    let mut pending = vec![root];
    for id in order {
        if let Some(mut element) = pending.pop() {
            pending.extend(std::mem::take(&mut element.children).into_iter().rev());
            nodes.insert(id.clone(), element);
        }
    }
    nodes
}

fn flatten_assembled_nodes(
    nodes: &mut HashMap<String, FnxElement>,
    children: &HashMap<String, Vec<String>>,
) {
    let mut pending: Vec<_> = std::mem::take(nodes).into_iter().collect();
    while let Some((id, mut element)) = pending.pop() {
        pending.extend(
            children
                .get(&id)
                .into_iter()
                .flatten()
                .cloned()
                .zip(std::mem::take(&mut element.children)),
        );
        nodes.insert(id, element);
    }
}

fn assemble_owned_tree(
    order: &[String],
    children: &HashMap<String, Vec<String>>,
    nodes: &mut HashMap<String, FnxElement>,
) -> Result<FnxElement, FnxError> {
    let root = order
        .first()
        .ok_or_else(|| FnxError::Parse("structural tree has no root".into()))?;
    if let Some(id) = order.iter().find(|id| !nodes.contains_key(*id)) {
        return Err(FnxError::Parse(format!("structural tree lost node {id}")));
    }
    for id in order.iter().rev() {
        let Some(mut element) = nodes.remove(id) else {
            flatten_assembled_nodes(nodes, children);
            return Err(FnxError::Parse(format!("structural tree lost parent {id}")));
        };
        for child in children.get(id).into_iter().flatten() {
            let Some(child_element) = nodes.remove(child) else {
                nodes.insert(id.clone(), element);
                flatten_assembled_nodes(nodes, children);
                return Err(FnxError::Parse(format!(
                    "structural tree lost child {child}"
                )));
            };
            element.children.push(child_element);
        }
        nodes.insert(id.clone(), element);
    }
    nodes
        .remove(root)
        .ok_or_else(|| FnxError::Parse("structural tree lost its root".into()))
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

fn materialize_ids(root: &mut FnxElement, sidecar: &FnxSidecar) -> Result<(), FnxError> {
    fn visit(
        element: &mut FnxElement,
        ids: &[crate::model::IdEntry],
        cursor: &mut usize,
    ) -> Result<(), FnxError> {
        let entry = ids.get(*cursor).ok_or(FnxError::SidecarMismatch {
            sidecar: ids.len(),
            elements: *cursor + 1,
        })?;
        *cursor += 1;
        if let Some(explicit) = element.attrs.get("id")
            && explicit.as_str() != Some(entry.id.as_str())
        {
            return Err(FnxError::Parse(format!(
                "explicit node id {explicit} disagrees with sidecar id {}",
                entry.id
            )));
        }
        element
            .attrs
            .insert("id".into(), Value::String(entry.id.clone()));
        for child in &mut element.children {
            visit(child, ids, cursor)?;
        }
        Ok(())
    }

    let mut cursor = 0;
    visit(root, &sidecar.ids, &mut cursor)?;
    if cursor != sidecar.ids.len() {
        return Err(FnxError::SidecarMismatch {
            sidecar: sidecar.ids.len(),
            elements: cursor,
        });
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
    fn structural_patch_validation_and_commit_failures_are_atomic() {
        let nodes = vec![
            serde_json::json!({"id":"root","type":"group","parent":null,"index":1,"name":"Root"}),
            serde_json::json!({"id":"child","type":"group","parent":"root","index":1,"name":"Child"}),
        ];
        let mut ir = ArtifactIr::from_nodes(ArtifactKind::Page, "Page", &nodes).expect("IR");
        let before = ir.clone();
        let original_token = ir.synchronization_token();
        for parent in ["absent", "child"] {
            let mut after = nodes[1].clone();
            after["parent"] = serde_json::json!(parent);
            assert!(
                ir.prepare_structural_patch(&[StructuralNodeChange {
                    id: "child".into(),
                    before: Some(nodes[1].clone()),
                    after: Some(after),
                }])
                .is_err()
            );
            assert_eq!(ir, before);
            assert!(Arc::ptr_eq(&original_token, &ir.synchronization_token()));
        }
        let make_insert = || StructuralNodeChange {
            id: "inserted".into(),
            before: None,
            after: Some(
                serde_json::json!({"id":"inserted","type":"text","parent":"root","index":2,"name":"Inserted"}),
            ),
        };
        for corrupt_order in [true, false] {
            let mut patch = ir
                .prepare_structural_patch(&[make_insert()])
                .expect("prepare");
            if corrupt_order {
                patch.order.push("missing".into());
            } else {
                patch
                    .children
                    .get_mut("root")
                    .expect("root children")
                    .push("missing".into());
            }
            assert!(ir.commit_structural_patch(patch).is_err());
            assert_eq!(ir, before);
            assert!(Arc::ptr_eq(&original_token, &ir.synchronization_token()));
            assert_eq!(
                ir.to_nodes().expect("restored nodes"),
                before.to_nodes().expect("original nodes")
            );
        }
        let patch = ir
            .prepare_structural_patch(&[make_insert()])
            .expect("prepare");
        let mut changed = nodes[1].clone();
        changed["opacity"] = serde_json::json!(0.5);
        ir.patch_node_delta("child", &nodes[1], &changed)
            .expect("property edit");
        assert!(!Arc::ptr_eq(&original_token, &ir.synchronization_token()));
        let edited = ir.clone();
        assert!(ir.commit_structural_patch(patch).is_err());
        assert_eq!(
            ir, edited,
            "a stale plan must not overwrite a later property edit"
        );
    }

    #[test]
    fn synchronization_tokens_and_single_node_projection_follow_semantic_writers() {
        let mut ir = wide_ir(2);
        let original = ir.synchronization_token();
        let unchanged = ir.clone();
        assert!(Arc::ptr_eq(&original, &unchanged.synchronization_token()));
        for node in ir.to_nodes().expect("all nodes") {
            let id = node.get("id").and_then(Value::as_str).expect("identity");
            assert_eq!(ir.node_value(id).expect("single node"), node);
        }
        let mut changed = ir.node_value("node-1").expect("node");
        changed["name"] = json!("Changed");
        ir.patch_node_value("node-1", &changed).expect("patch");
        assert!(!Arc::ptr_eq(&original, &ir.synchronization_token()));
        let patched = ir.synchronization_token();
        ir.replace_structure(ir.root().clone(), ir.sidecar().clone())
            .expect("replace");
        assert!(!Arc::ptr_eq(&patched, &ir.synchronization_token()));
    }

    #[test]
    fn lookup_metadata_does_not_change_semantic_equality() {
        let left = wide_ir(2);
        let mut right = left.clone();
        Arc::make_mut(&mut right.index).by_id.clear();

        assert_eq!(left, right);
    }
}

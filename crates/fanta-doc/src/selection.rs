//! Selection — the ordered set of currently-selected nodes.
//!
//! Order matters for keyboard navigation and the layers panel, so the items
//! stay in a `Vec`; membership is answered by a shadow `HashSet` kept in
//! lockstep. The original Vec-only design assumed selections stay small
//! ("a handful, occasionally a few hundred") — a select-all on an imported
//! SVG selects tens of thousands, and every per-node `contains` in the
//! canvas chrome walk and the layer-row rebuild then cost O(selection),
//! turning one frame into hundreds of millions of comparisons.

use std::collections::HashSet;

use crate::id::NodeId;
use serde::{Deserialize, Serialize};

/// An ordered, deduplicated set of node IDs plus the most recent "anchor" used
/// for shift-click range extension. Serialized as `{ items, anchor }` — the
/// membership index is derived and never leaves the process.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(from = "SelectionWire", into = "SelectionWire")]
pub struct Selection {
    items: Vec<NodeId>,
    index: HashSet<NodeId>,
    /// The last node that was clicked / added — used as the anchor for
    /// shift-click range extension in the tools layer.
    anchor: Option<NodeId>,
}

/// The persisted shape — identical to the pre-index format so documents
/// round-trip unchanged.
#[derive(Serialize, Deserialize)]
struct SelectionWire {
    items: Vec<NodeId>,
    #[serde(default)]
    anchor: Option<NodeId>,
}

impl From<SelectionWire> for Selection {
    fn from(wire: SelectionWire) -> Self {
        let mut selection = Selection {
            items: Vec::with_capacity(wire.items.len()),
            index: HashSet::with_capacity(wire.items.len()),
            anchor: wire.anchor,
        };
        for id in wire.items {
            if selection.index.insert(id) {
                selection.items.push(id);
            }
        }
        selection
    }
}

impl From<Selection> for SelectionWire {
    fn from(selection: Selection) -> Self {
        SelectionWire {
            items: selection.items,
            anchor: selection.anchor,
        }
    }
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        // The index is derived from `items`; comparing it would be redundant.
        self.items == other.items && self.anchor == other.anchor
    }
}

impl Eq for Selection {}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// O(1) — safe to call per node inside full-scene walks.
    pub fn contains(&self, id: NodeId) -> bool {
        self.index.contains(&id)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, NodeId> {
        self.items.iter()
    }

    pub fn as_slice(&self) -> &[NodeId] {
        &self.items
    }

    pub fn anchor(&self) -> Option<NodeId> {
        self.anchor
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.index.clear();
        self.anchor = None;
    }

    /// Replace the selection with exactly one node.
    pub fn select_only(&mut self, id: NodeId) {
        self.items.clear();
        self.index.clear();
        self.items.push(id);
        self.index.insert(id);
        self.anchor = Some(id);
    }

    /// Add `id` if not already present. Becomes the new anchor.
    pub fn add(&mut self, id: NodeId) {
        if self.index.insert(id) {
            self.items.push(id);
        }
        self.anchor = Some(id);
    }

    /// Remove `id`. Does nothing if not present. Anchor cleared if it was `id`.
    pub fn remove(&mut self, id: NodeId) {
        if self.index.remove(&id)
            && let Some(pos) = self.items.iter().position(|x| *x == id)
        {
            self.items.remove(pos);
        }
        if self.anchor == Some(id) {
            self.anchor = self.items.last().copied();
        }
    }

    /// Add if absent, remove if present (Cmd-click semantics).
    pub fn toggle(&mut self, id: NodeId) {
        if self.contains(id) {
            self.remove(id);
        } else {
            self.add(id);
        }
    }

    /// Replace with a multi-node selection (marquee / select-all result).
    /// O(n) — dedup through the index, not by scanning the Vec per insert.
    pub fn replace_with(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.items.clear();
        self.index.clear();
        for id in ids {
            if self.index.insert(id) {
                self.items.push(id);
            }
        }
        self.anchor = self.items.last().copied();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_dedups_and_updates_anchor() {
        let mut s = Selection::new();
        let a = NodeId::new();
        let b = NodeId::new();
        s.add(a);
        s.add(b);
        s.add(a); // re-add a; should not duplicate but should re-anchor
        assert_eq!(s.len(), 2);
        assert_eq!(s.anchor(), Some(a));
    }

    #[test]
    fn toggle_round_trips() {
        let mut s = Selection::new();
        let id = NodeId::new();
        s.toggle(id);
        assert!(s.contains(id));
        s.toggle(id);
        assert!(!s.contains(id));
    }

    #[test]
    fn select_only_replaces_existing() {
        let mut s = Selection::new();
        s.add(NodeId::new());
        s.add(NodeId::new());
        let target = NodeId::new();
        s.select_only(target);
        assert_eq!(s.as_slice(), &[target]);
        assert_eq!(s.anchor(), Some(target));
    }

    #[test]
    fn remove_falls_back_anchor_to_last_remaining() {
        let mut s = Selection::new();
        let a = NodeId::new();
        let b = NodeId::new();
        s.add(a);
        s.add(b);
        assert_eq!(s.anchor(), Some(b));
        s.remove(b);
        assert_eq!(s.anchor(), Some(a));
    }

    #[test]
    fn wire_round_trip_rebuilds_the_membership_index() {
        let mut s = Selection::new();
        let a = NodeId::new();
        let b = NodeId::new();
        s.add(a);
        s.add(b);
        let json = serde_json::to_string(&s).expect("serialize");
        let back: Selection = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, s);
        assert!(back.contains(a) && back.contains(b));
        assert!(!back.contains(NodeId::new()));
    }
}

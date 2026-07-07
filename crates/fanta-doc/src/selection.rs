//! Selection — the ordered set of currently-selected nodes.
//!
//! Order matters for keyboard navigation and the layers panel. The set is
//! small in practice (a handful, occasionally a few hundred); a `Vec` with
//! linear `contains` is faster than a `HashSet` until selection sizes blow
//! past ~1000, and we don't expect that to be the hot path.

use crate::id::NodeId;
use serde::{Deserialize, Serialize};

/// An ordered, deduplicated set of node IDs plus the most recent "anchor" used
/// for shift-click range extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    items: Vec<NodeId>,
    /// The last node that was clicked / added — used as the anchor for
    /// shift-click range extension in the tools layer.
    #[serde(default)]
    anchor: Option<NodeId>,
}

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

    pub fn contains(&self, id: NodeId) -> bool {
        self.items.contains(&id)
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
        self.anchor = None;
    }

    /// Replace the selection with exactly one node.
    pub fn select_only(&mut self, id: NodeId) {
        self.items.clear();
        self.items.push(id);
        self.anchor = Some(id);
    }

    /// Add `id` if not already present. Becomes the new anchor.
    pub fn add(&mut self, id: NodeId) {
        if !self.items.contains(&id) {
            self.items.push(id);
        }
        self.anchor = Some(id);
    }

    /// Remove `id`. Does nothing if not present. Anchor cleared if it was `id`.
    pub fn remove(&mut self, id: NodeId) {
        if let Some(pos) = self.items.iter().position(|x| *x == id) {
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

    /// Replace with a multi-node selection (marquee result).
    pub fn replace_with(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.items.clear();
        for id in ids {
            if !self.items.contains(&id) {
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
}

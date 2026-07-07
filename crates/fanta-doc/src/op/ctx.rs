//! The apply boundary — [`OpCtx`], the bundle of mutable doc slices an
//! [`Operation`](crate::op::Operation) may touch, and [`ModeScope`].

use crate::component::ComponentLibrary;
use crate::id::{ModeId, NodeId, VariableCollectionId};
use crate::op::Operation;
use crate::scene::Scene;
use crate::variables::VariableRegistry;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The mutable doc state an [`Operation`] may touch when applied or reverted.
///
/// Built by `Doc::apply`/`undo`/`redo` (and `History`'s replay paths) from the
/// owning [`crate::doc::Doc`]'s fields. Scene-only ops use just [`Self::scene`];
/// design-system ops reach into the other slices. Bundling them keeps the op
/// surface uniform while confining the "what can an op mutate" blast radius to
/// this one struct.
///
/// [`Operation`]: crate::op::Operation
pub struct OpCtx<'a> {
    pub scene: &'a mut Scene,
    pub components: &'a mut ComponentLibrary,
    pub variables: &'a mut VariableRegistry,
    pub active_modes: &'a mut BTreeMap<VariableCollectionId, ModeId>,
    pub flow_start: &'a mut Option<NodeId>,
}

impl OpCtx<'_> {
    /// After `op` applies or reverts, bump the revision of any component master
    /// that contains the op's touched node, invalidating memoized instance
    /// expansions of that master. No-op when there are no components, or the op
    /// targets no scene node (variable/mode/flow ops, or a delete whose node is
    /// already gone). O(depth) — see [`ComponentLibrary::bump_rev_for_node`].
    pub fn bump_revs(&mut self, op: &Operation) {
        if self.components.defs.is_empty() {
            return;
        }
        // A component-prop edit names a *component*, not a scene node — bump that
        // def's master directly (its `primary_target` is `None`) so every
        // instance re-expands with the new schema/bindings.
        if let Operation::SetComponentProps { component, .. } = op {
            if let Some(root) = self.components.defs.get(component).map(|d| d.root) {
                self.components.bump_rev_for_node(self.scene, root);
            }
            return;
        }
        // A variant-set edit names a *set*, not a scene node — its axes/values/
        // default drive how each member resolves, so bump every member's master
        // root to invalidate memoized instance expansions of the set.
        if let Operation::SetComponentSet { id, .. } = op {
            let roots: Vec<NodeId> = self
                .components
                .sets
                .get(id)
                .map(|s| {
                    s.members
                        .iter()
                        .filter_map(|m| self.components.defs.get(m).map(|d| d.root))
                        .collect()
                })
                .unwrap_or_default();
            for root in roots {
                self.components.bump_rev_for_node(self.scene, root);
            }
            return;
        }
        if let Some(node) = op.primary_target() {
            self.components.bump_rev_for_node(self.scene, node);
        }
    }
}

/// Which mode map a [`Operation::SetActiveMode`] writes to: the document-wide
/// active modes, or one frame's [`crate::node::GroupNode::explicit_modes`] pin.
///
/// [`Operation::SetActiveMode`]: crate::op::Operation::SetActiveMode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum ModeScope {
    /// The document's global active mode for the collection.
    Doc,
    /// A specific frame's per-frame theme pin.
    Frame { node: NodeId },
}

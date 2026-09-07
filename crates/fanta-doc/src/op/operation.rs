//! The [`Operation`] sum type — one atomic, reversible change per variant — plus
//! its construction and metadata helpers. The `apply`/`revert` behaviour lives in
//! [`crate::op::apply`].

use crate::binding::BoundProp;
use crate::component::{ComponentDef, ComponentPropDef, ComponentSet, ComponentSetMembership};
use crate::id::{
    AnimationClipId, AnimationTrackId, ComponentId, ComponentPropId, KeyframeId, ModeId, NodeId,
    VariableCollectionId, VariableId,
};
use crate::index::IndexKey;
use crate::motion::{AnimationClip, AnimationTrack, Keyframe, MotionTarget};
use crate::node::{CanvasNode, LayoutChild, NodeData, NodeFlags, Override, Reaction};
use crate::op::ModeScope;
use crate::style::{BlendMode, Blur, Shadow, UnitInterval};
use crate::transform::Transform2D;
use crate::value::VarValue;
use crate::variables::{Mode, Variable, VariableCollection};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// One atomic, reversible change. Each variant carries enough data to both
/// apply and revert; the revert path uses the "old" fields. Storing the old
/// value at op-construction time means undo is O(1) and doesn't depend on the
/// scene's current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    // ---- scene structure (pre-existing) -------------------------------------
    /// Add a new node. Reverts by removing it. Boxed because `CanvasNode` is
    /// large (~400 bytes) and we don't want every variant to inherit that.
    CreateNode { node: Box<CanvasNode> },

    /// Remove a node and its descendants. Stores the full snapshot so revert
    /// can re-insert the subtree exactly as it was.
    DeleteSubtree { snapshot: Vec<CanvasNode> },

    /// Move a node to a new parent and/or position.
    Reparent {
        id: NodeId,
        old_parent: Option<NodeId>,
        old_index: IndexKey,
        new_parent: Option<NodeId>,
        new_index: IndexKey,
    },

    /// Z-order change within the same parent.
    SetIndex {
        id: NodeId,
        old: IndexKey,
        new: IndexKey,
    },

    /// Replace a node's affine transform.
    SetTransform {
        id: NodeId,
        old: Transform2D,
        new: Transform2D,
    },

    /// Rename.
    SetName {
        id: NodeId,
        old: String,
        new: String,
    },

    /// Replace a node's opaque [`meta`](crate::node::CanvasNode::meta) blob —
    /// the user-extension JSON plugins/agents/UI stash per-node settings in
    /// (e.g. a page's chosen sidebar icon). Carries the whole old/new value so
    /// undo is O(1) and key-merging happens at the call site.
    SetMeta {
        id: NodeId,
        old: serde_json::Value,
        new: serde_json::Value,
    },

    /// Change opacity. Values are [`UnitInterval`]s, so they are always in
    /// `0.0..=1.0` — apply/revert just write the stored value, no clamping.
    SetOpacity {
        id: NodeId,
        old: UnitInterval,
        new: UnitInterval,
    },

    /// Change blend mode.
    SetBlendMode {
        id: NodeId,
        old: BlendMode,
        new: BlendMode,
    },

    /// Replace the node-level drop/inner shadow stack. Reverts to `old`.
    SetEffects {
        id: NodeId,
        old: SmallVec<[Shadow; 0]>,
        new: SmallVec<[Shadow; 0]>,
    },

    /// Replace the node-level blur stack (layer / background). Reverts to `old`.
    SetBlurs {
        id: NodeId,
        old: SmallVec<[Blur; 0]>,
        new: SmallVec<[Blur; 0]>,
    },

    /// Change the node-level flags (locked / hidden / isolated blend / …).
    SetFlags {
        id: NodeId,
        old: NodeFlags,
        new: NodeFlags,
    },

    /// Change how a node participates as a child of an auto-layout parent
    /// (FILL/grow, absolute positioning, per-child align-self).
    SetLayoutChild {
        id: NodeId,
        old: Option<LayoutChild>,
        new: Option<LayoutChild>,
    },

    /// Replace the variant-specific data. Heaviest op; AI re-rolls, path edits.
    ReplaceData {
        id: NodeId,
        old: Box<NodeData>,
        new: Box<NodeData>,
    },

    // ---- components ---------------------------------------------------------
    /// Place a new instance node in the scene. The master must already exist in
    /// the library. Reverts by removing the node. (Distinct from `CreateNode`
    /// only by label/intent; both insert a node.)
    CreateInstance { node: Box<CanvasNode> },

    /// Register a component definition in the library. Reverts by removing it.
    /// The master subtree is created via ordinary `CreateNode` ops.
    DefineComponent { def: Box<ComponentDef> },

    /// Remove a component definition. Stores the full def so revert restores it.
    DeleteComponent {
        id: ComponentId,
        def: Box<ComponentDef>,
    },

    /// Replace a component def's exposed-property schema (add/remove/rename a
    /// prop, change its default, or edit its descendant bindings). A coarse
    /// snapshot op — `old`/`new` are the whole props vec — so undo is symmetric.
    /// Bumps the def's `rev` (via `OpCtx`) so instances re-expand.
    SetComponentProps {
        component: ComponentId,
        old: Vec<ComponentPropDef>,
        new: Vec<ComponentPropDef>,
    },

    /// Register a component set (variant group).
    DefineComponentSet { set: Box<ComponentSet> },

    /// Remove a component set. Stores the full set so revert restores it.
    DeleteComponentSet {
        id: ComponentId,
        set: Box<ComponentSet>,
    },

    /// Set, replace, or clear a component def's variant-set membership.
    /// `old`/`new` are `None` when the def is outside any set before/after —
    /// covers joining a set, retargeting axis values, and detaching from a set.
    SetVariantMembership {
        id: ComponentId,
        old: Option<ComponentSetMembership>,
        new: Option<ComponentSetMembership>,
    },

    /// Replace a component set's definition (axes, values, members,
    /// default_variant) — variant-set editing after creation. A coarse
    /// old/new snapshot, so the inverse restores the prior set exactly (unlike
    /// re-`DefineComponentSet`, whose inverse would delete it).
    SetComponentSet {
        id: ComponentId,
        old: Box<ComponentSet>,
        new: Box<ComponentSet>,
    },

    /// Set (or replace, or clear) one override on an instance, addressed by
    /// `index` into the instance's `overrides` vec. `old`/`new` are `None` when
    /// the slot is absent before/after — covers add, edit, and remove.
    SetInstanceOverride {
        id: NodeId,
        index: usize,
        old: Option<Override>,
        new: Option<Override>,
    },

    /// Swap which component an instance is an instance of.
    SwapInstance {
        id: NodeId,
        old: ComponentId,
        new: ComponentId,
    },

    /// Set (or clear) one exposed prop value on an instance.
    SetInstanceProp {
        id: NodeId,
        prop: ComponentPropId,
        old: Option<VarValue>,
        new: Option<VarValue>,
    },

    /// Detach an instance into a concrete subtree: replaces the instance node's
    /// data with the resolved group and inserts the expanded children. Stored
    /// reversibly by snapshotting both the old and new instance data and the
    /// inserted subtree.
    DetachInstance {
        id: NodeId,
        old: Box<NodeData>,
        new: Box<NodeData>,
        /// Children produced by the detach, root-first; removed on revert.
        expanded: Vec<CanvasNode>,
    },

    // ---- variables ----------------------------------------------------------
    /// Create a variable collection. Reverts by removing it.
    CreateVariableCollection { collection: Box<VariableCollection> },

    /// Remove a variable collection. Stores the full collection for revert.
    DeleteVariableCollection {
        id: VariableCollectionId,
        collection: Box<VariableCollection>,
    },

    /// Add a mode to a collection (appended). Reverts by popping it.
    AddMode {
        collection: VariableCollectionId,
        mode: Mode,
    },

    /// Remove a mode from a collection. Stores the mode and its position so
    /// revert re-inserts it where it was.
    RemoveMode {
        collection: VariableCollectionId,
        index: usize,
        mode: Mode,
    },

    /// Create a variable. Reverts by removing it.
    CreateVariable { variable: Box<Variable> },

    /// Remove a variable. Stores the full variable for revert.
    DeleteVariable {
        id: VariableId,
        variable: Box<Variable>,
    },

    /// Set (or clear) a variable's value for one mode.
    SetVariableValue {
        variable: VariableId,
        mode: ModeId,
        old: Option<VarValue>,
        new: Option<VarValue>,
    },

    /// Rename a variable. Reverts to the old name.
    RenameVariable {
        id: VariableId,
        old: String,
        new: String,
    },

    /// Rename a variable collection. Reverts to the old name.
    RenameVariableCollection {
        id: VariableCollectionId,
        old: String,
        new: String,
    },

    /// Rename a mode within a collection. Reverts to the old name.
    RenameMode {
        collection: VariableCollectionId,
        mode: ModeId,
        old: String,
        new: String,
    },

    // ---- bindings -----------------------------------------------------------
    /// Bind a node property to a variable. Reverts to the prior binding (if any).
    BindProperty {
        node: NodeId,
        prop: BoundProp,
        old: Option<VariableId>,
        new: VariableId,
    },

    /// Unbind a node property, baking the last-resolved literal back onto the
    /// node (single-sourced through `BoundProp::apply_resolved` at construction
    /// time). Stores enough to reverse the bake: the variable removed, plus the
    /// node data / opacity / flags before and after.
    UnbindProperty {
        node: NodeId,
        prop: BoundProp,
        variable: VariableId,
        /// Node data before the literal was baked in (for revert).
        old_data: Box<NodeData>,
        /// Node data after baking the resolved literal (for redo).
        new_data: Box<NodeData>,
        /// Wrapper-level fields the bake may have touched.
        old_opacity: UnitInterval,
        new_opacity: UnitInterval,
        old_flags: NodeFlags,
        new_flags: NodeFlags,
    },

    /// Set the active mode for a collection (doc-wide or per-frame).
    SetActiveMode {
        scope: ModeScope,
        collection: VariableCollectionId,
        old: Option<ModeId>,
        new: Option<ModeId>,
    },

    // ---- motion -------------------------------------------------------------
    /// Add a persistent animation clip. Reverts by removing the whole clip.
    CreateAnimationClip { clip: Box<AnimationClip> },

    /// Remove a clip and every track/keyframe it owns. Revert restores the
    /// complete snapshot.
    DeleteAnimationClip {
        id: AnimationClipId,
        clip: Box<AnimationClip>,
    },

    /// Rename a clip without replacing its concurrently-edited timeline data.
    SetAnimationClipName {
        id: AnimationClipId,
        old: String,
        new: String,
    },

    /// Change the finite playback boundary of a clip.
    SetAnimationClipDuration {
        id: AnimationClipId,
        old: u32,
        new: u32,
    },

    /// Add, replace, or remove one track. `None` represents a missing track.
    SetAnimationTrack {
        clip: AnimationClipId,
        track: AnimationTrackId,
        old: Option<Box<AnimationTrack>>,
        new: Option<Box<AnimationTrack>>,
    },

    /// Add, replace, or remove one keyframe. `target` makes the scene node
    /// affected by this edit available without consulting mutable doc state.
    SetKeyframe {
        clip: AnimationClipId,
        track: AnimationTrackId,
        target: MotionTarget,
        keyframe: KeyframeId,
        old: Option<Keyframe>,
        new: Option<Keyframe>,
    },

    // ---- prototyping --------------------------------------------------------
    /// Add a reaction to a node (appended). Reverts by popping it.
    AddReaction { node: NodeId, reaction: Reaction },

    /// Remove a reaction from a node. Stores the reaction and its index.
    RemoveReaction {
        node: NodeId,
        index: usize,
        reaction: Reaction,
    },

    /// Replace the reaction at `index` on `node` in place (edit its trigger,
    /// action, frame transition, or property-animation binding). Stores both
    /// prior and new for a clean inverse.
    SetReaction {
        node: NodeId,
        index: usize,
        old: Reaction,
        new: Reaction,
    },

    /// Set the prototype flow start node (doc-level). Reverts to the prior one.
    SetFlowStart {
        old: Option<NodeId>,
        new: Option<NodeId>,
    },
}

impl Operation {
    /// Convenience constructor for [`Operation::CreateNode`] hiding the `Box`.
    pub fn create_node(node: CanvasNode) -> Self {
        Self::CreateNode {
            node: Box::new(node),
        }
    }

    /// The scene node this op primarily touches, if any — used to invalidate the
    /// owning component master's revision (see [`OpCtx::bump_revs`]). `None` for
    /// ops that touch no scene node (variable/mode/flow/component-registry ops)
    /// or whose node is removed by the op itself (`DeleteSubtree`).
    ///
    /// [`OpCtx::bump_revs`]: crate::op::OpCtx::bump_revs
    pub fn primary_target(&self) -> Option<NodeId> {
        match self {
            Self::CreateNode { node } | Self::CreateInstance { node } => Some(node.id),
            Self::Reparent { id, .. }
            | Self::SetIndex { id, .. }
            | Self::SetTransform { id, .. }
            | Self::SetName { id, .. }
            | Self::SetMeta { id, .. }
            | Self::SetOpacity { id, .. }
            | Self::SetBlendMode { id, .. }
            | Self::SetEffects { id, .. }
            | Self::SetBlurs { id, .. }
            | Self::SetFlags { id, .. }
            | Self::SetLayoutChild { id, .. }
            | Self::ReplaceData { id, .. }
            | Self::SetInstanceOverride { id, .. }
            | Self::SwapInstance { id, .. }
            | Self::SetInstanceProp { id, .. }
            | Self::DetachInstance { id, .. } => Some(*id),
            Self::BindProperty { node, .. }
            | Self::UnbindProperty { node, .. }
            | Self::SetKeyframe {
                target: MotionTarget { node, .. },
                ..
            }
            | Self::AddReaction { node, .. }
            | Self::RemoveReaction { node, .. }
            | Self::SetReaction { node, .. } => Some(*node),
            Self::SetAnimationTrack { old, new, .. } => new
                .as_deref()
                .or(old.as_deref())
                .map(|track| track.target.node),
            Self::CreateAnimationClip { clip } | Self::DeleteAnimationClip { clip, .. } => {
                common_clip_target(clip)
            }
            _ => None,
        }
    }

    /// A short, user-facing label for the undo/redo UI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CreateNode { .. } => "Create",
            Self::DeleteSubtree { .. } => "Delete",
            Self::Reparent { .. } => "Move",
            Self::SetIndex { .. } => "Reorder",
            Self::SetTransform { .. } => "Transform",
            Self::SetName { .. } => "Rename",
            Self::SetMeta { .. } => "Edit",
            Self::SetOpacity { .. } => "Opacity",
            Self::SetBlendMode { .. } => "Blend",
            Self::SetEffects { .. } => "Shadow",
            Self::SetBlurs { .. } => "Blur",
            Self::SetFlags { .. } => "Flags",
            Self::SetLayoutChild { .. } => "Layout Child",
            Self::ReplaceData { .. } => "Edit",
            Self::CreateInstance { .. } => "Insert Instance",
            Self::DefineComponent { .. } => "Create Component",
            Self::DeleteComponent { .. } => "Delete Component",
            Self::SetComponentProps { .. } => "Edit Component Props",
            Self::DefineComponentSet { .. } => "Create Variant Set",
            Self::DeleteComponentSet { .. } => "Delete Variant Set",
            Self::SetVariantMembership { .. } => "Set Variant",
            Self::SetComponentSet { .. } => "Edit Variant Set",
            Self::SetInstanceOverride { .. } => "Override",
            Self::SwapInstance { .. } => "Swap Instance",
            Self::SetInstanceProp { .. } => "Set Property",
            Self::DetachInstance { .. } => "Detach Instance",
            Self::CreateVariableCollection { .. } => "Create Collection",
            Self::DeleteVariableCollection { .. } => "Delete Collection",
            Self::AddMode { .. } => "Add Mode",
            Self::RemoveMode { .. } => "Remove Mode",
            Self::CreateVariable { .. } => "Create Variable",
            Self::DeleteVariable { .. } => "Delete Variable",
            Self::SetVariableValue { .. } => "Set Variable",
            Self::RenameVariable { .. } => "Rename Variable",
            Self::RenameVariableCollection { .. } => "Rename Collection",
            Self::RenameMode { .. } => "Rename Mode",
            Self::BindProperty { .. } => "Bind",
            Self::UnbindProperty { .. } => "Unbind",
            Self::SetActiveMode { .. } => "Set Mode",
            Self::CreateAnimationClip { .. } => "Create Animation",
            Self::DeleteAnimationClip { .. } => "Delete Animation",
            Self::SetAnimationClipName { .. } => "Rename Animation",
            Self::SetAnimationClipDuration { .. } => "Animation Duration",
            Self::SetAnimationTrack { .. } => "Edit Animation Track",
            Self::SetKeyframe { .. } => "Edit Keyframe",
            Self::AddReaction { .. } => "Add Interaction",
            Self::RemoveReaction { .. } => "Remove Interaction",
            Self::SetReaction { .. } => "Edit Interaction",
            Self::SetFlowStart { .. } => "Set Flow Start",
        }
    }
}

/// A clip-level edit has a primary node only when every track addresses the
/// same node. Multi-node clips intentionally return `None` rather than picking
/// an arbitrary target for cache invalidation and operation metadata.
fn common_clip_target(clip: &AnimationClip) -> Option<NodeId> {
    let mut nodes = clip.tracks.values().map(|track| track.target.node);
    let first = nodes.next()?;
    nodes.all(|node| node == first).then_some(first)
}

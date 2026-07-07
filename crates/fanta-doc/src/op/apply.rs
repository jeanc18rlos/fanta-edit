//! The `apply` / `revert` behaviour of [`Operation`], plus the small private
//! helpers (`with_instance`, the `set_instance_*` field writers, and
//! `set_active_mode`) shared by both directions.

use crate::id::{ComponentId, ComponentPropId, NodeId, VariableCollectionId};
use crate::node::{InstanceNode, NodeData, Override};
use crate::op::{ModeScope, OpCtx, Operation};
use crate::scene::{Scene, SceneError};
use crate::value::VarValue;

impl Operation {
    /// Apply this operation. On error, state is left unchanged.
    pub fn apply(&self, ctx: &mut OpCtx) -> Result<(), SceneError> {
        match self {
            // ---- scene structure --------------------------------------------
            Self::CreateNode { node } | Self::CreateInstance { node } => {
                ctx.scene.insert((**node).clone()).map(|_| ())
            }
            Self::DeleteSubtree { snapshot } => {
                let root_id = snapshot
                    .first()
                    .ok_or_else(|| SceneError::InvariantViolated("empty delete snapshot".into()))?
                    .id;
                ctx.scene.remove(root_id).map(|_| ())
            }
            Self::Reparent {
                id,
                new_parent,
                new_index,
                ..
            } => ctx.scene.set_parent(*id, *new_parent, *new_index),
            Self::SetIndex { id, new, .. } => ctx.scene.set_index(*id, *new),
            Self::SetTransform { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.transform = *new;
                Ok(())
            }
            Self::SetName { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.name = new.clone();
                Ok(())
            }
            Self::SetMeta { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.meta = new.clone();
                Ok(())
            }
            Self::SetOpacity { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.opacity = new.clamp(0.0, 1.0);
                Ok(())
            }
            Self::SetBlendMode { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blend_mode = *new;
                Ok(())
            }
            Self::SetEffects { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.effects = new.clone();
                Ok(())
            }
            Self::SetBlurs { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blurs = new.clone();
                Ok(())
            }
            Self::SetFlags { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.flags = *new;
                Ok(())
            }
            Self::SetLayoutChild { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.layout_child = *new;
                Ok(())
            }
            Self::ReplaceData { id, new, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.data = (**new).clone();
                Ok(())
            }

            // ---- components -------------------------------------------------
            Self::DefineComponent { def } => {
                ctx.components.defs.insert(def.id, (**def).clone());
                Ok(())
            }
            Self::DeleteComponent { id, .. } => {
                ctx.components.defs.remove(id);
                Ok(())
            }
            Self::SetComponentProps { component, new, .. } => {
                if let Some(def) = ctx.components.defs.get_mut(component) {
                    def.props = new.clone();
                }
                Ok(())
            }
            Self::SetVariantMembership { id, new, .. } => {
                if let Some(def) = ctx.components.defs.get_mut(id) {
                    def.variant_of = new.clone();
                }
                Ok(())
            }
            Self::SetComponentSet { id, new, .. } => {
                ctx.components.sets.insert(*id, (**new).clone());
                Ok(())
            }
            Self::DefineComponentSet { set } => {
                ctx.components.sets.insert(set.id, (**set).clone());
                Ok(())
            }
            Self::DeleteComponentSet { id, .. } => {
                ctx.components.sets.remove(id);
                Ok(())
            }
            Self::SetInstanceOverride { id, index, new, .. } => {
                set_instance_override(ctx.scene, *id, *index, new.clone())
            }
            Self::SwapInstance { id, new, .. } => set_instance_component(ctx.scene, *id, *new),
            Self::SetInstanceProp { id, prop, new, .. } => {
                set_instance_prop(ctx.scene, *id, *prop, new.clone())
            }
            Self::DetachInstance {
                id, new, expanded, ..
            } => {
                {
                    let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                    n.data = (**new).clone();
                }
                for child in expanded {
                    ctx.scene.insert(child.clone())?;
                }
                Ok(())
            }

            // ---- variables --------------------------------------------------
            Self::CreateVariableCollection { collection } => {
                ctx.variables
                    .collections
                    .insert(collection.id, (**collection).clone());
                Ok(())
            }
            Self::DeleteVariableCollection { id, .. } => {
                ctx.variables.collections.remove(id);
                Ok(())
            }
            Self::AddMode { collection, mode } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    c.modes.push(mode.clone());
                }
                Ok(())
            }
            Self::RemoveMode {
                collection, index, ..
            } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    if *index < c.modes.len() {
                        c.modes.remove(*index);
                    }
                }
                Ok(())
            }
            Self::CreateVariable { variable } => {
                ctx.variables
                    .variables
                    .insert(variable.id, (**variable).clone());
                Ok(())
            }
            Self::DeleteVariable { id, .. } => {
                ctx.variables.variables.remove(id);
                Ok(())
            }
            Self::SetVariableValue {
                variable,
                mode,
                new,
                ..
            } => {
                if let Some(v) = ctx.variables.variables.get_mut(variable) {
                    match new {
                        Some(val) => {
                            v.values_by_mode.insert(*mode, val.clone());
                        }
                        None => {
                            v.values_by_mode.remove(mode);
                        }
                    }
                }
                Ok(())
            }
            Self::RenameVariable { id, new, .. } => {
                if let Some(v) = ctx.variables.variables.get_mut(id) {
                    v.name = new.clone();
                }
                Ok(())
            }
            Self::RenameVariableCollection { id, new, .. } => {
                if let Some(c) = ctx.variables.collections.get_mut(id) {
                    c.name = new.clone();
                }
                Ok(())
            }
            Self::RenameMode {
                collection,
                mode,
                new,
                ..
            } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    if let Some(m) = c.modes.iter_mut().find(|m| m.id == *mode) {
                        m.name = new.clone();
                    }
                }
                Ok(())
            }

            // ---- bindings ---------------------------------------------------
            Self::BindProperty {
                node, prop, new, ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                n.bindings.insert(*prop, *new);
                Ok(())
            }
            Self::UnbindProperty {
                node,
                prop,
                new_data,
                new_opacity,
                new_flags,
                ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                n.bindings.remove(prop);
                n.data = (**new_data).clone();
                n.opacity = *new_opacity;
                n.flags = *new_flags;
                Ok(())
            }
            Self::SetActiveMode {
                scope,
                collection,
                new,
                ..
            } => set_active_mode(ctx, scope, *collection, *new),

            // ---- prototyping ------------------------------------------------
            Self::AddReaction { node, reaction } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                n.reactions.push(reaction.clone());
                Ok(())
            }
            Self::RemoveReaction { node, index, .. } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                if *index < n.reactions.len() {
                    n.reactions.remove(*index);
                }
                Ok(())
            }
            Self::SetReaction {
                node, index, new, ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                if let Some(slot) = n.reactions.get_mut(*index) {
                    *slot = new.clone();
                }
                Ok(())
            }
            Self::SetFlowStart { new, .. } => {
                *ctx.flow_start = *new;
                Ok(())
            }
        }
    }

    /// Revert this operation. Symmetric to [`apply`].
    ///
    /// [`apply`]: Operation::apply
    pub fn revert(&self, ctx: &mut OpCtx) -> Result<(), SceneError> {
        match self {
            // ---- scene structure --------------------------------------------
            Self::CreateNode { node } | Self::CreateInstance { node } => {
                ctx.scene.remove(node.id).map(|_| ())
            }
            Self::DeleteSubtree { snapshot } => {
                for n in snapshot {
                    ctx.scene.insert(n.clone())?;
                }
                Ok(())
            }
            Self::Reparent {
                id,
                old_parent,
                old_index,
                ..
            } => ctx.scene.set_parent(*id, *old_parent, *old_index),
            Self::SetIndex { id, old, .. } => ctx.scene.set_index(*id, *old),
            Self::SetTransform { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.transform = *old;
                Ok(())
            }
            Self::SetName { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.name = old.clone();
                Ok(())
            }
            Self::SetMeta { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.meta = old.clone();
                Ok(())
            }
            Self::SetOpacity { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.opacity = *old;
                Ok(())
            }
            Self::SetBlendMode { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blend_mode = *old;
                Ok(())
            }
            Self::SetEffects { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.effects = old.clone();
                Ok(())
            }
            Self::SetBlurs { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blurs = old.clone();
                Ok(())
            }
            Self::SetFlags { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.flags = *old;
                Ok(())
            }
            Self::SetLayoutChild { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.layout_child = *old;
                Ok(())
            }
            Self::ReplaceData { id, old, .. } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.data = (**old).clone();
                Ok(())
            }

            // ---- components -------------------------------------------------
            Self::DefineComponent { def } => {
                ctx.components.defs.remove(&def.id);
                Ok(())
            }
            Self::DeleteComponent { id, def } => {
                ctx.components.defs.insert(*id, (**def).clone());
                Ok(())
            }
            Self::SetComponentProps { component, old, .. } => {
                if let Some(def) = ctx.components.defs.get_mut(component) {
                    def.props = old.clone();
                }
                Ok(())
            }
            Self::SetVariantMembership { id, old, .. } => {
                if let Some(def) = ctx.components.defs.get_mut(id) {
                    def.variant_of = old.clone();
                }
                Ok(())
            }
            Self::SetComponentSet { id, old, .. } => {
                ctx.components.sets.insert(*id, (**old).clone());
                Ok(())
            }
            Self::DefineComponentSet { set } => {
                ctx.components.sets.remove(&set.id);
                Ok(())
            }
            Self::DeleteComponentSet { id, set } => {
                ctx.components.sets.insert(*id, (**set).clone());
                Ok(())
            }
            Self::SetInstanceOverride { id, index, old, .. } => {
                set_instance_override(ctx.scene, *id, *index, old.clone())
            }
            Self::SwapInstance { id, old, .. } => set_instance_component(ctx.scene, *id, *old),
            Self::SetInstanceProp { id, prop, old, .. } => {
                set_instance_prop(ctx.scene, *id, *prop, old.clone())
            }
            Self::DetachInstance {
                id, old, expanded, ..
            } => {
                // Remove the expanded children (reverse so leaves go before
                // their parents), then restore the instance data.
                for child in expanded.iter().rev() {
                    let _ = ctx.scene.remove(child.id);
                }
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.data = (**old).clone();
                Ok(())
            }

            // ---- variables --------------------------------------------------
            Self::CreateVariableCollection { collection } => {
                ctx.variables.collections.remove(&collection.id);
                Ok(())
            }
            Self::DeleteVariableCollection { id, collection } => {
                ctx.variables
                    .collections
                    .insert(*id, (**collection).clone());
                Ok(())
            }
            Self::AddMode { collection, .. } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    c.modes.pop();
                }
                Ok(())
            }
            Self::RemoveMode {
                collection,
                index,
                mode,
            } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    let at = (*index).min(c.modes.len());
                    c.modes.insert(at, mode.clone());
                }
                Ok(())
            }
            Self::CreateVariable { variable } => {
                ctx.variables.variables.remove(&variable.id);
                Ok(())
            }
            Self::DeleteVariable { id, variable } => {
                ctx.variables.variables.insert(*id, (**variable).clone());
                Ok(())
            }
            Self::SetVariableValue {
                variable,
                mode,
                old,
                ..
            } => {
                if let Some(v) = ctx.variables.variables.get_mut(variable) {
                    match old {
                        Some(val) => {
                            v.values_by_mode.insert(*mode, val.clone());
                        }
                        None => {
                            v.values_by_mode.remove(mode);
                        }
                    }
                }
                Ok(())
            }
            Self::RenameVariable { id, old, .. } => {
                if let Some(v) = ctx.variables.variables.get_mut(id) {
                    v.name = old.clone();
                }
                Ok(())
            }
            Self::RenameVariableCollection { id, old, .. } => {
                if let Some(c) = ctx.variables.collections.get_mut(id) {
                    c.name = old.clone();
                }
                Ok(())
            }
            Self::RenameMode {
                collection,
                mode,
                old,
                ..
            } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    if let Some(m) = c.modes.iter_mut().find(|m| m.id == *mode) {
                        m.name = old.clone();
                    }
                }
                Ok(())
            }

            // ---- bindings ---------------------------------------------------
            Self::BindProperty {
                node, prop, old, ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                match old {
                    Some(v) => {
                        n.bindings.insert(*prop, *v);
                    }
                    None => {
                        n.bindings.remove(prop);
                    }
                }
                Ok(())
            }
            Self::UnbindProperty {
                node,
                prop,
                variable,
                old_data,
                old_opacity,
                old_flags,
                ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                n.bindings.insert(*prop, *variable);
                n.data = (**old_data).clone();
                n.opacity = *old_opacity;
                n.flags = *old_flags;
                Ok(())
            }
            Self::SetActiveMode {
                scope,
                collection,
                old,
                ..
            } => set_active_mode(ctx, scope, *collection, *old),

            // ---- prototyping ------------------------------------------------
            Self::AddReaction { node, .. } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                n.reactions.pop();
                Ok(())
            }
            Self::RemoveReaction {
                node,
                index,
                reaction,
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                let at = (*index).min(n.reactions.len());
                n.reactions.insert(at, reaction.clone());
                Ok(())
            }
            Self::SetReaction {
                node, index, old, ..
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                if let Some(slot) = n.reactions.get_mut(*index) {
                    *slot = old.clone();
                }
                Ok(())
            }
            Self::SetFlowStart { old, .. } => {
                *ctx.flow_start = *old;
                Ok(())
            }
        }
    }
}

// ---- instance field helpers -------------------------------------------------
//
// Shared by apply/revert so the "reach into an InstanceNode and mutate one
// field" logic lives once. A non-instance target is tolerated as a no-op
// (returns Ok) — the op was constructed against an instance, but a stale op
// replayed against a detached node should not hard-error the whole transaction.

fn with_instance<R>(
    scene: &mut Scene,
    id: NodeId,
    f: impl FnOnce(&mut InstanceNode) -> R,
) -> Result<Option<R>, SceneError> {
    let n = scene.get_mut(id).ok_or(SceneError::NotFound(id))?;
    Ok(match &mut n.data {
        NodeData::Instance(inst) => Some(f(inst)),
        _ => None,
    })
}

fn set_instance_override(
    scene: &mut Scene,
    id: NodeId,
    index: usize,
    value: Option<Override>,
) -> Result<(), SceneError> {
    with_instance(scene, id, |inst| match value {
        Some(ov) => {
            if index < inst.overrides.len() {
                inst.overrides[index] = ov;
            } else {
                inst.overrides.push(ov);
            }
        }
        None => {
            if index < inst.overrides.len() {
                inst.overrides.remove(index);
            }
        }
    })
    .map(|_| ())
}

fn set_instance_component(
    scene: &mut Scene,
    id: NodeId,
    component: ComponentId,
) -> Result<(), SceneError> {
    with_instance(scene, id, |inst| inst.component = component).map(|_| ())
}

fn set_instance_prop(
    scene: &mut Scene,
    id: NodeId,
    prop: ComponentPropId,
    value: Option<VarValue>,
) -> Result<(), SceneError> {
    with_instance(scene, id, |inst| match value {
        Some(v) => {
            inst.prop_values.insert(prop, v);
        }
        None => {
            inst.prop_values.remove(&prop);
        }
    })
    .map(|_| ())
}

fn set_active_mode(
    ctx: &mut OpCtx,
    scope: &ModeScope,
    collection: VariableCollectionId,
    value: Option<crate::id::ModeId>,
) -> Result<(), SceneError> {
    match scope {
        ModeScope::Doc => {
            match value {
                Some(m) => {
                    ctx.active_modes.insert(collection, m);
                }
                None => {
                    ctx.active_modes.remove(&collection);
                }
            }
            Ok(())
        }
        ModeScope::Frame { node } => {
            let n = ctx
                .scene
                .get_mut(*node)
                .ok_or(SceneError::NotFound(*node))?;
            if let NodeData::Group(g) = &mut n.data {
                match value {
                    Some(m) => {
                        g.explicit_modes.insert(collection, m);
                    }
                    None => {
                        g.explicit_modes.remove(&collection);
                    }
                }
            }
            Ok(())
        }
    }
}

//! The `apply` / `revert` behaviour of [`Operation`], driven by one directioned
//! [`Operation::run`] match, plus the small private helpers (`with_instance`,
//! the `set_instance_*` field writers, and `set_active_mode`) shared by both
//! directions.

use crate::id::{ComponentId, ComponentPropId, NodeId, VariableCollectionId};
use crate::node::{InstanceNode, NodeData, Override};
use crate::op::{ModeScope, OpCtx, Operation};
use crate::scene::{Scene, SceneError};
use crate::value::VarValue;

/// Which way [`Operation::run`] is executing. Every operation stores its own
/// inverse (`old`/`new`), so the two directions share one match: symmetric
/// field writes pick the value with [`Dir::pick`], and the arms whose inverse
/// is a *different action* (insert↔remove, push↔pop) branch on `dir`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Apply,
    Revert,
}

impl Dir {
    /// The stored value this direction writes: `new` when applying, `old` when
    /// reverting.
    fn pick<'a, T: ?Sized>(self, old: &'a T, new: &'a T) -> &'a T {
        match self {
            Dir::Apply => new,
            Dir::Revert => old,
        }
    }
}

impl Operation {
    /// Apply this operation. On error, state is left unchanged.
    pub fn apply(&self, ctx: &mut OpCtx) -> Result<(), SceneError> {
        self.run(ctx, Dir::Apply)
    }

    /// Revert this operation. Symmetric to [`apply`](Operation::apply): it
    /// undoes exactly what `apply` did, restoring the pre-apply state.
    pub fn revert(&self, ctx: &mut OpCtx) -> Result<(), SceneError> {
        self.run(ctx, Dir::Revert)
    }

    /// The shared apply/revert body. See [`Dir`] for how the two directions
    /// collapse into one match.
    fn run(&self, ctx: &mut OpCtx, dir: Dir) -> Result<(), SceneError> {
        match self {
            // ---- scene structure --------------------------------------------
            Self::CreateNode { node } | Self::CreateInstance { node } => match dir {
                Dir::Apply => ctx.scene.insert((**node).clone()).map(|_| ()),
                Dir::Revert => ctx.scene.remove(node.id).map(|_| ()),
            },
            Self::DeleteSubtree { snapshot } => match dir {
                Dir::Apply => {
                    let root_id = snapshot
                        .first()
                        .ok_or_else(|| {
                            SceneError::InvariantViolated("empty delete snapshot".into())
                        })?
                        .id;
                    ctx.scene.remove(root_id).map(|_| ())
                }
                Dir::Revert => {
                    for n in snapshot {
                        ctx.scene.insert(n.clone())?;
                    }
                    Ok(())
                }
            },
            Self::Reparent {
                id,
                old_parent,
                old_index,
                new_parent,
                new_index,
            } => {
                let (parent, index) = match dir {
                    Dir::Apply => (new_parent, new_index),
                    Dir::Revert => (old_parent, old_index),
                };
                ctx.scene.set_parent(*id, *parent, *index)
            }
            Self::SetIndex { id, old, new } => ctx.scene.set_index(*id, *dir.pick(old, new)),
            Self::SetTransform { id, old, new } => {
                ctx.scene.set_transform(*id, *dir.pick(old, new))
            }
            Self::SetName { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.name = dir.pick(old, new).clone();
                Ok(())
            }
            Self::SetMeta { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.meta = dir.pick(old, new).clone();
                Ok(())
            }
            Self::SetOpacity { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                // Both values are UnitInterval, already in range — a plain pick,
                // no clamp branch (the value cannot be out of range).
                n.opacity = *dir.pick(old, new);
                Ok(())
            }
            Self::SetBlendMode { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blend_mode = *dir.pick(old, new);
                Ok(())
            }
            Self::SetEffects { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.effects = dir.pick(old, new).clone();
                Ok(())
            }
            Self::SetBlurs { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.blurs = dir.pick(old, new).clone();
                Ok(())
            }
            Self::SetFlags { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.flags = *dir.pick(old, new);
                Ok(())
            }
            Self::SetLayoutChild { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.layout_child = *dir.pick(old, new);
                Ok(())
            }
            Self::ReplaceData { id, old, new } => {
                let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                n.data = (**dir.pick(old, new)).clone();
                Ok(())
            }

            // ---- components -------------------------------------------------
            Self::DefineComponent { def } => match dir {
                Dir::Apply => {
                    ctx.components.defs.insert(def.id, (**def).clone());
                    Ok(())
                }
                Dir::Revert => {
                    ctx.components.defs.remove(&def.id);
                    Ok(())
                }
            },
            Self::DeleteComponent { id, def } => match dir {
                Dir::Apply => {
                    ctx.components.defs.remove(id);
                    Ok(())
                }
                Dir::Revert => {
                    ctx.components.defs.insert(*id, (**def).clone());
                    Ok(())
                }
            },
            Self::SetComponentProps {
                component,
                old,
                new,
                ..
            } => {
                if let Some(def) = ctx.components.defs.get_mut(component) {
                    def.props = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::SetVariantMembership { id, old, new, .. } => {
                if let Some(def) = ctx.components.defs.get_mut(id) {
                    def.variant_of = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::SetComponentSet { id, old, new, .. } => {
                ctx.components
                    .sets
                    .insert(*id, (**dir.pick(old, new)).clone());
                Ok(())
            }
            Self::DefineComponentSet { set } => match dir {
                Dir::Apply => {
                    ctx.components.sets.insert(set.id, (**set).clone());
                    Ok(())
                }
                Dir::Revert => {
                    ctx.components.sets.remove(&set.id);
                    Ok(())
                }
            },
            Self::DeleteComponentSet { id, set } => match dir {
                Dir::Apply => {
                    ctx.components.sets.remove(id);
                    Ok(())
                }
                Dir::Revert => {
                    ctx.components.sets.insert(*id, (**set).clone());
                    Ok(())
                }
            },
            Self::SetInstanceOverride {
                id,
                index,
                old,
                new,
                ..
            } => set_instance_override(ctx.scene, *id, *index, dir.pick(old, new).clone()),
            Self::SwapInstance { id, old, new, .. } => {
                set_instance_component(ctx.scene, *id, *dir.pick(old, new))
            }
            Self::SetInstanceProp {
                id, prop, old, new, ..
            } => set_instance_prop(ctx.scene, *id, *prop, dir.pick(old, new).clone()),
            Self::DetachInstance {
                id,
                old,
                new,
                expanded,
            } => match dir {
                Dir::Apply => {
                    {
                        let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                        n.data = (**new).clone();
                    }
                    for child in expanded {
                        ctx.scene.insert(child.clone())?;
                    }
                    Ok(())
                }
                Dir::Revert => {
                    // Remove the expanded children (reverse so leaves go before
                    // their parents), then restore the instance data.
                    for child in expanded.iter().rev() {
                        let _ = ctx.scene.remove(child.id);
                    }
                    let n = ctx.scene.get_mut(*id).ok_or(SceneError::NotFound(*id))?;
                    n.data = (**old).clone();
                    Ok(())
                }
            },

            // ---- variables --------------------------------------------------
            Self::CreateVariableCollection { collection } => match dir {
                Dir::Apply => {
                    ctx.variables
                        .collections
                        .insert(collection.id, (**collection).clone());
                    Ok(())
                }
                Dir::Revert => {
                    ctx.variables.collections.remove(&collection.id);
                    Ok(())
                }
            },
            Self::DeleteVariableCollection { id, collection } => match dir {
                Dir::Apply => {
                    ctx.variables.collections.remove(id);
                    Ok(())
                }
                Dir::Revert => {
                    ctx.variables
                        .collections
                        .insert(*id, (**collection).clone());
                    Ok(())
                }
            },
            Self::AddMode { collection, mode } => match dir {
                Dir::Apply => {
                    if let Some(c) = ctx.variables.collections.get_mut(collection) {
                        c.modes.push(mode.clone());
                    }
                    Ok(())
                }
                Dir::Revert => {
                    if let Some(c) = ctx.variables.collections.get_mut(collection) {
                        c.modes.pop();
                    }
                    Ok(())
                }
            },
            Self::RemoveMode {
                collection,
                index,
                mode,
            } => match dir {
                Dir::Apply => {
                    if let Some(c) = ctx.variables.collections.get_mut(collection) {
                        if *index < c.modes.len() {
                            c.modes.remove(*index);
                        }
                    }
                    Ok(())
                }
                Dir::Revert => {
                    if let Some(c) = ctx.variables.collections.get_mut(collection) {
                        let at = (*index).min(c.modes.len());
                        c.modes.insert(at, mode.clone());
                    }
                    Ok(())
                }
            },
            Self::CreateVariable { variable } => match dir {
                Dir::Apply => {
                    ctx.variables
                        .variables
                        .insert(variable.id, (**variable).clone());
                    if let Some(collection) =
                        ctx.variables.collections.get_mut(&variable.collection)
                        && !collection.variable_order.contains(&variable.id)
                    {
                        collection.variable_order.push(variable.id);
                    }
                    Ok(())
                }
                Dir::Revert => {
                    ctx.variables.variables.remove(&variable.id);
                    if let Some(collection) =
                        ctx.variables.collections.get_mut(&variable.collection)
                    {
                        collection.variable_order.retain(|id| *id != variable.id);
                    }
                    Ok(())
                }
            },
            Self::DeleteVariable { id, variable } => match dir {
                Dir::Apply => {
                    ctx.variables.variables.remove(id);
                    Ok(())
                }
                Dir::Revert => {
                    ctx.variables.variables.insert(*id, (**variable).clone());
                    Ok(())
                }
            },
            Self::SetVariableValue {
                variable,
                mode,
                old,
                new,
            } => {
                if let Some(v) = ctx.variables.variables.get_mut(variable) {
                    match dir.pick(old, new) {
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
            Self::RenameVariable { id, old, new } => {
                if let Some(v) = ctx.variables.variables.get_mut(id) {
                    v.name = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::RenameVariableCollection { id, old, new } => {
                if let Some(c) = ctx.variables.collections.get_mut(id) {
                    c.name = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::RenameMode {
                collection,
                mode,
                old,
                new,
            } => {
                if let Some(c) = ctx.variables.collections.get_mut(collection) {
                    if let Some(m) = c.modes.iter_mut().find(|m| m.id == *mode) {
                        m.name = dir.pick(old, new).clone();
                    }
                }
                Ok(())
            }

            // ---- bindings ---------------------------------------------------
            Self::BindProperty {
                node,
                prop,
                old,
                new,
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                match dir {
                    Dir::Apply => {
                        n.bindings.insert(*prop, *new);
                    }
                    // The prior binding may have been absent, so reverting can
                    // mean *removing* the binding, not restoring another.
                    Dir::Revert => match old {
                        Some(v) => {
                            n.bindings.insert(*prop, *v);
                        }
                        None => {
                            n.bindings.remove(prop);
                        }
                    },
                }
                Ok(())
            }
            Self::UnbindProperty {
                node,
                prop,
                variable,
                old_data,
                new_data,
                old_opacity,
                new_opacity,
                old_flags,
                new_flags,
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                match dir {
                    Dir::Apply => {
                        n.bindings.remove(prop);
                        n.data = (**new_data).clone();
                        n.opacity = *new_opacity;
                        n.flags = *new_flags;
                    }
                    Dir::Revert => {
                        n.bindings.insert(*prop, *variable);
                        n.data = (**old_data).clone();
                        n.opacity = *old_opacity;
                        n.flags = *old_flags;
                    }
                }
                Ok(())
            }
            Self::SetActiveMode {
                scope,
                collection,
                old,
                new,
            } => set_active_mode(ctx, scope, *collection, *dir.pick(old, new)),

            // ---- motion -----------------------------------------------------
            Self::CreateAnimationClip { clip } => match dir {
                Dir::Apply => {
                    ctx.motion.clips.insert(clip.id, (**clip).clone());
                    Ok(())
                }
                Dir::Revert => {
                    ctx.motion.clips.remove(&clip.id);
                    Ok(())
                }
            },
            Self::DeleteAnimationClip { id, clip } => match dir {
                Dir::Apply => {
                    ctx.motion.clips.remove(id);
                    Ok(())
                }
                Dir::Revert => {
                    ctx.motion.clips.insert(*id, (**clip).clone());
                    Ok(())
                }
            },
            Self::SetAnimationClipName { id, old, new } => {
                if let Some(clip) = ctx.motion.clips.get_mut(id) {
                    clip.name = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::SetAnimationClipDuration { id, old, new } => {
                if let Some(clip) = ctx.motion.clips.get_mut(id) {
                    clip.duration_ms = *dir.pick(old, new);
                }
                Ok(())
            }
            Self::SetAnimationTrack {
                clip,
                track,
                old,
                new,
            } => {
                if let Some(clip) = ctx.motion.clips.get_mut(clip) {
                    match dir.pick(old, new) {
                        Some(value) => {
                            let mut value = (**value).clone();
                            value.id = *track;
                            clip.tracks.insert(*track, value);
                        }
                        None => {
                            clip.tracks.remove(track);
                        }
                    }
                }
                Ok(())
            }
            Self::SetKeyframe {
                clip,
                track,
                target,
                keyframe,
                old,
                new,
            } => {
                let Some(track) = ctx
                    .motion
                    .clips
                    .get_mut(clip)
                    .and_then(|clip| clip.tracks.get_mut(track))
                else {
                    return Ok(());
                };
                if track.target != *target {
                    return Err(SceneError::InvariantViolated(format!(
                        "animation track target mismatch for {}",
                        track.id
                    )));
                }
                match dir.pick(old, new) {
                    Some(value) => {
                        let mut value = value.clone();
                        value.id = *keyframe;
                        track.keyframes.insert(*keyframe, value);
                    }
                    None => {
                        track.keyframes.remove(keyframe);
                    }
                }
                Ok(())
            }

            // ---- prototyping ------------------------------------------------
            Self::AddReaction { node, reaction } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                match dir {
                    Dir::Apply => n.reactions.push(reaction.clone()),
                    Dir::Revert => {
                        n.reactions.pop();
                    }
                }
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
                match dir {
                    Dir::Apply => {
                        if *index < n.reactions.len() {
                            n.reactions.remove(*index);
                        }
                    }
                    Dir::Revert => {
                        let at = (*index).min(n.reactions.len());
                        n.reactions.insert(at, reaction.clone());
                    }
                }
                Ok(())
            }
            Self::SetReaction {
                node,
                index,
                old,
                new,
            } => {
                let n = ctx
                    .scene
                    .get_mut(*node)
                    .ok_or(SceneError::NotFound(*node))?;
                if let Some(slot) = n.reactions.get_mut(*index) {
                    *slot = dir.pick(old, new).clone();
                }
                Ok(())
            }
            Self::SetFlowStart { old, new } => {
                *ctx.flow_start = *dir.pick(old, new);
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

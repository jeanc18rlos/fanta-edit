//! Variable / mode resolution: pick the mode in force for a collection at a
//! node ([`resolve_effective_mode`]), and chase a bound variable (with a cycle
//! guard, across collections) down to a concrete value ([`resolve_bound_value`]).

use crate::id::{ModeId, NodeId, VariableCollectionId, VariableId};
use crate::node::{CanvasNode, NodeData};
use crate::scene::Scene;
use crate::value::{ResolvedVarValue, VarValue};
use crate::variables::{VariableCollection, VariableRegistry};
use std::collections::BTreeMap;

/// The mode in force for `collection` when resolving a binding on `node_id`.
///
/// Precedence (highest first): the nearest-ancestor-or-self frame that pins this
/// collection via [`GroupNode::explicit_modes`]; then the document-level
/// [`active_modes`] entry; then the collection's `default_mode`. A pinned mode
/// that isn't actually one of the collection's modes is ignored (defends against
/// a stale pin after a mode is deleted).
///
/// [`GroupNode::explicit_modes`]: crate::node::GroupNode::explicit_modes
/// [`active_modes`]: crate::doc::Doc::active_modes
pub fn resolve_effective_mode(
    scene: &Scene,
    node_id: NodeId,
    collection: &VariableCollection,
    doc_active_modes: &BTreeMap<VariableCollectionId, ModeId>,
) -> ModeId {
    // Self, then ancestors, looking for a frame that pins this collection.
    if let Some(node) = scene.get(node_id) {
        if let Some(m) = pinned_mode(node, collection) {
            return m;
        }
    }
    for anc in scene.ancestors_of(node_id) {
        if let Some(m) = pinned_mode(anc, collection) {
            return m;
        }
    }
    // Document-level active mode, then the collection default.
    if let Some(m) = doc_active_modes.get(&collection.id) {
        if collection.has_mode(*m) {
            return *m;
        }
    }
    collection.default_mode
}

/// The mode `node` pins for `collection`, if it is a frame whose
/// `explicit_modes` names a real mode of the collection.
fn pinned_mode(node: &CanvasNode, collection: &VariableCollection) -> Option<ModeId> {
    if let NodeData::Group(g) = &node.data {
        if let Some(m) = g.explicit_modes.get(&collection.id) {
            if collection.has_mode(*m) {
                return Some(*m);
            }
        }
    }
    None
}

/// Resolve a bound variable to a concrete value, as seen from `node_id`.
///
/// Looks up the variable, resolves *its* collection's effective mode at
/// `node_id`, reads the per-mode value, and — if that value is itself an
/// [`VarValue::Alias`] — chases it, **re-resolving the aliased variable's own
/// collection mode at the same node** (so a cross-collection alias picks the
/// right mode, not the source collection's). A reference cycle yields `None`
/// rather than looping. A missing variable / collection / per-mode value also
/// yields `None` (dangling refs are tolerated, never panic).
pub fn resolve_bound_value(
    registry: &VariableRegistry,
    scene: &Scene,
    node_id: NodeId,
    doc_active_modes: &BTreeMap<VariableCollectionId, ModeId>,
    var_id: VariableId,
) -> Option<ResolvedVarValue> {
    let mut visited: Vec<VariableId> = Vec::new();
    resolve_var(
        registry,
        scene,
        node_id,
        doc_active_modes,
        var_id,
        &mut visited,
    )
}

fn resolve_var(
    registry: &VariableRegistry,
    scene: &Scene,
    node_id: NodeId,
    doc_active_modes: &BTreeMap<VariableCollectionId, ModeId>,
    var_id: VariableId,
    visited: &mut Vec<VariableId>,
) -> Option<ResolvedVarValue> {
    if visited.contains(&var_id) {
        return None; // alias cycle — bail rather than loop forever
    }
    visited.push(var_id);

    let var = registry.variable(var_id)?;
    let collection = registry.collections.get(&var.collection)?;
    let mode = resolve_effective_mode(scene, node_id, collection, doc_active_modes);
    // The value for the effective mode, falling back to the default mode's value
    // if this variable has no entry for the effective mode.
    let value = var
        .value_for_mode(mode)
        .or_else(|| var.value_for_mode(collection.default_mode))?;

    match value {
        VarValue::Alias { variable } => resolve_var(
            registry,
            scene,
            node_id,
            doc_active_modes,
            *variable,
            visited,
        ),
        concrete => concrete.as_resolved(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::node::{GroupNode, NodeData, VectorNode};
    use crate::value::VariableType;
    use crate::variables::{Mode, Variable, VariableCollection};
    use std::collections::BTreeMap;

    /// A registry with one "Theme" collection (Light default, Dark) and a `bg`
    /// color variable: white in Light, black in Dark.
    fn theme_registry() -> (
        VariableRegistry,
        VariableCollectionId,
        ModeId,
        ModeId,
        VariableId,
    ) {
        let coll = VariableCollectionId::new();
        let light = ModeId::new();
        let dark = ModeId::new();
        let bg = VariableId::new();
        let mut reg = VariableRegistry::new();
        reg.collections.insert(
            coll,
            VariableCollection {
                id: coll,
                name: "Theme".into(),
                modes: vec![
                    Mode {
                        id: light,
                        name: "Light".into(),
                    },
                    Mode {
                        id: dark,
                        name: "Dark".into(),
                    },
                ],
                default_mode: light,
                variable_order: vec![bg],
            },
        );
        reg.variables.insert(
            bg,
            Variable {
                id: bg,
                collection: coll,
                name: "bg".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([
                    (
                        light,
                        VarValue::Color {
                            value: Color::WHITE,
                        },
                    ),
                    (
                        dark,
                        VarValue::Color {
                            value: Color::BLACK,
                        },
                    ),
                ]),
                scopes: Vec::new(),
            },
        );
        (reg, coll, light, dark, bg)
    }

    #[test]
    fn bound_value_follows_doc_active_mode() {
        let (reg, coll, _light, dark, bg) = theme_registry();
        let scene = Scene::new();
        let node = NodeId::new(); // not in scene: only doc/default modes apply
        // Default mode (Light) → white.
        let v = resolve_bound_value(&reg, &scene, node, &BTreeMap::new(), bg);
        assert_eq!(
            v,
            Some(ResolvedVarValue::Color {
                value: Color::WHITE
            })
        );
        // Doc active mode Dark → black.
        let active = BTreeMap::from([(coll, dark)]);
        let v = resolve_bound_value(&reg, &scene, node, &active, bg);
        assert_eq!(
            v,
            Some(ResolvedVarValue::Color {
                value: Color::BLACK
            })
        );
    }

    #[test]
    fn frame_explicit_mode_overrides_doc_mode() {
        let (reg, coll, light, dark, bg) = theme_registry();
        let mut scene = Scene::new();
        // A frame pinning Dark, with a child node that reads the binding.
        let frame = CanvasNode::new(NodeData::Group(GroupNode {
            clip_size: Some([10.0, 10.0]),
            background: None,
            explicit_modes: BTreeMap::from([(coll, dark)]),
            ..Default::default()
        }));
        let frame_id = frame.id;
        scene.insert(frame).unwrap();
        let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            0.0,
            0.0,
            5.0,
            5.0,
            Color::WHITE,
        )));
        child.parent = Some(frame_id);
        let child_id = child.id;
        scene.insert(child).unwrap();

        // Doc says Light, but the ancestor frame pins Dark → black wins.
        let active = BTreeMap::from([(coll, light)]);
        let v = resolve_bound_value(&reg, &scene, child_id, &active, bg);
        assert_eq!(
            v,
            Some(ResolvedVarValue::Color {
                value: Color::BLACK
            })
        );
        let _ = frame_id;
    }

    #[test]
    fn alias_cycle_resolves_to_none() {
        let coll = VariableCollectionId::new();
        let mode = ModeId::new();
        let a = VariableId::new();
        let b = VariableId::new();
        let mut reg = VariableRegistry::new();
        reg.collections.insert(
            coll,
            VariableCollection {
                id: coll,
                name: "C".into(),
                modes: vec![Mode {
                    id: mode,
                    name: "M".into(),
                }],
                default_mode: mode,
                variable_order: vec![a, b],
            },
        );
        // a → b → a (cycle).
        reg.variables.insert(
            a,
            Variable {
                id: a,
                collection: coll,
                name: "a".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(mode, VarValue::Alias { variable: b })]),
                scopes: Vec::new(),
            },
        );
        reg.variables.insert(
            b,
            Variable {
                id: b,
                collection: coll,
                name: "b".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(mode, VarValue::Alias { variable: a })]),
                scopes: Vec::new(),
            },
        );
        let scene = Scene::new();
        assert_eq!(
            resolve_bound_value(&reg, &scene, NodeId::new(), &BTreeMap::new(), a),
            None
        );
    }
}

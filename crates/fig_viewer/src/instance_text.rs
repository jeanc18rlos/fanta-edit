//! On-canvas editing of TEXT that lives inside a component instance.
//!
//! An instance's descendants are VIRTUAL: [`expand_instance`] deep-clones the
//! master subtree with fresh ids and applies the instance's sparse overrides —
//! the clones never enter the [`Scene`](fanta_doc::Scene). So the plain text
//! editor in [`crate::text_edit`] (which writes `scene.get_mut(node)`) cannot
//! touch them. Figma models an edit to text inside an instance as an OVERRIDE
//! on the instance: a `Text` override sets the content, a `Fills` override on
//! the text node sets its glyph color (the engine's `apply_override` routes the
//! first solid fill onto `TextNode::style.color`).
//!
//! This module (a) finds the text clone under a world point, reconstructing its
//! world transform EXACTLY as the renderer composes it (`W_inst ∘ T₁ ∘ … ∘
//! T_target`, the expansion root's own transform suppressed — see
//! `render_expanded`), and (b) builds the add-or-replace [`SetInstanceOverride`]
//! operations, plus the transient override writes that drive the live preview.
//!
//! [`expand_instance`]: fanta_doc::expand_instance
//! [`SetInstanceOverride`]: fanta_doc::Operation::SetInstanceOverride

use std::collections::{BTreeMap, HashMap};

use fanta_doc::{
    BoundProp, Color, Doc, ExpandedNode, Fill, InstanceExpansionContext, NodeData, NodeFlags,
    NodeId, Operation, Override, OverridePath, OverrideValue, TextNode, Transform2D,
    expand_instance_with_context,
};
use glam::DVec2;
use smallvec::smallvec;

/// A text clone inside an instance, addressed for override editing.
pub(crate) struct InstanceTextTarget {
    /// The instance node — a real [`Scene`](fanta_doc::Scene) node whose
    /// `overrides` an edit writes.
    pub(crate) instance_id: NodeId,
    /// Def-local path (master ids, root-first, root excluded) of the text clone
    /// — the [`Override::target_path`] an edit addresses.
    pub(crate) def_path: OverridePath,
    /// The resolved text clone: content + style already reflect any existing
    /// overrides and Figma's baked derived data. Seeds the edit session and
    /// (with `world`) the caret/selection geometry.
    pub(crate) text: TextNode,
    /// Absolute world transform of the clone (`W_inst ∘ T₁ ∘ … ∘ T_target`),
    /// projecting the clone's node-local geometry to world space exactly as the
    /// renderer paints it.
    pub(crate) world: Transform2D,
}

/// Expand `instance_id` the way the renderer does — deep-clone + overrides, then
/// the auto-layout solve ONLY when the instance carries no baked derived data
/// (matching `expand_instance_memoized`, which skips the solve for a `.fig`'s
/// pre-resolved `derivedSymbolData`). Returns the resolved clones plus the
/// instance's absolute world transform `W_inst`. `None` if the node is not an
/// instance, its master is gone (empty expansion), or it has no world transform.
fn expand_resolved(doc: &Doc, instance_id: NodeId) -> Option<(Vec<ExpandedNode>, Transform2D)> {
    let node = doc.scene.get(instance_id)?;
    let NodeData::Instance(instance) = &node.data else {
        return None;
    };
    let world = doc.scene.world_transform(instance_id)?;
    let context = InstanceExpansionContext::new(&doc.variables, &doc.active_modes, instance_id);
    let mut expanded =
        expand_instance_with_context(&doc.scene, &doc.components, instance, &context);
    if expanded.is_empty() {
        return None;
    }
    if instance.derived.is_empty() {
        fanta_doc::solve_expanded(&mut expanded, &mut fanta_render::measure_text_node);
    }
    Some((expanded, world))
}

/// Whether the clone at `idx` (or any of its clone ancestors) is hidden or
/// fully transparent — the renderer skips those, so the hit-test must too, or
/// a click would open an edit session on text that is never painted.
fn clone_is_painted(expanded: &[ExpandedNode], by_id: &HashMap<NodeId, usize>, idx: usize) -> bool {
    let mut cursor = idx;
    loop {
        let node = &expanded[cursor].node;
        if node.flags.contains(NodeFlags::HIDDEN) || node.opacity.get() <= 0.0 {
            return false;
        }
        let Some(parent) = node.parent else {
            return true;
        };
        let Some(&parent_idx) = by_id.get(&parent) else {
            return true;
        };
        cursor = parent_idx;
    }
}

pub(crate) fn text_target_at(
    doc: &Doc,
    instance_id: NodeId,
    world_point: DVec2,
) -> Option<InstanceTextTarget> {
    let (expanded, world) = expand_resolved(doc, instance_id)?;
    let paths = expanded
        .iter()
        .map(|entry| (entry.node.id, entry.def_path.clone()))
        .collect::<HashMap<_, _>>();
    let mut scene = fanta_doc::Scene::new();
    let nodes = expanded.into_iter().map(|entry| {
        let mut node = entry.node;
        // Instance rendering suppresses the master root's placement, and its
        // invisible subtrees must not occlude another virtual text layer.
        if node.parent.is_none() {
            node.transform = world;
        }
        if node.opacity.get() <= 0.0 {
            node.flags.insert(NodeFlags::HIDDEN);
        }
        node
    });
    if let Err(error) = scene.insert_many(nodes) {
        log::warn!("could not hit-test component text: {error}");
        return None;
    }
    let hit = scene
        .deep_hits_where(world_point, |id| {
            let Some(node) = scene.get(id) else {
                return false;
            };
            !node.flags.contains(NodeFlags::LOCKED)
                && !scene.ancestors_of(id).any(|ancestor| {
                    ancestor.flags.contains(NodeFlags::LOCKED)
                        || !crate::canvas::inspect_descendants_visible_at(
                            &scene,
                            ancestor,
                            world_point,
                        )
                })
                && crate::canvas::inspect_node_contains_point(&scene, node, world_point)
        })
        .into_iter()
        .next()?;
    let NodeData::Text(text) = &scene.get(hit)?.data else {
        return None;
    };
    Some(InstanceTextTarget {
        instance_id,
        def_path: paths.get(&hit)?.clone(),
        text: text.clone(),
        world: scene.world_transform(hit)?,
    })
}

/// Every painted TEXT clone inside `instance_id`, in paint (DFS) order, as
/// `(def_path, master node name, resolved content)` — the rows the properties
/// panel shows as editable Content fields, Figma-style.
pub(crate) fn text_clones(doc: &Doc, instance_id: NodeId) -> Vec<(OverridePath, String, String)> {
    let Some((expanded, _)) = expand_resolved(doc, instance_id) else {
        return Vec::new();
    };
    let by_id: HashMap<NodeId, usize> = expanded
        .iter()
        .enumerate()
        .map(|(i, e)| (e.node.id, i))
        .collect();
    expanded
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            let NodeData::Text(text) = &entry.node.data else {
                return None;
            };
            if !clone_is_painted(&expanded, &by_id, idx) {
                return None;
            }
            Some((
                entry.def_path.clone(),
                entry.node.name.clone(),
                text.content.clone(),
            ))
        })
        .collect()
}

pub(crate) fn content_edit_errors(
    doc: &Doc,
    instance_id: NodeId,
) -> BTreeMap<OverridePath, String> {
    let Some(node) = doc.scene.get(instance_id) else {
        return BTreeMap::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return BTreeMap::new();
    };
    let context = InstanceExpansionContext::new(&doc.variables, &doc.active_modes, instance_id);
    fanta_doc::instance_bindings_with_context(&doc.scene, &doc.components, instance, &context)
        .into_iter()
        .filter_map(|(path, bindings)| {
            let variable = bindings.get(&BoundProp::TextContent)?;
            let name = doc.variables.variables.get(variable).map(|variable| variable.name.as_str());
            let source = name.filter(|name| !name.is_empty()).map_or_else(
                || "a variable".to_string(),
                |name| format!("the variable ‘{name}’"),
            );
            Some((path, format!("This text is controlled by {source}. Edit the variable, or remove its text binding before editing this instance.")))
        })
        .collect()
}

pub(crate) fn content_edit_error(
    doc: &Doc,
    instance_id: NodeId,
    path: &OverridePath,
) -> Option<String> {
    content_edit_errors(doc, instance_id).remove(path)
}

// ---------------------------------------------------------------------------
// Override construction: preview (transient) and commit (undoable op)
// ---------------------------------------------------------------------------

/// The index of the override at `path`/`prop` in `overrides`, if present.
fn override_index(overrides: &[Override], path: &OverridePath, prop: &BoundProp) -> Option<usize> {
    if *prop == BoundProp::TextContent {
        // Generic fields can clear bindings or carry unrelated authored data.
        // Keep those payloads, and place edited text after the last one so it wins.
        let last_field = overrides.iter().rposition(|entry| {
            entry.target_path == *path && matches!(&entry.value, OverrideValue::Field { .. })
        });
        return overrides
            .iter()
            .enumerate()
            .rfind(|(index, entry)| {
                entry.target_path == *path
                    && entry.target_prop == *prop
                    && matches!(&entry.value, OverrideValue::Text { .. })
                    && last_field.is_none_or(|last| *index > last)
            })
            .map(|(index, _)| index);
    }
    overrides
        .iter()
        .position(|entry| entry.target_path == *path && entry.target_prop == *prop)
}

/// A `Text` (content) override for the clone at `def_path`.
fn text_content_override(def_path: &OverridePath, content: &str) -> Override {
    Override {
        target_path: def_path.clone(),
        target_prop: BoundProp::TextContent,
        value: OverrideValue::Text {
            value: content.to_string(),
        },
    }
}

/// A `Fills` override carrying a single solid glyph color for the clone at
/// `def_path`. `apply_override` maps the first solid fill onto the text node's
/// `style.color`, so this is how instance text color is pinned.
fn text_color_override(def_path: &OverridePath, color: Color) -> Override {
    Override {
        target_path: def_path.clone(),
        target_prop: BoundProp::FillColor { index: 0 },
        value: OverrideValue::Fills {
            fills: smallvec![Fill::solid(color)],
        },
    }
}

/// Upsert `new_override` into `overrides` (replace the matching path/prop entry,
/// else append). Used to build the previewed override set.
fn upsert(overrides: &mut Vec<Override>, new_override: Override) {
    match override_index(
        overrides,
        &new_override.target_path,
        &new_override.target_prop,
    ) {
        Some(i) => overrides[i] = new_override,
        None => overrides.push(new_override),
    }
}

/// The ordered [`Operation::SetInstanceOverride`]s committing a content edit
/// (always, when `content` differs) and, when `color` is `Some`, a glyph-color
/// edit — with indices computed against the *evolving* override vec so two
/// fresh appends don't collide on the same slot. Applied in order, they leave
/// `instance_id` with the previewed content + color.
pub(crate) fn commit_ops(
    doc: &Doc,
    instance_id: NodeId,
    def_path: &OverridePath,
    content: Option<&str>,
    color: Option<Color>,
) -> Vec<Operation> {
    if content.is_some() && content_edit_error(doc, instance_id, def_path).is_some() {
        return Vec::new();
    }
    let Some(node) = doc.scene.get(instance_id) else {
        return Vec::new();
    };
    let NodeData::Instance(instance) = &node.data else {
        return Vec::new();
    };
    // Simulate the vec as each op lands so a second append targets the next
    // slot, not the one the first append just filled.
    let mut working = instance.overrides.clone();
    let mut ops = Vec::new();
    let push_upsert = |ops: &mut Vec<Operation>, working: &mut Vec<Override>, ov: Override| {
        let existing = override_index(working, &ov.target_path, &ov.target_prop);
        let (index, old) = match existing {
            Some(i) => (i, Some(working[i].clone())),
            None => (working.len(), None),
        };
        ops.push(Operation::SetInstanceOverride {
            id: instance_id,
            index,
            old,
            new: Some(ov.clone()),
        });
        upsert(working, ov);
    };
    if let Some(content) = content {
        push_upsert(
            &mut ops,
            &mut working,
            text_content_override(def_path, content),
        );
    }
    if let Some(color) = color {
        push_upsert(&mut ops, &mut working, text_color_override(def_path, color));
    }
    ops
}

pub(crate) fn reset_content_ops(
    doc: &Doc,
    instance_id: NodeId,
    path: &OverridePath,
) -> Vec<Operation> {
    if content_edit_error(doc, instance_id, path).is_some() {
        return Vec::new();
    }
    snapshot_overrides(doc, instance_id)
        .into_iter()
        .enumerate()
        .rev()
        .filter_map(|(index, entry)| {
            (entry.target_path == *path
                && entry.target_prop == BoundProp::TextContent
                && matches!(&entry.value, OverrideValue::Text { .. }))
            .then(|| Operation::SetInstanceOverride {
                id: instance_id,
                index,
                old: Some(entry),
                new: None,
            })
        })
        .collect()
}

/// The instance's current override vec (its pre-edit state), captured at session
/// open so the preview can be rewound before the undoable commit — mirroring the
/// real-text editor's rewind-then-`ReplaceData` staging.
pub(crate) fn snapshot_overrides(doc: &Doc, instance_id: NodeId) -> Vec<Override> {
    doc.scene
        .get(instance_id)
        .and_then(|node| match &node.data {
            NodeData::Instance(instance) => Some(instance.overrides.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Transiently (bypassing history) set the instance's override vec so the
/// renderer re-expands with the previewed content/color. The caller pairs this
/// with `invalidate_canvas_cache()` because a `get_mut` write does not bump the
/// scene revision the surface cache keys on.
pub(crate) fn set_overrides_transient(
    doc: &mut Doc,
    instance_id: NodeId,
    overrides: Vec<Override>,
) {
    if let Some(node) = doc.scene.get_mut(instance_id) {
        if let NodeData::Instance(instance) = &mut node.data {
            instance.overrides = overrides;
        }
    }
}

pub(crate) fn rewind_overrides_preview(
    doc: &mut Doc,
    instance_id: NodeId,
    base: &[Override],
    preview: &[Override],
) {
    let mut current = snapshot_overrides(doc, instance_id);
    // A binding can arrive after the last successful preview. Reverting the
    // entire vector would erase that payload while cancelling the rejected draft.
    for (index, value) in preview.iter().enumerate().rev() {
        if base.get(index) == Some(value) || current.get(index) != Some(value) {
            continue;
        }
        if let Some(original) = base.get(index) {
            if let Some(current) = current.get_mut(index) {
                *current = original.clone();
            }
        } else {
            current.remove(index);
        }
    }
    set_overrides_transient(doc, instance_id, current);
}

/// The override vec for a live preview: `base` (the pre-edit overrides) with the
/// text-content override upserted, and the glyph-color override upserted when
/// `color` differs from the master's resolved color. `base` is the snapshot from
/// [`snapshot_overrides`] so concurrent overrides on other descendants survive.
pub(crate) fn preview_overrides(
    base: &[Override],
    def_path: &OverridePath,
    content: &str,
    color: Option<Color>,
) -> Vec<Override> {
    let mut overrides = base.to_vec();
    upsert(&mut overrides, text_content_override(def_path, content));
    if let Some(color) = color {
        upsert(&mut overrides, text_color_override(def_path, color));
    }
    overrides
}

#[cfg(test)]
mod tests {
    use super::*;
    use fanta_doc::{CanvasNode, ComponentDef, ComponentId, GroupNode, InstanceNode, Operation};
    use std::collections::BTreeMap;

    /// Build a doc holding a component master (a group root with one text child
    /// at a known local offset) plus one instance of it at a known position.
    /// Returns the doc, the instance node id, and the text child's def-local
    /// path (`[text_child_id]`).
    fn doc_with_instance() -> (Doc, NodeId, OverridePath) {
        let mut doc = Doc::new();

        // Master root group.
        let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        root.transform = Transform2D::translation(1000.0, 1000.0); // off on the Components page
        doc.apply(Operation::create_node(root)).expect("root");

        // Text child, offset (10, 20) inside the master, box 120x30.
        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Master", 120.0, 30.0)));
        let text_id = text.id;
        text.parent = Some(root_id);
        text.transform = Transform2D::translation(10.0, 20.0);
        doc.apply(Operation::create_node(text)).expect("text");

        // Register the master as a component.
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Card"));

        // Place an instance at world (500, 300).
        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [120.0, 30.0],
        }));
        let instance_id = instance.id;
        instance.transform = Transform2D::translation(500.0, 300.0);
        doc.apply(Operation::create_node(instance))
            .expect("instance");
        doc.add_page(instance_id);

        let def_path: OverridePath = smallvec![text_id];
        (doc, instance_id, def_path)
    }

    struct ResolvedTextFixture {
        doc: Doc,
        instance: NodeId,
        master: NodeId,
        text: NodeId,
        frame: NodeId,
        path: OverridePath,
        collection: fanta_doc::VariableCollectionId,
        modes: [fanta_doc::ModeId; 2],
        caption: fanta_doc::VariableId,
        alias: fanta_doc::VariableId,
    }

    fn resolved_text_variable(
        fixture: &mut ResolvedTextFixture,
        name: &str,
        ty: fanta_doc::VariableType,
        values: [fanta_doc::VarValue; 2],
    ) -> fanta_doc::VariableId {
        let id = fanta_doc::VariableId::new();
        fixture.doc.variables.variables.insert(
            id,
            fanta_doc::Variable {
                id,
                collection: fixture.collection,
                name: name.into(),
                ty,
                values_by_mode: fixture.modes.into_iter().zip(values).collect(),
                scopes: Vec::new(),
            },
        );
        fixture
            .doc
            .variables
            .collections
            .get_mut(&fixture.collection)
            .expect("collection")
            .variable_order
            .push(id);
        id
    }

    fn resolved_text_fixture(alias_as_default: bool) -> ResolvedTextFixture {
        use fanta_doc::{
            ComponentPropDef, ComponentPropId, ComponentPropKind, Mode, ModeId, PropBindingTarget,
            VarValue, VariableCollection, VariableCollectionId, VariableType,
        };
        let (mut doc, instance, path) = doc_with_instance();
        let text = *path.last().expect("text path");
        let master = doc.scene.get(text).expect("text").parent.expect("master");
        let collection = VariableCollectionId::new();
        let modes = [ModeId::new(), ModeId::new()];
        doc.variables.collections.insert(
            collection,
            VariableCollection {
                id: collection,
                name: "Placed modes".into(),
                modes: modes
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| Mode {
                        id,
                        name: format!("Mode {index}"),
                    })
                    .collect(),
                default_mode: modes[0],
                variable_order: Vec::new(),
            },
        );
        let mut frame = CanvasNode::new(NodeData::Group(GroupNode::default()));
        frame.transform = Transform2D::translation(100.0, 50.0);
        let frame = doc.scene.insert(frame).expect("placed frame");
        doc.scene
            .set_parent(instance, Some(frame), fanta_doc::IndexKey::FIRST)
            .expect("instance parent");
        doc.remove_page(instance).expect("old test page");
        doc.add_page(frame);
        doc.set_active_page(Some(frame));
        let NodeData::Group(group) = &mut doc.scene.get_mut(master).expect("master").data else {
            panic!("master group");
        };
        group.clip_size = Some([240.0, 96.0]);
        group.explicit_modes.insert(collection, modes[0]);
        let mut fixture = ResolvedTextFixture {
            doc,
            instance,
            master,
            text,
            frame,
            path,
            collection,
            modes,
            caption: fanta_doc::VariableId::new(),
            alias: fanta_doc::VariableId::new(),
        };
        fixture.caption = resolved_text_variable(
            &mut fixture,
            "Caption",
            VariableType::String,
            [
                VarValue::String {
                    value: "Visible A".into(),
                },
                VarValue::String {
                    value: "Visible B".into(),
                },
            ],
        );
        let caption = fixture.caption;
        fixture.alias = resolved_text_variable(
            &mut fixture,
            "Caption alias",
            VariableType::String,
            [
                VarValue::Alias { variable: caption },
                VarValue::Alias { variable: caption },
            ],
        );
        let property = ComponentPropId::new();
        let NodeData::Instance(placed) =
            &mut fixture.doc.scene.get_mut(instance).expect("instance").data
        else {
            panic!("instance");
        };
        placed.local_size = [240.0, 96.0];
        let component = placed.component;
        if !alias_as_default {
            placed.prop_values.insert(
                property,
                VarValue::Alias {
                    variable: fixture.alias,
                },
            );
        }
        fixture
            .doc
            .components
            .defs
            .get_mut(&component)
            .expect("definition")
            .props
            .push(ComponentPropDef {
                id: property,
                name: "Caption".into(),
                kind: ComponentPropKind::Text,
                formatter: Default::default(),
                default: if alias_as_default {
                    VarValue::Alias {
                        variable: fixture.alias,
                    }
                } else {
                    VarValue::String {
                        value: "Default caption".into(),
                    }
                },
                bindings: vec![PropBindingTarget {
                    path: fixture.path.clone(),
                    prop: BoundProp::TextContent,
                }],
            });
        fixture.doc.history = Default::default();
        fixture
    }

    fn set_text_modes(fixture: &mut ResolvedTextFixture, active: usize, pin: Option<usize>) {
        fixture
            .doc
            .active_modes
            .insert(fixture.collection, fixture.modes[active]);
        let NodeData::Group(frame) = &mut fixture
            .doc
            .scene
            .get_mut(fixture.frame)
            .expect("frame")
            .data
        else {
            panic!("frame");
        };
        frame.explicit_modes.clear();
        if let Some(pin) = pin {
            frame
                .explicit_modes
                .insert(fixture.collection, fixture.modes[pin]);
        }
    }

    #[test]
    fn bound_instance_text_rejects_content_override_without_changing_binding_or_history() {
        let mut fixture = resolved_text_fixture(true);
        fixture
            .doc
            .scene
            .get_mut(fixture.text)
            .expect("master text")
            .bindings
            .insert(BoundProp::TextContent, fixture.alias);
        set_text_modes(&mut fixture, 0, Some(1));
        let before = serde_json::to_value(&fixture.doc).expect("authored document");
        let depth = fixture.doc.history.undo_depth();
        let target = text_target_at(&fixture.doc, fixture.instance, DVec2::new(620.0, 380.0))
            .expect("bound text is still visible and targetable");
        assert_eq!(target.text.content, "Visible B");
        assert!(
            commit_ops(
                &fixture.doc,
                fixture.instance,
                &fixture.path,
                Some("Ignored edit"),
                None
            )
            .is_empty(),
            "do not author an override that the direct variable binding will overwrite"
        );
        assert_eq!(
            serde_json::to_value(&fixture.doc).expect("unchanged document"),
            before
        );
        assert_eq!(fixture.doc.history.undo_depth(), depth);
        assert_eq!(
            text_clones(&fixture.doc, fixture.instance)[0].2,
            "Visible B"
        );
        assert!(
            !commit_ops(
                &fixture.doc,
                fixture.instance,
                &fixture.path,
                None,
                Some(Color::WHITE)
            )
            .is_empty(),
            "the content guard must not remove unrelated color capability"
        );
    }

    #[test]
    fn bound_instance_text_guard_uses_effective_field_overrides_and_preserves_alias_editability() {
        for authored_bound in [false, true] {
            for effective_bound in [false, true] {
                let mut fixture = resolved_text_fixture(true);
                if authored_bound {
                    fixture
                        .doc
                        .scene
                        .get_mut(fixture.text)
                        .expect("master text")
                        .bindings
                        .insert(BoundProp::TextContent, fixture.alias);
                }
                let mut binding_node = fixture.doc.scene.get(fixture.text).expect("text").clone();
                binding_node.bindings.clear();
                if effective_bound {
                    binding_node
                        .bindings
                        .insert(BoundProp::TextContent, fixture.alias);
                }
                let bindings = serde_json::to_value(binding_node)
                    .expect("binding codec")
                    .get("bindings")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                let field = Override {
                    target_path: fixture.path.clone(),
                    target_prop: BoundProp::TextContent,
                    value: OverrideValue::Field {
                        value: serde_json::json!({"bindings": bindings}),
                    },
                };
                let NodeData::Instance(instance) = &mut fixture
                    .doc
                    .scene
                    .get_mut(fixture.instance)
                    .expect("instance")
                    .data
                else {
                    panic!("instance");
                };
                instance.overrides.push(field);
                let before = serde_json::to_value(&fixture.doc).expect("authored state");
                let operations = commit_ops(
                    &fixture.doc,
                    fixture.instance,
                    &fixture.path,
                    Some("Edited"),
                    None,
                );
                assert_eq!(
                    operations.is_empty(),
                    effective_bound,
                    "effective binding must win over raw master status {authored_bound}/{effective_bound}"
                );
                assert_eq!(
                    serde_json::to_value(&fixture.doc).expect("unchanged state"),
                    before
                );
                if !effective_bound {
                    for operation in operations {
                        fixture.doc.apply(operation).expect("editable content");
                    }
                    assert_eq!(text_clones(&fixture.doc, fixture.instance)[0].2, "Edited");
                    assert!(fixture.doc.undo().expect("undo content"));
                    let mut restored = serde_json::to_value(&fixture.doc).expect("undo state");
                    restored["metadata"]["modified_at"] = before["metadata"]["modified_at"].clone();
                    let mut expected = before.clone();
                    restored
                        .as_object_mut()
                        .expect("document object")
                        .remove("history");
                    expected
                        .as_object_mut()
                        .expect("document object")
                        .remove("history");
                    assert_eq!(restored, expected);
                    assert!(!fixture.doc.history.can_undo());
                    assert!(fixture.doc.redo().expect("redo content"));
                    assert_eq!(text_clones(&fixture.doc, fixture.instance)[0].2, "Edited");
                }
            }
        }
        let mut fixture = resolved_text_fixture(true);
        fixture
            .doc
            .scene
            .get_mut(fixture.text)
            .expect("text")
            .bindings
            .insert(BoundProp::TextContent, fixture.alias);
        let field = Override {
            target_path: fixture.path.clone(),
            target_prop: BoundProp::TextContent,
            value: OverrideValue::Field {
                value: serde_json::json!({"bindings": [], "content": "Generic content", "name": "Keep generic name"}),
            },
        };
        let NodeData::Instance(instance) = &mut fixture
            .doc
            .scene
            .get_mut(fixture.instance)
            .expect("instance")
            .data
        else {
            panic!("instance");
        };
        instance.overrides = vec![
            text_content_override(&fixture.path, "Earlier text"),
            field,
        ];
        let original = instance.overrides.clone();
        for operation in commit_ops(
            &fixture.doc,
            fixture.instance,
            &fixture.path,
            Some("Final text"),
            None,
        ) {
            fixture
                .doc
                .apply(operation)
                .expect("append after generic content");
        }
        assert_eq!(
            text_clones(&fixture.doc, fixture.instance)[0].2,
            "Final text"
        );
        let entries = snapshot_overrides(&fixture.doc, fixture.instance);
        assert_eq!(&entries[..original.len()], &original);
        assert_eq!(entries.len(), original.len() + 1);
        let repeated = commit_ops(
            &fixture.doc,
            fixture.instance,
            &fixture.path,
            Some("Final edited twice"),
            None,
        );
        for operation in repeated {
            fixture
                .doc
                .apply(operation)
                .expect("update final typed text");
        }
        assert_eq!(
            snapshot_overrides(&fixture.doc, fixture.instance).len(),
            entries.len()
        );
        assert_eq!(
            text_clones(&fixture.doc, fixture.instance)[0].2,
            "Final edited twice"
        );
        let fixture = resolved_text_fixture(false);
        assert!(
            !commit_ops(
                &fixture.doc,
                fixture.instance,
                &fixture.path,
                Some("Placed override"),
                None
            )
            .is_empty(),
            "a component-property alias is applied before the sparse text override and remains editable"
        );
    }

    #[test]
    fn bound_instance_text_preview_and_commit_recheck_new_and_unresolved_bindings() {
        for missing_variable in [false, true] {
            let mut fixture = resolved_text_fixture(true);
            let target = text_target_at(&fixture.doc, fixture.instance, DVec2::new(620.0, 380.0))
                .expect("editable text");
            let mut session = crate::text_edit::TextEditSession::new_instance(
                target,
                snapshot_overrides(&fixture.doc, fixture.instance),
            );
            session.select_all();
            session.insert("Draft retained");
            fixture
                .doc
                .scene
                .get_mut(fixture.text)
                .expect("master text")
                .bindings
                .insert(
                    BoundProp::TextContent,
                    if missing_variable {
                        fanta_doc::VariableId::new()
                    } else {
                        fixture.alias
                    },
                );
            let before =
                serde_json::to_value(&fixture.doc).expect("binding introduced after opening");
            crate::text_edit::apply_preview(&mut fixture.doc, &session);
            assert_eq!(
                serde_json::to_value(&fixture.doc).expect("preview blocked"),
                before,
                "a newly bound draft must not install an ineffective transient override"
            );
            assert!(crate::text_edit::commit_ops(&fixture.doc, &session).is_empty());
            assert!(
                session.is_changed(),
                "the unsaved user draft remains available for correction/cancel"
            );
            assert_eq!(
                serde_json::to_value(&fixture.doc).expect("commit blocked"),
                before
            );
        }
    }

    #[test]
    fn bound_instance_text_cancel_preserves_late_same_instance_fields() {
        for original_text in [None, Some("Existing typed override")] {
            let mut fixture = resolved_text_fixture(true);
            if let Some(content) = original_text {
                for operation in commit_ops(
                    &fixture.doc,
                    fixture.instance,
                    &fixture.path,
                    Some(content),
                    None,
                ) {
                    fixture.doc.apply(operation).expect("existing content");
                }
            }
            let target = text_target_at(&fixture.doc, fixture.instance, DVec2::new(620.0, 380.0))
                .expect("editable text");
            let base = snapshot_overrides(&fixture.doc, fixture.instance);
            let mut session = crate::text_edit::TextEditSession::new_instance(target, base.clone());
            session.select_all();
            session.insert("Preview first");
            crate::text_edit::apply_preview(&mut fixture.doc, &session);
            let field = Override {
                target_path: fixture.path.clone(),
                target_prop: BoundProp::Opacity,
                value: OverrideValue::Field {
                    value: serde_json::json!({
                        "bindings": [[{"prop": "text_content"}, fixture.alias]],
                        "name": "Keep late payload"
                    }),
                },
            };
            let NodeData::Instance(instance) = &mut fixture
                .doc
                .scene
                .get_mut(fixture.instance)
                .expect("instance")
                .data
            else {
                panic!("instance")
            };
            instance.overrides.push(field.clone());
            let history = serde_json::to_value(&fixture.doc.history).expect("history");
            session.select_all();
            session.insert("Rejected draft");
            crate::text_edit::apply_preview(&mut fixture.doc, &session);
            assert!(crate::text_edit::commit_ops(&fixture.doc, &session).is_empty());
            crate::text_edit::rewind_preview(&mut fixture.doc, &session);
            let mut expected = base;
            expected.push(field);
            assert_eq!(
                snapshot_overrides(&fixture.doc, fixture.instance),
                expected,
                "cancel only owns the last successful preview, not a later Field payload"
            );
            assert_eq!(
                serde_json::to_value(&fixture.doc.history).expect("history"),
                history
            );
            assert_eq!(
                text_clones(&fixture.doc, fixture.instance)[0].2,
                "Visible A"
            );
        }
    }

    #[test]
    fn resolved_instance_text_aliases_use_placed_modes_and_preserve_override_history() {
        for alias_as_default in [false, true] {
            let mut fixture = resolved_text_fixture(alias_as_default);
            let point = DVec2::new(620.0, 380.0);
            for (active, pin, expected) in [
                (0, None, "Visible A"),
                (1, None, "Visible B"),
                (0, Some(1), "Visible B"),
                (1, Some(0), "Visible A"),
            ] {
                set_text_modes(&mut fixture, active, pin);
                let before = serde_json::to_value(&fixture.doc).expect("before targeting");
                let target = text_target_at(&fixture.doc, fixture.instance, point)
                    .expect("painted text is editable");
                assert_eq!(target.text.content, expected);
                assert_eq!(target.instance_id, fixture.instance);
                assert_eq!(target.def_path, fixture.path);
                assert_eq!(
                    target.world.transform_point(DVec2::ZERO),
                    DVec2::new(610.0, 370.0)
                );
                assert_eq!(
                    text_clones(&fixture.doc, fixture.instance)
                        .iter()
                        .map(|(path, _, content)| (path, content.as_str()))
                        .collect::<Vec<_>>(),
                    vec![(&fixture.path, expected)]
                );
                assert_eq!(
                    serde_json::to_value(&fixture.doc).expect("after targeting"),
                    before
                );
            }
            let original_instance = fixture
                .doc
                .scene
                .get(fixture.instance)
                .expect("instance")
                .clone();
            let original_master = fixture
                .doc
                .scene
                .get(fixture.text)
                .expect("master text")
                .clone();
            for operation in commit_ops(
                &fixture.doc,
                fixture.instance,
                &fixture.path,
                Some("Only this instance"),
                None,
            ) {
                fixture.doc.apply(operation).expect("commit text");
            }
            assert_eq!(fixture.doc.history.undo_depth(), 1);
            assert_eq!(
                text_target_at(&fixture.doc, fixture.instance, point)
                    .expect("edited text")
                    .text
                    .content,
                "Only this instance"
            );
            assert_eq!(fixture.doc.scene.get(fixture.text), Some(&original_master));
            assert!(fixture.doc.undo().expect("undo"));
            assert_eq!(
                fixture.doc.scene.get(fixture.instance),
                Some(&original_instance)
            );
            assert_eq!(
                text_target_at(&fixture.doc, fixture.instance, point)
                    .expect("restored text")
                    .text
                    .content,
                "Visible A"
            );
            assert!(fixture.doc.redo().expect("redo"));
            assert_eq!(
                text_target_at(&fixture.doc, fixture.instance, point)
                    .expect("redone text")
                    .text
                    .content,
                "Only this instance"
            );
            assert_eq!(fixture.doc.scene.get(fixture.text), Some(&original_master));
        }
    }

    #[test]
    fn resolved_instance_text_visibility_and_opacity_exclude_unpainted_clones() {
        use fanta_doc::{VarValue, VariableType};
        for property in [BoundProp::Visible, BoundProp::Opacity] {
            for bind_parent in [false, true] {
                let mut fixture = resolved_text_fixture(false);
                let (ty, values) = if property == BoundProp::Visible {
                    (
                        VariableType::Boolean,
                        [
                            VarValue::Boolean { value: true },
                            VarValue::Boolean { value: false },
                        ],
                    )
                } else {
                    (
                        VariableType::Float,
                        [
                            VarValue::Float { value: 1.0 },
                            VarValue::Float { value: 0.0 },
                        ],
                    )
                };
                let variable = resolved_text_variable(&mut fixture, "Painted", ty, values);
                let owner = if bind_parent {
                    fixture.master
                } else {
                    fixture.text
                };
                fixture
                    .doc
                    .scene
                    .get_mut(owner)
                    .expect("binding owner")
                    .bindings
                    .insert(property, variable);
                for (active, pin, visible) in [
                    (0, None, true),
                    (1, None, false),
                    (0, Some(1), false),
                    (1, Some(0), true),
                ] {
                    set_text_modes(&mut fixture, active, pin);
                    let before = serde_json::to_value(&fixture.doc).expect("before");
                    assert_eq!(
                        text_target_at(&fixture.doc, fixture.instance, DVec2::new(620.0, 380.0))
                            .is_some(),
                        visible,
                        "{property:?}, parent={bind_parent}, pin={pin:?}"
                    );
                    assert_eq!(
                        !text_clones(&fixture.doc, fixture.instance).is_empty(),
                        visible
                    );
                    assert_eq!(serde_json::to_value(&fixture.doc).expect("after"), before);
                }
            }
        }
    }

    #[test]
    fn resolved_instance_text_keeps_derived_geometry_and_resolved_clip_guards() {
        use fanta_doc::{DerivedOverride, TextAutoResize, TextStyle, VarValue, VariableType};
        let mut fixture = resolved_text_fixture(false);
        let typography = resolved_text_variable(
            &mut fixture,
            "Typography",
            VariableType::Typography,
            [
                VarValue::TextStyle {
                    value: TextStyle {
                        size_px: 12.0,
                        ..Default::default()
                    },
                },
                VarValue::TextStyle {
                    value: TextStyle {
                        size_px: 24.0,
                        ..Default::default()
                    },
                },
            ],
        );
        let text = fixture
            .doc
            .scene
            .get_mut(fixture.text)
            .expect("master text");
        text.bindings.insert(BoundProp::TextStyle, typography);
        let NodeData::Text(text) = &mut text.data else {
            panic!("text");
        };
        text.auto_resize = TextAutoResize::WidthAndHeight;
        let NodeData::Instance(instance) = &mut fixture
            .doc
            .scene
            .get_mut(fixture.instance)
            .expect("instance")
            .data
        else {
            panic!("instance");
        };
        instance.derived.push(DerivedOverride {
            path: fixture.path.clone(),
            transform: Some(Transform2D::translation(40.0, 12.0)),
            size: Some([180.0, 44.0]),
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        });
        set_text_modes(&mut fixture, 0, Some(1));
        let before = serde_json::to_value(&fixture.doc).expect("before");
        let point = DVec2::new(800.0, 390.0);
        let target =
            text_target_at(&fixture.doc, fixture.instance, point).expect("derived text bounds");
        assert_eq!(target.text.content, "Visible B");
        assert_eq!(target.text.style.size_px, 24.0);
        assert_eq!(target.text.local_size, [180.0, 44.0]);
        assert_eq!(
            target.world.transform_point(DVec2::ZERO),
            DVec2::new(640.0, 362.0)
        );
        assert!(text_target_at(&fixture.doc, fixture.instance, DVec2::new(830.0, 390.0)).is_none());
        assert_eq!(serde_json::to_value(&fixture.doc).expect("after"), before);
        fixture
            .doc
            .scene
            .get_mut(fixture.text)
            .expect("text")
            .flags
            .insert(NodeFlags::LOCKED);
        assert!(text_target_at(&fixture.doc, fixture.instance, point).is_none());
        fixture
            .doc
            .scene
            .get_mut(fixture.text)
            .expect("text")
            .flags
            .remove(NodeFlags::LOCKED);
        let width = resolved_text_variable(
            &mut fixture,
            "Clip width",
            VariableType::Float,
            [
                VarValue::Float { value: 240.0 },
                VarValue::Float { value: 100.0 },
            ],
        );
        fixture
            .doc
            .scene
            .get_mut(fixture.master)
            .expect("master")
            .bindings
            .insert(BoundProp::ClipWidth, width);
        assert!(
            text_target_at(&fixture.doc, fixture.instance, point).is_none(),
            "resolved ancestor clip hides the text"
        );
        set_text_modes(&mut fixture, 1, Some(0));
        assert!(
            text_target_at(&fixture.doc, fixture.instance, point).is_some(),
            "placed pin restores the wider clip"
        );
    }

    #[test]
    fn resolved_instance_text_unresolved_aliases_keep_the_authored_fallback() {
        use fanta_doc::VarValue;
        for cycle in [false, true] {
            let mut fixture = resolved_text_fixture(false);
            if cycle {
                fixture
                    .doc
                    .variables
                    .variables
                    .get_mut(&fixture.caption)
                    .expect("caption")
                    .values_by_mode = fixture
                    .modes
                    .into_iter()
                    .map(|mode| {
                        (
                            mode,
                            VarValue::Alias {
                                variable: fixture.alias,
                            },
                        )
                    })
                    .collect();
            } else {
                fixture.doc.variables.variables.remove(&fixture.caption);
            }
            let before = serde_json::to_value(&fixture.doc).expect("before");
            let target = text_target_at(&fixture.doc, fixture.instance, DVec2::new(620.0, 380.0))
                .expect("literal fallback text");
            assert_eq!(target.text.content, "Master");
            assert_eq!(serde_json::to_value(&fixture.doc).expect("after"), before);
        }
    }

    #[test]
    fn hit_test_finds_the_text_clone_with_its_def_path() {
        let (doc, instance_id, def_path) = doc_with_instance();
        // The text clone sits at world (500+10, 300+20) = (510, 320), box 120x30.
        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0))
            .expect("a text clone under the point");
        assert_eq!(target.instance_id, instance_id);
        assert_eq!(target.def_path, def_path);
        assert_eq!(target.text.content, "Master");
    }

    #[test]
    fn clone_world_transform_matches_instance_plus_child_offset() {
        let (doc, instance_id, _) = doc_with_instance();
        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).expect("target");
        // Local origin of the text clone maps to instance_pos + child_offset =
        // (500,300) + (10,20) = (510, 320) — the master root's own (1000,1000)
        // transform is suppressed.
        let origin = target.world.transform_point(DVec2::ZERO);
        assert!(
            (origin - DVec2::new(510.0, 320.0)).length() < 1e-6,
            "{origin:?}"
        );
    }

    #[test]
    fn a_point_outside_every_text_box_is_a_miss() {
        let (doc, instance_id, _) = doc_with_instance();
        assert!(text_target_at(&doc, instance_id, DVec2::new(490.0, 305.0)).is_none());
    }

    /// A hidden master text is skipped by the renderer, so it must not be an
    /// edit target either — the hit falls through to nothing instead of
    /// opening a session on invisible glyphs.
    #[test]
    fn hidden_text_clone_is_not_hit() {
        let (mut doc, instance_id, def_path) = doc_with_instance();
        let text_id = def_path[0];
        let node = doc.scene.get(text_id).expect("master text").clone();
        let mut flags = node.flags;
        flags.insert(fanta_doc::NodeFlags::HIDDEN);
        doc.apply(Operation::SetFlags {
            id: text_id,
            old: node.flags,
            new: flags,
        })
        .expect("hide the master text");
        assert!(
            text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).is_none(),
            "a hidden clone must not be an edit target"
        );
    }

    #[test]
    fn locked_text_or_ancestor_cannot_open_an_instance_override() {
        for lock_parent in [false, true] {
            let (mut doc, instance, path) = doc_with_nested_instance();
            let target = if lock_parent {
                path.first()
            } else {
                path.last()
            }
            .copied()
            .expect("master path");
            doc.scene
                .get_mut(target)
                .expect("master node")
                .flags
                .insert(NodeFlags::LOCKED);
            assert!(text_target_at(&doc, instance, DVec2::new(550.0, 370.0)).is_none());
            assert_eq!(
                text_clones(&doc, instance).len(),
                1,
                "locked text remains visible in properties"
            );
        }
    }

    #[test]
    fn virtual_text_hit_respects_occlusion_and_visible_cover_geometry() {
        for cover_kind in ["filled", "hidden", "transparent", "outline", "outside"] {
            let (mut doc, instance, path) = doc_with_instance();
            let text = doc
                .scene
                .get(*path.first().expect("text path"))
                .expect("master text");
            let mut vector = fanta_doc::VectorNode::rect_solid(0.0, 0.0, 120.0, 30.0, Color::WHITE);
            if cover_kind == "outline" {
                vector.fills.clear();
            }
            let mut cover = CanvasNode::new(NodeData::Vector(vector));
            cover.parent = text.parent;
            cover.transform = if cover_kind == "outside" {
                Transform2D::translation(200.0, 200.0)
            } else {
                text.transform
            };
            cover.index = fanta_doc::IndexKey::after(text.index);
            if cover_kind == "hidden" {
                cover.flags.insert(NodeFlags::HIDDEN);
            }
            if cover_kind == "transparent" {
                cover.opacity = fanta_doc::UnitInterval::new(0.0);
            }
            doc.apply(Operation::create_node(cover)).expect("cover");
            let before = serde_json::to_value(&doc).expect("original document");
            let hit = text_target_at(&doc, instance, DVec2::new(520.0, 330.0));
            assert_eq!(hit.is_some(), cover_kind != "filled", "{cover_kind}");
            assert_eq!(
                serde_json::to_value(&doc).expect("document after hit"),
                before
            );
        }
    }

    #[test]
    fn clipped_virtual_text_is_not_an_edit_target() {
        let (mut doc, instance, path) = doc_with_nested_instance();
        let inner = *path.first().expect("inner group");
        let NodeData::Group(group) = &mut doc.scene.get_mut(inner).expect("inner group").data
        else {
            panic!("group")
        };
        group.clip_size = Some([5.0, 5.0]);
        assert!(text_target_at(&doc, instance, DVec2::new(550.0, 370.0)).is_none());
    }

    #[test]
    fn rounded_virtual_clip_rejects_text_outside_its_visible_corner() {
        let (mut doc, instance, path) = doc_with_nested_instance();
        let inner = *path.first().expect("inner group");
        let text = *path.last().expect("text");
        doc.scene.get_mut(text).expect("text").transform = Transform2D::IDENTITY;
        let NodeData::Group(group) = &mut doc.scene.get_mut(inner).expect("inner group").data
        else {
            panic!("group")
        };
        group.clip_size = Some([100.0, 100.0]);
        group.corner_radius = Some(40.0);
        assert!(text_target_at(&doc, instance, DVec2::new(532.0, 342.0)).is_none());
        doc.scene.get_mut(inner).expect("inner group").meta =
            serde_json::json!({"clip_content": false});
        assert!(text_target_at(&doc, instance, DVec2::new(532.0, 342.0)).is_some());
    }

    #[test]
    fn content_op_adds_then_replaces_the_same_slot() {
        let (mut doc, instance_id, def_path) = doc_with_instance();

        // First edit appends (index 0, old None).
        let ops = commit_ops(&doc, instance_id, &def_path, Some("Edited"), None);
        assert_eq!(ops.len(), 1);
        let Operation::SetInstanceOverride {
            index, old, new, ..
        } = &ops[0]
        else {
            panic!("expected SetInstanceOverride");
        };
        assert_eq!(*index, 0);
        assert!(old.is_none());
        assert!(matches!(
            new.as_ref().map(|o| &o.value),
            Some(OverrideValue::Text { value }) if value == "Edited"
        ));
        doc.apply(ops[0].clone()).expect("apply");

        // Second edit replaces the same slot (index 0, old Some).
        let ops = commit_ops(&doc, instance_id, &def_path, Some("Again"), None);
        let Operation::SetInstanceOverride { index, old, .. } = &ops[0] else {
            panic!("expected SetInstanceOverride");
        };
        assert_eq!(*index, 0);
        assert!(old.is_some());
        doc.apply(ops[0].clone()).expect("apply");

        // Exactly one override, and the expansion now resolves to the new text.
        let NodeData::Instance(instance) = &doc.scene.get(instance_id).unwrap().data else {
            panic!("instance");
        };
        assert_eq!(instance.overrides.len(), 1);
        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).expect("target");
        assert_eq!(target.text.content, "Again");
    }

    #[test]
    fn color_and_content_are_independent_slots() {
        let (mut doc, instance_id, def_path) = doc_with_instance();
        // A single commit with both channels changed appends TWO distinct
        // overrides at non-colliding indices (0 then 1).
        let ops = commit_ops(
            &doc,
            instance_id,
            &def_path,
            Some("Hi"),
            Some(Color::rgb(255, 0, 0)),
        );
        assert_eq!(ops.len(), 2);
        for op in ops {
            doc.apply(op).expect("apply");
        }

        let NodeData::Instance(instance) = &doc.scene.get(instance_id).unwrap().data else {
            panic!("instance");
        };
        assert_eq!(
            instance.overrides.len(),
            2,
            "content + color are distinct overrides"
        );

        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).expect("target");
        assert_eq!(target.text.content, "Hi");
        assert_eq!(target.text.style.color, Color::rgb(255, 0, 0));
    }

    /// A master whose text sits under a NESTED group inside the root: root →
    /// inner group (translated 30,40) → text (translated 10,20). Returns the
    /// doc, the instance id, and the text clone's def-local path
    /// `[inner_group_id, text_id]` (root excluded, root-first).
    fn doc_with_nested_instance() -> (Doc, NodeId, OverridePath) {
        let mut doc = Doc::new();

        let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let root_id = root.id;
        root.transform = Transform2D::translation(1000.0, 1000.0);
        doc.apply(Operation::create_node(root)).expect("root");

        let mut inner = CanvasNode::new(NodeData::Group(GroupNode::default()));
        let inner_id = inner.id;
        inner.parent = Some(root_id);
        inner.transform = Transform2D::translation(30.0, 40.0);
        doc.apply(Operation::create_node(inner))
            .expect("inner group");

        let mut text = CanvasNode::new(NodeData::Text(TextNode::new("Nested", 120.0, 30.0)));
        let text_id = text.id;
        text.parent = Some(inner_id);
        text.transform = Transform2D::translation(10.0, 20.0);
        doc.apply(Operation::create_node(text)).expect("text");

        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root_id, "Card"));

        let mut instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: BTreeMap::new(),
            derived: Vec::new(),
            local_size: [160.0, 90.0],
        }));
        let instance_id = instance.id;
        instance.transform = Transform2D::translation(500.0, 300.0);
        doc.apply(Operation::create_node(instance))
            .expect("instance");
        doc.add_page(instance_id);

        let def_path: OverridePath = smallvec![inner_id, text_id];
        (doc, instance_id, def_path)
    }

    #[test]
    fn nested_group_text_is_hit_through_both_local_transforms() {
        let (doc, instance_id, def_path) = doc_with_nested_instance();
        // Text clone world origin = instance (500,300) + inner group (30,40) +
        // text (10,20) = (540, 360); the master root's own (1000,1000) is
        // suppressed. Box is 120x30.
        let target = text_target_at(&doc, instance_id, DVec2::new(550.0, 370.0))
            .expect("the nested text clone under the point");
        assert_eq!(target.instance_id, instance_id);
        assert_eq!(target.def_path, def_path, "def path is [inner_group, text]");
        assert_eq!(target.def_path.len(), 2);
        assert_eq!(target.text.content, "Nested");

        let origin = target.world.transform_point(DVec2::ZERO);
        assert!(
            (origin - DVec2::new(540.0, 360.0)).length() < 1e-6,
            "world composes W_inst ∘ T_inner ∘ T_text, got {origin:?}"
        );

        // A point inside the instance but before the nested offsets miss.
        assert!(text_target_at(&doc, instance_id, DVec2::new(510.0, 320.0)).is_none());
    }

    #[test]
    fn nested_path_override_edits_only_the_nested_clone() {
        let (mut doc, instance_id, def_path) = doc_with_nested_instance();
        let ops = commit_ops(&doc, instance_id, &def_path, Some("Deep edit"), None);
        assert_eq!(ops.len(), 1);
        for op in ops {
            doc.apply(op).expect("apply");
        }
        let target = text_target_at(&doc, instance_id, DVec2::new(550.0, 370.0)).expect("target");
        assert_eq!(target.text.content, "Deep edit");
        assert_eq!(target.def_path, def_path);
    }

    /// Committing content + color for one clone while the instance already
    /// carries an unrelated override (another descendant's) must append at the
    /// next free slots — indices 1 and 2 — and leave slot 0 untouched.
    #[test]
    fn commit_appends_after_a_pre_existing_unrelated_override() {
        let (mut doc, instance_id, def_path) = doc_with_nested_instance();

        // Pre-existing override on a DIFFERENT path (the inner group alone).
        let unrelated_path: OverridePath = smallvec![def_path[0]];
        let unrelated = commit_ops(&doc, instance_id, &unrelated_path, Some("Unrelated"), None);
        assert_eq!(unrelated.len(), 1);
        let Operation::SetInstanceOverride { index, .. } = &unrelated[0] else {
            panic!("expected SetInstanceOverride");
        };
        assert_eq!(*index, 0);
        for op in unrelated {
            doc.apply(op).expect("apply unrelated");
        }

        let ops = commit_ops(
            &doc,
            instance_id,
            &def_path,
            Some("Hi"),
            Some(Color::rgb(0, 0, 255)),
        );
        assert_eq!(ops.len(), 2);
        let indices: Vec<usize> = ops
            .iter()
            .map(|op| match op {
                Operation::SetInstanceOverride { index, old, .. } => {
                    assert!(old.is_none(), "both land in fresh slots");
                    *index
                }
                _ => panic!("expected SetInstanceOverride"),
            })
            .collect();
        assert_eq!(indices, vec![1, 2], "appends skip the occupied slot 0");
        for op in ops {
            doc.apply(op).expect("apply");
        }

        let NodeData::Instance(instance) = &doc.scene.get(instance_id).expect("instance").data
        else {
            panic!("instance");
        };
        assert_eq!(instance.overrides.len(), 3);
        assert_eq!(
            instance.overrides[0].target_path, unrelated_path,
            "the pre-existing override keeps its slot"
        );

        let target = text_target_at(&doc, instance_id, DVec2::new(550.0, 370.0)).expect("target");
        assert_eq!(target.text.content, "Hi");
        assert_eq!(target.text.style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn preview_then_rewind_round_trips_the_override_vec() {
        let (mut doc, instance_id, def_path) = doc_with_instance();
        let base = snapshot_overrides(&doc, instance_id);
        assert!(base.is_empty());

        // Preview writes a transient content + color override.
        let previewed = preview_overrides(&base, &def_path, "Draft", Some(Color::rgb(0, 128, 0)));
        set_overrides_transient(&mut doc, instance_id, previewed);
        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).expect("target");
        assert_eq!(target.text.content, "Draft");
        assert_eq!(target.text.style.color, Color::rgb(0, 128, 0));

        // Rewind restores the empty pre-edit vec.
        set_overrides_transient(&mut doc, instance_id, base);
        let target = text_target_at(&doc, instance_id, DVec2::new(520.0, 330.0)).expect("target");
        assert_eq!(target.text.content, "Master");
    }
}

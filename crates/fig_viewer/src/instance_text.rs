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

use std::collections::HashMap;

use fanta_doc::{
    BoundProp, Color, Doc, ExpandedNode, Fill, NodeData, NodeFlags, NodeId, Operation, Override,
    OverridePath, OverrideValue, TextNode, Transform2D, expand_instance,
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
    let mut expanded = expand_instance(&doc.scene, &doc.components, instance);
    if expanded.is_empty() {
        return None;
    }
    if instance.derived.is_empty() {
        fanta_doc::solve_expanded(&mut expanded, &mut fanta_render::measure_text_node);
    }
    Some((expanded, world))
}

/// The absolute world transform of the clone at `expanded[idx]`: fold the clone
/// transforms from the expansion root's direct child down to the target onto
/// `w_inst`, EXCLUDING the root clone's own transform (the renderer suppresses
/// it — the instance node's transform, already in `w_inst`, positions the
/// instance). `by_id` maps a clone id to its index in `expanded`.
fn clone_world_transform(
    expanded: &[ExpandedNode],
    by_id: &HashMap<NodeId, usize>,
    idx: usize,
    w_inst: Transform2D,
) -> Transform2D {
    // Walk clone parent links target → up, collecting each node's transform but
    // stopping BEFORE the root (a clone whose `parent` is `None`).
    let mut chain: Vec<Transform2D> = Vec::new();
    let mut cursor = idx;
    loop {
        let node = &expanded[cursor].node;
        let Some(parent) = node.parent else {
            break; // reached the expansion root — its transform is suppressed.
        };
        chain.push(node.transform);
        let Some(&parent_idx) = by_id.get(&parent) else {
            break;
        };
        cursor = parent_idx;
    }
    // `chain` is target → root-child; fold root-child → target so each inner
    // transform composes under the outer ones (`acc = Tᵢ.then(&acc)`).
    let mut world = w_inst;
    for transform in chain.iter().rev() {
        world = transform.then(&world);
    }
    world
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

/// The topmost (last-painted) TEXT clone inside `instance_id` whose local box
/// contains `world_point`, resolved for override editing. Paint order is the
/// expansion's DFS pre-order, so the last containing text clone is on top.
pub(crate) fn text_target_at(
    doc: &Doc,
    instance_id: NodeId,
    world_point: DVec2,
) -> Option<InstanceTextTarget> {
    let (expanded, w_inst) = expand_resolved(doc, instance_id)?;
    let by_id: HashMap<NodeId, usize> = expanded
        .iter()
        .enumerate()
        .map(|(i, e)| (e.node.id, i))
        .collect();

    let mut hit: Option<(usize, Transform2D)> = None;
    for (idx, entry) in expanded.iter().enumerate() {
        let NodeData::Text(text) = &entry.node.data else {
            continue;
        };
        if !clone_is_painted(&expanded, &by_id, idx) {
            continue;
        }
        let world = clone_world_transform(&expanded, &by_id, idx, w_inst);
        if !world.is_finite() {
            continue;
        }
        let local = world.inverse().transform_point(world_point);
        let [w, h] = text.local_size;
        if local.x >= 0.0 && local.x <= w && local.y >= 0.0 && local.y <= h {
            hit = Some((idx, world)); // keep the last (topmost) match
        }
    }

    let (idx, world) = hit?;
    let entry = &expanded[idx];
    let NodeData::Text(text) = &entry.node.data else {
        return None;
    };
    Some(InstanceTextTarget {
        instance_id,
        def_path: entry.def_path.clone(),
        text: text.clone(),
        world,
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

// ---------------------------------------------------------------------------
// Override construction: preview (transient) and commit (undoable op)
// ---------------------------------------------------------------------------

/// The index of the override at `path`/`prop` in `overrides`, if present.
fn override_index(overrides: &[Override], path: &OverridePath, prop: &BoundProp) -> Option<usize> {
    overrides
        .iter()
        .position(|ov| ov.target_path == *path && ov.target_prop == *prop)
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
    match override_index(overrides, &new_override.target_path, &new_override.target_prop) {
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
        push_upsert(&mut ops, &mut working, text_content_override(def_path, content));
    }
    if let Some(color) = color {
        push_upsert(&mut ops, &mut working, text_color_override(def_path, color));
    }
    ops
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
        doc.apply(Operation::create_node(instance)).expect("instance");
        doc.add_page(instance_id);

        let def_path: OverridePath = smallvec![text_id];
        (doc, instance_id, def_path)
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
        assert!((origin - DVec2::new(510.0, 320.0)).length() < 1e-6, "{origin:?}");
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
    fn content_op_adds_then_replaces_the_same_slot() {
        let (mut doc, instance_id, def_path) = doc_with_instance();

        // First edit appends (index 0, old None).
        let ops = commit_ops(&doc, instance_id, &def_path, Some("Edited"), None);
        assert_eq!(ops.len(), 1);
        let Operation::SetInstanceOverride { index, old, new, .. } = &ops[0] else {
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
        assert_eq!(instance.overrides.len(), 2, "content + color are distinct overrides");

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
        doc.apply(Operation::create_node(inner)).expect("inner group");

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
        doc.apply(Operation::create_node(instance)).expect("instance");
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

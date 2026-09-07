//! Generic [`OverrideValue::Field`] application: the serde-merge escape hatch
//! (F1 / OV-9 in docs/research/figma-parity-divergence.md). A Field payload is
//! a partial CanvasNode JSON object shallow-merged onto the clone; structural
//! keys are rejected and a malformed payload leaves the clone untouched.

use super::*;
use crate::style::{Shadow, ShadowKind};
use serde_json::json;

/// A master with a vector (rectangle) child — the corner-radius / effects
/// target. Returns (library, component id, root id, vector child id).
fn master_with_rect(scene: &mut Scene) -> (ComponentLibrary, ComponentId, NodeId, NodeId) {
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 40.0]),
        ..Default::default()
    }));
    root.name = "Card".into();
    let root_id = root.id;
    scene.insert(root).unwrap();

    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        100.0,
        40.0,
        Color::rgb(200, 200, 200),
    )));
    rect.parent = Some(root_id);
    let rect_id = rect.id;
    scene.insert(rect).unwrap();

    let comp_id = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp_id, ComponentDef::new(comp_id, root_id, "Card"));
    (lib, comp_id, root_id, rect_id)
}

/// An instance of `component` carrying exactly the given overrides.
fn instance_with(component: ComponentId, overrides: Vec<Override>) -> InstanceNode {
    InstanceNode {
        component,
        overrides,
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    }
}

fn field_override(target: NodeId, value: serde_json::Value) -> Override {
    Override {
        target_path: smallvec![target],
        // `target_prop` is unused for a Field value (same convention as swaps).
        target_prop: BoundProp::Visible,
        value: OverrideValue::Field { value },
    }
}

#[test]
fn field_override_applies_opacity() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    let inst = instance_with(
        comp_id,
        vec![field_override(label_id, json!({ "opacity": 0.35 }))],
    );
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    assert!(
        (child.node.opacity.get() - 0.35).abs() < 1e-4,
        "the Field opacity key lands on the wrapper, got {}",
        child.node.opacity.get()
    );
}

#[test]
fn field_override_applies_corner_radius() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, rect_id) = master_with_rect(&mut scene);
    let inst = instance_with(
        comp_id,
        vec![field_override(rect_id, json!({ "corner_radius": 6.0 }))],
    );
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [rect_id])
        .unwrap();
    match &child.node.data {
        // `corner_radius` is a flattened VectorNode key, proving the merge
        // reaches variant payload fields, not only wrapper fields.
        NodeData::Vector(v) => assert_eq!(v.corner_radius, Some(6.0)),
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn field_override_replaces_effects_list() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, rect_id) = master_with_rect(&mut scene);
    // The master child already carries a shadow; the Field list must REPLACE
    // it (Figma override semantics for the effects channel), not append.
    if let Some(node) = scene.get_mut(rect_id) {
        node.effects.push(Shadow {
            kind: ShadowKind::Drop,
            color: Color::BLACK,
            blur: 2.0,
            spread: 0.0,
            offset: [0.0, 1.0],
            show_behind_node: false,
        });
    }
    let override_shadow = Shadow {
        kind: ShadowKind::Inner,
        color: Color::rgba(0, 0, 0, 128),
        blur: 8.0,
        spread: 1.0,
        offset: [0.0, 4.0],
        show_behind_node: false,
    };
    let inst = instance_with(
        comp_id,
        vec![field_override(
            rect_id,
            json!({ "effects": serde_json::to_value(vec![override_shadow.clone()]).unwrap() }),
        )],
    );
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [rect_id])
        .unwrap();
    assert_eq!(
        child.node.effects.as_slice(),
        std::slice::from_ref(&override_shadow),
        "the Field effects list replaces the master's, wholesale"
    );
}

#[test]
fn field_override_ignores_structural_keys() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    // A hostile/stale payload smuggling identity, hierarchy, and a variant
    // retag alongside a legitimate opacity write: the structural keys must be
    // skipped while the opacity still applies.
    let inst = instance_with(
        comp_id,
        vec![field_override(
            label_id,
            json!({
                "id": "bogus",
                "parent": "bogus",
                "index": "!!",
                "type": "group",
                "opacity": 0.25,
            }),
        )],
    );
    let expanded = expand_instance(&scene, &lib, &inst);
    let root = expanded.iter().find(|e| e.def_path.is_empty()).unwrap();
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    assert!(
        matches!(child.node.data, NodeData::Text(_)),
        "`type` is never overridable — the clone stays a text node"
    );
    assert_eq!(
        child.node.parent,
        Some(root.node.id),
        "`parent` is never overridable — the clone stays wired to its root"
    );
    assert!(
        (child.node.opacity.get() - 0.25).abs() < 1e-4,
        "the legitimate key still applies alongside the rejected ones"
    );
}

#[test]
fn field_override_malformed_value_leaves_node_intact() {
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    // A wrong value shape for a known key must fail the round-trip and leave
    // the clone at its master state (never half-applied, never a panic); a
    // non-object payload is likewise a tolerated no-op.
    for bad in [json!({ "opacity": "not-a-number" }), json!("nonsense")] {
        let inst = instance_with(comp_id, vec![field_override(label_id, bad)]);
        let expanded = expand_instance(&scene, &lib, &inst);
        let child = expanded
            .iter()
            .find(|e| e.def_path.as_slice() == [label_id])
            .unwrap();
        match &child.node.data {
            NodeData::Text(t) => assert_eq!(t.content, "Label", "master content untouched"),
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(
            child.node.opacity.get(),
            1.0,
            "no partial write from a malformed payload"
        );
    }
}

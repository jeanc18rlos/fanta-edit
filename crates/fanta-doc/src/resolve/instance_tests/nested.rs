//! Nested-instance override routing across one component boundary: length-2
//! paths applied on recursion, swapping the nested component via an overridden
//! symbol id, icon-swap recolor falling back to the sole vector, and an
//! inheriting instance picking up the master's background.

use super::*;

// ---- nested-instance override routing -----------------------------------

/// Build two masters in `scene`:
/// - INNER "Chip": a frame root with a text child ("Inner").
/// - OUTER "Card": a frame root whose child is an INSTANCE of Chip.
///
/// Returns (lib, outer_comp, inner_comp, outer_root, nested_inst_id,
/// inner_label_id). `nested_inst_id` is the outer-master's def-local id of the
/// nested instance (so an outer override path of `[nested_inst_id,
/// inner_label_id]` crosses one boundary). `inner_label_id` is the inner
/// master's def-local id of the text child.
fn nested_masters(
    scene: &mut Scene,
) -> (
    ComponentLibrary,
    ComponentId,
    ComponentId,
    NodeId,
    NodeId,
    NodeId,
) {
    let mut lib = ComponentLibrary::new();

    // Inner master.
    let inner_comp = ComponentId::new();
    let mut inner_root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([60.0, 20.0]),
        background: Some(crate::style::Fill::solid(Color::WHITE)),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    inner_root.name = "Chip".into();
    let inner_root_id = inner_root.id;
    scene.insert(inner_root).unwrap();
    let mut inner_label = CanvasNode::new(NodeData::Text(TextNode::new("Inner", 50.0, 16.0)));
    inner_label.parent = Some(inner_root_id);
    let inner_label_id = inner_label.id;
    scene.insert(inner_label).unwrap();
    lib.defs.insert(
        inner_comp,
        ComponentDef::new(inner_comp, inner_root_id, "Chip"),
    );

    // Outer master: a frame holding an instance of the inner.
    let outer_comp = ComponentId::new();
    let mut outer_root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([120.0, 40.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    outer_root.name = "Card".into();
    let outer_root_id = outer_root.id;
    scene.insert(outer_root).unwrap();
    let mut nested = CanvasNode::new(NodeData::Instance(InstanceNode {
        component: inner_comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [60.0, 20.0],
    }));
    nested.parent = Some(outer_root_id);
    let nested_inst_id = nested.id;
    scene.insert(nested).unwrap();
    lib.defs.insert(
        outer_comp,
        ComponentDef::new(outer_comp, outer_root_id, "Card"),
    );

    (
        lib,
        outer_comp,
        inner_comp,
        outer_root_id,
        nested_inst_id,
        inner_label_id,
    )
}

#[test]
fn length2_override_routes_onto_nested_instance_and_applies_on_recursion() {
    // A length-2 guidPath override `[nested_inst, inner_label]` crosses the
    // nested-instance boundary. The OUTER expansion must NOT apply it
    // directly (no clone matches the full path); it must route the remainder
    // `[inner_label]` onto the cloned nested instance. Recursing on that
    // nested instance then applies the text override to the inner label.
    let mut scene = Scene::new();
    let (lib, outer_comp, _inner_comp, _outer_root, nested_inst_id, inner_label_id) =
        nested_masters(&mut scene);
    let inst = InstanceNode {
        component: outer_comp,
        overrides: vec![Override {
            target_path: smallvec![nested_inst_id, inner_label_id],
            target_prop: crate::binding::BoundProp::TextContent,
            value: OverrideValue::Text {
                value: "Routed".into(),
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [120.0, 40.0],
    };

    // Outer expansion: the nested instance clone must now carry the remainder.
    let expanded = expand_instance(&scene, &lib, &inst);
    let nested_clone = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [nested_inst_id])
        .expect("nested instance clone present");
    let nested_inst = match &nested_clone.node.data {
        NodeData::Instance(i) => i,
        other => panic!("expected nested instance, got {other:?}"),
    };
    assert_eq!(
        nested_inst.overrides.len(),
        1,
        "remainder routed onto nested"
    );
    assert_eq!(
        nested_inst.overrides[0].target_path.as_slice(),
        [inner_label_id],
        "remainder is the path minus the nested-instance prefix"
    );

    // Recursion: expanding the nested instance applies the routed override.
    let inner_expanded = expand_instance(&scene, &lib, nested_inst);
    let inner_label = inner_expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [inner_label_id])
        .expect("inner label clone present");
    match &inner_label.node.data {
        NodeData::Text(t) => assert_eq!(t.content, "Routed"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn overridden_symbol_id_swaps_the_nested_component() {
    // A SwapInstance override at the nested instance's path must re-point the
    // cloned nested instance to the swapped component, so the right master
    // expands on recursion.
    let mut scene = Scene::new();
    let (mut lib, outer_comp, _inner_comp, _outer_root, nested_inst_id, _inner_label_id) =
        nested_masters(&mut scene);

    // A second inner master ("Alt") to swap to.
    let alt_comp = ComponentId::new();
    let mut alt_root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([60.0, 20.0]),
        background: Some(crate::style::Fill::solid(Color::BLACK)),
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    alt_root.name = "Alt".into();
    let alt_root_id = alt_root.id;
    scene.insert(alt_root).unwrap();
    lib.defs
        .insert(alt_comp, ComponentDef::new(alt_comp, alt_root_id, "Alt"));

    let inst = InstanceNode {
        component: outer_comp,
        overrides: vec![Override {
            target_path: smallvec![nested_inst_id],
            target_prop: crate::binding::BoundProp::Visible,
            value: OverrideValue::SwapInstance {
                component: alt_comp,
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [120.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let nested_clone = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [nested_inst_id])
        .expect("nested instance clone present");
    match &nested_clone.node.data {
        NodeData::Instance(i) => assert_eq!(
            i.component, alt_comp,
            "overriddenSymbolID swap re-points the nested instance"
        ),
        other => panic!("expected instance, got {other:?}"),
    }
}

#[test]
fn icon_swap_recolor_falls_back_to_the_sole_vector() {
    // The icon-swap recolor case: a single-vector icon master is recolored by
    // a single-level `Fills` override whose path is the icon's *original*
    // vector id. After an icon swap the master's vector has a DIFFERENT id, so
    // the exact path no longer matches — the recolor must fall back to the
    // (one) vector leaf so the swapped icon still renders in the themed color
    // instead of its black master default (the Copy/Delete invisible-icon bug).
    let mut scene = Scene::new();
    let comp = ComponentId::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    root.name = "Icon".into();
    let root_id = root.id;
    scene.insert(root).unwrap();
    let mut vec_node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        16.0,
        16.0,
        Color::BLACK, // the icon master's black default
    )));
    vec_node.parent = Some(root_id);
    let vec_id = vec_node.id;
    scene.insert(vec_node).unwrap();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp, ComponentDef::new(comp, root_id, "Icon"));

    // A recolor override targeting a STALE vector id (a different NodeId than
    // this master's vector) — exactly what a swapped icon carries.
    let stale = NodeId::new();
    assert_ne!(stale, vec_id);
    let inst = InstanceNode {
        component: comp,
        overrides: vec![Override {
            target_path: smallvec![stale],
            target_prop: crate::binding::BoundProp::FillColor { index: 0 },
            value: OverrideValue::Fills {
                fills: smallvec![crate::style::Fill::solid(Color::WHITE)],
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [16.0, 16.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let v = expanded
        .iter()
        .find(|e| matches!(e.node.data, NodeData::Vector(_)))
        .expect("vector clone present");
    match &v.node.data {
        NodeData::Vector(vn) => match vn.fills.first() {
            Some(crate::style::Fill::Solid { color, .. }) => assert_eq!(
                *color,
                Color::WHITE,
                "stale-path recolor falls back to the sole vector"
            ),
            other => panic!("expected solid white fill, got {other:?}"),
        },
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn clipped_root_without_background_uses_instance_box() {
    let mut scene = Scene::new();
    let comp = ComponentId::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([18.0, 18.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    root.name = "Brackets".into();
    let root_id = root.id;
    scene.insert(root).unwrap();

    let mut vector = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        26.0,
        28.0,
        Color::BLACK,
    )));
    vector.parent = Some(root_id);
    scene.insert(vector).unwrap();

    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp, ComponentDef::new(comp, root_id, "Brackets"));
    let inst = InstanceNode {
        component: comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [32.0, 32.0],
    };

    let expanded = expand_instance(&scene, &lib, &inst);
    let root = expanded
        .iter()
        .find(|entry| entry.def_path.is_empty())
        .expect("root clone present");
    match &root.node.data {
        NodeData::Group(group) => assert_eq!(
            group.clip_size,
            Some([32.0, 32.0]),
            "clipped component roots without a background still use the placed instance box"
        ),
        other => panic!("expected group, got {other:?}"),
    }
}

#[test]
fn non_clipping_sized_root_uses_instance_box_without_becoming_a_frame() {
    let mut scene = Scene::new();
    let component = ComponentId::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode::default()));
    root.name = "Loose component".into();
    let root_id = root.id;
    scene.insert(root).unwrap();
    let mut library = ComponentLibrary::new();
    library.defs.insert(
        component,
        ComponentDef::new(component, root_id, "Loose component"),
    );
    let instance = InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [32.0, 24.0],
    };

    let expanded = expand_instance(&scene, &library, &instance);
    let root = expanded
        .iter()
        .find(|entry| entry.def_path.is_empty())
        .expect("root clone present");
    let NodeData::Group(group) = &root.node.data else {
        panic!("expected group");
    };
    assert_eq!(group.local_size, Some([32.0, 24.0]));
    assert_eq!(group.clip_size, None);
}

#[test]
fn inheriting_instance_gets_the_masters_background() {
    // An instance that does NOT override its surface inherits the master
    // root's background, painted at the instance's own box. The expansion
    // root must carry the master background AND clip to the instance size.
    let mut scene = Scene::new();
    let (lib, _outer, inner_comp, _outer_root, _nested, _label) = nested_masters(&mut scene);
    // The inner "Chip" master root has a WHITE background; an instance with
    // no overrides should inherit it.
    let inst = InstanceNode {
        component: inner_comp,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [80.0, 24.0], // resized vs the master's 60×20
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let root = expanded.iter().find(|e| e.def_path.is_empty()).unwrap();
    match &root.node.data {
        NodeData::Group(g) => {
            assert_eq!(
                g.background,
                Some(crate::style::Fill::solid(Color::WHITE)),
                "inherited master background must be present on the expansion root"
            );
            assert_eq!(
                g.clip_size,
                Some([80.0, 24.0]),
                "background paints at the instance's own box (merged surface)"
            );
        }
        other => panic!("expected group, got {other:?}"),
    }
}

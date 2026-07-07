//! Baked `derivedSymbolData` + sparse-override application onto clones:
//! transform/size, vector geometry/fills/stroke, group-frame background,
//! unmatched-field overwrite, text color/weight, glyph-fill color, and the
//! dangling-component empty-expansion case.

use super::*;

/// A master with a frame root + one vector child (a 10×10 white rect),
/// returns (lib, comp id, root id, vector child id). For derived-geometry /
/// derived-fill tests.
fn master_with_vector(scene: &mut Scene) -> (ComponentLibrary, ComponentId, NodeId, NodeId) {
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    root.name = "Header".into();
    let root_id = root.id;
    scene.insert(root).unwrap();

    let mut rect = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        10.0,
        10.0,
        Color::WHITE,
    )));
    rect.parent = Some(root_id);
    let rect_id = rect.id;
    scene.insert(rect).unwrap();

    let comp_id = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp_id, ComponentDef::new(comp_id, root_id, "Header"));
    (lib, comp_id, root_id, rect_id)
}

#[test]
fn expand_applies_derived_transform_and_size_to_descendant() {
    use crate::node::DerivedOverride;
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    // Figma baked: move the label to (12, 34) and resize its box to 64×18.
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![label_id],
            transform: Some(crate::transform::Transform2D::translation(12.0, 34.0)),
            size: Some([64.0, 18.0]),
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    // Transform overwritten with the baked translation.
    assert_eq!(
        child.node.transform,
        crate::transform::Transform2D::translation(12.0, 34.0)
    );
    // Size lands on the text node's local_size.
    match &child.node.data {
        NodeData::Text(t) => assert_eq!(t.local_size, [64.0, 18.0]),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn expand_applies_derived_geometry_fills_and_stroke_to_vector() {
    use crate::node::DerivedOverride;
    use crate::path::PathData;
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, rect_id) = master_with_vector(&mut scene);
    // Figma baked: replace the 10×10 white rect with a resolved black 40×8
    // path (a dark header bar) and a 2px stroke.
    let mut baked = PathData::new();
    baked
        .move_to(0.0, 0.0)
        .line_to(40.0, 0.0)
        .line_to(40.0, 8.0)
        .close();
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![rect_id],
            transform: None,
            size: None,
            fills: Some(smallvec![crate::style::Fill::solid(Color::BLACK)]),
            path_data: Some(baked.clone()),
            stroke_path: None,
            stroke_weight: Some(2.0),
            text: None,
        }],
        local_size: [100.0, 100.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [rect_id])
        .unwrap();
    match &child.node.data {
        NodeData::Vector(v) => {
            // Path replaced by the baked resolved geometry (not the master rect).
            assert_eq!(v.path.segments, baked.segments);
            // Fill resolved to black (the master was white).
            assert_eq!(
                v.fills.as_slice(),
                [crate::style::Fill::solid(Color::BLACK)].as_slice()
            );
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn derived_fill_geometry_without_stroke_clears_master_stroke() {
    use crate::node::DerivedOverride;
    use crate::path::PathData;

    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, rect_id) = master_with_vector(&mut scene);
    if let Some(NodeData::Vector(vector)) = scene.get_mut(rect_id).map(|node| &mut node.data) {
        vector.strokes = smallvec![crate::style::Stroke::solid(Color::BLACK, 1.0)];
    }

    let baked = PathData::rect(0.0, 0.0, 40.0, 4.0);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![rect_id],
            transform: None,
            size: None,
            fills: Some(smallvec![crate::style::Fill::solid(Color::WHITE)]),
            path_data: Some(baked),
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 100.0],
    };

    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|entry| entry.def_path.as_slice() == [rect_id])
        .unwrap();
    match &child.node.data {
        NodeData::Vector(vector) => {
            assert!(
                vector.strokes.is_empty(),
                "a derived filled path with no derived stroke must not keep the master outline"
            );
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn expand_keeps_stroked_vector_path_when_derived_stroke_path_is_present() {
    use crate::node::DerivedOverride;
    use crate::path::PathData;

    let mut scene = Scene::new();
    let mut root = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([100.0, 100.0]),
        background: None,
        explicit_modes: Default::default(),
        ..Default::default()
    }));
    root.name = "Wireframe".into();
    let root_id = root.id;
    scene.insert(root).unwrap();

    let mut master_path = PathData::new();
    master_path.move_to(0.0, 0.0).line_to(100.0, 0.0);
    let mut line = CanvasNode::new(NodeData::Vector(VectorNode {
        path: master_path.clone(),
        fills: Default::default(),
        strokes: smallvec![crate::style::Stroke::solid(Color::BLACK, 1.0)],
        corner_radius: None,
        corner_radii: None,
    }));
    line.parent = Some(root_id);
    let line_id = line.id;
    scene.insert(line).unwrap();

    let comp_id = ComponentId::new();
    let mut lib = ComponentLibrary::new();
    lib.defs
        .insert(comp_id, ComponentDef::new(comp_id, root_id, "Wireframe"));

    let mut baked = PathData::new();
    baked.move_to(0.0, 100.0).line_to(100.0, 100.0);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![line_id],
            transform: None,
            size: None,
            fills: None,
            path_data: None,
            stroke_path: Some(baked.clone()),
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 100.0],
    };

    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|entry| entry.def_path.as_slice() == [line_id])
        .unwrap();
    match &child.node.data {
        NodeData::Vector(vector) => {
            assert_eq!(vector.path.segments, master_path.segments);
            assert_ne!(vector.path.segments, baked.segments);
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

#[test]
fn override_fills_set_background_on_a_group_frame_root() {
    // A frame is a `Group` carrying a `background`; a Fills override on the
    // frame root must land in `background`, not be dropped. This is the
    // "white header box" bug: the master frame's background was white and a
    // dark-theme override fill was silently ignored.
    let mut scene = Scene::new();
    let (lib, comp_id, root_id, _rect_id) = master_with_vector(&mut scene);
    let inst = InstanceNode {
        component: comp_id,
        overrides: vec![Override {
            // Empty path targets the cloned root (the frame itself).
            target_path: smallvec![],
            target_prop: crate::binding::BoundProp::FillColor { index: 0 },
            value: OverrideValue::Fills {
                fills: smallvec![crate::style::Fill::solid(Color::BLACK)],
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 100.0],
    };
    let _ = root_id;
    let expanded = expand_instance(&scene, &lib, &inst);
    let root = expanded.iter().find(|e| e.def_path.is_empty()).unwrap();
    match &root.node.data {
        NodeData::Group(g) => assert_eq!(
            g.background,
            Some(crate::style::Fill::solid(Color::BLACK)),
            "frame override fill must land in background"
        ),
        other => panic!("expected group, got {other:?}"),
    }
}

#[test]
fn derived_fills_set_background_on_a_group_frame_root() {
    // The baked-derived path: a resolved (dark) fill on a frame must update
    // the `Group::background`, mirroring the override path above.
    use crate::node::DerivedOverride;
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, _rect_id) = master_with_vector(&mut scene);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            // Empty path = the frame root itself.
            path: smallvec![],
            transform: None,
            size: None,
            fills: Some(smallvec![crate::style::Fill::solid(Color::BLACK)]),
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 100.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let root = expanded.iter().find(|e| e.def_path.is_empty()).unwrap();
    match &root.node.data {
        NodeData::Group(g) => assert_eq!(
            g.background,
            Some(crate::style::Fill::solid(Color::BLACK)),
            "frame derived fill must land in background"
        ),
        other => panic!("expected group, got {other:?}"),
    }
}

#[test]
fn expand_derived_overwrites_an_unmatched_field_with_master_value() {
    // An entry that only sets `size` must leave the transform at the master's
    // value (no clobbering of fields the entry didn't populate).
    use crate::node::DerivedOverride;
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![label_id],
            transform: None,
            size: Some([1.0, 2.0]),
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    // Master label had identity transform; the size-only entry must not touch it.
    assert_eq!(
        child.node.transform,
        crate::transform::Transform2D::IDENTITY
    );
}

#[test]
fn expand_applies_derived_text_color_and_weight_to_a_text_clone() {
    // The themed-text fix: a derived entry carries the per-instance resolved
    // glyph color + weight + family (Figma bakes the dark-page label's real
    // light color here). They must overwrite the master text's defaults on
    // the cloned text node — not leave the near-black master color.
    use crate::node::{DerivedOverride, DerivedText};
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    {
        let Some(label) = scene.get_mut(label_id) else {
            panic!("missing label");
        };
        let NodeData::Text(text) = &mut label.data else {
            panic!("expected text");
        };
        let mut run_style = text.style.clone();
        run_style.color = Color::BLACK;
        text.style_runs.push(crate::node::TextStyleRun {
            start: 0,
            end: text.content.len(),
            style: run_style,
        });
    }
    // The master label is the default style: black, weight 400, Inter.
    let themed = Color::rgb(0xEE, 0xEE, 0xEE);
    let inst = InstanceNode {
        component: comp_id,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: smallvec![label_id],
            transform: None,
            size: None,
            fills: None,
            path_data: None,
            stroke_path: None,
            stroke_weight: None,
            text: Some(DerivedText {
                content: None,
                font_size: None,
                line_height: None,
                letter_spacing: None,
                color: Some(themed),
                weight: Some(600),
                family: Some("Inter".into()),
            }),
        }],
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    match &child.node.data {
        NodeData::Text(t) => {
            assert_eq!(
                t.style.color, themed,
                "derived color must overwrite master black"
            );
            assert!(
                t.style_runs.iter().all(|run| run.style.color == themed),
                "derived color must also overwrite rich text runs"
            );
            assert_eq!(
                t.style.weight, 600,
                "derived weight must overwrite master 400"
            );
            assert_eq!(
                t.style.font_family, "Inter",
                "derived family must overwrite master"
            );
            // Content untouched (the entry didn't set it).
            assert_eq!(t.content, "Label");
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn override_fills_set_glyph_color_on_a_text_clone() {
    // The dark-theme text-color fix: a themed instance pins its descendant
    // text's per-page color with a `styleIdForFill` symbolOverride (imported as
    // a `Fills` override). On a TEXT node that override must land on
    // `style.color` — not be dropped — so a Darkest card link/title renders in
    // its per-page light color (e.g. `darkest/gray/gray-700` = #D0D0D0) instead
    // of inheriting the light-master clone's near-black glyph color. Before the
    // fix `apply_override`'s `Fills` arm handled only Vector/Group, so the
    // override was silently dropped and dark text rendered dark-on-dark. This
    // is the text-channel analog of `override_fills_set_background_on_a_group_
    // frame_root` (the `_Header` surface fix). It must resolve to the override
    // color, NOT the master/default.
    let mut scene = Scene::new();
    let (lib, comp_id, _root_id, label_id) = master(&mut scene);
    {
        let Some(label) = scene.get_mut(label_id) else {
            panic!("missing label");
        };
        let NodeData::Text(text) = &mut label.data else {
            panic!("expected text");
        };
        let mut run_style = text.style.clone();
        run_style.color = Color::BLACK;
        text.style_runs.push(crate::node::TextStyleRun {
            start: 0,
            end: text.content.len(),
            style: run_style,
        });
    }
    // The master label is the default near-black; the per-page themed color is
    // a light gray (a Darkest gray-700).
    let themed = Color::rgb(0xD0, 0xD0, 0xD0);
    let inst = InstanceNode {
        component: comp_id,
        overrides: vec![Override {
            target_path: smallvec![label_id],
            target_prop: crate::binding::BoundProp::FillColor { index: 0 },
            value: OverrideValue::Fills {
                fills: smallvec![crate::style::Fill::solid(themed)],
            },
        }],
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [100.0, 40.0],
    };
    let expanded = expand_instance(&scene, &lib, &inst);
    let child = expanded
        .iter()
        .find(|e| e.def_path.as_slice() == [label_id])
        .unwrap();
    match &child.node.data {
        NodeData::Text(t) => {
            assert_eq!(
                t.style.color, themed,
                "text override fill must set the glyph color (per-page theme), not be dropped"
            );
            assert!(
                t.style_runs.iter().all(|run| run.style.color == themed),
                "text override fill must also recolor rich text runs"
            );
        }
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn expand_dangling_component_yields_empty() {
    let scene = Scene::new();
    let lib = ComponentLibrary::new();
    let inst = InstanceNode {
        component: ComponentId::new(),
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: Vec::new(),
        local_size: [10.0, 10.0],
    };
    assert!(expand_instance(&scene, &lib, &inst).is_empty());
}

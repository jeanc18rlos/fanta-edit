//! Boolean-operation rendering: the node's shape is the fold of its operand
//! children, painted with the node's own fill. Verified by probing pixels that
//! land inside vs. outside the folded region for Subtract and Intersect.

use super::*;
use fanta_doc::{BooleanNode, BooleanOp, CanvasNode, Doc, NodeData, Operation, VectorNode};

/// A boolean node with two overlapping rect operands: A = [-20,-20 .. 20,20]
/// (covers the center), B = [0,0 .. 20,20] (the bottom-right quadrant). The
/// node paints red. With an identity viewport on a 64×64 surface, world `(x, y)`
/// maps to pixel `(32 + x, 32 + y)`.
fn two_rect_boolean(op: BooleanOp) -> Doc {
    let mut doc = Doc::new();
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op,
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
        baked: None,
    }));
    let boolean_id = boolean.id;
    doc.apply(Operation::create_node(boolean)).unwrap();

    for (x, y, w, h) in [(-20.0, -20.0, 40.0, 40.0), (0.0, 0.0, 20.0, 20.0)] {
        let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
            x,
            y,
            w,
            h,
            Color::rgb(0, 255, 0), // operand paint is unused — the fold is painted
        )));
        operand.parent = Some(boolean_id);
        operand.index = doc.scene.next_child_index(Some(boolean_id));
        doc.apply(Operation::create_node(operand)).unwrap();
    }
    doc
}

#[test]
fn subtract_cuts_the_second_operand_out_of_the_first() {
    let doc = two_rect_boolean(BooleanOp::Subtract);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // Inside A, outside B (world -10,-10 → pixel 22,22): kept, red.
    let kept = rgba_at(&buf, 64, 22, 22);
    assert!(
        kept[0] > 200 && kept[3] > 200,
        "kept region should be opaque red, got {kept:?}"
    );
    // Inside A AND B (world 10,10 → pixel 42,42): subtracted away, transparent.
    let hole = rgba_at(&buf, 64, 42, 42);
    assert_eq!(
        hole[3], 0,
        "subtracted hole should be transparent, got {hole:?}"
    );
}

#[test]
fn intersect_keeps_only_the_overlap() {
    let doc = two_rect_boolean(BooleanOp::Intersect);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();

    // The overlap is exactly B = [0,0 .. 20,20]; its interior (world 10,10 →
    // pixel 42,42) is filled, and A-only (world -10,-10 → pixel 22,22) is empty.
    let overlap = rgba_at(&buf, 64, 42, 42);
    assert!(
        overlap[0] > 200 && overlap[3] > 200,
        "overlap should be opaque red, got {overlap:?}"
    );
    let outside = rgba_at(&buf, 64, 22, 22);
    assert_eq!(
        outside[3], 0,
        "outside the overlap should be transparent, got {outside:?}"
    );
}

#[test]
fn empty_boolean_paints_nothing() {
    // A boolean with no operands folds to nothing and must not panic.
    let mut doc = Doc::new();
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        op: BooleanOp::Union,
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        strokes: Default::default(),
        baked: None,
    }));
    doc.apply(Operation::create_node(boolean)).unwrap();
    let mut r = RasterRenderer::new(16, 16).unwrap();
    r.render(&doc.scene, &doc.viewport);
    assert_eq!(
        opaque_pixel_count(&r.copy_rgba()),
        0,
        "no operands ⇒ nothing painted"
    );
}

#[test]
fn boolean_cache_populates_and_clears() {
    let doc = two_rect_boolean(BooleanOp::Union);
    let mut r = RasterRenderer::new(64, 64).unwrap();
    assert_eq!(r.boolean_cache_len(), 0, "starts empty");
    r.render(&doc.scene, &doc.viewport);
    let after = r.boolean_cache_len();
    assert!(
        after > 0,
        "render of boolean should populate cache, got {}",
        after
    );
    r.clear_boolean_cache();
    assert_eq!(r.boolean_cache_len(), 0, "clear empties it");
    r.render(&doc.scene, &doc.viewport);
    assert!(r.boolean_cache_len() > 0, "re-render repopulates");
}

fn attach_test_bake(doc: &mut Doc, id: NodeId, vector: VectorNode) {
    let boolean = doc
        .scene
        .get_mut(id)
        .expect("Boolean")
        .data
        .as_boolean_mut()
        .expect("Boolean");
    boolean.baked = Some(fanta_doc::BooleanBakedGeometry {
        vector,
        source: String::new(),
        stroke_outline: false,
    });
    let source = fanta_doc::boolean_geometry_signature(&doc.scene, id).expect("signature");
    doc.scene
        .get_mut(id)
        .expect("Boolean")
        .data
        .as_boolean_mut()
        .expect("Boolean")
        .baked
        .as_mut()
        .expect("bake")
        .source = source;
}

#[test]
fn boolean_bake_preserves_pixels_until_operand_edit_and_undo() {
    let mut doc = two_rect_boolean(BooleanOp::Subtract);
    let root = *doc.scene.roots().first().expect("Boolean root");
    let vector = VectorNode {
        path: fanta_doc::PathData::ellipse(0., 0., 20., 20.),
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        local_size: Some([20., 20.]),
        ..Default::default()
    };
    let mut reference = Doc::new();
    reference
        .apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
            vector.clone(),
        ))))
        .expect("reference");
    attach_test_bake(&mut doc, root, vector);
    assert_eq!(
        crate::visual_world_bounds(&doc.scene, root, 0.),
        crate::visual_world_bounds(
            &reference.scene,
            *reference.scene.roots().first().expect("reference root"),
            0.
        ),
        "the imported viewport keeps export bounds unchanged"
    );
    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.render(&reference.scene, &reference.viewport);
    let before = renderer.copy_rgba();
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(renderer.copy_rgba(), before, "imported appearance is exact");
    let operand = *doc
        .scene
        .children_of(Some(root))
        .get(1)
        .expect("second operand");
    let old = doc.scene.get(operand).expect("operand").transform;
    doc.apply(Operation::SetTransform {
        id: operand,
        old,
        new: Transform2D::translation(60., 0.),
    })
    .expect("edit operand");
    renderer.render(&doc.scene, &doc.viewport);
    assert_ne!(
        renderer.copy_rgba(),
        before,
        "an operand edit must invalidate the import bake"
    );
    assert_eq!(
        rgba_at(&renderer.copy_rgba(), 64, 14, 14),
        [255, 0, 0, 255],
        "the new union footprint is painted"
    );
    doc.undo().expect("undo operand edit");
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(
        renderer.copy_rgba(),
        before,
        "Undo restores the authoritative original appearance"
    );
}

#[test]
fn nested_boolean_bake_keeps_unclipped_vector_geometry() {
    let mut doc = Doc::new();
    let outer = CanvasNode::new(NodeData::Boolean(BooleanNode {
        fills: smallvec_of(Fill::solid(Color::rgb(255, 0, 0))),
        ..Default::default()
    }));
    let outer_id = outer.id;
    doc.apply(Operation::create_node(outer)).expect("outer");
    let mut inner = CanvasNode::new(NodeData::Boolean(BooleanNode::default()));
    inner.parent = Some(outer_id);
    inner.flags.insert(NodeFlags::UNCLIPPED_VECTOR);
    let inner_id = inner.id;
    doc.apply(Operation::create_node(inner)).expect("inner");
    let mut operand = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.,
        0.,
        10.,
        10.,
        Color::BLACK,
    )));
    operand.parent = Some(inner_id);
    doc.apply(Operation::create_node(operand)).expect("operand");
    let vector = VectorNode {
        local_size: Some([10., 10.]),
        ..VectorNode::rect_solid(-10., -10., 30., 30., Color::rgb(255, 0, 0))
    };
    attach_test_bake(&mut doc, inner_id, vector.clone());
    let mut reference = Doc::new();
    let mut node = CanvasNode::new(NodeData::Vector(vector));
    node.flags.insert(NodeFlags::UNCLIPPED_VECTOR);
    reference
        .apply(Operation::create_node(node))
        .expect("reference");
    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.render(&reference.scene, &reference.viewport);
    let expected = renderer.copy_rgba();
    assert_eq!(rgba_at(&expected, 64, 27, 27), [255, 0, 0, 255]);
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(
        renderer.copy_rgba(),
        expected,
        "a nested bake must use the operand's viewport flags"
    );
}

#[test]
fn boolean_instances_match_live_folds_effects_and_explicit_paints() {
    use fanta_doc::{
        BoundProp, ComponentDef, ComponentId, InstanceNode, Override, OverrideValue, Shadow,
        ShadowKind,
    };
    for operation in [
        BooleanOp::Union,
        BooleanOp::Subtract,
        BooleanOp::Intersect,
        BooleanOp::Exclude,
    ] {
        let mut doc = two_rect_boolean(operation);
        let root = *doc.scene.roots().first().expect("root");
        for child in doc.scene.children_of(Some(root)).to_vec() {
            doc.scene
                .set_index(child, fanta_doc::IndexKey::FIRST)
                .expect("equal source indices");
        }
        doc.scene
            .get_mut(root)
            .expect("Boolean")
            .effects
            .push(Shadow {
                kind: ShadowKind::Inner,
                color: Color::rgba(0, 0, 0, 120),
                offset: [2., 1.],
                blur: 3.,
                spread: 0.,
                show_behind_node: true,
            });
        let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
        renderer.render(&doc.scene, &doc.viewport);
        let live = renderer.copy_rgba();
        doc.scene
            .set_transform(root, Transform2D::translation(10_000., 0.))
            .expect("move master");
        let component = ComponentId::new();
        doc.components
            .defs
            .insert(component, ComponentDef::new(component, root, "Boolean"));
        let instance = CanvasNode::new(NodeData::Instance(InstanceNode {
            component,
            overrides: Vec::new(),
            prop_values: Default::default(),
            derived: Vec::new(),
            local_size: [40., 40.],
        }));
        let instance_id = instance.id;
        doc.apply(Operation::create_node(instance))
            .expect("instance");
        let metrics = renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
        assert!(!metrics.incomplete_artwork, "{operation:?}");
        assert!(!metrics.non_artwork_content, "{operation:?}");
        assert_eq!(
            renderer.copy_rgba(),
            live,
            "expanded {operation:?} including inner shadows"
        );
        doc.scene
            .get_mut(instance_id)
            .expect("instance")
            .data
            .as_instance_mut()
            .expect("instance")
            .overrides
            .push(Override {
                target_path: Default::default(),
                target_prop: BoundProp::FillColor { index: 0 },
                value: OverrideValue::Fills {
                    fills: smallvec_of(Fill::solid(Color::rgb(0, 0, 255))),
                },
            });
        renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
        let pixels = renderer.copy_rgba();
        let mut recolored = live.clone();
        for pixel in recolored.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        assert_eq!(
            pixels, recolored,
            "explicit {operation:?} fill override preserves geometry and effects"
        );
        assert!(
            pixels
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0),
            "operand green paint must never leak"
        );
    }
}

#[test]
fn boolean_fold_honors_rounded_operands_mixed_winding_and_visibility() {
    let mut doc = Doc::new();
    let mut shape = VectorNode::rect_solid(-20., -20., 40., 40., Color::rgb(255, 0, 0));
    shape.corner_radius = Some(12.);
    let reference = CanvasNode::new(NodeData::Vector(shape.clone()));
    let mut reference_doc = Doc::new();
    reference_doc
        .apply(Operation::create_node(reference))
        .expect("reference");
    let boolean = CanvasNode::new(NodeData::Boolean(BooleanNode {
        fills: shape.fills.clone(),
        ..Default::default()
    }));
    let root = boolean.id;
    doc.apply(Operation::create_node(boolean)).expect("Boolean");
    let mut operand = CanvasNode::new(NodeData::Vector(shape));
    operand.parent = Some(root);
    let operand_id = operand.id;
    doc.apply(Operation::create_node(operand)).expect("operand");
    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.render(&reference_doc.scene, &reference_doc.viewport);
    let rounded = renderer.copy_rgba();
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(renderer.copy_rgba(), rounded);
    let mut path = fanta_doc::PathData::rect(-20., -20., 40., 40.);
    path.segments
        .extend(fanta_doc::PathData::rect(-10., -10., 20., 20.).segments);
    path.fill_rule = fanta_doc::FillRule::NonZero;
    path.subpath_rules = vec![fanta_doc::FillRule::EvenOdd, fanta_doc::FillRule::EvenOdd];
    let operand = doc.scene.get_mut(operand_id).expect("operand");
    let vector = operand.data.as_vector_mut().expect("vector");
    vector.path = path;
    vector.corner_radius = None;
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(
        rgba_at(&renderer.copy_rgba(), 64, 32, 32)[3],
        0,
        "mixed winding hole remains empty"
    );
    doc.scene
        .get_mut(operand_id)
        .expect("operand")
        .flags
        .insert(NodeFlags::HIDDEN);
    renderer.render(&doc.scene, &doc.viewport);
    assert_eq!(opaque_pixel_count(&renderer.copy_rgba()), 0);
}

#[test]
fn boolean_stroke_outline_instance_keeps_paint_when_derived_path_changes() {
    use fanta_doc::{ComponentDef, ComponentId, DerivedOverride, InstanceNode, PathData, Stroke};
    let mut doc = two_rect_boolean(BooleanOp::Union);
    let root = *doc.scene.roots().first().expect("root");
    let vector = VectorNode::rect_solid(-20., -20., 40., 40., Color::rgb(0, 0, 255));
    attach_test_bake(&mut doc, root, vector);
    let boolean = doc
        .scene
        .get_mut(root)
        .expect("root")
        .data
        .as_boolean_mut()
        .expect("Boolean");
    boolean.fills.clear();
    boolean
        .strokes
        .push(Stroke::solid(Color::rgb(0, 0, 255), 2.));
    boolean.baked.as_mut().expect("bake").stroke_outline = true;
    let source = fanta_doc::boolean_geometry_signature(&doc.scene, root).expect("signature");
    doc.scene
        .get_mut(root)
        .expect("root")
        .data
        .as_boolean_mut()
        .expect("Boolean")
        .baked
        .as_mut()
        .expect("bake")
        .source = source;
    doc.scene
        .set_transform(root, Transform2D::translation(10_000., 0.))
        .expect("move master");
    let component = ComponentId::new();
    doc.components.defs.insert(
        component,
        ComponentDef::new(component, root, "Stroke Boolean"),
    );
    let path = PathData::ellipse(0., 0., 16., 16.);
    let instance = CanvasNode::new(NodeData::Instance(InstanceNode {
        component,
        overrides: Vec::new(),
        prop_values: Default::default(),
        derived: vec![DerivedOverride {
            path: Default::default(),
            transform: None,
            size: None,
            fills: None,
            path_data: Some(path.clone()),
            stroke_path: None,
            stroke_weight: None,
            text: None,
        }],
        local_size: [40., 40.],
    }));
    doc.apply(Operation::create_node(instance))
        .expect("instance");
    let mut reference = Doc::new();
    reference
        .apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
            VectorNode {
                path,
                fills: smallvec_of(Fill::solid(Color::rgb(0, 0, 255))),
                ..Default::default()
            },
        ))))
        .expect("reference");
    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.render(&reference.scene, &reference.viewport);
    let expected = renderer.copy_rgba();
    let metrics = renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    assert!(!metrics.incomplete_artwork);
    assert_eq!(
        renderer.copy_rgba(),
        expected,
        "derived outline retains its source stroke paint"
    );
}

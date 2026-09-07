use super::*;
use fanta_doc::{
    AnimationClipId, BoundProp, ComponentLibrary, Doc, GroupNode, MaskType, Mode, ModeId,
    MotionEvaluation, MotionProperty, MotionTarget, Operation, ResolvedVarValue, Transform2D,
    UnitInterval, VarValue, Variable, VariableCollection, VariableCollectionId, VariableId,
    VariableRegistry, VariableType, VectorNode,
};

#[test]
fn motion_overlay_moves_an_offscreen_node_without_mutating_the_scene() {
    let mut doc = Doc::new();
    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    rectangle.transform = Transform2D::translation(1_000.0, 0.0);
    let node_id = rectangle.id;
    doc.apply(Operation::create_node(rectangle)).unwrap();

    let target = MotionTarget::new(node_id, MotionProperty::PositionX);
    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::from([(target, ResolvedVarValue::Float { value: 0.0 })]),
    };
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.motion = Some(&motion);

    let mut renderer = RasterRenderer::new(64, 64).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();
    let center = rgba_at(&buffer, 64, 32, 32);
    assert!(center[0] > 200 && center[3] > 200, "got {center:?}");

    assert_eq!(
        doc.scene.get(node_id).map(|node| node.transform),
        Some(Transform2D::translation(1_000.0, 0.0))
    );
}

#[test]
fn motion_overlay_uses_moved_bounds_for_mask_effect_layer_and_culling() {
    let mut doc = Doc::new();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode::default()));
    group.opacity = UnitInterval::new(0.5);
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();

    let mut mask = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -200.0,
        -10.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    mask.parent = Some(group_id);
    mask.index = fanta_doc::IndexKey::from_raw(1.0);
    mask.is_mask = true;
    mask.mask_type = MaskType::Alpha;
    let mask_id = mask.id;
    doc.apply(Operation::create_node(mask)).unwrap();

    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -200.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    rectangle.parent = Some(group_id);
    rectangle.index = fanta_doc::IndexKey::from_raw(2.0);
    let rectangle_id = rectangle.id;
    doc.apply(Operation::create_node(rectangle)).unwrap();

    let mask_target = MotionTarget::new(mask_id, MotionProperty::PositionX);
    let rectangle_target = MotionTarget::new(rectangle_id, MotionProperty::PositionX);
    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::from([
            (mask_target, ResolvedVarValue::Float { value: 220.0 }),
            (rectangle_target, ResolvedVarValue::Float { value: 220.0 }),
        ]),
    };
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.motion = Some(&motion);

    let mut renderer = RasterRenderer::new(140, 80).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();
    let moved_center = rgba_at(&buffer, 140, 100, 40);
    assert!(
        moved_center[0] > 200 && moved_center[1] < 40 && moved_center[3] > 80,
        "animated masked child was clipped by authored cull/effect/mask bounds: {moved_center:?}"
    );
}

#[test]
fn motion_bounds_include_children_overflowing_a_non_clipping_group_box() {
    let mut doc = Doc::new();
    let mut group = CanvasNode::new(NodeData::Group(GroupNode {
        local_size: Some([20.0, 20.0]),
        ..Default::default()
    }));
    group.transform = Transform2D::translation(-200.0, 0.0);
    let group_id = group.id;
    doc.apply(Operation::create_node(group)).unwrap();

    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    rectangle.parent = Some(group_id);
    rectangle.transform = Transform2D::translation(200.0, 0.0);
    doc.apply(Operation::create_node(rectangle)).unwrap();

    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::new(),
    };
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.motion = Some(&motion);

    let mut renderer = RasterRenderer::new(64, 64).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();
    let center = rgba_at(&buffer, 64, 32, 32);
    assert!(
        center[0] > 200 && center[1] < 40 && center[2] < 40 && center[3] > 200,
        "overflowing child was culled by the group's authored box: {center:?}"
    );
}

#[test]
fn disabled_frame_clipping_keeps_off_box_overflow_visible_for_normal_and_motion_rendering() {
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([20.0, 20.0]),
        ..Default::default()
    }));
    frame.meta = serde_json::json!({ "clip_content": false });
    frame.opacity = UnitInterval::new(0.5);
    frame.transform = Transform2D::translation(-200.0, 0.0);
    let frame_id = frame.id;
    doc.apply(Operation::create_node(frame)).unwrap();

    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    rectangle.parent = Some(frame_id);
    rectangle.transform = Transform2D::translation(200.0, 0.0);
    doc.apply(Operation::create_node(rectangle)).unwrap();

    let mut renderer = RasterRenderer::new(64, 64).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    let buffer = renderer.copy_rgba();
    let center = rgba_at(&buffer, 64, 32, 32);
    assert!(
        center[0] > 200 && center[1] < 40 && center[2] < 40 && center[3] > 80,
        "disabled frame clipping culled visible overflow: {center:?}"
    );

    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::new(),
    };
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.motion = Some(&motion);

    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();
    let center = rgba_at(&buffer, 64, 32, 32);
    assert!(
        center[0] > 200 && center[1] < 40 && center[2] < 40 && center[3] > 80,
        "motion bounds culled visible overflow despite disabled frame clipping: {center:?}"
    );
}

#[test]
fn motion_bound_property_overrides_the_resolved_variable_value() {
    let collection_id = VariableCollectionId::new();
    let mode_id = ModeId::new();
    let variable_id = VariableId::new();
    let variables = VariableRegistry {
        collections: BTreeMap::from([(
            collection_id,
            VariableCollection {
                id: collection_id,
                name: "Theme".into(),
                modes: vec![Mode {
                    id: mode_id,
                    name: "Default".into(),
                }],
                default_mode: mode_id,
                variable_order: vec![variable_id],
            },
        )]),
        variables: BTreeMap::from([(
            variable_id,
            Variable {
                id: variable_id,
                collection: collection_id,
                name: "Surface".into(),
                ty: VariableType::Color,
                values_by_mode: BTreeMap::from([(
                    mode_id,
                    VarValue::Color {
                        value: Color::rgb(0, 0, 255),
                    },
                )]),
                scopes: Vec::new(),
            },
        )]),
    };

    let mut doc = Doc::new();
    let mut rectangle = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    rectangle
        .bindings
        .insert(BoundProp::FillColor { index: 0 }, variable_id);
    let rectangle_id = rectangle.id;
    doc.apply(Operation::create_node(rectangle)).unwrap();

    let target = MotionTarget::new(
        rectangle_id,
        MotionProperty::bound(BoundProp::FillColor { index: 0 }),
    );
    let motion = MotionEvaluation {
        clip: AnimationClipId::new(),
        playhead_ms: 500,
        overrides: BTreeMap::from([(
            target,
            ResolvedVarValue::Color {
                value: Color::rgb(255, 0, 0),
            },
        )]),
    };
    let components = ComponentLibrary::new();
    let inputs = RenderInputs {
        components: &components,
        variables: &variables,
        active_modes: &BTreeMap::new(),
        mode_generation: 0,
        motion: Some(&motion),
        playback: None,
        dark_ui: false,
    };

    let mut renderer = RasterRenderer::new(64, 64).unwrap();
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let buffer = renderer.copy_rgba();
    let center = rgba_at(&buffer, 64, 32, 32);
    assert!(
        center[0] > 200 && center[1] < 40 && center[2] < 40,
        "motion must override the blue variable value, got {center:?}"
    );
}

use super::*;
use fanta_doc::{
    BlendMode, Doc, GroupNode, Operation, PatternFill, PatternHorizontalAlignment, PatternSpacing,
    PatternTileType, VectorNode,
};

fn pattern_fill(
    source_node_id: NodeId,
    tile_type: PatternTileType,
    spacing: PatternSpacing,
    horizontal_alignment: PatternHorizontalAlignment,
) -> Fill {
    Fill::Pattern {
        pattern: Box::new(PatternFill {
            source_node_id,
            tile_type,
            scaling_factor: 1.0,
            spacing,
            horizontal_alignment,
        }),
        opacity: 1.0,
        blend: BlendMode::Normal,
    }
}

fn pattern_doc(
    tile_type: PatternTileType,
    spacing: PatternSpacing,
    horizontal_alignment: PatternHorizontalAlignment,
    target_width: f64,
) -> (Doc, NodeId) {
    let mut doc = Doc::new();
    let mut source = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([10.0, 10.0]),
        ..GroupNode::default()
    }));
    let source_id = source.id;
    source.transform = Transform2D::translation(200.0, 200.0);
    doc.apply(Operation::create_node(source))
        .expect("source group");
    let mut red = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        0.0,
        0.0,
        5.0,
        10.0,
        Color::rgb(255, 0, 0),
    )));
    red.parent = Some(source_id);
    let red_id = red.id;
    doc.apply(Operation::create_node(red))
        .expect("source red half");
    let mut blue = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        5.0,
        0.0,
        5.0,
        10.0,
        Color::rgb(0, 0, 255),
    )));
    blue.parent = Some(source_id);
    doc.apply(Operation::create_node(blue))
        .expect("source blue half");

    let mut vector = VectorNode::rect_solid(0.0, 0.0, target_width, 20.0, Color::BLACK);
    vector.fills = smallvec_of(pattern_fill(
        source_id,
        tile_type,
        spacing,
        horizontal_alignment,
    ));
    let mut target = CanvasNode::new(NodeData::Vector(vector));
    target.transform = Transform2D::translation(-target_width * 0.5, -10.0);
    doc.apply(Operation::create_node(target)).expect("target");
    (doc, red_id)
}

fn render(doc: &Doc) -> (Vec<u8>, RenderMetrics) {
    let mut renderer = RasterRenderer::new(64, 64).expect("surface");
    let metrics = renderer.render(&doc.scene, &doc.viewport);
    (renderer.copy_rgba(), metrics)
}

fn pattern_target_id(doc: &Doc) -> NodeId {
    doc.scene
        .roots()
        .iter()
        .copied()
        .find(|id| {
            doc.scene.get(*id).is_some_and(|node| {
                matches!(&node.data, NodeData::Vector(vector) if vector.fills.iter().any(|fill| matches!(fill, Fill::Pattern { .. })))
            })
        })
        .expect("pattern target")
}

#[test]
fn node_pattern_repeats_even_when_source_is_outside_viewport() {
    let (doc, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        40.0,
    );
    let (pixels, _) = render(&doc);
    let first_red = rgba_at(&pixels, 64, 14, 27);
    let first_blue = rgba_at(&pixels, 64, 19, 27);
    let second_red = rgba_at(&pixels, 64, 24, 27);
    assert!(first_red[0] > 220 && first_red[2] < 40, "{first_red:?}");
    assert!(first_blue[2] > 220 && first_blue[0] < 40, "{first_blue:?}");
    assert!(second_red[0] > 220 && second_red[2] < 40, "{second_red:?}");
}

#[test]
fn pattern_spacing_and_alignment_change_rendered_pixels() {
    let (unspaced, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        30.0,
    );
    let (spaced_start, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing { x: 0.5, y: 0.0 },
        PatternHorizontalAlignment::Start,
        30.0,
    );
    let (spaced_center, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing { x: 0.5, y: 0.0 },
        PatternHorizontalAlignment::Center,
        30.0,
    );
    let (spaced_end, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing { x: 0.5, y: 0.0 },
        PatternHorizontalAlignment::End,
        30.0,
    );
    let unspaced = render(&unspaced).0;
    let start = render(&spaced_start).0;
    let center = render(&spaced_center).0;
    let end = render(&spaced_end).0;
    assert!(rgba_at(&unspaced, 64, 29, 27)[3] > 200);
    assert_eq!(rgba_at(&start, 64, 29, 27)[3], 0);
    assert_ne!(start, center);
    assert_ne!(center, end);
}

#[test]
fn three_pattern_tile_types_produce_distinct_images() {
    let mut images = Vec::new();
    for tile_type in [
        PatternTileType::Rectangular,
        PatternTileType::HorizontalHexagonal,
        PatternTileType::VerticalHexagonal,
    ] {
        let (doc, _) = pattern_doc(
            tile_type,
            PatternSpacing::default(),
            PatternHorizontalAlignment::Start,
            40.0,
        );
        images.push(render(&doc).0);
    }
    assert_ne!(images[0], images[1]);
    assert_ne!(images[0], images[2]);
    assert_ne!(images[1], images[2]);
}

#[test]
fn cyclic_pattern_source_stops_recursion() {
    let mut doc = Doc::new();
    let mut node = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::BLACK,
    )));
    let id = node.id;
    node.data.as_vector_mut().expect("vector").fills = smallvec_of(pattern_fill(
        id,
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
    ));
    doc.apply(Operation::create_node(node))
        .expect("self-referential pattern");
    let (_, metrics) = render(&doc);
    assert!(metrics.nodes_visited < 10, "recursion stopped: {metrics:?}");
}

#[test]
fn editing_source_updates_existing_pattern_cache() {
    let (mut doc, red_id) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        40.0,
    );
    let mut renderer = RasterRenderer::new(64, 64).expect("surface");
    renderer.render(&doc.scene, &doc.viewport);
    let before = rgba_at(&renderer.copy_rgba(), 64, 14, 27);
    let Some(red) = doc.scene.get_mut(red_id) else {
        panic!("red source node exists");
    };
    red.data.as_vector_mut().expect("vector").fills =
        smallvec_of(Fill::solid(Color::rgb(0, 255, 0)));
    renderer.render(&doc.scene, &doc.viewport);
    let after = rgba_at(&renderer.copy_rgba(), 64, 14, 27);
    assert!(before[0] > 220 && before[1] < 40, "{before:?}");
    assert!(after[1] > 220 && after[0] < 40, "{after:?}");
}

#[test]
fn hidden_source_still_supplies_visible_pattern() {
    let (mut doc, red_id) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        40.0,
    );
    let source_id = doc
        .scene
        .get(red_id)
        .and_then(|node| node.parent)
        .expect("source");
    doc.scene
        .get_mut(source_id)
        .expect("source")
        .flags
        .insert(fanta_doc::NodeFlags::HIDDEN);
    let (pixels, _) = render(&doc);
    let sample = rgba_at(&pixels, 64, 14, 27);
    assert!(sample[0] > 220 && sample[2] < 40, "{sample:?}");
}

#[test]
fn pattern_scale_and_opacity_affect_painted_tile() {
    let (mut doc, _) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        40.0,
    );
    let target_id = pattern_target_id(&doc);
    let target = doc.scene.get_mut(target_id).expect("target");
    let vector = target.data.as_vector_mut().expect("vector");
    let Fill::Pattern {
        pattern, opacity, ..
    } = vector.fills.first_mut().expect("fill")
    else {
        panic!("pattern fill");
    };
    pattern.scaling_factor = 2.0;
    *opacity = 0.5;
    let (pixels, _) = render(&doc);
    let enlarged_red = rgba_at(&pixels, 64, 19, 27);
    assert!(
        enlarged_red[0] > 220 && enlarged_red[2] < 40,
        "{enlarged_red:?}"
    );
    assert!((110..=145).contains(&enlarged_red[3]), "{enlarged_red:?}");
}

#[test]
fn pattern_paints_frame_background_and_vector_stroke() {
    let (mut doc, red_id) = pattern_doc(
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
        40.0,
    );
    let source_id = doc
        .scene
        .get(red_id)
        .and_then(|node| node.parent)
        .expect("source");
    let target_id = pattern_target_id(&doc);
    let target = doc.scene.get_mut(target_id).expect("target");
    target.data = NodeData::Group(GroupNode {
        clip_size: Some([40.0, 20.0]),
        background: Some(pattern_fill(
            source_id,
            PatternTileType::Rectangular,
            PatternSpacing::default(),
            PatternHorizontalAlignment::Start,
        )),
        ..GroupNode::default()
    });
    let (frame_pixels, _) = render(&doc);
    let frame_sample = rgba_at(&frame_pixels, 64, 14, 27);
    assert!(
        frame_sample[0] > 220 && frame_sample[2] < 40,
        "{frame_sample:?}"
    );

    let target = doc.scene.get_mut(target_id).expect("target");
    let mut vector = VectorNode::rect_solid(0.0, 0.0, 40.0, 20.0, Color::BLACK);
    vector.fills.clear();
    let mut stroke = fanta_doc::Stroke::solid(Color::BLACK, 4.0);
    stroke.paint = pattern_fill(
        source_id,
        PatternTileType::Rectangular,
        PatternSpacing::default(),
        PatternHorizontalAlignment::Start,
    );
    vector.strokes.push(stroke);
    target.data = NodeData::Vector(vector);
    let (stroke_pixels, _) = render(&doc);
    let border_sample = rgba_at(&stroke_pixels, 64, 14, 22);
    let interior_sample = rgba_at(&stroke_pixels, 64, 32, 32);
    assert!(
        border_sample[0] > 220 && border_sample[2] < 40,
        "{border_sample:?}"
    );
    assert_eq!(interior_sample[3], 0, "{interior_sample:?}");
}

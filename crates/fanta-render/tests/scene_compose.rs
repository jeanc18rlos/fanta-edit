//! End-to-end: build a Doc with multiple node types, render it, and verify the
//! pixel buffer reflects the composition order, group transforms, opacity, and
//! variant-specific placeholders for unrenderable types.

use fanta_doc::{
    BlendMode, CanvasNode, Color, Doc, GroupNode, IndexKey, NodeData, Operation, Transform2D,
    VectorNode,
};
use fanta_render::RasterRenderer;

fn rgba_at(buf: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let row = (width * 4) as usize;
    let i = (y as usize) * row + (x as usize) * 4;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

#[test]
fn higher_z_index_paints_over_lower() {
    let mut doc = Doc::new();
    // Bottom rect (large, blue)
    let mut bottom = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -30.0,
        -30.0,
        60.0,
        60.0,
        Color::rgb(0, 0, 255),
    )));
    bottom.index = IndexKey::from_raw(1.0);
    doc.apply(Operation::create_node(bottom)).unwrap();
    // Top rect (small, red), same center.
    let mut top = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 0, 0),
    )));
    top.index = IndexKey::from_raw(2.0);
    doc.apply(Operation::create_node(top)).unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // Center pixel is under the red rect.
    let center = rgba_at(&buf, 80, 40, 40);
    assert!(center[0] > 200, "center should be red, got {center:?}");
    // Pixel offset 20 from center, still over the blue rect but past the red.
    let blue_band = rgba_at(&buf, 80, 60, 40);
    assert!(blue_band[2] > 200, "edge should be blue, got {blue_band:?}");
}

#[test]
fn group_transform_composes_with_child_transform() {
    let mut doc = Doc::new();
    // Group translated +20 in X.
    let mut g = CanvasNode::new(NodeData::Group(GroupNode::default()));
    g.transform = Transform2D::translation(20.0, 0.0);
    let g_id = g.id;
    doc.apply(Operation::create_node(g)).unwrap();
    // Child rect, no own translation, drawn around origin (in group-local space).
    let mut child = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -5.0,
        -5.0,
        10.0,
        10.0,
        Color::rgb(0, 200, 0),
    )));
    child.parent = Some(g_id);
    doc.apply(Operation::create_node(child)).unwrap();

    let mut r = RasterRenderer::new(80, 80).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    // The rect should be at world (15, -5) to (25, 5) → screen (55, 35) to (65, 45).
    let inside = rgba_at(&buf, 80, 60, 40);
    let outside = rgba_at(&buf, 80, 40, 40);
    assert!(
        inside[1] > 150,
        "inside group should be green, got {inside:?}"
    );
    assert!(
        outside[3] == 0,
        "world origin should be transparent, got {outside:?}"
    );
}

#[test]
fn opacity_attenuates_node_alpha() {
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::rgb(255, 255, 255),
    )));
    n.opacity = 0.5;
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    r.render(&doc.scene, &doc.viewport);
    let buf = r.copy_rgba();
    let center = rgba_at(&buf, 64, 32, 32);
    // 50% opacity gives an alpha around 127. Be generous on tolerance for any
    // gamma / quantization differences.
    assert!(center[3] > 90 && center[3] < 165, "alpha was {}", center[3]);
}

#[test]
fn unrenderable_variants_draw_placeholders_not_panic() {
    use fanta_doc::{AiArtifactNode, AssetId, GenerationStatus};
    let mut doc = Doc::new();
    let n = CanvasNode::new(NodeData::AiArtifact(AiArtifactNode {
        local_size: [40.0, 40.0],
        prompt: "test".into(),
        model: "test".into(),
        params: serde_json::json!({}),
        inputs: vec![],
        lineage_parent: None,
        output: Some(AssetId::new()),
        status: GenerationStatus::Done,
        seed: None,
    }));
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(64, 64).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    // Placeholder draws — that's enough to confirm we don't panic for missing
    // asset / asset store integration.
    assert!(metrics.nodes_drawn >= 1);
}

#[test]
fn blend_mode_field_is_accepted_even_if_not_yet_honored() {
    // We don't actually honor blend modes yet (Normal only). This test
    // confirms a non-Normal value doesn't break the render path — future-
    // proofing the integration before the feature lands.
    let mut doc = Doc::new();
    let mut n = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        -10.0,
        -10.0,
        20.0,
        20.0,
        Color::WHITE,
    )));
    n.blend_mode = BlendMode::Multiply;
    doc.apply(Operation::create_node(n)).unwrap();

    let mut r = RasterRenderer::new(32, 32).unwrap();
    let metrics = r.render(&doc.scene, &doc.viewport);
    assert!(metrics.nodes_drawn >= 1);
}

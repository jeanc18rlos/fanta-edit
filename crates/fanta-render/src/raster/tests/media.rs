use super::*;
use fanta_doc::{
    AssetId, Doc, GroupNode, ImageFitMode, IndexKey, Operation, VectorNode, VideoNode,
};
use std::collections::HashMap;

fn video() -> CanvasNode {
    CanvasNode::new(NodeData::Video(VideoNode {
        asset: AssetId::new(),
        natural_size: [16, 16],
        local_size: [80., 80.],
        time_range_us: [0, 1_000_000],
        speed: 1.,
        muted: true,
        volume: 0.,
        poster_frame_us: None,
        poster: None,
        fit: ImageFitMode::Fit,
    }))
}

fn decoded_frame(color: skia_safe::Color) -> skia_safe::Image {
    let mut surface =
        skia_safe::surfaces::raster_n32_premul((16, 16)).expect("small test frame surface");
    surface.canvas().clear(color);
    surface.image_snapshot()
}

fn playback(node: NodeId, color: skia_safe::Color) -> HashMap<NodeId, MediaPlayback> {
    HashMap::from([(
        node,
        MediaPlayback {
            progress: 0.25,
            frame: None,
            decoded_frame: Some(decoded_frame(color)),
        },
    )])
}

#[test]
fn live_video_frames_respect_parent_clip_transform_and_layer_order() {
    let mut doc = Doc::new();
    let mut parent = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([64., 64.]),
        ..Default::default()
    }));
    parent.transform = Transform2D::translation(-32., -32.);
    let parent_id = parent.id;
    doc.apply(Operation::create_node(parent)).expect("parent");
    let mut clip = video();
    clip.parent = Some(parent_id);
    clip.transform = Transform2D::translation(-8., -8.);
    let clip_id = clip.id;
    doc.apply(Operation::create_node(clip)).expect("video");
    let mut above = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        28.,
        28.,
        8.,
        8.,
        Color::rgb(0, 0, 255),
    )));
    above.parent = Some(parent_id);
    above.index = IndexKey::after(IndexKey::FIRST);
    doc.apply(Operation::create_node(above))
        .expect("foreground");
    let original = serde_json::to_value(&doc.scene).expect("scene snapshot");
    let live = playback(clip_id, skia_safe::Color::RED);
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.playback = Some(&live);
    let mut renderer = RasterRenderer::new(128, 128).expect("renderer");
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let pixels = renderer.copy_rgba();
    assert_eq!(rgba_at(&pixels, 128, 44, 52), [255, 0, 0, 255]);
    assert_eq!(rgba_at(&pixels, 128, 64, 64), [0, 0, 255, 255]);
    assert_eq!(rgba_at(&pixels, 128, 20, 52)[3], 0);
    assert_eq!(rgba_at(&pixels, 128, 108, 52)[3], 0);
    assert_eq!(serde_json::to_value(&doc.scene).expect("scene"), original);
}

#[test]
fn live_video_frames_bypass_stale_effect_layers_without_document_edits() {
    let mut doc = Doc::new();
    let mut clip = video();
    clip.transform = Transform2D::translation(-40., -40.);
    clip.blurs.push(Blur::layer(2.));
    let clip_id = clip.id;
    doc.apply(Operation::create_node(clip)).expect("video");
    let mut static_layer = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        44.,
        -30.,
        10.,
        10.,
        Color::rgb(0, 255, 0),
    )));
    static_layer.blurs.push(Blur::layer(2.));
    static_layer.index = IndexKey::after(IndexKey::FIRST);
    doc.apply(Operation::create_node(static_layer))
        .expect("static effect");
    let revision = doc.scene.revision();
    let mut renderer = RasterRenderer::new(128, 128).expect("renderer");
    for _ in 0..4 {
        renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    }
    assert!(
        renderer.layer_cache_stats().0 > 0,
        "warm cached effect layer"
    );
    for color in [skia_safe::Color::RED, skia_safe::Color::BLUE] {
        let live = playback(clip_id, color);
        let mut inputs = RenderInputs::for_doc(&doc);
        inputs.playback = Some(&live);
        let metrics = renderer.render_with(&doc.scene, &doc.viewport, &inputs);
        assert!(
            metrics.layer_cache_hits > 0,
            "unrelated static effects stay cached"
        );
        let expected = if color == skia_safe::Color::RED {
            [255, 0, 0, 255]
        } else {
            [0, 0, 255, 255]
        };
        assert_eq!(rgba_at(&renderer.copy_rgba(), 128, 64, 64), expected);
    }
    assert_eq!(doc.scene.revision(), revision);
}

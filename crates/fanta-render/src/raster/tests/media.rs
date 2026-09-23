use super::*;
use fanta_doc::{
    AssetId, BlendMode, Doc, GroupNode, ImageAdjust, ImageFitMode, IndexKey, Operation, VectorNode,
    VideoFill, VideoNode,
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

fn video_fill(asset: AssetId, poster: Option<AssetId>, opacity: f32) -> Fill {
    Fill::Video {
        video: Box::new(VideoFill {
            asset,
            poster,
            mode: ImageFitMode::Stretch,
            crop: None,
            scale: None,
            rotation: None,
            adjust: ImageAdjust::default(),
        }),
        opacity,
        blend: BlendMode::Normal,
    }
}

#[test]
fn separate_video_fills_on_one_node_use_their_own_source_frames() {
    let red_source = AssetId::new();
    let blue_source = AssetId::new();
    let mut vector = VectorNode::rect_solid(-20.0, -20.0, 40.0, 40.0, Color::BLACK);
    vector.fills = smallvec_of(video_fill(red_source, None, 1.0));
    vector.fills.push(video_fill(blue_source, None, 0.5));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        vector,
    ))))
    .expect("video fill node");

    let frames = HashMap::from([
        (red_source, decoded_frame(skia_safe::Color::RED)),
        (blue_source, decoded_frame(skia_safe::Color::BLUE)),
    ]);
    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.video_fill_frames = Some(&frames);
    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let pixel = rgba_at(&renderer.copy_rgba(), 64, 32, 32);
    assert!(
        (115..=140).contains(&pixel[0])
            && pixel[1] < 10
            && (115..=140).contains(&pixel[2])
            && pixel[3] == 255,
        "two independent video source frames should composite purple: {pixel:?}"
    );
}

#[test]
fn video_fill_prefers_live_frame_and_reverts_to_poster() {
    use crate::asset::{DecodedImage, InMemoryAssetResolver};

    let source = AssetId::new();
    let poster = AssetId::new();
    let mut resolver = InMemoryAssetResolver::new();
    resolver.insert(
        poster,
        DecodedImage::new(Arc::new(vec![0, 255, 0, 255].repeat(4)), 2, 2),
    );
    let mut vector = VectorNode::rect_solid(-20.0, -20.0, 40.0, 40.0, Color::BLACK);
    vector.fills = smallvec_of(video_fill(source, Some(poster), 1.0));
    let mut node = CanvasNode::new(NodeData::Vector(vector));
    node.blurs.push(Blur::layer(2.0));
    let mut doc = Doc::new();
    doc.apply(Operation::create_node(node))
        .expect("video fill node");
    let mut static_layer = CanvasNode::new(NodeData::Vector(VectorNode::rect_solid(
        24.0,
        -20.0,
        10.0,
        10.0,
        Color::rgb(255, 255, 0),
    )));
    static_layer.blurs.push(Blur::layer(2.0));
    doc.apply(Operation::create_node(static_layer))
        .expect("unrelated static layer");

    let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
    renderer.set_asset_resolver(Arc::new(resolver));
    for _ in 0..4 {
        renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    }
    assert!(
        renderer.layer_cache_stats().0 >= 2,
        "poster and unrelated static layers should both be cached"
    );
    let poster_metrics =
        renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    assert!(poster_metrics.layer_cache_hits >= 2);
    let poster_pixel = rgba_at(&renderer.copy_rgba(), 64, 32, 32);
    assert!(
        poster_pixel[1] > 200 && poster_pixel[0] < 20,
        "{poster_pixel:?}"
    );

    for (color, channel) in [(skia_safe::Color::RED, 0), (skia_safe::Color::BLUE, 2)] {
        let frames = HashMap::from([(source, decoded_frame(color))]);
        let mut inputs = RenderInputs::for_doc(&doc);
        inputs.video_fill_frames = Some(&frames);
        let live_metrics = renderer.render_with(&doc.scene, &doc.viewport, &inputs);
        assert!(
            live_metrics.layer_cache_hits > 0,
            "unrelated static layer should stay cached"
        );
        let pixel = rgba_at(&renderer.copy_rgba(), 64, 32, 32);
        assert!(
            pixel[channel] > 200,
            "live frame should replace poster: {pixel:?}"
        );
        assert!(
            pixel[1] < 20,
            "poster must not cover a live frame: {pixel:?}"
        );
    }

    renderer.render_with(&doc.scene, &doc.viewport, &RenderInputs::for_doc(&doc));
    let poster_pixel = rgba_at(&renderer.copy_rgba(), 64, 32, 32);
    assert!(
        poster_pixel[1] > 200 && poster_pixel[0] < 20,
        "{poster_pixel:?}"
    );
}

#[test]
fn live_video_paint_renders_frame_background_and_vector_stroke() {
    use fanta_doc::{PathData, Stroke};

    let frame_asset = AssetId::new();
    let stroke_asset = AssetId::new();
    let frames = HashMap::from([
        (frame_asset, decoded_frame(skia_safe::Color::RED)),
        (stroke_asset, decoded_frame(skia_safe::Color::BLUE)),
    ]);
    let mut doc = Doc::new();
    let mut frame = CanvasNode::new(NodeData::Group(GroupNode {
        clip_size: Some([20.0, 20.0]),
        background: Some(video_fill(frame_asset, None, 1.0)),
        ..GroupNode::default()
    }));
    frame.transform = Transform2D::translation(-30.0, -10.0);
    doc.apply(Operation::create_node(frame)).expect("frame");
    let mut stroke = Stroke::solid(Color::WHITE, 6.0);
    stroke.paint = video_fill(stroke_asset, None, 1.0);
    let vector = VectorNode {
        path: PathData::rect(10.0, -10.0, 20.0, 20.0),
        fills: Default::default(),
        strokes: [stroke].into_iter().collect(),
        corner_radius: None,
        corner_radii: None,
        corner_smoothing: 0.0,
        local_size: None,
        parametric: None,
    };
    doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
        vector,
    ))))
    .expect("stroke");

    let mut inputs = RenderInputs::for_doc(&doc);
    inputs.video_fill_frames = Some(&frames);
    let mut renderer = RasterRenderer::new(96, 64).expect("renderer");
    renderer.render_with(&doc.scene, &doc.viewport, &inputs);
    let pixels = renderer.copy_rgba();
    let background = rgba_at(&pixels, 96, 28, 32);
    let border = rgba_at(&pixels, 96, 58, 32);
    assert_eq!(background, [255, 0, 0, 255]);
    assert_eq!(border, [0, 0, 255, 255]);
    assert_eq!(rgba_at(&pixels, 96, 68, 32)[3], 0);
}

#[test]
fn video_paint_covers_outside_per_side_borders_with_square_and_rounded_corners() {
    use fanta_doc::{PathData, Stroke, StrokeAlign};

    let source = AssetId::new();
    let frames = HashMap::from([(source, decoded_frame(skia_safe::Color::BLUE))]);
    for corner_radius in [None, Some(8.0)] {
        let mut stroke = Stroke::solid(Color::WHITE, 6.0);
        stroke.paint = video_fill(source, None, 1.0);
        stroke.align = StrokeAlign::Outside;
        stroke.per_side = Some([6.0, 0.0, 0.0, 0.0]);
        let vector = VectorNode {
            path: PathData::rect(-20.0, -20.0, 40.0, 40.0),
            fills: Default::default(),
            strokes: [stroke].into_iter().collect(),
            corner_radius,
            corner_radii: None,
            corner_smoothing: 0.0,
            local_size: None,
            parametric: None,
        };
        let mut doc = Doc::new();
        doc.apply(Operation::create_node(CanvasNode::new(NodeData::Vector(
            vector,
        ))))
        .expect("video border");
        let mut inputs = RenderInputs::for_doc(&doc);
        inputs.video_fill_frames = Some(&frames);
        let mut renderer = RasterRenderer::new(64, 64).expect("renderer");
        renderer.render_with(&doc.scene, &doc.viewport, &inputs);
        let pixels = renderer.copy_rgba();
        assert_eq!(rgba_at(&pixels, 64, 32, 9), [0, 0, 255, 255]);
        assert_eq!(rgba_at(&pixels, 64, 32, 32)[3], 0);
    }
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
